//! Toolpaths -> Z-Suite binary command stream (`ZCodeData`), following the structure of real files:
//! prologue (temperatures, homing, calibration), per-layer markers, retraction + Z-hop travel,
//! `Progress` marks and the shutdown sequence.

use zcodex2::{Axes, Command, Move, Rapid, Record};

use crate::geom::{dist, Pt};
use crate::settings::Settings;
use crate::toolpath::{role, Plan};

pub(crate) const JUMP: u8 = 252;
pub(crate) const RETRACT: u8 = 253;
pub(crate) const UNRETRACT: u8 = 254;
pub(crate) const UNKNOWN: u8 = 255;

#[derive(Debug)]
pub struct Emitted {
    pub records: Vec<Record>,
    /// Estimated print time in seconds.
    pub print_time_s: f64,
    /// Filament pushed during printing, in mm (retractions excluded).
    pub filament_mm: f64,
    /// Part of `filament_mm` used by supports.
    pub support_mm: f64,
    /// `[min_x, min_y, min_z, max_x, max_y, max_z]` in µm.
    pub bbox: [i64; 6],
    /// Cumulative filament (mm) and support filament (mm) at each `Progress` mark 0..=100.
    pub progress_usage: Vec<(f64, f64)>,
}

struct Em<'a> {
    s: &'a Settings,
    out: Vec<(Record, f64)>, // record + cumulative time after it
    t: f64,
    x: i64,
    y: i64,
    z: i64,
    feed: i32,
    fan: u8,
    retracted: bool,
    e_carry: f64,
    filament_um: f64,
    support_um: f64,
    bbox: [i64; 6],
}

pub(crate) fn is_support(role_: u8) -> bool {
    matches!(role_, role::SUPPORT_CONTOUR | role::SUPPORT_INFILL | role::INTERFACE_INFILL)
}

impl<'a> Em<'a> {
    fn push(&mut self, c: Command) {
        self.out.push((c.to_record(), self.t));
    }

    fn set_feed(&mut self, f: f64) {
        let f = f.round().max(1.0) as i32;
        if f != self.feed {
            self.feed = f;
            self.push(Command::Feedrate(f));
        }
    }

    fn move_time(&mut self, len_um: f64) {
        self.t += len_um / 1000.0 / (self.feed as f64 / 60.0);
    }

    fn rapid_xy(&mut self, p: Pt) {
        let d = dist(Pt::new(self.x, self.y), p);
        self.move_time(d);
        self.push(Command::Rapid(Rapid {
            role: JUMP,
            x: (p.x != self.x).then_some(p.x as i32),
            y: (p.y != self.y).then_some(p.y as i32),
            z: None,
        }));
        self.x = p.x;
        self.y = p.y;
    }

    fn rapid_z(&mut self, z: i64) {
        self.move_time((z - self.z).abs() as f64);
        self.push(Command::Rapid(Rapid { role: JUMP, x: None, y: None, z: Some(z as i32) }));
        self.z = z;
    }

    fn retract(&mut self) {
        if self.retracted || self.s.retraction_distance <= 0.0 {
            return;
        }
        self.set_feed(self.s.retraction_speed * 60.0);
        let e = (self.s.retraction_distance * 1000.0).round() as i32;
        self.t += self.s.retraction_distance / self.s.retraction_speed.max(1.0);
        self.push(Command::Move(Move { role: RETRACT, e: Some(-e), ..Default::default() }));
        self.retracted = true;
    }

    fn unretract(&mut self) {
        if !self.retracted {
            return;
        }
        if self.s.retraction_distance > 0.0 {
            self.set_feed(self.s.retraction_speed * 60.0);
            let e = ((self.s.retraction_distance + self.s.retraction_extra) * 1000.0).round() as i32;
            self.t += self.s.retraction_distance / self.s.retraction_speed.max(1.0);
            self.push(Command::Move(Move { role: UNRETRACT, e: Some(e), ..Default::default() }));
        }
        self.retracted = false;
    }

    fn set_fan(&mut self, pct: u8) {
        let v = ((pct.min(100) as f64) * 2.55).round() as u8;
        if v != self.fan {
            self.fan = v;
            self.push(Command::Move(Move { role: UNKNOWN, fan2: Some(v as i32), ..Default::default() }));
        }
    }

    /// Travel to `to`, retracting and hopping when the move is long.
    fn travel(&mut self, to: Pt, layer_z: i64) {
        let d = dist(Pt::new(self.x, self.y), to);
        if d < 50.0 {
            return;
        }
        let long = d > self.s.retract_min_travel * 1000.0;
        if long {
            self.retract();
            if self.s.z_hop > 0.0 {
                self.set_feed(self.s.z_hop_speed * 60.0);
                self.rapid_z(layer_z + (self.s.z_hop * 1000.0).round() as i64);
            }
        }
        self.set_feed(self.s.travel_speed * 60.0);
        self.rapid_xy(to);
        if long && self.s.z_hop > 0.0 {
            self.set_feed(self.s.z_hop_speed * 60.0);
            self.rapid_z(layer_z);
        }
    }

    fn note_point(&mut self, p: Pt) {
        self.bbox[0] = self.bbox[0].min(p.x);
        self.bbox[1] = self.bbox[1].min(p.y);
        self.bbox[3] = self.bbox[3].max(p.x);
        self.bbox[4] = self.bbox[4].max(p.y);
    }
}

pub fn emit(plan: &Plan, s: &Settings) -> Result<Emitted, String> {
    if plan.layers.is_empty() {
        return Err("aucune couche à imprimer".into());
    }
    let first = &plan.layers[0];
    let first_z = first.z;
    let first_pt = first.paths.first().and_then(|p| p.pts.first()).copied().ok_or("la première couche est vide")?;
    let mut em = Em {
        s,
        out: Vec::new(),
        t: 0.0,
        x: 0,
        y: 0,
        z: 0,
        feed: 0,
        fan: 0,
        retracted: true, // the printer starts with the filament retracted
        e_carry: 0.0,
        filament_um: 0.0,
        support_um: 0.0,
        bbox: [i64::MAX, i64::MAX, first_z, i64::MIN, i64::MIN, first_z],
    };

    // ---- prologue (same sequence as Z-Suite)
    em.push(Command::SetPosition(Move { e: Some(0), ..Default::default() }));
    em.push(Command::BedTemp(s.bed_temp));
    em.push(Command::NozzleTemp { tool: 0, temp: s.nozzle_temp });
    em.push(Command::Other { kind: 20, payload: Vec::new() });
    em.push(Command::G29 { a: first.thickness_um() as i32, b: plan.raft_top.max(first.thickness_um()) as i32, c: 0, d: 0 });
    em.push(Command::Home(Axes(Axes::X | Axes::Y)));
    em.push(Command::Home(Axes(Axes::Z)));
    em.push(Command::SetPosition(Move { x: Some(0), y: Some(0), z: Some(0), ..Default::default() }));
    em.feed = 180;
    em.push(Command::Feedrate(180));
    em.push(Command::Move(Move { role: UNKNOWN, z: Some(15_000), ..Default::default() }));
    em.z = 15_000;
    em.push(Command::M900 { k: 0.0, tool: 0 });
    em.push(Command::M900 { k: 0.0, tool: 1 });
    em.push(Command::M70(1));
    em.push(Command::ChamberTemp(first.marker));
    em.push(Command::SelectTool(0));
    em.push(Command::M111(3));
    em.feed = 0;
    em.set_feed(s.travel_speed * 60.0);
    em.rapid_xy(first_pt);
    em.rapid_z(first_z);
    em.push(Command::Progress(0));

    let mut progress_at: Vec<usize> = vec![em.out.len() - 1]; // index of Progress(0)
    let _ = &mut progress_at;

    // ---- layers
    for (li, layer) in plan.layers.iter().enumerate() {
        let lz = layer.z;
        if li > 0 {
            let start = layer.paths[0].pts[0];
            em.push(Command::SetPosition(Move { e: Some(0), ..Default::default() }));
            if li == 3 {
                em.push(Command::M111(0));
            }
            if layer.marker == 0 && plan.raft_layers > 0 {
                em.push(Command::M70(101));
            }
            em.push(Command::ChamberTemp(layer.marker));
            em.retract();
            em.set_feed(s.z_hop_speed * 60.0);
            em.rapid_z(lz + (s.z_hop * 1000.0).round() as i64);
            em.set_feed(s.travel_speed * 60.0);
            em.rapid_xy(start);
            em.set_feed(s.z_hop_speed * 60.0);
            em.rapid_z(lz);
            em.bbox[5] = em.bbox[5].max(lz);
        }
        for path in &layer.paths {
            let start = path.pts[0];
            em.travel(start, lz);
            em.unretract();
            if let Some(f) = path.fan {
                em.set_fan(f);
            }
            em.set_feed(path.speed * 60.0);
            em.note_point(start);
            for w in 1..path.pts.len() {
                let (a, b) = (path.pts[w - 1], path.pts[w]);
                let len = dist(a, b);
                if len < 1.0 {
                    continue;
                }
                let fil_mm = s.filament_for(len / 1000.0, path.width, layer.thickness, path.flow);
                let exact = fil_mm * 1000.0 + em.e_carry;
                let e = exact.round();
                em.e_carry = exact - e;
                em.filament_um += e;
                if is_support(path.role) {
                    em.support_um += e;
                }
                em.move_time(len);
                em.out.push((
                    Command::Move(Move {
                        role: path.role,
                        x: (b.x != em.x).then_some(b.x as i32),
                        y: (b.y != em.y).then_some(b.y as i32),
                        e: Some(e as i32),
                        ..Default::default()
                    })
                    .to_record(),
                    em.t,
                ));
                em.x = b.x;
                em.y = b.y;
                em.note_point(b);
            }
        }
    }

    // ---- epilogue
    em.set_feed(s.retraction_speed * 60.0);
    em.retract();
    let lift = ((em.z + 10_000).min((s.max_z * 1000.0) as i64)).max(em.z);
    em.push(Command::Move(Move { role: JUMP, z: Some(lift as i32), ..Default::default() }));
    em.push(Command::NozzleTemp { tool: 0, temp: 0 });
    em.push(Command::Home(Axes(Axes::X | Axes::Y)));
    em.push(Command::Progress(100));
    em.push(Command::EndProgram);

    let (records, usage) = finish(&em.out, em.t);

    Ok(Emitted {
        records,
        print_time_s: em.t,
        filament_mm: em.filament_um / 1000.0,
        support_mm: em.support_um / 1000.0,
        bbox: em.bbox,
        progress_usage: usage,
    })
}

trait LayerExt {
    fn thickness_um(&self) -> i64;
}
impl LayerExt for crate::toolpath::Layer {
    fn thickness_um(&self) -> i64 {
        (self.thickness * 1000.0).round() as i64
    }
}

/// Inserts `Progress` marks by elapsed time (like Z-Suite) and builds the cumulative usage table.
/// `out` holds each record with the cumulative time after it; the last record must be `Progress(100)`.
pub(crate) fn finish(out: &[(Record, f64)], t_total: f64) -> (Vec<Record>, Vec<(f64, f64)>) {
    let total_t = t_total.max(1e-6);
    let mut records: Vec<Record> = Vec::with_capacity(out.len() + 110);
    let mut next_mark = 1u8;
    let mut fil_run = 0.0;
    let mut sup_run = 0.0;
    let mut usage: Vec<(f64, f64)> = vec![(0.0, 0.0)];
    let mut started = false; // marks 1..99 only after the stream's own Progress(0)
    for (rec, t) in out.iter() {
        if rec.kind == 1 {
            if let Command::Move(m) = rec.decode() {
                if let Some(e) = m.e {
                    if e > 0 && m.role != UNRETRACT {
                        fil_run += e as f64 / 1000.0;
                        if is_support(m.role) {
                            sup_run += e as f64 / 1000.0;
                        }
                    }
                }
            }
        }
        let is_final = matches!(rec.decode(), Command::Progress(100));
        while started && !is_final && next_mark < 100 && *t / total_t * 100.0 >= next_mark as f64 {
            records.push(Command::Progress(next_mark).to_record());
            usage.push((fil_run, sup_run));
            next_mark += 1;
        }
        if is_final {
            while next_mark < 100 {
                records.push(Command::Progress(next_mark).to_record());
                usage.push((fil_run, sup_run));
                next_mark += 1;
            }
            usage.push((fil_run, sup_run));
        }
        records.push(rec.clone());
        if matches!(rec.decode(), Command::Progress(0)) {
            started = true; // marks 1..99 follow the stream's own Progress(0)
        }
    }
    while usage.len() < 101 {
        usage.push((fil_run, sup_run));
    }
    (records, usage)
}
