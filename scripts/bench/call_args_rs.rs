// bench: call_args_rs — native Rust floor for the arity ladder (builder-E)
// Same shapes as call_args.op / call_args_py.py; plain function calls on the
// native stack = the per-arg binding floor. `rustc -O call_args_rs.rs -o
// call_args_rs && ./call_args_rs <n> <reps>`
use std::env;

#[inline(never)]  // force REAL calls: the floor must pay the native call cost
fn f0() -> i64 { 7 }
#[inline(never)]
fn f1(a: i64) -> i64 { a }
#[inline(never)]
fn f2(a: i64, b: i64) -> i64 { a + b }
#[inline(never)]
fn f4(a: i64, b: i64, c: i64, d: i64) -> i64 { a + b + c + d }
#[inline(never)]
fn f8(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64 {
    a + b + c + d + e + f + g + h
}

#[inline(never)]
fn leg0(n: i64) -> i64 {
    let mut s = 0i64; let mut i = 0i64;
    while i < n { s = s + f0(); i = i + 1; }
    s
}
#[inline(never)]
fn leg1(n: i64) -> i64 {
    let mut s = 0i64; let mut i = 0i64;
    while i < n { s = s + f1(i); i = i + 1; }
    s
}
#[inline(never)]
fn leg2(n: i64) -> i64 {
    let mut s = 0i64; let mut i = 0i64;
    while i < n { s = s + f2(i, i); i = i + 1; }
    s
}
#[inline(never)]
fn leg4(n: i64) -> i64 {
    let mut s = 0i64; let mut i = 0i64;
    while i < n { s = s + f4(i, i, i, i); i = i + 1; }
    s
}
#[inline(never)]
fn leg8(n: i64) -> i64 {
    let mut s = 0i64; let mut i = 0i64;
    while i < n { s = s + f8(i, i, i, i, i, i, i, i); i = i + 1; }
    s
}

fn main() {
    let a: Vec<String> = env::args().collect();
    let n: i64 = a[1].parse().unwrap();
    let reps: usize = a[2].parse().unwrap();
    let mut best = [(0i64, f64::MAX); 5];
    for _ in 0..reps {
        for k in 0..5 {
            let t0 = std::time::Instant::now();
            let ck = match k {
                0 => leg0(n), 1 => leg1(n), 2 => leg2(n), 3 => leg4(n), _ => leg8(n),
            };
            let ns = t0.elapsed().as_secs_f64() * 1e9 / n as f64;
            if ns < best[k].1 { best[k] = (ck, ns); }
        }
    }
    for (ar, b) in [(0, 0), (1, 1), (2, 2), (4, 3), (8, 4)] {
        let (ck, ns) = best[b];
        println!("ARGS arity={} calls={} ns_per_call={} ck={}", ar, n, ns, ck);
    }
}
