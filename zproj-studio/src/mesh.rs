//! A model on the print bed: original mesh + scale / rotation / position, and the placement
//! rules Z-Suite uses (model rests on the bed, `Move` = centre of its bounding box).

use crate::stl::Mesh;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use zcodex2::Model;

#[derive(Debug, Clone)]
pub struct Item {
    pub name: String,
    /// Original geometry, re-centred on the middle of its bounding box.
    pub mesh: Mesh,
    pub scale: [f64; 3],
    /// Rotation in degrees about X, Y, Z (applied in that order).
    pub rot: [f64; 3],
    /// Bed position (mm) of the centre of the model's footprint.
    pub pos: [f64; 2],
    /// Height of the lowest point above the bed (mm). 0 = resting on the bed.
    pub lift: f64,
    pub unrepaired: bool,
    pub id: [u8; 16],
    placed: Vec<[f64; 3]>,
}

pub fn bbox(v: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in v {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    (lo, hi)
}

pub fn random_guid() -> [u8; 16] {
    let mut id = [0u8; 16];
    for half in id.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_u8(1);
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    id[7] = (id[7] & 0x0f) | 0x40; // version 4 (group 3 is little-endian in Guid.ToByteArray)
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

impl Item {
    pub fn new(name: impl Into<String>, mut mesh: Mesh, pos: [f64; 2]) -> Item {
        let (lo, hi) = bbox(&mesh.vertices);
        let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
        for v in &mut mesh.vertices {
            for i in 0..3 {
                v[i] -= c[i];
            }
        }
        let mut it = Item {
            name: name.into(),
            mesh,
            scale: [1.0; 3],
            rot: [0.0; 3],
            pos,
            lift: 0.0,
            unrepaired: false,
            id: random_guid(),
            placed: Vec::new(),
        };
        it.update();
        it
    }

    /// Builds an item from a model read out of a project (vertices are already placed).
    pub fn from_model(m: &Model) -> Item {
        let mesh = Mesh { vertices: m.vertices.clone(), triangles: m.triangles.clone() };
        let (lo, hi) = bbox(&mesh.vertices);
        let mut it = Item::new(m.name.clone(), mesh, [m.translation[0], m.translation[1]]);
        it.lift = if m.vertices.is_empty() { 0.0 } else { lo[2].max(0.0) };
        it.unrepaired = m.unrepaired;
        it.id = m.id;
        let _ = hi;
        it.update();
        it
    }

    /// Recomputes the placed geometry after any parameter changed.
    pub fn update(&mut self) {
        let (sx, sy, sz) = (self.scale[0], self.scale[1], self.scale[2]);
        let (a, b, c) = (self.rot[0].to_radians(), self.rot[1].to_radians(), self.rot[2].to_radians());
        let (sa, ca, sb, cb, sc, cc) = (a.sin(), a.cos(), b.sin(), b.cos(), c.sin(), c.cos());
        let mut out: Vec<[f64; 3]> = self
            .mesh
            .vertices
            .iter()
            .map(|v| {
                let (x, y, z) = (v[0] * sx, v[1] * sy, v[2] * sz);
                let (y, z) = (y * ca - z * sa, y * sa + z * ca); // about X
                let (x, z) = (x * cb + z * sb, -x * sb + z * cb); // about Y
                let (x, y) = (x * cc - y * sc, x * sc + y * cc); // about Z
                [x, y, z]
            })
            .collect();
        if !out.is_empty() {
            let (lo, hi) = bbox(&out);
            let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
            for v in &mut out {
                v[0] += self.pos[0] - cx;
                v[1] += self.pos[1] - cy;
                v[2] += self.lift - lo[2];
            }
        }
        self.placed = out;
    }

    /// Geometry in bed coordinates (what Z-Suite stores in the project).
    pub fn placed(&self) -> &[[f64; 3]] {
        &self.placed
    }

    /// (min, max) corners in bed coordinates.
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        bbox(&self.placed)
    }

    pub fn size(&self) -> [f64; 3] {
        let (lo, hi) = self.bounds();
        [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
    }

    pub fn to_model(&self) -> Model {
        let (lo, hi) = self.bounds();
        Model {
            name: self.name.clone(),
            vertices: self.placed.clone(),
            triangles: self.mesh.triangles.clone(),
            translation: [self.pos[0], self.pos[1], self.lift + (hi[2] - lo[2]) / 2.0],
            unrepaired: self.unrepaired,
            id: self.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stl::cube;

    #[test]
    fn rests_on_bed_and_centres_on_position() {
        let mut it = Item::new("c.stl", cube(10.0), [100.0, 100.0]);
        let (lo, hi) = it.bounds();
        assert_eq!((lo, hi), ([95.0, 95.0, 0.0], [105.0, 105.0, 10.0]));
        it.scale = [2.0, 1.0, 0.5];
        it.pos = [50.0, 60.0];
        it.update();
        let (lo, hi) = it.bounds();
        assert!((lo[0] - 40.0).abs() < 1e-9 && (hi[0] - 60.0).abs() < 1e-9);
        assert!((lo[2]).abs() < 1e-9 && (hi[2] - 5.0).abs() < 1e-9);
        let m = it.to_model();
        assert_eq!(m.translation, [50.0, 60.0, 2.5]);
    }

    #[test]
    fn rotation_by_90_swaps_extents() {
        let mut it = Item::new("b.stl", Mesh { vertices: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 2.0, 1.0]], triangles: vec![[0, 1, 2]] }, [0.0, 0.0]);
        it.rot = [0.0, 0.0, 90.0];
        it.update();
        let s = it.size();
        assert!((s[0] - 2.0).abs() < 1e-9 && (s[1] - 10.0).abs() < 1e-9);
    }

    #[test]
    fn guid_has_v4_bits() {
        let g = random_guid();
        assert_eq!(g[7] >> 4, 4);
        assert_eq!(g[8] >> 6, 2);
        assert_ne!(random_guid(), random_guid());
    }
}
