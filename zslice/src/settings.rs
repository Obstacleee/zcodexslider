//! All slicing parameters. Units: mm, mm/s, degrees Celsius, percent, unless the name says otherwise.
//!
//! Defaults reproduce what Z-Suite 3.7 writes for a Zortrax M200 Plus, Z-PLA, 0.4 mm nozzle,
//! 0.15 mm layers (measured on real `.zcodex2` files: extrusion per mm, feed rates, retraction,
//! raft and travel behaviour).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InfillPattern {
    /// Parallel lines, direction alternating 45° / 135° every layer.
    Lines,
    /// Lines in both directions on every layer.
    Grid,
    /// Three directions (0°, 60°, 120°), one per layer.
    Triangles,
    /// Concentric loops following the outline.
    Concentric,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportType {
    /// Vertical columns under every overhang.
    Normal,
    /// Branches that grow from the overhang down to the plate, leaning away from the model and merging.
    Tree,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportPattern {
    Lines,
    Grid,
    Concentric,
}

/// A box in model coordinates (mm, z above the model's bottom) used to force or block support.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Volume {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Volume {
    pub fn around(cx: f64, cy: f64, size: f64) -> Volume {
        Volume { min: [cx - size / 2.0, cy - size / 2.0, 0.0], max: [cx + size / 2.0, cy + size / 2.0, size] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Seam {
    /// Start every loop at the point nearest to the previous position.
    Nearest,
    /// Random start point on every loop.
    Random,
    /// Always the rear-most point (largest Y), so the seam lines up.
    Rear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// Material id Z-Suite writes in the file header.
    pub id: i32,
    pub nozzle_temp: i32,
    pub bed_temp: i32,
    /// Grams per millimetre of 1.75 mm filament (taken from Z-Suite's material table).
    pub grams_per_mm: f64,
    /// `true` when the temperatures were measured on a Z-Suite file; the others are typical values.
    pub verified: bool,
}

pub fn materials() -> Vec<Material> {
    let m = |name: &str, id, nozzle, bed, g, verified| Material {
        name: name.into(),
        id,
        nozzle_temp: nozzle,
        bed_temp: bed,
        grams_per_mm: g,
        verified,
    };
    vec![
        m("Z-PLA", 10, 210, 30, 0.0027964, true),
        m("Z-PLA Pro", 11, 210, 60, 0.0032, false),
        m("Z-ABS", 0, 275, 90, 0.00238, false),
        m("Z-ULTRAT", 1, 275, 90, 0.00238, false),
        m("Z-GLASS", 2, 235, 80, 0.0030226, false),
        m("Z-HIPS", 3, 250, 80, 0.0024752, false),
        m("Z-PCABS", 4, 270, 95, 0.0028973, false),
        m("Z-PETG", 5, 240, 70, 0.0030226, false),
        m("Z-ESD", 8, 245, 70, 0.0031, false),
        m("Z-ASA Pro", 12, 260, 90, 0.00279, false),
        m("Z-SEMIFLEX", 14, 235, 40, 0.0024686, false),
    ]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ---- printer
    pub printer_name: String,
    /// Printer id written in the file header (24 = M200 Plus).
    pub printer_model_id: i32,
    pub bed: [f64; 2],
    pub max_z: f64,
    pub nozzle_diameter: f64,
    pub filament_diameter: f64,
    pub min_firmware: String,

    // ---- material and temperatures
    pub material: Material,
    pub nozzle_temp: i32,
    pub bed_temp: i32,
    /// Part-cooling fan, percent, from layer `fan_start_layer`; bridges always use `bridge_fan`.
    pub fan_percent: u8,
    pub fan_start_layer: u32,
    pub bridge_fan: u8,

    // ---- layers
    pub layer_height: f64,
    /// Height of the first layer printed on the plate (no raft) in mm.
    pub first_layer_height: f64,

    // ---- walls
    pub wall_count: u32,
    pub outer_width: f64,
    pub inner_width: f64,
    /// `true`: outer wall first; `false`: inner walls first.
    pub outer_first: bool,
    pub seam: Seam,

    // ---- skins and infill
    pub top_layers: u32,
    pub bottom_layers: u32,
    pub solid_width: f64,
    pub infill_density: f64,
    pub infill_pattern: InfillPattern,
    pub infill_width: f64,
    pub infill_angle: f64,
    /// How far infill lines reach into the walls (mm).
    pub infill_overlap: f64,
    /// Skip solid-skin regions smaller than this (mm²); they become sparse infill.
    pub min_skin_area: f64,

    // ---- flow (percent)
    pub flow: f64,
    pub first_layer_flow: f64,
    pub bridge_flow: f64,
    pub support_flow: f64,
    pub raft_flow: f64,

    // ---- speeds (mm/s)
    pub outer_speed: f64,
    pub inner_speed: f64,
    pub infill_speed: f64,
    pub top_speed: f64,
    pub bottom_speed: f64,
    pub first_layer_speed: f64,
    pub bridge_speed: f64,
    pub support_speed: f64,
    pub support_interface_speed: f64,
    pub raft_base_speed: f64,
    pub raft_speed: f64,
    pub travel_speed: f64,
    pub min_layer_time: f64,
    pub min_speed: f64,

    // ---- retraction
    pub retraction_distance: f64,
    pub retraction_speed: f64,
    /// Extra filament pushed back after a retraction (mm).
    pub retraction_extra: f64,
    pub z_hop: f64,
    pub z_hop_speed: f64,
    /// Travel shorter than this does not retract (mm).
    pub retract_min_travel: f64,

    // ---- raft and brim
    pub raft: bool,
    pub raft_layers: u32,
    pub raft_layer_height: f64,
    pub raft_margin: f64,
    /// Thickness of the first model layer on a raft (Z-Suite "first layer gap").
    pub raft_gap: f64,
    pub brim_width: f64,
    pub brim_offset: f64,

    // ---- supports
    pub support: bool,
    /// Surfaces flatter than this angle from the horizontal get support (Z-Suite default 30°).
    pub support_angle: f64,
    pub support_density: f64,
    pub support_xy_gap: f64,
    pub support_z_gap_layers: u32,
    pub support_interface_layers: u32,
    pub support_interface_density: f64,
    pub support_width: f64,
    /// `true`: only support that can reach the build plate (nothing resting on the model).
    pub support_from_bed_only: bool,
    pub support_type: SupportType,
    pub support_pattern: SupportPattern,
    /// Grow the support area outward by this much (mm).
    pub support_expansion: f64,
    /// Ignore support regions smaller than this (mm²).
    pub support_min_area: f64,
    /// Solid layers where the support starts (on the plate or on the model).
    pub support_bottom_layers: u32,
    pub support_bottom_density: f64,
    /// Boxes inside which support is generated under all material (even without an overhang).
    pub support_enforcers: Vec<Volume>,
    /// Boxes inside which no support is generated.
    pub support_blockers: Vec<Volume>,
    // ---- tree supports
    /// Maximum lean of a branch from the vertical (degrees).
    pub tree_branch_angle: f64,
    /// Diameter of a branch tip touching the model (mm).
    pub tree_tip_diameter: f64,
    /// Maximum trunk diameter (mm).
    pub tree_max_diameter: f64,
    /// Radius growth per mm of height going down.
    pub tree_growth: f64,
    /// Distance between branch tips under a flat overhang (mm).
    pub tree_tip_spacing: f64,

    // ---- bridges
    pub bridges: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let pla = materials().remove(0);
        Settings {
            printer_name: "M200 Plus".into(),
            printer_model_id: 24,
            bed: [200.0, 200.0],
            max_z: 180.0,
            nozzle_diameter: 0.4,
            filament_diameter: 1.75,
            min_firmware: "2.6.0".into(),

            nozzle_temp: pla.nozzle_temp,
            bed_temp: pla.bed_temp,
            material: pla,
            fan_percent: 100,
            fan_start_layer: 2,
            bridge_fan: 100,

            layer_height: 0.15,
            first_layer_height: 0.26,

            wall_count: 3,
            outer_width: 0.46,
            inner_width: 0.45,
            outer_first: true,
            seam: Seam::Nearest,

            top_layers: 4,
            bottom_layers: 3,
            solid_width: 0.5,
            infill_density: 20.0,
            infill_pattern: InfillPattern::Lines,
            infill_width: 0.6,
            infill_angle: 45.0,
            infill_overlap: 0.1,
            min_skin_area: 1.0,

            flow: 100.0,
            first_layer_flow: 100.0,
            bridge_flow: 80.0,
            support_flow: 100.0,
            raft_flow: 100.0,

            // "Normal" quality values measured on a real M200 Plus Z-PLA 0.15 mm file
            outer_speed: 20.0,
            inner_speed: 20.0,
            infill_speed: 70.0,
            top_speed: 35.0,
            bottom_speed: 35.0,
            first_layer_speed: 25.0,
            bridge_speed: 10.0,
            support_speed: 60.0,
            support_interface_speed: 40.0,
            raft_base_speed: 12.5,
            raft_speed: 60.0,
            travel_speed: 200.0,
            min_layer_time: 8.0,
            min_speed: 12.0,

            retraction_distance: 0.8,
            retraction_speed: 60.0,
            retraction_extra: 0.01,
            z_hop: 0.4,
            z_hop_speed: 60.0,
            retract_min_travel: 3.0,

            raft: true,
            raft_layers: 5,
            raft_layer_height: 0.2,
            raft_margin: 5.0,
            raft_gap: 0.3,
            brim_width: 0.0,
            brim_offset: 0.25,

            support: false,
            support_angle: 30.0,
            support_density: 15.0,
            support_xy_gap: 0.16,
            support_z_gap_layers: 1,
            support_interface_layers: 2,
            support_interface_density: 100.0,
            support_width: 0.45,
            support_from_bed_only: false,
            support_type: SupportType::Normal,
            support_pattern: SupportPattern::Lines,
            support_expansion: 0.0,
            support_min_area: 2.0,
            support_bottom_layers: 1,
            support_bottom_density: 100.0,
            support_enforcers: Vec::new(),
            support_blockers: Vec::new(),
            tree_branch_angle: 40.0,
            tree_tip_diameter: 1.2,
            tree_max_diameter: 8.0,
            tree_growth: 0.04,
            tree_tip_spacing: 2.5,

            bridges: true,
        }
    }
}

/// Print quality presets (layer height + matching speeds, measured on Z-Suite files for Draft/Normal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Draft,
    Normal,
    Fine,
}

impl Settings {
    /// Applies a quality preset without touching material, supports, infill or adhesion choices.
    pub fn apply_quality(&mut self, q: Quality) {
        let base = Settings::default();
        let speeds = |s: &mut Settings, from: &Settings, f: f64| {
            s.outer_speed = from.outer_speed * f;
            s.inner_speed = from.inner_speed * f;
            s.infill_speed = from.infill_speed * f;
            s.top_speed = from.top_speed * f;
            s.bottom_speed = from.bottom_speed * f;
            s.first_layer_speed = from.first_layer_speed * f;
        };
        match q {
            Quality::Draft => {
                self.layer_height = 0.20;
                let d = base.clone().draft();
                speeds(self, &d, 1.0);
            }
            Quality::Normal => {
                self.layer_height = 0.15;
                speeds(self, &base, 1.0);
            }
            Quality::Fine => {
                self.layer_height = 0.10;
                speeds(self, &base, 0.8);
            }
        }
    }

    /// The preset closest to the current layer height.
    pub fn quality(&self) -> Quality {
        if self.layer_height >= 0.175 {
            Quality::Draft
        } else if self.layer_height <= 0.125 {
            Quality::Fine
        } else {
            Quality::Normal
        }
    }

    /// The faster "Draft" speeds Z-Suite uses for 0.15 mm / Draft (measured on a real file).
    pub fn draft(mut self) -> Self {
        self.outer_speed = 25.0;
        self.inner_speed = 30.0;
        self.infill_speed = 90.0;
        self.top_speed = 60.0;
        self.bottom_speed = 80.0;
        self.first_layer_speed = 40.0;
        self.raft_base_speed = 15.0;
        self
    }

    pub fn set_material(&mut self, m: &Material) {
        self.material = m.clone();
        self.nozzle_temp = m.nozzle_temp;
        self.bed_temp = m.bed_temp;
    }

    /// Cross-section of the filament in mm².
    pub fn filament_area(&self) -> f64 {
        std::f64::consts::PI / 4.0 * self.filament_diameter * self.filament_diameter
    }

    /// Filament length (mm) needed to lay `length` mm of a bead `width` wide and `height` thick.
    pub fn filament_for(&self, length: f64, width: f64, height: f64, flow_pct: f64) -> f64 {
        length * width * height * (flow_pct / 100.0) * (self.flow / 100.0) / self.filament_area()
    }

    /// Problems that would make the print unsafe or impossible, as human-readable messages.
    pub fn validate(&self) -> Vec<String> {
        let mut v = Vec::new();
        let mut chk = |ok: bool, msg: &str| {
            if !ok {
                v.push(msg.to_owned());
            }
        };
        chk(self.layer_height >= 0.05 && self.layer_height <= self.nozzle_diameter * 0.8, "épaisseur de couche hors limites (0,05 mm à 80 % de la buse)");
        chk(self.first_layer_height >= 0.1 && self.first_layer_height <= self.nozzle_diameter, "épaisseur de première couche hors limites");
        chk((150..=300).contains(&self.nozzle_temp), "température de buse hors de 150–300 °C");
        chk((0..=110).contains(&self.bed_temp), "température du plateau hors de 0–110 °C");
        chk(self.wall_count >= 1, "au moins une paroi");
        chk(self.outer_width >= 0.2 && self.outer_width <= self.nozzle_diameter * 1.6, "largeur de paroi externe hors limites");
        chk(self.inner_width >= 0.2 && self.inner_width <= self.nozzle_diameter * 1.6, "largeur de paroi interne hors limites");
        chk((0.0..=100.0).contains(&self.infill_density), "densité de remplissage hors de 0–100 %");
        chk(self.retraction_distance >= 0.0 && self.retraction_distance <= 6.0, "rétraction hors de 0–6 mm");
        chk(self.z_hop >= 0.0 && self.z_hop <= 5.0, "saut en Z hors de 0–5 mm");
        for (name, s) in [
            ("vitesse paroi externe", self.outer_speed),
            ("vitesse paroi interne", self.inner_speed),
            ("vitesse remplissage", self.infill_speed),
            ("vitesse dessus", self.top_speed),
            ("vitesse dessous", self.bottom_speed),
            ("vitesse première couche", self.first_layer_speed),
            ("vitesse pont", self.bridge_speed),
            ("vitesse support", self.support_speed),
            ("vitesse déplacement", self.travel_speed),
        ] {
            chk(s >= 1.0 && s <= 300.0, &format!("{name} hors de 1–300 mm/s"));
        }
        chk(self.raft_layers >= 1 || !self.raft, "radeau : au moins une couche");
        chk(self.support_angle > 0.0 && self.support_angle < 90.0, "angle de support hors de 0–90°");
        chk(self.tree_branch_angle >= 5.0 && self.tree_branch_angle <= 80.0, "angle des branches hors de 5–80°");
        chk(self.tree_tip_diameter >= 0.4 && self.tree_max_diameter >= self.tree_tip_diameter, "diamètres des branches incohérents");
        v
    }
}
