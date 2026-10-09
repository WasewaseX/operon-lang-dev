// bench: large_map_rs — native Rust floor for the large-map workload
// (R0.8, L-039). Mirrors scripts/bench/large_map.op / native_large_map
// (bench_compare.py) exactly: build n "key-i" entries, then a FULL second
// loop of lookups (both loops scale — the workload is the build+lookup
// pair). Prints the same line the .op promotes.
// `rustc -O large_map_rs.rs -o large_map_rs && ./large_map_rs [n]`
use std::collections::HashMap;
use std::env;

fn main() {
    let n: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6000);
    let mut m: HashMap<String, i64> = HashMap::with_capacity(n as usize);
    for i in 0..n {
        m.insert(format!("key-{i}"), (i * 3) as i64);
    }
    let mut acc: i64 = 0;
    for i in 0..n {
        acc += m[&format!("key-{i}")];
    }
    println!("map sum = {acc}");
}
