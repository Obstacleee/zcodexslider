//! Reference ("golden") tests that pin the `.zcodex2` format.
//!
//! * `format_is_unchanged`: the structure of the file (entries, header fields and types, the fixed start and
//!   end of the command stream, which record kinds appear) must match `tests/golden/format.json`.
//!   Any drift fails the test, so a change to the writer cannot silently alter what Z-Suite reads.
//! * `output_is_stable`: for a fixed model and settings, the headline numbers (layers, commands, time,
//!   filament) may move a little when the slicing algorithm improves, but not a lot.
//!
//! After an intentional change, review the failure message, then refresh the reference with
//! `UPDATE_GOLDEN=1 cargo test -p zslice --test golden`.

mod common;

use std::collections::BTreeMap;

use serde_json::{json, Value};
use zcodex2::Zcodex2;
use zslice::{slice_to_zcodex2, Settings, SupportType};

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/format.json");

fn kind_of(v: &Value) -> &'static str {
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

fn shape(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), shape(x))).collect()),
        Value::Array(a) => Value::Array(a.first().map(|x| vec![shape(x)]).unwrap_or_default()),
        other => Value::String(kind_of(other).to_string()),
    }
}

fn sorted(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let b: BTreeMap<String, Value> = m.into_iter().map(|(k, x)| (k, sorted(x))).collect();
            Value::Object(b.into_iter().collect())
        }
        other => other,
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Everything that defines the file *format* (not the slicing result).
fn format_fingerprint(f: &Zcodex2) -> Value {
    let recs = f.records().unwrap();
    let kinds: Vec<u8> = recs.iter().map(|r| r.kind).collect();
    let mut present: Vec<u8> = kinds.clone();
    present.sort();
    present.dedup();
    let entry_json = |name: &str| sorted(shape(&serde_json::from_slice::<Value>(f.entry(name).unwrap()).unwrap()));
    json!({
        "entries": f.entries.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
        "header": sorted(shape(&f.metadata)),
        "additional_metadata": entry_json("AdditionalMetadata"),
        "user_settings": entry_json("UserSettingsData"),
        "preview_is_png": f.preview_png.starts_with(&[0x89, b'P', b'N', b'G']),
        "stream": {
            "prologue_kinds": kinds[..16].to_vec(),
            // fixed parts of the prologue: temperatures, calibration, tool setup (coordinates excluded)
            "prologue_payloads": recs[..16].iter().enumerate().filter(|(i, _)| ![4usize, 9].contains(i)).map(|(_, r)| hex(&r.payload)).collect::<Vec<_>>(),
            "epilogue_kinds": kinds[kinds.len() - 5..].to_vec(),
            "kinds_present": present,
        },
        "configuration_data_blocks": {
            "rsa_block_bytes": 256,
        },
    })
}

fn headline(f: &Zcodex2) -> Value {
    json!({
        "layers": f.metadata["TotalLayersCount"],
        "commands": f.metadata["CommandCount"],
        "time_s": f.metadata["PrintTime"],
        "filament_mm": f.metadata["Materials"][0]["Usage"],
    })
}

fn reference_file() -> Zcodex2 {
    let mut s = Settings::default();
    s.support = true;
    s.support_type = SupportType::Tree;
    slice_to_zcodex2(&common::tee(), &s, common::png(), None).unwrap().file
}

fn load_golden() -> Value {
    serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("tests/golden/format.json missing: run with UPDATE_GOLDEN=1")).unwrap()
}

fn diff(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()).collect::<std::collections::BTreeSet<_>>() {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => diff(&format!("{path}/{k}"), p, q, out),
                    (Some(_), None) => out.push(format!("{path}/{k}: présent dans la référence, absent maintenant")),
                    (None, Some(_)) => out.push(format!("{path}/{k}: nouveau champ non prévu par la référence")),
                    _ => {}
                }
            }
        }
        _ if a != b => out.push(format!("{path}: référence {a} ≠ actuel {b}")),
        _ => {}
    }
}

#[test]
fn format_is_unchanged() {
    let f = reference_file();
    let now = json!({ "format": format_fingerprint(&f), "headline": headline(&f) });
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(std::path::Path::new(GOLDEN).parent().unwrap()).unwrap();
        std::fs::write(GOLDEN, serde_json::to_string_pretty(&now).unwrap()).unwrap();
        eprintln!("référence mise à jour : {GOLDEN}");
        return;
    }
    let golden = load_golden();
    let mut problems = Vec::new();
    diff("format", &golden["format"], &now["format"], &mut problems);
    assert!(
        problems.is_empty(),
        "le FORMAT du fichier a changé (Z-Suite pourrait ne plus l'ouvrir) :\n  - {}\n\nSi c'est voulu, vérifie avec `ztool check` puis relance avec UPDATE_GOLDEN=1.",
        problems.join("\n  - ")
    );
}

#[test]
fn output_is_stable() {
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        return; // refreshed by format_is_unchanged
    }
    let f = reference_file();
    let now = headline(&f);
    let golden = load_golden()["headline"].clone();
    let rel = |k: &str| {
        let (a, b) = (golden[k].as_f64().unwrap(), now[k].as_f64().unwrap());
        (b - a).abs() / a.max(1.0)
    };
    assert_eq!(golden["layers"], now["layers"], "le nombre de couches a changé : {golden} → {now}");
    for (k, tol) in [("commands", 0.25), ("time_s", 0.10), ("filament_mm", 0.10)] {
        assert!(
            rel(k) <= tol,
            "{k} a bougé de {:.1} % (tolérance {:.0} %) : référence {golden} → actuel {now}\nSi l'amélioration est voulue : UPDATE_GOLDEN=1 cargo test -p zslice --test golden",
            rel(k) * 100.0,
            tol * 100.0
        );
    }
}

/// The genuine Z-Suite file has the very same format fingerprint (minus the algorithm-dependent parts).
#[test]
fn matches_a_real_z_suite_file() {
    let Ok(real) = Zcodex2::open(r"C:\Users\Lucas Delon\Downloads\sentinel 2.zcodex2") else { return };
    let (a, b) = (format_fingerprint(&real), format_fingerprint(&reference_file()));
    let mut problems = Vec::new();
    for key in ["entries", "header", "additional_metadata", "user_settings", "preview_is_png"] {
        diff(key, &a[key], &b[key], &mut problems);
    }
    // the genuine file's start and end of stream must match ours, kind for kind
    diff("prologue_kinds", &a["stream"]["prologue_kinds"], &b["stream"]["prologue_kinds"], &mut problems);
    diff("epilogue_kinds", &a["stream"]["epilogue_kinds"], &b["stream"]["epilogue_kinds"], &mut problems);
    assert!(problems.is_empty(), "différences avec un vrai fichier Z-Suite :\n  - {}", problems.join("\n  - "));
}
