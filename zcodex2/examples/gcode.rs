//! Pretty-prints the ZCodeData stream of a .zcodex2 as G-code-like text.
use zcodex2::{Command, Zcodex2};

fn main() {
    let f = Zcodex2::open(std::env::args().nth(1).expect("usage: gcode <file.zcodex2>")).unwrap();
    println!("{}", serde_json::to_string_pretty(&f.metadata).unwrap());
    let limit: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    for (i, r) in f.records().unwrap().iter().enumerate().take(limit) {
        println!("{i:6} {:?}", r.decode());
    }
}
