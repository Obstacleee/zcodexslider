//! Top-view preview image of toolpaths (used as the `.zcodex2` preview when a G-code file is converted).

use crate::toolpath::{role, Layer};

fn color(r: u8) -> [u8; 3] {
    match r {
        role::OUTER | role::FIRST_CONTOUR => [255, 130, 90],
        role::INNER => [240, 190, 70],
        role::INFILL => [90, 160, 240],
        role::TOP => [120, 230, 130],
        role::BOTTOM | role::FIRST_FILL => [190, 120, 250],
        role::BRIDGE => [255, 90, 200],
        role::SUPPORT_CONTOUR | role::SUPPORT_INFILL | role::INTERFACE_INFILL => [90, 220, 220],
        role::BRIM => [255, 170, 170],
        role::RAFT | role::RAFT_BOTTOM | role::RAFT_GRID | role::RAFT_INTERFACE => [160, 160, 160],
        _ => [200, 200, 200],
    }
}

/// Renders every layer from bottom to top into a `size` x `size` RGBA image.
pub fn top_view_rgba(layers: &[Layer], bed: [f64; 2], size: usize) -> Vec<u8> {
    let bg = [22u8, 24, 29];
    let mut px = vec![[bg[0], bg[1], bg[2], 255u8]; size * size];
    let pad = 6.0;
    let scale = ((size as f64 - 2.0 * pad) / bed[0]).min((size as f64 - 2.0 * pad) / bed[1]);
    let map = |x: i64, y: i64| (pad + x as f64 / 1000.0 * scale, size as f64 - pad - y as f64 / 1000.0 * scale);
    let mut put = |x: f64, y: f64, c: [u8; 3]| {
        let (ix, iy) = (x.round() as i64, y.round() as i64);
        if ix >= 0 && iy >= 0 && (ix as usize) < size && (iy as usize) < size {
            px[iy as usize * size + ix as usize] = [c[0], c[1], c[2], 255];
        }
    };
    // bed outline
    for k in 0..=size * 2 {
        let t = k as f64 / (size * 2) as f64;
        for (x, y) in [(t * bed[0], 0.0), (t * bed[0], bed[1]), (0.0, t * bed[1]), (bed[0], t * bed[1])] {
            let (sx, sy) = map((x * 1000.0) as i64, (y * 1000.0) as i64);
            put(sx, sy, [70, 74, 84]);
        }
    }
    let top = layers.len().max(1) as f64;
    for (li, l) in layers.iter().enumerate() {
        let shade = 0.55 + 0.45 * li as f64 / top;
        for p in &l.paths {
            let c = color(p.role).map(|v| (v as f64 * shade).min(255.0) as u8);
            for w in p.pts.windows(2) {
                let (a, b) = (map(w[0].x, w[0].y), map(w[1].x, w[1].y));
                let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
                for i in 0..=n {
                    let t = i as f64 / n as f64;
                    put(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, c);
                }
            }
        }
    }
    px.into_iter().flatten().collect()
}

pub fn top_view_png(layers: &[Layer], bed: [f64; 2], size: usize) -> Vec<u8> {
    let rgba = top_view_rgba(layers, bed, size);
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, size as u32, size as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().expect("png header");
        w.write_image_data(&rgba).expect("png data");
    }
    out
}
