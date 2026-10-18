//! layer_png <project.zprojx|tshape> <layer index> <out.png> [support]
use zslice::toolpath::{plan, role};
use zslice::{Mesh, Settings};

fn color(r: u8) -> [u8; 3] {
    match r {
        role::OUTER | role::FIRST_CONTOUR => [255, 120, 80],
        role::INNER => [240, 190, 70],
        role::INFILL => [90, 170, 255],
        role::TOP => [120, 230, 130],
        role::BOTTOM | role::FIRST_FILL => [200, 120, 255],
        role::BRIDGE => [255, 80, 200],
        role::SUPPORT_CONTOUR | role::SUPPORT_INFILL => [90, 230, 230],
        role::INTERFACE_INFILL => [255, 255, 255],
        role::RAFT | role::RAFT_BOTTOM | role::RAFT_GRID | role::RAFT_INTERFACE => [170, 170, 170],
        role::BRIM => [255, 160, 160],
        _ => [200, 200, 200],
    }
}

fn tshape() -> Mesh {
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    let mut boxm = |x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64| {
        let b = verts.len() as i32;
        verts.extend([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]);
        for t in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
            tris.push([t[0] + b, t[1] + b, t[2] + b]);
        }
    };
    boxm(90.0, 90.0, 0.0, 110.0, 110.0, 20.0);
    boxm(60.0, 85.0, 20.0, 140.0, 115.0, 26.0);
    Mesh { vertices: verts, triangles: tris }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mesh = if a[1] == "tshape" {
        tshape()
    } else {
        let m = &zcodex2::Store::open(&a[1]).unwrap().models().unwrap()[0];
        Mesh { vertices: m.vertices.clone(), triangles: m.triangles.clone() }
    };
    let idx: usize = a[2].parse().unwrap();
    let mut s = Settings::default();
    s.support = a.iter().any(|x| x == "support" || x == "tree");
    s.raft = !a.iter().any(|x| x == "noraft");
    if a.iter().any(|x| x == "tree") {
        s.support_type = zslice::SupportType::Tree;
    }
    let p = plan(&mesh, &s).unwrap();
    let l = &p.layers[idx.min(p.layers.len() - 1)];
    println!("layer {} marker {} z {} paths {}", idx, l.marker, l.z, l.paths.len());
    let (w, h, sc) = (900usize, 900usize, 4.5f64); // px per mm
    let mut img = vec![[20u8, 22, 27]; w * h];
    let put = |img: &mut Vec<[u8; 3]>, x: f64, y: f64, c: [u8; 3]| {
        let (px, py) = ((x / 1000.0 * sc) as i64, h as i64 - 1 - (y / 1000.0 * sc) as i64);
        if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
            img[py as usize * w + px as usize] = c;
        }
    };
    let mut seen = std::collections::HashMap::new();
    for path in &l.paths {
        *seen.entry(path.role).or_insert(0usize) += 1;
        for k in 1..path.pts.len() {
            let (a, b) = (path.pts[k - 1], path.pts[k]);
            let n = ((((b.x - a.x) as f64).hypot((b.y - a.y) as f64)) / 1000.0 * sc * 2.0).ceil().max(1.0) as i32;
            for i in 0..=n {
                let t = i as f64 / n as f64;
                put(&mut img, a.x as f64 + (b.x - a.x) as f64 * t, a.y as f64 + (b.y - a.y) as f64 * t, color(path.role));
            }
        }
    }
    println!("roles: {seen:?}");
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
        // raw is RGB+A already; rebuild properly
        wr.write_image_data(&raw).unwrap();
    }
    std::fs::write(&a[3], out).unwrap();
}
