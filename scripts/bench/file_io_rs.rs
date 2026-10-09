// bench: file_io_rs — native Rust floor for the file-I/O workload
// (R0.8, L-039). Mirrors scripts/bench/file_io.op exactly: a 4 KiB payload
// ("0123456789abcdef" x 256), rewritten n times, content length verified
// each pass. (The native-py mirror's payload is 16x — a documented
// shape-compare in BENCH.md; file_io is the suite's documented NOISY
// workload and its rows are indicative only.) Writes its OWN path so the
// three runners never share a temp file mid-flight.
// `rustc -O file_io_rs.rs -o file_io_rs && ./file_io_rs [n]` (default 300)
use std::env;
use std::fs;
use std::process;

fn main() {
    let n: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let payload = "0123456789abcdef".repeat(256);
    let path = "/tmp/operon_bench_io_rs.txt";
    let mut ok = 0u64;
    for i in 0..n {
        if fs::write(path, format!("{payload}{i}")).is_err() {
            eprintln!("write failed at {i}");
            process::exit(1);
        }
        let back = match fs::read_to_string(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("read failed at {i}: {e}");
                process::exit(1);
            }
        };
        if back.len() == payload.len() + i.to_string().len() {
            ok += 1;
        }
    }
    let _ = fs::remove_file(path);
    println!("io ok = {ok}");
}
