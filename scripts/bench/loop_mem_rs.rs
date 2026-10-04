// loop_mem_rs.rs — native-Rust floor for the P3 loop-memory audit (builder-E).
// One leg per invocation: args <shape> <n> [outer]. The accumulate leg
// (Vec<i64> push) shows the 8 B/elem typed floor that the roadmap's typed-
// array design space targets; the transient legs show a flat-RSS loop.
use std::env;
use std::time::Instant;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: loop_mem_rs <shape> <n> [outer]");
        std::process::exit(2);
    }
    let shape = args[1].clone();
    let n: u64 = args[2].parse().unwrap();
    let outer: u64 = if args.len() > 3 { args[3].parse().unwrap() } else { 1000 };

    let t0 = Instant::now();
    let mut s: u64 = 0;

    match shape.as_str() {
        "acc_list" => {
            let mut xs: Vec<u64> = Vec::with_capacity(0);
            let mut i: u64 = 0;
            while i < n {
                xs.push(i);
                i += 1;
            }
            for v in &xs {
                s += v;
            }
        }
        "transient_while" => {
            let mut i: u64 = 0;
            while i < n {
                s += i;
                i += 1;
            }
        }
        "transient_forrange" => {
            for i in 0..n {
                s += i;
            }
        }
        "transient_while_let" => {
            let mut i: u64 = 0;
            while i < n {
                let t = i * 2;
                s += t;
                i += 1;
            }
        }
        "nested" => {
            for _ in 0..outer {
                let mut j: u64 = 0;
                while j < n {
                    s += j;
                    j += 1;
                }
            }
        }
        _ => {
            eprintln!("unknown shape {}", shape);
            std::process::exit(2);
        }
    }

    let dt = t0.elapsed().as_secs_f64() * 1000.0;
    // s is the per-shape deterministic checksum; the cross-engine
    // differential contract is: same shape + same n + same outer => same s.
    // VmHWM = peak RSS since exec (clean accounting; the spawner's fork
    // floor pollutes ru_maxrss, so we self-report).
    let hwm: u64 = std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("VmHWM"))
        .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        .unwrap_or(0);
    println!("LOOP engine=rust shape={} n={} outer={} time_ms={:.3} hwm_kb={} cksum={}", shape, n, outer, dt, hwm, s);
}
