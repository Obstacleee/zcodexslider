//! Reader for the subset of MS-NRBF (.NET `BinaryFormatter`) that Z-Suite produces.
//!
//! Produces a resolved [`Value`] tree. Object graphs with shared references are expanded
//! (cycles are cut at a fixed depth and become [`Value::Null`]).

use std::collections::HashMap;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Object { class: String, fields: Vec<(String, Value)> },
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        if let Value::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        if let Value::Int(i) = self { Some(*i) } else { None }
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self { Some(*b) } else { None }
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        if let Value::Bytes(b) = self { Some(b) } else { None }
    }
    /// Field of an object. Auto-property backing fields (`<Name>k__BackingField`) match `Name`.
    pub fn field(&self, name: &str) -> Option<&Value> {
        let Value::Object { fields, .. } = self else { return None };
        let backing = format!("<{name}>k__BackingField");
        fields.iter().find(|(n, _)| n == name || *n == backing).map(|(_, v)| v)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ClassDef {
    pub name: String,
    pub members: Vec<String>,
    types: Vec<(u8, Extra)>,
}

#[derive(Debug, Clone)]
enum Extra {
    None,
    Prim(u8),
}

#[derive(Debug, Clone)]
enum Raw {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Ref(i32),
    List(Vec<Raw>),            // array elements
    Fields(i32, Vec<Raw>),     // class instance: class id, member values
}

pub(crate) struct Parsed {
    root: i32,
    pub classes: HashMap<i32, ClassDef>,
    objs: HashMap<i32, Raw>,
}

fn bad(msg: &str) -> Error {
    Error::Nrbf(msg.to_owned())
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
    classes: HashMap<i32, ClassDef>,
    objs: HashMap<i32, Raw>,
}

/// Parses a whole NRBF stream into a [`Value`] tree rooted at the stream's root object.
pub fn parse(data: &[u8]) -> Result<Value> {
    let parsed = parse_raw(data)?;
    Ok(parsed.resolve_root())
}

pub(crate) fn parse_raw(data: &[u8]) -> Result<Parsed> {
    let mut r = Reader { d: data, p: 0, classes: HashMap::new(), objs: HashMap::new() };
    let root = r.run()?;
    Ok(Parsed { root, classes: r.classes, objs: r.objs })
}

impl Parsed {
    pub(crate) fn resolve_root(&self) -> Value {
        self.resolve(&Raw::Ref(self.root), 0)
    }

    fn resolve(&self, v: &Raw, depth: u32) -> Value {
        if depth > 48 {
            return Value::Null;
        }
        match v {
            Raw::Null => Value::Null,
            Raw::Bool(b) => Value::Bool(*b),
            Raw::Int(i) => Value::Int(*i),
            Raw::Float(f) => Value::Float(*f),
            Raw::Str(s) => Value::Str(s.clone()),
            Raw::Bytes(b) => Value::Bytes(b.clone()),
            Raw::Ref(id) => match self.objs.get(id) {
                Some(o) => self.resolve(o, depth + 1),
                None => Value::Null,
            },
            Raw::List(items) => Value::Array(items.iter().map(|x| self.resolve(x, depth + 1)).collect()),
            Raw::Fields(cid, vals) => {
                let def = &self.classes[cid];
                Value::Object {
                    class: def.name.clone(),
                    fields: def
                        .members
                        .iter()
                        .cloned()
                        .zip(vals.iter().map(|x| self.resolve(x, depth + 1)))
                        .collect(),
                }
            }
        }
    }
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Result<u8> {
        self.d.get(self.p).copied().ok_or_else(|| bad("unexpected end"))
    }
    fn u8(&mut self) -> Result<u8> {
        let b = self.peek()?;
        self.p += 1;
        Ok(b)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).ok_or_else(|| bad("length overflow"))?;
        let s = self.d.get(self.p..end).ok_or_else(|| bad("unexpected end"))?;
        self.p = end;
        Ok(s)
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn lp(&mut self) -> Result<String> {
        let (mut n, mut shift) = (0usize, 0);
        loop {
            let b = self.u8()?;
            n |= ((b & 0x7f) as usize) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                return Err(bad("bad string length"));
            }
        }
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }

    fn prim(&mut self, t: u8) -> Result<Raw> {
        Ok(match t {
            1 => Raw::Bool(self.u8()? != 0),
            2 => Raw::Int(self.u8()? as i64),
            10 => Raw::Int(self.u8()? as i8 as i64),
            7 => Raw::Int(i16::from_le_bytes(self.take(2)?.try_into().unwrap()) as i64),
            14 => Raw::Int(u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as i64),
            8 => Raw::Int(self.i32()? as i64),
            15 => Raw::Int(u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as i64),
            9 | 12 | 13 => Raw::Int(i64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            16 => Raw::Int(u64::from_le_bytes(self.take(8)?.try_into().unwrap()) as i64),
            11 => Raw::Float(f32::from_le_bytes(self.take(4)?.try_into().unwrap()) as f64),
            6 => Raw::Float(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            3 => {
                // Char: one UTF-8 encoded scalar
                let first = self.u8()?;
                let extra = match first {
                    0..=0x7f => 0,
                    0xc0..=0xdf => 1,
                    0xe0..=0xef => 2,
                    _ => 3,
                };
                let mut buf = vec![first];
                buf.extend(self.take(extra)?);
                Raw::Str(String::from_utf8_lossy(&buf).into_owned())
            }
            5 | 18 => Raw::Str(self.lp()?),
            _ => return Err(bad("unsupported primitive type")),
        })
    }

    fn class_def(&mut self, with_lib: bool) -> Result<(i32, ClassDef)> {
        let id = self.i32()?;
        let name = self.lp()?;
        let count = self.i32()?;
        if !(0..4096).contains(&count) {
            return Err(bad("bad member count"));
        }
        let members = (0..count).map(|_| self.lp()).collect::<Result<Vec<_>>>()?;
        let kinds = (0..count).map(|_| self.u8()).collect::<Result<Vec<_>>>()?;
        let mut types = Vec::new();
        for k in kinds {
            let extra = match k {
                0 | 7 => Extra::Prim(self.u8()?),
                3 => {
                    self.lp()?;
                    Extra::None
                }
                4 => {
                    self.lp()?;
                    self.i32()?;
                    Extra::None
                }
                _ => Extra::None,
            };
            types.push((k, extra));
        }
        if with_lib {
            self.i32()?;
        }
        Ok((id, ClassDef { name, members, types }))
    }

    fn fields(&mut self, id: i32, class_id: i32) -> Result<Raw> {
        let def = self.classes.get(&class_id).cloned().ok_or_else(|| bad("unknown class"))?;
        let mut vals: Vec<Raw> = Vec::new();
        let mut pending_nulls = 0u32;
        for (k, extra) in &def.types {
            if pending_nulls > 0 {
                pending_nulls -= 1;
                vals.push(Raw::Null);
                continue;
            }
            let v = match (k, extra) {
                (0, Extra::Prim(t)) => self.prim(*t)?,
                _ => match self.peek()? {
                    13 => {
                        self.u8()?;
                        pending_nulls = self.u8()? as u32 - 1;
                        Raw::Null
                    }
                    14 => {
                        self.u8()?;
                        pending_nulls = (self.i32()? as u32).saturating_sub(1);
                        Raw::Null
                    }
                    _ => self.value()?,
                },
            };
            vals.push(v);
        }
        let obj = Raw::Fields(class_id, vals);
        self.objs.insert(id, obj);
        Ok(Raw::Ref(id))
    }

    fn array_elems(&mut self, total: i64) -> Result<Vec<Raw>> {
        if !(0..=(1 << 28)).contains(&total) {
            return Err(bad("bad array length"));
        }
        let mut elems = Vec::new();
        let mut i = 0;
        while i < total {
            match self.peek()? {
                13 => {
                    self.u8()?;
                    let n = self.u8()? as i64;
                    elems.extend(std::iter::repeat(Raw::Null).take(n as usize));
                    i += n;
                }
                14 => {
                    self.u8()?;
                    let n = self.i32()? as i64;
                    elems.extend(std::iter::repeat(Raw::Null).take(n.max(0) as usize));
                    i += n;
                }
                _ => {
                    elems.push(self.value()?);
                    i += 1;
                }
            }
        }
        Ok(elems)
    }

    /// Reads one record that stands for a value.
    fn value(&mut self) -> Result<Raw> {
        let t = self.u8()?;
        match t {
            9 => Ok(Raw::Ref(self.i32()?)),
            10 => Ok(Raw::Null),
            8 => {
                // MemberPrimitiveTyped (boxed primitive)
                let pt = self.u8()?;
                self.prim(pt)
            }
            12 => {
                // BinaryLibrary declared in the middle of an object graph
                self.i32()?;
                self.lp()?;
                self.value()
            }
            6 => {
                let id = self.i32()?;
                let s = self.lp()?;
                self.objs.insert(id, Raw::Str(s));
                Ok(Raw::Ref(id))
            }
            4 | 5 => {
                let (id, def) = self.class_def(t == 5)?;
                self.classes.insert(id, def);
                self.fields(id, id)
            }
            1 => {
                let id = self.i32()?;
                let meta = self.i32()?;
                self.fields(id, meta)
            }
            15 => {
                let id = self.i32()?;
                let len = self.i32()?;
                let pt = self.u8()?;
                if !(0..=(1 << 30)).contains(&len) {
                    return Err(bad("bad array length"));
                }
                let obj = if pt == 2 {
                    Raw::Bytes(self.take(len as usize)?.to_vec())
                } else {
                    Raw::List((0..len).map(|_| self.prim(pt)).collect::<Result<Vec<_>>>()?)
                };
                self.objs.insert(id, obj);
                Ok(Raw::Ref(id))
            }
            16 | 17 => {
                // ArraySingleObject / ArraySingleString
                let id = self.i32()?;
                let len = self.i32()?;
                let elems = self.array_elems(len as i64)?;
                self.objs.insert(id, Raw::List(elems));
                Ok(Raw::Ref(id))
            }
            7 => {
                let id = self.i32()?;
                let at = self.u8()?;
                let rank = self.i32()?;
                if !(1..=8).contains(&rank) {
                    return Err(bad("bad array rank"));
                }
                let lens = (0..rank).map(|_| self.i32()).collect::<Result<Vec<_>>>()?;
                if at >= 3 {
                    for _ in 0..rank {
                        self.i32()?;
                    }
                }
                let et = self.u8()?;
                let mut prim = None;
                match et {
                    0 | 7 => prim = Some(self.u8()?),
                    3 => {
                        self.lp()?;
                    }
                    4 => {
                        self.lp()?;
                        self.i32()?;
                    }
                    _ => {}
                }
                let total: i64 = lens.iter().map(|&l| l as i64).product();
                let elems = if let (0, Some(pt)) = (et, prim) {
                    if !(0..=(1 << 28)).contains(&total) {
                        return Err(bad("bad array length"));
                    }
                    (0..total).map(|_| self.prim(pt)).collect::<Result<Vec<_>>>()?
                } else {
                    self.array_elems(total)?
                };
                self.objs.insert(id, Raw::List(elems));
                Ok(Raw::Ref(id))
            }
            _ => Err(Error::Nrbf(format!("unsupported record type {t} at offset {}", self.p - 1))),
        }
    }

    fn run(&mut self) -> Result<i32> {
        if self.u8()? != 0 {
            return Err(bad("missing stream header"));
        }
        let root = self.i32()?;
        self.take(12)?;
        loop {
            match self.peek()? {
                11 => return Ok(root),
                12 => {
                    self.u8()?;
                    self.i32()?;
                    self.lp()?;
                }
                _ => {
                    self.value()?;
                }
            }
        }
    }
}

fn stream_header(o: &mut Vec<u8>) {
    o.push(0);
    o.extend(1i32.to_le_bytes()); // root id
    o.extend((-1i32).to_le_bytes()); // header id
    o.extend(1i32.to_le_bytes()); // major
    o.extend(0i32.to_le_bytes()); // minor
}

/// Serialises a root-level `string` the way `BinaryFormatter` does.
pub fn write_string(s: &str) -> Vec<u8> {
    let mut o = Vec::new();
    stream_header(&mut o);
    o.push(6);
    o.extend(1i32.to_le_bytes());
    let mut n = s.len();
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            o.push(b);
            break;
        }
        o.push(b | 0x80);
    }
    o.extend(s.as_bytes());
    o.push(11);
    o
}

/// Serialises a root-level `byte[]` the way `BinaryFormatter` does.
pub fn write_byte_array(data: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(data.len() + 28);
    stream_header(&mut o);
    o.push(15);
    o.extend(1i32.to_le_bytes());
    o.extend((data.len() as i32).to_le_bytes());
    o.push(2);
    o.extend_from_slice(data);
    o.push(11);
    o
}
