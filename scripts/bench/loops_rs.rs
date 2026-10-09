// bench: loops_rs — native Rust floor for the loops workload (R0.8, L-039).
// Mirrors scripts/bench/loops.op / native_loops (bench_compare.py) exactly:
// acc = acc + i % 7 over the work loop. Prints the same line the .op
// promotes so bars.py can differential-verify before recording a timing.
// `rustc -O loops_rs.rs -o loops_rs && ./loops_rs [n]` (default n = 200000)
use std::env;

fn main() {
    let n: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200000);
    let mut acc: i64 = 0;
    for i in 0..n {
        acc += (i % 7) as i64;
    }
    println!("acc = {acc}");
}
