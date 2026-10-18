use crate::crc::crc8;
use crate::error::{Error, Result};

/// One raw record of the `ZCodeData` stream: `[len][kind][payload][crc8]`
/// where `len` counts everything except the CRC byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub kind: u8,
    pub payload: Vec<u8>,
    /// Stored bit-inverted (kind > 127 on disk). Never seen in samples; handled per Z-Suite's reader.
    pub flipped: bool,
}

impl Record {
    pub fn new(kind: u8, payload: Vec<u8>) -> Self {
        Self { kind, payload, flipped: false }
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<()> {
        if self.payload.len() > 253 {
            return Err(Error::PayloadTooLarge(self.payload.len()));
        }
        let start = out.len();
        out.push((self.payload.len() + 2) as u8);
        out.push(self.kind);
        out.extend_from_slice(&self.payload);
        out.push(crc8(&out[start..]));
        if self.flipped {
            out[start..].iter_mut().for_each(|b| *b ^= 0xFF);
        }
        Ok(())
    }

    pub fn decode(&self) -> Command {
        Command::from_record(self)
    }
}

/// Parses a whole `ZCodeData` stream, verifying every CRC.
pub fn parse_records(data: &[u8]) -> Result<Vec<Record>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 2 <= data.len() {
        let (mut len, mut kind) = (data[p], data[p + 1]);
        let flipped = kind > 127;
        if flipped {
            len ^= 0xFF;
            kind ^= 0xFF;
        }
        if len == 0 || kind == 0 {
            break; // terminator, as in Z-Suite's reader
        }
        if len < 2 {
            return Err(Error::BadRecord { offset: p, reason: "length < 2" });
        }
        let end = p + len as usize + 1;
        if end > data.len() {
            return Err(Error::BadRecord { offset: p, reason: "record exceeds stream" });
        }
        let mut rec = data[p..end].to_vec();
        if flipped {
            rec.iter_mut().for_each(|b| *b ^= 0xFF);
        }
        let stored = rec[len as usize];
        let computed = crc8(&rec[..len as usize]);
        if stored != computed {
            return Err(Error::BadCrc { offset: p, stored, computed });
        }
        out.push(Record { kind, payload: rec[2..len as usize].to_vec(), flipped });
        p = end;
    }
    Ok(out)
}

pub fn encode_records(records: &[Record]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for r in records {
        r.encode_into(&mut out)?;
    }
    Ok(out)
}

/// Linear move (kind 1). Coordinates are integers in the printer's internal units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Move {
    /// Toolpath role (enum in Z-Suite: 0 = model contour, 5 = support infill, 10 = raft, ...).
    pub role: u8,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub z: Option<i32>,
    pub e: Option<i32>,
    pub fan1: Option<i32>,
    pub fan2: Option<i32>,
    pub z2: Option<i32>,
    pub tmp4: Option<i32>,
}

/// Rapid move (kind 27): same encoding as [`Move`] but only X, Y, Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rapid {
    pub role: u8,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub z: Option<i32>,
}

/// Bit set of axes/outputs used by home / positioning-mode / zero-set records:
/// bit0 X, bit1 Y, bit2 Z, bit3 E, bit4 Fan1, bit5 Fan2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Axes(pub u8);

impl Axes {
    pub const X: u8 = 1;
    pub const Y: u8 = 2;
    pub const Z: u8 = 4;
    pub const E: u8 = 8;
    pub fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

/// Typed view of a [`Record`]. Names follow the G/M code each record is converted from
/// in Z-Suite. Layouts were read from the decompiled serializers; kinds that never appear in
/// the sample files (3, 9, 10, 12, 13, 15, 17, 21, 22, 26, 28) are untested on real data.
/// A record whose payload does not match its expected layout is kept as [`Command::Other`].
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// kind 1, G1: linear move.
    Move(Move),
    /// kind 2, F: feedrate.
    Feedrate(i32),
    /// kind 3, G92 with zero for the flagged axes.
    ZeroPosition(Axes),
    /// kind 4, G92: set position (flags + optional X,Y,Z,E,Fan1,Fan2,Z2,Tmp4; no role byte).
    SetPosition(Move),
    /// kind 5, G28: home the flagged axes.
    Home(Axes),
    /// kind 7, T: select tool.
    SelectTool(u8),
    /// kind 8, M104/M109: set nozzle temperature.
    NozzleTemp { tool: u8, temp: i32 },
    /// kind 9, G90: absolute positioning for the flagged axes.
    Absolute(Axes),
    /// kind 10, G91: relative positioning for the flagged axes.
    Relative(Axes),
    /// kind 11: progress percentage.
    Progress(u8),
    /// kind 12, G4: dwell (`P`, `u32`).
    Dwell(u32),
    /// kind 13: `u8` + `f32` (meaning not determined).
    Kind13 { a: u8, b: f32 },
    /// kind 14, M190: bed temperature.
    BedTemp(i32),
    /// kind 15: two `i32` (first is "Height").
    Kind15 { height: i32, b: i32 },
    /// kind 16, M191: chamber temperature.
    ChamberTemp(i32),
    /// kind 17, M101.
    M101(u8),
    /// kind 19, M2: end of program.
    EndProgram,
    /// kind 21, M111: two booleans packed in a byte.
    M111(u8),
    /// kind 22, M192.
    M192(i32),
    /// kind 26, M70.
    M70(u8),
    /// kind 27, G0: rapid move.
    Rapid(Rapid),
    /// kind 28, M900: `f32` K factor + `u8` tool.
    M900 { k: f32, tool: u8 },
    /// kind 30, G29 with parameters: `i32, i32, u8, u8`.
    G29 { a: i32, b: i32, c: u8, d: u8 },
    /// Any kind not (yet) understood, or not matching the expected layout.
    Other { kind: u8, payload: Vec<u8> },
}

fn read_flagged<const N: usize>(payload: &[u8], with_role: bool) -> Option<(u8, [Option<i32>; N])> {
    let (role, rest) = if with_role {
        let (&r, rest) = payload.split_first()?;
        (r, rest)
    } else {
        (0, payload)
    };
    let (&flags, mut rest) = rest.split_first()?;
    if N < 8 && flags >> N != 0 {
        return None;
    }
    let mut vals = [None; N];
    for (i, v) in vals.iter_mut().enumerate() {
        if flags & (1 << i) != 0 {
            let (b, r) = rest.split_first_chunk::<4>()?;
            *v = Some(i32::from_le_bytes(*b));
            rest = r;
        }
    }
    rest.is_empty().then_some((role, vals))
}

fn write_flagged(role: Option<u8>, vals: &[Option<i32>]) -> Vec<u8> {
    let flags = vals.iter().enumerate().fold(0u8, |f, (i, v)| f | ((v.is_some() as u8) << i));
    let mut out = Vec::new();
    out.extend(role);
    out.push(flags);
    for v in vals.iter().flatten() {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn i32_at(p: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(p[at..at + 4].try_into().unwrap())
}

impl Command {
    pub fn from_record(r: &Record) -> Command {
        let p = r.payload.as_slice();
        let typed = match (r.kind, p.len()) {
            (1, _) => read_flagged::<8>(p, true).map(|(role, [x, y, z, e, fan1, fan2, z2, tmp4])| {
                Command::Move(Move { role, x, y, z, e, fan1, fan2, z2, tmp4 })
            }),
            (4, _) => read_flagged::<8>(p, false).map(|(role, [x, y, z, e, fan1, fan2, z2, tmp4])| {
                Command::SetPosition(Move { role, x, y, z, e, fan1, fan2, z2, tmp4 })
            }),
            (27, _) => read_flagged::<3>(p, true)
                .map(|(role, [x, y, z])| Command::Rapid(Rapid { role, x, y, z })),
            (2, 4) => Some(Command::Feedrate(i32_at(p, 0))),
            (3, 1) => Some(Command::ZeroPosition(Axes(p[0]))),
            (5, 1) => Some(Command::Home(Axes(p[0]))),
            (7, 1) => Some(Command::SelectTool(p[0])),
            (8, 5) => Some(Command::NozzleTemp { tool: p[0], temp: i32_at(p, 1) }),
            (9, 1) => Some(Command::Absolute(Axes(p[0]))),
            (10, 1) => Some(Command::Relative(Axes(p[0]))),
            (11, 1) => Some(Command::Progress(p[0])),
            (12, 4) => Some(Command::Dwell(u32::from_le_bytes(p.try_into().unwrap()))),
            (13, 5) => Some(Command::Kind13 { a: p[0], b: f32::from_le_bytes(p[1..5].try_into().unwrap()) }),
            (14, 4) => Some(Command::BedTemp(i32_at(p, 0))),
            (15, 8) => Some(Command::Kind15 { height: i32_at(p, 0), b: i32_at(p, 4) }),
            (16, 4) => Some(Command::ChamberTemp(i32_at(p, 0))),
            (17, 1) => Some(Command::M101(p[0])),
            (19, 0) => Some(Command::EndProgram),
            (21, 1) => Some(Command::M111(p[0])),
            (22, 4) => Some(Command::M192(i32_at(p, 0))),
            (26, 1) => Some(Command::M70(p[0])),
            (28, 5) => Some(Command::M900 { k: f32::from_le_bytes(p[0..4].try_into().unwrap()), tool: p[4] }),
            (30, 10) => Some(Command::G29 { a: i32_at(p, 0), b: i32_at(p, 4), c: p[8], d: p[9] }),
            _ => None,
        };
        typed.unwrap_or_else(|| Command::Other { kind: r.kind, payload: r.payload.clone() })
    }

    pub fn to_record(&self) -> Record {
        let r = |kind: u8, payload: Vec<u8>| Record::new(kind, payload);
        let mv = |m: &Move| [m.x, m.y, m.z, m.e, m.fan1, m.fan2, m.z2, m.tmp4];
        match self {
            Command::Move(m) => r(1, write_flagged(Some(m.role), &mv(m))),
            Command::SetPosition(m) => r(4, write_flagged(None, &mv(m))),
            Command::Rapid(m) => r(27, write_flagged(Some(m.role), &[m.x, m.y, m.z])),
            Command::Feedrate(v) => r(2, v.to_le_bytes().to_vec()),
            Command::ZeroPosition(a) => r(3, vec![a.0]),
            Command::Home(a) => r(5, vec![a.0]),
            Command::SelectTool(t) => r(7, vec![*t]),
            Command::NozzleTemp { tool, temp } => {
                let mut v = vec![*tool];
                v.extend_from_slice(&temp.to_le_bytes());
                r(8, v)
            }
            Command::Absolute(a) => r(9, vec![a.0]),
            Command::Relative(a) => r(10, vec![a.0]),
            Command::Progress(v) => r(11, vec![*v]),
            Command::Dwell(v) => r(12, v.to_le_bytes().to_vec()),
            Command::Kind13 { a, b } => {
                let mut v = vec![*a];
                v.extend_from_slice(&b.to_le_bytes());
                r(13, v)
            }
            Command::BedTemp(v) => r(14, v.to_le_bytes().to_vec()),
            Command::Kind15 { height, b } => {
                let mut v = height.to_le_bytes().to_vec();
                v.extend_from_slice(&b.to_le_bytes());
                r(15, v)
            }
            Command::ChamberTemp(v) => r(16, v.to_le_bytes().to_vec()),
            Command::M101(v) => r(17, vec![*v]),
            Command::EndProgram => r(19, vec![]),
            Command::M111(v) => r(21, vec![*v]),
            Command::M192(v) => r(22, v.to_le_bytes().to_vec()),
            Command::M70(v) => r(26, vec![*v]),
            Command::M900 { k, tool } => {
                let mut v = k.to_le_bytes().to_vec();
                v.push(*tool);
                r(28, v)
            }
            Command::G29 { a, b, c, d } => {
                let mut v = a.to_le_bytes().to_vec();
                v.extend_from_slice(&b.to_le_bytes());
                v.extend([*c, *d]);
                r(30, v)
            }
            Command::Other { kind, payload } => r(*kind, payload.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_move() {
        let rec = [0x08, 0x01, 0xff, 0x04, 0x98, 0x3a, 0x00, 0x00, 0x5f];
        let recs = parse_records(&rec).unwrap();
        assert_eq!(recs.len(), 1);
        match recs[0].decode() {
            Command::Move(m) => {
                assert_eq!(m.role, 0xff);
                assert_eq!(m.z, Some(15000));
                assert_eq!(m.x, None);
            }
            c => panic!("unexpected {c:?}"),
        }
        assert_eq!(encode_records(&recs).unwrap(), rec);
    }

    #[test]
    fn rejects_bad_crc() {
        let rec = [0x08, 0x01, 0xff, 0x04, 0x98, 0x3a, 0x00, 0x00, 0x00];
        assert!(matches!(parse_records(&rec), Err(Error::BadCrc { .. })));
    }

    #[test]
    fn flipped_roundtrip() {
        let mut r = Record::new(2, 180u32.to_le_bytes().to_vec());
        r.flipped = true;
        let bytes = encode_records(&[r.clone()]).unwrap();
        assert_eq!(parse_records(&bytes).unwrap(), vec![r]);
    }
}

#[cfg(test)]
mod typed_tests {
    use super::*;

    #[test]
    fn all_typed_variants_roundtrip() {
        let cmds = vec![
            Command::Feedrate(180),
            Command::ZeroPosition(Axes(8)),
            Command::SetPosition(Move { e: Some(0), ..Default::default() }),
            Command::Home(Axes(3)),
            Command::SelectTool(1),
            Command::NozzleTemp { tool: 0, temp: 210 },
            Command::Absolute(Axes(15)),
            Command::Relative(Axes(8)),
            Command::Progress(50),
            Command::Dwell(1000),
            Command::Kind13 { a: 1, b: 2.5 },
            Command::BedTemp(30),
            Command::Kind15 { height: 5, b: 6 },
            Command::ChamberTemp(-1),
            Command::M101(1),
            Command::EndProgram,
            Command::M111(3),
            Command::M192(7),
            Command::M70(2),
            Command::Rapid(Rapid { role: 255, x: Some(1), y: Some(-2), z: None }),
            Command::M900 { k: 0.5, tool: 1 },
            Command::G29 { a: 1028, b: 1460, c: 0, d: 0 },
        ];
        for c in cmds {
            let rec = c.to_record();
            assert_eq!(rec.decode(), c, "{c:?}");
        }
    }
}
