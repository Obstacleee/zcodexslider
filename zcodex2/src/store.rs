//! Key/value store used by Z-Suite for print profiles (`.zprof`) and projects (`.zproj`/`.zprojx`).
//!
//! On disk: `gzip( MS-NRBF( Dictionary<string, byte[]> ) )` (.NET `BinaryFormatter`).
//! Values are opaque here. Z-Suite serialises them per key:
//! - `"Models"` -> protobuf-net,
//! - `HollowHoles`, `HeightAreas`, `PreSlicedSupportStructures`, `ResinSupportStructures`,
//!   `ResinLayers`, `OriginalResinLayers`, `SupportCrosses`, `Bridges` -> MessagePack,
//! - anything else (`Defaults`, `ProjectVersion`, `Printer.Model`, ...) -> nested `BinaryFormatter`.

use std::io::{Read, Write};

use crate::error::{Error, Result};
use crate::nrbf::{self, Value};

const MSCORLIB: &str = "mscorlib, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089";

fn dict_name() -> String {
    format!(
        "System.Collections.Generic.Dictionary`2[[System.String, {MSCORLIB}],[System.Byte[], {MSCORLIB}]]"
    )
}
fn comparer_name() -> String {
    format!("System.Collections.Generic.GenericEqualityComparer`1[[System.String, {MSCORLIB}]]")
}
fn kvp_name() -> String {
    format!("System.Collections.Generic.KeyValuePair`2[[System.String, {MSCORLIB}],[System.Byte[], {MSCORLIB}]]")
}

/// Decoded store: `Dictionary<string, byte[]>` plus the dictionary's own bookkeeping fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    /// `Dictionary.Version` field (mutation counter). Kept so that a re-save is byte-identical.
    pub version: i32,
    /// `Dictionary.HashSize` field (bucket count, a prime).
    pub hash_size: i32,
    pub entries: Vec<(String, Vec<u8>)>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Store { version: 0, hash_size: 0, entries: Vec::new() }
    }

    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice())
    }

    pub fn set(&mut self, key: &str, value: Vec<u8>) {
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.entries.push((key.to_owned(), value)),
        }
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::from_gzip(&std::fs::read(path)?)
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        Ok(std::fs::write(path, self.to_gzip()?)?)
    }

    pub fn from_gzip(data: &[u8]) -> Result<Self> {
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(data).read_to_end(&mut raw)?;
        Self::from_nrbf(&raw)
    }

    /// Compresses like .NET's `GZipStream` (fast level, OS byte 0).
    pub fn to_gzip(&self) -> Result<Vec<u8>> {
        let raw = self.to_nrbf();
        let mut enc = flate2::GzBuilder::new()
            .operating_system(0)
            .write(Vec::new(), flate2::Compression::fast());
        enc.write_all(&raw)?;
        Ok(enc.finish()?)
    }

    // ---------------------------------------------------------------- writer

    pub fn to_nrbf(&self) -> Vec<u8> {
        let n = self.entries.len() as i32;
        let hash_size = if self.hash_size > 0 { self.hash_size } else { next_prime(n.max(3)) };
        let version = if self.version > 0 { self.version } else { n };

        let mut o = Vec::new();
        // SerializedStreamHeader
        o.push(0);
        put_i32(&mut o, 1); // root id
        put_i32(&mut o, -1); // header id
        put_i32(&mut o, 1); // major
        put_i32(&mut o, 0); // minor

        // Dictionary (SystemClassWithMembersAndTypes), id 1
        o.push(4);
        put_i32(&mut o, 1);
        put_lp(&mut o, &dict_name());
        put_i32(&mut o, 4);
        for name in ["Version", "Comparer", "HashSize", "KeyValuePairs"] {
            put_lp(&mut o, name);
        }
        o.extend([0, 3, 0, 3]); // Primitive, SystemClass, Primitive, SystemClass
        o.push(8); // Int32
        put_lp(&mut o, &comparer_name());
        o.push(8);
        put_lp(&mut o, &format!("{}[]", kvp_name()));
        put_i32_member_ref(&mut o, None, version, 2, hash_size, 3);

        // Comparer object, id 2
        o.push(4);
        put_i32(&mut o, 2);
        put_lp(&mut o, &comparer_name());
        put_i32(&mut o, 0);
        // (no members -> empty MemberTypeInfo)

        // KeyValuePair array, id 3
        o.push(7);
        put_i32(&mut o, 3);
        o.push(0); // Single
        put_i32(&mut o, 1); // rank
        put_i32(&mut o, n);
        o.push(3); // SystemClass
        put_lp(&mut o, &kvp_name());

        let kvp_meta_id = -4;
        for (i, (key, _)) in self.entries.iter().enumerate() {
            let i = i as i32;
            let kvp_id = -(4 + 3 * i);
            let key_id = 5 + 3 * i;
            let val_id = 6 + 3 * i;
            if i == 0 {
                o.push(4);
                put_i32(&mut o, kvp_id);
                put_lp(&mut o, &kvp_name());
                put_i32(&mut o, 2);
                put_lp(&mut o, "key");
                put_lp(&mut o, "value");
                o.extend([1, 7]); // String, PrimitiveArray
                o.push(2); // element: Byte
            } else {
                o.push(1);
                put_i32(&mut o, kvp_id);
                put_i32(&mut o, kvp_meta_id);
            }
            o.push(6);
            put_i32(&mut o, key_id);
            put_lp(&mut o, key);
            o.push(9);
            put_i32(&mut o, val_id);
        }
        for (i, (_, val)) in self.entries.iter().enumerate() {
            o.push(15);
            put_i32(&mut o, 6 + 3 * i as i32);
            put_i32(&mut o, val.len() as i32);
            o.push(2);
            o.extend_from_slice(val);
        }
        o.push(11); // MessageEnd
        o
    }

    // ---------------------------------------------------------------- reader

    pub fn from_nrbf(data: &[u8]) -> Result<Self> {
        let root = nrbf::parse(data)?;
        let bad = |m: &str| Error::Nrbf(m.to_owned());
        match &root {
            Value::Object { class, .. } if class.starts_with("System.Collections.Generic.Dictionary`2") => {}
            _ => return Err(bad("root is not a Dictionary<,>")),
        }
        let version = root.field("Version").and_then(Value::as_i64).ok_or_else(|| bad("no Version"))? as i32;
        let hash_size = root.field("HashSize").and_then(Value::as_i64).ok_or_else(|| bad("no HashSize"))? as i32;
        let Some(Value::Array(pairs)) = root.field("KeyValuePairs") else {
            return Err(bad("KeyValuePairs is not an array"));
        };
        let mut entries = Vec::new();
        for kv in pairs {
            let key = kv.field("key").and_then(Value::as_str).ok_or_else(|| bad("key is not a string"))?;
            let value = match kv.field("value") {
                Some(Value::Bytes(b)) => b.clone(),
                Some(Value::Null) | None => Vec::new(),
                _ => return Err(bad("value is not a byte array")),
            };
            entries.push((key.to_owned(), value));
        }
        Ok(Store { version, hash_size, entries })
    }

    /// Parses a value stored with Z-Suite's nested `BinaryFormatter` (strings, numbers, arrays, objects).
    pub fn value(&self, key: &str) -> Result<Option<Value>> {
        self.get(key).map(nrbf::parse).transpose()
    }

    /// Decodes the `"Models"` entry (protobuf-net `List<Model3>`).
    pub fn models(&self) -> Result<Vec<crate::models::Model>> {
        match self.get("Models") {
            Some(b) => crate::models::decode_models(b),
            None => Ok(Vec::new()),
        }
    }

    pub fn set_models(&mut self, models: &[crate::models::Model]) {
        self.set("Models", crate::models::encode_models(models));
    }
}

fn next_prime(mut n: i32) -> i32 {
    fn is_prime(n: i32) -> bool {
        n >= 2 && (2..).take_while(|d| d * d <= n).all(|d| n % d != 0)
    }
    while !is_prime(n) {
        n += 1;
    }
    n
}

fn put_i32(o: &mut Vec<u8>, v: i32) {
    o.extend(v.to_le_bytes());
}

fn put_lp(o: &mut Vec<u8>, s: &str) {
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
}

/// Dictionary member values: Version (i32), Comparer (ref), HashSize (i32), KeyValuePairs (ref).
fn put_i32_member_ref(o: &mut Vec<u8>, _unused: Option<()>, version: i32, comparer: i32, hash: i32, array: i32) {
    put_i32(o, version);
    o.push(9);
    put_i32(o, comparer);
    put_i32(o, hash);
    o.push(9);
    put_i32(o, array);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_synthetic() {
        for n in 0..6 {
            let mut s = Store::new();
            for i in 0..n {
                s.set(&format!("key{i}"), vec![i as u8; i * 7]);
            }
            let back = Store::from_gzip(&s.to_gzip().unwrap()).unwrap();
            assert_eq!(back.entries, s.entries, "n={n}");
            // second pass keeps the stored bookkeeping and is byte-stable
            assert_eq!(back.to_nrbf(), Store::from_nrbf(&back.to_nrbf()).unwrap().to_nrbf());
        }
    }

    #[test]
    fn prime() {
        assert_eq!(next_prime(3), 3);
        assert_eq!(next_prime(4), 5);
    }
}
