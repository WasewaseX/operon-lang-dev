// bench: strings_rs — native Rust floor for the strings workload (R0.8, L-039).
// Mirrors scripts/bench/strings.op / native_strings (bench_compare.py)
// exactly, INCLUDING the quadratic `s = s + frag` idiom (a new String is
// allocated every iteration — push_str would be a different algorithm).
// Prints the same line the .op promotes.
// `rustc -O strings_rs.rs -o strings_rs && ./strings_rs [n]` (default 4000)
use std::env;

fn main() {
    let n: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4000);
    let mut s = String::new();
    for i in 0..n {
        let frag = format!("x{}y", i % 10);
        s = s + &frag;
    }
    println!("len = {}", s.len());
}
