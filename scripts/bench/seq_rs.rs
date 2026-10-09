// bench: seq_rs — native Rust floor for the seq workload (R0.8, L-039).
// Mirrors scripts/bench/seq_motifs.op / native_seq (bench_compare.py)
// exactly: seeded LCG genome (i128 intermediates keep the multiply exact,
// matching CPython bignum semantics), GC count, 3-mer "ACG" scan, 2-mer
// "GT" scan. Prints the same line the .op promotes.
// `rustc -O seq_rs.rs -o seq_rs && ./seq_rs [n]` (default n = 6000)
use std::env;

fn main() {
    let n: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6000);
    let mut seed: i128 = 42;
    let alpha = [b'A', b'C', b'G', b'T'];
    let mut genome: Vec<u8> = Vec::with_capacity(n as usize);
    for _ in 0..n {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        genome.push(alpha[(seed % 4) as usize]);
    }
    let g = &genome[..];
    let gc = g.iter().filter(|&&c| c == b'G' || c == b'C').count();
    let mut kmers = 0usize;
    for i in 0..g.len().saturating_sub(3) {
        if &g[i..i + 3] == b"ACG" {
            kmers += 1;
        }
    }
    let mut motifs = 0usize;
    for i in 0..g.len().saturating_sub(2) {
        if &g[i..i + 2] == b"GT" {
            motifs += 1;
        }
    }
    println!("gc={gc} acg={kmers} gt={motifs}");
}
