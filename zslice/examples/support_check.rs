//! support_check: how much support floats in mid-air (before the grounding pass) on several shapes.
use zslice::slicer::slice;
use zslice::support::{floating_area, plan_supports_stats, plan_supports_ungrounded, In};
use zslice::{Mesh, Settings, SupportType};

fn boxes(list: &[[f64; 6]]) -> Mesh {
    let (mut v, mut t) = (Vec::new(), Vec::new());
    for b in list {
        let base = v.len() as i32;
        let [x0, y0, z0, x1, y1, z1] = *b;
        v.extend([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]);
        for k in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
            t.push([k[0] + base, k[1] + base, k[2] + base]);
        }
    }
    Mesh { vertices: v, triangles: t }
}

fn ball(r: f64, nu: usize, nv: usize) -> Mesh {
    let (cx, cy, cz) = (100.0, 100.0, r + 12.0);
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
    let foot = boxes(&[[98.0, 98.0, 0.0, 102.0, 102.0, 13.0]]);
    let base = verts.len() as i32;
    verts.extend(foot.vertices);
    tris.extend(foot.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
    Mesh { vertices: verts, triangles: tris }
}

fn main() {
    let shapes: Vec<(&str, Mesh)> = vec![
        ("T", boxes(&[[95.0, 95.0, 0.0, 105.0, 105.0, 20.0], [60.0, 85.0, 20.0, 140.0, 115.0, 26.0]])),
        ("arche", boxes(&[[70.0, 90.0, 0.0, 80.0, 110.0, 25.0], [120.0, 90.0, 0.0, 130.0, 110.0, 25.0], [70.0, 90.0, 25.0, 130.0, 110.0, 31.0]])),
        ("escalier", boxes(&[[60.0, 90.0, 0.0, 80.0, 110.0, 10.0], [60.0, 90.0, 10.0, 100.0, 110.0, 20.0], [60.0, 90.0, 20.0, 120.0, 110.0, 30.0], [60.0, 90.0, 30.0, 140.0, 110.0, 40.0]])),
        ("balle", ball(20.0, 48, 24)),
    ];
    for (name, mesh) in &shapes {
        for kind in [SupportType::Normal, SupportType::Tree] {
            let mut s = Settings::default();
            s.support = true;
            s.support_type = kind;
            let (lo, hi) = mesh.bounds();
            let h = hi[2] - lo[2];
            let n = ((h - 0.26) / 0.15).round() as usize + 1;
            let planes: Vec<f64> = (0..n).map(|i| if i == 0 { 0.13 } else { 0.26 + (i as f64 - 0.5) * 0.15 }).collect();
            let regions = slice(mesh, &planes);
            let layers: Vec<In> = (0..n)
                .map(|i| In { region: &regions[i], thickness: if i == 0 { 0.26 } else { 0.15 }, zc: planes[i] })
                .collect();
            let raw = plan_supports_ungrounded(&layers, &s);
            let areas: Vec<_> = raw.iter().map(|l| l.area.clone()).collect();
            let total: f64 = raw.iter().map(|l| zslice::geom::total_area(&l.area)).sum();
            let floating = floating_area(&layers, &areas, &s);
            let (_, removed) = plan_supports_stats(&layers, &s);
            println!(
                "{name:9} {kind:?}: support {:.0} mm2-couches, en suspension AVANT correction {:.1} mm2 ({:.1} %), retire par la correction {:.1} mm2",
                total / 1e6,
                floating / 1e6,
                100.0 * floating / total.max(1.0),
                removed / 1e6
            );
        }
    }
}
