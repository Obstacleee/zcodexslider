//! Runs against real Z-Suite output. Set ZCODEX2_SAMPLES to a directory of `.zcodex2` files
//! (defaults to the user's Downloads folder); skips when none are found.
use std::path::PathBuf;
use zcodex2::*;

fn samples() -> Vec<PathBuf> {
    let dir = std::env::var_os("ZCODEX2_SAMPLES")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Downloads")));
    let Some(dir) = dir else { return vec![] };
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    rd.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "zcodex2"))
        .collect()
}

#[test]
fn parse_roundtrip_all_samples() {
    let files = samples();
    if files.is_empty() {
        eprintln!("no sample .zcodex2 files found, skipping");
        return;
    }
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let f = Zcodex2::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        if f.entry(CONFIGURATION_DATA).is_none() {
            // written by an early version of our own slicer, not by Z-Suite: Z-Suite cannot open these either
            eprintln!("{name}: no ConfigurationData, not a Z-Suite file, skipped");
            continue;
        }

        for e in [ADDITIONAL_METADATA, ZCODE_DATA, CONFIGURATION_DATA, USER_SETTINGS_DATA] {
            assert!(f.entry(e).is_some(), "{name}: missing {e}");
        }

        // CRC is verified while parsing; record count must match the header
        let records = f.records().unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(Some(records.len() as u64), f.command_count(), "{name}: CommandCount");

        // byte-identical record re-encoding
        assert_eq!(encode_records(&records).unwrap(), f.entry(ZCODE_DATA).unwrap(), "{name}");

        // typed decode -> encode is lossless for every record
        for (i, r) in records.iter().enumerate() {
            assert_eq!(&r.decode().to_record(), r, "{name}: record {i} kind {}", r.kind);
        }

        // whole-file rewrite and reparse
        let g = Zcodex2::from_bytes(&f.to_bytes().unwrap()).unwrap();
        assert_eq!(g.metadata, f.metadata, "{name}: metadata");
        assert_eq!(g.preview_png, f.preview_png, "{name}: png");
        assert_eq!(g.entries, f.entries, "{name}: entries");
        eprintln!("{name}: {} records OK", records.len());
    }
}
