//! G-code (Marlin flavour, as written by PrusaSlicer, SuperSlicer, OrcaSlicer, Cura, Simplify3D,
//! IdeaMaker, Slic3r, KISSlicer...) -> Zortrax `ZCodeData` stream and `.zcodex2` container.
//!
//! The slicer's own start/end code is not copied: homing, bed levelling and shutdown follow Z-Suite's
//! sequence. Everything between the first extruding move and the last move is converted as is.

use zcodex2::{Axes, Command, Move, Rapid, Record};

use crate::emit::{finish, is_support, Emitted, JUMP, RETRACT, UNKNOWN, UNRETRACT};
use crate::export::{package, PackageInfo, Sliced};
use crate::geom::{dist, Pt};
use crate::settings::Settings;
use crate::toolpath::{role, Layer, Path};

/// Result of reading a G-code file.
#[derive(Debug)]
pub struct Imported {
    pub emitted: Emitted,
    /// Toolpaths per layer, for preview.
    pub layers: Vec<Layer>,
    pub nozzle_temp: i32,
    pub bed_temp: i32,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
struct Mv {
    from: [f64; 3],
    to: [f64; 3],
    e: f64,
    feed: f64,
    role: u8,
    line: usize,
}

enum Ev {
    Move(Mv),
    Nozzle(i32),
    Bed(i32),
    Fan(u8),
    Dwell(f64),
}

/// Maps a slicer feature comment ("TYPE:External perimeter", "FEATURE: Outer wall", "feature outer perimeter"...) to a role.
fn role_from_comment(c: &str) -> Option<u8> {
    let c = c.trim().to_ascii_lowercase();
    let text = if let Some(r) = c.strip_prefix("type:") {
        r
    } else if let Some(r) = c.strip_prefix("feature:") {
        r
    } else if let Some(r) = c.strip_prefix("feature ") {
        r
    } else {
        return None;
    };
    let t = text.trim();
    let has = |k: &[&str]| k.iter().any(|x| t.contains(x));
    Some(if has(&["support material interface", "support-interface", "support interface"]) {
        role::INTERFACE_INFILL
    } else if has(&["support"]) {
        role::SUPPORT_INFILL
    } else if has(&["external perimeter", "wall-outer", "outer wall", "outer perimeter", "overhang perimeter", "overhang wall"]) {
        role::OUTER
    } else if has(&["perimeter", "wall-inner", "inner wall"]) {
        role::INNER
    } else if has(&["bridge"]) {
        role::BRIDGE
    } else if has(&["top solid", "top surface", "ironing"]) {
        role::TOP
    } else if has(&["solid infill", "skin", "bottom surface", "solid layer", "internal solid"]) {
        role::BOTTOM
    } else if has(&["skirt", "brim"]) {
        role::BRIM
    } else if has(&["raft"]) {
        role::RAFT
    } else {
        role::INFILL
    })
}

struct Words {
    cmd: String,
    args: Vec<(char, f64)>,
}

impl Words {
    fn get(&self, l: char) -> Option<f64> {
        self.args.iter().find(|a| a.0 == l).map(|a| a.1)
    }
}

/// Splits one line into a command and its numeric arguments; returns the comment text too.
fn tokenize(line: &str) -> (Option<Words>, String) {
    let (code, comment) = match line.find(';') {
        Some(i) => (&line[..i], line[i + 1..].to_string()),
        None => (line, String::new()),
    };
    // Marlin accepts parenthesised comments too
    let code: String = {
        let mut out = String::new();
        let mut depth = 0;
        for ch in code.chars() {
            match ch {
                '(' => depth += 1,
                ')' => depth = (depth - 1).max(0),
                _ if depth == 0 => out.push(ch),
                _ => {}
            }
        }
        out
    };
    let code = code.trim();
    if code.is_empty() {
        return (None, comment);
    }
    let b: Vec<char> = code.chars().collect();
    let mut i = 0;
    let mut words = Vec::new();
    while i < b.len() {
        if b[i].is_whitespace() {
            i += 1;
            continue;
        }
        let letter = b[i].to_ascii_uppercase();
        i += 1;
        let start = i;
        while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], '.' | '-' | '+')) {
            i += 1;
        }
        let num: String = b[start..i].iter().collect();
        words.push((letter, num));
    }
    let Some((l0, n0)) = words.first().cloned() else { return (None, comment) };
    let cmd = format!("{l0}{}", n0.trim_start_matches('0').replace("+", ""));
    let cmd = if cmd.ends_with(char::is_alphabetic) || cmd.len() == 1 { format!("{l0}{n0}") } else { cmd };
    let cmd = if n0 == "0" || n0.chars().all(|c| c == '0') { format!("{l0}0") } else { cmd };
    let args = words[1..].iter().filter_map(|(l, n)| n.parse::<f64>().ok().map(|v| (*l, v)).or(if n.is_empty() { Some((*l, f64::NAN)) } else { None })).collect();
    (Some(Words { cmd, args }), comment)
}

struct Parse {
    events: Vec<Ev>,
    warnings: Vec<String>,
}

fn parse_events(text: &str, s: &Settings) -> Result<Parse, String> {
    let mut pos = [0.0f64; 3];
    let mut e_pos = 0.0f64;
    let mut abs_xyz = true;
    let mut abs_e = true;
    let mut inch = false;
    let mut feed = 1500.0f64;
    let mut flow = 1.0f64;
    let mut cur_role = role::INFILL;
    let mut events = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut unsupported: Vec<String> = Vec::new();
    let note = |list: &mut Vec<String>, what: &str| {
        if !list.iter().any(|w| w == what) {
            list.push(what.to_string());
        }
    };
    let k = |v: f64, inch: bool| if inch { v * 25.4 } else { v };

    for (ln, line) in text.lines().enumerate() {
        let (words, comment) = tokenize(line);
        if let Some(r) = role_from_comment(&comment) {
            cur_role = r;
        }
        let Some(w) = words else { continue };
        let up = w.cmd.as_str();
        match up {
            "G0" | "G1" => {
                if let Some(f) = w.get('F') {
                    if f.is_finite() && f > 0.0 {
                        feed = k(f, inch);
                    }
                }
                let target = |axis: char, cur: f64| match w.get(axis) {
                    Some(v) if v.is_finite() => {
                        let v = k(v, inch);
                        if abs_xyz { v } else { cur + v }
                    }
                    _ => cur,
                };
                let to = [target('X', pos[0]), target('Y', pos[1]), target('Z', pos[2])];
                let de = match w.get('E') {
                    Some(v) if v.is_finite() => {
                        let v = k(v, inch);
                        let d = if abs_e { v - e_pos } else { v };
                        e_pos = if abs_e { v } else { e_pos + v };
                        d * flow
                    }
                    _ => 0.0,
                };
                if to != pos || de != 0.0 {
                    events.push(Ev::Move(Mv { from: pos, to, e: de, feed, role: cur_role, line: ln + 1 }));
                }
                pos = to;
            }
            "G2" | "G3" => {
                if let Some(f) = w.get('F') {
                    if f.is_finite() && f > 0.0 {
                        feed = k(f, inch);
                    }
                }
                let cw = up == "G2";
                let target = |axis: char, cur: f64| match w.get(axis) {
                    Some(v) if v.is_finite() => {
                        let v = k(v, inch);
                        if abs_xyz { v } else { cur + v }
                    }
                    _ => cur,
                };
                let to = [target('X', pos[0]), target('Y', pos[1]), target('Z', pos[2])];
                let (cx, cy) = if let (Some(i), Some(j)) = (w.get('I'), w.get('J')) {
                    (pos[0] + k(i, inch), pos[1] + k(j, inch))
                } else if let Some(r) = w.get('R') {
                    // radius form: pick the centre on the correct side
                    let r = k(r, inch);
                    let (dx, dy) = (to[0] - pos[0], to[1] - pos[1]);
                    let d = (dx * dx + dy * dy).sqrt().max(1e-9);
                    let h = (r * r - d * d / 4.0).max(0.0).sqrt() * if (r < 0.0) != cw { 1.0 } else { -1.0 };
                    (pos[0] + dx / 2.0 - dy / d * h, pos[1] + dy / 2.0 + dx / d * h)
                } else {
                    note(&mut warnings, "arc sans centre ignoré");
                    pos = to;
                    continue;
                };
                let de_total = match w.get('E') {
                    Some(v) if v.is_finite() => {
                        let v = k(v, inch);
                        let d = if abs_e { v - e_pos } else { v };
                        e_pos = if abs_e { v } else { e_pos + v };
                        d * flow
                    }
                    _ => 0.0,
                };
                let r = ((pos[0] - cx).powi(2) + (pos[1] - cy).powi(2)).sqrt();
                let a0 = (pos[1] - cy).atan2(pos[0] - cx);
                let mut a1 = (to[1] - cy).atan2(to[0] - cx);
                if cw {
                    if a1 >= a0 { a1 -= std::f64::consts::TAU }
                } else if a1 <= a0 {
                    a1 += std::f64::consts::TAU;
                }
                let arc_len = (a1 - a0).abs() * r;
                let segs = ((arc_len / 0.5).ceil() as usize).clamp(1, 2000);
                let mut prev = pos;
                for sgm in 1..=segs {
                    let t = sgm as f64 / segs as f64;
                    let a = a0 + (a1 - a0) * t;
                    let p = if sgm == segs { to } else { [cx + r * a.cos(), cy + r * a.sin(), pos[2] + (to[2] - pos[2]) * t] };
                    events.push(Ev::Move(Mv { from: prev, to: p, e: de_total / segs as f64, feed, role: cur_role, line: ln + 1 }));
                    prev = p;
                }
                pos = to;
            }
            "G10" if w.args.is_empty() => events.push(Ev::Move(Mv { from: pos, to: pos, e: -s.retraction_distance, feed: s.retraction_speed * 60.0, role: cur_role, line: ln + 1 })),
            "G11" => events.push(Ev::Move(Mv { from: pos, to: pos, e: s.retraction_distance, feed: s.retraction_speed * 60.0, role: cur_role, line: ln + 1 })),
            "G20" => inch = true,
            "G21" => inch = false,
            "G90" => abs_xyz = true,
            "G91" => abs_xyz = false,
            "M82" => abs_e = true,
            "M83" => abs_e = false,
            "G92" => {
                if let Some(v) = w.get('E') {
                    if v.is_finite() {
                        e_pos = k(v, inch);
                    }
                }
                for (i, a) in ['X', 'Y', 'Z'].iter().enumerate() {
                    if let Some(v) = w.get(*a) {
                        if v.is_finite() {
                            pos[i] = k(v, inch);
                        }
                    }
                }
            }
            "G4" => {
                let ms = w.get('P').filter(|v| v.is_finite()).map(|v| v / 1000.0).or(w.get('S').filter(|v| v.is_finite()));
                if let Some(sec) = ms {
                    events.push(Ev::Dwell(sec));
                }
            }
            "M104" | "M109" => {
                if let Some(v) = w.get('S').or(w.get('R')) {
                    if v.is_finite() {
                        events.push(Ev::Nozzle(v.round() as i32));
                    }
                }
            }
            "M140" | "M190" => {
                if let Some(v) = w.get('S').or(w.get('R')) {
                    if v.is_finite() {
                        events.push(Ev::Bed(v.round() as i32));
                    }
                }
            }
            "M106" => {
                let v = w.get('S').filter(|v| v.is_finite()).unwrap_or(255.0);
                events.push(Ev::Fan(v.round().clamp(0.0, 255.0) as u8));
            }
            "M107" => events.push(Ev::Fan(0)),
            "M221" => {
                if let Some(v) = w.get('S').filter(|v| v.is_finite()) {
                    flow = (v / 100.0).clamp(0.2, 3.0);
                }
            }
            "M220" => note(&mut warnings, "M220 (vitesse en %) ignoré : les vitesses du fichier sont reprises telles quelles"),
            "M600" | "M601" | "M25" | "M0" | "M1" => note(&mut warnings, "pause / changement de filament (M600, M25, M0…) ignoré"),
            "G28" | "G29" | "G30" | "G32" | "M84" | "M18" | "M17" | "M73" | "M117" | "M118" | "M201" | "M202" | "M203" | "M204" | "M205" | "M207" | "M208" | "M209"
            | "M200" | "M290" | "M300" | "M301" | "M302" | "M303" | "M412" | "M420" | "M500" | "M501" | "M502" | "M503" | "M900" | "M105" | "M115" | "M114" | "M155"
            | "M75" | "M76" | "M77" | "M81" | "M80" | "M110" | "M111" | "M113" | "M42" | "M119" | "M122" | "M862" | "M555" | "M486" | "M572" | "M575" | "T0" | "T1" | "G54" => {}
            other => {
                if matches!(other.chars().next(), Some('G') | Some('M') | Some('T')) {
                    note(&mut unsupported, other);
                }
            }
        }
    }
    if !unsupported.is_empty() {
        warnings.push(format!("commandes ignorées : {}", unsupported.join(", ")));
    }
    Ok(Parse { events, warnings })
}

fn um(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// Reads G-code and builds the Z-Suite command stream. `s` supplies the printer limits, the material and
/// the fallback temperatures (the file's own M104/M109/M140/M190 values win).
#[allow(unused_assignments)]
pub fn parse_gcode(text: &str, s: &Settings) -> Result<Imported, String> {
    let Parse { events, mut warnings } = parse_events(text, s)?;

    // first extruding move
    let is_extrusion = |m: &Mv| m.e > 0.0 && ((m.to[0] - m.from[0]).abs() > 1e-6 || (m.to[1] - m.from[1]).abs() > 1e-6);
    let first = events
        .iter()
        .position(|e| matches!(e, Ev::Move(m) if is_extrusion(m)))
        .ok_or("aucune extrusion trouvée dans ce G-code")?;
    let Ev::Move(first_mv) = &events[first] else { unreachable!() };
    let first_z = first_mv.to[2];
    if first_z <= 0.0 {
        return Err("la première extrusion est à Z ≤ 0 : G-code invalide".into());
    }

    // temperatures
    let nozzle_temp = events.iter().find_map(|e| if let Ev::Nozzle(t) = e { (*t > 0).then_some(*t) } else { None }).unwrap_or(s.nozzle_temp);
    let bed_temp = events.iter().find_map(|e| if let Ev::Bed(t) = e { (*t > 0).then_some(*t) } else { None }).unwrap_or(s.bed_temp);
    if !(150..=300).contains(&nozzle_temp) {
        return Err(format!("température de buse {nozzle_temp} °C hors de 150–300 °C"));
    }
    if !(0..=110).contains(&bed_temp) {
        return Err(format!("température de plateau {bed_temp} °C hors de 0–110 °C"));
    }
    for e in &events {
        if let Ev::Nozzle(t) = e {
            if *t != 0 && !(150..=300).contains(t) {
                return Err(format!("température de buse {t} °C hors de 150–300 °C"));
            }
        }
    }

    // bounds check on everything we will print or travel to
    let max_z = s.max_z + 0.01;
    let first_start = Mv { to: first_mv.from, ..*first_mv };
    for e in std::iter::once(&Ev::Move(first_start)).chain(events[first..].iter()) {
        if let Ev::Move(m) = e {
            if m.to[0] < -0.5 || m.to[1] < -0.5 || m.to[0] > s.bed[0] + 0.5 || m.to[1] > s.bed[1] + 0.5 {
                return Err(format!("ligne {} : déplacement hors du plateau (X {:.1}, Y {:.1} pour {}×{} mm)", m.line, m.to[0], m.to[1], s.bed[0], s.bed[1]));
            }
            if m.to[2] > max_z || m.to[2] < -0.5 {
                return Err(format!("ligne {} : Z = {:.2} mm hors de 0–{} mm", m.line, m.to[2], s.max_z));
            }
            if m.feed > 18_000.0 {
                warnings.push(format!("ligne {} : vitesse {:.0} mm/min limitée à 18000", m.line, m.feed));
            }
        }
    }
    warnings.dedup();

    // layer index of each extruding move (new layer when Z rises by >= 0.04 mm)
    let mut layer_of: Vec<Option<usize>> = vec![None; events.len()];
    let mut layer_z: Vec<f64> = Vec::new();
    for (i, e) in events.iter().enumerate().skip(first) {
        if let Ev::Move(m) = e {
            if is_extrusion(m) {
                if layer_z.last().map_or(true, |&lz| m.to[2] >= lz + 0.04) {
                    layer_z.push(m.to[2]);
                }
                layer_of[i] = Some(layer_z.len() - 1);
            }
        }
    }
    // for every event: index of the layer of the next extruding move
    let mut next_layer: Vec<usize> = vec![0; events.len() + 1];
    let mut cur = layer_z.len().saturating_sub(1);
    for i in (0..events.len()).rev() {
        if let Some(l) = layer_of[i] {
            cur = l;
        }
        next_layer[i] = cur;
    }

    // ---- build the stream
    let mut out: Vec<(Record, f64)> = Vec::with_capacity(events.len() + 64);
    let mut t = 0.0f64;
    let push = |out: &mut Vec<(Record, f64)>, t: f64, c: Command| out.push((c.to_record(), t));

    let first_z_um = um(first_z);
    push(&mut out, t, Command::SetPosition(Move { e: Some(0), ..Default::default() }));
    push(&mut out, t, Command::BedTemp(bed_temp));
    push(&mut out, t, Command::NozzleTemp { tool: 0, temp: nozzle_temp });
    push(&mut out, t, Command::Other { kind: 20, payload: Vec::new() });
    push(&mut out, t, Command::G29 { a: first_z_um as i32, b: first_z_um as i32, c: 0, d: 0 });
    push(&mut out, t, Command::Home(Axes(Axes::X | Axes::Y)));
    push(&mut out, t, Command::Home(Axes(Axes::Z)));
    push(&mut out, t, Command::SetPosition(Move { x: Some(0), y: Some(0), z: Some(0), ..Default::default() }));
    push(&mut out, t, Command::Feedrate(180));
    push(&mut out, t, Command::Move(Move { role: UNKNOWN, z: Some(15_000), ..Default::default() }));
    push(&mut out, t, Command::M900 { k: 0.0, tool: 0 });
    push(&mut out, t, Command::M900 { k: 0.0, tool: 1 });
    push(&mut out, t, Command::M70(1));
    push(&mut out, t, Command::ChamberTemp(0));
    push(&mut out, t, Command::SelectTool(0));
    push(&mut out, t, Command::M111(3));

    let mut st_feed: i32 = 0;
    let (mut x, mut y, mut z) = (0i64, 0i64, 15_000i64);
    let mut fan: u8 = 0;
    let mut e_carry = 0.0f64;
    let mut retracted = true;
    let mut cur_nozzle = nozzle_temp;
    let mut cur_bed = bed_temp;
    let mut filament_um = 0.0f64;
    let mut support_um = 0.0f64;
    let mut bbox = [i64::MAX, i64::MAX, first_z_um, i64::MIN, i64::MIN, first_z_um];
    let mut printed_layers = 0usize;

    macro_rules! feed {
        ($f:expr) => {{
            let f = ($f).round().clamp(60.0, 18_000.0) as i32;
            if f != st_feed {
                st_feed = f;
                push(&mut out, t, Command::Feedrate(f));
            }
        }};
    }

    // start at the first extruding move's origin
    let start = Pt::new(um(first_mv.from[0]), um(first_mv.from[1]));
    feed!(s.travel_speed * 60.0);
    {
        let d = dist(Pt::new(x, y), start);
        t += d / 1000.0 / (st_feed as f64 / 60.0);
        push(&mut out, t, Command::Rapid(Rapid { role: JUMP, x: Some(start.x as i32), y: Some(start.y as i32), z: None }));
        x = start.x;
        y = start.y;
        t += (z - first_z_um).abs() as f64 / 1000.0 / (st_feed as f64 / 60.0);
        push(&mut out, t, Command::Rapid(Rapid { role: JUMP, x: None, y: None, z: Some(first_z_um as i32) }));
        z = first_z_um;
    }
    push(&mut out, t, Command::Progress(0));
    // the printer starts with the filament retracted (Z-Suite's first move pushes it back); the slicer's own
    // start-up priming was skipped, so do it here
    if s.retraction_distance > 0.0 {
        feed!(s.retraction_speed * 60.0);
        t += s.retraction_distance / s.retraction_speed.max(1.0);
        push(&mut out, t, Command::Move(Move { role: UNRETRACT, e: Some(((s.retraction_distance + s.retraction_extra) * 1000.0).round() as i32), ..Default::default() }));
    }
    retracted = false;

    let mut view: Vec<Layer> = Vec::new();
    let mut path_pts: Vec<Pt> = Vec::new();
    let mut path_role = role::INFILL;
    let mut cur_layer: usize = 0;
    let mut view_layer = Layer { marker: 0, z: first_z_um, thickness: first_z, paths: Vec::new(), is_raft: false };
    let flush = |view_layer: &mut Layer, pts: &mut Vec<Pt>, r: u8| {
        if pts.len() >= 2 {
            view_layer.paths.push(Path { pts: std::mem::take(pts), closed: false, role: r, speed: 0.0, width: s.nozzle_diameter, flow: 100.0, fan: None });
        } else {
            pts.clear();
        }
    };

    for (i, ev) in events.iter().enumerate().skip(first) {
        match ev {
            Ev::Nozzle(v) => {
                if *v != cur_nozzle {
                    cur_nozzle = *v;
                    push(&mut out, t, Command::NozzleTemp { tool: 0, temp: *v });
                }
            }
            Ev::Bed(v) => {
                if *v != cur_bed {
                    cur_bed = *v;
                    push(&mut out, t, Command::BedTemp(*v));
                }
            }
            Ev::Fan(v) => {
                if *v != fan {
                    fan = *v;
                    push(&mut out, t, Command::Move(Move { role: UNKNOWN, fan2: Some(*v as i32), ..Default::default() }));
                }
            }
            Ev::Dwell(sec) => {
                t += sec;
                push(&mut out, t, Command::Dwell((sec * 1000.0).round() as u32));
            }
            Ev::Move(m) => {
                let (tx, ty, tz) = (um(m.to[0]), um(m.to[1]), um(m.to[2]));
                let xy_move = tx != x || ty != y;
                let z_move = tz != z;
                let extruding = m.e > 0.0 && xy_move;

                // a Z change that leads to the next layer carries the layer marker
                if z_move && !xy_move && m.e == 0.0 && next_layer[i] > cur_layer {
                    cur_layer = next_layer[i];
                    printed_layers += 1;
                    flush(&mut view_layer, &mut path_pts, path_role);
                    let done = std::mem::replace(
                        &mut view_layer,
                        Layer { marker: cur_layer as i32, z: um(layer_z[cur_layer]), thickness: layer_z[cur_layer] - layer_z[cur_layer - 1], paths: Vec::new(), is_raft: false },
                    );
                    view.push(done);
                    push(&mut out, t, Command::SetPosition(Move { e: Some(0), ..Default::default() }));
                    if printed_layers == 3 {
                        push(&mut out, t, Command::M111(0));
                    }
                    push(&mut out, t, Command::ChamberTemp(cur_layer as i32));
                }

                let e_um_exact = m.e * 1000.0 + e_carry;
                let e_um = e_um_exact.round();
                e_carry = e_um_exact - e_um;
                let len = dist(Pt::new(x, y), Pt::new(tx, ty));
                let seconds = |len_um: f64, f: i32| len_um / 1000.0 / (f.max(1) as f64 / 60.0);

                if m.e < 0.0 {
                    flush(&mut view_layer, &mut path_pts, path_role);
                    feed!(m.feed);
                    t += (m.e.abs()) / (st_feed as f64 / 60.0);
                    push(&mut out, t, Command::Move(Move { role: RETRACT, e: Some(e_um as i32), ..Default::default() }));
                    retracted = true;
                } else if m.e > 0.0 && !xy_move {
                    feed!(m.feed);
                    t += m.e / (st_feed as f64 / 60.0);
                    push(&mut out, t, Command::Move(Move { role: UNRETRACT, e: Some(e_um as i32), ..Default::default() }));
                    retracted = false;
                } else if extruding {
                    if retracted {
                        // the slicer may have primed before: make sure the filament is pushed back
                        retracted = false;
                    }
                    feed!(m.feed);
                    t += seconds(len, st_feed);
                    let role_ = m.role;
                    let e_pos = e_um as i32;
                    filament_um += e_um;
                    if is_support(role_) {
                        support_um += e_um;
                    }
                    push(
                        &mut out,
                        t,
                        Command::Move(Move {
                            role: role_,
                            x: (tx != x).then_some(tx as i32),
                            y: (ty != y).then_some(ty as i32),
                            z: z_move.then_some(tz as i32),
                            e: Some(e_pos),
                            ..Default::default()
                        }),
                    );
                    bbox[0] = bbox[0].min(x.min(tx));
                    bbox[1] = bbox[1].min(y.min(ty));
                    bbox[3] = bbox[3].max(x.max(tx));
                    bbox[4] = bbox[4].max(y.max(ty));
                    if path_pts.is_empty() || path_role != role_ {
                        flush(&mut view_layer, &mut path_pts, path_role);
                        path_role = role_;
                        path_pts.push(Pt::new(x, y));
                    }
                    path_pts.push(Pt::new(tx, ty));
                    x = tx;
                    y = ty;
                    z = tz;
                    continue;
                }

                // travel (or the XY / Z part left after a retract)
                if xy_move || z_move {
                    flush(&mut view_layer, &mut path_pts, path_role);
                    feed!(m.feed);
                    t += seconds(len, st_feed) + seconds((tz - z).abs() as f64, st_feed);
                    push(
                        &mut out,
                        t,
                        Command::Rapid(Rapid {
                            role: JUMP,
                            x: (tx != x).then_some(tx as i32),
                            y: (ty != y).then_some(ty as i32),
                            z: z_move.then_some(tz as i32),
                        }),
                    );
                    x = tx;
                    y = ty;
                    z = tz;
                }
            }
        }
    }
    flush(&mut view_layer, &mut path_pts, path_role);
    view.push(view_layer);

    // epilogue
    feed!(s.retraction_speed * 60.0);
    if !retracted && s.retraction_distance > 0.0 {
        t += s.retraction_distance / s.retraction_speed.max(1.0);
        push(&mut out, t, Command::Move(Move { role: RETRACT, e: Some(-((s.retraction_distance * 1000.0).round() as i32)), ..Default::default() }));
    }
    let lift = ((z + 10_000).min((s.max_z * 1000.0) as i64)).max(z);
    push(&mut out, t, Command::Move(Move { role: JUMP, z: Some(lift as i32), ..Default::default() }));
    push(&mut out, t, Command::NozzleTemp { tool: 0, temp: 0 });
    push(&mut out, t, Command::Home(Axes(Axes::X | Axes::Y)));
    push(&mut out, t, Command::Progress(100));
    push(&mut out, t, Command::EndProgram);

    let layer_count = layer_z.len();
    bbox[5] = um(*layer_z.last().unwrap_or(&first_z));
    let (records, usage) = finish(&out, t);
    let emitted = Emitted {
        records,
        print_time_s: t,
        filament_mm: filament_um / 1000.0,
        support_mm: support_um / 1000.0,
        bbox,
        progress_usage: usage,
    };
    if layer_count == 0 {
        return Err("aucune couche détectée".into());
    }
    Ok(Imported { emitted, layers: view, nozzle_temp, bed_temp, warnings })
}

/// Converts G-code into a `.zcodex2` file. `preview_png` is stored as the preview image.
pub fn gcode_to_zcodex2(text: &str, s: &Settings, preview_png: Vec<u8>, configuration_data: Option<Vec<u8>>) -> Result<Sliced, String> {
    let imp = parse_gcode(text, s)?;
    let preview_png = if preview_png.is_empty() { crate::preview::top_view_png(&imp.layers, s.bed, 160) } else { preview_png };
    let mut s2 = s.clone();
    s2.nozzle_temp = imp.nozzle_temp;
    s2.bed_temp = imp.bed_temp;
    let first = imp.layers.first().map(|l| l.z).unwrap_or(260);
    let last = imp.layers.last().map(|l| l.z).unwrap_or(first);
    let info = PackageInfo {
        first_z_um: first,
        last_z_um: imp.emitted.bbox[5].max(last),
        min_layer: 0,
        max_layer: imp.layers.len() as i32 - 1,
        layer_count: imp.layers.len(),
    };
    package(&imp.emitted, &s2, &info, preview_png, configuration_data, imp.warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRUSA: &str = "\
; generated by PrusaSlicer 2.7
M107
M104 S210 ; set temp
M140 S60
M190 S60
M109 S210
G28 ; home
G90
M82
G92 E0
G1 Z0.3 F7200
G1 X50 Y50 F7200
G1 E1.5 F2400
;TYPE:External perimeter
G1 X70 Y50 E2.5 F1200
G1 X70 Y70 E3.5
G1 X50 Y70 E4.5
G1 X50 Y50 E5.5
G1 E3.5 F2400 ; retract
G1 Z0.8 F7200
;LAYER_CHANGE
G1 X60 Y60 F7200
G1 Z0.5 F7200
G1 E5.5 F2400
;TYPE:Internal infill
G1 X65 Y60 E5.8 F3000
M106 S255
;TYPE:Top solid infill
G1 X65 Y65 E6.1
G1 E4.1 F2400
G1 Z1.0
G1 X60 Y60 F7200
G1 Z0.8 F7200
G1 E6.1
G1 X61 Y60 E6.2 F1500
M104 S0
M84
";

    fn kinds(imp: &Imported) -> Vec<Command> {
        imp.emitted.records.iter().map(|r| r.decode()).collect()
    }

    #[test]
    fn prusa_style_file_converts_with_roles_layers_and_retraction() {
        let imp = parse_gcode(PRUSA, &Settings::default()).unwrap();
        assert_eq!((imp.nozzle_temp, imp.bed_temp), (210, 60));
        let cmds = kinds(&imp);
        assert!(cmds.iter().any(|c| matches!(c, Command::NozzleTemp { temp: 210, .. })));
        // roles: outer wall = 0, sparse infill = 3, top = 45
        let roles: Vec<u8> = cmds.iter().filter_map(|c| if let Command::Move(m) = c { Some(m.role) } else { None }).collect();
        assert!(roles.contains(&0) && roles.contains(&3) && roles.contains(&45));
        // retractions become role 253 with negative E, primes role 254
        assert!(cmds.iter().any(|c| matches!(c, Command::Move(m) if m.role == RETRACT && m.e == Some(-2000))));
        assert!(cmds.iter().any(|c| matches!(c, Command::Move(m) if m.role == UNRETRACT && m.e.unwrap_or(0) > 0)));
        // fan 255 appears once
        assert!(cmds.iter().any(|c| matches!(c, Command::Move(m) if m.role == UNKNOWN && m.fan2 == Some(255))));
        // layers: z 0.3 and 0.5/0.8...: at least two layers detected, markers present
        assert!(imp.layers.len() >= 2, "{}", imp.layers.len());
        assert!(cmds.iter().any(|c| matches!(c, Command::ChamberTemp(1))));
        assert!(matches!(cmds.last().unwrap(), Command::EndProgram));
        // slicer homing / M84 are not copied: exactly the two prologue homes + the final one
        assert_eq!(cmds.iter().filter(|c| matches!(c, Command::Home(_))).count(), 3);
        // filament: only positive extrusion counted (1+1+1+1 mm ... )
        assert!(imp.emitted.filament_mm > 4.0 && imp.emitted.filament_mm < 8.0, "{}", imp.emitted.filament_mm);
    }

    #[test]
    fn relative_extrusion_and_arcs_cura_style() {
        let g = "\
;FLAVOR:Marlin
M140 S50
M104 S200
M109 S200
G90
M83
G1 F1500 X100 Y100 Z0.2
;TYPE:WALL-OUTER
G1 X110 Y100 E0.4
G2 X100 Y110 I-10 J0 E1.2
;TYPE:SKIN
G1 X100 Y100 E0.4
";
        let imp = parse_gcode(g, &Settings::default()).unwrap();
        let cmds = kinds(&imp);
        // the clockwise quarter arc from (110,100) around (100,100) is split into many short moves
        let wall_moves = cmds.iter().filter(|c| matches!(c, Command::Move(m) if m.role == 0)).count();
        assert!(wall_moves > 10, "{wall_moves}");
        // E of the outer wall: the straight 0.4 mm plus the arc's 1.2 mm
        let wall_e: i32 = cmds.iter().filter_map(|c| if let Command::Move(m) = c { (m.role == 0).then(|| m.e.unwrap_or(0)) } else { None }).sum();
        assert!((wall_e - 1600).abs() <= 2, "{wall_e}");
        assert!(cmds.iter().any(|c| matches!(c, Command::Move(m) if m.role == 46)));
    }

    #[test]
    fn limits_are_enforced() {
        let s = Settings::default();
        let base = |extra: &str| format!("M104 S210\nG1 Z0.2 F3000\nG1 X10 Y10\n{extra}\nG1 X20 Y20 E1\n");
        assert!(parse_gcode(&base(""), &s).is_ok());
        assert!(parse_gcode(&base("G1 X500"), &s).unwrap_err().contains("hors du plateau"));
        assert!(parse_gcode(&base("G1 Z400"), &s).unwrap_err().contains("hors de"));
        assert!(parse_gcode(&format!("M104 S400\n{}", base("")), &s).unwrap_err().contains("400"));
        assert!(parse_gcode("G1 X1 Y1\nM104 S210\n", &s).unwrap_err().contains("aucune extrusion"));
    }

    #[test]
    fn firmware_retraction_and_dwell() {
        let g = "M104 S210\nG1 Z0.2 F3000\nG1 X10 Y10\nG1 X20 Y10 E1 F1200\nG10\nG4 P500\nG11\nG1 X30 Y10 E2\n";
        let imp = parse_gcode(g, &Settings::default()).unwrap();
        let cmds = kinds(&imp);
        assert!(cmds.iter().any(|c| matches!(c, Command::Move(m) if m.role == RETRACT && m.e == Some(-800))));
        assert!(cmds.iter().any(|c| matches!(c, Command::Dwell(500))));
        assert!(cmds.iter().any(|c| matches!(c, Command::Move(m) if m.role == UNRETRACT && m.e == Some(800))));
    }

    #[test]
    fn tokenizer_handles_compact_and_commented_code() {
        let (w, c) = tokenize("G1X10.5Y-3 E.4 ; hello");
        let w = w.unwrap();
        assert_eq!(w.cmd, "G1");
        assert_eq!(w.get('X'), Some(10.5));
        assert_eq!(w.get('Y'), Some(-3.0));
        assert_eq!(w.get('E'), Some(0.4));
        assert_eq!(c.trim(), "hello");
        let (w, _) = tokenize("G01 X1 (comment) Y2");
        assert_eq!(w.unwrap().cmd, "G1");
        assert!(tokenize("; only a comment").0.is_none());
    }

    #[test]
    fn container_is_valid() {
        let r = gcode_to_zcodex2(PRUSA, &Settings::default(), Vec::new(), None).unwrap();
        let back = zcodex2::Zcodex2::from_bytes(&r.file.to_bytes().unwrap()).unwrap();
        let recs = back.records().unwrap();
        assert_eq!(Some(recs.len() as u64), back.command_count());
        assert_eq!(back.metadata["Materials"][0]["Temperature"], 210);
        assert_eq!(back.metadata["HeatbedTemperature"], 60);
    }
}
