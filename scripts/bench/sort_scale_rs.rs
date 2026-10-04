// sort_scale_rs.rs — native-Rust mirror for the P2 sort audit (builder-E).
// Same LCG seed-42 stream, same insertion-sort algorithm with a native
// comparator closure (isolates the per-comparison VM callback cost: operon
// total minus native insertion = callback overhead at identical call counts),
// plus sort_unstable_by as the native algorithmic floor.
// Build: rustc -O sort_scale_rs.rs -o sort_scale_rs
// Output lines match sort_scale.op / sort_scale_py.py.
use std::env;
use std::time::Instant;

fn lcg_list(n: usize) -> Vec<i64> {
    let mut xs = Vec::with_capacity(n);
    let mut x: i64 = 42;
    for _ in 0..n {
        x = (x * 1103515245 + 12345) % 2147483648;
        xs.push(x % 1000000);
    }
    xs
}

fn insertion_sort_callback(xs: &[i64]) -> (Vec<i64>, u64) {
    let mut v = xs.to_vec();
    let mut calls: u64 = 0;
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 {
            calls += 1;
            if !(v[j - 1] < v[j]) {
                v.swap(j - 1, j);
                j -= 1;
            } else {
                break;
            }
        }
    }
    (v, calls)
}

fn main() {
    let arg = env::args().nth(1).unwrap_or_else(|| "1000,2000,4000,8000".into());
    let sizes: Vec<usize> = arg.split(',').map(|s| s.parse().unwrap()).collect();

    for n in sizes {
        let xs = lcg_list(n);

        // matched-algorithm insertion sort with native closure
        let mut best = f64::MAX;
        let mut out = Vec::new();
        for _ in 0..3 {
            let t0 = Instant::now();
            out = insertion_sort_callback(&xs).0;
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            if dt < best {
                best = dt;
            }
        }
        let (v, calls) = insertion_sort_callback(&xs);
        let ck: i64 = v.iter().sum();
        println!("SCALE engine=rust n={} mode=insertion min_ms={:.3} cksum={}", n, best, ck);
        println!("SCALE engine=rust n={} mode=counted calls={} cksum={}", n, calls, ck);

        // native algorithmic floor
        let mut best = f64::MAX;
        for _ in 0..3 {
            let t0 = Instant::now();
            let mut v = xs.clone();
            v.sort_unstable();
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            if dt < best {
                best = dt;
            }
            out = v;
        }
        let ck: i64 = out.iter().sum();
        println!("SCALE engine=rust n={} mode=builtin min_ms={:.3} cksum={}", n, best, ck);
    }
}
