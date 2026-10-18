//! No printed support may float in mid-air: every piece must rest on the plate, on support below it or on the model.
use zslice::slicer::slice;
use zslice::support::{floating_area, plan_supports, In};
use zslice::{Mesh, Settings, SupportType};

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

#[test]
fn supports_never_float() {
    let shapes = [
        boxes(&[[95.0, 95.0, 0.0, 105.0, 105.0, 20.0], [60.0, 85.0, 20.0, 140.0, 115.0, 26.0]]),
        boxes(&[[70.0, 90.0, 0.0, 80.0, 110.0, 25.0], [120.0, 90.0, 0.0, 130.0, 110.0, 25.0], [70.0, 90.0, 25.0, 130.0, 110.0, 31.0]]),
    ];
    for mesh in &shapes {
        for kind in [SupportType::Normal, SupportType::Tree] {
            let mut s = Settings::default();
            s.support = true;
            s.support_type = kind;
            let (lo, hi) = mesh.bounds();
            let n = ((hi[2] - lo[2] - 0.26) / 0.15).round() as usize + 1;
            let planes: Vec<f64> = (0..n).map(|i| if i == 0 { 0.13 } else { 0.26 + (i as f64 - 0.5) * 0.15 }).collect();
            let regions = slice(mesh, &planes);
            let layers: Vec<In> = (0..n).map(|i| In { region: &regions[i], thickness: if i == 0 { 0.26 } else { 0.15 }, zc: planes[i] }).collect();
            let sup = plan_supports(&layers, &s);
            let areas: Vec<_> = sup.iter().map(|l| l.area.clone()).collect();
            assert!(sup.iter().any(|l| !l.area.is_empty()), "{kind:?}: no support generated at all");
            let f = floating_area(&layers, &areas, &s);
            assert!(f < 1.0e5, "{kind:?}: support floating in mid-air: {f} um2");
        }
    }
}
