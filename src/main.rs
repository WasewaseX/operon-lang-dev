//! main.rs — Operon toolchain CLI (Rust core).
//! run | check | test | fmt | build | profile | crispr | bench | version

// The language core lives in the `operon` library crate (src/lib.rs);
// this binary is the CLI shell over it.
use operon::genes;
use operon::interp;
use operon::parser;
use operon::tools;

use operon::die;
use operon::tools::Opts;
use operon::ast::Stmt;
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
    };
    let mut json = false;
    let mut strict = false;
    let mut nmd = false;
    let mut purge = false;
    let mut write = false;
    let mut matrix = false;
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
                if let Some(p) = rest.get(i).and_then(|s| s.parse::<u64>().ok()) {
                    fuel = Some(p);
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
                iters = rest.get(i).and_then(|s| s.parse().ok()).unwrap_or(20);
            }
            "-o" | "--out" => {
                i += 1;
                outfile = rest.get(i).cloned().unwrap_or_default();
            }
            "--ires" => opts.use_ires = true,
            "--json" => json = true,
            "--strict" => strict = true,
            "--quiet" => opts.quiet = true,
            "--nmd" => nmd = true,
            "--nmd=purge" | "--purge" => {
                nmd = true;
                purge = true;
            }
            "--write" => write = true,
            _ => positional.push(a),
        }
        i += 1;
    }

    match cmd.as_str() {
        "version" => {
            println!("Operon 2.2.0 (rust-core, c-runtime, cpp-kernel)");
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
                        eprintln!("[contained] [{}] {}", s.kind, s.message);
                    }
                }
            }
            tools::flush_notes(&l, opts.quiet);
            let strict_cell = l.interp.cell.get("wobble.strict").map(|v| v == "true").unwrap_or(false);
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
                    .map(|(k, m)| format!("{{\"kind\":\"{}\",\"message\":\"{}\"}}", tools::json_escape(k), tools::json_escape(m)))
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
                println!("operon check: {} — score {}/100 (grade {})", file, rep.score, rep.letter);
                if rep.wobbles > 0 || rep.fallbacks > 0 {
                    println!("  repairs: {} wobble, {} fallback", rep.wobbles, rep.fallbacks);
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
                        let found = sp.variants.iter().find(|(n, _)| n == want).map(|(n, d)| (n.clone(), d.clone()));
                        if let Some((vname, d)) = found {
                            let sp_mut = std::sync::Arc::make_mut(sp);
                            sp_mut.variants = vec![(vname, d)];
                        }
                    }
                }
            }
            // strip proof frames from the baked artifact
            prog.stmts.retain(|s| !matches!(s, Stmt::Frame { is_proof: true, .. }));
            let out = tools::format_program(&prog);
            let dest = if outfile.is_empty() {
                format!("{}.built.op", stem)
            } else {
                outfile.clone()
            };
            std::fs::write(&dest, out).expect("write failed");
            eprintln!("build: {} → {} (variant: {})", file, dest, opts.variant.as_deref().unwrap_or("default"));
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
                println!("{:<24} {:>8} {:>12.1}  {}", name, calls, time, flags.trim_end());
            }
            let fp = {
                let total_defined = l.interp.defined_genes.len();
                let mature = l.interp.call_counts.len().min(total_defined);
                let nascent = total_defined.saturating_sub(mature);
                let maturation = if total_defined > 0 { mature as f64 / total_defined as f64 } else { 0.0 };
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
                if calls >= 1 && calls * 10 >= max_calls
                    && !l.interp.enhanced.contains(g)
                    && g != "main"
                {
                    suggestions.push(g.clone());
                }
            }
            if !suggestions.is_empty() {
                println!("enhance candidates (hot but unannotated): {}", suggestions.join(", "));
            }
            tools::flush_notes(&l, opts.quiet);
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
                let essential: Vec<&String> = rows.iter().filter(|(_, s, t)| *s < *t).map(|(n, _, _)| n).collect();
                for (name, survivors, total) in &rows {
                    let tag = if total > survivors { "ESSENTIAL" } else { "dispensable" };
                    println!("  {:<24} {}/{} survived  {}", name, survivors, total, tag);
                }
                if json {
                    let cells: Vec<String> = rows
                        .iter()
                        .map(|(n, s, t)| format!("{{\"gene\":\"{}\",\"survivors\":{},\"proofs\":{}}}", tools::json_escape(n), s, t))
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
        _ => usage(),
    }
}

// ------------------------------------------------------------ repl
fn repl() {
    use std::io::{BufRead, Write};
    println!("Operon 2.2.0 repl — gene-expression shell (type :quit to leave)");
    let mut l = match tools::load_file("/dev/null", &Opts {
        cell: None, variant: None, rna: None, entry: None, use_ires: false,
        frame: None, args: Vec::new(), quiet: true, caps: interp::Caps::default(),
    }) {
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
        if t.is_empty() && buffer.is_empty() {
            continue;
        }
        if t.is_empty() {
            // execute accumulated block
            let src = std::mem::take(&mut buffer);
            repl_eval(&mut l, &src);
            continue;
        }
        buffer.push_str(&line);
        // execute as soon as brace balance closes (single-line defs, exprs);
        // open blocks keep accumulating until the braces close
        if repl_brace_balance(&buffer) <= 0 {
            let src = std::mem::take(&mut buffer);
            repl_eval(&mut l, &src);
        }
    }
    // flush any remainder
    if !buffer.is_empty() {
        repl_eval(&mut l, &buffer);
    }
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

fn repl_eval(l: &mut tools::Loaded, src: &str) {
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
                    Err(st) => println!("  [{}] {}", st.kind, st.message),
                }
            } else {
                if let Err(st) = l.interp.exec_stmt(&env, stmt) {
                    println!("  [{}] {}", st.kind, st.message);
                }
            }
        }
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
    for stmt in &wprog.stmts {
        if let Err(st) = l.interp.exec_stmt(&env, stmt) {
            println!("  [{}] {}", st.kind, st.message);
        }
    }
    if let Some(v) = env.get("__repl_val") {
        if !matches!(v, Value::Null) {
            println!("{}", v.repr());
        }
    }
}

fn usage() {
    eprintln!(
        "Operon 2.2.0 — the gene-expression language (Total Grammar)
usage:
  operon run f.op [--entry g] [--variant v] [--cell c] [--rna r] [--frame name] [--ires] [--strict] [--quiet]
  operon check f.op [--nmd | --nmd=purge] [--json]
  operon test [paths...] [--json]
  operon fmt f.op [--write]
  operon repl
  operon build f.op [--variant v] [-o out.op]
  operon profile f.op
  operon crispr f.op --knockout gene [--json]
  operon bench f.op [--iters n]
  operon version"
    );
    std::process::exit(2);
}
