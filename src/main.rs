//! main.rs — Operon toolchain CLI (Rust core).
//! run | check | test | fmt | build | profile | crispr | bench | version

// The language core lives in the `operon` library crate (src/lib.rs);
// this binary is the CLI shell over it.
use operon::genes;
use operon::graph;
use operon::interp;
use operon::parser;
use operon::tools;

use operon::ast::Stmt;
use operon::die;
use operon::tools::Opts;
use operon::value::Value;

fn main() {
    // The evaluator recurses through exec_block → eval → call_gene; deep
    // Operon recursion needs a real stack. The toolchain therefore runs on a
    // dedicated worker with a 512 MiB stack, and the interpreter's own depth
    // limit (10_000) contains runaway recursion as catchable overflow stress
    // long before the native stack is at risk.
    let child = std::thread::Builder::new()
        .stack_size(1 << 29)
        .spawn(real_main)
        .expect("cannot spawn toolchain worker");
    // a worker panic must never masquerade as success (Critic-X B2)
    match child.join() {
        Ok(()) => {}
        Err(_) => {
            eprintln!("[fatal] internal toolchain panic — this input crashed the runtime");
            std::process::exit(101);
        }
    }
}

fn real_main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        usage();
        std::process::exit(2);
    }
    let cmd = argv[0].clone();
    let rest = &argv[1..];

    // dx-r5 (audit D-4): rustc/go/tsc answer --help; operon used to say
    // "unknown command". Same usage as the no-args case, exit 0.
    if cmd == "--help" || cmd == "-h" || cmd == "help" {
        usage();
        std::process::exit(0);
    }

    // extract flags
    let mut opts = Opts {
        cell: None,
        variant: None,
        rna: None,
        entry: None,
        use_ires: false,
        frame: None,
        args: Vec::new(),
        quiet: false,
        caps: interp::Caps::default(),
        profile: false,
        stdout_sink: None,
        vm: false,
    };
    let mut json = false;
    let mut strict = false;
    let mut nmd = false;
    let mut purge = false;
    let mut write = false;
    let mut matrix = false;
    // dx-r6: true after the `--` separator — remaining args are program argv
    let mut passthrough = false;
    let mut fuel: Option<u64> = None;
    let mut knockout = String::new();
    let mut iters = 20usize;
    let mut outfile = String::new();
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].clone();
        match a.as_str() {
            "--entry" => {
                i += 1;
                opts.entry = rest.get(i).cloned();
            }
            "--allow-read" => {
                i += 1;
                match rest.get(i) {
                    Some(p) if !p.starts_with("--") => {
                        if let Err(s) = opts.caps.add_grant("read", p) {
                            die(&format!("invalid --allow-read: {}", s.message));
                        }
                    }
                    _ => die("--allow-read needs a path argument"),
                }
            }
            "--allow-write" => {
                i += 1;
                match rest.get(i) {
                    Some(p) if !p.starts_with("--") => {
                        if let Err(s) = opts.caps.add_grant("write", p) {
                            die(&format!("invalid --allow-write: {}", s.message));
                        }
                    }
                    _ => die("--allow-write needs a path argument"),
                }
            }
            "--allow-run" => {
                i += 1;
                match rest.get(i) {
                    Some(p) if !p.starts_with("--") => {
                        if let Err(s) = opts.caps.add_grant("run", p) {
                            die(&format!("invalid --allow-run: {}", s.message));
                        }
                    }
                    _ => die("--allow-run needs a program name argument"),
                }
            }
            "--allow-py" => {
                // substrate-r1: Python bridge grant — exact-match per module
                i += 1;
                match rest.get(i) {
                    Some(p) if !p.starts_with("--") => {
                        if let Err(s) = opts.caps.add_grant("py", p) {
                            die(&format!("invalid --allow-py: {}", s.message));
                        }
                    }
                    _ => die("--allow-py needs a module name argument"),
                }
            }
            "--allow-exit" => {
                // sec-r2 (audit C-11): exit() is a capability, default-deny
                opts.caps.exit_allowed = true;
            }
            "--allow-net" => {
                i += 1;
                match rest.get(i) {
                    Some(p) if !p.starts_with("--") => {
                        if let Err(s) = opts.caps.add_grant("net", p) {
                            die(&format!("invalid --allow-net: {}", s.message));
                        }
                    }
                    _ => die("--allow-net needs a host:port argument"),
                }
            }
            "--allow-env" => {
                i += 1;
                match rest.get(i) {
                    Some(p) if !p.starts_with("--") => {
                        if let Err(s) = opts.caps.add_grant("env", p) {
                            die(&format!("invalid --allow-env: {}", s.message));
                        }
                    }
                    _ => die("--allow-env needs a variable name argument"),
                }
            }
            "--allow-all" => {
                opts.caps = interp::Caps::allow_all();
            }
            "--fuel" => {
                i += 1;
                match rest.get(i).map(|s| s.parse::<u64>()) {
                    Some(Ok(p)) => fuel = Some(p),
                    _ => die("--fuel needs a number argument (e.g. --fuel 500000000)"),
                }
            }
            "--matrix" => matrix = true,
            "--variant" => {
                i += 1;
                opts.variant = rest.get(i).cloned();
            }
            "--cell" => {
                i += 1;
                opts.cell = rest.get(i).cloned();
            }
            "--rna" => {
                i += 1;
                opts.rna = rest.get(i).cloned();
            }
            "--frame" => {
                i += 1;
                opts.frame = rest.get(i).cloned();
            }
            "--knockout" => {
                i += 1;
                knockout = rest.get(i).cloned().unwrap_or_default();
            }
            "--iters" => {
                i += 1;
                match rest.get(i).map(|s| s.parse::<usize>()) {
                    Some(Ok(n)) => iters = n,
                    _ => die("--iters needs a number argument (e.g. --iters 50)"),
                }
            }
            "-o" | "--out" => {
                i += 1;
                outfile = rest.get(i).cloned().unwrap_or_default();
            }
            "--ires" => opts.use_ires = true,
            "--json" => json = true,
            "--strict" => strict = true,
            "--quiet" => opts.quiet = true,
            // W09 (A2): run compiled gene bodies on the bytecode VM
            "--vm" => opts.vm = true,
            "--nmd" => nmd = true,
            "--nmd=purge" | "--purge" => {
                nmd = true;
                purge = true;
            }
            "--write" => write = true,
            "--" => {
                // dx-r6 (loop-5-a audit MED): POSIX `--` separator — everything
                // after it belongs to the PROGRAM, not the host CLI. Without
                // it, `operon run app.op --key value` died with "unknown flag
                // '--key'" and half of std/args.op's documented conventions
                // were unreachable from a real command line. Unknown --flags
                // BEFORE the separator still fail loudly (dx-r1 typo guard).
                passthrough = true;
            }
            _ => {
                // dx-r1 (parity audit W6): unknown flags silently became
                // program argv — `operon run f.op --strick` ran with a typo'd
                // flag and no warning. Fail loudly instead.
                if a.starts_with("--") && !passthrough {
                    die(&format!(
                        "unknown flag '{}' — run `operon` with no arguments for usage",
                        a
                    ));
                }
                positional.push(a);
            }
        }
        i += 1;
    }

    match cmd.as_str() {
        "--version" | "-V" => {
            println!(
                "Operon {} (rust-core, cpp-kernel)",
                env!("CARGO_PKG_VERSION")
            );
        }
        "version" => {
            println!(
                "Operon {} (rust-core, cpp-kernel)",
                env!("CARGO_PKG_VERSION")
            );
        }
        "repl" => {
            repl();
        }
        "run" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("run needs a file"),
            };
            opts.args = positional[1..].to_vec();
            let mut l = match tools::load_file(&file, &opts) {
                Ok(l) => l,
                Err(e) => die(&e),
            };
            // W09 (A2/A3): compile gene bodies for the bytecode VM. Must
            // happen after load (top-level binds gene defs) and before the
            // entry call; sequences/workers stay on the tree-walk.
            if opts.vm {
                tools::vm_compile(&mut l);
            }
            if let Some(f) = fuel {
                l.interp.step_budget = f;
            }
            // run-wide shared fuel pool: host + every spawned worker cell
            // drain ONE pool, so `loop { spawn(...) }` cannot multiply the
            // budget (SPEC §9b: 200M steps per run, not per interpreter)
            let total = fuel.unwrap_or(500_000_000);
            l.interp.fuel_pool = Some(std::sync::Arc::new(std::sync::atomic::AtomicI64::new(
                total as i64,
            )));
            if opts.frame.is_none() {
                let result = tools::run_entry(&mut l, &opts);
                match result {
                    Ok(_) => {}
                    Err(s) => {
                        // dx-r3 (re-audit): the PRIMARY diagnostic gets a
                        // location when the stress carries its origin line —
                        // matching mainstream norms where the fatal error is
                        // the located one
                        if s.line > 0 {
                            eprintln!(
                                "[contained] [{}] {}:{}: {}",
                                s.kind, l.interp.file, s.line, s.message
                            );
                        } else {
                            eprintln!("[contained] [{}] {}", s.kind, s.message);
                        }
                        // W007: call-chain traceback — innermost frame first,
                        // each frame the gene and the call site that invoked
                        // it. Render capped at 64 (the capture cap); the
                        // chain leaks nothing beyond the script path already
                        // printed above (no env, no cwd, no host paths).
                        if !s.chain.is_empty() {
                            for (i, (name, line)) in s.chain.iter().enumerate() {
                                if i >= 64 {
                                    eprintln!("  … {} more frame(s)", s.chain.len() - 64);
                                    break;
                                }
                                if *line > 0 {
                                    eprintln!("  at {} ({}:{})", name, l.interp.file, line);
                                } else {
                                    eprintln!("  at {}", name);
                                }
                            }
                            eprintln!("  at main");
                        }
                        // dx-r1 (parity audit W2): a failing program must not
                        // report success — CI/shell pipelines trusted rc=0
                        // from scripts that died. 1 = uncaught top-level stress.
                        tools::flush_notes(&l, opts.quiet);
                        std::process::exit(1);
                    }
                }
            }
            tools::flush_notes(&l, opts.quiet);
            let strict_cell = l
                .interp
                .cell
                .get("wobble.strict")
                .map(|v| v == "true")
                .unwrap_or(false);
            if (strict || strict_cell) && l.interp.notes.iter().any(|n| n.rung >= 3) {
                std::process::exit(3);
            }
        }
        "check" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("check needs a file"),
            };
            let rep = tools::check(&file, &opts, nmd, purge);
            if json {
                let nmd_json: Vec<String> = rep
                    .nmd
                    .iter()
                    .map(|(k, m)| {
                        format!(
                            "{{\"kind\":\"{}\",\"message\":\"{}\"}}",
                            tools::json_escape(k),
                            tools::json_escape(m)
                        )
                    })
                    .collect();
                let ph_json: Vec<String> = rep
                    .phantoms
                    .iter()
                    .map(|p| format!("\"{}\"", tools::json_escape(p)))
                    .collect();
                println!(
                    "{{\"file\":\"{}\",\"score\":{},\"letter\":\"{}\",\"notes\":{},\"wobbles\":{},\"fallbacks\":{},\"phantoms\":[{}],\"nmd\":[{}]}}",
                    tools::json_escape(&file),
                    rep.score,
                    rep.letter,
                    rep.notes,
                    rep.wobbles,
                    rep.fallbacks,
                    ph_json.join(","),
                    nmd_json.join(",")
                );
            } else {
                println!(
                    "operon check: {} — score {}/100 (grade {})",
                    file, rep.score, rep.letter
                );
                if rep.wobbles > 0 || rep.fallbacks > 0 {
                    println!(
                        "  repairs: {} wobble, {} fallback",
                        rep.wobbles, rep.fallbacks
                    );
                }
                if !rep.phantoms.is_empty() {
                    println!("  phantom calls: {}", rep.phantoms.join(", "));
                }
                for (k, m) in &rep.nmd {
                    println!("  nmd[{}]: {}", k, m);
                }
            }
            if strict && (rep.wobbles > 0 || rep.fallbacks > 0) {
                std::process::exit(3);
            }
        }
        "test" => {
            let paths: Vec<String> = if positional.is_empty() {
                vec!["tests".to_string()]
            } else {
                positional.clone()
            };
            let rep = tools::run_tests(&paths, &opts, json);
            if json {
                let fails: Vec<String> = rep
                    .failures
                    .iter()
                    .map(|f| format!("\"{}\"", tools::json_escape(f)))
                    .collect();
                println!(
                    "{{\"files\":{},\"proofs\":{},\"passed\":{},\"failed\":{},\"failures\":[{}]}}",
                    rep.files,
                    rep.proofs,
                    rep.passed,
                    rep.failed,
                    fails.join(",")
                );
            }
            if rep.failed > 0 {
                std::process::exit(1);
            }
        }
        "disasm" => {
            // W10: bytecode listing of compiled gene bodies (parse-only)
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("disasm needs a file"),
            };
            match tools::disasm_file(&file, json) {
                Ok(out) => println!("{}", out),
                Err(e) => die(&e),
            }
        }
        "fmt" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("fmt needs a file"),
            };
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            let prog = parser::parse(&src);
            let out = tools::format_program(&prog);
            if write {
                std::fs::write(&file, out).expect("write failed");
                eprintln!("fmt: {} rewritten", file);
            } else {
                print!("{}", out);
            }
        }
        "rna" => {
            // W068 safety mode: default = checked dry-run (writes nothing);
            // --write applies the same engine; exit 1 on any miss so scripts notice.
            if positional.len() < 2 {
                die("rna needs: operon rna <file.op> <patch.rna> [--write] [--json]");
            }
            let file = positional[0].clone();
            let patch_path = positional[1].clone();
            let src = match std::fs::read_to_string(&file) {
                Ok(s) => s,
                Err(e) => die(&format!("rna: cannot read {}: {}", file, e)),
            };
            let patch_src = match std::fs::read_to_string(&patch_path) {
                Ok(s) => s,
                Err(e) => die(&format!("rna: cannot read {}: {}", patch_path, e)),
            };
            let stem = std::path::Path::new(&file)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let report = genes::apply_rna_checked(&src, &patch_src, &stem);
            if json {
                let rows: Vec<String> = report
                    .edits
                    .iter()
                    .map(|e| {
                        format!(
                            "{{\"target\":\"{}\",\"scope\":\"{}\",\"target_found\":{},\"from\":\"{}\",\"to\":\"{}\",\"hits\":{},\"applied\":{}}}",
                            tools::json_escape(&e.target),
                            if e.gene_scoped { "gene" } else { "anywhere" },
                            e.target_found,
                            tools::json_escape(&e.from),
                            tools::json_escape(&e.to),
                            e.hits,
                            e.applied
                        )
                    })
                    .collect();
                println!(
                    "{{\"file\":\"{}\",\"patch\":\"{}\",\"edits\":[{}],\"applied\":{},\"missed\":{},\"would_change\":{}}}",
                    tools::json_escape(&file),
                    tools::json_escape(&patch_path),
                    rows.join(","),
                    report.applied(),
                    report.missed(),
                    report.would_change()
                );
            } else {
                println!(
                    "rna: {} <- {} — {} applied, {} missed{}",
                    file,
                    patch_path,
                    report.applied(),
                    report.missed(),
                    if report.would_change() {
                        ""
                    } else {
                        " (no change)"
                    }
                );
                for e in &report.edits {
                    let scope = if e.gene_scoped { "gene" } else { "anywhere" };
                    if !e.target_found {
                        println!("  MISS [{}] target gene '{}' not found", scope, e.target);
                    } else if e.applied {
                        println!(
                            "  ok   [{}] {} '{}' -> '{}' ({} hit{})",
                            scope,
                            e.target,
                            e.from,
                            e.to,
                            e.hits,
                            if e.hits == 1 { "" } else { "s" }
                        );
                    } else {
                        println!("  MISS [{}] {} '{}' not present", scope, e.target, e.from);
                    }
                }
            }
            if write {
                std::fs::write(&file, &report.new_text).expect("write failed");
                eprintln!("rna: {} rewritten", file);
            }
            if report.missed() > 0 {
                std::process::exit(1);
            }
        }
        "build" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("build needs a file"),
            };
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            let stem = std::path::Path::new(&file)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut src2 = src.clone();
            if let Some(rna_path) = &opts.rna {
                if let Ok(patch) = std::fs::read_to_string(rna_path) {
                    let (s2, _) = genes::apply_rna(&src, &patch, &stem);
                    src2 = s2;
                }
            }
            let mut prog = parser::parse(&src2);
            // bake selected splice variants (drop non-selected bodies)
            if let Some(want) = &opts.variant {
                for s in prog.stmts.iter_mut() {
                    if let Stmt::Splice(sp) = s {
                        let found = sp
                            .variants
                            .iter()
                            .find(|(n, _)| n == want)
                            .map(|(n, d)| (n.clone(), d.clone()));
                        if let Some((vname, d)) = found {
                            let sp_mut = std::sync::Arc::make_mut(sp);
                            sp_mut.variants = vec![(vname, d)];
                        }
                    }
                }
            }
            // strip proof frames from the baked artifact
            prog.stmts
                .retain(|s| !matches!(s, Stmt::Frame { is_proof: true, .. }));
            let out = tools::format_program(&prog);
            let dest = if outfile.is_empty() {
                format!("{}.built.op", stem)
            } else {
                outfile.clone()
            };
            std::fs::write(&dest, out).expect("write failed");
            eprintln!(
                "build: {} → {} (variant: {})",
                file,
                dest,
                opts.variant.as_deref().unwrap_or("default")
            );
        }
        "profile" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("profile needs a file"),
            };
            let l = tools::profile(&file, &opts);
            let mut rows: Vec<(String, u64, f64)> = l
                .interp
                .call_counts
                .iter()
                .map(|(k, c)| {
                    let t = l.interp.call_time_self.get(k).cloned().unwrap_or(0.0);
                    (k.clone(), *c, t)
                })
                .collect();
            rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            // W096: machine-readable profile. Self-describing (units, version,
            // per-gene flags). NOTE: Chrome-trace format is deliberately NOT
            // emitted yet — the profiler records aggregate self-time only;
            // a trace needs per-call spans (interp instrumentation, dev-1
            // lane). Emitting synthetic intervals would misrepresent timing.
            if json {
                let genes_json: Vec<String> = rows
                    .iter()
                    .map(|(name, calls, time)| {
                        let mut flags: Vec<String> = Vec::new();
                        if l.interp.enhanced.contains(name) {
                            flags.push("\"enhanced\"".to_string());
                        }
                        if let Some(Value::Gene(d, _)) = l.interp.global.get(name) {
                            if d.acetylate {
                                flags.push("\"active\"".to_string());
                            }
                            if d.methylate {
                                flags.push("\"repressed\"".to_string());
                            }
                        }
                        format!(
                            "{{\"name\":\"{}\",\"calls\":{},\"self_us\":{:.1},\"flags\":[{}]}}",
                            tools::json_escape(name),
                            calls,
                            time,
                            flags.join(",")
                        )
                    })
                    .collect();
                let total_us: f64 = rows.iter().map(|(_, _, t)| t).sum();
                let total_defined = l.interp.defined_genes.len();
                let mature = l.interp.call_counts.len().min(total_defined);
                println!(
                    "{{\"format\":\"operon-profile\",\"version\":\"{}\",\"file\":\"{}\",\"unit_self_time\":\"microseconds\",\"genes\":[{}],\"total_self_us\":{:.1},\"mature\":{},\"nascent\":{},\"maturation\":{:.2}}}",
                    env!("CARGO_PKG_VERSION"),
                    tools::json_escape(&file),
                    genes_json.join(","),
                    total_us,
                    mature,
                    total_defined.saturating_sub(mature),
                    if total_defined > 0 {
                        mature as f64 / total_defined as f64
                    } else {
                        0.0
                    }
                );
                tools::flush_notes(&l, opts.quiet);
                return;
            }
            println!("operon profile: {} ({} gene(s) executed)", file, rows.len());
            println!("{:<24} {:>8} {:>12}  flags", "gene", "calls", "self µs");
            for (name, calls, time) in &rows {
                let mut flags = String::new();
                if l.interp.enhanced.contains(name) {
                    flags.push_str("enhanced ");
                }
                if let Some(Value::Gene(d, _)) = l.interp.global.get(name) {
                    if d.acetylate {
                        flags.push_str("active ");
                    }
                    if d.methylate {
                        flags.push_str("repressed ");
                    }
                }
                println!(
                    "{:<24} {:>8} {:>12.1}  {}",
                    name,
                    calls,
                    time,
                    flags.trim_end()
                );
            }
            let fp = {
                let total_defined = l.interp.defined_genes.len();
                let mature = l.interp.call_counts.len().min(total_defined);
                let nascent = total_defined.saturating_sub(mature);
                let maturation = if total_defined > 0 {
                    mature as f64 / total_defined as f64
                } else {
                    0.0
                };
                (mature, nascent, maturation)
            };
            println!(
                "telemetry: mature {} · nascent {} · maturation {:.2}",
                fp.0, fp.1, fp.2
            );
            let mut suggestions: Vec<String> = Vec::new();
            let max_calls = l.interp.call_counts.values().copied().max().unwrap_or(0);
            for g in &l.interp.defined_genes {
                let calls = l.interp.call_counts.get(g).copied().unwrap_or(0);
                // only genuinely hot genes (≥10% of the hottest) are candidates
                if calls >= 1
                    && calls * 10 >= max_calls
                    && !l.interp.enhanced.contains(g)
                    && g != "main"
                {
                    suggestions.push(g.clone());
                }
            }
            if !suggestions.is_empty() {
                println!(
                    "enhance candidates (hot but unannotated): {}",
                    suggestions.join(", ")
                );
            }
            tools::flush_notes(&l, opts.quiet);
        }
        "watch" => {
            // W072: re-run on change. v1: mtime polling (200 ms, no external
            // deps) over the entry file + its local (non-std) import tree;
            // each iteration is a fresh `operon run` child, so fuel/caps/
            // interpreter state reset per run — no cross-run contamination.
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("watch needs a file"),
            };
            let passthrough = positional[1..].to_vec();
            fn collect_deps(path: &str, visited: &mut Vec<String>) {
                if visited.len() >= 64 || visited.iter().any(|p| p == path) {
                    return;
                }
                visited.push(path.to_string());
                let src = std::fs::read_to_string(path).unwrap_or_default();
                let prog = parser::parse(&src);
                let base = std::path::Path::new(path).parent().map(|p| p.to_path_buf());
                for s in &prog.stmts {
                    if let Stmt::Use(u, _) = s {
                        let f = u.trim_end_matches(".op").to_string() + ".op";
                        if let Some(b) = &base {
                            let cand = b.join(&f);
                            if cand.is_file() {
                                collect_deps(cand.to_string_lossy().as_ref(), visited);
                            }
                        }
                    }
                }
            }
            let snapshot = |files: &Vec<String>| -> Vec<(String, Option<std::time::SystemTime>)> {
                files
                    .iter()
                    .map(|f| {
                        let m = std::fs::metadata(f).and_then(|m| m.modified()).ok();
                        (f.clone(), m)
                    })
                    .collect()
            };
            let exe =
                std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("operon"));
            let mut iter: usize = 0;
            loop {
                let mut deps: Vec<String> = Vec::new();
                collect_deps(&file, &mut deps);
                let before = snapshot(&deps);
                iter += 1;
                println!(
                    "\n[watch #{}] {} ({} file(s) watched)",
                    iter,
                    file,
                    deps.len()
                );
                let t0 = std::time::Instant::now();
                let status = std::process::Command::new(&exe)
                    .arg("run")
                    .arg(&file)
                    .args(&passthrough)
                    .status();
                let ms = t0.elapsed().as_millis();
                match status {
                    Ok(s) => println!(
                        "[watch #{}] exit={} in {}ms",
                        iter,
                        s.code().unwrap_or(-1),
                        ms
                    ),
                    Err(e) => println!("[watch #{}] spawn failed: {}", iter, e),
                }
                // poll for changes; the dep set itself is recomputed on the
                // next iteration so newly added imports start being watched
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    let after = snapshot(&deps);
                    if after != before {
                        break;
                    }
                }
            }
        }
        "graph" => {
            // W094: static regulate-network export — parse-only (like
            // check/fmt), no run, no capabilities beyond reading the file.
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("graph needs a file"),
            };
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            let prog = parser::parse(&src);
            let g = graph::collect(&prog.stmts);
            if json {
                println!("{}", graph::to_json(&g));
            } else {
                print!("{}", graph::to_dot(&g));
            }
        }
        "crispr" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("crispr needs a file"),
            };
            if matrix {
                // perturbation matrix: knock out EVERY top-level gene in a
                // fresh interpreter, run all proofs, tabulate viability
                let src = std::fs::read_to_string(&file).unwrap_or_default();
                let prog = parser::parse(&src);
                let mut targets: Vec<String> = Vec::new();
                for s in &prog.stmts {
                    if let Stmt::Gene(g) = s {
                        if let Some(n) = &g.name {
                            targets.push(n.clone());
                        }
                    }
                }
                println!("operon crispr matrix: {} target(s) × proofs", targets.len());
                let mut rows: Vec<(String, usize, usize)> = Vec::new();
                for t in &targets {
                    let rep = tools::crispr(&file, &opts, t);
                    rows.push((t.clone(), rep.survivors, rep.proofs_total));
                }
                let essential: Vec<&String> = rows
                    .iter()
                    .filter(|(_, s, t)| *s < *t)
                    .map(|(n, _, _)| n)
                    .collect();
                for (name, survivors, total) in &rows {
                    let tag = if total > survivors {
                        "ESSENTIAL"
                    } else {
                        "dispensable"
                    };
                    println!("  {:<24} {}/{} survived  {}", name, survivors, total, tag);
                }
                if json {
                    let cells: Vec<String> = rows
                        .iter()
                        .map(|(n, s, t)| {
                            format!(
                                "{{\"gene\":\"{}\",\"survivors\":{},\"proofs\":{}}}",
                                tools::json_escape(n),
                                s,
                                t
                            )
                        })
                        .collect();
                    println!("{{\"matrix\":[{}]}}", cells.join(","));
                } else if essential.is_empty() {
                    println!("  no essential genes found (all knockouts viable)");
                }
            } else {
                if knockout.is_empty() {
                    die("crispr needs --knockout <gene> (or --matrix)");
                }
                let rep = tools::crispr(&file, &opts, &knockout);
                if json {
                    let fails: Vec<String> = rep
                        .failures
                        .iter()
                        .map(|f| format!("\"{}\"", tools::json_escape(f)))
                        .collect();
                    println!(
                        "{{\"knockout\":\"{}\",\"proofs\":{},\"survivors\":{},\"failures\":[{}]}}",
                        tools::json_escape(&rep.knockout),
                        rep.proofs_total,
                        rep.survivors,
                        fails.join(",")
                    );
                } else {
                    println!(
                        "operon crispr: knocked out '{}' — {}/{} proof(s) survived",
                        rep.knockout, rep.survivors, rep.proofs_total
                    );
                    for f in &rep.failures {
                        eprintln!("  died: {}", f);
                    }
                }
            }
        }
        "bench" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("bench needs a file"),
            };
            let rep = tools::bench(&file, &opts, iters);
            println!(
                "operon bench: {} × {} iters — min {:.3} ms · avg {:.3} ms",
                file, rep.iters, rep.min_ms, rep.avg_ms
            );
        }
        other => {
            // dx-r3 (re-audit): an unknown subcommand says so — a silent
            // usage dump reads like a broken flag
            die(&format!(
                "unknown command '{}' — run `operon` with no arguments for usage",
                other
            ));
        }
    }
}

// ------------------------------------------------------------ repl
fn repl() {
    use std::io::{BufRead, Write};
    println!(
        "Operon {} repl — gene-expression shell (:help for commands, :quit to leave)",
        env!("CARGO_PKG_VERSION")
    );
    let mut l = match tools::load_file(
        "/dev/null",
        &Opts {
            cell: None,
            variant: None,
            rna: None,
            entry: None,
            use_ires: false,
            frame: None,
            args: Vec::new(),
            quiet: true,
            caps: interp::Caps::default(),
            profile: false,
            stdout_sink: None,
            vm: false,
        },
    ) {
        Ok(l) => l,
        Err(_) => {
            // /dev/null missing (Windows): build an empty Loaded by hand
            tools::Loaded {
                interp: interp::Interp::new(),
                prog: parser::parse(""),
            }
        }
    };
    l.interp.proof_mode = false;
    // sec-r3 (re-audit #9): the REPL shares the run-wide fuel pool too —
    // without it each spawned worker got its own full 200M budget, the
    // spawn-budget multiplication rt_p2e closed for `run` (SPEC §9b)
    l.interp.fuel_pool = Some(std::sync::Arc::new(std::sync::atomic::AtomicI64::new(
        500_000_000,
    )));
    // every executed chunk is kept so `:proof` can replay the session's
    // proof frames against the live interpreter state
    let mut session = String::new();
    let stdin = std::io::stdin();
    let mut buffer = String::new();
    loop {
        {
            let mut out = std::io::stdout();
            if buffer.is_empty() {
                let _ = write!(out, "op> ");
            } else {
                let _ = write!(out, " .. ");
            }
            let _ = out.flush();
        }
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }
        let t = line.trim().to_string();
        if t == ":quit" || t == ":q" || t == "exit" {
            break;
        }
        if let Some(cmd) = t.strip_prefix(':') {
            // commands never join the code buffer
            if buffer.is_empty() {
                let first = cmd.split_whitespace().next().unwrap_or("");
                let arg = cmd[first.len()..].trim();
                match first {
                    "help" | "h" => {
                        println!(
                            ":load f.op   read a file into this session (genes become callable)"
                        );
                        println!(":proof [f]   run proof frames — this session's, or file f's");
                        println!(":genes       list genes defined so far");
                        println!(":vars        list top-level variables");
                        println!(
                            ":symbols     inspect the symbol table (every name the lexer has seen)"
                        );
                        println!(":reset       discard session state and start fresh");
                        println!(":quit        leave the repl (definitions end with the session)");
                    }
                    "load" => {
                        if arg.is_empty() {
                            println!("  usage: :load path/to/file.op");
                        } else {
                            match std::fs::read_to_string(arg) {
                                Ok(src) => {
                                    println!("  loaded {} ({} bytes)", arg, src.len());
                                    session.push_str(&src);
                                    session.push('\n');
                                    repl_eval(&mut l, &src);
                                }
                                Err(e) => println!("  [io] cannot read '{}': {}", arg, e),
                            }
                        }
                    }
                    "proof" => {
                        if arg.is_empty() {
                            repl_proof_session(&mut l, &session);
                        } else {
                            let opts = Opts {
                                cell: None,
                                variant: None,
                                rna: None,
                                entry: None,
                                use_ires: false,
                                frame: None,
                                args: Vec::new(),
                                quiet: true,
                                caps: interp::Caps::default(),
                                profile: false,
                                stdout_sink: None,
                                vm: false,
                            };
                            let rep = tools::run_tests(&[arg.to_string()], &opts, false);
                            println!(
                                "  {}: {}/{} proof(s) passed ({} assertion(s))",
                                arg, rep.passed, rep.proofs, rep.asserts
                            );
                        }
                    }
                    "genes" => {
                        let mut names = l.interp.defined_genes.clone();
                        if names.is_empty() {
                            println!("  (no genes defined yet — try: gene hi() {{ return 1 }})");
                        } else {
                            names.sort();
                            println!("  {}", names.join(", "));
                        }
                    }
                    "symbols" => {
                        // sec-r2: a real consumer of the intern table — the
                        // canonical record of every identifier this session
                        // has lexed, in first-seen order
                        let syms = operon::ffi::symbols();
                        println!("  {} symbol(s) interned this process", syms.len());
                        for s in syms.iter().rev().take(24).collect::<Vec<_>>().iter().rev() {
                            println!("    {}", s);
                        }
                        if syms.len() > 24 {
                            println!("    … {} more", syms.len() - 24);
                        }
                    }
                    "vars" => {
                        let env = l.interp.global.clone();
                        let mut keys: Vec<String> = env
                            .vars
                            .borrow()
                            .keys()
                            .filter(|k| !k.starts_with("__"))
                            .cloned()
                            .collect();
                        if keys.is_empty() {
                            println!("  (no top-level variables yet)");
                        } else {
                            keys.sort();
                            for k in keys {
                                let v = env.get(&k).map(|v| v.repr()).unwrap_or_default();
                                let short = if v.chars().count() > 60 {
                                    format!("{}…", v.chars().take(60).collect::<String>())
                                } else {
                                    v
                                };
                                println!("  {} = {}", k, short);
                            }
                        }
                    }
                    "reset" => {
                        l = match tools::load_file(
                            "/dev/null",
                            &Opts {
                                cell: None,
                                variant: None,
                                rna: None,
                                entry: None,
                                use_ires: false,
                                frame: None,
                                args: Vec::new(),
                                quiet: true,
                                caps: interp::Caps::default(),
                                profile: false,
                                stdout_sink: None,
                                vm: false,
                            },
                        ) {
                            Ok(nl) => nl,
                            Err(_) => tools::Loaded {
                                interp: interp::Interp::new(),
                                prog: parser::parse(""),
                            },
                        };
                        l.interp.proof_mode = false;
                        session.clear();
                        println!("  session reset");
                    }
                    other => println!("  unknown command ':{}' — try :help", other),
                }
                continue;
            }
            // a ':' line while a block is open is treated as code text
        }
        if t.is_empty() && buffer.is_empty() {
            continue;
        }
        if t.is_empty() {
            // execute accumulated block
            let src = std::mem::take(&mut buffer);
            session.push_str(&src);
            session.push('\n');
            repl_eval(&mut l, &src);
            continue;
        }
        buffer.push_str(&line);
        // execute as soon as brace balance closes (single-line defs, exprs);
        // open blocks keep accumulating until the braces close
        if repl_brace_balance(&buffer) <= 0 {
            let src = std::mem::take(&mut buffer);
            session.push_str(&src);
            session.push('\n');
            repl_eval(&mut l, &src);
        }
    }
    // flush any remainder
    if !buffer.is_empty() {
        repl_eval(&mut l, &buffer);
    }
}

// :proof (no file) — run every proof frame the session has defined so far,
// against the live interpreter state, mirroring `operon test` semantics:
// a proof must run to completion AND exercise at least one assertion.
fn repl_proof_session(l: &mut tools::Loaded, session: &str) {
    if session.trim().is_empty() {
        println!("  (empty session — define some code first)");
        return;
    }
    let prog = parser::parse(session);
    if prog.proofs.is_empty() {
        println!(
            "  no proof frames in this session — add one: frame proof {{ assert(1 == 1, \"ok\") }}"
        );
        return;
    }
    let total = prog.proofs.len();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let genv = l.interp.global.clone();
    let proof_was = l.interp.proof_mode;
    l.interp.proof_mode = true;
    for (i, proof) in prog.proofs.iter().enumerate() {
        let before = l.interp.asserts_run;
        match l.interp.exec_block(&genv, proof) {
            Ok(interp::Flow::Norm) => {
                if l.interp.asserts_run == before {
                    println!(
                        "  session proof #{}: FAILED (vacuous — no assertion exercised)",
                        i + 1
                    );
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
            Ok(_) => {
                println!(
                    "  session proof #{}: FAILED (exited early — return/break inside proof)",
                    i + 1
                );
                failed += 1;
            }
            Err(s) if s.prop.is_some() => {
                println!(
                    "  session proof #{}: FAILED (propagation left the proof frame: {})",
                    i + 1,
                    s.prop.unwrap().repr()
                );
                failed += 1;
            }
            Err(s) => {
                println!("  session proof #{}: FAILED ({})", i + 1, s.message);
                failed += 1;
            }
        }
    }
    l.interp.proof_mode = proof_was;
    println!(
        "  session proof: {}/{} passed ({} failed)",
        passed, total, failed
    );
}

fn repl_brace_balance(s: &str) -> i32 {
    let mut bal = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for c in s.chars() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => bal += 1,
            '}' => bal -= 1,
            _ => {}
        }
    }
    bal
}

/// dx-r1 (audit W4): print only the notes this REPL input produced —
/// previously notes were recorded but never shown, so the wobble/phantom
/// diagnostics that are the language's brand were invisible interactively.
fn repl_flush_new_notes(l: &tools::Loaded, start: usize) {
    for n in &l.interp.notes[start..] {
        let tag = match n.rung {
            1 => "info",
            2 => "synonym",
            3 => "wobble",
            _ => "fallback",
        };
        // A13 (dx-r2): real locations when the note carries a line
        if n.line > 0 {
            println!("  [{}] {}:{}: {}", tag, l.interp.file, n.line, n.message);
        } else {
            println!("  [{}] {}", tag, n.message);
        }
    }
}

fn repl_eval(l: &mut tools::Loaded, src: &str) {
    let note_start = l.interp.notes.len();
    let prog = parser::parse(src);
    let broken = prog.notes.iter().any(|n| n.rung >= 4) || prog.stmts.is_empty();
    if !broken {
        let env = l.interp.global.clone();
        for stmt in &prog.stmts {
            if let Stmt::ExprStmt(e) = stmt {
                match l.interp.eval(&env, e) {
                    Ok(v) => {
                        if !matches!(v, Value::Null) {
                            println!("{}", v.repr());
                        }
                    }
                    // W06 (D-014): propagation with no enclosing gene in the
                    // REPL — the variant value passes through, tagged honestly.
                    Err(st) if st.prop.is_some() => {
                        println!("  [propagate] {} passes through", st.prop.unwrap().repr())
                    }
                    Err(st) => println!("  [{}] {}", st.kind, st.message),
                }
            } else {
                if let Err(st) = l.interp.exec_stmt(&env, stmt) {
                    // W06 (D-014): REPL top-level propagation — value passes
                    // through, never a leaked bare kind.
                    if let Some(v) = st.prop {
                        println!("  [propagate] {} passes through", v.repr());
                    } else {
                        println!("  [{}] {}", st.kind, st.message);
                    }
                }
            }
        }
        repl_flush_new_notes(l, note_start);
        return;
    }
    // expression mode: a bare `1 + 2 * 3` is not a statement — evaluate it
    // by assignment-to-scratch and print the bound value
    let wrapped = format!("__repl_val = ({})", src.trim().trim_end_matches(';'));
    let wprog = parser::parse(&wrapped);
    if wprog.notes.iter().any(|n| n.rung >= 4) {
        for n in &prog.notes {
            println!("  [note] {}", n.message);
        }
        return;
    }
    let env = l.interp.global.clone();
    let note_start = l.interp.notes.len();
    for stmt in &wprog.stmts {
        if let Err(st) = l.interp.exec_stmt(&env, stmt) {
            // W06 (D-014): REPL expression-mode propagation — same
            // passes-through contract as statement mode.
            if let Some(v) = st.prop {
                println!("  [propagate] {} passes through", v.repr());
            } else {
                println!("  [{}] {}", st.kind, st.message);
            }
        }
    }
    // dx-r1 (parity audit W4): notes were recorded but never shown — the
    // REPL swallowed the wobble/phantom notes that are the language's brand
    // ("never leaves you guessing"). Print only the notes this input
    // produced (tools::flush_notes would replay the whole session).
    repl_flush_new_notes(l, note_start);
    if let Some(v) = env.get("__repl_val") {
        if !matches!(v, Value::Null) {
            println!("{}", v.repr());
        }
    }
}

fn usage() {
    eprintln!(
        "Operon {} — the gene-expression language (Total Grammar)
usage:
  operon run f.op [--entry g] [--variant v] [--cell c] [--rna r] [--frame name] [--ires] [--strict] [--quiet]
                  [--fuel steps] [--allow-read path] [--allow-write path] [--allow-net host:port]
                  [--allow-run cmd] [--allow-py module] [--allow-exit] [--allow-env var] [--allow-all]
  operon check f.op [--nmd | --nmd=purge] [--json]
  operon test [paths...] [--json]
  operon fmt f.op [--write]
  operon repl
  operon build f.op [--variant v] [-o out.op]
  operon rna f.op patch.rna [--write] [--json]
  operon graph f.op [--json]
  operon watch f.op [args...]
  operon profile f.op
  operon disasm f.op [--json]     W10: bytecode listing of compiled genes
  operon crispr f.op --knockout gene [--json]
  operon bench f.op [--iters n]
  operon run f.op --vm            A2/A3: bytecode VM on gene bodies
  operon version",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2);
}
