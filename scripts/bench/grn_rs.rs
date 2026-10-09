// bench: grn_rs — native Rust floor for the grn workload (R0.8, L-039).
// Mirrors scripts/bench/grn.op / native_grn (bench_compare.py) exactly:
// the regulation call gate's HAPPY PATH — every call checks its source
// level against the edge threshold (worker_a's threshold carries the
// ENHANCE_DELTA 0.25 reduction, 0.3 - 0.25 = 0.05), grn_fire lifts the
// driver level to 1.0, then the main loop calls all three regulated genes.
// Prints the same line the .op promotes.
// `rustc -O grn_rs.rs -o grn_rs && ./grn_rs [n]` (default n = 20000)
use std::env;

fn main() {
    let n: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20000);
    let mut level: f64 = 0.0;
    // the regulated genes, each with the gate the .op machinery imposes
    fn worker_a(n: u64, level: &f64) -> Option<u64> {
        if *level < 0.3 - 0.25 {
            return None;
        }
        Some(n + 1)
    }
    fn worker_b(n: u64, level: &f64) -> Option<u64> {
        if *level < 0.5 {
            return None;
        }
        Some(n * 2)
    }
    fn reporter(n: u64, level: &f64) -> Option<u64> {
        if *level < 0.7 {
            return None;
        }
        Some(n - 1)
    }
    // grn_fire("driver") — the enhanced source, all gates pass; the
    // pre-fire level is never read (all main-loop calls happen after the
    // fire), kept symbolic to mirror the .op's fire order.
    std::hint::black_box(&level);
    level = 1.0;
    let mut acc: u64 = 0;
    for i in 0..n {
        acc += worker_a(i, &level).unwrap_or(0);
        acc += worker_b(i, &level).unwrap_or(0);
        acc += reporter(i, &level).unwrap_or(0);
    }
    println!("acc = {acc}");
}
