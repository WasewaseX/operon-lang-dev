// native.rs — Rust-native mirrors of the operon benchmark fixtures.
// Zero external crates; compiled with `rustc -O --edition 2021`.
// Every workload prints the SAME canonical string as the operon fixture,
// so run_xlang.py can cross-verify correctness across all four runners.
// Mirrors: scripts/bench/*.op + scripts/bench/micro/*.op (1:1 with the
// native-py mirrors in scripts/bench_compare.py).
use std::collections::HashMap;
use std::env;

fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn pasc(n: i64, k: i64) -> i64 {
    if k == 0 || k == n { 1 } else { pasc(n - 1, k - 1) + pasc(n - 1, k) }
}

fn w_fib25() -> String {
    format!("fib(25) = {}", fib(25))
}

fn w_loops() -> String {
    let mut acc: i64 = 0;
    for i in 0..200000i64 {
        acc += i % 7;
    }
    format!("acc = {}", acc)
}

fn w_strings() -> String {
    let mut s = String::new();
    for i in 0..4000i64 {
        s = s + "x{t}y".replace("{t}", &(i % 10).to_string()).as_str();
    }
    format!("len = {}", s.len())
}

fn w_collections() -> String {
    let mut m: HashMap<String, i64> = HashMap::new();
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..20000i64 {
        let k = format!("k{}", i % 2000);
        *m.entry(k).or_insert(0) += 1;
        xs.push(i % 97);
    }
    let total: i64 = xs.iter().sum();
    let distinct = m.len();
    let k1 = m["k1"];
    format!("total = {}, distinct = {}, k1 = {}", total, distinct, k1)
}

fn w_recursion() -> String {
    format!("C(20,10) = {}", pasc(20, 10))
}

fn w_grn() -> String {
    // mirror of grn.op: per-call gate check (level >= threshold), one gene
    // enhanced by 0.25. driver fired at 1.0 — all gates pass (happy path).
    let level: f64 = 1.0;
    let worker_a = |n: i64| -> Option<i64> {
        if level < 0.3 - 0.25 { None } else { Some(n + 1) }
    };
    let worker_b = |n: i64| -> Option<i64> {
        if level < 0.5 { None } else { Some(n * 2) }
    };
    let reporter = |n: i64| -> Option<i64> {
        if level < 0.7 { None } else { Some(n - 1) }
    };
    let mut acc: i64 = 0;
    for i in 0..20000i64 {
        acc += worker_a(i).unwrap_or(0) + worker_b(i).unwrap_or(0) + reporter(i).unwrap_or(0);
    }
    format!("acc = {}", acc)
}

fn w_m_empty() -> String {
    "ok".to_string()
}

fn w_m_call() -> String {
    fn nop(n: i64) -> i64 { n }
    let mut a: i64 = 0;
    for i in 0..100000i64 {
        a += nop(i);
    }
    format!("a = {}", a)
}

fn w_m_forrange() -> String {
    let mut a: i64 = 0;
    for _ in 0..200000i64 {
        a += 1;
    }
    format!("a = {}", a)
}

fn w_m_while() -> String {
    let (mut i, mut a) = (200000i64, 0i64);
    while i > 0 {
        a += 1;
        i -= 1;
    }
    format!("a = {}", a)
}

fn w_m_varread() -> String {
    let (x, mut a) = (7i64, 0i64);
    for _ in 0..200000i64 {
        a += x;
    }
    format!("a = {}", a)
}

fn w_m_intadd() -> String {
    let (mut a, b) = (0i64, 1i64);
    for _ in 0..300000i64 {
        a += b;
    }
    format!("a = {}", a)
}

fn w_m_listpush() -> String {
    let mut xs: Vec<i64> = Vec::new();
    for i in 0..50000i64 {
        xs.push(i);
    }
    format!("len = {}", xs.len())
}

fn w_m_listidx() -> String {
    let xs: Vec<i64> = (0..2000i64).collect();
    let mut a: i64 = 0;
    for i in 0..100000i64 {
        a += xs[(i % 2000) as usize];
    }
    format!("a = {}", a)
}

fn w_m_mapset() -> String {
    let mut m: HashMap<String, i64> = HashMap::new();
    for i in 0..40000i64 {
        m.insert((i % 4000).to_string(), i);
    }
    format!("len = {}", m.len())
}

fn w_m_mapget() -> String {
    let mut m: HashMap<String, i64> = HashMap::new();
    for i in 0..4000i64 {
        m.insert(i.to_string(), i);
    }
    let mut a: i64 = 0;
    for i in 0..50000i64 {
        a += m[&(i % 4000).to_string()];
    }
    format!("a = {}", a)
}

fn w_m_strcat() -> String {
    let mut s = String::new();
    for _ in 0..12000 {
        s += "ab";
    }
    format!("len = {}", s.len())
}

fn main() {
    let name = env::args().nth(1).unwrap_or_default();
    let out = match name.as_str() {
        "fib25" => w_fib25(),
        "loops" => w_loops(),
        "strings" => w_strings(),
        "collections" => w_collections(),
        "recursion" => w_recursion(),
        "grn" => w_grn(),
        "m_empty" => w_m_empty(),
        "m_call" => w_m_call(),
        "m_forrange" => w_m_forrange(),
        "m_while" => w_m_while(),
        "m_varread" => w_m_varread(),
        "m_intadd" => w_m_intadd(),
        "m_listpush" => w_m_listpush(),
        "m_listidx" => w_m_listidx(),
        "m_mapset" => w_m_mapset(),
        "m_mapget" => w_m_mapget(),
        "m_strcat" => w_m_strcat(),
        _ => format!("unknown workload: {}", name),
    };
    println!("{}", out);
}
