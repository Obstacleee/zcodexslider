//! Randomised settings matrix: many combinations of parameters on several shapes must always produce
//! files that pass the full verification (container, Z-Suite format, command stream), survive a
//! write/read round trip and a trip through G-code. The generator is deterministic (fixed seed), so a
//! failure is reproducible: the message names the seed and the exact settings.
//!
//! More thorough run:  ZSLICE_MATRIX=300 cargo test --release -p zslice --test matrix

mod common;

use zcodex2::Zcodex2;
use zslice::{
    materials, records_to_gcode, slice_to_zcodex2, verify_file, InfillPattern, Seam, Settings, SupportPattern, SupportType, Volume,
};

/// xorshift64*: tiny, deterministic, good enough for picking settings.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    }
    fn u(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.next() % (hi - lo + 1) as u64) as u32
    }
    fn b(&mut self) -> bool {
        self.next() & 1 == 1
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next() % v.len() as u64) as usize]
    }
}

fn random_settings(r: &mut Rng) -> Settings {
    let mut s = Settings::default();
    let mats = materials();
    let m = mats[(r.next() % mats.len() as u64) as usize].clone();
    s.set_material(&m);
    s.layer_height = r.pick(&[0.10, 0.15, 0.15, 0.20, 0.25, 0.30]);
    s.first_layer_height = r.f(0.15, 0.35);
    s.wall_count = r.u(1, 5);
    s.outer_width = r.f(0.35, 0.6);
    s.inner_width = r.f(0.35, 0.6);
    s.outer_first = r.b();
    s.seam = r.pick(&[Seam::Nearest, Seam::Random, Seam::Rear]);
    s.top_layers = r.u(0, 8);
    s.bottom_layers = r.u(0, 6);
    s.solid_width = r.f(0.35, 0.6);
    s.infill_density = r.pick(&[0.0, 5.0, 10.0, 20.0, 35.0, 60.0, 100.0]);
    s.infill_pattern = r.pick(&[InfillPattern::Lines, InfillPattern::Grid, InfillPattern::Triangles, InfillPattern::Concentric]);
    s.infill_width = r.f(0.4, 0.8);
    s.infill_angle = r.f(0.0, 180.0);
    s.bridges = r.b();
    s.flow = r.f(85.0, 115.0);
    s.first_layer_flow = r.f(90.0, 120.0);
    s.bridge_flow = r.f(60.0, 110.0);
    s.outer_speed = r.f(10.0, 60.0);
    s.inner_speed = r.f(10.0, 80.0);
    s.infill_speed = r.f(20.0, 120.0);
    s.top_speed = r.f(10.0, 60.0);
    s.bottom_speed = r.f(10.0, 60.0);
    s.first_layer_speed = r.f(10.0, 50.0);
    s.travel_speed = r.f(60.0, 250.0);
    s.min_layer_time = r.f(0.0, 20.0);
    s.fan_percent = r.u(0, 100) as u8;
    s.fan_start_layer = r.u(0, 6);
    s.retraction_distance = r.pick(&[0.0, 0.5, 0.8, 1.5, 3.0]);
    s.retraction_speed = r.f(20.0, 100.0);
    s.z_hop = r.pick(&[0.0, 0.2, 0.4, 1.0]);
    s.retract_min_travel = r.f(0.5, 15.0);
    s.raft = r.b();
    s.raft_layers = r.u(1, 7);
    s.raft_margin = r.f(1.0, 8.0);
    s.brim_width = if r.b() { r.f(0.0, 8.0) } else { 0.0 };
    s.support = r.b();
    s.support_type = r.pick(&[SupportType::Normal, SupportType::Tree]);
    s.support_pattern = r.pick(&[SupportPattern::Lines, SupportPattern::Grid, SupportPattern::Concentric]);
    s.support_angle = r.f(15.0, 70.0);
    s.support_density = r.f(5.0, 40.0);
    s.support_xy_gap = r.f(0.1, 0.8);
    s.support_z_gap_layers = r.u(0, 3);
    s.support_interface_layers = r.u(0, 4);
    s.support_bottom_layers = r.u(0, 3);
    s.support_expansion = r.pick(&[0.0, 0.0, 0.5, 1.0]);
    s.support_from_bed_only = r.b();
    s.tree_branch_angle = r.f(15.0, 60.0);
    s.tree_tip_diameter = r.f(0.8, 3.0);
    s.tree_max_diameter = r.f(4.0, 12.0);
    s.tree_tip_spacing = r.f(1.5, 5.0);
    if r.next() % 4 == 0 {
        s.support_blockers.push(Volume { min: [90.0, 90.0, 0.0], max: [110.0, 110.0, 5.0] });
    }
    if r.next() % 4 == 0 {
        s.support_enforcers.push(Volume { min: [60.0, 60.0, 4.0], max: [140.0, 140.0, 10.0] });
    }
    s
}

fn describe(seed: u64, name: &str, s: &Settings) -> String {
    format!("seed {seed}, forme {name}, réglages : {}", serde_json::to_string(s).unwrap())
}

fn combos() -> usize {
    std::env::var("ZSLICE_MATRIX").ok().and_then(|v| v.parse().ok()).unwrap_or(24)
}

#[test]
fn every_combination_yields_a_valid_file() {
    let meshes = common::all();
    let n = combos();
    let mut failures = Vec::new();
    for i in 0..n {
        let seed = 0x9E3779B97F4A7C15u64.wrapping_mul(i as u64 + 1);
        let mut rng = Rng(seed | 1);
        let s = random_settings(&mut rng);
        let (name, mesh) = &meshes[i % meshes.len()];
        match slice_to_zcodex2(mesh, &s, common::png(), None) {
            Err(e) => failures.push(format!("{} -> {e}", describe(seed, name, &s))),
            Ok(r) => {
                // an independent second pass over the finished file
                let problems = verify_file(&r.file, &s);
                if !problems.is_empty() {
                    failures.push(format!("{} -> {problems:#?}", describe(seed, name, &s)));
                    continue;
                }
                // write -> read round trip keeps every entry
                let back = Zcodex2::from_bytes(&r.file.to_bytes().unwrap()).unwrap();
                if back.entries != r.file.entries || back.metadata != r.file.metadata {
                    failures.push(format!("{} -> relecture différente", describe(seed, name, &s)));
                }
                if !(r.stats.filament_g > 0.0 && r.stats.print_time_s > 0.0 && r.stats.layers > 0) {
                    failures.push(format!("{} -> statistiques absurdes {:?}", describe(seed, name, &s), r.stats));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} combinaison(s) sur {n} en échec :\n{}", failures.len(), failures.join("\n\n"));
}

#[test]
fn every_third_combination_survives_a_trip_through_gcode() {
    let meshes = common::all();
    let mut failures = Vec::new();
    for i in (0..combos()).step_by(3) {
        let seed = 0xD1B54A32D192ED03u64.wrapping_mul(i as u64 + 1);
        let mut rng = Rng(seed | 1);
        let mut s = random_settings(&mut rng);
        s.nozzle_temp = s.nozzle_temp.clamp(190, 290);
        let (name, mesh) = &meshes[i % meshes.len()];
        let Ok(first) = slice_to_zcodex2(mesh, &s, common::png(), None) else { continue };
        let gcode = records_to_gcode(&first.file.records().unwrap(), "matrix-test");
        match zslice::gcode_to_zcodex2(&gcode, &s, Vec::new(), None) {
            Err(e) => failures.push(format!("{} -> aller-retour G-code refusé : {e}", describe(seed, name, &s))),
            Ok(back) => {
                let (a, b) = (first.stats.filament_mm, back.stats.filament_mm);
                if (a - b).abs() > 0.02 * a + 2.0 {
                    failures.push(format!("{} -> matière {a:.0} mm puis {b:.0} mm après G-code", describe(seed, name, &s)));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn invalid_settings_are_refused_not_written() {
    let mut s = Settings::default();
    s.nozzle_temp = 400;
    assert!(slice_to_zcodex2(&common::cube(), &s, common::png(), None).is_err());
    let mut s = Settings::default();
    s.layer_height = 0.5;
    assert!(slice_to_zcodex2(&common::cube(), &s, common::png(), None).is_err());
    let mut s = Settings::default();
    s.travel_speed = 900.0;
    assert!(slice_to_zcodex2(&common::cube(), &s, common::png(), None).is_err());
}

#[test]
fn old_settings_files_without_new_fields_still_load() {
    // settings saved by an earlier version lack the newest fields: they must fall back to defaults
    let json = r#"{"layer_height": 0.2, "wall_count": 2, "support": true}"#;
    let s: Settings = serde_json::from_str(json).unwrap();
    assert_eq!(s.layer_height, 0.2);
    assert_eq!(s.wall_count, 2);
    assert!(s.support);
    assert_eq!(s.support_type, SupportType::Normal);
    assert!(slice_to_zcodex2(&common::tee(), &s, common::png(), None).is_ok());
}
