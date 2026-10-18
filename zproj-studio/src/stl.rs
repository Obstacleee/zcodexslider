//! STL import (binary and ASCII) into an indexed mesh with welded vertices,
//! the representation Z-Suite stores in a project.

use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[i32; 3]>,
}

#[derive(Default)]
struct Welder {
    map: HashMap<[u64; 3], i32>,
    mesh: Mesh,
    dropped: usize,
}

impl Welder {
    fn index(&mut self, p: [f64; 3]) -> i32 {
        // -0.0 and 0.0 must weld together
        let key = p.map(|v| if v == 0.0 { 0u64 } else { v.to_bits() });
        *self.map.entry(key).or_insert_with(|| {
            self.mesh.vertices.push(p);
            self.mesh.vertices.len() as i32 - 1
        })
    }

    fn triangle(&mut self, a: [f64; 3], b: [f64; 3], c: [f64; 3]) {
        if a.iter().chain(&b).chain(&c).any(|v| !v.is_finite()) {
            self.dropped += 1;
            return;
        }
        let (i, j, k) = (self.index(a), self.index(b), self.index(c));
        if i == j || j == k || i == k {
            self.dropped += 1; // degenerate after welding
        } else {
            self.mesh.triangles.push([i, j, k]);
        }
    }
}

pub struct Imported {
    pub mesh: Mesh,
    /// Triangles skipped because they were degenerate or contained NaN/inf.
    pub dropped: usize,
}

pub fn parse(data: &[u8]) -> Result<Imported, String> {
    let binary = data.len() >= 84 && {
        let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
        data.len() == 84 + 50 * n
    };
    let mut w = Welder::default();
    if binary {
        let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
        let f = |o: usize| f32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as f64;
        for t in 0..n {
            let base = 84 + 50 * t + 12; // skip the normal
            let v = |k: usize| [f(base + 12 * k), f(base + 12 * k + 4), f(base + 12 * k + 8)];
            w.triangle(v(0), v(1), v(2));
        }
    } else {
        let text = std::str::from_utf8(data).map_err(|_| "not a valid STL file (neither binary nor ASCII)")?;
        if !text.trim_start().to_ascii_lowercase().starts_with("solid") {
            return Err("not a valid STL file (neither binary nor ASCII)".into());
        }
        let mut pts: Vec<[f64; 3]> = Vec::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if it.next().map(|t| t.eq_ignore_ascii_case("vertex")) == Some(true) {
                let c: Vec<f64> = it.map(|t| t.parse::<f64>().map_err(|_| format!("bad number {t:?}"))).collect::<Result<_, _>>()?;
                if c.len() != 3 {
                    return Err(format!("bad vertex line: {line:?}"));
                }
                pts.push([c[0], c[1], c[2]]);
            }
        }
        if pts.is_empty() || pts.len() % 3 != 0 {
            return Err("ASCII STL has no complete facets".into());
        }
        for t in pts.chunks(3) {
            w.triangle(t[0], t[1], t[2]);
        }
    }
    if w.mesh.triangles.is_empty() {
        return Err("the file contains no usable triangles".into());
    }
    Ok(Imported { mesh: w.mesh, dropped: w.dropped })
}

/// Binary STL writer (used by tests and handy for exporting).
pub fn to_binary(mesh: &Mesh) -> Vec<u8> {
    let mut o = vec![0u8; 80];
    o.extend((mesh.triangles.len() as u32).to_le_bytes());
    for t in &mesh.triangles {
        o.extend([0u8; 12]);
        for &i in t {
            for c in mesh.vertices[i as usize] {
                o.extend((c as f32).to_le_bytes());
            }
        }
        o.extend([0u8; 2]);
    }
    o
}

#[cfg(test)]
pub(crate) fn cube(size: f64) -> Mesh {
    let s = size;
    let vertices = vec![
        [0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, s, 0.0], [0.0, s, 0.0],
        [0.0, 0.0, s], [s, 0.0, s], [s, s, s], [0.0, s, s],
    ];
    let triangles = vec![
        [0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4],
        [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7],
    ];
    Mesh { vertices, triangles }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_roundtrip_welds_vertices() {
        let m = cube(10.0);
        let back = parse(&to_binary(&m)).unwrap();
        assert_eq!(back.mesh.vertices.len(), 8);
        assert_eq!(back.mesh.triangles.len(), 12);
        assert_eq!(back.dropped, 0);
    }

    #[test]
    fn ascii_parses() {
        let txt = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\n\
                   facet normal 0 0 1\nouter loop\nvertex 1 0 0\nvertex 1 1 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let r = parse(txt.as_bytes()).unwrap();
        assert_eq!((r.mesh.vertices.len(), r.mesh.triangles.len()), (4, 2));
    }

    #[test]
    fn rejects_garbage_and_drops_degenerate() {
        assert!(parse(b"hello world").is_err());
        let mut m = cube(1.0);
        m.triangles.push([0, 0, 1]);
        let r = parse(&to_binary(&m)).unwrap();
        assert_eq!(r.mesh.triangles.len(), 12);
    }
}
