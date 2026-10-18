//! The `"Models"` entry of a Z-Suite project: protobuf-net serialisation of `ObservableCollection<Model3>`.
//!
//! Schema (from the `[ProtoContract]` attributes in Z-SUITE.dll):
//! ```text
//! root:      repeated Model3 = 1
//! Model3:    repeated Point3 Vertices = 1; repeated TriangleIndices Triangles = 2; Point3 Move = 3;
//!            string Name = 4; bool Unrepaired = 5; Guid Id = 7
//! Point3:    double X = 1; double Y = 2; double Z = 3
//! TriangleIndices: int32 V1 = 1; V2 = 2; V3 = 3
//! Guid:      fixed64 lo = 1; fixed64 hi = 2   (the two halves of Guid.ToByteArray())
//! ```
//! Verified on a project written by Z-Suite 3.7: decoding is lenient (missing fields = default,
//! unknown fields skipped) and re-encoding reproduces the original bytes.

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub name: String,
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[i32; 3]>,
    pub translation: [f64; 3],
    pub unrepaired: bool,
    /// `Guid.ToByteArray()` layout (first three groups little-endian).
    pub id: [u8; 16],
}

impl Default for Model {
    fn default() -> Self {
        Model {
            name: String::new(),
            vertices: Vec::new(),
            triangles: Vec::new(),
            translation: [0.0; 3],
            unrepaired: false,
            id: [0; 16],
        }
    }
}

fn bad(msg: &str) -> Error {
    Error::Protobuf(msg.to_owned())
}

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn done(&self) -> bool {
        self.p >= self.d.len()
    }
    fn varint(&mut self) -> Result<u64> {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let b = *self.d.get(self.p).ok_or_else(|| bad("truncated varint"))?;
            self.p += 1;
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
            if shift > 63 {
                return Err(bad("varint too long"));
            }
        }
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).ok_or_else(|| bad("length overflow"))?;
        let s = self.d.get(self.p..end).ok_or_else(|| bad("truncated field"))?;
        self.p = end;
        Ok(s)
    }
    fn fixed64(&mut self) -> Result<[u8; 8]> {
        Ok(self.bytes(8)?.try_into().unwrap())
    }
    /// Next field header: (field number, wire type).
    fn key(&mut self) -> Result<(u32, u8)> {
        let k = self.varint()?;
        Ok(((k >> 3) as u32, (k & 7) as u8))
    }
    fn skip(&mut self, wire: u8) -> Result<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => {
                self.bytes(8)?;
            }
            2 => {
                let n = self.varint()? as usize;
                self.bytes(n)?;
            }
            5 => {
                self.bytes(4)?;
            }
            _ => return Err(bad("unsupported wire type")),
        }
        Ok(())
    }
}

fn point3(d: &[u8]) -> Result<[f64; 3]> {
    let mut r = Rd { d, p: 0 };
    let mut out = [0.0; 3];
    while !r.done() {
        let (f, w) = r.key()?;
        match (f, w) {
            (1..=3, 1) => out[f as usize - 1] = f64::from_le_bytes(r.fixed64()?),
            _ => r.skip(w)?,
        }
    }
    Ok(out)
}

fn triangle(d: &[u8]) -> Result<[i32; 3]> {
    let mut r = Rd { d, p: 0 };
    let mut out = [0; 3];
    while !r.done() {
        let (f, w) = r.key()?;
        match (f, w) {
            (1..=3, 0) => out[f as usize - 1] = r.varint()? as i64 as i32,
            _ => r.skip(w)?,
        }
    }
    Ok(out)
}

fn guid(d: &[u8]) -> Result<[u8; 16]> {
    let mut r = Rd { d, p: 0 };
    let mut out = [0u8; 16];
    while !r.done() {
        let (f, w) = r.key()?;
        match (f, w) {
            (1, 1) => out[..8].copy_from_slice(&r.fixed64()?),
            (2, 1) => out[8..].copy_from_slice(&r.fixed64()?),
            _ => r.skip(w)?,
        }
    }
    Ok(out)
}

fn model(d: &[u8]) -> Result<Model> {
    let mut r = Rd { d, p: 0 };
    let mut m = Model::default();
    while !r.done() {
        let (f, w) = r.key()?;
        if w == 2 {
            let n = r.varint()? as usize;
            let body = r.bytes(n)?;
            match f {
                1 => m.vertices.push(point3(body)?),
                2 => m.triangles.push(triangle(body)?),
                3 => m.translation = point3(body)?,
                4 => m.name = String::from_utf8_lossy(body).into_owned(),
                7 => m.id = guid(body)?,
                _ => {}
            }
        } else if (f, w) == (5, 0) {
            m.unrepaired = r.varint()? != 0;
        } else {
            r.skip(w)?;
        }
    }
    Ok(m)
}

/// Decodes the `"Models"` store entry.
pub fn decode_models(data: &[u8]) -> Result<Vec<Model>> {
    let mut r = Rd { d: data, p: 0 };
    let mut out = Vec::new();
    while !r.done() {
        let (f, w) = r.key()?;
        if (f, w) == (1, 2) {
            let n = r.varint()? as usize;
            out.push(model(r.bytes(n)?)?);
        } else {
            r.skip(w)?;
        }
    }
    Ok(out)
}

fn put_varint(o: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        o.push((v as u8) | 0x80);
        v >>= 7;
    }
    o.push(v as u8);
}

fn put_msg(o: &mut Vec<u8>, field: u32, body: &[u8]) {
    put_varint(o, ((field as u64) << 3) | 2);
    put_varint(o, body.len() as u64);
    o.extend_from_slice(body);
}

fn enc_point3(p: &[f64; 3]) -> Vec<u8> {
    let mut o = Vec::with_capacity(27);
    for (i, v) in p.iter().enumerate() {
        if *v == 0.0 {
            continue; // protobuf-net omits default values
        }
        put_varint(&mut o, ((i as u64 + 1) << 3) | 1);
        o.extend(v.to_le_bytes());
    }
    o
}

/// Encodes models the way protobuf-net writes a `List<Model3>` (default/zero values omitted;
/// byte-identical to Z-Suite 3.7 output on the sample project).
pub fn encode_models(models: &[Model]) -> Vec<u8> {
    let mut out = Vec::new();
    for m in models {
        let mut b = Vec::new();
        for v in &m.vertices {
            put_msg(&mut b, 1, &enc_point3(v));
        }
        for t in &m.triangles {
            let mut tb = Vec::new();
            for (i, v) in t.iter().enumerate() {
                if *v == 0 {
                    continue;
                }
                put_varint(&mut tb, (i as u64 + 1) << 3);
                put_varint(&mut tb, *v as i64 as u64);
            }
            put_msg(&mut b, 2, &tb);
        }
        put_msg(&mut b, 3, &enc_point3(&m.translation));
        if !m.name.is_empty() {
            put_msg(&mut b, 4, m.name.as_bytes());
        }
        if m.unrepaired {
            put_varint(&mut b, 5 << 3);
            put_varint(&mut b, 1);
        }
        if m.id != [0; 16] {
            let mut g = Vec::new();
            for (field, half) in [(1u64, &m.id[..8]), (2, &m.id[8..])] {
                if half != [0; 8] {
                    put_varint(&mut g, (field << 3) | 1);
                    g.extend_from_slice(half);
                }
            }
            put_msg(&mut b, 7, &g);
        }
        put_msg(&mut out, 1, &b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let m = Model {
            name: "cube".into(),
            vertices: vec![[0.0, 1.5, -2.0], [10.0, 0.0, 0.25]],
            triangles: vec![[0, 1, 2], [2, 1, -1]],
            translation: [5.0, 6.0, 7.0],
            unrepaired: true,
            id: *b"0123456789abcdef",
        };
        let bytes = encode_models(&[m.clone(), Model::default()]);
        let back = decode_models(&bytes).unwrap();
        assert_eq!(back, vec![m, Model::default()]);
    }

    #[test]
    fn skips_unknown_fields() {
        // field 9 varint, then a valid empty model
        let mut b = vec![(9 << 3), 5];
        put_msg(&mut b, 1, &[]);
        assert_eq!(decode_models(&b).unwrap(), vec![Model::default()]);
    }
}
