// bench: recursion_rs — native Rust floor for the recursion workload
// (R0.8, L-039). Mirrors scripts/bench/recursion.op / native_recursion
// (bench_compare.py) exactly: naive wide-tree binomial C(20,10).
// Prints the same line the .op promotes.
// `rustc -O recursion_rs.rs -o recursion_rs && ./recursion_rs [depth]`
// (default depth = 20; k = depth / 2 — the scaling table marks this
// workload n/a because the work is C(2n, n)-exponential, not linear)
use std::env;

#[inline(never)]
fn pasc(n: u64, k: u64) -> u64 {
    if k == 0 || k == n {
        return 1;
    }
    pasc(n - 1, k - 1) + pasc(n - 1, k)
}

fn main() {
    let d: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let v = pasc(d, d / 2);
    println!("C({d},{}) = {v}", d / 2);
}
