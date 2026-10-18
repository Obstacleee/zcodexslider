use zcodex2::Store;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file.zprojx|.zprof>");
    let s = Store::open(&path).expect("open");
    println!("version {} hash {}", s.version, s.hash_size);
    for (k, v) in &s.entries {
        let head: String = v.iter().take(24).map(|b| format!("{b:02x}")).collect();
        println!("{k:34} {:>8} bytes  {head}", v.len());
    }
}
