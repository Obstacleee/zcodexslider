//! Headless conversion: stl2zprojx <in.stl> <out.zprojx> [template.zprojx]
use zproj_studio::mesh::Item;
use zproj_studio::{project, render, stl};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: stl2zprojx <in.stl> <out.zprojx> [template.zprojx]");
        std::process::exit(2);
    }
    let imp = stl::parse(&std::fs::read(&a[1]).expect("read stl")).expect("parse stl");
    let name = std::path::Path::new(&a[1]).file_name().unwrap().to_string_lossy().into_owned();
    let item = Item::new(name, imp.mesh, [100.0, 100.0]);
    let template = match a.get(3) {
        Some(p) => zcodex2::Store::open(p).expect("template"),
        None => project::bundled_template(),
    };
    let (w, h) = project::preview_size(&template).unwrap_or((256, 256));
    let mut cam = render::Camera::for_bed([200.0, 200.0]);
    let (lo, hi) = item.bounds();
    cam.frame(lo, hi);
    let d = [render::Drawable { vertices: item.placed(), triangles: &item.mesh.triangles, color: [205, 210, 220], limit: None }];
    let rgba = render::render(&d, [200.0, 200.0], &cam, w as usize, h as usize);
    let store = project::build(&template, &[item], &project::encode_png(&rgba, w, h));
    store.save(&a[2]).expect("write");
    println!("wrote {}", a[2]);
}
