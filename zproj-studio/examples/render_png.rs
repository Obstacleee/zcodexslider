//! Renders a project's models to a PNG: render_png <project.zprojx|.stl> <out.png>
use zproj_studio::mesh::Item;
use zproj_studio::render::{render, Camera, Drawable};
use zproj_studio::{project, stl};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let items: Vec<Item> = if args[1].to_lowercase().ends_with(".stl") {
        let imp = stl::parse(&std::fs::read(&args[1]).unwrap()).unwrap();
        vec![Item::new("m.stl", imp.mesh, [100.0, 100.0])]
    } else {
        zcodex2::Store::open(&args[1]).unwrap().models().unwrap().iter().map(Item::from_model).collect()
    };
    let mut cam = Camera::for_bed([200.0, 200.0]);
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for it in &items {
        let (l, h) = it.bounds();
        for i in 0..3 {
            lo[i] = lo[i].min(l[i]);
            hi[i] = hi[i].max(h[i]);
        }
    }
    cam.frame(lo, hi);
    let d: Vec<Drawable> = items.iter().map(|i| Drawable { vertices: i.placed(), triangles: &i.mesh.triangles, color: [150, 190, 255], limit: None }).collect();
    let t = std::time::Instant::now();
    let rgba = render(&d, [200.0, 200.0], &cam, 800, 560);
    eprintln!("render {:?}", t.elapsed());
    std::fs::write(&args[2], project::encode_png(&rgba, 800, 560)).unwrap();
}
