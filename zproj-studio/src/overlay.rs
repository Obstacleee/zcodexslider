//! Lines drawn over the 3D view for what the slicer adds: supports, raft and brim.

use zslice::toolpath::{role, Plan};

/// Lines for supports, raft and brim, so they can be drawn in the 3D view (physical Z, so the model has to be
/// lifted by the raft height when drawn together). Anything outside the plate is red.
pub fn plan_wires(plan: &Plan, bed: [f64; 2], show_support: bool, show_base: bool) -> Vec<crate::render::Wire> {
    use crate::render::Wire;
    let mut out = Vec::new();
    let outside = |x: f64, y: f64| x < -0.01 || y < -0.01 || x > bed[0] + 0.01 || y > bed[1] + 0.01;
    let mut push = |a: (i64, i64), b: (i64, i64), z: f64, color: [u8; 3]| {
        let (ax, ay, bx, by) = (a.0 as f64 / 1000.0, a.1 as f64 / 1000.0, b.0 as f64 / 1000.0, b.1 as f64 / 1000.0);
        let c = if outside(ax, ay) || outside(bx, by) { [255, 60, 60] } else { color };
        out.push(Wire { a: [ax as f32, ay as f32, z as f32], b: [bx as f32, by as f32, z as f32], color: c });
    };
    // draw about 40 slices of the supports, not every layer, so the branches stay readable
    let support_layers = plan.layers.iter().filter(|l| !l.is_raft).count();
    let stride = (support_layers / 40).max(1);
    for (li, l) in plan.layers.iter().enumerate() {
        if !l.is_raft && li % stride != 0 {
            continue;
        }
        let z = l.z as f64 / 1000.0;
        for p in &l.paths {
            let kind = match (l.is_raft, p.role) {
                (true, _) if p.closed && show_base => Some([150, 150, 165]),
                (false, role::BRIM) if show_base => Some([255, 170, 170]),
                (false, role::SUPPORT_CONTOUR) if show_support => Some([70, 225, 225]),
                _ => None,
            };
            let Some(color) = kind else { continue };
            for w in p.pts.windows(2) {
                push((w[0].x, w[0].y), (w[1].x, w[1].y), z, color);
            }
            if p.closed && p.pts.len() > 2 && p.pts.first() != p.pts.last() {
                let (a, b) = (p.pts[p.pts.len() - 1], p.pts[0]);
                push((a.x, a.y), (b.x, b.y), z, color);
            }
        }
    }
    // keep the view responsive on huge supports
    if out.len() > 300_000 {
        let stride = out.len() / 300_000 + 1;
        out = out.into_iter().step_by(stride).collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zslice::{Mesh, Settings, SupportType};

    fn tee() -> Mesh {
        let (mut v, mut t) = (Vec::new(), Vec::new());
        for b in [[95.0, 95.0, 0.0, 105.0, 105.0, 14.0], [70.0, 85.0, 14.0, 130.0, 115.0, 18.0]] {
            let base = v.len() as i32;
            let [x0, y0, z0, x1, y1, z1] = b;
            v.extend([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]);
            for k in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
                t.push([k[0] + base, k[1] + base, k[2] + base]);
            }
        }
        Mesh { vertices: v, triangles: t }
    }

    #[test]
    fn supports_raft_and_brim_become_wires() {
        let mut s = Settings::default();
        s.raft = true;
        s.support = true;
        s.support_type = SupportType::Tree;
        let plan = zslice::toolpath::plan(&tee(), &s).unwrap();
        let sup = plan_wires(&plan, [200.0, 200.0], true, false);
        let base = plan_wires(&plan, [200.0, 200.0], false, true);
        assert!(sup.len() > 500, "{}", sup.len());
        assert!(!base.is_empty());
        // supports sit below the overhang (14 mm model height + raft), raft layers at the bottom
        let raft_top = plan.raft_top as f32 / 1000.0;
        assert!(sup.iter().all(|w| w.a[2] <= raft_top + 14.1 && w.a[2] >= raft_top));
        assert!(base.iter().all(|w| w.a[2] <= raft_top + 0.01));
        // nothing is drawn when the toggles are off
        assert!(plan_wires(&plan, [200.0, 200.0], false, false).is_empty());
    }

    #[test]
    fn anything_beyond_the_plate_is_red() {
        let mut s = Settings::default();
        s.support = true;
        let plan = zslice::toolpath::plan(&tee(), &s).unwrap();
        // the same plan on a plate that is too small: every wire leaves it
        let wires = plan_wires(&plan, [60.0, 60.0], true, true);
        assert!(!wires.is_empty());
        assert!(wires.iter().all(|w| w.color == [255, 60, 60]));
        assert!(plan_wires(&plan, [200.0, 200.0], true, true).iter().all(|w| w.color != [255, 60, 60]));
    }
}
