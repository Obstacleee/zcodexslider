//! Support planning: classic column supports and branching "tree" supports, with
//! enforcer / blocker volumes. Works on per-layer regions and returns, for each layer, the area
//! that receives support, plus the parts printed solid (roof under the model, floor on the plate).

use rayon::prelude::*;
use crate::geom::{self, contains, mm, offset, pt, Poly, Polys};
use crate::settings::{Settings, SupportType, Volume};

#[derive(Clone, Default)]
pub struct SupportLayer {
    /// Area that gets support material on this layer.
    pub area: Polys,
    /// Part printed solid right under the model (roof).
    pub interface: Polys,
    /// Part printed solid where the support starts (on the plate or on the model).
    pub floor: Polys,
}

pub struct In<'a> {
    pub region: &'a Polys,
    /// Layer thickness in mm.
    pub thickness: f64,
    /// Height of the layer's centre above the model bottom, in mm (model coordinates).
    pub zc: f64,
}

fn rect(v: &Volume) -> Polys {
    let (x0, y0, x1, y1) = (mm(v.min[0]), mm(v.min[1]), mm(v.max[0]), mm(v.max[1]));
    vec![vec![pt(x0, y0), pt(x1, y0), pt(x1, y1), pt(x0, y1)]]
}

fn circle(x: f64, y: f64, r: f64) -> Poly {
    let n = if r < 1500.0 { 10 } else { 16 };
    (0..n)
        .map(|k| {
            let a = k as f64 / n as f64 * std::f64::consts::TAU;
            pt((x + r * a.cos()).round() as i64, (y + r * a.sin()).round() as i64)
        })
        .collect()
}

/// Support layers for the model. Every printed support piece is guaranteed to rest on something
/// (the plate, the layer of support below it, or the model).
pub fn plan_supports(layers: &[In], s: &Settings) -> Vec<SupportLayer> {
    build(layers, s, true).0
}

/// Like [`plan_supports`], also returning the area (µm²) of floating support that was removed.
pub fn plan_supports_stats(layers: &[In], s: &Settings) -> (Vec<SupportLayer>, f64) {
    build(layers, s, true)
}

/// Without the final "nothing may float" pass; only for measuring what that pass removes.
pub fn plan_supports_ungrounded(layers: &[In], s: &Settings) -> Vec<SupportLayer> {
    build(layers, s, false).0
}

/// Area (µm²) of support that has nothing beneath it: neither support one layer down, nor the model
/// (unless `support_from_bed_only`), nor the plate (layer 0).
pub fn floating_area(layers: &[In], area: &[Polys], s: &Settings) -> f64 {
    let tol = ground_tolerance(s);
    let mut total = 0.0;
    for i in 1..area.len() {
        if area[i].is_empty() {
            continue;
        }
        let below = support_below(layers, area, i, s);
        let allowed = offset(&below, tol);
        // sub-micron slivers along long edges are Clipper rounding, not floating material
        let d = geom::total_area(&geom::diff(&area[i], &allowed));
        if d > 1.0e5 && d > 0.005 * geom::total_area(&area[i]) {
            total += d;
        }
    }
    total.max(0.0)
}

fn ground_tolerance(s: &Settings) -> f64 {
    (mm(s.support_xy_gap) * 2).max(400) as f64
}

fn support_below(layers: &[In], area: &[Polys], i: usize, s: &Settings) -> Polys {
    if s.support_from_bed_only {
        area[i - 1].clone()
    } else {
        geom::union2(&area[i - 1], layers[i - 1].region)
    }
}

/// Bottom-up pass that trims every support piece to what is actually supported from below.
fn ground(layers: &[In], area: &mut [Polys], s: &Settings, min_area: f64) -> f64 {
    let t_g = std::time::Instant::now();
    let tol = ground_tolerance(s);
    let trim = |a: &Polys, allowed: &Polys| -> (Polys, f64) {
        let kept = geom::remove_small(&geom::inter(a, allowed), min_area);
        let lost = geom::total_area(a) - geom::total_area(&kept);
        (kept, lost)
    };
    // dilation distributes over union: the model part is independent of any trimming, so it is done once
    let grown_model: Vec<Polys> = (0..area.len())
        .into_par_iter()
        .map(|i| if i == 0 || s.support_from_bed_only { Vec::new() } else { offset(layers[i - 1].region, tol) })
        .collect();
    let allowed_for = |area: &[Polys], i: usize| -> Polys { geom::union2(&offset(&geom::simplify(&area[i - 1], 40.0), tol), &grown_model[i]) };
    // optimistic round, all layers at once: assumes nothing below gets trimmed
    let first: Vec<Option<(Polys, f64)>> = {
        let a: &[Polys] = area;
        (0..a.len())
            .into_par_iter()
            .map(|i| if i == 0 || a[i].is_empty() { None } else { Some(trim(&a[i], &allowed_for(a, i))) })
            .collect()
    };
    if std::env::var("ZTIME").is_ok() {
        eprintln!("    [ground] optimistic round done: {:.0?}", t_g.elapsed());
    }
    // exact round, bottom-up: a layer is only recomputed when the layer below really changed
    let mut changed_prev = false;
    let mut removed = 0.0;
    for (i, f) in first.into_iter().enumerate() {
        let Some((kept, lost)) = f else {
            changed_prev = false;
            continue;
        };
        let (kept, lost) = if changed_prev { trim(&area[i], &allowed_for(area, i)) } else { (kept, lost) };
        changed_prev = lost > 2.0e4;
        removed += lost.max(0.0);
        area[i] = kept;
    }
    if std::env::var("ZTIME").is_ok() {
        eprintln!("    [ground] total {:.0?}", t_g.elapsed());
    }
    removed.max(0.0)
}

fn build(layers: &[In], s: &Settings, do_ground: bool) -> (Vec<SupportLayer>, f64) {
    let clock = std::time::Instant::now();
    let tick = |what: &str| {
        if std::env::var("ZTIME").is_ok() {
            eprintln!("    [support] {what}: {:.0?}", clock.elapsed());
        }
    };
    let n = layers.len();
    let width_um = mm(s.support_width) as f64;
    let open_r = width_um.max(100.0);
    let min_area = (s.support_min_area * 1.0e6).max(width_um * width_um * 2.0);
    let gap = mm(s.support_xy_gap) as f64;
    let zgap = s.support_z_gap_layers as usize;

    // ---- overhangs (what needs something underneath), plus enforcers
    let mut ov: Vec<Polys> = (0..n)
        .into_par_iter()
        .map(|i| {
            if i == 0 {
                return Vec::new();
            }
            let reach = layers[i].thickness / s.support_angle.to_radians().tan() * 1000.0;
            let d = geom::diff(layers[i].region, &offset(layers[i - 1].region, reach));
            geom::remove_small(&geom::open(&d, open_r / 2.0), min_area)
        })
        .collect();
    for v in &s.support_enforcers {
        for i in 0..n {
            if layers[i].zc >= v.min[2] && layers[i].zc <= v.max[2] {
                let inside = geom::inter(layers[i].region, &rect(v));
                if !inside.is_empty() {
                    ov[i] = geom::union2(&ov[i], &geom::remove_small(&inside, min_area));
                }
            }
        }
    }

    tick("surplombs");
    // ---- area per layer
    let mut area = match s.support_type {
        SupportType::Normal => normal_areas(layers, &ov, s, gap, zgap, open_r, min_area),
        SupportType::Tree => tree_areas(layers, &ov, s, gap, zgap, min_area),
    };

    tick("zones");
    if s.support_expansion > 0.0 {
        let e = mm(s.support_expansion) as f64;
        area.par_iter_mut().enumerate().for_each(|(i, a)| {
            if !a.is_empty() {
                *a = geom::diff(&offset(a, e), &offset(layers[i].region, gap));
            }
        });
    }
    // supports cannot be printed beyond the plate
    {
        let (m, bx, by) = (mm(0.6), mm(s.bed[0] - 0.6), mm(s.bed[1] - 0.6));
        let bed: Polys = vec![vec![pt(m, m), pt(bx, m), pt(bx, by), pt(m, by)]];
        area.par_iter_mut().for_each(|a| {
            if !a.is_empty() {
                *a = geom::inter(a, &bed);
            }
        });
    }
    if s.support_from_bed_only {
        let mut below: Polys = Vec::new();
        for i in 0..n {
            if !below.is_empty() {
                area[i] = geom::diff(&area[i], &offset(&below, gap));
            }
            below = geom::union2(&below, layers[i].region);
        }
    }
    for v in &s.support_blockers {
        let r = rect(v);
        for i in 0..n {
            if layers[i].zc >= v.min[2] && layers[i].zc <= v.max[2] && !area[i].is_empty() {
                area[i] = geom::diff(&area[i], &r);
            }
        }
    }
    area.par_iter_mut().for_each(|a| *a = geom::remove_small(a, min_area));
    tick("découpes");
    let mut removed = 0.0;
    if do_ground && s.support_type == SupportType::Tree {
        removed = ground(layers, &mut area, s, min_area);
    }

    tick("ancrage");
    // ---- roof (under the model) and floor (where support begins)
    let iface_n = s.support_interface_layers as usize;
    let floor_n = s.support_bottom_layers as usize;
    let out: Vec<SupportLayer> = (0..n)
        .into_par_iter()
        .map(|i| {
            let mut iface: Polys = Vec::new();
            for k in 0..iface_n {
                let j = i + 1 + zgap + k;
                if j < n && !ov[j].is_empty() {
                    iface = geom::union2(&iface, &ov[j]);
                }
            }
            let interface = if iface.is_empty() || area[i].is_empty() { Vec::new() } else { geom::inter(&area[i], &offset(&iface, open_r)) };
            let mut floor: Polys = Vec::new();
            if s.support_type == SupportType::Tree {
                // branches drift sideways, so only the first layers on the plate get a solid floor
                if i < floor_n {
                    floor = area[i].clone();
                }
            } else if floor_n > 0 && !area[i].is_empty() {
                let mut under = area[i].clone();
                for k in 1..=floor_n {
                    let prev = if i >= k { area[i - k].clone() } else { Vec::new() };
                    under = geom::inter(&under, &prev);
                    if under.is_empty() {
                        break;
                    }
                }
                floor = geom::remove_small(&geom::diff(&area[i], &under), min_area);
            }
            SupportLayer { area: area[i].clone(), interface, floor }
        })
        .collect();
    (out, removed)
}

fn normal_areas(layers: &[In], ov: &[Polys], s: &Settings, gap: f64, zgap: usize, open_r: f64, min_area: f64) -> Vec<Polys> {
    let n = layers.len();
    let _ = s;
    let mut area: Vec<Polys> = vec![Vec::new(); n];
    let mut carry: Polys = Vec::new();
    // the per-layer offsets do not depend on the sweep: compute them in parallel
    let grown_ov: Vec<Polys> = ov.par_iter().map(|o| if o.is_empty() { Vec::new() } else { offset(o, open_r / 2.0) }).collect();
    let grown_model: Vec<Polys> = (0..n).into_par_iter().map(|i| offset(layers[i].region, gap)).collect();
    for i in (0..n).rev() {
        let src = i + 1 + zgap;
        if src < n && !ov[src].is_empty() {
            carry = geom::union2(&carry, &grown_ov[src]);
        }
        carry = geom::diff(&carry, &grown_model[i]);
        // the morphological opening is the expensive step: keep the sweep cheap, open sparingly...
        if i % 8 == 0 {
            carry = geom::remove_small(&geom::open(&carry, open_r / 2.0), min_area);
        }
        area[i] = carry.clone();
    }
    // ...and clean every layer's copy in parallel
    area.par_iter_mut().for_each(|a| {
        if !a.is_empty() {
            *a = geom::remove_small(&geom::open(a, open_r / 2.0), min_area);
        }
    });
    area
}

#[derive(Clone)]
struct Node {
    x: f64,
    y: f64,
    r: f64,
}

fn tree_areas(layers: &[In], ov: &[Polys], s: &Settings, gap: f64, zgap: usize, min_area: f64) -> Vec<Polys> {
    let n = layers.len();
    let mut area: Vec<Polys> = vec![Vec::new(); n];
    let tip_r = (mm(s.tree_tip_diameter) as f64 / 2.0).max(200.0);
    let max_r = (mm(s.tree_max_diameter) as f64 / 2.0).max(tip_r);
    let spacing = mm(s.tree_tip_spacing).max(800) as f64;
    let tan_branch = s.tree_branch_angle.to_radians().tan();
    let mut nodes: Vec<Node> = Vec::new();
    let mut snaps: Vec<Vec<Node>> = vec![Vec::new(); n];

    for i in (0..n).rev() {
        let thick_um = layers[i].thickness * 1000.0;

        // new branch tips under this layer's overhangs
        let src = i + 1 + zgap;
        if src < n && !ov[src].is_empty() {
            let region = &ov[src];
            let tip = tip_r.max(spacing * 0.45);
            let mut pts: Vec<(f64, f64)> = Vec::new();
            if let Some((x0, y0, x1, y1)) = geom::bbox(region) {
                // work on a slightly shrunk copy so every sample really lies on the overhang
                let shrunk = offset(region, -tip_r * 0.3);
                let base = if shrunk.is_empty() { region.clone() } else { shrunk };
                let mut sp = spacing;
                let est = ((x1 - x0) as f64 / sp + 1.0) * ((y1 - y0) as f64 / sp + 1.0);
                if est > 900.0 {
                    sp *= (est / 900.0).sqrt();
                }
                // interior grid (wide flat overhangs)
                let mut y = (y0 as f64 / sp).ceil() * sp;
                while y <= y1 as f64 {
                    let mut x = (x0 as f64 / sp).ceil() * sp;
                    while x <= x1 as f64 {
                        if contains(&base, x, y) {
                            pts.push((x, y));
                        }
                        x += sp;
                    }
                    y += sp;
                }
                // along the contours: curved overhangs only form a thin ring per layer, which a grid misses
                for ring in &base {
                    let m = ring.len();
                    let mut carry = sp; // start with a point on the first vertex
                    for k in 0..m {
                        let (a, b) = (ring[k], ring[(k + 1) % m]);
                        let len = geom::dist(a, b);
                        if len < 1.0 {
                            continue;
                        }
                        let mut t = carry;
                        while t <= len {
                            let f = t / len;
                            pts.push((a.x as f64 + (b.x - a.x) as f64 * f, a.y as f64 + (b.y - a.y) as f64 * f));
                            t += sp;
                        }
                        carry = t - len;
                    }
                    if ring.len() >= 3 {
                        pts.push((ring[0].x as f64, ring[0].y as f64)); // every island gets at least one tip
                    }
                }
            }
            for (x, y) in pts {
                let covered = nodes.iter().any(|nd| ((nd.x - x).powi(2) + (nd.y - y).powi(2)).sqrt() < nd.r + spacing * 0.35);
                if !covered {
                    nodes.push(Node { x, y, r: tip });
                }
            }
        }

        // the support area of this layer is built later, in parallel, from this snapshot
        if !nodes.is_empty() {
            snaps[i] = nodes.clone();
        }
        if i == 0 || nodes.is_empty() {
            continue;
        }

        // move every branch down one layer: grow, avoid the model, lean towards neighbours, merge
        let step = (thick_um * tan_branch).max(1.0);
        let growth = s.tree_growth * thick_um;
        let below = layers[i - 1].region;
        let snapshot = std::mem::take(&mut nodes);
        // the model offsets are shared by many branches: compute each distinct one once, in parallel
        let key_of = |r: f64| (r * 0.3 / 250.0).round() as i64;
        let mut keys: Vec<i64> = snapshot.iter().map(|nd| key_of((nd.r + growth).min(max_r))).collect();
        keys.sort_unstable();
        keys.dedup();
        let avoid_cache: Vec<(i64, Polys)> = keys.par_iter().map(|&k| (k, offset(below, gap + k as f64 * 250.0))).collect();
        let moved_nodes: Vec<Option<Node>> = snapshot.par_iter().enumerate().map(|(k, nd)| {
            let r = (nd.r + growth).min(max_r);
            let (mut x, mut y) = (nd.x, nd.y);
            // attraction to the nearest other branch
            let mut best: Option<(f64, f64, f64)> = None;
            for (m, o) in snapshot.iter().enumerate() {
                if m == k {
                    continue;
                }
                let d = ((o.x - nd.x).powi(2) + (o.y - nd.y).powi(2)).sqrt();
                if best.map_or(true, |b| d < b.0) {
                    best = Some((d, o.x, o.y));
                }
            }
            if let Some((d, ox, oy)) = best {
                // lean towards the nearest branch at the full allowed angle; both move, so they close in twice as fast
                if d > (nd.r + 1.0) && d < 40_000.0 {
                    let mv = step.min(d / 2.0);
                    x += (ox - nd.x) / d * mv;
                    y += (oy - nd.y) / d * mv;
                }
            }
            // keep the branch centre clear of the model one layer down; a branch that starts too close slides
            // outward by at most one step per layer (the branch angle), and dies only when it is enclosed
            let key = key_of(r);
            let avoid = &avoid_cache.iter().find(|(k2, _)| *k2 == key).unwrap().1;
            if contains(avoid, x, y) {
                let mut moved = None;
                'search: for ring in 1..=8 {
                    let reach = step * ring as f64;
                    let mut free: Vec<(f64, f64)> = Vec::new();
                    let (mut bx, mut by, mut nb) = (0.0, 0.0, 0.0);
                    for a in 0..24 {
                        let ang = a as f64 / 24.0 * std::f64::consts::TAU;
                        let (dx, dy) = (ang.cos() * reach, ang.sin() * reach);
                        if contains(avoid, x + dx, y + dy) {
                            bx += dx;
                            by += dy;
                            nb += 1.0;
                        } else {
                            free.push((dx, dy));
                        }
                    }
                    if free.is_empty() {
                        continue;
                    }
                    // go away from where the model is
                    let (ax, ay) = if nb > 0.0 { (-bx / nb, -by / nb) } else { (0.0, 0.0) };
                    let best = free.iter().copied().max_by(|p, q| (p.0 * ax + p.1 * ay).partial_cmp(&(q.0 * ax + q.1 * ay)).unwrap()).unwrap();
                    let d = (best.0 * best.0 + best.1 * best.1).sqrt().max(1e-9);
                    let lim = step.min(d);
                    moved = Some((x + best.0 / d * lim, y + best.1 / d * lim));
                    break 'search;
                }
                match moved {
                    Some((mx, my)) => {
                        x = mx;
                        y = my;
                    }
                    None => return None, // enclosed: this branch ends here
                }
            }
            Some(Node { x, y, r })
        }).collect();
        let next: Vec<Node> = moved_nodes.into_iter().flatten().collect();
        // merge branches that touch
        let mut merged: Vec<Node> = Vec::with_capacity(next.len());
        let mut used = vec![false; next.len()];
        for a in 0..next.len() {
            if used[a] {
                continue;
            }
            let mut cur = next[a].clone();
            for b in (a + 1)..next.len() {
                if used[b] {
                    continue;
                }
                let d = ((cur.x - next[b].x).powi(2) + (cur.y - next[b].y).powi(2)).sqrt();
                if d < (cur.r + next[b].r) * 0.85 {
                    let (wa, wb) = (cur.r * cur.r, next[b].r * next[b].r);
                    cur = Node {
                        x: (cur.x * wa + next[b].x * wb) / (wa + wb),
                        y: (cur.y * wa + next[b].y * wb) / (wa + wb),
                        r: (wa + wb).sqrt().min(max_r),
                    };
                    used[b] = true;
                }
            }
            merged.push(cur);
        }
        nodes = merged;
    }
    area.par_iter_mut().enumerate().for_each(|(i, a)| {
        if !snaps[i].is_empty() {
            let circles: Polys = snaps[i].iter().map(|nd| circle(nd.x, nd.y, nd.r)).collect();
            let u = geom::union(&circles);
            *a = geom::remove_small(&geom::diff(&u, &offset(layers[i].region, gap)), min_area.min(2.0e5));
        }
    });
    area
}
