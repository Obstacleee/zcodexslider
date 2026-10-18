//! view_png <t|arch|ball|edge> <out.png> [tree|columns] [raft]
//! Reproduces the app's 3D view headlessly: model (red where it leaves the plate), raft, brim and supports.
use zproj_studio::mesh::Item;
use zproj_studio::overlay::plan_wires;
use zproj_studio::render::{render_with, Camera, Drawable};
use zproj_studio::stl::Mesh;
use zslice::{Settings, SupportType};

fn boxes(list: &[[f64; 6]]) -> Mesh {
    let (mut v, mut t) = (Vec::new(), Vec::new());
    for b in list {
        let base = v.len() as i32;
        let [x0, y0, z0, x1, y1, z1] = *b;
        v.extend([[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]);
        for k in [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]] {
            t.push([k[0] + base, k[1] + base, k[2] + base]);
        }
    }
    Mesh { vertices: v, triangles: t }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (mesh, pos) = match a[1].as_str() {
        "arch" => (boxes(&[[0.0, 0.0, 0.0, 10.0, 20.0, 15.0], [50.0, 0.0, 0.0, 60.0, 20.0, 15.0], [0.0, 0.0, 15.0, 60.0, 20.0, 19.0]]), [100.0, 100.0]),
        // pushed against the left edge: raft and part leave the plate
        "edge" => (boxes(&[[0.0, 0.0, 0.0, 14.0, 14.0, 14.0], [0.0, 0.0, 14.0, 50.0, 20.0, 18.0]]), [18.0, 100.0]),
        // too big for the plate
        "big" => (boxes(&[[0.0, 0.0, 0.0, 230.0, 60.0, 20.0]]), [100.0, 100.0]),
        _ => (boxes(&[[0.0, 0.0, 0.0, 10.0, 10.0, 20.0], [-25.0, -10.0, 20.0, 35.0, 20.0, 26.0]]), [100.0, 100.0]),
    };
    let item = Item::new("piece.stl", mesh.clone(), pos);
    let mut s = Settings::default();
    s.raft = a.iter().any(|x| x == "raft");
    s.support = true;
    s.support_type = if a.iter().any(|x| x == "columns") { SupportType::Normal } else { SupportType::Tree };

    let bed = [200.0, 200.0];
    let m = zslice::Mesh { vertices: item.placed().to_vec(), triangles: item.mesh.triangles.clone() };
    let plan = zslice::toolpath::plan(&m, &s);
    let (wires_s, wires_b, lift) = match &plan {
        Ok(p) => (plan_wires(p, bed, true, false), plan_wires(p, bed, false, true), p.raft_top as f64 / 1000.0),
        Err(e) => {
            eprintln!("plan: {e}");
            (Vec::new(), Vec::new(), 0.0)
        }
    };
    let verts: Vec<[f64; 3]> = item.placed().iter().map(|v| [v[0], v[1], v[2] + lift]).collect();
    let d = [Drawable { vertices: &verts, triangles: &item.mesh.triangles, color: [120, 180, 255], limit: Some([200.0, 200.0]) }];
    let mut wires = wires_s;
    wires.extend(wires_b);
    let mut cam = Camera::for_bed(bed);
    cam.frame([pos[0] - 40.0, pos[1] - 30.0, 0.0], [pos[0] + 40.0, pos[1] + 30.0, 30.0]);
    cam.zoom = 0.8;
    let rgba = render_with(&d, &wires, [200.0, 200.0], &cam, 900, 620);
    std::fs::write(&a[2], zproj_studio::project::encode_png(&rgba, 900, 620)).unwrap();
    println!("wires: {}", wires.len());
}
