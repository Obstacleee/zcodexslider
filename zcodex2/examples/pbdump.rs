use zcodex2::Store;
fn main() {
    let s = Store::open(std::env::args().nth(1).unwrap()).unwrap();
    let b = s.get("Models").unwrap();
    let hex = |x: &[u8]| x.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
    println!("head: {}", hex(&b[..140]));
    println!("tail: {}", hex(&b[b.len() - 120..]));
}
