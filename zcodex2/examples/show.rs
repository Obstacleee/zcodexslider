use zcodex2::Store;

fn main() {
    let path = std::env::args().nth(1).expect("usage: show <file>");
    let s = Store::open(&path).expect("open");
    for k in ["ProjectVersion", "Printer.Model", "PrinterId", "CurrentProjectState"] {
        println!("{k} = {:?}", s.value(k).map(|v| v));
    }
    match s.models() {
        Ok(ms) => {
            println!("models: {}", ms.len());
            for m in &ms {
                let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
                for v in &m.vertices {
                    for i in 0..3 {
                        lo[i] = lo[i].min(v[i]);
                        hi[i] = hi[i].max(v[i]);
                    }
                }
                let maxidx = m.triangles.iter().flatten().max();
                println!(
                    "  {:?}: {} vertices, {} triangles, translation {:?}, unrepaired {}, bbox {:?}..{:?}, max index {:?}",
                    m.name, m.vertices.len(), m.triangles.len(), m.translation, m.unrepaired, lo, hi, maxidx
                );
            }
        }
        Err(e) => println!("models error: {e}"),
    }
    for k in ["Settings", "ResinSettings"] {
        println!("--- {k}\n{:#?}", s.value(k));
    }
}
