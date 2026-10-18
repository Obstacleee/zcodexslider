//! bench: time of each export phase on a few shapes (supports on).   cargo run --release --example bench
use std::time::Instant;
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
        ("balle 20k tri", ball(25.0, 200, 100)),
        ("gros T 60mm", boxes(&[[80.0, 80.0, 0.0, 120.0, 120.0, 40.0], [30.0, 60.0, 40.0, 170.0, 140.0, 60.0]])),
    ];
    for (name, mesh) in &shapes {
        for (kind, label) in [(SupportType::Normal, "normal"), (SupportType::Tree, "arbre")] {
            let mut s = Settings::default();
            s.support = true;
            s.support_type = kind;
            let t = Instant::now();
            let plan = zslice::toolpath::plan(mesh, &s).unwrap();
            let t_plan = t.elapsed();
            let t = Instant::now();
            let _ = zslice::emit::emit(&plan, &s).unwrap();
            let t_emit = t.elapsed();
            let t = Instant::now();
            let png = zslice::preview::top_view_png(&plan.layers, [200.0, 200.0], 256);
            let _ = zslice::slice_to_zcodex2(mesh, &s, png, None).unwrap();
            let t_all = t.elapsed();
            println!(
                "{name:14} {label:7} {} tri, {} couches | plan {:>6.2?}  emit {:>6.2?}  export complet {:>6.2?}",
                mesh.triangles.len(),
                plan.layers.len(),
                t_plan,
                t_emit,
                t_all
            );
        }
    }
}
