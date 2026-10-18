#![recursion_limit = "512"]
//! FDM slicer that writes Zortrax `.zcodex2` files.

pub mod compat;
pub mod config_data;
pub mod emit;
pub mod export;
pub mod geom;
pub mod gcode_out;
pub mod import;
pub mod preview;
pub mod settings;
pub mod slicer;
pub mod support;
pub mod toolpath;
pub mod verify;

pub use settings::{materials, InfillPattern, Material, Quality, Seam, Settings, SupportPattern, SupportType, Volume};
pub use slicer::Mesh;
pub use compat::check as check_zsuite_compat;
pub use verify::verify as verify_file;
pub use gcode_out::records_to_gcode;
pub use preview::top_view_png;
pub use import::{gcode_to_zcodex2, parse_gcode, Imported};
pub use export::{human_time, plan_to_zcodex2, slice_to_zcodex2, Sliced, Stats};

#[cfg(test)]
pub(crate) fn slicer_test_cube(x: f64, y: f64, s: f64) -> Mesh {
    slicer::cube_mesh(x, y, s)
}
