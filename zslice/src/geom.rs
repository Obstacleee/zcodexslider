//! 2D geometry on integer micrometre coordinates (thin layer over Clipper2).

use clipper2_rust::{
    difference_64, inflate_paths_64, intersect_64, simplify_paths, union_64, union_subjects_64, EndType, FillRule, JoinType, Point64,
};

pub type Pt = Point64;
pub type Poly = Vec<Point64>;
pub type Polys = Vec<Vec<Point64>>;

pub fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

pub fn pt(x: i64, y: i64) -> Pt {
    Point64::new(x, y)
}

pub fn dist(a: Pt, b: Pt) -> f64 {
    (((a.x - b.x) as f64).powi(2) + ((a.y - b.y) as f64).powi(2)).sqrt()
}

/// Signed area in µm² (positive = counter-clockwise).
pub fn area(p: &[Pt]) -> f64 {
    let n = p.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    for i in 0..n {
        let (p0, p1) = (p[i], p[(i + 1) % n]);
        a += p0.x as f64 * p1.y as f64 - p1.x as f64 * p0.y as f64;
    }
    a / 2.0
}

/// Perimeter of a closed polygon in µm.
pub fn perimeter(p: &[Pt]) -> f64 {
    (0..p.len()).map(|i| dist(p[i], p[(i + 1) % p.len()])).sum()
}

pub fn union(p: &Polys) -> Polys {
    union_subjects_64(p, FillRule::EvenOdd)
}

pub fn union2(a: &Polys, b: &Polys) -> Polys {
    union_64(a, b, FillRule::NonZero)
}

pub fn diff(a: &Polys, b: &Polys) -> Polys {
    if a.is_empty() || b.is_empty() {
        return a.clone();
    }
    difference_64(a, b, FillRule::NonZero)
}

pub fn inter(a: &Polys, b: &Polys) -> Polys {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    intersect_64(a, b, FillRule::NonZero)
}

/// Offsets a region by `delta` µm (positive grows, negative shrinks) with round corners.
pub fn offset(p: &Polys, delta: f64) -> Polys {
    if p.is_empty() {
        return Vec::new();
    }
    if delta == 0.0 {
        return p.clone();
    }
    inflate_paths_64(p, delta, JoinType::Round, EndType::Polygon, 2.0, 40.0)
}

pub fn simplify(p: &Polys, eps: f64) -> Polys {
    simplify_paths(p, eps, true).into_iter().filter(|x| x.len() >= 3).collect()
}

/// Removes parts thinner than `2*r` µm (morphological opening).
pub fn open(p: &Polys, r: f64) -> Polys {
    offset(&offset(p, -r), r)
}

/// Drops polygons whose absolute area is below `min_um2` (holes included, so tiny holes vanish too).
pub fn remove_small(p: &Polys, min_um2: f64) -> Polys {
    p.iter().filter(|x| area(x).abs() >= min_um2).cloned().collect()
}

pub fn total_area(p: &Polys) -> f64 {
    // outer rings are positive, holes negative after a Clipper operation
    p.iter().map(|x| area(x)).sum::<f64>().abs()
}

pub fn bbox(p: &Polys) -> Option<(i64, i64, i64, i64)> {
    let mut it = p.iter().flatten();
    let f = it.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (f.x, f.y, f.x, f.y);
    for q in it {
        x0 = x0.min(q.x);
        y0 = y0.min(q.y);
        x1 = x1.max(q.x);
        y1 = y1.max(q.y);
    }
    Some((x0, y0, x1, y1))
}

/// Even-odd point-in-polygon test against all rings.
pub fn contains(region: &Polys, x: f64, y: f64) -> bool {
    let mut inside = false;
    for ring in region {
        let n = ring.len();
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (ring[i], ring[j]);
            if (a.y as f64 > y) != (b.y as f64 > y) {
                let t = (y - a.y as f64) / (b.y as f64 - a.y as f64);
                if x < a.x as f64 + t * (b.x as f64 - a.x as f64) {
                    inside = !inside;
                }
            }
            j = i;
        }
    }
    inside
}

/// Parallel infill lines clipped to `region` (even-odd). `angle` in degrees, `spacing` in µm.
/// Lines sit on a global lattice (phase 0), so consecutive layers line up.
/// Returns segments grouped in rows (all segments of one scan line together), rows in scan order.
pub fn fill_lines(region: &Polys, angle: f64, spacing: f64) -> Vec<Vec<[Pt; 2]>> {
    let mut rows = Vec::new();
    if region.is_empty() || spacing < 50.0 {
        return rows;
    }
    let (s, c) = angle.to_radians().sin_cos();
    // rotate into the scan frame: u along the line, v across
    let to_uv = |p: Pt| (p.x as f64 * c + p.y as f64 * s, -(p.x as f64) * s + p.y as f64 * c);
    let from_uv = |u: f64, v: f64| pt((u * c - v * s).round() as i64, (u * s + v * c).round() as i64);

    let mut vmin = f64::MAX;
    let mut vmax = f64::MIN;
    let rings: Vec<Vec<(f64, f64)>> = region
        .iter()
        .map(|r| {
            r.iter()
                .map(|&p| {
                    let uv = to_uv(p);
                    vmin = vmin.min(uv.1);
                    vmax = vmax.max(uv.1);
                    uv
                })
                .collect()
        })
        .collect();

    let mut k = (vmin / spacing).ceil();
    while k * spacing <= vmax {
        let v = k * spacing + 0.5; // tiny shift avoids hitting vertices exactly
        let mut xs: Vec<f64> = Vec::new();
        for ring in &rings {
            let n = ring.len();
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                if (a.1 <= v) != (b.1 <= v) {
                    let t = (v - a.1) / (b.1 - a.1);
                    xs.push(a.0 + t * (b.0 - a.0));
                }
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut row = Vec::new();
        for pair in xs.chunks_exact(2) {
            if pair[1] - pair[0] > 20.0 {
                row.push([from_uv(pair[0], v), from_uv(pair[1], v)]);
            }
        }
        if !row.is_empty() {
            rows.push(row);
        }
        k += 1.0;
    }
    rows
}

#[cfg(test)]
pub fn square(x: i64, y: i64, s: i64) -> Poly {
    vec![pt(x, y), pt(x + s, y), pt(x + s, y + s), pt(x, y + s)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_shrinks_and_grows() {
        let r = vec![square(0, 0, 10_000)];
        let a = total_area(&offset(&r, -1000.0));
        assert!((a - 64e6).abs() < 1e5, "{a}");
        let b = total_area(&offset(&r, 1000.0));
        assert!(b > 142e6 && b < 144e6, "{b}");
    }

    #[test]
    fn fill_lines_covers_square_with_holes() {
        let mut r = vec![square(0, 0, 20_000)];
        let mut hole = square(5_000, 5_000, 10_000);
        hole.reverse();
        r.push(hole);
        let rows = fill_lines(&r, 0.0, 1000.0);
        let n: usize = rows.iter().map(|r| r.len()).sum();
        // rows crossing the hole give two segments, others one
        assert!(n > 20 && n < 40, "{n}");
        for row in &rows {
            for s in row {
                assert!(contains(&r, ((s[0].x + s[1].x) / 2) as f64, ((s[0].y + s[1].y) / 2) as f64));
            }
        }
    }

    #[test]
    fn contains_works_with_holes() {
        let mut r = vec![square(0, 0, 100)];
        let mut h = square(40, 40, 20);
        h.reverse();
        r.push(h);
        assert!(contains(&r, 10.0, 10.0));
        assert!(!contains(&r, 50.0, 50.0));
        assert!(!contains(&r, 500.0, 50.0));
    }
}
