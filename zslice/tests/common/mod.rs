//! Shared test meshes (all inside the 200 x 200 mm bed, resting on it).
#![allow(dead_code)]

use zslice::Mesh;

pub fn boxes(list: &[[f64; 6]]) -> Mesh {
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

pub fn cube() -> Mesh {
    boxes(&[[60.0, 60.0, 0.0, 90.0, 90.0, 12.0]])
}

pub fn tee() -> Mesh {
    boxes(&[[95.0, 95.0, 0.0, 105.0, 105.0, 14.0], [70.0, 85.0, 14.0, 130.0, 115.0, 18.0]])
}

pub fn arch() -> Mesh {
    boxes(&[[70.0, 90.0, 0.0, 80.0, 110.0, 15.0], [120.0, 90.0, 0.0, 130.0, 110.0, 15.0], [70.0, 90.0, 15.0, 130.0, 110.0, 19.0]])
}

/// A ball on a thin foot (curved overhang).
pub fn ball() -> Mesh {
    let (cx, cy, cz, r) = (100.0, 100.0, 24.0, 12.0);
    let (nu, nv) = (32usize, 16usize);
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

/// Close to the plate's edges, so raft, brim and supports have to be clipped.
pub fn edge() -> Mesh {
    boxes(&[[3.0, 120.0, 0.0, 14.0, 131.0, 14.0], [3.0, 120.0, 14.0, 50.0, 150.0, 18.0]])
}

pub fn all() -> Vec<(&'static str, Mesh)> {
    vec![("cube", cube()), ("tee", tee()), ("arch", arch()), ("ball", ball()), ("edge", edge())]
}

/// A valid preview image.
pub fn png() -> Vec<u8> {
    zslice::top_view_png(&[], [200.0, 200.0], 32)
}
