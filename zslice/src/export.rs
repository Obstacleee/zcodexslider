//! Slicing result -> `.zcodex2` container (metadata, preview, ZCodeData, settings).

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use zcodex2::{Zcodex2, ADDITIONAL_METADATA, CONFIGURATION_DATA, USER_SETTINGS_DATA, ZCODE_DATA};

use crate::emit::{emit, Emitted};
use crate::settings::{InfillPattern, Seam, Settings};
use crate::slicer::Mesh;
use crate::toolpath::plan;

pub struct Sliced {
    pub file: Zcodex2,
    /// `file` already serialized (written once, reused by the check and by [`Sliced::save`]).
    pub bytes: Vec<u8>,
    pub stats: Stats,
    pub warnings: Vec<String>,
}

impl Sliced {
    /// Writes the already serialized file.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        std::fs::write(path, &self.bytes)
    }
}

#[derive(Debug, Clone)]
pub struct Stats {
    pub layers: usize,
    pub commands: usize,
    pub print_time_s: f64,
    pub filament_mm: f64,
    pub filament_g: f64,
    pub support_g: f64,
    pub bbox_um: [i64; 6],
}

pub fn human_time(s: f64) -> String {
    let m = (s / 60.0).round() as u64;
    if m >= 60 {
        format!("{}h {:02}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

fn utc_stamp() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}{:04}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        d.subsec_millis() * 10
    )
}

fn guid_string() -> String {
    let g = crate_guid();
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        g[3], g[2], g[1], g[0], g[5], g[4], g[7], g[6], g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
    )
}

fn crate_guid() -> [u8; 16] {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut id = [0u8; 16];
    for half in id.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_u8(7);
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    id[7] = (id[7] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

fn additional_metadata(e: &Emitted, s: &Settings) -> Value {
    let g = |mm: f64| format!("{:.2}", mm * s.material.grams_per_mm);
    let mut usages = vec![json!({"Percentage": 0, "UsageGrams": "0.00", "SupportUsage": "0.00"})];
    for (p, (fil, sup)) in e.progress_usage.iter().enumerate().take(101) {
        usages.push(json!({"Percentage": p, "UsageGrams": g(*fil), "SupportUsage": g(*sup)}));
    }
    json!({"Name": "AdditionalMetadata", "Layers": null, "MaterialUsages": usages, "Pauses": []})
}

/// `UserSettingsData` of a real Z-Suite file (M200 Plus, Z-PLA). Z-Suite applies *all* of these fields when it
/// opens a file and looks several of them up in its own option lists (printer, material group and id,
/// layer thickness, quality, infill, surface layers), so the complete set must be present with valid values.
const USER_SETTINGS_TEMPLATE: &str = include_str!("../templates/usersettings_m200plus_zpla.json");

fn nearest(options: &[f64], v: f64) -> f64 {
    *options.iter().min_by(|a, b| (**a - v).abs().partial_cmp(&(**b - v).abs()).unwrap()).unwrap()
}

fn user_settings(e: &Emitted, s: &Settings, min_layer: i32, max_layer: i32) -> Value {
    let mut v: Value = serde_json::from_str(USER_SETTINGS_TEMPLATE).expect("template");
    let grams = e.filament_mm * s.material.grams_per_mm;
    let vol_cm3 = e.filament_mm * s.filament_area() / 1000.0;
    let sup_g = e.support_mm * s.material.grams_per_mm;
    let mut set = |k: &str, x: Value| {
        debug_assert!(v.get(k).is_some(), "unknown key {k}");
        v[k] = x;
    };
    set("MaxLayer", json!(max_layer));
    set("MinLayer", json!(min_layer));
    set("PrintTime", json!(human_time(e.print_time_s)));
    set("MaterialLength", json!(format!("{:.2}", e.filament_mm / 1000.0)));
    set("MaterialWeight", json!((grams * 100.0).round() / 100.0));
    set("MaterialVolume", json!(vol_cm3.round()));
    set("SupportLength", json!(if e.support_mm > 0.0 { format!("{:.2}", e.support_mm / 1000.0) } else { String::new() }));
    set("SupportWeight", json!((sup_g * 100.0).round() / 100.0));
    set("SupportVolume", json!((e.support_mm * s.filament_area() / 1000.0).round()));

    // Z-Suite only knows the printer / material group / material ids of its own profiles. Z-PLA is the one
    // we measured; any other material keeps Z-PLA's identity here (the header still names the real one).
    if s.material.verified {
        set("MaterialType", json!(s.material.name));
        set("MaterialId", json!(s.material.id.to_string()));
    }
    // values that are looked up in Z-Suite's option lists: pick the nearest valid one
    set("LayerThickness", json!(format!("{:.2} mm", nearest(&[0.10, 0.15, 0.20, 0.25, 0.30], s.layer_height))));
    set("Quality", json!(if s.outer_speed >= 23.0 { "Draft" } else { "Normal" }));
    let dens = (s.infill_density / 10.0).round().clamp(1.0, 10.0) * 10.0;
    set("Infill", json!(format!("{dens:.0}%")));
    set("SurfaceLayersTop", json!(s.top_layers.clamp(1, 10).to_string()));
    set("SurfaceLayersBottom", json!(s.bottom_layers.clamp(1, 10).to_string()));

    // plain values
    set("ExtruderTemperature", json!(s.nozzle_temp));
    set("PlatformTemperature", json!(s.bed_temp));
    set("FanSpeed", json!(s.fan_percent));
    set("RetractionSpeed", json!(s.retraction_speed.round() as i64));
    set("RetractionDistance", json!(s.retraction_distance));
    set("FirstLayerGap", json!(s.raft_gap));
    set("SupportAngle", json!(format!("{}°", s.support_angle.round())));
    set("XyGap", json!(s.support_xy_gap));
    set("ZGap", json!(s.support_z_gap_layers));
    set("SupportDensity", json!(s.support_density));
    set("SupportSurfaceLayers", json!(s.support_interface_layers));
    set("SupportSurfaceDensity", json!(s.support_interface_density.round() as i64));
    set("SupportBottoms", json!(s.support_bottom_layers));
    set("SupportBottomDensity", json!(s.support_bottom_density.round() as i64));
    set("SupportFlowRatio", json!(s.support_flow.round() as i64));
    set("OuterWallsNumber", json!(s.wall_count));
    set("RandomSeam", json!(s.seam == Seam::Random));
    set("ConcentricInfill", json!(s.infill_pattern == InfillPattern::Concentric));
    set("Bridges", json!(s.bridges));
    set("AutomaticSupport", json!(s.support));
    set("RaftEnabled", json!(s.raft));
    set("RaftLayers", json!(s.raft_layers));
    set("BrimWidth", json!(s.brim_width.round() as i64));
    set("BrimOffset", json!(s.brim_offset));
    v
}

/// Everything needed to write the container once a command stream exists.
pub(crate) struct PackageInfo {
    pub first_z_um: i64,
    pub last_z_um: i64,
    pub min_layer: i32,
    pub max_layer: i32,
    pub layer_count: usize,
}

/// Builds the `.zcodex2` container from an emitted command stream.
pub(crate) fn package(
    e: &Emitted,
    s: &Settings,
    info: &PackageInfo,
    preview_png: Vec<u8>,
    configuration_data: Option<Vec<u8>>,
    warnings: Vec<String>,
) -> Result<Sliced, String> {
    let grams = e.filament_mm * s.material.grams_per_mm;
    let metadata = json!({
        "PrintId": guid_string(),
        "ZCodexVersion": "2.0.0.0",
        "SoftwareVersion": "3.7.0.0",
        "MinFirmwareVersion": s.min_firmware,
        "PrinterModelEnumId": s.printer_model_id,
        "PrinterName": s.printer_name,
        "Materials": [{
            "ExtruderType": "MAIN",
            "Id": s.material.id,
            "Name": s.material.name,
            "ShortName": null,
            "Usage": e.filament_mm.round() as i64,
            "UsageGrams": format!("{:.2}", grams),
            "Temperature": s.nozzle_temp,
        }],
        "HeatbedTemperature": s.bed_temp,
        "ChamberTemperature": -1,
        "CommandCount": e.records.len(),
        "PrintTime": e.print_time_s.round() as i64,
        "NozzleDiameter": s.nozzle_diameter,
        "PrintBoundingBox": {
            "MinX": e.bbox[0], "MinY": e.bbox[1], "MinZ": info.first_z_um,
            "MaxX": e.bbox[3], "MaxY": e.bbox[4], "MaxZ": info.last_z_um,
        },
        "Guid": null,
        "LayerThickness": s.layer_height,
        "TotalMaterialVolumeUsed": 0.0,
        "TotalMaterialWeightUsed": 0.0,
        "TotalLayersCount": info.layer_count,
        "Timestamp": utc_stamp(),
        "ResinMetadata": null,
    });

    let zcode = zcodex2::encode_records(&e.records).map_err(|x| x.to_string())?;
    let json_bytes = |v: &Value| serde_json::to_vec(v).unwrap();
    let mut entries = vec![
        (ADDITIONAL_METADATA.to_string(), json_bytes(&additional_metadata(e, s))),
        (ZCODE_DATA.to_string(), zcode),
    ];
    // Z-Suite refuses to open a file without this entry
    let config = match configuration_data {
        Some(c) => c,
        None => crate::config_data::make_configuration_data(s)?,
    };
    entries.push((CONFIGURATION_DATA.to_string(), config));
    entries.push((USER_SETTINGS_DATA.to_string(), json_bytes(&user_settings(e, s, info.min_layer, info.max_layer))));

    let stats = Stats {
        layers: info.layer_count,
        commands: e.records.len(),
        print_time_s: e.print_time_s,
        filament_mm: e.filament_mm,
        filament_g: grams,
        support_g: e.support_mm * s.material.grams_per_mm,
        bbox_um: [e.bbox[0], e.bbox[1], info.first_z_um, e.bbox[3], e.bbox[4], info.last_z_um],
    };
    let file = Zcodex2 { metadata, preview_png, entries };
    let bytes = file.to_bytes().map_err(|e| e.to_string())?;
    let sliced = Sliced { file, bytes, stats, warnings };
    // never hand out a file that Z-Suite or the printer could choke on
    let problems = crate::verify::verify_bytes(&sliced.file, &sliced.bytes, s);
    if !problems.is_empty() {
        return Err(format!("fichier généré invalide, il n'est pas écrit :
 - {}", problems.join("
 - ")));
    }
    Ok(sliced)
}

/// Slices `mesh` (bed coordinates, mm) and builds the `.zcodex2` file.
///
/// `preview_png` is stored as the file's preview image. `configuration_data`, when given, is copied
/// verbatim as the `ConfigurationData` entry; otherwise a valid one is generated (see `config_data`).
pub fn slice_to_zcodex2(mesh: &Mesh, s: &Settings, preview_png: Vec<u8>, configuration_data: Option<Vec<u8>>) -> Result<Sliced, String> {
    let p = plan(mesh, s)?;
    plan_to_zcodex2(&p, s, preview_png, configuration_data)
}

/// Builds the `.zcodex2` file from an already computed [`Plan`] (what the preview showed), so exporting
/// does not slice a second time. `p` must come from [`plan`] with the same `s`.
pub fn plan_to_zcodex2(p: &crate::toolpath::Plan, s: &Settings, preview_png: Vec<u8>, configuration_data: Option<Vec<u8>>) -> Result<Sliced, String> {
    let clock = std::time::Instant::now();
    let tick = |what: &str| {
        if std::env::var("ZTIME").is_ok() {
            eprintln!("  [export] {what}: {:.0?}", clock.elapsed());
        }
    };
    let e = emit(p, s)?;
    tick("commandes");
    let info = PackageInfo {
        first_z_um: p.layers[0].z,
        last_z_um: p.layers.last().unwrap().z,
        min_layer: p.layers[0].marker,
        max_layer: p.layers.last().unwrap().marker,
        layer_count: p.layers.len(),
    };
    let r = package(&e, s, &info, preview_png, configuration_data, p.warnings.clone());
    tick("conditionnement + vérification");
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::cube_mesh;

    fn tiny_png() -> Vec<u8> {
        crate::preview::top_view_png(&[], [200.0, 200.0], 32)
    }

    #[test]
    fn cube_roundtrips_through_the_container() {
        let mesh = cube_mesh(80.0, 80.0, 20.0);
        let r = slice_to_zcodex2(&mesh, &Settings::default(), tiny_png(), None).unwrap();
        let bytes = r.file.to_bytes().unwrap();
        let back = Zcodex2::from_bytes(&bytes).unwrap();
        let recs = back.records().unwrap(); // verifies every CRC
        assert_eq!(Some(recs.len() as u64), back.command_count());
        assert_eq!(recs.len(), r.stats.commands);
        assert!(r.stats.filament_g > 3.0 && r.stats.filament_g < 40.0, "{}", r.stats.filament_g);
        assert_eq!(back.metadata["PrinterName"], "M200 Plus");
        assert!(matches!(recs.last().unwrap().decode(), zcodex2::Command::EndProgram));
    }

    #[test]
    fn stream_is_physically_sane() {
        let mesh = cube_mesh(80.0, 80.0, 20.0);
        let s = Settings::default();
        let r = slice_to_zcodex2(&mesh, &s, tiny_png(), None).unwrap();
        let recs = r.file.records().unwrap();
        let (mut x, mut y, mut z) = (0i64, 0i64, 0i64);
        let mut max_e = 0i32;
        let mut max_feed = 0i32;
        let mut homed = false;
        for rec in &recs {
            match rec.decode() {
                zcodex2::Command::Home(_) => homed = true,
                zcodex2::Command::Feedrate(f) => max_feed = max_feed.max(f),
                zcodex2::Command::Move(m) => {
                    if let Some(v) = m.x { x = v as i64 }
                    if let Some(v) = m.y { y = v as i64 }
                    if let Some(v) = m.z { z = v as i64 }
                    max_e = max_e.max(m.e.unwrap_or(0));
                    if homed && m.e.map_or(false, |e| e > 0 && m.role < 250) {
                        assert!((0..=200_000).contains(&x) && (0..=200_000).contains(&y), "off plate: {x},{y}");
                        assert!(z >= 260 && z <= 180_000, "z {z}");
                    }
                }
                zcodex2::Command::Rapid(m) => {
                    if let Some(v) = m.x { x = v as i64 }
                    if let Some(v) = m.y { y = v as i64 }
                    if let Some(v) = m.z { z = v as i64 }
                    if homed {
                        assert!((0..=200_000).contains(&x) && (0..=200_000).contains(&y), "travel off plate: {x},{y}");
                        assert!(z >= 0 && z <= 180_000, "z {z}");
                    }
                }
                _ => {}
            }
        }
        assert!(homed);
        assert!(max_feed as f64 <= s.travel_speed * 60.0 + 1.0);
        assert!(max_e < 100_000, "single move pushes too much filament: {max_e}");
    }

    #[test]
    fn configuration_data_is_optional_and_copied_verbatim() {
        let mesh = cube_mesh(80.0, 80.0, 5.0);
        let a = slice_to_zcodex2(&mesh, &Settings::default(), tiny_png(), None).unwrap();
        assert!(a.file.entry(CONFIGURATION_DATA).is_some(), "Z-Suite refuses files without ConfigurationData");
        let b = slice_to_zcodex2(&mesh, &Settings::default(), tiny_png(), Some(vec![1, 2, 3])).unwrap();
        assert_eq!(b.file.entry(CONFIGURATION_DATA), Some(&[1u8, 2, 3][..]));
    }

    #[test]
    fn utc_stamp_has_expected_shape() {
        let t = utc_stamp();
        assert_eq!(t.len(), 18);
        assert!(t.starts_with("20"));
    }
}
