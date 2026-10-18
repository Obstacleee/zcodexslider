//! Small CPU renderer (z-buffer, flat shading) used for the 3D view and for the project preview PNG.

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub target: V3,
    pub radius: f32,
}

impl Camera {
    /// Looks at the middle of the bed from the front-left.
    pub fn for_bed(bed: [f64; 2]) -> Camera {
        Camera {
            yaw: -std::f32::consts::FRAC_PI_2 - 0.5,
            pitch: 0.55,
            zoom: 1.0,
            target: [bed[0] as f32 / 2.0, bed[1] as f32 / 2.0, 10.0],
            radius: bed[0].max(bed[1]) as f32 * 0.8,
        }
    }

    /// Aims at a bounding box and sizes the view to it.
    pub fn frame(&mut self, lo: [f64; 3], hi: [f64; 3]) {
        self.target = [
            ((lo[0] + hi[0]) / 2.0) as f32,
            ((lo[1] + hi[1]) / 2.0) as f32,
            ((lo[2] + hi[2]) / 2.0) as f32,
        ];
        let d = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() as f32;
        self.radius = (d * 0.75).max(5.0);
        self.zoom = 1.0;
    }

    fn basis(&self) -> (V3, V3, V3, V3) {
        let pitch = self.pitch.clamp(-1.5, 1.5);
        let dist = self.radius * 2.6 / self.zoom.max(0.05);
        let eye = [
            self.target[0] + dist * pitch.cos() * self.yaw.cos(),
            self.target[1] + dist * pitch.cos() * self.yaw.sin(),
            self.target[2] + dist * pitch.sin(),
        ];
        let fwd = norm(sub(self.target, eye));
        let right = norm(cross(fwd, [0.0, 0.0, 1.0]));
        let up = cross(right, fwd);
        (eye, fwd, right, up)
    }
}

pub struct Drawable<'a> {
    pub vertices: &'a [[f64; 3]],
    pub triangles: &'a [[i32; 3]],
    pub color: [u8; 3],
    /// When set, triangles with a vertex outside `[0, w] x [0, h]` (the plate) are drawn red.
    pub limit: Option<[f32; 2]>,
}

struct Frame {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
    depth: Vec<f32>,
}

impl Frame {
    fn put(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            let i = (y as usize * self.w + x as usize) * 4;
            self.rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), c: [u8; 3]) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let steps = dx.abs().max(dy.abs()).ceil().min(6000.0) as i32;
        if steps == 0 {
            return;
        }
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            self.put((a.0 + dx * t).round() as i32, (a.1 + dy * t).round() as i32, c);
        }
    }
}

/// A line segment drawn on top of the scene (support volumes, guides...).
#[derive(Clone)]
pub struct Wire {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub color: [u8; 3],
}

/// The 12 edges of an axis-aligned box.
pub fn box_wires(lo: [f64; 3], hi: [f64; 3], color: [u8; 3]) -> Vec<Wire> {
    let c = |x: usize, y: usize, z: usize| [if x == 0 { lo[0] } else { hi[0] } as f32, if y == 0 { lo[1] } else { hi[1] } as f32, if z == 0 { lo[2] } else { hi[2] } as f32];
    let mut out = Vec::new();
    for (a, b) in [
        ((0, 0, 0), (1, 0, 0)), ((0, 1, 0), (1, 1, 0)), ((0, 0, 1), (1, 0, 1)), ((0, 1, 1), (1, 1, 1)),
        ((0, 0, 0), (0, 1, 0)), ((1, 0, 0), (1, 1, 0)), ((0, 0, 1), (0, 1, 1)), ((1, 0, 1), (1, 1, 1)),
        ((0, 0, 0), (0, 0, 1)), ((1, 0, 0), (1, 0, 1)), ((0, 1, 0), (0, 1, 1)), ((1, 1, 0), (1, 1, 1)),
    ] {
        out.push(Wire { a: c(a.0, a.1, a.2), b: c(b.0, b.1, b.2), color });
    }
    out
}

/// Renders the scene to an RGBA buffer of `w * h` pixels.
pub fn render(items: &[Drawable], bed: [f32; 2], cam: &Camera, w: usize, h: usize) -> Vec<u8> {
    render_with(items, &[], bed, cam, w, h)
}

/// Like [`render`], with extra line segments drawn over the models.
pub fn render_with(items: &[Drawable], wires: &[Wire], bed: [f32; 2], cam: &Camera, w: usize, h: usize) -> Vec<u8> {
    let bg = [22u8, 24, 29];
    let rgba = (0..w * h).flat_map(|_| [bg[0], bg[1], bg[2], 255]).collect();
    let mut f = Frame { w, h, rgba, depth: vec![0.0; w * h] };

    let (eye, fwd, right, up) = cam.basis();
    let focal = (h as f32 / 2.0) / (35.0f32.to_radians() / 2.0).tan();
    let near = 0.5;
    let project = |p: V3| -> Option<(f32, f32, f32)> {
        let d = sub(p, eye);
        let z = dot(d, fwd);
        if z < near {
            return None;
        }
        Some((w as f32 / 2.0 + focal * dot(d, right) / z, h as f32 / 2.0 - focal * dot(d, up) / z, z))
    };

    // bed grid
    let grid = [48u8, 52, 60];
    let edge = [110u8, 118, 135];
    let seg = |f: &mut Frame, a: V3, b: V3, c: [u8; 3]| {
        if let (Some(pa), Some(pb)) = (project(a), project(b)) {
            f.line((pa.0, pa.1), (pb.0, pb.1), c);
        }
    };
    let mut x = 0.0;
    while x <= bed[0] + 0.01 {
        let c = if x == 0.0 || (x - bed[0]).abs() < 0.01 { edge } else { grid };
        seg(&mut f, [x, 0.0, 0.0], [x, bed[1], 0.0], c);
        x += 10.0;
    }
    let mut y = 0.0;
    while y <= bed[1] + 0.01 {
        let c = if y == 0.0 || (y - bed[1]).abs() < 0.01 { edge } else { grid };
        seg(&mut f, [0.0, y, 0.0], [bed[0], y, 0.0], c);
        y += 10.0;
    }

    // models
    let light = [-fwd[0], -fwd[1], -fwd[2]];
    for it in items {
        let proj: Vec<Option<(f32, f32, f32)>> = it
            .vertices
            .iter()
            .map(|v| project([v[0] as f32, v[1] as f32, v[2] as f32]))
            .collect();
        for t in it.triangles {
            let (Some(a), Some(b), Some(c)) = (proj[t[0] as usize], proj[t[1] as usize], proj[t[2] as usize]) else {
                continue;
            };
            let p = |i: i32| {
                let v = it.vertices[i as usize];
                [v[0] as f32, v[1] as f32, v[2] as f32]
            };
            let n = norm(cross(sub(p(t[1]), p(t[0])), sub(p(t[2]), p(t[0]))));
            let k = 0.28 + 0.72 * dot(n, light).abs();
            let out_of_plate = |v: [f32; 3]| it.limit.map_or(false, |lim| v[0] < -0.01 || v[1] < -0.01 || v[0] > lim[0] + 0.01 || v[1] > lim[1] + 0.01);
            let (p0, p1, p2) = (p(t[0]), p(t[1]), p(t[2]));
            let maybe_out = out_of_plate(p0) || out_of_plate(p1) || out_of_plate(p2);
            let col = it.color.map(|c| (c as f32 * k).min(255.0) as u8);
            let red = [255u8, 60, 60].map(|c| (c as f32 * k.max(0.55)).min(255.0) as u8);

            let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if area.abs() < 1e-6 {
                continue;
            }
            let min_x = a.0.min(b.0).min(c.0).floor().max(0.0) as i32;
            let max_x = a.0.max(b.0).max(c.0).ceil().min(w as f32 - 1.0) as i32;
            let min_y = a.1.min(b.1).min(c.1).floor().max(0.0) as i32;
            let max_y = a.1.max(b.1).max(c.1).ceil().min(h as f32 - 1.0) as i32;
            let (ia, ib, ic) = (1.0 / a.2, 1.0 / b.2, 1.0 / c.2);
            for py in min_y..=max_y {
                for px in min_x..=max_x {
                    let (sx, sy) = (px as f32 + 0.5, py as f32 + 0.5);
                    let w0 = ((b.0 - sx) * (c.1 - sy) - (b.1 - sy) * (c.0 - sx)) / area;
                    let w1 = ((c.0 - sx) * (a.1 - sy) - (c.1 - sy) * (a.0 - sx)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let inv = w0 * ia + w1 * ib + w2 * ic;
                    let di = py as usize * w + px as usize;
                    if inv > f.depth[di] {
                        f.depth[di] = inv;
                        let mut c = col;
                        if maybe_out {
                            // perspective-correct world position of this pixel
                            let (l0, l1, l2) = (w0 * ia / inv, w1 * ib / inv, w2 * ic / inv);
                            let q = [
                                l0 * p0[0] + l1 * p1[0] + l2 * p2[0],
                                l0 * p0[1] + l1 * p1[1] + l2 * p2[1],
                                0.0,
                            ];
                            if out_of_plate(q) {
                                c = red;
                            }
                        }
                        f.put(px, py, c);
                    }
                }
            }
        }
    }

    // axes at the bed origin
    for (end, c) in [([30.0, 0.0, 0.0], [230u8, 70, 70]), ([0.0, 30.0, 0.0], [80, 200, 90]), ([0.0, 0.0, 30.0], [90, 140, 240])] {
        seg(&mut f, [0.0, 0.0, 0.0], end, c);
    }
    for wr in wires {
        let (Some(a), Some(b)) = (project(wr.a), project(wr.b)) else { continue };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let steps = dx.abs().max(dy.abs()).ceil().clamp(1.0, 6000.0) as i32;
        let (ia, ib) = (1.0 / a.2, 1.0 / b.2);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = ((a.0 + dx * t).round() as i32, (a.1 + dy * t).round() as i32);
            if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                continue;
            }
            let inv = ia + (ib - ia) * t;
            // visible unless the model is clearly in front of this point
            if inv >= f.depth[y as usize * w + x as usize] * 0.995 {
                f.put(x, y, wr.color);
            }
        }
    }
    f.rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_triangle_in_front_of_camera() {
        let cam = Camera::for_bed([200.0, 200.0]);
        let v = [[90.0, 90.0, 0.0], [110.0, 90.0, 0.0], [100.0, 100.0, 30.0]];
        let t = [[0, 1, 2]];
        let img = render(&[Drawable { vertices: &v, triangles: &t, color: [200, 200, 200], limit: None }], [200.0, 200.0], &cam, 320, 240);
        assert_eq!(img.len(), 320 * 240 * 4);
        let bright = img.chunks(4).filter(|p| p[0] > 120 && p[1] > 120).count();
        assert!(bright > 50, "model should cover some pixels, got {bright}");
    }

    #[test]
    fn only_the_part_outside_the_plate_is_red() {
        // a flat square straddling the left edge of a 200 mm plate
        let v = [[-30.0, 80.0, 0.0], [30.0, 80.0, 0.0], [30.0, 120.0, 0.0], [-30.0, 120.0, 0.0]];
        let t = [[0, 1, 2], [0, 2, 3]];
        let mut cam = Camera::for_bed([200.0, 200.0]);
        cam.frame([-40.0, 70.0, 0.0], [40.0, 130.0, 10.0]);
        cam.pitch = 1.4;
        let img = render(&[Drawable { vertices: &v, triangles: &t, color: [90, 150, 255], limit: Some([200.0, 200.0]) }], [200.0, 200.0], &cam, 400, 300);
        let red = img.chunks(4).filter(|p| p[0] > 150 && p[1] < 90 && p[2] < 90).count();
        let blue = img.chunks(4).filter(|p| p[2] > 120 && p[0] < 100).count();
        assert!(red > 500, "red pixels {red}");
        assert!(blue > 500, "blue pixels {blue}");
    }

    #[test]
    fn overlay_lines_are_hidden_behind_the_model() {
        let cam = Camera::for_bed([200.0, 200.0]);
        let v = [[90.0, 90.0, 0.0], [110.0, 90.0, 0.0], [110.0, 90.0, 40.0], [90.0, 90.0, 40.0]];
        let t = [[0, 1, 2], [0, 2, 3]];
        let wall = Drawable { vertices: &v, triangles: &t, color: [200, 200, 200], limit: None };
        let green = [0u8, 255, 0];
        // behind the wall as seen from the camera (camera is at -y): hidden; in front: visible
        let behind = Wire { a: [95.0, 100.0, 20.0], b: [105.0, 100.0, 20.0], color: green };
        let front = Wire { a: [95.0, 80.0, 20.0], b: [105.0, 80.0, 20.0], color: green };
        let count = |w: &Wire| render_with(&[Drawable { vertices: &v, triangles: &t, color: wall.color, limit: None }], std::slice::from_ref(w), [200.0, 200.0], &cam, 500, 400).chunks(4).filter(|p| p[1] > 240 && p[0] < 30).count();
        assert!(count(&front) >= 10, "front wire should be visible");
        assert!(count(&behind) <= 2, "wire behind the wall must be hidden (only edge pixels may leak)");
    }
}
