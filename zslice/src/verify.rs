//! Automatic verification of generated `.zcodex2` files.
//!
//! Three levels, all run on every file we write (see `export::package`):
//! 1. [`check_container`]: byte-level layout (header JSON, PNG, ZIP with absolute offsets);
//! 2. [`crate::compat::check`]: entries, fields and values Z-Suite requires when it opens a file;
//! 3. [`check_stream`]: physical sanity of the command stream and consistency with the header
//!    (bed limits, temperatures, retraction state, layer markers, extrusion totals...).
//!
//! A file that fails any of them is never written.

use serde_json::Value;
use zcodex2::{Command, Record, Zcodex2};

use crate::settings::Settings;

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Max number of messages per kind of problem, so a systematic defect does not drown the report.
const PER_KIND: usize = 3;

struct Report {
    problems: Vec<String>,
    counts: std::collections::HashMap<&'static str, usize>,
}

impl Report {
    fn new() -> Self {
        Report { problems: Vec::new(), counts: Default::default() }
    }
    fn add(&mut self, kind: &'static str, msg: String) {
        let c = self.counts.entry(kind).or_insert(0);
        *c += 1;
        if *c <= PER_KIND {
            self.problems.push(msg);
        } else if *c == PER_KIND + 1 {
            self.problems.push(format!("… et d'autres problèmes de type « {kind} »"));
        }
    }
}

fn u32_at(b: &[u8], p: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?) as usize)
}

/// Layout of the serialised file: `u32 json_len | JSON | u32 png_len | PNG | ZIP` where the ZIP's offsets
/// are absolute in the whole file (Z-Suite writes the ZIP into the same stream, after the header).
pub fn check_container(bytes: &[u8]) -> Vec<String> {
    let mut p = Vec::new();
    let Some(n) = u32_at(bytes, 0) else { return vec!["fichier trop court".into()] };
    let Some(json) = bytes.get(4..4 + n) else { return vec!["longueur de l'en-tête JSON hors du fichier".into()] };
    if serde_json::from_slice::<Value>(json).is_err() {
        p.push("l'en-tête JSON n'est pas du JSON valide".into());
    }
    let Some(m) = u32_at(bytes, 4 + n) else { return vec!["fichier tronqué avant l'aperçu".into()] };
    let png_start = 8 + n;
    let Some(png) = bytes.get(png_start..png_start + m) else { return vec!["longueur de l'aperçu hors du fichier".into()] };
    if !png.starts_with(PNG_MAGIC) || png.len() < 33 || &png[12..16] != b"IHDR" {
        p.push("l'aperçu n'est pas un PNG valide (signature ou bloc IHDR)".into());
    } else {
        let (w, h) = (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap()));
        if w == 0 || h == 0 || w > 8192 || h > 8192 {
            p.push(format!("dimensions de l'aperçu invraisemblables ({w}×{h})"));
        }
    }
    let zip_start = png_start + m;
    if bytes.get(zip_start..zip_start + 4) != Some(b"PK\x03\x04".as_slice()) {
        p.push("le ZIP ne commence pas juste après l'aperçu".into());
    }
    // end of central directory
    let tail_from = bytes.len().saturating_sub(70_000);
    match bytes[tail_from..].windows(4).rposition(|w| w == b"PK\x05\x06") {
        None => p.push("fin de répertoire central du ZIP introuvable".into()),
        Some(rel) => {
            let eocd = tail_from + rel;
            let cd_size = u32_at(bytes, eocd + 12);
            let cd_off = u32_at(bytes, eocd + 16);
            match (cd_size, cd_off) {
                (Some(sz), Some(off)) => {
                    if off + sz != eocd {
                        p.push(format!("décalages du ZIP non absolus (répertoire à {off}+{sz}, fin à {eocd}) : Z-Suite ne pourrait pas le lire"));
                    }
                    if off < zip_start {
                        p.push("le répertoire central du ZIP pointe avant le début du ZIP".into());
                    }
                }
                _ => p.push("fin de répertoire central tronquée".into()),
            }
        }
    }
    p
}

struct Sim<'a> {
    s: &'a Settings,
    x: i64,
    y: i64,
    z: i64,
    homed: bool,
    retracted: bool,
    last_marker: Option<i32>,
    last_progress: i32,
    layer_z: i64,
    filament_um: i64,
    bbox: [i64; 4], // min x, min y, max x, max y of extruding moves
    nozzle: i32,
    extruded_since_temp: bool,
}

/// Physical sanity of the command stream and agreement with the header.
pub fn check_stream(records: &[Record], header: &Value, s: &Settings) -> Vec<String> {
    let mut r = Report::new();
    let cmds: Vec<Command> = records.iter().map(Record::decode).collect();

    // --- fixed structure at both ends (same sequence as Z-Suite)
    let kinds: Vec<u8> = records.iter().map(|x| x.kind).collect();
    const PROLOGUE: [u8; 16] = [4, 14, 8, 20, 30, 5, 5, 4, 2, 1, 28, 28, 26, 16, 7, 21];
    if kinds.len() < 30 || kinds[..16] != PROLOGUE {
        r.add("début", format!("le début du flux ne suit pas la séquence de Z-Suite (types {:?})", &kinds[..kinds.len().min(16)]));
    }
    if !matches!(cmds.last(), Some(Command::EndProgram)) {
        r.add("fin", "le flux ne se termine pas par EndProgram".into());
    }
    let tail: Vec<&Command> = cmds.iter().rev().take(5).collect();
    if !tail.iter().any(|c| matches!(c, Command::NozzleTemp { temp: 0, .. })) {
        r.add("fin", "la buse n'est pas éteinte à la fin".into());
    }
    if !tail.iter().any(|c| matches!(c, Command::Progress(100))) {
        r.add("fin", "Progress(100) manquant avant la fin".into());
    }

    let mut st = Sim {
        s,
        x: 0,
        y: 0,
        z: 0,
        homed: false,
        retracted: s.retraction_distance > 0.0,
        last_marker: None,
        last_progress: -1,
        layer_z: 0,
        filament_um: 0,
        bbox: [i64::MAX, i64::MAX, i64::MIN, i64::MIN],
        nozzle: 0,
        extruded_since_temp: false,
    };
    let (bed_x, bed_y) = ((s.bed[0] * 1000.0) as i64 + 500, (s.bed[1] * 1000.0) as i64 + 500);
    let max_z = (s.max_z * 1000.0) as i64 + 10;
    let mut layer_markers = 0usize;
    let mut first_extrusion_seen = false;

    for (i, c) in cmds.iter().enumerate() {
        match c {
            Command::Home(_) => st.homed = true,
            Command::NozzleTemp { temp, .. } => {
                if *temp != 0 && !(150..=300).contains(temp) {
                    r.add("température", format!("enregistrement {i} : buse à {temp} °C hors de 150–300 °C"));
                }
                st.nozzle = *temp;
            }
            Command::BedTemp(t) if !(0..=110).contains(t) => {
                r.add("température", format!("enregistrement {i} : plateau à {t} °C hors de 0–110 °C"));
            }
            Command::Feedrate(f) if !(60..=18_000).contains(f) && !(i < 20 && *f == 180) => {
                r.add("vitesse", format!("enregistrement {i} : vitesse {f} mm/min hors de 60–18000"));
            }
            Command::ChamberTemp(marker) => {
                layer_markers += 1;
                if let Some(prev) = st.last_marker {
                    if *marker != prev + 1 {
                        r.add("couches", format!("enregistrement {i} : marqueur de couche {marker} après {prev} (doit augmenter de 1)"));
                    }
                }
                st.last_marker = Some(*marker);
            }
            Command::Progress(p) => {
                if (*p as i32) < st.last_progress {
                    r.add("progression", format!("enregistrement {i} : progression {p} % qui recule"));
                }
                st.last_progress = *p as i32;
            }
            Command::Rapid(m) => {
                if let Some(v) = m.x { st.x = v as i64 }
                if let Some(v) = m.y { st.y = v as i64 }
                if let Some(v) = m.z { st.z = v as i64 }
                st.check_position(&mut r, i, bed_x, bed_y, max_z);
            }
            Command::Move(m) => {
                let (px, py) = (st.x, st.y);
                if let Some(v) = m.x { st.x = v as i64 }
                if let Some(v) = m.y { st.y = v as i64 }
                if let Some(v) = m.z { st.z = v as i64 }
                if m.role == 255 {
                    continue; // fan / unlabelled setup moves
                }
                st.check_position(&mut r, i, bed_x, bed_y, max_z);
                let e = m.e.unwrap_or(0);
                match m.role {
                    253 => {
                        if st.retracted {
                            r.add("rétraction", format!("enregistrement {i} : rétraction alors que le filament est déjà rétracté"));
                        }
                        if e >= 0 {
                            r.add("rétraction", format!("enregistrement {i} : rétraction avec E = {e} (doit être négatif)"));
                        }
                        st.retracted = true;
                    }
                    254 => {
                        if !st.retracted {
                            r.add("rétraction", format!("enregistrement {i} : rechargement sans rétraction préalable (sur-extrusion)"));
                        }
                        st.retracted = false;
                    }
                    252 => {}
                    role if e > 0 && role < 250 => {
                        first_extrusion_seen = true;
                        if st.retracted {
                            r.add("rétraction", format!("enregistrement {i} : extrusion alors que le filament est rétracté (sous-extrusion)"));
                        }
                        if !st.homed {
                            r.add("repérage", format!("enregistrement {i} : extrusion avant le repérage (Home)"));
                        }
                        if st.nozzle < 150 {
                            r.add("température", format!("enregistrement {i} : extrusion avec la buse à {} °C", st.nozzle));
                        }
                        if e > 200_000 {
                            r.add("extrusion", format!("enregistrement {i} : {} mm de filament en un seul mouvement", e / 1000));
                        }
                        let len = (((st.x - px) as f64).hypot((st.y - py) as f64)) / 1000.0;
                        if len > 0.0 && (e as f64 / 1000.0) / len > 4.0 {
                            r.add("extrusion", format!("enregistrement {i} : {:.1} mm de filament pour {:.2} mm de trajet", e as f64 / 1000.0, len));
                        }
                        if st.z + 1 < st.layer_z {
                            r.add("couches", format!("enregistrement {i} : extrusion à z = {} µm sous la couche précédente ({} µm)", st.z, st.layer_z));
                        }
                        st.layer_z = st.layer_z.max(st.z);
                        st.filament_um += e as i64;
                        for (px, py) in [(px, py), (st.x, st.y)] {
                            st.bbox = [st.bbox[0].min(px), st.bbox[1].min(py), st.bbox[2].max(px), st.bbox[3].max(py)];
                        }
                        st.extruded_since_temp = true;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if !first_extrusion_seen {
        r.add("extrusion", "aucune extrusion dans le flux".into());
    }
    if !st.homed {
        r.add("repérage", "aucun Home dans le flux".into());
    }

    // --- agreement with the header
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_i64);
    if num(header, "CommandCount") != Some(records.len() as i64) {
        r.add("en-tête", format!("CommandCount {:?} ≠ {} enregistrements", num(header, "CommandCount"), records.len()));
    }
    if num(header, "TotalLayersCount") != Some(layer_markers as i64) {
        r.add("en-tête", format!("TotalLayersCount {:?} ≠ {layer_markers} marqueurs de couche", num(header, "TotalLayersCount")));
    }
    if num(header, "PrintTime").map_or(true, |t| t <= 0) {
        r.add("en-tête", "PrintTime doit être positif".into());
    }
    let used = header.pointer("/Materials/0/Usage").and_then(Value::as_i64).unwrap_or(-1) as f64;
    let from_stream = st.filament_um as f64 / 1000.0;
    if (used - from_stream).abs() > 1.0 + 0.01 * from_stream {
        r.add("en-tête", format!("Materials[0].Usage {used} mm ≠ {from_stream:.0} mm d'extrusion dans le flux"));
    }
    if let Some(bb) = header.get("PrintBoundingBox") {
        let g = |k: &str| bb.get(k).and_then(Value::as_i64).unwrap_or(i64::MIN);
        let tol = 2;
        if g("MinX") > st.bbox[0] + tol || g("MinY") > st.bbox[1] + tol || g("MaxX") < st.bbox[2] - tol || g("MaxY") < st.bbox[3] - tol {
            r.add("en-tête", format!("PrintBoundingBox ne contient pas toutes les extrusions (flux : {:?})", st.bbox));
        }
        if g("MaxZ") < st.layer_z - tol {
            r.add("en-tête", format!("PrintBoundingBox.MaxZ {} < dernière couche {}", g("MaxZ"), st.layer_z));
        }
    }
    r.problems
}

impl Sim<'_> {
    fn check_position(&self, r: &mut Report, i: usize, bed_x: i64, bed_y: i64, max_z: i64) {
        if !self.homed {
            return;
        }
        if self.x < -500 || self.y < -500 || self.x > bed_x || self.y > bed_y {
            r.add("plateau", format!("enregistrement {i} : position ({:.1}, {:.1}) mm hors du plateau {}×{}", self.x as f64 / 1000.0, self.y as f64 / 1000.0, self.s.bed[0], self.s.bed[1]));
        }
        if self.z < 0 || self.z > max_z {
            r.add("plateau", format!("enregistrement {i} : z = {:.2} mm hors de 0–{} mm", self.z as f64 / 1000.0, self.s.max_z));
        }
    }
}

/// Everything: container bytes + Z-Suite requirements + command stream. Empty = the file is sound.
pub fn verify(file: &Zcodex2, s: &Settings) -> Vec<String> {
    match file.to_bytes() {
        Ok(bytes) => verify_bytes(file, &bytes, s),
        Err(e) => vec![format!("[conteneur] sérialisation impossible : {e}")],
    }
}

/// Same as [`verify`] when the file is already serialized (`bytes` = `file.to_bytes()`): the container is
/// not written a second time and the command stream is decoded once for all the checks.
pub fn verify_bytes(file: &Zcodex2, bytes: &[u8], s: &Settings) -> Vec<String> {
    let mut p: Vec<String> = check_container(bytes).into_iter().map(|m| format!("[conteneur] {m}")).collect();
    let recs = file.records();
    p.extend(crate::compat::check_with(file, &recs).into_iter().map(|m| format!("[format Z-Suite] {m}")));
    match &recs {
        Ok(recs) => p.extend(check_stream(recs, &file.metadata, s).into_iter().map(|m| format!("[flux] {m}"))),
        Err(e) => p.push(format!("[flux] ZCodeData illisible : {e}")),
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{slice_to_zcodex2, Mesh};

    fn png() -> Vec<u8> {
        crate::preview::top_view_png(&[], [200.0, 200.0], 32)
    }

    fn cube() -> Mesh {
        crate::slicer_test_cube(80.0, 80.0, 10.0)
    }

    #[test]
    fn a_real_z_suite_file_passes_everything() {
        let Ok(f) = Zcodex2::open(r"C:\Users\Lucas Delon\Downloads\sentinel 2.zcodex2") else { return };
        let p = verify(&f, &Settings::default());
        assert!(p.is_empty(), "{p:#?}");
    }

    #[test]
    fn generated_files_pass_everything() {
        let r = slice_to_zcodex2(&cube(), &Settings::default(), png(), None).unwrap();
        assert!(verify(&r.file, &Settings::default()).is_empty());
    }

    fn corrupt(f: &mut Zcodex2, edit: impl Fn(&mut Vec<Record>)) {
        let mut recs = f.records().unwrap();
        edit(&mut recs);
        f.set_records(&recs).unwrap();
        f.metadata["CommandCount"] = Value::from(recs.len());
    }

    #[test]
    fn each_kind_of_defect_is_caught() {
        let base = slice_to_zcodex2(&cube(), &Settings::default(), png(), None).unwrap().file;
        let s = Settings::default();
        let find_move = |recs: &Vec<Record>| recs.iter().position(|r| matches!(r.decode(), Command::Move(m) if m.role == 3 && m.e.unwrap_or(0) > 0)).unwrap();

        // extrusion outside the bed
        let mut f = base.clone();
        corrupt(&mut f, |r| {
            let i = find_move(r);
            if let Command::Move(mut m) = r[i].decode() {
                m.x = Some(500_000);
                r[i] = Command::Move(m).to_record();
            }
        });
        assert!(verify(&f, &s).iter().any(|m| m.contains("hors du plateau")));

        // a retraction that is never undone (double retraction)
        let mut f = base.clone();
        corrupt(&mut f, |r| {
            let i = find_move(r);
            r.insert(i, Command::Move(zcodex2::Move { role: 253, e: Some(-800), ..Default::default() }).to_record());
            r.insert(i, Command::Move(zcodex2::Move { role: 253, e: Some(-800), ..Default::default() }).to_record());
        });
        assert!(verify(&f, &s).iter().any(|m| m.contains("rétract")));

        // absurd temperature
        let mut f = base.clone();
        corrupt(&mut f, |r| r[2] = Command::NozzleTemp { tool: 0, temp: 450 }.to_record());
        assert!(verify(&f, &s).iter().any(|m| m.contains("450")));

        // a layer marker skipped
        let mut f = base.clone();
        corrupt(&mut f, |r| {
            let i = r.iter().enumerate().filter(|(_, x)| x.kind == 16).nth(3).unwrap().0;
            r[i] = Command::ChamberTemp(99).to_record();
        });
        assert!(verify(&f, &s).iter().any(|m| m.contains("marqueur de couche")));

        // header out of sync with the stream
        let mut f = base.clone();
        f.metadata["TotalLayersCount"] = Value::from(3);
        assert!(verify(&f, &s).iter().any(|m| m.contains("TotalLayersCount")));
        let mut f = base.clone();
        f.metadata["Materials"][0]["Usage"] = Value::from(1);
        assert!(verify(&f, &s).iter().any(|m| m.contains("Usage")));

        // too much filament in one go
        let mut f = base.clone();
        corrupt(&mut f, |r| {
            let i = find_move(r);
            if let Command::Move(mut m) = r[i].decode() {
                m.e = Some(900_000);
                r[i] = Command::Move(m).to_record();
            }
        });
        assert!(verify(&f, &s).iter().any(|m| m.contains("filament")));
    }

    #[test]
    fn container_defects_are_caught() {
        let f = slice_to_zcodex2(&cube(), &Settings::default(), png(), None).unwrap().file;
        let good = f.to_bytes().unwrap();
        assert!(check_container(&good).is_empty());
        // a ZIP written on its own (relative offsets) and appended after the header must be rejected
        let n = u32::from_le_bytes(good[..4].try_into().unwrap()) as usize;
        let m = u32::from_le_bytes(good[4 + n..8 + n].try_into().unwrap()) as usize;
        let zip_start = 8 + n + m;
        let mut shifted = good[..zip_start].to_vec();
        shifted.extend_from_slice(&good[zip_start..]);
        // break the central directory offset
        let eocd = shifted.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        let off = u32::from_le_bytes(shifted[eocd + 16..eocd + 20].try_into().unwrap()) - zip_start as u32;
        shifted[eocd + 16..eocd + 20].copy_from_slice(&off.to_le_bytes());
        assert!(check_container(&shifted).iter().any(|m| m.contains("absolus")));
        assert!(!check_container(&good[..good.len() / 2]).is_empty());
        let mut bad_png = good.clone();
        bad_png[8 + n] = 0;
        assert!(check_container(&bad_png).iter().any(|m| m.contains("PNG")));
    }
}
