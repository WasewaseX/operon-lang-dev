// loglens.rs — the native Rust baseline of apps/loglens/loglens.op (Bar C)
//
// BYTE-IDENTITY CONTRACT (see loglens.py header for the full rules):
// the parser is split-based and identical in all four engines; integers
// render via to_string(); every printed order is TOTAL; render_ratio()
// mirrors the Operon float sequence op-for-op (IEEE-754 doubles on
// exact integer inputs => bit-identical, then integer-only arithmetic);
// no float is ever printed except through render_ratio().
//
// Build: rustc -O loglens.rs -o loglens_rs
// This is the roadmap's Bar C for the loglens workload: the native
// algorithmic ceiling the interpreted engines are measured against.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::process;

fn render_ratio(num: i64, den: i64, nd: usize) -> String {
    // MIRRORED FLOAT SEQUENCE — keep op-for-op identical with loglens.op
    let x = num as f64 / den as f64;
    let mut sc: f64 = 1.0;
    for _ in 0..nd {
        sc = sc * 10.0;
    }
    let r = (x * sc + 0.5).floor() as i64;
    let si = sc as i64;
    let whole = r / si;
    let frac = r % si;
    if nd == 0 {
        return whole.to_string();
    }
    let mut fd = frac.to_string();
    while fd.len() < nd {
        fd = format!("0{}", fd);
    }
    format!("{}.{}", whole, fd)
}

fn is_digits(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| b.is_ascii_digit())
}

fn split_lines(text: &str) -> Vec<&str> {
    // split on \n, drop ONE trailing empty piece ("...\n" files)
    let mut parts: Vec<&str> = text.split('\n').collect();
    if !parts.is_empty() && parts[parts.len() - 1].is_empty() {
        parts.pop();
    }
    parts
}

fn parse_i64(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(0)
}

fn parse_line(line: &str) -> Option<(String, String, String, i64)> {
    let pieces: Vec<&str> = line.split('"').collect();
    if pieces.len() != 3 {
        return None;
    }
    let host_parts: Vec<&str> = pieces[0].split(' ').collect();
    if host_parts.is_empty() || host_parts[0].is_empty() {
        return None;
    }
    let host = host_parts[0].to_string();
    let req: Vec<&str> = pieces[1].split(' ').collect();
    if req.len() != 3 {
        return None;
    }
    let rest: Vec<&str> = pieces[2].split(' ').collect();
    if rest.len() != 3 || !rest[0].is_empty() {
        return None;
    }
    if !is_digits(rest[1]) || !is_digits(rest[2]) {
        return None;
    }
    Some((host, req[1].to_string(), rest[1].to_string(), parse_i64(rest[2])))
}

fn parse_records(text: &str) -> Vec<(String, String, String, i64)> {
    let mut out = Vec::new();
    for line in split_lines(text) {
        if let Some(rec) = parse_line(line) {
            out.push(rec);
        }
    }
    out
}

fn count_map(values: Vec<String>) -> HashMap<String, i64> {
    let mut m: HashMap<String, i64> = HashMap::new();
    for v in values {
        *m.entry(v).or_insert(0) += 1;
    }
    m
}

fn top_pairs(m: &HashMap<String, i64>, n: usize, numeric: bool) -> Vec<(String, i64)> {
    // count desc, then value asc (numeric value asc for status)
    let mut items: Vec<(String, i64)> = m.iter().map(|(k, c)| (k.clone(), *c)).collect();
    items.sort_by(|a, b| {
        if a.1 != b.1 {
            b.1.cmp(&a.1)
        } else if numeric {
            parse_i64(&a.0).cmp(&parse_i64(&b.0))
        } else {
            a.0.cmp(&b.0)
        }
    });
    items.truncate(n);
    items
}

fn sum_bytes(recs: &[(String, String, String, i64)]) -> i64 {
    recs.iter().map(|r| r.3).sum()
}

fn cmd_stats(text: &str, path: &str) {
    let recs = parse_records(text);
    let hosts = count_map(recs.iter().map(|r| r.0.clone()).collect());
    let urls = count_map(recs.iter().map(|r| r.1.clone()).collect());
    let status = count_map(recs.iter().map(|r| r.2.clone()).collect());
    println!("loglens stats {}", path);
    println!("records,{}", recs.len());
    println!("hosts,{}", hosts.len());
    println!("urls,{}", urls.len());
    println!("bytes,{}", sum_bytes(&recs));
    println!("status,count");
    let mut items: Vec<(String, i64)> = status.iter().map(|(k, c)| (k.clone(), *c)).collect();
    items.sort_by(|a, b| parse_i64(&a.0).cmp(&parse_i64(&b.0)));
    for (k, c) in items {
        println!("{},{}", k, c);
    }
}

fn cmd_top(text: &str, path: &str, field: &str, n: usize) {
    if field != "host" && field != "url" && field != "status" {
        println!("loglens: unknown field {}", field);
        process::exit(2);
    }
    let recs = parse_records(text);
    let idx = if field == "host" {
        0
    } else if field == "url" {
        1
    } else {
        2
    };
    let m = count_map(
        recs
            .iter()
            .map(|r| match idx {
                0 => r.0.clone(),
                1 => r.1.clone(),
                _ => r.2.clone(),
            })
            .collect(),
    );
    println!("loglens top {} {} {}", path, field, n);
    println!("rank,value,count,share");
    let pairs = top_pairs(&m, n, field == "status");
    for (i, p) in pairs.iter().enumerate() {
        let share = render_ratio(p.1 * 100, recs.len() as i64, 2);
        println!("{},{},{},{}", i + 1, p.0, p.1, share);
    }
}

fn cmd_errors(text: &str, path: &str, n: usize) {
    let recs = parse_records(text);
    let errs: Vec<_> = recs
        .iter()
        .filter(|r| parse_i64(&r.2) >= 400)
        .cloned()
        .collect();
    let m = count_map(errs.iter().map(|r| r.1.clone()).collect());
    println!("loglens errors {} {}", path, n);
    println!("count,{}", errs.len());
    println!("bytes,{}", sum_bytes(&errs));
    println!("rank,url,count,share");
    let pairs = top_pairs(&m, n, false);
    for (i, p) in pairs.iter().enumerate() {
        let share = render_ratio(p.1 * 100, errs.len() as i64, 2);
        println!("{},{},{},{}", i + 1, p.0, p.1, share);
    }
}

fn dashes(widths: &[usize]) -> String {
    let mut out = String::from("+");
    for w in widths {
        for _ in 0..(w + 2) {
            out.push('-');
        }
        out.push('+');
    }
    out
}

fn table_row(cells: &[String], widths: &[usize]) -> String {
    let mut out = String::from("|");
    for (i, c) in cells.iter().enumerate() {
        out.push(' ');
        out.push_str(c);
        for _ in 0..(widths[i] - c.len()) {
            out.push(' ');
        }
        out.push_str(" |");
    }
    out
}

fn render_table(head: &[&str], rows: &[Vec<String>]) -> Vec<String> {
    let ncols = head.len();
    let mut widths: Vec<usize> = head.iter().map(|h| h.len()).collect();
    for r in rows {
        for i in 0..ncols {
            if r[i].len() > widths[i] {
                widths[i] = r[i].len();
            }
        }
    }
    let mut out = vec![
        dashes(&widths),
        table_row(
            &head.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &widths,
        ),
        dashes(&widths),
    ];
    for r in rows {
        out.push(table_row(r, &widths));
    }
    out.push(dashes(&widths));
    out
}

fn cmd_table(text: &str, path: &str, n: usize) {
    let recs = parse_records(text);
    println!("loglens table {} {}", path, n);
    let head = ["host", "url", "status", "bytes"];
    let shown: Vec<Vec<String>> = recs
        .iter()
        .take(n)
        .map(|r| vec![r.0.clone(), r.1.clone(), r.2.clone(), r.3.to_string()])
        .collect();
    for l in render_table(&head, &shown) {
        println!("{}", l);
    }
}

fn usage() {
    println!("usage: loglens stats FILE");
    println!("       loglens top FILE FIELD N");
    println!("       loglens errors FILE [N]");
    println!("       loglens table FILE [N]");
}

fn main() {
    let pos: Vec<String> = env::args().skip(1).collect();
    if pos.len() < 2 {
        usage();
        return;
    }
    let cmd = pos[0].as_str();
    let path = pos[1].as_str();
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            println!("loglens: cannot read {}", path);
            process::exit(2);
        }
    };
    match cmd {
        "stats" => cmd_stats(&text, path),
        "top" => {
            if pos.len() < 4 {
                usage();
                return;
            }
            cmd_top(&text, path, &pos[2], parse_i64(&pos[3]) as usize);
        }
        "errors" => {
            let n = if pos.len() >= 3 {
                parse_i64(&pos[2]) as usize
            } else {
                10
            };
            cmd_errors(&text, path, n);
        }
        "table" => {
            let n = if pos.len() >= 3 {
                parse_i64(&pos[2]) as usize
            } else {
                10
            };
            cmd_table(&text, path, n);
        }
        _ => usage(),
    }
}
