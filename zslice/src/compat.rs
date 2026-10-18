//! Compatibility check against what Z-Suite 3.7 requires when it opens a `.zcodex2` file.
//!
//! Derived from reading Z-Suite's loader: every entry below is read without a fallback, `UserSettingsData`
//! is deserialised and applied field by field (several fields are looked up in Z-Suite's option lists),
//! and the header, preview and command stream are parsed.

use serde_json::Value;
use zcodex2::{Zcodex2, ADDITIONAL_METADATA, CONFIGURATION_DATA, USER_SETTINGS_DATA, ZCODE_DATA};

const USER_SETTINGS: &str = include_str!("../templates/usersettings_m200plus_zpla.json");
const METADATA: &str = include_str!("../templates/metadata_m200plus.json");
const ADDITIONAL: &str = include_str!("../templates/additional_metadata.json");

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_i64() || n.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `int` is accepted where a float is expected, and a float with an integral value where an int is expected.
fn compatible(want: &str, got: &Value) -> bool {
    match (want, kind(got)) {
        (a, b) if a == b => true,
        ("null", _) => true, // optional in the reference: any value type is fine
        (_, "null") => true,
        ("float", "int") => true,
        ("int", "float") => got.as_f64().map_or(false, |f| f.fract() == 0.0),
        _ => false,
    }
}

fn check_object(what: &str, reference: &Value, got: &Value, problems: &mut Vec<String>) {
    let (Some(ro), Some(go)) = (reference.as_object(), got.as_object()) else {
        problems.push(format!("{what} n'est pas un objet JSON"));
        return;
    };
    for (k, rv) in ro {
        match go.get(k) {
            None => problems.push(format!("{what} : champ « {k} » manquant")),
            Some(gv) if !compatible(kind(rv), gv) => {
                problems.push(format!("{what} : « {k} » devrait être {} mais est {}", kind(rv), kind(gv)))
            }
            _ => {}
        }
    }
}

fn parse(s: &str) -> Value {
    serde_json::from_str(s).expect("embedded reference")
}

/// Returns the list of problems that would stop Z-Suite from opening `f` (empty = looks compatible).
pub fn check(f: &Zcodex2) -> Vec<String> {
    check_with(f, &f.records())
}

/// [`check`] with the decoded command stream supplied by the caller (decoding is the costly part).
pub fn check_with(f: &Zcodex2, records: &Result<Vec<zcodex2::Record>, zcodex2::Error>) -> Vec<String> {
    let mut p = Vec::new();

    for name in [ADDITIONAL_METADATA, ZCODE_DATA, CONFIGURATION_DATA, USER_SETTINGS_DATA] {
        if f.entry(name).is_none() {
            p.push(format!("entrée « {name} » absente (Z-Suite la lit sans valeur par défaut)"));
        }
    }
    if !f.preview_png.starts_with(PNG_MAGIC) || f.preview_png.len() < 33 {
        p.push("l'aperçu n'est pas un PNG valide".into());
    }

    let meta_ref = parse(METADATA);
    check_object("l'en-tête", &meta_ref, &f.metadata, &mut p);
    if let (Some(a), Some(b)) = (f.metadata.get("Materials").and_then(|m| m.get(0)), meta_ref.get("Materials").and_then(|x| x.get(0))) {
        check_object("l'en-tête, matériau", b, a, &mut p);
    }
    if let (Some(a), Some(b)) = (f.metadata.get("PrintBoundingBox"), meta_ref.get("PrintBoundingBox")) {
        check_object("l'en-tête, emprise", b, a, &mut p);
    }
    if f.metadata.get("PrinterModelEnumId").and_then(Value::as_i64) != Some(24) {
        p.push("PrinterModelEnumId différent de 24 (seule la M200 Plus est connue)".into());
    }
    if let Some(v) = f.metadata.get("MinFirmwareVersion").and_then(Value::as_str) {
        if !v.split('.').all(|x| x.parse::<u8>().is_ok()) {
            p.push("MinFirmwareVersion doit être de la forme 2.6.0 (trois octets)".into());
        }
    }

    for (name, reference) in [(ADDITIONAL_METADATA, ADDITIONAL), (USER_SETTINGS_DATA, USER_SETTINGS)] {
        match f.entry(name).map(serde_json::from_slice::<Value>) {
            Some(Ok(v)) => check_object(name, &parse(reference), &v, &mut p),
            Some(Err(e)) => p.push(format!("{name} n'est pas du JSON valide : {e}")),
            None => {}
        }
    }
    if let Some(Ok(v)) = f.entry(USER_SETTINGS_DATA).map(serde_json::from_slice::<Value>) {
        let ang = v.get("SupportAngle").and_then(Value::as_str).unwrap_or("");
        let numeric = ang.chars().count() >= 2 && ang.chars().take(ang.chars().count() - 1).collect::<String>().parse::<f32>().is_ok();
        if !numeric {
            p.push("UserSettingsData : SupportAngle doit être un nombre suivi d'un symbole (ex. 30°)".into());
        }
        let lt = v.get("LayerThickness").and_then(Value::as_str).unwrap_or("");
        if !["0.10 mm", "0.15 mm", "0.20 mm", "0.25 mm", "0.30 mm"].contains(&lt) {
            p.push(format!("UserSettingsData : épaisseur « {lt} » inconnue de Z-Suite"));
        }
        if !["Draft", "Normal", "High"].contains(&v.get("Quality").and_then(Value::as_str).unwrap_or("")) {
            p.push("UserSettingsData : Quality doit être Draft, Normal ou High".into());
        }
    }

    match records {
        Ok(recs) => {
            if f.command_count() != Some(recs.len() as u64) {
                p.push(format!("CommandCount ({:?}) ≠ nombre d'enregistrements ({})", f.command_count(), recs.len()));
            }
            if !matches!(recs.last().map(|r| r.decode()), Some(zcodex2::Command::EndProgram)) {
                p.push("le flux ne se termine pas par EndProgram".into());
            }
            let layers = recs.iter().filter(|r| matches!(r.decode(), zcodex2::Command::ChamberTemp(_))).count();
            if f.metadata.get("TotalLayersCount").and_then(Value::as_u64) != Some(layers as u64) {
                p.push(format!("TotalLayersCount ≠ nombre de marqueurs de couche ({layers})"));
            }
        }
        Err(e) => p.push(format!("ZCodeData illisible : {e}")),
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{slice_to_zcodex2, Mesh, Settings};

    fn png() -> Vec<u8> {
        crate::preview::top_view_png(&[], [200.0, 200.0], 32)
    }

    fn cube() -> Mesh {
        crate::slicer_test_cube(80.0, 80.0, 12.0)
    }

    #[test]
    fn a_real_z_suite_file_passes_its_own_check() {
        let Ok(f) = Zcodex2::open(r"C:\Users\Lucas Delon\Downloads\sentinel 2.zcodex2") else { return };
        let problems = check(&f);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn generated_files_pass_the_check() {
        for tree in [false, true] {
            let mut s = Settings::default();
            s.support = tree;
            s.support_type = if tree { crate::SupportType::Tree } else { crate::SupportType::Normal };
            let r = slice_to_zcodex2(&cube(), &s, png(), None).unwrap();
            let problems = check(&r.file);
            assert!(problems.is_empty(), "{problems:#?}");
        }
    }

    #[test]
    fn missing_pieces_are_reported() {
        let mut f = slice_to_zcodex2(&cube(), &Settings::default(), png(), None).unwrap().file;
        f.entries.retain(|(n, _)| n != CONFIGURATION_DATA);
        assert!(check(&f).iter().any(|m| m.contains("ConfigurationData")));
        let mut f2 = slice_to_zcodex2(&cube(), &Settings::default(), png(), None).unwrap().file;
        f2.preview_png = vec![1, 2, 3];
        f2.metadata["PrinterModelEnumId"] = Value::from(7);
        let p = check(&f2);
        assert!(p.iter().any(|m| m.contains("PNG")) && p.iter().any(|m| m.contains("PrinterModelEnumId")));
    }

    #[test]
    fn other_materials_still_pass() {
        let mut s = Settings::default();
        s.set_material(&crate::materials()[2]); // Z-ABS
        let r = slice_to_zcodex2(&cube(), &s, png(), None).unwrap();
        assert!(check(&r.file).is_empty());
    }

    #[test]
    fn previous_generation_would_have_failed() {
        // a file like the first versions wrote: no ConfigurationData and a partial UserSettingsData
        let mut f = slice_to_zcodex2(&cube(), &Settings::default(), png(), None).unwrap().file;
        f.entries.retain(|(n, _)| n != CONFIGURATION_DATA);
        let partial = serde_json::json!({"MaxLayer": 1, "MinLayer": 0, "PrintTime": "1m", "Printer": "Zortrax M200 Plus"});
        f.set_entry(USER_SETTINGS_DATA, serde_json::to_vec(&partial).unwrap());
        let p = check(&f);
        assert!(p.len() > 40, "should report many problems, got {}", p.len());
    }
}
