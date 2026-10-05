// bench: call_fib_rs — native Rust floor for the P4 call ladder (builder-E)
// Same recursive shape; #[inline(never)] forces real calls. Timed with
// Instant, min of reps. `rustc -O call_fib_rs.rs -o call_fib_rs && ./call_fib_rs <n> <reps>`
use std::env;

#[inline(never)]
fn fib(n: u64) -> u64 {
    if n < 2 { return n; }
    fib(n - 1) + fib(n - 2)
}

fn main() {
    let a: Vec<String> = env::args().collect();
    let n: u64 = a[1].parse().unwrap();
    let reps: usize = a[2].parse().unwrap();
    fib(n.min(20)); // warm
    let mut best = f64::MAX;
    let mut r = 0u64;
    for _ in 0..reps {
        let t0 = std::time::Instant::now();
        r = fib(n);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if ms < best { best = ms; }
    }
    println!("CALLFIB n={} r={} ms={}", n, r, best);
}
