//! Layer planning: perimeters, skins, infill, bridges, supports, raft and brim as ordered paths.

use crate::geom::{self, contains, dist, mm, offset, pt, Polys, Pt};
use crate::settings::{InfillPattern, Seam, Settings, SupportPattern};
use rayon::prelude::*;
use crate::support::{self, SupportLayer};
use crate::slicer::{slice, Mesh};

/// Toolpath roles, numbered like Z-Suite's "mode" byte.
pub mod role {
    pub const OUTER: u8 = 0;
    pub const INNER: u8 = 2;
    pub const INFILL: u8 = 3;
    pub const SUPPORT_CONTOUR: u8 = 4;
    pub const SUPPORT_INFILL: u8 = 5;
    pub const INTERFACE_INFILL: u8 = 7;
    pub const RAFT: u8 = 10;
    pub const RAFT_BOTTOM: u8 = 11;
    pub const RAFT_GRID: u8 = 12;
    pub const RAFT_INTERFACE: u8 = 13;
    pub const FIRST_CONTOUR: u8 = 22;
    pub const FIRST_FILL: u8 = 23;
    pub const BRIDGE: u8 = 25;
    pub const BRIM: u8 = 34;
    pub const TOP: u8 = 45;
    pub const BOTTOM: u8 = 46;
}

#[derive(Clone, Debug)]
pub struct Path {
    pub pts: Vec<Pt>,
    pub closed: bool,
    pub role: u8,
    /// mm/s
    pub speed: f64,
    /// bead width in mm (used for the extrusion amount)
    pub width: f64,
    /// flow multiplier in percent
    pub flow: f64,
    /// fan override (percent) while printing this path
    pub fan: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct Layer {
    /// Z-Suite layer number: raft layers are negative, the first model layer is 0.
    pub marker: i32,
    /// Z of the top of the layer in µm.
    pub z: i64,
    /// mm
    pub thickness: f64,
    pub paths: Vec<Path>,
    pub is_raft: bool,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub layers: Vec<Layer>,
    pub model_layers: usize,
    pub raft_layers: usize,
    /// Z of the top of the raft in µm (0 without raft).
    pub raft_top: i64,
    pub warnings: Vec<String>,
}

/// Drops points closer than `min` µm to the previous kept one (keeps the first and the last point).
fn thin(pts: &[Pt], min: f64) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
    for (i, &p) in pts.iter().enumerate() {
        match out.last() {
            Some(&q) if dist(q, p) < min => {
                if i == pts.len() - 1 && out.len() > 1 {
                    *out.last_mut().unwrap() = p;
                }
            }
            _ => out.push(p),
        }
    }
    out
}

fn seg_len(p: &[Pt], closed: bool) -> f64 {
    let n = p.len();
    let mut l: f64 = (1..n).map(|i| dist(p[i - 1], p[i])).sum();
    if closed && n > 2 {
        l += dist(p[n - 1], p[0]);
    }
    l
}

// ---------------------------------------------------------------------------------- ordering

struct Orderer {
    cur: Pt,
    seam: Seam,
    counter: u64,
}

impl Orderer {
    fn rotate_loop(&mut self, p: &mut Vec<Pt>) {
        let idx = match self.seam {
            Seam::Nearest => (0..p.len()).min_by(|&a, &b| dist(p[a], self.cur).partial_cmp(&dist(p[b], self.cur)).unwrap()).unwrap_or(0),
            Seam::Rear => (0..p.len()).max_by_key(|&i| (p[i].y, -p[i].x)).unwrap_or(0),
            Seam::Random => {
                self.counter = self.counter.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (self.counter >> 33) as usize % p.len().max(1)
            }
        };
        p.rotate_left(idx);
    }

    /// Orders a group of paths by nearest neighbour, orienting open paths and starting loops at the seam.
    fn order(&mut self, mut items: Vec<Path>) -> Vec<Path> {
        let mut out = Vec::with_capacity(items.len());
        let nn = items.len() <= 1500;
        while !items.is_empty() {
            let pick = if nn {
                (0..items.len())
                    .map(|i| {
                        let p = &items[i];
                        let d = if p.closed {
                            p.pts.iter().map(|&q| dist(q, self.cur)).fold(f64::MAX, f64::min)
                        } else {
                            dist(p.pts[0], self.cur).min(dist(*p.pts.last().unwrap(), self.cur))
                        };
                        (i, d)
                    })
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                    .unwrap()
                    .0
            } else {
                0
            };
            let mut p = items.swap_remove(pick);
            if p.closed {
                self.rotate_loop(&mut p.pts);
                let first = p.pts[0];
                p.pts.push(first); // the loop is printed back to its start
            } else if dist(*p.pts.last().unwrap(), self.cur) < dist(p.pts[0], self.cur) {
                p.pts.reverse();
            }
            self.cur = *p.pts.last().unwrap();
            out.push(p);
        }
        out
    }


    /// Parallel lines over `region` as paths; neighbouring lines closer than 1.6 x `spacing` are chained
    /// into one zig-zag path when the connector stays inside the region.
    fn fill(&mut self, region: &Polys, angle: f64, spacing: f64, mk: &dyn Fn(Vec<Pt>) -> Path) -> Vec<Path> {
        let rows = geom::fill_lines(region, angle, spacing);
        let guard = offset(region, 60.0);
        let singles = self.order_rows(rows, mk);
        let mut out: Vec<Path> = Vec::with_capacity(singles.len());
        for p in singles {
            if let Some(last) = out.last_mut() {
                let a = *last.pts.last().unwrap();
                let b = p.pts[0];
                let d = dist(a, b);
                if d <= spacing * 1.6 && d > 0.0 && contains(&guard, ((a.x + b.x) / 2) as f64, ((a.y + b.y) / 2) as f64) {
                    last.pts.extend(p.pts);
                    continue;
                }
            }
            out.push(p);
        }
        out
    }

    /// Fill rows -> paths in zig-zag order.
    fn order_rows(&mut self, rows: Vec<Vec<[Pt; 2]>>, mk: &dyn Fn(Vec<Pt>) -> Path) -> Vec<Path> {
        let mut out = Vec::new();
        for mut row in rows {
            while !row.is_empty() {
                let i = (0..row.len())
                    .min_by(|&a, &b| {
                        let da = dist(row[a][0], self.cur).min(dist(row[a][1], self.cur));
                        let db = dist(row[b][0], self.cur).min(dist(row[b][1], self.cur));
                        da.partial_cmp(&db).unwrap()
                    })
                    .unwrap();
                let mut s = row.swap_remove(i);
                if dist(s[1], self.cur) < dist(s[0], self.cur) {
                    s.swap(0, 1);
                }
                self.cur = s[1];
                out.push(mk(vec![s[0], s[1]]));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------- helpers

fn spacing_um(width: f64, density_pct: f64) -> f64 {
    mm(width * 100.0 / density_pct.max(1.0)) as f64
}


/// Concentric loops of `region`, outermost first.
fn concentric(region: &Polys, pitch_um: f64) -> Vec<Vec<Pt>> {
    let mut out = Vec::new();
    let mut cur = offset(region, -pitch_um / 2.0);
    let mut guard = 0;
    while !cur.is_empty() && guard < 400 {
        for r in &cur {
            if r.len() >= 3 {
                out.push(r.clone());
            }
        }
        cur = offset(&cur, -pitch_um);
        guard += 1;
    }
    out
}

struct LayerData {
    z_top: i64,
    thickness: f64,
    region: Polys,
}

// ---------------------------------------------------------------------------------- planning

pub fn plan(mesh: &Mesh, s: &Settings) -> Result<Plan, String> {
    let clock = std::time::Instant::now();
    let tick = |what: &str| {
        if std::env::var("ZTIME").is_ok() {
            eprintln!("  [plan] {what}: {:.0?}", clock.elapsed());
        }
    };
    if mesh.triangles.is_empty() {
        return Err("le modèle est vide".into());
    }
    let problems = s.validate();
    if !problems.is_empty() {
        return Err(format!("réglages invalides : {}", problems.join(" ; ")));
    }
    let (lo, hi) = mesh.bounds();
    let mut warnings = Vec::new();
    if lo[0] < -0.01 || lo[1] < -0.01 || hi[0] > s.bed[0] + 0.01 || hi[1] > s.bed[1] + 0.01 {
        return Err(format!(
            "le modèle dépasse du plateau ({:.1}–{:.1} × {:.1}–{:.1} mm pour un plateau de {} × {} mm)",
            lo[0], hi[0], lo[1], hi[1], s.bed[0], s.bed[1]
        ));
    }
    let height = hi[2] - lo[2];
    if height < 0.05 {
        return Err("le modèle est trop plat (moins de 0,05 mm de haut)".into());
    }

    // ---- layer stack
    let raft_n = if s.raft { s.raft_layers as usize } else { 0 };
    let mut raft_tops = Vec::new();
    let mut z = 0.0;
    for k in 0..raft_n {
        z += if k == 0 { s.first_layer_height } else { s.raft_layer_height };
        raft_tops.push(z);
    }
    let raft_top = z;
    let t0 = (if s.raft { s.raft_gap } else { s.first_layer_height }).min(height);
    let rest = ((height - t0) / s.layer_height).round().max(0.0) as usize;
    let t_rest = if rest > 0 { (height - t0) / rest as f64 } else { 0.0 };
    let n = 1 + rest;
    let mut thick = vec![t0];
    thick.extend(std::iter::repeat(t_rest).take(rest));
    let mut bottoms = Vec::with_capacity(n);
    let mut acc = 0.0;
    for &t in &thick {
        bottoms.push(acc);
        acc += t;
    }
    let planes: Vec<f64> = (0..n).map(|i| lo[2] + bottoms[i] + thick[i] / 2.0).collect();
    let top_z_total = raft_top + height;
    if top_z_total > s.max_z {
        return Err(format!("hauteur totale {:.1} mm (radeau compris) supérieure à la hauteur max {} mm", top_z_total, s.max_z));
    }

    let mut regions = slice(mesh, &planes);
    let min_area = geom::mm(s.nozzle_diameter).pow(2) as f64 * 2.0;
    for r in &mut regions {
        *r = geom::remove_small(r, min_area);
    }
    let layers: Vec<LayerData> = (0..n)
        .map(|i| LayerData {
            z_top: mm(raft_top + bottoms[i] + thick[i]),
            thickness: thick[i],
            region: std::mem::take(&mut regions[i]),
        })
        .collect();
    if layers.iter().all(|l| l.region.is_empty()) {
        return Err("le découpage n'a produit aucune couche : le modèle est-il fermé (étanche) ?".into());
    }
    let empty_mid = layers.iter().skip(1).take(n.saturating_sub(2)).filter(|l| l.region.is_empty()).count();
    if empty_mid > 0 {
        warnings.push(format!("{empty_mid} couche(s) vide(s) au milieu du modèle : le maillage n'est probablement pas étanche"));
    }

    tick("tranches (cumulé)");
    // ---- supports (regions per layer)
    let supports = if s.support {
        let inputs: Vec<support::In> =
            (0..n).map(|i| support::In { region: &layers[i].region, thickness: layers[i].thickness, zc: bottoms[i] + thick[i] / 2.0 }).collect();
        let (sup, removed) = support::plan_supports_stats(&inputs, s);
        if removed > 2.0e6 {
            warnings.push(format!(
                "{:.0} mm² de supports en suspension ont été retirés (rien dessous pour les porter) : vérifie les zones en surplomb, ou active les supports « partout »",
                removed / 1.0e6
            ));
        }
        sup
    } else {
        vec![SupportLayer::default(); n]
    };

    tick("supports (cumulé)");
    // ---- walls and skins
    let wall_d: Vec<f64> = {
        let mut d = Vec::new();
        let mut cur = s.outer_width / 2.0;
        for k in 0..s.wall_count {
            if k == 1 {
                cur += (s.outer_width + s.inner_width) / 2.0;
            } else if k > 1 {
                cur += s.inner_width;
            }
            d.push(cur);
        }
        d
    };
    let last_w = if s.wall_count == 1 { s.outer_width } else { s.inner_width };
    let inner_edge = wall_d.last().copied().unwrap_or(0.0) + last_w / 2.0;

    let infill_area: Vec<Polys> = layers
        .iter()
        .map(|l| geom::remove_small(&offset(&l.region, -(mm(inner_edge - s.infill_overlap) as f64)), 4.0e6 * 0.05))
        .collect();

    let covered = |from: isize, count: u32, dir: isize| -> Polys {
        // intersection of the regions of the next `count` layers in direction `dir`
        let mut acc: Option<Polys> = None;
        for j in 1..=count as isize {
            let idx = from + dir * j;
            let r = if idx >= 0 && (idx as usize) < n { layers[idx as usize].region.clone() } else { Vec::new() };
            acc = Some(match acc {
                None => r,
                Some(a) => geom::inter(&a, &r),
            });
            if acc.as_ref().map_or(false, |a| a.is_empty()) {
                break;
            }
        }
        acc.unwrap_or_default()
    };

    let min_skin = s.min_skin_area * 1.0e6;
    let skin_r = mm(s.solid_width) as f64 / 2.0;

    tick("parois (cumulé)");
    // ---- assemble layers
    let mut out_layers: Vec<Layer> = Vec::new();
    let mut ord = Orderer { cur: pt(mm(s.bed[0] / 2.0), mm(s.bed[1] / 2.0)), seam: s.seam, counter: 12345 };
    // layers that end up empty are skipped: markers stay consecutive and the next printed layer's thickness
    // is the real Z step, so the extrusion amount stays right
    let mut last_z = mm(raft_top);
    let mut next_marker = 0i32;

    // everything we print must stay on the plate (0.6 mm in from each edge)
    let bed_rect: Polys = {
        let (m, bx, by) = (mm(0.6), mm(s.bed[0] - 0.6), mm(s.bed[1] - 0.6));
        vec![vec![pt(m, m), pt(bx, m), pt(bx, by), pt(m, by)]]
    };

    // raft first
    if raft_n > 0 {
        let mut foot: Polys = layers[0].region.clone();
        if s.support {
            foot = geom::union2(&foot, &supports[0].area);
        }
        let grown = offset(&foot, mm(s.raft_margin) as f64);
        let foot = geom::inter(&grown, &bed_rect);
        if geom::total_area(&foot) < geom::total_area(&grown) * 0.995 {
            warnings.push("le radeau a été réduit pour tenir sur le plateau".into());
        }
        for k in 0..raft_n {
            let t = if k == 0 { s.first_layer_height } else { s.raft_layer_height };
            let (r, speed, width, density) = if k == 0 {
                (role::RAFT, s.raft_base_speed, 0.8, 100.0)
            } else if k == raft_n - 1 {
                (role::RAFT_INTERFACE, s.raft_speed, 0.5, 100.0)
            } else if k == 1 {
                (role::RAFT_BOTTOM, s.raft_speed, 0.6, 100.0)
            } else {
                (role::RAFT_GRID, s.raft_speed, 0.6, 100.0)
            };
            let angle = 45.0 + 90.0 * (k % 2) as f64;
            let mk = |pts: Vec<Pt>| Path { pts, closed: false, role: r, speed, width, flow: s.raft_flow, fan: None };
            let mut paths = Vec::new();
            // outline
            let outline: Vec<Path> = offset(&foot, -(mm(width) as f64) / 2.0)
                .into_iter()
                .filter(|p| p.len() >= 3)
                .map(|pts| Path { pts, closed: true, role: r, speed, width, flow: s.raft_flow, fan: None })
                .collect();
            paths.extend(ord.order(outline));
            let inner = offset(&foot, -(mm(width) as f64));
            paths.extend(ord.fill(&inner, angle, spacing_um(width, density), &mk));
            for p in &mut paths {
                p.pts = thin(&p.pts, 80.0);
            }
            paths.retain(|p| p.pts.len() >= 2);
            out_layers.push(Layer { marker: k as i32 - raft_n as i32, z: mm(raft_tops[k]), thickness: t, paths, is_raft: true });
        }
    }

    // layers are independent: each one is generated on its own thread with its own ordering state
    let layer_paths: Vec<Vec<Path>> = (0..n)
        .into_par_iter()
        .map(|i| {
        let l = &layers[i];
        let first_on_plate = !s.raft && i == 0;
        let mut paths: Vec<Path> = Vec::new();
        let mut ord = Orderer { cur: pt(mm(s.bed[0] / 2.0), mm(s.bed[1] / 2.0)), seam: s.seam, counter: 12345 + 7919 * i as u64 };

        if !l.region.is_empty() {
            // brim (first layer on the plate only)
            if first_on_plate && s.brim_width > 0.0 {
                let loops = (s.brim_width / s.outer_width).ceil() as usize;
                let mut b = Vec::new();
                for j in (0..loops).rev() {
                    let d = s.brim_offset + (j as f64 + 0.5) * s.outer_width;
                    for p in geom::inter(&offset(&l.region, mm(d) as f64), &bed_rect) {
                        if p.len() >= 3 {
                            b.push(Path { pts: p, closed: true, role: role::BRIM, speed: s.first_layer_speed, width: s.outer_width, flow: s.first_layer_flow, fan: None });
                        }
                    }
                }
                paths.extend(ord.order(b));
            }

            // walls
            let (spd_o, spd_i, flow_w) = if first_on_plate {
                (s.first_layer_speed, s.first_layer_speed, s.first_layer_flow)
            } else {
                (s.outer_speed, s.inner_speed, 100.0)
            };
            let mut tiers: Vec<Vec<Path>> = Vec::new();
            for (k, &d) in wall_d.iter().enumerate() {
                let loops = geom::simplify(&offset(&l.region, -(mm(d) as f64)), 30.0);
                let tier: Vec<Path> = loops
                    .into_iter()
                    .filter(|p| p.len() >= 3 && geom::perimeter(p) > 1500.0)
                    .map(|pts| {
                        if k == 0 {
                            Path { pts, closed: true, role: if first_on_plate { role::FIRST_CONTOUR } else { role::OUTER }, speed: spd_o, width: s.outer_width, flow: flow_w, fan: None }
                        } else {
                            Path { pts, closed: true, role: if first_on_plate { role::FIRST_CONTOUR } else { role::INNER }, speed: spd_i, width: s.inner_width, flow: flow_w, fan: None }
                        }
                    })
                    .collect();
                tiers.push(tier);
            }
            if !s.outer_first {
                tiers.reverse();
            }
            for t in tiers {
                paths.extend(ord.order(t));
            }

            // skins
            let ia = &infill_area[i];
            let mut solid_top = if i + 1 >= n { ia.clone() } else { geom::diff(ia, &covered(i as isize, s.top_layers, 1)) };
            if s.top_layers == 0 {
                solid_top.clear();
            }
            let mut solid_bottom = if s.bottom_layers == 0 { Vec::new() } else { geom::diff(ia, &covered(i as isize, s.bottom_layers, -1)) };
            if first_on_plate {
                solid_bottom = ia.clone();
            }
            let clean = |p: Polys| geom::remove_small(&geom::open(&p, skin_r.min(200.0)), min_skin);
            solid_top = clean(solid_top);
            solid_bottom = clean(solid_bottom);

            // bridges: bottom skin with nothing underneath (neither model nor support)
            let mut bridge: Polys = Vec::new();
            if s.bridges && i > 0 && !solid_bottom.is_empty() {
                let below = geom::union2(&layers[i - 1].region, &supports[i - 1].area);
                let unsupported = geom::diff(&solid_bottom, &offset(&below, 300.0));
                bridge = clean(unsupported);
                if !bridge.is_empty() {
                    solid_bottom = geom::diff(&solid_bottom, &offset(&bridge, 100.0));
                }
            }
            let solid_all = geom::union2(&geom::union2(&solid_top, &solid_bottom), &bridge);

            let angle0 = s.infill_angle + 90.0 * (i % 2) as f64;
            let solid_sp = spacing_um(s.solid_width, 100.0);

            // bridge lines run across the short side of the bounding box
            if !bridge.is_empty() {
                for part in bridge.chunks(1) {
                    let (x0, y0, x1, y1) = geom::bbox(&part.to_vec()).unwrap();
                    let angle = if (x1 - x0) >= (y1 - y0) { 90.0 } else { 0.0 };
                    let mk = |pts: Vec<Pt>| Path { pts, closed: false, role: role::BRIDGE, speed: s.bridge_speed, width: s.nozzle_diameter, flow: s.bridge_flow, fan: Some(s.bridge_fan) };
                    paths.extend(ord.fill(&part.to_vec(), angle, mm(s.nozzle_diameter * 1.0) as f64, &mk));
                }
            }
            for (area, r_default, spd) in [(&solid_bottom, role::BOTTOM, s.bottom_speed), (&solid_top, role::TOP, s.top_speed)] {
                if area.is_empty() {
                    continue;
                }
                let (r, speed, flow) = if first_on_plate { (role::FIRST_FILL, s.first_layer_speed, s.first_layer_flow) } else { (r_default, spd, 100.0) };
                let mk = |pts: Vec<Pt>| Path { pts, closed: false, role: r, speed, width: s.solid_width, flow, fan: None };
                paths.extend(ord.fill(area, angle0, solid_sp, &mk));
            }

            // sparse infill
            if s.infill_density > 0.0 {
                let sparse = geom::open(&geom::diff(ia, &solid_all), 100.0);
                let sparse = geom::remove_small(&sparse, 1.0e6 * 0.5);
                if !sparse.is_empty() {
                    let speed = if first_on_plate { s.first_layer_speed } else { s.infill_speed };
                    let mk = |pts: Vec<Pt>| Path { pts, closed: false, role: role::INFILL, speed, width: s.infill_width, flow: 100.0, fan: None };
                    match s.infill_pattern {
                        InfillPattern::Lines => {
                            paths.extend(ord.fill(&sparse, angle0, spacing_um(s.infill_width, s.infill_density), &mk));
                        }
                        InfillPattern::Grid => {
                            let sp = spacing_um(s.infill_width, s.infill_density) * 2.0;
                            paths.extend(ord.fill(&sparse, s.infill_angle, sp, &mk));
                            paths.extend(ord.fill(&sparse, s.infill_angle + 90.0, sp, &mk));
                        }
                        InfillPattern::Triangles => {
                            let sp = spacing_um(s.infill_width, s.infill_density) * 3.0;
                            for a in [0.0, 60.0, 120.0] {
                                paths.extend(ord.fill(&sparse, s.infill_angle + a, sp, &mk));
                            }
                        }
                        InfillPattern::Concentric => {
                            let loops: Vec<Path> = concentric(&sparse, spacing_um(s.infill_width, s.infill_density))
                                .into_iter()
                                .map(|pts| Path { pts, closed: true, role: role::INFILL, speed, width: s.infill_width, flow: 100.0, fan: None })
                                .collect();
                            paths.extend(ord.order(loops));
                        }
                    }
                }
            }
        }

        // support on this layer
        let sl = &supports[i];
        if !sl.area.is_empty() {
            let w = s.support_width;
            let wu = mm(w) as f64;
            let flow = s.support_flow;
            let contour: Vec<Path> = offset(&sl.area, -wu / 2.0)
                .into_iter()
                .filter(|p| p.len() >= 3)
                .map(|pts| Path { pts, closed: true, role: role::SUPPORT_CONTOUR, speed: s.support_speed, width: w, flow, fan: None })
                .collect();
            paths.extend(ord.order(contour));
            let inner = offset(&sl.area, -wu);
            let iface = geom::inter(&inner, &sl.interface);
            let floor = geom::diff(&geom::inter(&inner, &sl.floor), &iface);
            let body = geom::diff(&geom::diff(&inner, &iface), &floor);
            let angle = 90.0 * (i % 2) as f64;
            let mk = |role_: u8, speed: f64| move |pts: Vec<Pt>| Path { pts, closed: false, role: role_, speed, width: w, flow, fan: None };
            if !iface.is_empty() {
                let mk_i = mk(role::INTERFACE_INFILL, s.support_interface_speed);
                paths.extend(ord.fill(&iface, angle + 45.0, spacing_um(w, s.support_interface_density), &mk_i));
            }
            if !floor.is_empty() {
                let mk_f = mk(role::INTERFACE_INFILL, s.support_interface_speed);
                paths.extend(ord.fill(&floor, angle + 45.0, spacing_um(w, s.support_bottom_density), &mk_f));
            }
            if s.support_density > 0.0 && !body.is_empty() {
                let mk_b = mk(role::SUPPORT_INFILL, s.support_speed);
                let sp = spacing_um(w, s.support_density);
                match s.support_pattern {
                    SupportPattern::Lines => paths.extend(ord.fill(&body, angle, sp, &mk_b)),
                    SupportPattern::Grid => {
                        paths.extend(ord.fill(&body, 0.0, sp * 2.0, &mk_b));
                        paths.extend(ord.fill(&body, 90.0, sp * 2.0, &mk_b));
                    }
                    SupportPattern::Concentric => {
                        let loops: Vec<Path> = concentric(&body, sp)
                            .into_iter()
                            .map(|pts| Path { pts, closed: true, role: role::SUPPORT_INFILL, speed: s.support_speed, width: w, flow, fan: None })
                            .collect();
                        paths.extend(ord.order(loops));
                    }
                }
            }
        }

        for p in &mut paths {
            p.pts = thin(&p.pts, 80.0);
        }
        paths.retain(|p| p.pts.len() >= 2);
        paths
        })
        .collect();

    for (i, mut paths) in layer_paths.into_iter().enumerate() {
        if paths.is_empty() {
            continue; // nothing to print on this layer
        }
        let l = &layers[i];
        let printed_index = out_layers.len() as u32;

        // fan
        let fan = if printed_index < s.fan_start_layer { 0 } else { s.fan_percent };
        for p in &mut paths {
            if p.fan.is_none() {
                p.fan = Some(fan);
            }
        }

        // slow down short layers
        let time: f64 = paths.iter().map(|p| seg_len(&p.pts, false) / 1000.0 / p.speed.max(1.0)).sum();
        if time > 0.0 && time < s.min_layer_time {
            let f = time / s.min_layer_time;
            for p in &mut paths {
                if p.role != role::BRIDGE {
                    p.speed = (p.speed * f).max(s.min_speed).min(p.speed);
                }
            }
        }

        let thickness = (l.z_top - last_z) as f64 / 1000.0;
        out_layers.push(Layer { marker: next_marker, z: l.z_top, thickness, paths, is_raft: false });
        last_z = l.z_top;
        next_marker += 1;
    }

    let model_layers = out_layers.iter().filter(|l| !l.is_raft).count();
    tick("chemins (cumulé)");
    Ok(Plan { layers: out_layers, model_layers, raft_layers: raft_n, raft_top: mm(raft_top), warnings })
}

#[allow(dead_code)]
fn point_in(region: &Polys, p: Pt) -> bool {
    contains(region, p.x as f64, p.y as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::cube_mesh;

    fn total_len(plan: &Plan) -> f64 {
        plan.layers.iter().flat_map(|l| &l.paths).map(|p| seg_len(&p.pts, false)).sum::<f64>() / 1000.0
    }

    #[test]
    fn cube_without_raft_has_expected_layers_and_paths() {
        let mesh = cube_mesh(90.0, 90.0, 20.0);
        let mut s = Settings::default();
        s.raft = false;
        let p = plan(&mesh, &s).unwrap();
        // 0.26 first layer then 0.15-ish layers up to 20 mm
        assert!(p.model_layers >= 130 && p.model_layers <= 136, "{}", p.model_layers);
        assert_eq!(p.layers[0].marker, 0);
        assert!(p.layers[0].paths.iter().any(|x| x.role == role::FIRST_CONTOUR));
        assert!(p.layers.last().unwrap().paths.iter().any(|x| x.role == role::TOP));
        assert!((p.layers.last().unwrap().z - 20_000).abs() <= 1, "{}", p.layers.last().unwrap().z);
        assert!(total_len(&p) > 10_000.0);
    }

    #[test]
    fn raft_adds_negative_markers_and_lifts_the_model() {
        let mesh = cube_mesh(90.0, 90.0, 10.0);
        let p = plan(&mesh, &Settings::default()).unwrap();
        assert_eq!(p.raft_layers, 5);
        assert_eq!(p.layers[0].marker, -5);
        assert_eq!(p.layers[4].marker, -1);
        assert_eq!(p.raft_top, 1060);
        assert_eq!(p.layers[5].marker, 0);
        assert_eq!(p.layers[5].z, 1060 + 300);
        assert!((p.layers.last().unwrap().z - (1060 + 10_000)).abs() <= 1);
    }

    #[test]
    fn rejects_models_outside_bed_and_too_tall() {
        let s = Settings::default();
        assert!(plan(&cube_mesh(195.0, 90.0, 10.0), &s).is_err(), "outside the bed");
        assert!(plan(&cube_mesh(5.0, 5.0, 190.0), &s).is_err(), "too tall");
        assert!(plan(&cube_mesh(5.0, 5.0, 100.0), &s).is_ok());
    }

    #[test]
    fn overhang_gets_support_only_when_enabled() {
        // a T shape: stem 10x10x20 and a 40x10x5 bar on top
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        let mut boxm = |x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64| {
            let b = verts.len() as i32;
            verts.extend([
                [x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0],
                [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1],
            ]);
            for t in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
                tris.push([t[0] + b, t[1] + b, t[2] + b]);
            }
        };
        boxm(95.0, 95.0, 0.0, 105.0, 105.0, 20.0);
        boxm(80.0, 95.0, 20.0, 120.0, 105.0, 25.0);
        let mesh = Mesh { vertices: verts, triangles: tris };
        let mut s = Settings::default();
        s.raft = false;
        let without = plan(&mesh, &s).unwrap();
        assert!(!without.layers.iter().flat_map(|l| &l.paths).any(|p| p.role == role::SUPPORT_INFILL || p.role == role::SUPPORT_CONTOUR));
        s.support = true;
        let with = plan(&mesh, &s).unwrap();
        assert!(with.layers.iter().flat_map(|l| &l.paths).any(|p| p.role == role::SUPPORT_CONTOUR));
        assert!(with.layers.iter().flat_map(|l| &l.paths).any(|p| p.role == role::INTERFACE_INFILL));
        // support never appears above the overhang
        let top = with.layers.last().unwrap();
        assert!(!top.paths.iter().any(|p| p.role == role::SUPPORT_CONTOUR));
    }

    fn t_mesh() -> Mesh {
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        let mut boxm = |x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64| {
            let b = verts.len() as i32;
            verts.extend([
                [x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0],
                [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1],
            ]);
            for t in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
                tris.push([t[0] + b, t[1] + b, t[2] + b]);
            }
        };
        boxm(95.0, 95.0, 0.0, 105.0, 105.0, 20.0);
        boxm(70.0, 90.0, 20.0, 130.0, 110.0, 25.0);
        Mesh { vertices: verts, triangles: tris }
    }

    fn support_points(p: &Plan) -> Vec<(i64, i64, i64)> {
        p.layers
            .iter()
            .flat_map(|l| l.paths.iter().filter(|x| x.role == role::SUPPORT_CONTOUR || x.role == role::SUPPORT_INFILL).flat_map(move |x| x.pts.iter().map(move |q| (q.x, q.y, l.z))))
            .collect()
    }

    #[test]
    fn tree_supports_branch_and_avoid_the_stem() {
        let mut s = Settings::default();
        s.raft = false;
        s.support = true;
        s.support_type = crate::settings::SupportType::Tree;
        let p = plan(&t_mesh(), &s).unwrap();
        let pts = support_points(&p);
        assert!(pts.len() > 100, "tree support should exist: {}", pts.len());
        // never inside the stem footprint (95..105) below the bar (z < 20 mm)
        assert!(pts.iter().filter(|q| q.2 < 19_500).all(|q| !(q.0 > 95_000 && q.0 < 105_000 && q.1 > 95_000 && q.1 < 105_000)), "support inside the stem");
        // support stays below the bar
        assert!(pts.iter().all(|q| q.2 <= 20_000), "support above the overhang");
        // a tree uses much less material than solid columns
        let mut n = s.clone();
        n.support_type = crate::settings::SupportType::Normal;
        let normal = plan(&t_mesh(), &n).unwrap();
        let len = |p: &Plan| p.layers.iter().flat_map(|l| &l.paths).filter(|x| x.role == role::SUPPORT_CONTOUR || x.role == role::SUPPORT_INFILL).map(|x| seg_len(&x.pts, false)).sum::<f64>();
        assert!(len(&p) < len(&normal), "tree {} vs normal {}", len(&p), len(&normal));
    }

    #[test]
    fn blockers_remove_support_and_enforcers_force_it() {
        let mut s = Settings::default();
        s.raft = false;
        s.support = true;
        let with = plan(&t_mesh(), &s).unwrap();
        assert!(!support_points(&with).is_empty());

        let mut b = s.clone();
        b.support_blockers.push(crate::settings::Volume { min: [0.0, 0.0, -1.0], max: [200.0, 200.0, 100.0] });
        assert!(support_points(&plan(&t_mesh(), &b).unwrap()).is_empty(), "blocker over everything");

        // overhang detection off (0.2 degrees), then an enforcer around the bar brings supports back
        let mut e = s.clone();
        e.support_angle = 0.2;
        assert!(support_points(&plan(&t_mesh(), &e).unwrap()).is_empty());
        e.support_enforcers.push(crate::settings::Volume { min: [60.0, 80.0, 19.0], max: [140.0, 120.0, 26.0] });
        assert!(!support_points(&plan(&t_mesh(), &e).unwrap()).is_empty(), "enforcer");
    }

    #[test]
    fn support_patterns_and_floor_are_printed() {
        let mut s = Settings::default();
        s.raft = false;
        s.support = true;
        for pat in [crate::settings::SupportPattern::Lines, crate::settings::SupportPattern::Grid, crate::settings::SupportPattern::Concentric] {
            s.support_pattern = pat;
            let p = plan(&t_mesh(), &s).unwrap();
            assert!(p.layers.iter().flat_map(|l| &l.paths).any(|x| x.role == role::SUPPORT_INFILL), "{pat:?}");
            // the first layer carries a solid floor
            assert!(p.layers[0].paths.iter().any(|x| x.role == role::INTERFACE_INFILL), "{pat:?} floor");
        }
    }

    fn boxes(list: &[[f64; 6]]) -> Mesh {
        let (mut verts, mut tris) = (Vec::new(), Vec::new());
        for b in list {
            let base = verts.len() as i32;
            let [x0, y0, z0, x1, y1, z1] = *b;
            verts.extend([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]);
            for t in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
                tris.push([t[0] + base, t[1] + base, t[2] + base]);
            }
        }
        Mesh { vertices: verts, triangles: tris }
    }

    /// A ball (lower half needs support) standing on a thin foot.
    fn ball_on_foot() -> Mesh {
        let (cx, cy, cz, r) = (100.0, 100.0, 40.0, 20.0);
        let (nu, nv) = (48usize, 24usize);
        let mut verts = Vec::new();
        for j in 0..=nv {
            let phi = std::f64::consts::PI * j as f64 / nv as f64;
            for i in 0..nu {
                let th = std::f64::consts::TAU * i as f64 / nu as f64;
                verts.push([cx + r * phi.sin() * th.cos(), cy + r * phi.sin() * th.sin(), cz - r * phi.cos()]);
            }
        }
        let mut tris = Vec::new();
        for j in 0..nv {
            for i in 0..nu {
                let (a, b) = ((j * nu + i) as i32, (j * nu + (i + 1) % nu) as i32);
                let (c, d) = (((j + 1) * nu + i) as i32, ((j + 1) * nu + (i + 1) % nu) as i32);
                if j > 0 {
                    tris.push([a, b, c]);
                }
                if j < nv - 1 {
                    tris.push([b, d, c]);
                }
            }
        }
        let foot = boxes(&[[97.0, 97.0, 0.0, 103.0, 103.0, 22.0]]);
        let base = verts.len() as i32;
        verts.extend(foot.vertices);
        tris.extend(foot.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
        Mesh { vertices: verts, triangles: tris }
    }

    fn support_len(p: &Plan) -> f64 {
        p.layers
            .iter()
            .flat_map(|l| &l.paths)
            .filter(|x| matches!(x.role, role::SUPPORT_CONTOUR | role::SUPPORT_INFILL | role::INTERFACE_INFILL))
            .map(|x| seg_len(&x.pts, false) / 1000.0)
            .sum()
    }

    fn compare(mesh: &Mesh) -> (f64, f64) {
        let mut s = Settings::default();
        s.raft = false;
        s.support = true;
        let normal = plan(mesh, &s).unwrap();
        s.support_type = crate::settings::SupportType::Tree;
        let tree = plan(mesh, &s).unwrap();
        (support_len(&normal), support_len(&tree))
    }

    #[test]
    fn tree_supports_curved_overhangs_and_saves_material() {
        // curved underside: used to get almost no support because the overhang is a thin ring per layer
        let (normal, tree) = compare(&ball_on_foot());
        assert!(tree > 3000.0, "the ball's underside must be supported, got {tree} mm");
        assert!(tree < normal * 1.1, "tree {tree} vs normal {normal}");

        // wide flat undersides: branches must converge into a few trunks (much less material)
        let deck = boxes(&[[70.0, 90.0, 0.0, 80.0, 110.0, 25.0], [120.0, 90.0, 0.0, 130.0, 110.0, 25.0], [70.0, 90.0, 25.0, 130.0, 110.0, 31.0]]);
        let (n, t) = compare(&deck);
        assert!(t < n * 0.6, "arch: tree {t} vs normal {n}");
        let tee = boxes(&[[95.0, 95.0, 0.0, 105.0, 105.0, 20.0], [60.0, 85.0, 20.0, 140.0, 115.0, 26.0]]);
        let (n, t) = compare(&tee);
        assert!(t < n * 0.7, "T: tree {t} vs normal {n}");
    }

    #[test]
    fn tree_branches_respect_the_lean_limit() {
        let mut s = Settings::default();
        s.raft = false;
        s.support = true;
        s.support_type = crate::settings::SupportType::Tree;
        s.tree_branch_angle = 20.0;
        let tee = boxes(&[[95.0, 95.0, 0.0, 105.0, 105.0, 20.0], [60.0, 85.0, 20.0, 140.0, 115.0, 26.0]]);
        let flat = plan(&tee, &s).unwrap();
        s.tree_branch_angle = 60.0;
        let steep = plan(&tee, &s).unwrap();
        // leaning more allows the branches to converge earlier, so less material
        assert!(support_len(&steep) <= support_len(&flat), "{} vs {}", support_len(&steep), support_len(&flat));
    }

    #[test]
    fn raft_brim_and_supports_never_leave_the_plate() {
        // a model close to the edges: its raft margin and supports would stick out
        let near = boxes(&[[3.0, 120.0, 0.0, 14.0, 131.0, 25.0], [3.0, 120.0, 25.0, 60.0, 150.0, 31.0]]);
        for (raft, brim, support, tree) in [(true, 0.0, false, false), (false, 6.0, false, false), (true, 0.0, true, false), (false, 0.0, true, true)] {
            let mut s = Settings::default();
            s.raft = raft;
            s.brim_width = brim;
            s.support = support;
            if tree {
                s.support_type = crate::settings::SupportType::Tree;
            }
            let p = plan(&near, &s).unwrap();
            let (mut lo, mut hi) = ([i64::MAX; 2], [i64::MIN; 2]);
            for q in p.layers.iter().flat_map(|l| &l.paths).flat_map(|x| &x.pts) {
                lo = [lo[0].min(q.x), lo[1].min(q.y)];
                hi = [hi[0].max(q.x), hi[1].max(q.y)];
            }
            assert!(lo[0] >= 0 && lo[1] >= 0 && hi[0] <= 200_000 && hi[1] <= 200_000, "raft {raft} brim {brim} support {support}: {lo:?} {hi:?}");
        }
        // and the file passes verification (this used to fail with positions like (-0.6, 166.9))
        let mut s = Settings::default();
        s.raft = true;
        s.support = true;
        let png = crate::preview::top_view_png(&[], [200.0, 200.0], 32);
        assert!(crate::slice_to_zcodex2(&near, &s, png, None).is_ok());
    }
}
