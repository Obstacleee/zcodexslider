//! ztool - command line helper for Zortrax M200 Plus workflows.
//!
//!   ztool to-zcodex2 <input.gcode> [options]     convert G-code from any slicer to a .zcodex2 print file
//!   ztool to-gcode   <input.zcodex2> [-o out.gcode]  convert a Z-Suite print file back to G-code
//!   ztool info       <input.zcodex2>              print the file's metadata
//!   ztool check      <input.zcodex2>              check that Z-Suite 3.7 should be able to open the file
//!
//! As a post-processing script (PrusaSlicer, SuperSlicer, OrcaSlicer, Slic3r) the slicer appends the
//! temporary G-code path; the output name then comes from SLIC3R_PP_OUTPUT_NAME.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use zslice::{gcode_to_zcodex2, materials, records_to_gcode, Settings};

const USAGE: &str = "\
ztool - outils Zortrax M200 Plus

  ztool to-zcodex2 <fichier.gcode> [options]
      -o <sortie.zcodex2>        fichier de sortie (sinon : SLIC3R_PP_OUTPUT_NAME, sinon à côté de l'entrée)
      --material <nom>           matériau (Z-PLA, Z-ABS, Z-PETG... ; défaut Z-PLA)
      --nozzle-temp <°C>         température de buse si le G-code n'en contient pas
      --bed-temp <°C>            température de plateau si le G-code n'en contient pas
      --bed <LxP>                taille du plateau en mm (défaut 200x200)
      --max-z <mm>               hauteur maximale (défaut 180)
      --nozzle <mm>              diamètre de buse (défaut 0.4)
      --retraction <mm>          distance utilisée pour G10/G11 (défaut 0.8)
      --config-from <f.zcodex2>  copie ConfigurationData d'un fichier Z-Suite
      --quiet                    n'affiche que les erreurs
  ztool to-gcode <fichier.zcodex2> [-o sortie.gcode]
  ztool info <fichier.zcodex2>
  ztool check <fichier.zcodex2>
      vérifie que Z-Suite 3.7 devrait pouvoir ouvrir le fichier (entrées, champs, valeurs)
";

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("Erreur : {msg}");
    ExitCode::from(1)
}

fn human_time(s: f64) -> String {
    zslice::human_time(s)
}

fn default_output(input: &Path, ext: &str) -> PathBuf {
    if let Some(name) = std::env::var_os("SLIC3R_PP_OUTPUT_NAME") {
        let mut p = PathBuf::from(name);
        if p.extension().is_some() {
            p.set_extension(ext);
        } else {
            p.set_extension(ext);
        }
        return p;
    }
    input.with_extension(ext)
}

fn to_zcodex2(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut s = Settings::default();
    let mut config_from: Option<PathBuf> = None;
    let mut quiet = false;
    let mut it = args.iter();
    macro_rules! val {
        ($flag:expr) => {
            match it.next() {
                Some(v) => v,
                None => return fail(format!("{} attend une valeur", $flag)),
            }
        };
    }
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => out = Some(PathBuf::from(val!("-o"))),
            "--material" => {
                let want = val!("--material").to_lowercase();
                match materials().into_iter().find(|m| m.name.to_lowercase() == want || m.name.to_lowercase().replace("z-", "") == want) {
                    Some(m) => s.set_material(&m),
                    None => return fail(format!("matériau inconnu « {want} » (Z-PLA, Z-ABS, Z-PETG...)")),
                }
            }
            "--nozzle-temp" => match val!("--nozzle-temp").parse() {
                Ok(v) => s.nozzle_temp = v,
                Err(_) => return fail("--nozzle-temp : nombre attendu"),
            },
            "--bed-temp" => match val!("--bed-temp").parse() {
                Ok(v) => s.bed_temp = v,
                Err(_) => return fail("--bed-temp : nombre attendu"),
            },
            "--bed" => {
                let v = val!("--bed").to_lowercase();
                let parts: Vec<f64> = v.split('x').filter_map(|p| p.parse().ok()).collect();
                if parts.len() != 2 {
                    return fail("--bed attend LxP, par exemple 200x200");
                }
                s.bed = [parts[0], parts[1]];
            }
            "--max-z" => match val!("--max-z").parse() {
                Ok(v) => s.max_z = v,
                Err(_) => return fail("--max-z : nombre attendu"),
            },
            "--nozzle" => match val!("--nozzle").parse() {
                Ok(v) => s.nozzle_diameter = v,
                Err(_) => return fail("--nozzle : nombre attendu"),
            },
            "--retraction" => match val!("--retraction").parse() {
                Ok(v) => s.retraction_distance = v,
                Err(_) => return fail("--retraction : nombre attendu"),
            },
            "--config-from" => config_from = Some(PathBuf::from(val!("--config-from"))),
            "--quiet" | "-q" => quiet = true,
            other if other.starts_with('-') => return fail(format!("option inconnue {other}\n\n{USAGE}")),
            other => input = Some(PathBuf::from(other)),
        }
    }
    let Some(input) = input else { return fail(format!("aucun fichier G-code\n\n{USAGE}")) };
    let out = out.unwrap_or_else(|| default_output(&input, "zcodex2"));

    let text = match std::fs::read(&input) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return fail(format!("lecture de {} impossible : {e}", input.display())),
    };
    let config = match &config_from {
        Some(p) => match zcodex2::Zcodex2::open(p) {
            Ok(f) => match f.entry(zcodex2::CONFIGURATION_DATA) {
                Some(c) => Some(c.to_vec()),
                None => return fail(format!("{} ne contient pas ConfigurationData", p.display())),
            },
            Err(e) => return fail(format!("{} illisible : {e}", p.display())),
        },
        None => None,
    };
    let r = match gcode_to_zcodex2(&text, &s, Vec::new(), config) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    if let Err(e) = r.save(&out) {
        return fail(format!("écriture de {} impossible : {e}", out.display()));
    }
    for w in &r.warnings {
        eprintln!("Avertissement : {w}");
    }
    if !quiet {
        let st = &r.stats;
        println!(
            "{} : {} couches, {} commandes, durée estimée {}, matière {:.1} g ({:.1} m)",
            out.display(),
            st.layers,
            st.commands,
            human_time(st.print_time_s),
            st.filament_g,
            st.filament_mm / 1000.0
        );
    }
    ExitCode::SUCCESS
}

fn to_gcode(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--output" => out = it.next().map(PathBuf::from),
            other => input = Some(PathBuf::from(other)),
        }
    }
    let Some(input) = input else { return fail(format!("aucun fichier .zcodex2\n\n{USAGE}")) };
    let f = match zcodex2::Zcodex2::open(&input) {
        Ok(f) => f,
        Err(e) => return fail(format!("{} illisible : {e}", input.display())),
    };
    let recs = match f.records() {
        Ok(r) => r,
        Err(e) => return fail(format!("ZCodeData illisible : {e}")),
    };
    let out = out.unwrap_or_else(|| input.with_extension("gcode"));
    if let Err(e) = std::fs::write(&out, records_to_gcode(&recs, "ztool")) {
        return fail(format!("écriture impossible : {e}"));
    }
    println!("{} : {} commandes converties", out.display(), recs.len());
    ExitCode::SUCCESS
}

fn info(args: &[String]) -> ExitCode {
    let Some(p) = args.first() else { return fail(format!("aucun fichier .zcodex2\n\n{USAGE}")) };
    match zcodex2::Zcodex2::open(p) {
        Ok(f) => {
            println!("{}", serde_json::to_string_pretty(&f.metadata).unwrap_or_default());
            println!("entrées : {}", f.entries.iter().map(|(n, b)| format!("{n} ({} o)", b.len())).collect::<Vec<_>>().join(", "));
            ExitCode::SUCCESS
        }
        Err(e) => fail(format!("{p} illisible : {e}")),
    }
}

fn check(args: &[String]) -> ExitCode {
    let Some(p) = args.first() else { return fail(format!("aucun fichier .zcodex2

{USAGE}")) };
    let f = match zcodex2::Zcodex2::open(p) {
        Ok(f) => f,
        Err(e) => return fail(format!("{p} illisible : {e}")),
    };
    let problems = zslice::verify_file(&f, &Settings::default());
    if problems.is_empty() {
        println!("{p} : fichier sain (conteneur, format Z-Suite 3.7 et flux de commandes vérifiés)");
        ExitCode::SUCCESS
    } else {
        eprintln!("{p} : {} problème(s) à corriger avant de l'ouvrir dans Z-Suite ou d'imprimer :", problems.len());
        for m in &problems {
            eprintln!("  - {m}");
        }
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("to-zcodex2") => to_zcodex2(&args[1..]),
        Some("to-gcode") => to_gcode(&args[1..]),
        Some("info") => info(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("-h") | Some("--help") | Some("help") | None => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(_) => fail(format!("commande inconnue\n\n{USAGE}")),
    }
}
