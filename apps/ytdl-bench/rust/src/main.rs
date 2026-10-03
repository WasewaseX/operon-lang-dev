// ============================================================
// ytdl-rs — lightweight video-download orchestrator, rust edition
// Same CLI contract as operon/ytdl.op, python/, node/ ports.
//   ytdl-rs doctor | info <url> | get <url> [options] |
//            meta <file> | queue <jobs.txt> [--workers N] |
//            startup | spawnbench N | cpu N
// std-only (zero crates): subprocess orchestration via std::process,
// concurrency via scoped threads, JSON via the bundled mini_json.
// ============================================================
mod mini_json;

use mini_json::Json;
use std::io;
use std::process::{Command, Stdio};
use std::time::Instant;

const OUT_DEFAULT: &str = "downloads";
const CLIP: usize = 60;

fn clip(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

fn probe(tool: &str, flag: &str) -> String {
    match Command::new(tool).arg(flag).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => String::new(),
    }
}

fn cmd_doctor() -> i32 {
    let tools: [(&str, &str); 4] = [
        ("yt-dlp", "--version"),
        ("ffmpeg", "-version"),
        ("ffprobe", "-version"),
        ("aria2c", "--version"),
    ];
    let mut found = 0;
    for (tool, flag) in tools {
        let v = probe(tool, flag);
        if !v.is_empty() {
            found += 1;
            println!("tool={} version={}", tool, clip(&v, CLIP));
        } else {
            println!("tool={} MISSING", tool);
        }
    }
    println!("doctor: ok {}/{}", found, tools.len());
    if found < 3 {
        return 1;
    }
    0
}

fn cmd_info(url: &str) -> i32 {
    let t0 = Instant::now();
    let out = match Command::new("yt-dlp")
        .args(["-J", "--no-warnings", url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            println!("info: FAIL err={}", clip(&e.to_string(), 160));
            return 1;
        }
    };
    if !out.status.success() {
        println!(
            "info: FAIL code={} err={}",
            out.status.code().unwrap_or(-1),
            clip(&String::from_utf8_lossy(&out.stderr), 160)
        );
        return 1;
    }
    let j = match mini_json::parse(&String::from_utf8_lossy(&out.stdout)) {
        Ok(j) => j,
        Err(e) => {
            println!("info: FAIL json={}", clip(&e, 160));
            return 1;
        }
    };
    let formats = j.get("formats").map(|f| f.as_arr()).unwrap_or(&[]).to_vec();
    let mut best = String::new();
    for f in &formats {
        if let Some(id) = f.get("format_id") {
            best = id.as_str().to_string();
        }
    }
    println!("title={}", j.get("title").map(|t| t.as_str()).unwrap_or(""));
    println!("formats={}", formats.len());
    println!("best={}", best);
    println!("info: ok secs={:.3}", t0.elapsed().as_secs_f64());
    0
}

fn build_args(url: &str, mode: &str, fmt: &str, out: &str, resume: bool) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if resume {
        args.push("-c".into());
    }
    args.push("--no-warnings".into());
    match mode {
        "audio" => args.extend(["-x", "--audio-format", "mp3"].iter().map(|s| s.to_string())),
        "subs" => args.extend(
            ["--write-subs", "--skip-download", "--sub-langs", "en"]
                .iter()
                .map(|s| s.to_string()),
        ),
        _ => args.extend(["-f", fmt].iter().map(|s| s.to_string())),
    }
    args.push("-o".into());
    args.push(format!("{}/%(title)s.%(ext)s", out));
    args.push(url.into());
    args
}

fn do_job(idx: usize, url: &str, mode: &str, fmt: &str, out: &str, resume: bool) -> (i32, String) {
    let t0 = Instant::now();
    println!("[get] start url={} mode={} fmt={}", url, mode, fmt);
    let r = Command::new("yt-dlp")
        .args(build_args(url, mode, fmt, out, resume))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let wall = t0.elapsed().as_secs_f64();
    match r {
        Err(e) => (
            0,
            format!(
                "job {}: {} {} -> FAIL(err={}) secs={:.3}",
                idx,
                mode,
                url,
                clip(&e.to_string(), CLIP),
                wall
            ),
        ),
        Ok(o) if !o.status.success() => (
            0,
            format!(
                "job {}: {} {} -> FAIL(code={}) secs={:.3}",
                idx,
                mode,
                url,
                o.status.code().unwrap_or(-1),
                wall
            ),
        ),
        Ok(_) => (
            1,
            format!("job {}: {} {} -> ok secs={:.3}", idx, mode, url, wall),
        ),
    }
}

fn cmd_get(url: &str, fmt: &str, out: &str, mode: &str, resume: bool) -> i32 {
    let (ok, _) = do_job(0, url, mode, fmt, out, resume);
    if ok == 0 {
        return 1;
    }
    println!("get: ok");
    0
}

fn cmd_meta(path: &str) -> i32 {
    let out = match Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_format", path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            println!("meta: FAIL err={}", clip(&e.to_string(), 160));
            return 1;
        }
    };
    if !out.status.success() {
        println!(
            "meta: FAIL code={} err={}",
            out.status.code().unwrap_or(-1),
            clip(&String::from_utf8_lossy(&out.stderr), 160)
        );
        return 1;
    }
    let j = match mini_json::parse(&String::from_utf8_lossy(&out.stdout)) {
        Ok(j) => j,
        Err(e) => {
            println!("meta: FAIL json={}", clip(&e, 160));
            return 1;
        }
    };
    let f = j.get("format");
    let duration = f.and_then(|f| f.get("duration")).map(|v| match v {
        Json::Num(n) => n.to_string(),
        Json::Str(s) => s.clone(),
        _ => "?".into(),
    }).unwrap_or_else(|| "?".into());
    let size = f.and_then(|f| f.get("size")).map(|v| match v {
        Json::Num(n) => n.to_string(),
        Json::Str(s) => s.clone(),
        _ => "?".into(),
    }).unwrap_or_else(|| "?".into());
    println!("duration={} size={}", duration, size);
    println!("meta: ok");
    0
}

fn load_jobs(path: &str) -> io::Result<Vec<(usize, String, String)>> {
    let text = std::fs::read_to_string(path)?;
    let mut jobs = Vec::new();
    for line in text.lines() {
        let mut parts = line.trim().split(' ');
        if let (Some(url), Some(mode)) = (parts.next(), parts.next()) {
            jobs.push((jobs.len(), url.to_string(), mode.to_string()));
        }
    }
    Ok(jobs)
}

fn cmd_queue(jobsfile: &str, workers: usize) -> i32 {
    let t0 = Instant::now();
    let jobs = match load_jobs(jobsfile) {
        Ok(j) => j,
        Err(e) => {
            println!("queue: FAIL read err={}", clip(&e.to_string(), 160));
            return 2;
        }
    };
    let total = jobs.len();
    if total == 0 {
        println!("queue: no jobs");
        return 2;
    }
    let w = workers.min(total);
    let oks: i32 = std::thread::scope(|scope| {
        // contiguous chunks, one thread per chunk (same shape as the
        // Operon port's spawn/join map-reduce; no shared mutation)
        let per = (total + w - 1) / w;
        let mut handles = Vec::new();
        for c in 0..w {
            let lo = c * per;
            let hi = ((c + 1) * per).min(total);
            if lo >= hi {
                continue;
            }
            let slice = &jobs[lo..hi];
            handles.push(scope.spawn(move || {
                let mut out = Vec::new();
                for (idx, url, mode) in slice {
                    out.push(do_job(*idx, url, mode, "best", OUT_DEFAULT, false));
                }
                out
            }));
        }
        let mut oks: i32 = 0;
        for h in handles {
            for (ok, line) in h.join().unwrap_or_default() {
                oks += ok;
                println!("{}", line);
            }
        }
        oks
    });
    let wall = t0.elapsed().as_secs_f64();
    println!("queue: {}/{} ok in {:.3}s (workers={})", oks, total, wall, w);
    if oks < total as i32 {
        return 1;
    }
    0
}

fn fib(n: u64) -> u64 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

fn cmd_spawnbench(n: usize) -> i32 {
    let t0 = Instant::now();
    for i in 0..n {
        match Command::new("yt-dlp").arg("--version").output() {
            Ok(o) if o.status.success() => {}
            _ => {
                println!("spawnbench: FAIL at {}", i);
                return 1;
            }
        }
    }
    println!("spawnbench: ok n={} secs={:.3}", n, t0.elapsed().as_secs_f64());
    0
}

fn cmd_cpu(n: u64) -> i32 {
    let t0 = Instant::now();
    let v = fib(n);
    println!("cpu: ok fib({})={} secs={:.3}", n, v, t0.elapsed().as_secs_f64());
    0
}

fn flag_of(a: &[String], name: &str, dflt: &str) -> String {
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1))
        .cloned()
        .unwrap_or_else(|| dflt.to_string())
}

fn usage(code: i32) -> i32 {
    println!(
        "usage: ytdl doctor | info <url> | get <url> [options] | meta <file> | \
         queue <jobs> [--workers N] | startup | spawnbench N | cpu N"
    );
    code
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let code = if a.is_empty() {
        usage(2)
    } else {
        match a[0].as_str() {
            "doctor" => cmd_doctor(),
            "info" if a.len() >= 2 => cmd_info(&a[1]),
            "get" if a.len() >= 2 => {
                let mode = if a.iter().any(|x| x == "--audio") {
                    "audio"
                } else if a.iter().any(|x| x == "--subs") {
                    "subs"
                } else {
                    "video"
                };
                cmd_get(
                    &a[1],
                    &flag_of(&a, "--format", "bv*+ba/b"),
                    &flag_of(&a, "--out", OUT_DEFAULT),
                    mode,
                    a.iter().any(|x| x == "--resume"),
                )
            }
            "meta" if a.len() >= 2 => cmd_meta(&a[1]),
            "queue" if a.len() >= 2 => {
                let workers = flag_of(&a, "--workers", "4").parse().unwrap_or(4);
                cmd_queue(&a[1], workers)
            }
            "startup" => {
                println!("startup: ok");
                0
            }
            "spawnbench" if a.len() >= 2 => cmd_spawnbench(a[1].parse().unwrap_or(0)),
            "cpu" if a.len() >= 2 => cmd_cpu(a[1].parse().unwrap_or(0)),
            "info" | "get" | "meta" | "queue" | "spawnbench" | "cpu" => usage(2),
            other => {
                println!("ytdl: unknown command '{}'", other);
                2
            }
        }
    };
    std::process::exit(code);
}
