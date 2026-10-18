use std::io::{Cursor, Read, Write};

use crate::error::{Error, Result};
use crate::record::{encode_records, parse_records, Record};

pub const ADDITIONAL_METADATA: &str = "AdditionalMetadata";
pub const ZCODE_DATA: &str = "ZCodeData";
pub const CONFIGURATION_DATA: &str = "ConfigurationData";
pub const USER_SETTINGS_DATA: &str = "UserSettingsData";

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A parsed `.zcodex2` file. ZIP entries are kept as raw bytes, in file order.
#[derive(Debug, Clone)]
pub struct Zcodex2 {
    /// Header metadata (`ZCodeX2Metadata`): printer, materials, `CommandCount`, ...
    pub metadata: serde_json::Value,
    pub preview_png: Vec<u8>,
    /// `(name, bytes)` of every ZIP entry, in order.
    pub entries: Vec<(String, Vec<u8>)>,
}

fn read_u32(data: &[u8], pos: &mut usize, what: &'static str) -> Result<usize> {
    let b = data.get(*pos..*pos + 4).ok_or(Error::Truncated(what))?;
    *pos += 4;
    Ok(u32::from_le_bytes(b.try_into().unwrap()) as usize)
}

fn take<'a>(data: &'a [u8], pos: &mut usize, n: usize, what: &'static str) -> Result<&'a [u8]> {
    let end = pos.checked_add(n).ok_or(Error::Truncated(what))?;
    let s = data.get(*pos..end).ok_or(Error::Truncated(what))?;
    *pos = end;
    Ok(s)
}

impl Zcodex2 {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut pos = 0;
        let n = read_u32(data, &mut pos, "metadata length")?;
        let metadata = serde_json::from_slice(take(data, &mut pos, n, "metadata")?)?;
        let n = read_u32(data, &mut pos, "preview length")?;
        let preview_png = take(data, &mut pos, n, "preview")?.to_vec();
        if !preview_png.starts_with(PNG_MAGIC) {
            return Err(Error::NotPng);
        }
        let _ = pos;
        let mut zip = zip::ZipArchive::new(Cursor::new(data))?;
        let mut entries = Vec::with_capacity(zip.len());
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            let mut buf = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut buf)?;
            entries.push((f.name().to_owned(), buf));
        }
        Ok(Self { metadata, preview_png, entries })
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::from_bytes(&std::fs::read(path)?)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let meta = serde_json::to_vec(&self.metadata)?;
        let mut out = Vec::new();
        out.extend_from_slice(&(meta.len() as u32).to_le_bytes());
        out.extend_from_slice(&meta);
        out.extend_from_slice(&(self.preview_png.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.preview_png);

        // Z-Suite writes the ZIP into the same stream after the header, so ZIP offsets are
        // absolute in the file. Keep that: put the prefix in the stream before the ZipWriter.
        let mut cur = Cursor::new(out);
        cur.set_position(cur.get_ref().len() as u64);
        let mut zip = zip::ZipWriter::new(cur);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(3)); // much faster than the default for little size
        for (name, bytes) in &self.entries {
            zip.start_file(name.as_str(), opts)?;
            zip.write_all(bytes)?;
        }
        Ok(zip.finish()?.into_inner())
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        Ok(std::fs::write(path, self.to_bytes()?)?)
    }

    pub fn entry(&self, name: &str) -> Option<&[u8]> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice())
    }

    /// Replaces an entry's bytes, or appends it if absent.
    pub fn set_entry(&mut self, name: &str, bytes: Vec<u8>) {
        match self.entries.iter_mut().find(|(n, _)| n == name) {
            Some((_, b)) => *b = bytes,
            None => self.entries.push((name.to_owned(), bytes)),
        }
    }

    /// Decodes the `ZCodeData` entry, checking every CRC.
    pub fn records(&self) -> Result<Vec<Record>> {
        parse_records(self.entry(ZCODE_DATA).ok_or(Error::Truncated("ZCodeData entry"))?)
    }

    pub fn set_records(&mut self, records: &[Record]) -> Result<()> {
        self.set_entry(ZCODE_DATA, encode_records(records)?);
        Ok(())
    }

    /// `CommandCount` from the header, if present.
    pub fn command_count(&self) -> Option<u64> {
        self.metadata.get("CommandCount")?.as_u64()
    }
}
