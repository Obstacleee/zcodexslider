//! Slicing settings panel and layer preview drawing.

use eframe::egui::{self, Color32, RichText};
use zslice::toolpath::{role, Layer, Plan};
use zslice::{materials, InfillPattern, Seam, Settings, SupportPattern, SupportType, Volume};

struct Ctx<'a> {
    changed: &'a mut bool,
}

fn num(ui: &mut egui::Ui, c: &mut Ctx, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, speed: f64, suffix: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.add(egui::DragValue::new(v).speed(speed).range(range).suffix(suffix)).changed() {
            *c.changed = true;
        }
    });
}

fn int(ui: &mut egui::Ui, c: &mut Ctx, label: &str, v: &mut u32, range: std::ops::RangeInclusive<u32>) {
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.add(egui::DragValue::new(v).range(range)).changed() {
            *c.changed = true;
        }
    });
}

fn flag(ui: &mut egui::Ui, c: &mut Ctx, label: &str, v: &mut bool) {
    if ui.checkbox(v, label).changed() {
        *c.changed = true;
    }
}

/// All slicing parameters. Returns `true` when something changed.
pub fn settings_ui(ui: &mut egui::Ui, s: &mut Settings, anchor: [f64; 2]) -> bool {
    let mut changed = false;
    let mut c = Ctx { changed: &mut changed };

    ui.horizontal(|ui| {
        if ui.button("Préréglage Normal").on_hover_text("vitesses d'un fichier Z-Suite « Normal » 0,15 mm").clicked() {
            let keep = (s.material.clone(), s.nozzle_temp, s.bed_temp, s.bed);
            *s = Settings::default();
            s.material = keep.0;
            s.nozzle_temp = keep.1;
            s.bed_temp = keep.2;
            s.bed = keep.3;
            *c.changed = true;
        }
        if ui.button("Préréglage Brouillon").on_hover_text("vitesses d'un fichier Z-Suite « Draft » 0,15 mm").clicked() {
            let keep = (s.material.clone(), s.nozzle_temp, s.bed_temp, s.bed);
            *s = Settings::default().draft();
            s.material = keep.0;
            s.nozzle_temp = keep.1;
            s.bed_temp = keep.2;
            s.bed = keep.3;
            *c.changed = true;
        }
    });

    egui::CollapsingHeader::new("Matériau et températures").default_open(true).show(ui, |ui| {
        let cur = s.material.name.clone();
        egui::ComboBox::from_label("Matériau").selected_text(&cur).show_ui(ui, |ui| {
            for m in materials() {
                if ui.selectable_label(m.name == cur, format!("{}{}", m.name, if m.verified { "" } else { "  (valeurs typiques)" })).clicked() {
                    s.set_material(&m);
                    *c.changed = true;
                }
            }
        });
        if !s.material.verified {
            ui.colored_label(Color32::from_rgb(240, 170, 60), "Températures typiques, non reprises d'un fichier Z-Suite : vérifie-les.");
        }
        let mut t = s.nozzle_temp as f64;
        num(ui, &mut c, "Buse", &mut t, 150.0..=300.0, 1.0, " °C");
        s.nozzle_temp = t as i32;
        let mut b = s.bed_temp as f64;
        num(ui, &mut c, "Plateau", &mut b, 0.0..=110.0, 1.0, " °C");
        s.bed_temp = b as i32;
        num(ui, &mut c, "Diamètre buse", &mut s.nozzle_diameter, 0.2..=1.0, 0.01, " mm");
        num(ui, &mut c, "Diamètre filament", &mut s.filament_diameter, 1.0..=3.0, 0.01, " mm");
        let mut fan = s.fan_percent as f64;
        num(ui, &mut c, "Ventilateur", &mut fan, 0.0..=100.0, 1.0, " %");
        s.fan_percent = fan as u8;
        int(ui, &mut c, "Ventilateur dès la couche", &mut s.fan_start_layer, 0..=50);
    });

    egui::CollapsingHeader::new("Couches").default_open(true).show(ui, |ui| {
        num(ui, &mut c, "Épaisseur de couche", &mut s.layer_height, 0.05..=0.32, 0.005, " mm");
        num(ui, &mut c, "Première couche", &mut s.first_layer_height, 0.1..=0.4, 0.01, " mm");
    });

    egui::CollapsingHeader::new("Parois").default_open(true).show(ui, |ui| {
        int(ui, &mut c, "Nombre de parois", &mut s.wall_count, 1..=10);
        num(ui, &mut c, "Largeur paroi externe", &mut s.outer_width, 0.2..=1.0, 0.01, " mm");
        num(ui, &mut c, "Largeur parois internes", &mut s.inner_width, 0.2..=1.0, 0.01, " mm");
        flag(ui, &mut c, "Paroi externe en premier", &mut s.outer_first);
        let cur = s.seam;
        egui::ComboBox::from_label("Couture").selected_text(match cur {
            Seam::Nearest => "Au plus proche",
            Seam::Random => "Aléatoire",
            Seam::Rear => "À l'arrière",
        }).show_ui(ui, |ui| {
            for (v, name) in [(Seam::Nearest, "Au plus proche"), (Seam::Random, "Aléatoire"), (Seam::Rear, "À l'arrière")] {
                if ui.selectable_label(cur == v, name).clicked() {
                    s.seam = v;
                    *c.changed = true;
                }
            }
        });
    });

    egui::CollapsingHeader::new("Remplissage et surfaces").default_open(true).show(ui, |ui| {
        num(ui, &mut c, "Densité de remplissage", &mut s.infill_density, 0.0..=100.0, 0.5, " %");
        let cur = s.infill_pattern;
        let label = |p: InfillPattern| match p {
            InfillPattern::Lines => "Lignes",
            InfillPattern::Grid => "Grille",
            InfillPattern::Triangles => "Triangles",
            InfillPattern::Concentric => "Concentrique",
        };
        egui::ComboBox::from_label("Motif").selected_text(label(cur)).show_ui(ui, |ui| {
            for p in [InfillPattern::Lines, InfillPattern::Grid, InfillPattern::Triangles, InfillPattern::Concentric] {
                if ui.selectable_label(cur == p, label(p)).clicked() {
                    s.infill_pattern = p;
                    *c.changed = true;
                }
            }
        });
        num(ui, &mut c, "Largeur remplissage", &mut s.infill_width, 0.2..=1.2, 0.01, " mm");
        num(ui, &mut c, "Angle remplissage", &mut s.infill_angle, 0.0..=180.0, 1.0, " °");
        num(ui, &mut c, "Recouvrement parois", &mut s.infill_overlap, 0.0..=0.5, 0.01, " mm");
        int(ui, &mut c, "Couches du dessus", &mut s.top_layers, 0..=20);
        int(ui, &mut c, "Couches du dessous", &mut s.bottom_layers, 0..=20);
        num(ui, &mut c, "Largeur des surfaces", &mut s.solid_width, 0.2..=1.0, 0.01, " mm");
        num(ui, &mut c, "Surface pleine min.", &mut s.min_skin_area, 0.0..=50.0, 0.1, " mm²");
        flag(ui, &mut c, "Ponts (couches sur le vide)", &mut s.bridges);
    });

    egui::CollapsingHeader::new("Vitesses (mm/s)").show(ui, |ui| {
        num(ui, &mut c, "Paroi externe", &mut s.outer_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Parois internes", &mut s.inner_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Remplissage", &mut s.infill_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Dessus", &mut s.top_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Dessous", &mut s.bottom_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Première couche", &mut s.first_layer_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Ponts", &mut s.bridge_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Supports", &mut s.support_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Interface supports", &mut s.support_interface_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Base du radeau", &mut s.raft_base_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Radeau", &mut s.raft_speed, 1.0..=300.0, 0.5, "");
        num(ui, &mut c, "Déplacements", &mut s.travel_speed, 1.0..=300.0, 1.0, "");
        num(ui, &mut c, "Temps min. par couche", &mut s.min_layer_time, 0.0..=60.0, 0.5, " s");
        num(ui, &mut c, "Vitesse min. (refroidissement)", &mut s.min_speed, 1.0..=100.0, 0.5, "");
    });

    egui::CollapsingHeader::new("Débit (%)").show(ui, |ui| {
        num(ui, &mut c, "Débit global", &mut s.flow, 50.0..=150.0, 0.5, " %");
        num(ui, &mut c, "Première couche", &mut s.first_layer_flow, 50.0..=150.0, 0.5, " %");
        num(ui, &mut c, "Ponts", &mut s.bridge_flow, 30.0..=150.0, 0.5, " %");
        num(ui, &mut c, "Supports", &mut s.support_flow, 30.0..=150.0, 0.5, " %");
        num(ui, &mut c, "Radeau", &mut s.raft_flow, 30.0..=150.0, 0.5, " %");
    });

    egui::CollapsingHeader::new("Rétraction").show(ui, |ui| {
        num(ui, &mut c, "Distance", &mut s.retraction_distance, 0.0..=6.0, 0.05, " mm");
        num(ui, &mut c, "Vitesse", &mut s.retraction_speed, 5.0..=150.0, 1.0, " mm/s");
        num(ui, &mut c, "Rechargement supplémentaire", &mut s.retraction_extra, 0.0..=0.5, 0.01, " mm");
        num(ui, &mut c, "Saut en Z", &mut s.z_hop, 0.0..=5.0, 0.05, " mm");
        num(ui, &mut c, "Vitesse du saut en Z", &mut s.z_hop_speed, 5.0..=150.0, 1.0, " mm/s");
        num(ui, &mut c, "Déplacement min. pour rétracter", &mut s.retract_min_travel, 0.0..=50.0, 0.1, " mm");
    });

    egui::CollapsingHeader::new("Radeau et bord").show(ui, |ui| {
        flag(ui, &mut c, "Radeau", &mut s.raft);
        int(ui, &mut c, "Couches du radeau", &mut s.raft_layers, 1..=12);
        num(ui, &mut c, "Épaisseur couches radeau", &mut s.raft_layer_height, 0.1..=0.35, 0.01, " mm");
        num(ui, &mut c, "Marge du radeau", &mut s.raft_margin, 0.0..=20.0, 0.1, " mm");
        num(ui, &mut c, "Première couche sur radeau", &mut s.raft_gap, 0.1..=0.5, 0.01, " mm");
        num(ui, &mut c, "Bord (brim), largeur", &mut s.brim_width, 0.0..=20.0, 0.1, " mm");
        num(ui, &mut c, "Bord, distance au modèle", &mut s.brim_offset, 0.0..=2.0, 0.05, " mm");
        if s.raft && s.brim_width > 0.0 {
            ui.weak("Le bord n'est utilisé que sans radeau.");
        }
    });

    egui::CollapsingHeader::new("Supports").show(ui, |ui| {
        flag(ui, &mut c, "Supports automatiques", &mut s.support);
        let cur = s.support_type;
        egui::ComboBox::from_label("Type").selected_text(match cur {
            SupportType::Normal => "Colonnes (classique)",
            SupportType::Tree => "Arbre (branches)",
        }).show_ui(ui, |ui| {
            for (v, name) in [(SupportType::Normal, "Colonnes (classique)"), (SupportType::Tree, "Arbre (branches)")] {
                if ui.selectable_label(cur == v, name).clicked() {
                    s.support_type = v;
                    *c.changed = true;
                }
            }
        });
        let cur = s.support_pattern;
        let label = |p: SupportPattern| match p {
            SupportPattern::Lines => "Lignes",
            SupportPattern::Grid => "Grille",
            SupportPattern::Concentric => "Concentrique",
        };
        egui::ComboBox::from_label("Motif").selected_text(label(cur)).show_ui(ui, |ui| {
            for p in [SupportPattern::Lines, SupportPattern::Grid, SupportPattern::Concentric] {
                if ui.selectable_label(cur == p, label(p)).clicked() {
                    s.support_pattern = p;
                    *c.changed = true;
                }
            }
        });
        num(ui, &mut c, "Angle de surplomb", &mut s.support_angle, 5.0..=85.0, 1.0, " °");
        num(ui, &mut c, "Densité", &mut s.support_density, 0.0..=100.0, 0.5, " %");
        num(ui, &mut c, "Distance XY au modèle", &mut s.support_xy_gap, 0.0..=2.0, 0.01, " mm");
        int(ui, &mut c, "Couches de séparation en Z", &mut s.support_z_gap_layers, 0..=5);
        int(ui, &mut c, "Couches d'interface (dessus)", &mut s.support_interface_layers, 0..=8);
        num(ui, &mut c, "Densité d'interface", &mut s.support_interface_density, 10.0..=100.0, 1.0, " %");
        int(ui, &mut c, "Couches pleines au départ (plancher)", &mut s.support_bottom_layers, 0..=8);
        num(ui, &mut c, "Densité du plancher", &mut s.support_bottom_density, 10.0..=100.0, 1.0, " %");
        num(ui, &mut c, "Largeur", &mut s.support_width, 0.2..=1.0, 0.01, " mm");
        num(ui, &mut c, "Expansion horizontale", &mut s.support_expansion, 0.0..=5.0, 0.05, " mm");
        num(ui, &mut c, "Surface minimale", &mut s.support_min_area, 0.0..=50.0, 0.1, " mm²");
        flag(ui, &mut c, "Seulement depuis le plateau", &mut s.support_from_bed_only);
        if s.support_type == SupportType::Tree {
            ui.separator();
            ui.label(RichText::new("Supports en arbre").strong());
            num(ui, &mut c, "Angle max. des branches", &mut s.tree_branch_angle, 5.0..=80.0, 1.0, " °");
            num(ui, &mut c, "Diamètre de la pointe", &mut s.tree_tip_diameter, 0.4..=5.0, 0.05, " mm");
            num(ui, &mut c, "Diamètre max. du tronc", &mut s.tree_max_diameter, 1.0..=30.0, 0.1, " mm");
            num(ui, &mut c, "Épaississement (rayon par mm)", &mut s.tree_growth, 0.0..=0.5, 0.005, "");
            num(ui, &mut c, "Écart entre pointes", &mut s.tree_tip_spacing, 0.8..=10.0, 0.1, " mm");
        }
        ui.separator();
        ui.label(RichText::new("Volumes imposés / bloqués").strong());
        ui.weak("Boîtes en mm, Z mesuré depuis le dessous du modèle.");
        volume_list(ui, &mut c, "Support imposé", &mut s.support_enforcers, anchor);
        volume_list(ui, &mut c, "Support bloqué", &mut s.support_blockers, anchor);
    });

    changed
}

fn role_color(r: u8) -> Color32 {
    match r {
        role::OUTER | role::FIRST_CONTOUR => Color32::from_rgb(255, 120, 80),
        role::INNER => Color32::from_rgb(240, 190, 70),
        role::INFILL => Color32::from_rgb(90, 170, 255),
        role::TOP => Color32::from_rgb(120, 230, 130),
        role::BOTTOM | role::FIRST_FILL => Color32::from_rgb(200, 120, 255),
        role::BRIDGE => Color32::from_rgb(255, 80, 200),
        role::SUPPORT_CONTOUR | role::SUPPORT_INFILL => Color32::from_rgb(90, 230, 230),
        role::INTERFACE_INFILL => Color32::WHITE,
        role::RAFT | role::RAFT_BOTTOM | role::RAFT_GRID | role::RAFT_INTERFACE => Color32::from_gray(150),
        role::BRIM => Color32::from_rgb(255, 160, 160),
        _ => Color32::from_gray(200),
    }
}

pub fn role_legend(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for (name, r) in [
            ("paroi externe", role::OUTER),
            ("parois internes", role::INNER),
            ("remplissage", role::INFILL),
            ("dessus", role::TOP),
            ("dessous", role::BOTTOM),
            ("pont", role::BRIDGE),
            ("support", role::SUPPORT_INFILL),
            ("interface", role::INTERFACE_INFILL),
            ("radeau", role::RAFT),
        ] {
            ui.label(RichText::new("■").color(role_color(r)));
            ui.label(name);
        }
    });
}

/// Draws one layer's toolpaths inside `rect` (top view of the bed).
pub fn draw_layer(painter: &egui::Painter, rect: egui::Rect, bed: [f64; 2], layer: &Layer, below: Option<&Layer>) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(22, 24, 29));
    let pad = 14.0;
    let scale = ((rect.width() - 2.0 * pad) as f64 / bed[0]).min((rect.height() - 2.0 * pad) as f64 / bed[1]);
    let origin = egui::pos2(rect.left() + pad, rect.bottom() - pad);
    let map = |x: i64, y: i64| egui::pos2(origin.x + (x as f64 / 1000.0 * scale) as f32, origin.y - (y as f64 / 1000.0 * scale) as f32);
    let bed_rect = egui::Rect::from_two_pos(map(0, 0), map((bed[0] * 1000.0) as i64, (bed[1] * 1000.0) as i64));
    painter.rect_stroke(bed_rect, 0.0, egui::Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
    if let Some(b) = below {
        for p in &b.paths {
            let pts: Vec<egui::Pos2> = p.pts.iter().map(|q| map(q.x, q.y)).collect();
            painter.add(egui::Shape::line(pts, egui::Stroke::new(0.6, Color32::from_gray(48))));
        }
    }
    for p in &layer.paths {
        let w = ((p.width * scale) as f32).clamp(0.8, 4.0);
        let pts: Vec<egui::Pos2> = p.pts.iter().map(|q| map(q.x, q.y)).collect();
        painter.add(egui::Shape::line(pts, egui::Stroke::new(w, role_color(p.role))));
    }
}

pub fn plan_summary(p: &Plan) -> String {
    format!(
        "{} couches ({} radeau + {} modèle), sommet du radeau à {:.2} mm",
        p.layers.len(),
        p.raft_layers,
        p.model_layers,
        p.raft_top as f64 / 1000.0
    )
}

fn volume_list(ui: &mut egui::Ui, c: &mut Ctx, title: &str, list: &mut Vec<Volume>, anchor: [f64; 2]) {
    let mut remove = None;
    for (i, v) in list.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("{title} {}", i + 1));
            if ui.small_button("✖").clicked() {
                remove = Some(i);
            }
        });
        for (axis, name) in ["X", "Y", "Z"].iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(*name);
                let mut ch = ui.add(egui::DragValue::new(&mut v.min[axis]).speed(0.5).prefix("min ")).changed();
                ch |= ui.add(egui::DragValue::new(&mut v.max[axis]).speed(0.5).prefix("max ")).changed();
                if v.max[axis] < v.min[axis] {
                    v.max[axis] = v.min[axis];
                }
                if ch {
                    *c.changed = true;
                }
            });
        }
    }
    if let Some(i) = remove {
        list.remove(i);
        *c.changed = true;
    }
    if ui.button(format!("+ {title}")).clicked() {
        list.push(Volume::around(anchor[0], anchor[1], 20.0));
        *c.changed = true;
    }
}

// ------------------------------------------------------------------------------------ simple mode

fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(14.0));
        ui.add_space(2.0);
        body(ui);
    });
    ui.add_space(4.0);
}

fn slider<N: egui::emath::Numeric>(ui: &mut egui::Ui, changed: &mut bool, v: &mut N, range: std::ops::RangeInclusive<N>, label: &str, suffix: &str, help: &str) {
    let r = ui.add(egui::Slider::new(v, range).text(label).suffix(suffix)).on_hover_text(help);
    if r.changed() {
        *changed = true;
    }
}

/// The handful of choices most people need. Everything else keeps its tested default.
pub fn simple_ui(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    let mut changed = false;

    card(ui, "Matériau", |ui| {
        let cur = s.material.name.clone();
        egui::ComboBox::from_id_salt("simple_material").selected_text(&cur).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
            for m in materials() {
                let label = format!("{}{}", m.name, if m.verified { "" } else { "  (températures typiques)" });
                if ui.selectable_label(m.name == cur, label).clicked() {
                    s.set_material(&m);
                    changed = true;
                }
            }
        });
        ui.label(format!("Buse {} °C  •  plateau {} °C", s.nozzle_temp, s.bed_temp));
        if !s.material.verified {
            ui.colored_label(Color32::from_rgb(240, 170, 60), "Températures non mesurées sur un fichier Z-Suite : vérifie-les (mode avancé).");
        }
    });

    card(ui, "Qualité", |ui| {
        ui.horizontal(|ui| {
            for (q, name, tip) in [
                (zslice::Quality::Draft, "Brouillon", "couches de 0,20 mm, rapide"),
                (zslice::Quality::Normal, "Normal", "couches de 0,15 mm"),
                (zslice::Quality::Fine, "Fin", "couches de 0,10 mm, lent"),
            ] {
                let active = s.quality() == q;
                if ui.add(egui::Button::new(name).selected(active)).on_hover_text(tip).clicked() {
                    s.apply_quality(q);
                    changed = true;
                }
            }
        });
        ui.weak(format!("Épaisseur de couche : {:.2} mm", s.layer_height));
    });

    card(ui, "Solidité", |ui| {
        let mut walls = s.wall_count;
        slider(ui, &mut changed, &mut walls, 1..=6, "Parois", "", "Nombre de parois autour de la pièce : plus = plus solide et plus lourd");
        s.wall_count = walls;
        slider(ui, &mut changed, &mut s.infill_density, 0.0..=100.0, "Remplissage", " %", "Densité du remplissage intérieur (20 % convient à la plupart des pièces)");
        let mut top = s.top_layers;
        slider(ui, &mut changed, &mut top, 1..=8, "Couches dessus", "", "Couches pleines sous la surface du dessus");
        s.top_layers = top;
        let mut bottom = s.bottom_layers;
        slider(ui, &mut changed, &mut bottom, 1..=8, "Couches dessous", "", "Couches pleines au-dessus du plateau ou d'un support");
        s.bottom_layers = bottom;
    });

    card(ui, "Supports", |ui| {
        #[derive(PartialEq, Clone, Copy)]
        enum K {
            None,
            Columns,
            Tree,
        }
        let cur = if !s.support { K::None } else if s.support_type == SupportType::Tree { K::Tree } else { K::Columns };
        let mut pick = cur;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut pick, K::None, "Aucun");
            ui.selectable_value(&mut pick, K::Columns, "Colonnes").on_hover_text("Piliers verticaux sous chaque surplomb : simple et solide");
            ui.selectable_value(&mut pick, K::Tree, "Arbre").on_hover_text("Branches fines qui rejoignent le plateau : moins de matière, plus faciles à retirer");
        });
        if pick != cur {
            changed = true;
            s.support = pick != K::None;
            if pick == K::Tree {
                s.support_type = SupportType::Tree;
            } else if pick == K::Columns {
                s.support_type = SupportType::Normal;
            }
        }
        if s.support {
            slider(ui, &mut changed, &mut s.support_angle, 15.0..=70.0, "Sensibilité", " °", "Angle des surplombs à soutenir : plus il est grand, plus il y a de supports (30° est la valeur de Z-Suite)");
            slider(ui, &mut changed, &mut s.support_density, 5.0..=40.0, "Densité", " %", "Densité du remplissage des supports");
            if ui.checkbox(&mut s.support_from_bed_only, "Seulement depuis le plateau").on_hover_text("Pas de support posé sur la pièce elle-même").changed() {
                changed = true;
            }
        }
    });

    card(ui, "Adhérence au plateau", |ui| {
        #[derive(PartialEq, Clone, Copy)]
        enum A {
            None,
            Brim,
            Raft,
        }
        let cur = if s.raft { A::Raft } else if s.brim_width > 0.0 { A::Brim } else { A::None };
        let mut pick = cur;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut pick, A::None, "Aucune");
            ui.selectable_value(&mut pick, A::Brim, "Bord").on_hover_text("Une collerette autour de la première couche");
            ui.selectable_value(&mut pick, A::Raft, "Radeau").on_hover_text("Plusieurs couches sous la pièce : adhérence maximale (réglage de Z-Suite)");
        });
        if pick != cur {
            changed = true;
            s.raft = pick == A::Raft;
            s.brim_width = if pick == A::Brim { 5.0 } else { 0.0 };
        }
        match pick {
            A::Brim => slider(ui, &mut changed, &mut s.brim_width, 1.0..=12.0, "Largeur du bord", " mm", "Largeur de la collerette"),
            A::Raft => {
                let mut layers = s.raft_layers;
                slider(ui, &mut changed, &mut layers, 2..=8, "Couches du radeau", "", "5 est la valeur de Z-Suite");
                s.raft_layers = layers;
                slider(ui, &mut changed, &mut s.raft_margin, 1.0..=10.0, "Marge du radeau", " mm", "Dépassement du radeau autour de la pièce (réduit automatiquement près du bord du plateau)");
            }
            A::None => {}
        }
    });

    card(ui, "Refroidissement", |ui| {
        let mut fan = s.fan_percent as u32;
        slider(ui, &mut changed, &mut fan, 0..=100, "Ventilateur", " %", "Vitesse du ventilateur après les premières couches");
        s.fan_percent = fan as u8;
    });

    changed
}

pub use zproj_studio::overlay::plan_wires;
