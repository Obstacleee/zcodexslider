//! compare <project.zprojx> <reference.zcodex2>: slice the project's model and compare with Z-Suite's output.
use zcodex2::{Store, Zcodex2};
use zslice::{slice_to_zcodex2, human_time, Mesh, Settings};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let store = Store::open(&a[1]).expect("project");
    let models = store.models().expect("models");
    let m = &models[0];
    let mesh = Mesh { vertices: m.vertices.clone(), triangles: m.triangles.clone() };
    let reference = Zcodex2::open(&a[2]).expect("reference");

    let mut s = Settings::default().draft();
    s.wall_count = 3;
    s.raft = true;
    s.raft_layers = 5;
    let t = std::time::Instant::now();
    let r = slice_to_zcodex2(&mesh, &s, placeholder_png(), None).expect("slice");
    let took = t.elapsed();
    let rm = &reference.metadata;
    let rs = &r.file.metadata;
    println!("slicing took {took:?}; warnings: {:?}", r.warnings);
    let row = |name: &str, mine: String, theirs: String| println!("{name:22} mine {mine:>14}   Z-Suite {theirs:>14}");
    row("print time", human_time(r.stats.print_time_s), human_time(rm["PrintTime"].as_f64().unwrap()));
    row("filament (mm)", format!("{:.0}", r.stats.filament_mm), rm["Materials"][0]["Usage"].to_string());
    row("filament (g)", format!("{:.2}", r.stats.filament_g), rm["Materials"][0]["UsageGrams"].to_string());
    row("layers", r.stats.layers.to_string(), rm["TotalLayersCount"].to_string());
    row("commands", r.stats.commands.to_string(), rm["CommandCount"].to_string());
    for k in ["MinX", "MinY", "MaxX", "MaxY", "MinZ", "MaxZ"] {
        row(k, rs["PrintBoundingBox"][k].to_string(), rm["PrintBoundingBox"][k].to_string());
    }
    if let Some(out) = a.get(3) {
        r.save(out).expect("save");
        println!("wrote {out}");
    }
}

/// A valid 128x128 grey PNG (the GUI renders a real preview; examples use this placeholder).
fn placeholder_png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 128, 128);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        let px: Vec<u8> = (0..128 * 128).flat_map(|_| [60u8, 64, 72, 255]).collect();
        w.write_image_data(&px).unwrap();
    }
    out
}
