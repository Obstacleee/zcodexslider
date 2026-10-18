//! Mesh -> per-layer cross-section regions.

use std::collections::HashMap;

use crate::geom::{self, pt, Polys};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    /// Vertices in mm, bed coordinates (Z up, z = 0 is the plate).
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[i32; 3]>,
}

impl Mesh {
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for v in &self.vertices {
            for i in 0..3 {
                lo[i] = lo[i].min(v[i]);
                hi[i] = hi[i].max(v[i]);
            }
        }
        (lo, hi)
    }
}

type Key = (i64, i64);

fn key(x: f64, y: f64) -> Key {
    (x.round() as i64, y.round() as i64)
}

/// Cross-section of the mesh at height `z` (mm), as closed loops in µm.
fn section(mesh: &Mesh, tris: &[usize], z: f64) -> Vec<Vec<(f64, f64)>> {
    // intersection segments
    let mut segs: Vec<((f64, f64), (f64, f64))> = Vec::new();
    for &t in tris {
        let tri = mesh.triangles[t];
        let v = [mesh.vertices[tri[0] as usize], mesh.vertices[tri[1] as usize], mesh.vertices[tri[2] as usize]];
        let above = [v[0][2] >= z, v[1][2] >= z, v[2][2] >= z];
        let n_above = above.iter().filter(|&&a| a).count();
        if n_above == 0 || n_above == 3 {
            continue;
        }
        let mut pts: Vec<(f64, f64)> = Vec::with_capacity(2);
        for i in 0..3 {
            let j = (i + 1) % 3;
            if above[i] != above[j] {
                let t = (z - v[i][2]) / (v[j][2] - v[i][2]);
                pts.push((
                    (v[i][0] + t * (v[j][0] - v[i][0])) * 1000.0,
                    (v[i][1] + t * (v[j][1] - v[i][1])) * 1000.0,
                ));
            }
        }
        if pts.len() == 2 && key(pts[0].0, pts[0].1) != key(pts[1].0, pts[1].1) {
            // keep a consistent winding: material on the left when walking the segment
            let a_is_lone = n_above == 1;
            let _ = a_is_lone;
            segs.push((pts[0], pts[1]));
        }
    }

    // chain segments into loops (undirected, matching endpoints at 1 µm)
    let mut by_end: HashMap<Key, Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        by_end.entry(key(s.0 .0, s.0 .1)).or_default().push(i);
        by_end.entry(key(s.1 .0, s.1 .1)).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut loops = Vec::new();
    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut chain = vec![segs[start].0, segs[start].1];
        loop {
            let tail = *chain.last().unwrap();
            let k = key(tail.0, tail.1);
            let next = by_end.get(&k).and_then(|c| c.iter().copied().find(|&i| !used[i]));
            let Some(i) = next else { break };
            used[i] = true;
            let s = segs[i];
            if key(s.0 .0, s.0 .1) == k {
                chain.push(s.1);
            } else {
                chain.push(s.0);
            }
        }
        let first = chain[0];
        let last = *chain.last().unwrap();
        let closed = key(first.0, first.1) == key(last.0, last.1)
            || ((first.0 - last.0).powi(2) + (first.1 - last.1).powi(2)).sqrt() < 100.0;
        if closed && chain.len() >= 4 {
            chain.pop();
            loops.push(chain);
        }
    }
    loops
}

/// Slices the mesh at each plane height (mm). Returns one region per plane.
pub fn slice(mesh: &Mesh, planes: &[f64]) -> Vec<Polys> {
    if planes.is_empty() {
        return Vec::new();
    }
    // bucket triangles by the planes they cross
    let mut per_plane: Vec<Vec<usize>> = vec![Vec::new(); planes.len()];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let zs = [mesh.vertices[tri[0] as usize][2], mesh.vertices[tri[1] as usize][2], mesh.vertices[tri[2] as usize][2]];
        let (lo, hi) = (zs.iter().cloned().fold(f64::MAX, f64::min), zs.iter().cloned().fold(f64::MIN, f64::max));
        // first plane >= lo, last plane <= hi (planes are sorted ascending)
        let first = planes.partition_point(|&p| p < lo);
        let last = planes.partition_point(|&p| p <= hi);
        for k in first..last {
            per_plane[k].push(t);
        }
    }
    planes
        .iter()
        .zip(per_plane)
        .map(|(&z, tris)| {
            let loops = section(mesh, &tris, z);
            let polys: Polys = loops
                .into_iter()
                .map(|l| l.into_iter().map(|(x, y)| pt(x.round() as i64, y.round() as i64)).collect())
                .collect();
            geom::simplify(&geom::union(&polys), 5.0)
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn cube_mesh(x: f64, y: f64, s: f64) -> Mesh {
    let v = vec![
        [x, y, 0.0], [x + s, y, 0.0], [x + s, y + s, 0.0], [x, y + s, 0.0],
        [x, y, s], [x + s, y, s], [x + s, y + s, s], [x, y + s, s],
    ];
    let t = vec![
        [0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4],
        [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7],
    ];
    Mesh { vertices: v, triangles: t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_section_is_a_square() {
        let m = cube_mesh(10.0, 20.0, 30.0);
        let layers = slice(&m, &[0.075, 15.0, 29.9]);
        for l in &layers {
            assert_eq!(l.len(), 1);
            let a = geom::total_area(l);
            assert!((a - 900e6).abs() < 1e3, "{a}");
            assert_eq!(geom::bbox(l), Some((10_000, 20_000, 40_000, 50_000)));
        }
    }

    #[test]
    fn planes_outside_the_mesh_are_empty() {
        let m = cube_mesh(0.0, 0.0, 10.0);
        let layers = slice(&m, &[-1.0, 11.0]);
        assert!(layers.iter().all(|l| l.is_empty()));
    }
}
