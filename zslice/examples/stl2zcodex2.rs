//! stl2zcodex2 <in.stl> <out.zcodex2> [support] [noraft]
//! Places the model in the middle of the bed (resting on it) and slices it with default settings.
use zslice::{slice_to_zcodex2, Mesh, Settings};

fn parse_stl(data: &[u8]) -> Mesh {
    let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
    assert_eq!(data.len(), 84 + 50 * n, "only binary STL in this example");
    let f = |o: usize| f32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as f64;
    let mut m = Mesh::default();
    for t in 0..n {
        let b = 84 + 50 * t + 12;
        let base = m.vertices.len() as i32;
        for k in 0..3 {
            m.vertices.push([f(b + 12 * k), f(b + 12 * k + 4), f(b + 12 * k + 8)]);
        }
        m.triangles.push([base, base + 1, base + 2]);
    }
    m
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut mesh = parse_stl(&std::fs::read(&a[1]).expect("read"));
    let (lo, hi) = mesh.bounds();
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    for v in &mut mesh.vertices {
        v[0] += 100.0 - cx;
        v[1] += 100.0 - cy;
        v[2] -= lo[2];
    }
    let mut s = Settings::default();
    s.support = a.iter().any(|x| x == "support" || x == "tree");
    if a.iter().any(|x| x == "tree") {
        s.support_type = zslice::SupportType::Tree;
    }
    s.raft = !a.iter().any(|x| x == "noraft");
    let r = slice_to_zcodex2(&mesh, &s, placeholder_png(), None).expect("slice");
    r.save(&a[2]).expect("save");
    println!("{:?}", r.stats);
}

/// A valid 128x128 grey PNG (the GUI renders a real preview; examples use this placeholder).
fn placeholder_png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 128, 128);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        let px: Vec<u8> = (0..128 * 128).flat_map(|_| [60u8, 64, 72, 255]).collect();
        w.write_image_data(&px).unwrap();
    }
    out
}
