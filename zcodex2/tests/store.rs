//! Uses the profile shipped with Z-Suite; skipped when it is not installed.
use std::io::Read;
use zcodex2::Store;

const PROFILE: &str = r"C:\Program Files\Zortrax\Z-Suite\Profiles\default mode.zprof";

#[test]
fn shipped_profile_roundtrip() {
    let Ok(gz) = std::fs::read(PROFILE) else {
        eprintln!("Z-Suite profile not found, skipping");
        return;
    };
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut raw).unwrap();

    let s = Store::from_gzip(&gz).unwrap();
    let keys: Vec<_> = s.entries.iter().map(|(k, _)| k.as_str()).collect();
    eprintln!("keys: {keys:?}, version {}, hash {}", s.version, s.hash_size);
    assert!(keys.contains(&"Defaults"));
    assert!(keys.contains(&"ProjectVersion"));

    // re-encoding the NRBF stream reproduces Microsoft's BinaryFormatter output byte for byte
    assert_eq!(s.to_nrbf(), raw);
    // and the gzip we write reads back identically
    assert_eq!(Store::from_gzip(&s.to_gzip().unwrap()).unwrap(), s);
}

#[test]
fn shipped_profile_nested_values() {
    let Ok(s) = Store::open(PROFILE) else { return };
    let v = s.value("ProjectVersion").unwrap().unwrap();
    eprintln!("ProjectVersion = {v:?}");
    let d = s.value("Defaults").unwrap().unwrap();
    eprintln!("Defaults = {d:#?}");
    assert!(matches!(&d, zcodex2::nrbf::Value::Object { class, .. } if class.ends_with("DefaultProfileToJson")));
    assert!(d.field("NozzleDiameter").and_then(|v| v.as_f64()).is_some());
}
