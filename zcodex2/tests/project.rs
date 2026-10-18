//! Real project written by Z-Suite 3.7. Set ZPROJ_SAMPLE to override the default location.
use std::io::Read;
use std::path::PathBuf;
use zcodex2::{decode_models, encode_models, nrbf::Value, Store};

fn sample() -> Option<PathBuf> {
    let p = std::env::var_os("ZPROJ_SAMPLE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|u| PathBuf::from(u).join("Downloads").join("sentinel 2.zprojx")))?;
    p.exists().then_some(p)
}

#[test]
fn project_envelope_is_byte_identical() {
    let Some(p) = sample() else { return eprintln!("no sample project, skipping") };
    let gz = std::fs::read(&p).unwrap();
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut raw).unwrap();
    let s = Store::from_gzip(&gz).unwrap();
    assert_eq!(s.to_nrbf(), raw, "BinaryFormatter dictionary re-encoding");
    assert_eq!(Store::from_gzip(&s.to_gzip().unwrap()).unwrap(), s);
}

#[test]
fn project_values() {
    let Some(p) = sample() else { return };
    let s = Store::open(&p).unwrap();
    assert_eq!(s.value("Printer.Model").unwrap().unwrap().as_str(), Some("M200 Plus"));
    assert_eq!(s.value("ProjectVersion").unwrap().unwrap().as_str(), Some("3.7.0.0"));
    let settings = s.value("Settings").unwrap().unwrap();
    assert!(matches!(&settings, Value::Object { class, .. } if class.ends_with("PrintSettingsToJson")));
    // embedded preview is a PNG inside a nested byte[]
    let preview = s.value("ZCodePreview").unwrap().unwrap();
    assert!(preview.as_bytes().unwrap().starts_with(b"\x89PNG"));
}

#[test]
fn project_models_match_geometry() {
    let Some(p) = sample() else { return };
    let s = Store::open(&p).unwrap();
    let models = s.models().unwrap();
    assert_eq!(models.len(), 1);
    let m = &models[0];
    assert_eq!(m.name, "sentinel 2.stl");
    assert_eq!((m.vertices.len(), m.triangles.len()), (750, 1552));
    assert!(m.triangles.iter().flatten().all(|&i| (0..750).contains(&i)));
    // re-encoding reproduces protobuf-net's bytes
    let original = s.get("Models").unwrap();
    assert_eq!(decode_models(&encode_models(&models)).unwrap(), models);
    if encode_models(&models) != original {
        let enc = encode_models(&models);
        let n = enc.iter().zip(original).position(|(a, b)| a != b);
        panic!("protobuf differs: len {} vs {}, first diff at {:?}", enc.len(), original.len(), n);
    }
}

#[test]
fn nested_writers_match_binaryformatter() {
    let Some(p) = sample() else { return };
    let s = Store::open(&p).unwrap();
    for key in ["Printer.Model", "PrinterId", "ProjectVersion", "CurrentProjectState"] {
        let v = s.value(key).unwrap().unwrap();
        assert_eq!(zcodex2::nrbf::write_string(v.as_str().unwrap()), s.get(key).unwrap(), "{key}");
    }
    for key in ["ZCodePreview", "ZCode", "ZCodeOriginal"] {
        let v = s.value(key).unwrap().unwrap();
        let bytes = v.as_bytes().unwrap_or(&[]);
        assert_eq!(zcodex2::nrbf::write_byte_array(bytes), s.get(key).unwrap(), "{key}");
    }
}
