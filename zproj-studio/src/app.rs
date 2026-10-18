use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use eframe::egui::{self, Color32, RichText};
use zcodex2::Store;
use zproj_studio::mesh::Item;
use zproj_studio::render::{box_wires, render, render_with, Camera, Drawable, Wire};
use zproj_studio::{project, stl};
use zslice::toolpath::Plan;
use zslice::Settings;

use crate::slicing_ui;

#[derive(PartialEq, Clone, Copy)]
enum ViewMode {
    Model,
    Layers,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Model,
    Settings,
    Export,
}

enum Job {
    Plan(Result<Plan, String>, u64, bool),
    Gcode(Result<(Plan, String, String, zslice::Imported), String>),
    Export(Result<(PathBuf, zslice::Stats, Vec<String>), String>),
}

pub struct App {
    items: Vec<Item>,
    sel: usize,
    template: Store,
    template_name: String,
    bed: [f64; 2],
    cam: Camera,
    tex: Option<egui::TextureHandle>,
    last_size: [usize; 2],
    dirty: bool,
    uniform: bool,
    status: (String, bool),
    last_dir: Option<PathBuf>,
    // slicing
    settings: Settings,
    view_mode: ViewMode,
    plan: Option<Plan>,
    layer_idx: usize,
    job: Option<Receiver<Job>>,
    copy_config: bool,
    config_src: Option<PathBuf>,
    slice_info: Option<String>,
    gcode_text: Option<String>,
    gcode_name: String,
    tab: Tab,
    advanced: bool,
    edit_version: u64,
    last_edit: std::time::Instant,
    plan_error: Option<String>,
    plan_is_gcode: bool,
    plan_wires_support: Vec<Wire>,
    plan_wires_base: Vec<Wire>,
    show_support: bool,
    show_base: bool,
    auto_preview: bool,
}

impl App {
    pub fn new() -> Self {
        let bed = [200.0, 200.0];
        App {
            items: Vec::new(),
            sel: 0,
            template: project::bundled_template(),
            template_name: "modèle intégré (M200 Plus, Z-Suite 3.7)".into(),
            bed,
            cam: Camera::for_bed(bed),
            tex: None,
            last_size: [0, 0],
            dirty: true,
            uniform: true,
            status: ("Glisse un fichier .stl ou .zprojx dans la fenêtre, ou utilise les boutons.".into(), false),
            last_dir: None,
            settings: Settings::default(),
            view_mode: ViewMode::Model,
            plan: None,
            layer_idx: 0,
            job: None,
            copy_config: false,
            config_src: None,
            slice_info: None,
            gcode_text: None,
            gcode_name: String::new(),
            tab: Tab::Model,
            advanced: false,
            edit_version: 0,
            last_edit: std::time::Instant::now(),
            plan_error: None,
            plan_is_gcode: false,
            plan_wires_support: Vec::new(),
            plan_wires_base: Vec::new(),
            show_support: true,
            show_base: true,
            auto_preview: true,
        }
    }

    fn say(&mut self, msg: impl Into<String>, err: bool) {
        self.status = (msg.into(), err);
    }

    fn dialog(&self) -> rfd::FileDialog {
        let d = rfd::FileDialog::new();
        match &self.last_dir {
            Some(p) => d.set_directory(p),
            None => d,
        }
    }

    fn load_path(&mut self, path: &Path) {
        self.last_dir = path.parent().map(Path::to_path_buf);
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("stl") => self.open_stl(path),
            Some("zprojx") | Some("zproj") => self.open_project(path),
            _ => self.say(format!("Format non pris en charge : {}", path.display()), true),
        }
    }

    fn open_stl(&mut self, path: &Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "model.stl".into());
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => return self.say(format!("Lecture impossible : {e}"), true),
        };
        match stl::parse(&data) {
            Ok(imp) => {
                self.invalidate();
                // the first model goes to the middle of the bed, the next ones to its right
                let mut it = Item::new(name.clone(), imp.mesh, [self.bed[0] / 2.0, self.bed[1] / 2.0]);
                if let Some(right) = self.items.iter().map(|m| m.bounds().1[0]).reduce(f64::max) {
                    it.pos[0] = right + 5.0 + it.size()[0] / 2.0;
                    it.update();
                }
                let (tris, verts) = (it.mesh.triangles.len(), it.mesh.vertices.len());
                let size = it.size();
                self.items.push(it);
                self.sel = self.items.len() - 1;
                self.frame_all();
                let dropped = if imp.dropped > 0 { format!(" ({} triangles dégénérés ignorés)", imp.dropped) } else { String::new() };
                self.say(
                    format!("{name} : {verts} sommets, {tris} triangles, {:.1} × {:.1} × {:.1} mm{dropped}", size[0], size[1], size[2]),
                    false,
                );
            }
            Err(e) => self.say(format!("{name} : {e}"), true),
        }
    }

    fn open_project(&mut self, path: &Path) {
        match Store::open(path) {
            Ok(store) => match store.models() {
                Ok(models) => {
                    self.invalidate();
                    self.items = models.iter().map(Item::from_model).collect();
                    self.sel = 0;
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.template = store;
                    self.template_name = format!("projet ouvert : {name}");
                    self.frame_all();
                    self.say(format!("{name} ouvert : {} modèle(s). Ses réglages servent de modèle pour l'export.", self.items.len()), false);
                }
                Err(e) => self.say(format!("Modèles illisibles : {e}"), true),
            },
            Err(e) => self.say(format!("Projet illisible : {e}"), true),
        }
    }

    fn frame_all(&mut self) {
        self.dirty = true;
        if self.items.is_empty() {
            self.cam = Camera::for_bed(self.bed);
            return;
        }
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for it in &self.items {
            let (l, h) = it.bounds();
            for i in 0..3 {
                lo[i] = lo[i].min(l[i]);
                hi[i] = hi[i].max(h[i]);
            }
        }
        self.cam.frame(lo, hi);
    }

    fn export(&mut self) {
        if self.items.is_empty() {
            return self.say("Rien à exporter : importe d'abord un fichier STL.", true);
        }
        let stem = self.items[0].name.trim_end_matches(".stl").trim_end_matches(".STL").to_owned();
        let Some(mut path) = self
            .dialog()
            .add_filter("Projet Z-Suite", &["zprojx"])
            .set_file_name(format!("{stem}.zprojx"))
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("zprojx");
        }
        self.last_dir = path.parent().map(Path::to_path_buf);

        let (pw, ph) = project::preview_size(&self.template).unwrap_or((256, 256));
        let mut cam = Camera::for_bed(self.bed);
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for it in &self.items {
            let (l, h) = it.bounds();
            for i in 0..3 {
                lo[i] = lo[i].min(l[i]);
                hi[i] = hi[i].max(h[i]);
            }
        }
        cam.frame(lo, hi);
        let drawables: Vec<Drawable> = self
            .items
            .iter()
            .map(|it| Drawable { vertices: it.placed(), triangles: &it.mesh.triangles, color: [205, 210, 220], limit: None })
            .collect();
        let rgba = render(&drawables, [self.bed[0] as f32, self.bed[1] as f32], &cam, pw as usize, ph as usize);
        let png = project::encode_png(&rgba, pw, ph);

        let store = match project::build_checked(&self.template, &self.items, &png) {
            Ok(s) => s,
            Err(e) => return self.say(e, true),
        };
        match store.save(&path) {
            Ok(()) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.say(format!("Exporté et vérifié : {} ({} Ko). Ouvre-le dans Z-Suite.", path.display(), size / 1024 + 1), false)
            }
            Err(e) => self.say(format!("Export impossible : {e}"), true),
        }
    }


    fn merged_mesh(&self) -> zslice::Mesh {
        let mut m = zslice::Mesh::default();
        for it in &self.items {
            let base = m.vertices.len() as i32;
            m.vertices.extend_from_slice(it.placed());
            m.triangles.extend(it.mesh.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
        }
        m
    }

    fn slice_settings(&self) -> Settings {
        let mut s = self.settings.clone();
        s.bed = self.bed;
        s
    }

    /// Anything that changes the models, the plate or the settings makes the computed plan stale.
    fn invalidate(&mut self) {
        self.edit_version += 1;
        self.last_edit = std::time::Instant::now();
        self.plan = None;
        self.plan_wires_support.clear();
        self.plan_wires_base.clear();
        self.plan_error = None;
        self.plan_is_gcode = false;
        self.dirty = true;
    }

    fn outside_bed(&self) -> bool {
        self.items.iter().any(|it| {
            let (lo, hi) = it.bounds();
            lo[0] < -0.01 || lo[1] < -0.01 || hi[0] > self.bed[0] + 0.01 || hi[1] > self.bed[1] + 0.01
        })
    }

    /// Computes supports / raft / brim in the background whenever they matter, so they show up in the 3D view.
    fn maybe_auto_plan(&mut self, ctx: &egui::Context) {
        if !self.auto_preview || self.job.is_some() || self.plan.is_some() || self.plan_error.is_some() || self.plan_is_gcode || self.items.is_empty() {
            return;
        }
        let s = &self.settings;
        if !(s.support || s.raft || s.brim_width > 0.0) || self.outside_bed() {
            return;
        }
        // wait until the user stops dragging/typing: every keystroke would start a full computation
        let idle = self.last_edit.elapsed();
        let wait = std::time::Duration::from_millis(450);
        if idle < wait {
            ctx.request_repaint_after(wait - idle);
            return;
        }
        self.start_plan(ctx, false);
    }

    fn start_plan(&mut self, ctx: &egui::Context, manual: bool) {
        if self.items.is_empty() {
            if manual {
                self.say("Importe d'abord un modèle.", true);
            }
            return;
        }
        let (mesh, s) = (self.merged_mesh(), self.slice_settings());
        let version = self.edit_version;
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Job::Plan(zslice::toolpath::plan(&mesh, &s), version, manual));
            ctx.request_repaint();
        });
        self.job = Some(rx);
        if manual {
            self.say("Calcul des couches…", false);
        }
    }

    fn start_export_zcodex(&mut self, ctx: &egui::Context) {
        if self.items.is_empty() {
            return self.say("Importe d'abord un modèle.", true);
        }
        let stem = self.items[0].name.trim_end_matches(".stl").trim_end_matches(".STL").to_owned();
        let Some(mut path) = self.dialog().add_filter("Fichier d'impression Zortrax", &["zcodex2"]).set_file_name(format!("{stem}.zcodex2")).save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("zcodex2");
        }
        self.last_dir = path.parent().map(Path::to_path_buf);

        let config = if self.copy_config {
            match &self.config_src {
                Some(src) => match zcodex2::Zcodex2::open(src) {
                    Ok(f) => f.entry(zcodex2::CONFIGURATION_DATA).map(<[u8]>::to_vec),
                    Err(e) => return self.say(format!("Fichier de référence illisible : {e}"), true),
                },
                None => return self.say("Choisis d'abord le .zcodex2 de référence pour ConfigurationData.", true),
            }
        } else {
            None
        };

        let (mesh, s) = (self.merged_mesh(), self.slice_settings());
        // preview image
        let (pw, ph) = (256usize, 256usize);
        let mut cam = Camera::for_bed(self.bed);
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for it in &self.items {
            let (l, h) = it.bounds();
            for i in 0..3 {
                lo[i] = lo[i].min(l[i]);
                hi[i] = hi[i].max(h[i]);
            }
        }
        cam.frame(lo, hi);
        let drawables: Vec<Drawable> = self
            .items
            .iter()
            .map(|it| Drawable { vertices: it.placed(), triangles: &it.mesh.triangles, color: [205, 210, 220], limit: None })
            .collect();
        let rgba = render(&drawables, [self.bed[0] as f32, self.bed[1] as f32], &cam, pw, ph);
        let png = project::encode_png(&rgba, pw as u32, ph as u32);

        // the preview already sliced this exact model with these exact settings: do not slice again
        let ready = if self.plan_is_gcode { None } else { self.plan.clone() };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let sliced = match &ready {
                Some(p) => zslice::plan_to_zcodex2(p, &s, png, config),
                None => zslice::slice_to_zcodex2(&mesh, &s, png, config),
            };
            let res = sliced.and_then(|r| {
                r.save(&path).map_err(|e| format!("écriture impossible : {e}"))?;
                Ok((path, r.stats, r.warnings))
            });
            let _ = tx.send(Job::Export(res));
            ctx.request_repaint();
        });
        self.job = Some(rx);
        self.say("Découpage en cours…", false);
    }


    fn start_gcode_import(&mut self, ctx: &egui::Context) {
        let Some(path) = self.dialog().add_filter("G-code", &["gcode", "gco", "g", "nc"]).pick_file() else { return };
        self.last_dir = path.parent().map(Path::to_path_buf);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let s = self.slice_settings();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res = std::fs::read(&path).map_err(|e| format!("lecture impossible : {e}")).and_then(|b| {
                let text = String::from_utf8_lossy(&b).into_owned();
                let imp = zslice::parse_gcode(&text, &s)?;
                let plan = Plan {
                    layers: imp.layers.clone(),
                    model_layers: imp.layers.len(),
                    raft_layers: 0,
                    raft_top: 0,
                    warnings: imp.warnings.clone(),
                };
                Ok((plan, text, name, imp))
            });
            let _ = tx.send(Job::Gcode(res));
            ctx.request_repaint();
        });
        self.job = Some(rx);
        self.say("Lecture du G-code…", false);
    }

    fn start_gcode_export(&mut self, ctx: &egui::Context) {
        let Some(text) = self.gcode_text.clone() else { return };
        let stem = Path::new(&self.gcode_name).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "print".into());
        let Some(mut path) = self.dialog().add_filter("Fichier d'impression Zortrax", &["zcodex2"]).set_file_name(format!("{stem}.zcodex2")).save_file() else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("zcodex2");
        }
        self.last_dir = path.parent().map(Path::to_path_buf);
        let config = if self.copy_config {
            match self.config_src.as_ref().and_then(|p| zcodex2::Zcodex2::open(p).ok()) {
                Some(f) => f.entry(zcodex2::CONFIGURATION_DATA).map(<[u8]>::to_vec),
                None => return self.say("Choisis d'abord un .zcodex2 de référence lisible pour ConfigurationData.", true),
            }
        } else {
            None
        };
        let s = self.slice_settings();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res = zslice::gcode_to_zcodex2(&text, &s, Vec::new(), config).and_then(|r| {
                r.save(&path).map_err(|e| format!("écriture impossible : {e}"))?;
                Ok((path, r.stats, r.warnings))
            });
            let _ = tx.send(Job::Export(res));
            ctx.request_repaint();
        });
        self.job = Some(rx);
        self.say("Conversion en cours…", false);
    }

    fn poll_job(&mut self) {
        let Some(rx) = &self.job else { return };
        let Ok(job) = rx.try_recv() else { return };
        self.job = None;
        match job {
            Job::Plan(res, version, manual) => {
                if version != self.edit_version {
                    return; // something changed meanwhile: a fresh computation will start
                }
                match res {
                    Ok(p) => {
                        self.layer_idx = self.layer_idx.min(p.layers.len().saturating_sub(1));
                        let info = slicing_ui::plan_summary(&p);
                        let warn = if p.warnings.is_empty() { String::new() } else { format!(" ⚠ {}", p.warnings.join(" ; ")) };
                        self.plan_wires_support = slicing_ui::plan_wires(&p, self.bed, true, false);
                        self.plan_wires_base = slicing_ui::plan_wires(&p, self.bed, false, true);
                        self.plan = Some(p);
                        self.dirty = true;
                        if manual {
                            self.view_mode = ViewMode::Layers;
                            self.say(format!("{info}{warn}"), false);
                        } else if !warn.is_empty() {
                            self.say(warn.trim().to_string(), false);
                        }
                    }
                    Err(e) => {
                        self.plan_error = Some(e.clone());
                        if manual {
                            self.say(format!("Découpage impossible : {e}"), true);
                        } else {
                            self.say(format!("Aperçu des supports indisponible : {e}"), true);
                        }
                    }
                }
            }
            Job::Gcode(Ok((plan, text, name, imp))) => {
                let n = plan.layers.len();
                let warn = if imp.warnings.is_empty() { String::new() } else { format!(" ⚠ {}", imp.warnings.join(" ; ")) };
                self.say(
                    format!(
                        "{name} : {n} couches, durée estimée {}, matière {:.1} m, buse {} °C, plateau {} °C{warn}",
                        zslice::human_time(imp.emitted.print_time_s),
                        imp.emitted.filament_mm / 1000.0,
                        imp.nozzle_temp,
                        imp.bed_temp
                    ),
                    false,
                );
                self.layer_idx = 0;
                self.plan = Some(plan);
                self.plan_is_gcode = true;
                self.plan_wires_support.clear();
                self.plan_wires_base.clear();
                self.gcode_text = Some(text);
                self.gcode_name = name;
                self.view_mode = ViewMode::Layers;
            }
            Job::Gcode(Err(e)) => self.say(format!("G-code refusé : {e}"), true),
            Job::Export(Ok((path, st, warnings))) => {
                let g = st.filament_g;
                let msg = format!(
                    "Exporté et vérifié (conteneur, format Z-Suite, flux) : {} — {} couches, {} commandes, durée estimée {}, matière {:.1} g ({:.1} m){}",
                    path.display(),
                    st.layers,
                    st.commands,
                    zslice::human_time(st.print_time_s),
                    g,
                    st.filament_mm / 1000.0,
                    if st.support_g > 0.0 { format!(", dont supports {:.1} g", st.support_g) } else { String::new() },
                );
                self.slice_info = Some(if warnings.is_empty() { msg.clone() } else { format!("{msg} ⚠ {}", warnings.join(" ; ")) });
                self.say(msg, false);
            }
            Job::Export(Err(e)) => self.say(format!("Export impossible : {e}"), true),
        }
    }

    fn layer_view(&mut self, ui: &mut egui::Ui) {
        let Some(plan) = &self.plan else {
            ui.centered_and_justified(|ui| {
                ui.label("Clique sur « Aperçu des couches » pour calculer le découpage.");
            });
            return;
        };
        let n = plan.layers.len();
        ui.horizontal(|ui| {
            ui.label("Couche");
            ui.add(egui::Slider::new(&mut self.layer_idx, 0..=n - 1));
            let l = &plan.layers[self.layer_idx.min(n - 1)];
            ui.label(format!(
                "n°{} • z = {:.2} mm • épaisseur {:.2} mm • {} trajets{}",
                l.marker,
                l.z as f64 / 1000.0,
                l.thickness,
                l.paths.len(),
                if l.is_raft { " • radeau" } else { "" }
            ));
        });
        slicing_ui::role_legend(ui);
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let i = self.layer_idx.min(n - 1);
        slicing_ui::draw_layer(&ui.painter_at(rect), rect, self.bed, &plan.layers[i], if i > 0 { Some(&plan.layers[i - 1]) } else { None });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (t, label) in [(Tab::Model, "① Modèle"), (Tab::Settings, "② Réglages"), (Tab::Export, "③ Exporter")] {
                ui.selectable_value(&mut self.tab, t, RichText::new(label).size(15.0));
            }
        });
        ui.separator();
        match self.tab {
            Tab::Model => self.model_tab(ui),
            Tab::Settings => self.settings_tab(ui),
            Tab::Export => self.export_tab(ui),
        }
    }

    // ------------------------------------------------------------------------------ tab 1: model

    fn model_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Ouvrir un STL…").clicked() {
                if let Some(paths) = self.dialog().add_filter("STL", &["stl", "STL"]).pick_files() {
                    for p in paths {
                        self.load_path(&p);
                    }
                }
            }
            if ui.button("Ouvrir un .zprojx…").clicked() {
                if let Some(p) = self.dialog().add_filter("Projet Z-Suite", &["zprojx", "zproj"]).pick_file() {
                    self.load_path(&p);
                }
            }
        });
        ui.weak("Tu peux aussi glisser un fichier .stl ou .zprojx dans la fenêtre.");

        if self.items.is_empty() {
            ui.add_space(6.0);
            ui.weak("Aucun modèle pour l'instant.");
        } else if self.outside_bed() {
            ui.add_space(6.0);
            ui.colored_label(Color32::from_rgb(255, 90, 90), "⚠ Une partie dépasse du plateau (en rouge dans la vue 3D). Déplace ou réduis la pièce : l'export serait refusé.");
        } else {
            ui.add_space(6.0);
            ui.colored_label(Color32::from_rgb(110, 210, 130), "✔ La pièce tient sur le plateau");
        }

        ui.separator();
        let mut remove = None;
        for (i, it) in self.items.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.selectable_label(self.sel == i, &it.name).clicked() {
                    self.sel = i;
                }
                if ui.small_button("✖").on_hover_text("Retirer").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.items.remove(i);
            self.sel = self.sel.min(self.items.len().saturating_sub(1));
            self.invalidate();
        }

        let bed = self.bed;
        let mut changed = false;
        let mut center = false;
        if let Some(it) = self.items.get_mut(self.sel) {
            ui.separator();
            ui.text_edit_singleline(&mut it.name);
            let s = it.size();
            ui.label(format!("{:.1} × {:.1} × {:.1} mm   •   {} triangles", s[0], s[1], s[2], it.mesh.triangles.len()));

            ui.add_space(4.0);
            ui.label(RichText::new("Taille (%)").strong());
            ui.checkbox(&mut self.uniform, "proportionnelle");
            let mut pct = [it.scale[0] * 100.0, it.scale[1] * 100.0, it.scale[2] * 100.0];
            ui.horizontal(|ui| {
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.label(*axis);
                    let r = ui.add(egui::DragValue::new(&mut pct[k]).speed(0.5).range(1.0..=10000.0));
                    if r.changed() {
                        if self.uniform {
                            let v = pct[k];
                            pct = [v, v, v];
                        }
                        it.scale = [pct[0] / 100.0, pct[1] / 100.0, pct[2] / 100.0];
                        changed = true;
                    }
                }
            });

            ui.add_space(4.0);
            ui.label(RichText::new("Rotation (°)").strong());
            ui.horizontal(|ui| {
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    ui.label(*axis);
                    changed |= ui.add(egui::DragValue::new(&mut it.rot[k]).speed(1.0)).changed();
                }
            });
            ui.horizontal(|ui| {
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    if ui.small_button(format!("{axis} +90°")).clicked() {
                        it.rot[k] = (it.rot[k] + 90.0) % 360.0;
                        changed = true;
                    }
                }
                if ui.small_button("0").on_hover_text("Remettre la rotation à zéro").clicked() {
                    it.rot = [0.0; 3];
                    changed = true;
                }
            });

            ui.add_space(4.0);
            ui.label(RichText::new("Position sur le plateau (mm)").strong());
            ui.horizontal(|ui| {
                ui.label("X");
                changed |= ui.add(egui::DragValue::new(&mut it.pos[0]).speed(0.5)).changed();
                ui.label("Y");
                changed |= ui.add(egui::DragValue::new(&mut it.pos[1]).speed(0.5)).changed();
                ui.label("Z");
                changed |= ui.add(egui::DragValue::new(&mut it.lift).speed(0.1).range(0.0..=500.0)).on_hover_text("hauteur au-dessus du plateau").changed();
            });
            if ui.button("Centrer sur le plateau").clicked() {
                center = true;
            }
            ui.checkbox(&mut it.unrepaired, "marquer « non réparé »");
            if changed || center {
                if center {
                    it.pos = [bed[0] / 2.0, bed[1] / 2.0];
                }
                it.update();
            }
        }
        if changed || center {
            self.invalidate();
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Plateau");
            let a = ui.add(egui::DragValue::new(&mut self.bed[0]).speed(1.0).range(50.0..=1000.0).suffix(" mm"));
            ui.label("×");
            let b = ui.add(egui::DragValue::new(&mut self.bed[1]).speed(1.0).range(50.0..=1000.0).suffix(" mm"));
            if a.changed() || b.changed() {
                self.invalidate();
            }
        });
    }

    // ------------------------------------------------------------------------------ tab 2: settings

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Mode :");
            ui.selectable_value(&mut self.advanced, false, "Simple");
            ui.selectable_value(&mut self.advanced, true, "Avancé (tous les réglages)");
        });
        ui.add_space(4.0);
        let anchor = self.items.iter().map(|it| it.pos).next().unwrap_or([self.bed[0] / 2.0, self.bed[1] / 2.0]);
        let changed = if self.advanced {
            slicing_ui::settings_ui(ui, &mut self.settings, anchor)
        } else {
            slicing_ui::simple_ui(ui, &mut self.settings)
        };
        if changed {
            self.invalidate();
        }
        for msg in self.slice_settings().validate() {
            ui.colored_label(Color32::from_rgb(240, 110, 100), format!("⚠ {msg}"));
        }
        ui.separator();
        if ui.checkbox(&mut self.auto_preview, "Calculer les supports / radeau et les montrer en 3D").changed() {
            self.dirty = true;
        }
        ui.horizontal(|ui| {
            if ui.button("Enregistrer les réglages…").clicked() {
                if let Some(p) = self.dialog().add_filter("Réglages", &["json"]).set_file_name("reglages.json").save_file() {
                    match serde_json_to_file(&self.settings, &p) {
                        Ok(()) => self.say(format!("Réglages enregistrés : {}", p.display()), false),
                        Err(e) => self.say(e, true),
                    }
                }
            }
            if ui.button("Charger des réglages…").clicked() {
                if let Some(p) = self.dialog().add_filter("Réglages", &["json"]).pick_file() {
                    match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| serde_json_from_str(&t)) {
                        Ok(s) => {
                            self.settings = s;
                            self.invalidate();
                            self.say("Réglages chargés.", false);
                        }
                        Err(e) => self.say(format!("Réglages illisibles : {e}"), true),
                    }
                }
            }
        });
    }

    // ------------------------------------------------------------------------------ tab 3: export

    fn export_tab(&mut self, ui: &mut egui::Ui) {
        ui.colored_label(
            Color32::from_rgb(240, 170, 60),
            "Fichiers générés par ce logiciel : non validés sur imprimante. Surveille la première couche et garde la main sur l'arrêt.",
        );
        ui.add_space(6.0);
        let busy = self.job.is_some();

        ui.label(RichText::new("Fichier d'impression (.zcodex2)").strong().size(15.0));
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Aperçu des couches")).clicked() {
                let ctx = ui.ctx().clone();
                self.start_plan(&ctx, true);
            }
            if ui.add_enabled(!busy, egui::Button::new(RichText::new("Trancher et enregistrer…").strong())).clicked() {
                let ctx = ui.ctx().clone();
                self.start_export_zcodex(&ctx);
            }
        });
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("calcul en cours…");
            });
        }
        if let Some(info) = &self.slice_info {
            ui.label(info);
        }
        ui.checkbox(&mut self.copy_config, "Reprendre ConfigurationData d'un fichier Z-Suite (optionnel)")
            .on_hover_text("Une ConfigurationData valide est générée automatiquement (Z-Suite refuse d'ouvrir un fichier qui n'en a pas). Cette option copie à la place celle d'un .zcodex2 fait par Z-Suite.");
        if self.copy_config {
            ui.horizontal(|ui| {
                if ui.button("Choisir le .zcodex2…").clicked() {
                    if let Some(p) = self.dialog().add_filter("Fichier d'impression Zortrax", &["zcodex2"]).pick_file() {
                        self.config_src = Some(p);
                    }
                }
                ui.label(self.config_src.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "aucun".into()));
            });
        }

        ui.separator();
        ui.label(RichText::new("G-code d'un autre slicer").strong().size(15.0));
        ui.weak("PrusaSlicer, OrcaSlicer, Cura, Simplify3D... : lis le G-code (aperçu des couches) puis convertis-le.");
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Ouvrir un G-code…")).clicked() {
                let ctx = ui.ctx().clone();
                self.start_gcode_import(&ctx);
            }
            let can = self.gcode_text.is_some() && !busy;
            if ui.add_enabled(can, egui::Button::new("Convertir en .zcodex2…")).clicked() {
                let ctx = ui.ctx().clone();
                self.start_gcode_export(&ctx);
            }
        });
        if !self.gcode_name.is_empty() {
            ui.weak(format!("G-code chargé : {}", self.gcode_name));
        }

        ui.separator();
        ui.label(RichText::new("Projet Z-Suite (.zprojx)").strong().size(15.0));
        ui.label(format!("Réglages repris de : {}", self.template_name));
        ui.horizontal(|ui| {
            if ui.button("Autre projet comme modèle…").clicked() {
                if let Some(p) = self.dialog().add_filter("Projet Z-Suite", &["zprojx", "zproj"]).pick_file() {
                    match Store::open(&p) {
                        Ok(s) => {
                            self.template = s;
                            self.template_name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            self.say("Modèle de réglages changé (les modèles 3D déjà importés sont conservés).", false);
                        }
                        Err(e) => self.say(format!("Projet illisible : {e}"), true),
                    }
                }
            }
            if ui.button("Intégré").clicked() {
                self.template = project::bundled_template();
                self.template_name = "modèle intégré (M200 Plus, Z-Suite 3.7)".into();
            }
        });
        if ui.add_sized([ui.available_width(), 30.0], egui::Button::new("Enregistrer en .zprojx…")).clicked() {
            self.export();
        }
    }

    fn view(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let (w, h) = (rect.width().max(16.0) as usize, rect.height().max(16.0) as usize);

        if resp.dragged_by(egui::PointerButton::Primary) {
            let d = resp.drag_delta();
            self.cam.yaw -= d.x * 0.01;
            self.cam.pitch = (self.cam.pitch + d.y * 0.01).clamp(-1.4, 1.5);
            self.dirty = true;
        }
        if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
            let d = resp.drag_delta();
            let k = self.cam.radius * 2.6 / self.cam.zoom / h as f32 * 0.9;
            let (sy, cy) = self.cam.yaw.sin_cos();
            // screen-right is (-sin, cos), screen-up on the ground is (-cos, -sin)
            self.cam.target[0] += (sy * d.x + cy * d.y) * k;
            self.cam.target[1] += (-cy * d.x + sy * d.y) * k;
            self.dirty = true;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.cam.zoom = (self.cam.zoom * (1.0 + scroll * 0.002)).clamp(0.1, 40.0);
                self.dirty = true;
            }
        }
        if resp.double_clicked() {
            self.frame_all();
        }

        if self.dirty || self.last_size != [w, h] || self.tex.is_none() {
            // with a raft the model really sits above it: lift everything by the raft height
            let lift = match (&self.plan, self.plan_is_gcode) {
                (Some(p), false) => p.raft_top as f64 / 1000.0,
                _ => 0.0,
            };
            let lifted: Vec<Vec<[f64; 3]>> = if lift > 0.0 {
                self.items.iter().map(|it| it.placed().iter().map(|v| [v[0], v[1], v[2] + lift]).collect()).collect()
            } else {
                Vec::new()
            };
            let limit = Some([self.bed[0] as f32, self.bed[1] as f32]);
            let drawables: Vec<Drawable> = self
                .items
                .iter()
                .enumerate()
                .map(|(i, it)| Drawable {
                    vertices: if lift > 0.0 { &lifted[i] } else { it.placed() },
                    triangles: &it.mesh.triangles,
                    color: if i == self.sel { [120, 180, 255] } else { [190, 195, 205] },
                    limit,
                })
                .collect();
            let mut wires: Vec<Wire> = Vec::new();
            for v in &self.settings.support_enforcers {
                wires.extend(box_wires([v.min[0], v.min[1], v.min[2] + lift], [v.max[0], v.max[1], v.max[2] + lift], [90, 230, 130]));
            }
            for v in &self.settings.support_blockers {
                wires.extend(box_wires([v.min[0], v.min[1], v.min[2] + lift], [v.max[0], v.max[1], v.max[2] + lift], [240, 90, 90]));
            }
            if self.show_support {
                wires.extend(self.plan_wires_support.iter().cloned());
            }
            if self.show_base {
                wires.extend(self.plan_wires_base.iter().cloned());
            }
            let rgba = render_with(&drawables, &wires, [self.bed[0] as f32, self.bed[1] as f32], &self.cam, w, h);
            let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            match &mut self.tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.tex = Some(ui.ctx().load_texture("view", img, egui::TextureOptions::LINEAR)),
            }
            self.last_size = [w, h];
            self.dirty = false;
        }
        if let Some(t) = &self.tex {
            ui.painter().image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        if self.outside_bed() {
            ui.painter().text(
                rect.left_top() + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                "⚠ La pièce dépasse du plateau : les parties en rouge ne tiennent pas",
                egui::FontId::proportional(15.0),
                Color32::from_rgb(255, 90, 90),
            );
        } else if let Some(e) = &self.plan_error {
            ui.painter().text(
                rect.left_top() + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                format!("Supports non calculés : {e}"),
                egui::FontId::proportional(13.0),
                Color32::from_rgb(240, 170, 60),
            );
        }
        ui.painter().text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            "glisser : tourner • clic droit : déplacer • molette : zoom • double-clic : recadrer",
            egui::FontId::proportional(12.0),
            Color32::from_gray(150),
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let dropped: Vec<PathBuf> =
            ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            self.load_path(&p);
        }

        egui::Panel::bottom("status").show(ui, |ui| {
            let (msg, err) = &self.status;
            ui.colored_label(if *err { Color32::from_rgb(240, 110, 100) } else { Color32::from_gray(190) }, msg);
        });
        egui::Panel::left("controls").min_size(330.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.side_panel(ui));
        });
        self.poll_job();
        let ctx = ui.ctx().clone();
        self.maybe_auto_plan(&ctx);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.view_mode, ViewMode::Model, "Vue 3D");
                ui.selectable_value(&mut self.view_mode, ViewMode::Layers, "Couches");
                if self.view_mode == ViewMode::Model {
                    ui.separator();
                    if ui.checkbox(&mut self.show_support, "Supports").changed() {
                        self.dirty = true;
                    }
                    if ui.checkbox(&mut self.show_base, "Radeau / bord").changed() {
                        self.dirty = true;
                    }
                    if self.job.is_some() {
                        ui.spinner();
                    }
                }
            });
            match self.view_mode {
                ViewMode::Model => self.view(ui),
                ViewMode::Layers => self.layer_view(ui),
            }
        });
    }
}

fn serde_json_to_file(s: &Settings, p: &Path) -> Result<(), String> {
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(p, text).map_err(|e| format!("écriture impossible : {e}"))
}

fn serde_json_from_str(t: &str) -> Result<Settings, String> {
    serde_json::from_str(t).map_err(|e| e.to_string())
}
