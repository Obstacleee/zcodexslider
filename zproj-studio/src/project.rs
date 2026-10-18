//! Builds a Z-Suite project (`.zprojx`) from models, using an existing project as template
//! for everything we do not generate (printer, print settings, resin settings, ...).

use crate::mesh::Item;
use zcodex2::nrbf::write_byte_array;
use zcodex2::Store;

/// Project written by Z-Suite 3.7 for an M200 Plus; its settings are reused by default.
pub const BUNDLED_TEMPLATE: &[u8] = include_bytes!("../template/template.zprojx");

pub fn bundled_template() -> Store {
    Store::from_gzip(BUNDLED_TEMPLATE).expect("bundled template is valid")
}

/// Template with the models and the preview replaced. The previous print file (`ZCode*`) is cleared
/// because it belonged to the template's model.
pub fn build(template: &Store, items: &[Item], preview_png: &[u8]) -> Store {
    let mut s = template.clone();
    let models: Vec<_> = items.iter().map(Item::to_model).collect();
    s.set_models(&models);
    s.set("ZCodePreview", write_byte_array(preview_png));
    s.set("ZCode", write_byte_array(&[]));
    s.set("ZCodeOriginal", write_byte_array(&[]));
    s
}

/// Keys a Z-Suite project must contain (all present in a real 3.7 project).
const REQUIRED_KEYS: [&str; 10] = [
    "ProjectVersion", "Printer.Model", "PrinterId", "CurrentProjectState", "Models", "Settings", "ResinSettings", "ZCode", "ZCodeOriginal",
    "ZCodePreview",
];

/// Everything that would make Z-Suite refuse or misread a project. Empty = sound.
pub fn verify_project(store: &Store) -> Vec<String> {
    let mut p = Vec::new();
    for k in REQUIRED_KEYS {
        if store.get(k).is_none() {
            p.push(format!("clé « {k} » absente"));
        }
    }
    // envelope: gzip + BinaryFormatter dictionary must survive a write/read round trip unchanged
    match store.to_gzip().and_then(|g| Store::from_gzip(&g)) {
        Ok(back) if back.entries == store.entries => {}
        Ok(_) => p.push("le conteneur ne se relit pas à l'identique".into()),
        Err(e) => p.push(format!("conteneur illisible après écriture : {e}")),
    }
    // nested values
    match store.value("ProjectVersion") {
        Ok(Some(v)) => {
            let ok = v.as_str().map_or(false, |t| t.split('.').count() == 4 && t.split('.').all(|x| x.parse::<u32>().is_ok()));
            if !ok {
                p.push("ProjectVersion doit être de la forme 3.7.0.0".into());
            }
        }
        _ => p.push("ProjectVersion illisible".into()),
    }
    for (k, class) in [("Settings", "PrintSettingsToJson"), ("ResinSettings", "ResinPrintSettingsToJson")] {
        match store.value(k) {
            Ok(Some(v)) if matches!(&v, zcodex2::nrbf::Value::Object { class: c, .. } if c.ends_with(class)) => {}
            _ => p.push(format!("« {k} » n'est pas un objet {class}")),
        }
    }
    match store.value("ZCodePreview") {
        Ok(Some(v)) if v.as_bytes().map_or(false, |b| b.starts_with(&[0x89, b'P', b'N', b'G'])) => {}
        _ => p.push("ZCodePreview n'est pas un PNG".into()),
    }
    // models: geometry sane, protobuf identical to what protobuf-net writes
    match (store.get("Models"), store.models()) {
        (Some(raw), Ok(models)) => {
            if models.is_empty() {
                p.push("le projet ne contient aucun modèle".into());
            }
            if zcodex2::encode_models(&models) != raw {
                p.push("Models ne se réécrit pas à l'identique (format protobuf-net)".into());
            }
            for (i, m) in models.iter().enumerate() {
                let n = m.vertices.len() as i32;
                if m.vertices.iter().flatten().any(|v| !v.is_finite()) {
                    p.push(format!("modèle {i} : coordonnées non finies"));
                }
                if m.triangles.iter().flatten().any(|&k| k < 0 || k >= n) {
                    p.push(format!("modèle {i} : indice de triangle hors des sommets"));
                }
                if m.triangles.is_empty() || m.name.is_empty() {
                    p.push(format!("modèle {i} : sans triangles ou sans nom"));
                }
                if m.id == [0; 16] {
                    p.push(format!("modèle {i} : identifiant nul"));
                }
            }
        }
        (None, _) => {}
        (_, Err(e)) => p.push(format!("Models illisible : {e}")),
    }
    p
}

/// Like [`build`] but refuses to return a project that fails [`verify_project`].
pub fn build_checked(template: &Store, items: &[Item], preview_png: &[u8]) -> Result<Store, String> {
    let s = build(template, items, preview_png);
    let problems = verify_project(&s);
    if problems.is_empty() {
        Ok(s)
    } else {
        Err(format!("projet invalide, il n'est pas écrit :\n - {}", problems.join("\n - ")))
    }
}

/// Width and height of the PNG preview stored in a project, if any.
pub fn preview_size(store: &Store) -> Option<(u32, u32)> {
    let v = store.value("ZCodePreview").ok()??;
    let b = v.as_bytes()?;
    if b.len() < 24 || !b.starts_with(b"\x89PNG") {
        return None;
    }
    Some((u32::from_be_bytes(b[16..20].try_into().ok()?), u32::from_be_bytes(b[20..24].try_into().ok()?)))
}

pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().expect("png header");
        wr.write_image_data(rgba).expect("png data");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stl::cube;

    #[test]
    fn bundled_template_is_readable() {
        let t = bundled_template();
        assert_eq!(t.value("Printer.Model").unwrap().unwrap().as_str(), Some("M200 Plus"));
        assert!(preview_size(&t).is_some());
    }

    #[test]
    fn build_replaces_models_and_preview() {
        let t = bundled_template();
        let item = Item::new("cube.stl", cube(20.0), [100.0, 100.0]);
        let png = encode_png(&[255u8; 4 * 4 * 4], 4, 4);
        let s = build(&t, &[item.clone()], &png);

        let back = Store::from_gzip(&s.to_gzip().unwrap()).unwrap();
        let models = back.models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "cube.stl");
        assert_eq!(models[0].vertices.len(), 8);
        assert_eq!(models[0].translation, [100.0, 100.0, 10.0]);
        assert_eq!(preview_size(&back), Some((4, 4)));
        // everything else is carried over from the template
        for k in ["Settings", "ResinSettings", "PrinterId", "Printer.Model", "ProjectVersion"] {
            assert_eq!(back.get(k), t.get(k), "{k}");
        }
    }

    /// Loading the real template's model and rebuilding it must give back the same geometry.
    #[test]
    fn template_model_survives_load_and_rebuild() {
        let t = bundled_template();
        let orig = t.models().unwrap();
        let items: Vec<Item> = orig.iter().map(Item::from_model).collect();
        let rebuilt = build(&t, &items, b"\x89PNG\r\n\x1a\n").models().unwrap();
        assert_eq!(rebuilt.len(), 1);
        let (a, b) = (&orig[0], &rebuilt[0]);
        assert_eq!(a.triangles, b.triangles);
        assert_eq!(a.id, b.id);
        for (p, q) in a.vertices.iter().zip(&b.vertices) {
            for i in 0..3 {
                assert!((p[i] - q[i]).abs() < 1e-9, "{p:?} vs {q:?}");
            }
        }
        for i in 0..3 {
            assert!((a.translation[i] - b.translation[i]).abs() < 1e-9);
        }
    }

    #[test]
    fn bundled_template_and_built_projects_pass_verification() {
        let t = bundled_template();
        assert!(verify_project(&t).is_empty(), "{:?}", verify_project(&t));
        let items = vec![Item::new("a.stl", cube(20.0), [60.0, 60.0]), Item::new("b.stl", cube(10.0), [140.0, 100.0])];
        let png = encode_png(&[255u8; 4 * 4 * 4], 4, 4);
        let s = build_checked(&t, &items, &png).unwrap();
        assert!(verify_project(&s).is_empty());
    }

    #[test]
    fn a_real_z_suite_project_passes_verification() {
        let Ok(real) = Store::open(r"C:\Users\Lucas Delon\Downloads\sentinel 2.zprojx") else { return };
        let p = verify_project(&real);
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn broken_projects_are_refused() {
        let t = bundled_template();
        let png = encode_png(&[255u8; 4 * 4 * 4], 4, 4);
        // no models
        assert!(build_checked(&t, &[], &png).is_err());
        // a key removed
        let mut s = t.clone();
        s.entries.retain(|(k, _)| k != "ResinSettings");
        assert!(verify_project(&s).iter().any(|m| m.contains("ResinSettings")));
        // preview that is not a PNG
        let item = Item::new("a.stl", cube(20.0), [100.0, 100.0]);
        assert!(build_checked(&t, &[item.clone()], b"not a png").is_err());
        // triangle index out of range
        let mut s = build(&t, &[item], &png);
        let mut models = s.models().unwrap();
        models[0].triangles[0][0] = 9999;
        s.set_models(&models);
        assert!(verify_project(&s).iter().any(|m| m.contains("indice")));
    }
}
