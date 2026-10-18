//! support_side <shape: t|sphere|arch> <normal|tree> <out.png>
//! Side view (X horizontal, Z up) of the support structure: support contours in cyan, the model's X extent in grey.
use zslice::toolpath::{plan, role};
use zslice::{Mesh, Settings, SupportType};

fn boxm(verts: &mut Vec<[f64; 3]>, tris: &mut Vec<[i32; 3]>, x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) {
    let b = verts.len() as i32;
    verts.extend([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]);
    for t in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
        tris.push([t[0] + b, t[1] + b, t[2] + b]);
    }
}

fn sphere(cx: f64, cy: f64, cz: f64, r: f64) -> Mesh {
    let (nu, nv) = (48usize, 24usize);
    let mut v = Vec::new();
    for j in 0..=nv {
        let phi = std::f64::consts::PI * j as f64 / nv as f64;
        for i in 0..nu {
            let th = std::f64::consts::TAU * i as f64 / nu as f64;
            v.push([cx + r * phi.sin() * th.cos(), cy + r * phi.sin() * th.sin(), cz - r * phi.cos()]);
        }
    }
    let mut t = Vec::new();
    for j in 0..nv {
        for i in 0..nu {
            let a = (j * nu + i) as i32;
            let b = (j * nu + (i + 1) % nu) as i32;
            let c = ((j + 1) * nu + i) as i32;
            let d = ((j + 1) * nu + (i + 1) % nu) as i32;
            if j > 0 {
                t.push([a, b, c]);
            }
            if j < nv - 1 {
                t.push([b, d, c]);
            }
        }
    }
    Mesh { vertices: v, triangles: t }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mesh = match a[1].as_str() {
        "sphere" => {
            // a ball resting on a thin foot so the whole lower half needs support
            let mut m = sphere(100.0, 100.0, 40.0, 20.0);
            let (mut v, mut t) = (m.vertices.clone(), m.triangles.clone());
            boxm(&mut v, &mut t, 97.0, 97.0, 0.0, 103.0, 103.0, 22.0);
            m = Mesh { vertices: v, triangles: t };
            m
        }
        "arch" => {
            // two pillars with a bridge deck on top and a small knob hanging off one side
            let (mut v, mut t) = (Vec::new(), Vec::new());
            boxm(&mut v, &mut t, 70.0, 90.0, 0.0, 80.0, 110.0, 25.0);
            boxm(&mut v, &mut t, 120.0, 90.0, 0.0, 130.0, 110.0, 25.0);
            boxm(&mut v, &mut t, 70.0, 90.0, 25.0, 130.0, 110.0, 31.0);
            boxm(&mut v, &mut t, 130.0, 95.0, 15.0, 145.0, 105.0, 21.0);
            Mesh { vertices: v, triangles: t }
        }
        _ => {
            let (mut v, mut t) = (Vec::new(), Vec::new());
            boxm(&mut v, &mut t, 95.0, 95.0, 0.0, 105.0, 105.0, 20.0);
            boxm(&mut v, &mut t, 60.0, 85.0, 20.0, 140.0, 115.0, 26.0);
            Mesh { vertices: v, triangles: t }
        }
    };
    let mut s = Settings::default();
    s.raft = false;
    s.support = true;
    s.support_type = if a[2] == "tree" { SupportType::Tree } else { SupportType::Normal };
    let p = plan(&mesh, &s).unwrap();

    let (w, h, sc) = (1000usize, 700usize, 6.0f64);
    let mut img = vec![[20u8, 22, 27]; w * h];
    let ox = 100.0 - w as f64 / sc / 2.0;
    let mut put = |x: f64, z: f64, c: [u8; 3]| {
        let (px, py) = (((x - ox) * sc) as i64, h as i64 - 20 - (z * sc) as i64);
        if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
            img[py as usize * w + px as usize] = c;
        }
    };
    let is_sup = |r: u8| matches!(r, role::SUPPORT_CONTOUR | role::SUPPORT_INFILL | role::INTERFACE_INFILL);
    // model silhouettes first (each wall loop on its own), supports on top
    for l in &p.layers {
        let z = l.z as f64 / 1000.0;
        for path in l.paths.iter().filter(|x| !is_sup(x.role) && x.role == role::OUTER) {
            let (mut x0, mut x1) = (f64::MAX, f64::MIN);
            for q in &path.pts {
                x0 = x0.min(q.x as f64 / 1000.0);
                x1 = x1.max(q.x as f64 / 1000.0);
            }
            for i in 0..=((x1 - x0) * sc) as i64 {
                put(x0 + i as f64 / sc, z, [95, 95, 110]);
            }
        }
    }
    let mut n_support = 0usize;
    let mut total_len = 0.0;
    for l in &p.layers {
        let z = l.z as f64 / 1000.0;
        for path in l.paths.iter().filter(|x| is_sup(x.role)) {
            for q in &path.pts {
                put(q.x as f64 / 1000.0, z, if path.role == role::INTERFACE_INFILL { [255, 255, 255] } else { [80, 230, 230] });
                n_support += 1;
            }
            total_len += path.pts.windows(2).map(|w| (((w[1].x - w[0].x) as f64).hypot((w[1].y - w[0].y) as f64)) / 1000.0).sum::<f64>();
        }
    }
    println!("support points {n_support}, support path length {:.0} mm", total_len);
    let mut raw = Vec::new();
    for px in img {
        raw.extend(px);
        raw.push(255);
    }
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w as u32, h as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(&raw).unwrap();
    }
    std::fs::write(&a[3], out).unwrap();
}
