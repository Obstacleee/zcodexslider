#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod app;
mod slicing_ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 780.0])
            .with_title("ZProj Studio - STL vers Z-Suite (.zprojx et .zcodex2)")
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("ZProj Studio", options, Box::new(|_cc| Ok(Box::new(app::App::new()))))
}
