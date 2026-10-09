// bench: collections_rs — native Rust floor for the collections workload
// (R0.8, L-039). Mirrors scripts/bench/collections.op / native_collections
// (bench_compare.py) exactly: insert-or-increment churn on a 2000-key
// space, list push, full list sum, full map-key distinct pass, one keyed
// read. Prints the same line the .op promotes (k1 is a documented constant
// of the shape: with n iterations over i % 2000, key "k1" is hit n/2000
// times — the base fixture's k1 = 10).
// `rustc -O collections_rs.rs -o collections_rs && ./collections_rs [n]`
use std::collections::HashMap;
use std::env;

fn main() {
    let n: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20000);
    let mut m: HashMap<String, i64> = HashMap::new();
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..n {
        let k = format!("k{}", i % 2000);
        if m.contains_key(&k) {
            *m.get_mut(&k).unwrap() += 1;
        } else {
            m.insert(k, 1);
        }
        xs.push((i % 97) as i64);
    }
    let total: i64 = xs.iter().sum();
    let distinct = m.len();
    let k1 = m["k1"];
    println!("total = {total}, distinct = {distinct}, k1 = {k1}");
}
