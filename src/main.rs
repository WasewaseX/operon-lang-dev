//! main.rs, Operon toolchain CLI (Rust core).
//! run | check | test | fmt | build | profile | crispr | bench | version

// The language core lives in the `operon` library crate (src/lib.rs);
// this binary is the CLI shell over it.
use operon::genes;
use operon::graph;
use operon::interp;
use operon::parser;
use operon::pkg;
use operon::rna2;
use operon::tools;
use operon::vm;
// W101 slice 3: IsTerminal drives the auto color policy for diagnostics
use std::io::IsTerminal;

use operon::ast::Stmt;
use operon::die;
use operon::tools::Opts;
use operon::value::Value;

/// ai/ecosystem-r3 hardening: die the Unix way when stdout closes early.
/// A CLI whose consumer exits first (`operon add x | grep -q ...`,
/// `operon run f.op | head -1`) used to panic in println! (exit 101, a
/// scary backtrace for a normal `head`); with SIGPIPE at SIG_DFL the
/// process dies the way cat/grep do — silently, 141 under a shell.
/// Zero crates: `signal(2)` is declared through the C ABI that the Rust
/// standard library already links on every Unix target (SIGPIPE = 13,
/// SIG_DFL = 0 on Linux and macOS). Windows has no SIGPIPE; its console
/// piping semantics differ and are left untouched.
/// The differential/proof harnesses always read full output, so nothing
/// pinned changes.
#[cfg(unix)]
fn reset_sigpipe_to_default() {
    unsafe {
        extern "C" {
            fn signal(signum: i32, handler: usize) -> usize;
        }
        signal(13, 0);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe_to_default() {}

fn main() {
    reset_sigpipe_to_default();
    operon::w009a::init_from_env();
    let _w009a_guard = operon::w009a::Guard;
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
            eprintln!("[fatal] internal toolchain panic, this input crashed the runtime");
            // ast-grep-ignore: no-std-process-exit-in-core
            std::process::exit(101);
        }
    }
}

fn real_main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        usage();
        // ast-grep-ignore: no-std-process-exit-in-core
        std::process::exit(2);
    }
    let cmd = argv[0].clone();
    let rest = &argv[1..];

    // dx-r5 (audit D-4): rustc/go/tsc answer --help; operon used to say
    // "unknown command". Same usage as the no-args case, exit 0.
    if cmd == "--help" || cmd == "-h" || cmd == "help" {
        usage();
        // ast-grep-ignore: no-std-process-exit-in-core
        std::process::exit(0);
    }

    // W19/W20: the package-manager command group owns its own flag
    // vocabulary (add --as/--rev, etc.), dispatch before generic parsing.
    if cmd == "mod" {
        pkg::mod_command(rest);
    }
    // W20-r1: top-level package commands. The `mod` group stays the
    // explicit spelling; these are the day-one verbs developers expect
    // (operon new myapp, operon add http, operon publish ...).
    if cmd == "new" {
        pkg::new_command(rest);
    }
    if cmd == "registry" {
        pkg::registry_command(rest);
    }
    match cmd.as_str() {
        "init" | "add" | "remove" | "update" | "install" | "tree" | "verify" | "publish"
        | "search" => {
            let mut full: Vec<String> = Vec::with_capacity(rest.len() + 1);
            full.push(cmd.clone());
            full.extend_from_slice(rest);
            pkg::mod_command(&full);
        }
        _ => {}
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
        spans: false,
        stdout_sink: None,
        use_vm: true,
    };
    let mut json = false;
    // W097-A: `--chrome <file>` — write the profiled run's per-call spans
    // as a Chrome Trace Format JSON file (about://tracing / Perfetto).
    let mut chrome: Option<String> = None;
    let mut strict = false;
    // W23: --locked, fail on operon.toml <-> operon.lock drift (CI pin)
    let mut locked = false;
    let mut nmd = false;
    let mut purge = false;
    let mut write = false;
    let mut allow_comment_drop = false;
    // W086 build-contract honesty: the flags EXIST so the CLI cannot silently
    // promise what build does not do; both refuse with their design pointer.
    let mut build_bundle = false;
    let mut build_native = false;
    // W68: `operon rna --check` validates a patch and writes NOTHING, ever
    let mut rna_check = false;
    let mut trace_grn_path: Option<String> = None;
    // W101: --json-errors, fatal diagnostics as one JSON object (SPEC 9a).
    let mut json_errors = false;
    // W09 A2/A6: run gene bodies through the OIR1 bytecode machine
    // (src/vm.rs); calls/gates stay on the shared path, so output is
    // byte-identical. DEFAULT ON since the A6 flip (v2.6.0 release
    // boundary, D-009); --no-vm opts back to the tree-walk for the
    // differential harness and debugging. The vm lane parity (219/219)
    // and the differential harness hold for both engines.
    let mut use_vm = true;
    // W08 phase 1: `operon debug` break lines (--break N, repeatable)
    #[allow(unused_assignments)]
    let mut debug_mode = false;
    let mut debug_breaks: Vec<usize> = Vec::new();
    // W08r stage 2: `debug --protocol=json` — the NDJSON machine protocol
    let mut debug_protocol_mode = false;
    // W11: the optimization level (0 = off); --opt-passes overrides with
    // an explicit per-pass set (the W011 toggle matrix)
    let mut opt_level: u8 = 0;
    let mut opt_passes: Option<operon::vm::PassSet> = None;
    let mut matrix = false;
    // dx-r6: true after the `--` separator, remaining args are program argv
    let mut passthrough = false;
    let mut fuel: Option<u64> = None;
    let mut knockout = String::new();
    let mut iters = 20usize;
    let mut outfile = String::new();
    let mut positional: Vec<String> = Vec::new();
    // W49 (ROADMAP-100): test-runner filtering/determinism knobs
    let mut test_filter: Option<String> = None;
    let mut list_only = false;
    let mut repeat = 1usize;
    // W41: check output format, "diag" is the default, "score" is the
    // transitional escape (--format score) for anything that still wants the
    // school-grade banner; nothing in scripts/ or CI parses the grade.
    let mut check_format = String::from("diag");
    // TYPED-MODE: static type checking gate. `--typed` runs the type-check
    // pass before execution (run) or folds T-series findings into the check
    // stream (check). Additive: without the flag, nothing changes.
    let mut typed = false;
    // W48: lint-side CLI allow list (--allow rule1,rule2, repeatable);
    // check-side --style inlines the lint-owned style stream in diag output.
    let mut lint_allows: Vec<String> = Vec::new();
    let mut check_style = false;
    // W47 (ROADMAP-100): formatter configuration, file first, flags override
    let mut fmt_indent: Option<usize> = None;
    let mut fmt_quotes: Option<tools::QuoteMode> = None;
    let mut fmt_width: Option<usize> = None; // W47-v2: 0 = explicitly off
    let mut fmt_config_path: Option<String> = None;

    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].clone();
        // dx-r6 STRICT (ytdl-app audit): once `--` is seen, EVERY remaining
        // arg belongs to the program verbatim — including names the host
        // also defines (--out, --json, --cell, ...). The arms below used to
        // keep matching after the separator, so `operon run app.op -- get
        // URL --out D` silently lost `--out D` to the host's build flag. A
        // second `--` is a literal program argument (POSIX).
        if passthrough {
            positional.push(a);
            i += 1;
            continue;
        }
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
                // substrate-r1: Python bridge grant, exact-match per module
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
            // W48: file-wide lint suppression, `--allow unused-gene,dead-const`
            // (repeatable). Rule names or stable codes. Line-local suppression
            // stays with the '// allow:' comment mechanism.
            "--allow" => {
                i += 1;
                match rest.get(i) {
                    Some(list) => {
                        for r in list.split(',') {
                            let r = r.trim();
                            if !r.is_empty() {
                                lint_allows.push(r.to_string());
                            }
                        }
                    }
                    None => die("--allow needs a rule list (e.g. --allow unused-gene)"),
                }
            }
            // W48: inline the lint-owned style stream in check's diag output
            // (default: a labeled count + pointer, the split is about command
            // purpose and defaults, not about hiding data)
            "--style" => check_style = true,
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
            // W097-A: Chrome Trace Format export for `operon profile`
            // (closes W096's blocked REMAIN — per-call spans). Needs a
            // file path; implies span capture for the profiled run.
            "--chrome" => {
                i += 1;
                match rest.get(i).map(|s| s.as_str()) {
                    Some(p) if !p.is_empty() => {
                        chrome = Some(p.to_string());
                        // span capture implied: the trace needs the timeline,
                        // and it must be armed BEFORE load (dx-r1: top-level
                        // calls execute during load_file)
                        opts.spans = true;
                    }
                    _ => die("--chrome needs a file path argument (e.g. --chrome trace.json)"),
                }
            }
            "--strict" => strict = true,
            "--typed" => typed = true,
            "--locked" => locked = true,
            "--quiet" => opts.quiet = true,
            "--nmd" => nmd = true,
            "--nmd=purge" | "--purge" => {
                nmd = true;
                purge = true;
            }
            "--write" => write = true,
            // W086: honest refusal flags for the build subcommand (W87/W85).
            "--bundle" => build_bundle = true,
            "--native" => build_native = true,
            // W68: rna validation mode (no application, no file mutation)
            "--check" => rna_check = true,
            // W067 v2: the AST reprint drops plain `#` comments; the v2 rna
            // engine refuses such files unless this flag is passed.
            "--allow-comment-drop" => allow_comment_drop = true,
            // W101: machine-readable fatal diagnostics (SPEC 9a schema), one
            // JSON object on stderr instead of the rendered text block.
            "--json-errors" => json_errors = true,
            // W095: GRN tick-stream, every engine update point (fire pulse
            // / decay tick) snapshots the sorted level map as one JSONL
            // frame; the buffer lands in the file after the run.
            "--trace-grn" => {
                i += 1;
                let p = rest.get(i).cloned();
                if p.as_deref().map(str::is_empty).unwrap_or(true) {
                    die("--trace-grn needs a file path");
                }
                trace_grn_path = p;
            }
            // W09 A2: the bytecode lane (same semantics, machine-executed);
            // A6: it is the DEFAULT, the flag remains for explicitness
            "--vm" => {
                use_vm = true;
                opts.use_vm = true;
            }
            // W09 A6: opt back to the tree-walking interpreter (--interp is
            // the design-contract name, docs/vm-design.md §9; --no-vm alias)
            "--interp" | "--no-vm" => {
                use_vm = false;
                opts.use_vm = false;
            }
            // W11: the optimization pipeline level
            "--opt" => {
                i += 1;
                let n = rest
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--opt needs a level (0..=2)"));
                opt_level = n
                    .parse()
                    .unwrap_or_else(|_| die("--opt needs a level (0..=2)"));
            }
            // W011 toggle matrix: run EXACTLY these passes, e.g.
            // --opt-passes fold,dce (valid: fold, thread, dce, none, all)
            "--opt-passes" => {
                i += 1;
                let spec = rest.get(i).cloned().unwrap_or_else(|| {
                    die("--opt-passes needs a list (fold,thread,dce | none | all)")
                });
                opt_passes = Some(operon::vm::PassSet::parse(&spec).unwrap_or_else(|e| die(&e)));
            }
            // W08 phase 1: a line breakpoint for `operon debug`
            "--break" => {
                i += 1;
                let n = rest
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--break needs a line number"));
                let n: usize = n
                    .parse()
                    .unwrap_or_else(|_| die("--break needs a line number"));
                debug_breaks.push(n);
            }
            // W08r stage 2: machine protocol mode — `--protocol=json` serves
            // the debugger state as NDJSON on stdout (the human REPL stays
            // the default; program print output moves to stderr)
            other if other == "--protocol" || other.starts_with("--protocol=") => {
                let p = if other.starts_with("--protocol=") {
                    other.trim_start_matches("--protocol=").to_string()
                } else {
                    i += 1;
                    rest.get(i)
                        .cloned()
                        .unwrap_or_else(|| die("--protocol needs a kind (json)"))
                };
                if p != "json" {
                    die("--protocol: unknown kind (only json is supported)");
                }
                debug_protocol_mode = true;
            }
            "--filter" => {
                i += 1;
                test_filter = Some(
                    rest.get(i)
                        .cloned()
                        .unwrap_or_else(|| die("--filter needs a substring argument")),
                );
            }
            "--list" => list_only = true,
            "--repeat" => {
                i += 1;
                // W49: 1..=1000, a flake-hunt re-run cap, not an unbounded loop.
                match rest.get(i).map(|s| s.parse::<usize>()) {
                    Some(Ok(n)) if (1..=1000).contains(&n) => repeat = n,
                    _ => die("--repeat needs a number 1..=1000 (flake-hunt re-runs)"),
                }
            }
            "--format" => {
                i += 1;
                match rest.get(i).cloned() {
                    Some(f) if f == "diag" || f == "score" => check_format = f,
                    _ => die("--format needs 'diag' or 'score'"),
                }
            }
            // W47: formatter knobs (fmt arm). Precedence: flags > config file > defaults.
            "--indent" => {
                i += 1;
                match rest.get(i).map(|s| s.parse::<usize>()) {
                    Some(Ok(n)) if (1..=16).contains(&n) => fmt_indent = Some(n),
                    _ => die("--indent needs a number 1..=16 (e.g. --indent 4)"),
                }
            }
            "--quotes" => {
                i += 1;
                match rest.get(i).cloned().as_deref() {
                    Some("single") => fmt_quotes = Some(tools::QuoteMode::Single),
                    Some("double") => fmt_quotes = Some(tools::QuoteMode::Double),
                    _ => die("--quotes needs 'single' or 'double'"),
                }
            }
            "--fmt-config" => {
                i += 1;
                fmt_config_path = Some(
                    rest.get(i)
                        .cloned()
                        .unwrap_or_else(|| die("--fmt-config needs a file path")),
                );
            }
            // W47-v2: soft line width. 0 = explicitly off (overrides the
            // config file); absent = off. N in 1..=10000.
            "--width" => {
                i += 1;
                match rest.get(i).map(|s| s.parse::<usize>()) {
                    Some(Ok(n)) if n <= 10_000 => fmt_width = Some(n),
                    _ => die("--width needs a number 0..=10000 (0 = off, e.g. --width 80)"),
                }
            }
            "--" => {
                // dx-r6 (loop-5-a audit MED): POSIX `--` separator, everything
                // after it belongs to the PROGRAM, not the host CLI. Without
                // it, `operon run app.op --key value` died with "unknown flag
                // '--key'" and half of std/args.op's documented conventions
                // were unreachable from a real command line. Unknown --flags
                // BEFORE the separator still fail loudly (dx-r1 typo guard).
                passthrough = true;
            }
            _ => {
                // dx-r1 (parity audit W6): unknown flags silently became
                // program argv, `operon run f.op --strick` ran with a typo'd
                // flag and no warning. Fail loudly instead.
                if a.starts_with("--") && !passthrough {
                    die(&format!(
                        "unknown flag '{}', run `operon` with no arguments for usage",
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
                "Operon {}-vm (rust-core, cpp-kernel)",
                env!("CARGO_PKG_VERSION")
            );
        }
        "version" => {
            println!(
                "Operon {}-vm (rust-core, cpp-kernel)",
                env!("CARGO_PKG_VERSION")
            );
        }
        "repl" => {
            repl();
        }
        // W08r stage 3: the Debug Adapter Protocol adapter. `operon dap f.op`
        // speaks DAP on stdio (Content-Length framing) so editors debug
        // Operon programs natively — see src/dap.rs and editors/vscode/.
        "dap" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("dap needs a file"),
            };
            opts.args = positional[1..].to_vec();
            let mut l = match tools::load_file(&file, &opts) {
                Ok(l) => l,
                Err(e) => die(&e),
            };
            if let Some(f) = fuel {
                l.interp.step_budget = f;
            }
            let total = fuel.unwrap_or(500_000_000);
            l.interp.fuel_pool = Some(std::sync::Arc::new(std::sync::atomic::AtomicI64::new(
                total as i64,
            )));
            l.interp.debug_file = file.clone();
            l.interp.debug_dap = true;
            // the debuggee's print output is captured and forwarded as DAP
            // output events; the adapter's stdout is the protocol transport
            l.interp.stdout_sink = Some(std::rc::Rc::new(std::cell::RefCell::new(Vec::new())));
            // pre-run configuration: initialize/launch/setBreakpoints/
            // configurationDone (breakpoints land in the interp's set)
            operon::dap::configure(&mut l.interp);
            let result = tools::run_entry(&mut l, &opts);
            // final output events, then the lifecycle close
            operon::dap::drain_sink_to_events(&mut l.interp);
            operon::dap::session_end(0);
            match result {
                Ok(_) => {
                    // ast-grep-ignore: no-std-process-exit-in-core
                    std::process::exit(0);
                }
                Err(s) => {
                    eprintln!("[dap] program raised [{}] {}", s.kind, s.message);
                    // ast-grep-ignore: no-std-process-exit-in-core
                    std::process::exit(1);
                }
            }
        }
        // W39 (ROADMAP-100): AST dump, the Total Grammar structural window.
        // Purely structural: repair notes are W38 `explain`'s surface and are
        // never printed here, default or otherwise.
        "ast" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("ast needs a file"),
            };
            let src = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| die(&format!("cannot read {}: {}", file, e)));
            let prog = parser::parse(&src);
            // W39 + fuzz finding (2026-09-26): the pretty tree grows
            // quadratically with nesting depth (indent x depth), so a
            // pathological-but-parseable input (thousands of `(`) turns `ast`
            // into a hang. check() parses the same file in milliseconds, the
            // parser is fine, the PRINTER is the problem. Guard: measure
            // source nesting depth; beyond 400 print the compact one-line
            // tree (same structure, no indentation).
            let depth = src
                .bytes()
                .fold((0usize, 0usize), |(d, m), b| match b {
                    b'(' | b'{' | b'[' => {
                        let d = d + 1;
                        (d, m.max(d))
                    }
                    b')' | b'}' | b']' => (d.saturating_sub(1), m),
                    _ => (d, m),
                })
                .1;
            let pretty = depth <= 400;
            if json {
                // W39.2: structural JSON, the same tree as the text form.
                println!(
                    "{{\"file\":\"{}\",\"format\":\"ast-json-v1\",\"stmts\":{},\"notes\":{},\"ast\":{}}}",
                    tools::json_escape(&file),
                    prog.stmts.len(),
                    prog.notes.len(),
                    tools::ast_dump_json(&prog)
                );
            } else if pretty {
                println!("{}", tools::ast_dump(&prog, true));
            } else {
                eprintln!(
                    "[ast] nesting depth {} exceeds 400, compact dump (pretty dump is quadratic on deep trees)",
                    depth
                );
                println!("{}", tools::ast_dump(&prog, false));
            }
        }
        // W38 (ROADMAP-100): explain, what did Total Grammar do to my file?
        "explain" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("explain needs a file"),
            };
            let src = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| die(&format!("cannot read {}: {}", file, e)));
            let prog = parser::parse(&src);
            let rung_name = |r: u8| {
                if r == 1 {
                    "canonical"
                } else if r == 2 {
                    "synonym"
                } else if r == 3 {
                    "wobble"
                } else {
                    "fallback"
                }
            };
            if json {
                let notes: Vec<String> = prog
                    .notes
                    .iter()
                    .map(|n| {
                        format!(
                            "{{\"line\":{},\"rung\":{},\"rung_name\":\"{}\",\"code\":{},\"message\":\"{}\"}}",
                            n.line,
                            n.rung,
                            rung_name(n.rung),
                            tools::json_escape(operon::diag::note_code(&n.message)),
                            tools::json_escape(&n.message)
                        )
                    })
                    .collect();
                println!(
                    "{{\"file\":\"{}\",\"notes\":[{}],\"canonical\":{},\"synonym\":{},\"wobble\":{},\"fallback\":{}}}",
                    tools::json_escape(&file),
                    notes.join(","),
                    prog.notes.iter().filter(|n| n.rung == 1).count(),
                    prog.notes.iter().filter(|n| n.rung == 2).count(),
                    prog.notes.iter().filter(|n| n.rung == 3).count(),
                    prog.notes.iter().filter(|n| n.rung >= 4).count()
                );
            } else {
                println!("Total Grammar report for {}:", file);
                if prog.notes.is_empty() {
                    println!("  canonical, no repairs, no recoveries");
                }
                for n in &prog.notes {
                    let code = operon::diag::note_code(&n.message);
                    if code.is_empty() {
                        println!("  [{}] line {}: {}", rung_name(n.rung), n.line, n.message);
                    } else {
                        println!(
                            "  [{} {}] line {}: {}",
                            rung_name(n.rung),
                            code,
                            n.line,
                            n.message
                        );
                    }
                }
                let w = prog.notes.iter().filter(|n| n.rung == 3).count();
                let fb = prog.notes.iter().filter(|n| n.rung >= 4).count();
                if strict && (w > 0 || fb > 0) {
                    println!(
                        "  --strict verdict: FAIL ({} wobble(s), {} fallback(s))",
                        w, fb
                    );
                } else {
                    println!("  --strict verdict: PASS");
                }
            }
        }
        // W48 (ROADMAP-100): the style/quality front door. `operon lint` runs
        // ONLY the lint stream of the shared rule engine (see src/lint.rs for
        // the ownership table); correctness lives in `operon check`. Flags:
        //   --strict  any finding = exit 3 (CI gate policy, W37 escalation)
        //   --allow   file-wide CLI suppression on top of '// allow:' comments
        //   --json    {code,severity,line,rule,message} findings array
        "lint" => {
            if positional.is_empty() {
                die("lint needs at least one file");
            }
            if let Some(f) = positional.iter().find(|p| std::path::Path::new(p).is_dir()) {
                die(&format!(
                    "lint: '{}' is a directory, pass .op files (or run `operon lint` per file)",
                    f
                ));
            }
            let mut results: Vec<(String, Vec<operon::lint::Finding>)> = Vec::new();
            for file in &positional {
                let src = std::fs::read_to_string(file)
                    .unwrap_or_else(|e| die(&format!("cannot read {}: {}", file, e)));
                let prog = parser::parse(&src);
                let mut findings = operon::lint::lint_style(&prog);
                // W66: validate a .cell payload against the schema on request;
                // cell-schema rules are check-owned, they surface here only
                // because the payload was explicitly handed to the linter
                if let Some(cell) = opts.cell.clone() {
                    match std::fs::read_to_string(&cell) {
                        Ok(cs) => findings.extend(operon::lint::lint_cell(&cs)),
                        Err(e) => die(&format!("cannot read {}: {}", cell, e)),
                    }
                }
                // line-local comment suppression, then the file-wide --allow list
                operon::lint::apply_allows(&mut findings, &src);
                operon::lint::apply_cli_allows(&mut findings, &lint_allows);
                results.push((file.clone(), findings));
            }
            let total: usize = results.iter().map(|(_, f)| f.len()).sum();
            if json {
                let file_json = |file: &str, fs: &[operon::lint::Finding]| {
                    let items: Vec<String> = fs
                        .iter()
                        .map(|f| {
                            format!(
                                "{{\"code\":\"{}\",\"severity\":\"{}\",\"line\":{},\"rule\":\"{}\",\"message\":\"{}\"}}",
                                f.code,
                                f.sev.name(),
                                f.line,
                                f.rule,
                                tools::json_escape(&f.message)
                            )
                        })
                        .collect();
                    format!(
                        "{{\"file\":\"{}\",\"findings\":[{}]}}",
                        tools::json_escape(file),
                        items.join(",")
                    )
                };
                if results.len() == 1 {
                    // back-compat single-file shape (now with stable codes)
                    println!("{}", file_json(&results[0].0, &results[0].1));
                } else {
                    let parts: Vec<String> =
                        results.iter().map(|(f, fs)| file_json(f, fs)).collect();
                    println!("{{\"results\":[{}],\"total\":{}}}", parts.join(","), total);
                }
            } else {
                for (file, fs) in &results {
                    if fs.is_empty() {
                        println!("lint: {}, clean", file);
                    } else {
                        let fsrc = std::fs::read_to_string(file).unwrap_or_default();
                        let color = operon::diag::color_enabled(std::io::stdout().is_terminal());
                        for f in fs {
                            print!(
                                "{}",
                                operon::diag::render_finding(
                                    file,
                                    &fsrc,
                                    f.sev.name(),
                                    f.code,
                                    &f.rule,
                                    f.line,
                                    &f.message,
                                    color
                                )
                            );
                        }
                    }
                }
                if results.len() == 1 {
                    if total > 0 {
                        println!("lint: {} finding(s)", total);
                    }
                    // the clean single-file line was already printed above
                } else {
                    println!("lint: {} finding(s) in {} file(s)", total, results.len());
                }
            }
            // W48 exit-code contract: lint is advisory, exit 0 unless --strict
            // escalates (any finding = 3) or an Error-severity finding exists
            // (none in the lint stream today; the rule is kept for append-only
            // safety, mirroring the pre-W48 behavior).
            if strict && total > 0 {
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(3);
            }
            if results
                .iter()
                .any(|(_, fs)| fs.iter().any(|f| f.sev == operon::lint::Sev::Error))
            {
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(1);
            }
        }
        // W55: machine-readable keyword inventory (docs/KEYWORDS.md is the
        // generated human table from the same source of truth).
        "keywords" => {
            let kws = parser::keyword_list();
            if json {
                let items: Vec<String> = kws.iter().map(|k| format!("\"{}\"", k)).collect();
                println!(
                    "{{\"count\":{},\"keywords\":[{}]}}",
                    kws.len(),
                    items.join(",")
                );
            } else {
                for k in kws {
                    println!("{}", k);
                }
            }
        }
        // W09 A2 / W10 stage 1: the OIR1 listing is real. Every top-level
        // gene compiles (unsupported constructs bridge at runtime, they do
        // not block compilation). W010-A: ir and disasm are the same tool
        // now — both accept --json (the disasm arm below is the alias).
        "ir" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("ir needs a file"),
            };
            let src = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| die(&format!("cannot read '{}': {}", file, e)));
            let prog = parser::parse(&src);
            if json {
                println!("{}", operon::vm::disassemble_program_json(&prog));
            } else {
                print!("{}", operon::vm::disassemble_program(&prog));
            }
        }
        "run" | "debug" => {
            debug_mode = cmd == "debug";
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("run needs a file"),
            };
            if locked && std::path::Path::new("operon.toml").exists() {
                pkg::check_locked_manifest();
            }
            // TYPED-MODE: the compile-time gate. Type errors abort BEFORE
            // execution; the dynamic side only ever sees a clean program.
            if typed {
                let src = std::fs::read_to_string(&file)
                    .unwrap_or_else(|e| die(&format!("cannot read {}: {}", file, e)));
                let prog = parser::parse(&src);
                let tfinds = operon::typeck::check_program(&prog);
                let hard: Vec<_> = tfinds
                    .iter()
                    .filter(|f| f.sev == operon::lint::Sev::Error)
                    .collect();
                let soft: Vec<_> = tfinds
                    .iter()
                    .filter(|f| f.sev != operon::lint::Sev::Error)
                    .collect();
                if !hard.is_empty() {
                    for f in &tfinds {
                        eprintln!("type-error {}:{}: {} [{}]", file, f.line, f.message, f.code);
                    }
                    eprintln!(
                        "operon: {} type error(s) in {} (--typed gate; not executed)",
                        hard.len(),
                        file
                    );
                    // ast-grep-ignore: no-std-process-exit-in-core
                    std::process::exit(3);
                }
                if !soft.is_empty() {
                    for f in &soft {
                        eprintln!("type-note {}:{}: {} [{}]", file, f.line, f.message, f.code);
                    }
                    if strict {
                        eprintln!("operon: --strict refuses type warnings");
                        // ast-grep-ignore: no-std-process-exit-in-core
                        std::process::exit(3);
                    }
                }
            }
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
            if trace_grn_path.is_some() {
                l.interp.trace_grn = Some(Vec::new());
            }
            if use_vm && !debug_mode {
                l.interp.vm = true;
                l.interp.vm_opt = opt_level;
                l.interp.opt_passes = opt_passes;
                l.interp.vm_program = Some(operon::vm::VmProgram::default());
            }
            // W08r hotfix: `debug` is an interpreter-side feature (the traps,
            // frame inspection and `p EXPR` all live in interp.rs). Since the
            // VM became the default lane, `operon debug` silently executed the
            // program in the VM where none of those hooks exist — the session
            // banner printed and the program ran to completion with no break
            // ever firing (the same silent-surface-death class as the lost
            // disasm arm). Debug mode now forces the tree-walking lane; the
            // banner says which lane you are on.
            if debug_mode {
                l.interp.debug_file = file.clone();
                for b in &debug_breaks {
                    // CLI --break stays numeric-only: conditions arrive
                    // through the live surfaces (REPL `b N if COND`, NDJSON
                    // breakpoints, DAP setBreakpoints)
                    l.interp.debug_breaks.insert(*b, None);
                }
                if debug_protocol_mode {
                    // W08r stage 2: stdout is the protocol transport — the
                    // debuggee's print output is captured and drained to
                    // stderr at every stop instead of polluting the stream
                    l.interp.debug_protocol = true;
                    l.interp.stdout_sink =
                        Some(std::rc::Rc::new(std::cell::RefCell::new(Vec::new())));
                    eprintln!("[debug] protocol mode: NDJSON on stdout, program output on stderr");
                }
                eprintln!(
                    "[debug] interactive session on {} (interp lane, breaks at {:?}); c=continue s=step q=quit",
                    file, debug_breaks
                );
            }
            if opts.frame.is_none() {
                // W101 slice 6: a typo'd --entry used to wobble-repair to the
                // nearest builtin and exit 0 having run NOTHING (the silent
                // no-op disaster). An explicitly requested entry gene that
                // the program does not define is a fatal, located, suggested
                // diagnostic now: rc 1 (dx-r1), did-you-mean attached.
                if let Some(e) = &opts.entry {
                    let mut gene_names: Vec<String> = Vec::new();
                    for st in &l.prog.stmts {
                        match st {
                            operon::ast::Stmt::Gene(g) | operon::ast::Stmt::Seq(g) => {
                                if let Some(n) = &g.name {
                                    gene_names.push(n.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                    if !gene_names.iter().any(|n| n == e) {
                        let cands: Vec<&str> = gene_names.iter().map(|s| s.as_str()).collect();
                        let sug = operon::diag::did_you_mean(e, &cands);
                        let s = operon::value::Stress::new(
                            "missing",
                            format!("entry gene '{}' is not defined in this program", e),
                        );
                        let src_text = std::fs::read_to_string(&file).unwrap_or_default();
                        if json_errors {
                            eprint!(
                                "{}",
                                operon::diag::render_json(&l.interp.file, &src_text, &s, &sug)
                            );
                        } else {
                            eprint!(
                                "{}",
                                operon::diag::render(
                                    &l.interp.file,
                                    &src_text,
                                    &s,
                                    operon::diag::color_enabled(std::io::stderr().is_terminal()),
                                    &sug
                                )
                            );
                        }
                        write_grn_trace(&l.interp, &trace_grn_path);
                        tools::flush_notes(&l, opts.quiet);
                        if debug_protocol_mode {
                            l.interp.debug_drain_print_sink();
                        }
                        // ast-grep-ignore: no-std-process-exit-in-core
                        std::process::exit(1);
                    }
                }
                let result = tools::run_entry(&mut l, &opts);
                match result {
                    Ok(_) => {}
                    Err(s) => {
                        // W101: excellent errors. The rendered block (code,
                        // snippet, note, help) replaces the old one-liner;
                        // --json-errors swaps the whole block for one JSON
                        // object (SPEC 9a). rc semantics and the W007 chain
                        // rendering below are unchanged, downstream parsers
                        // of stderr keep working.
                        // The snippet source: re-read the script on the error
                        // path (error path only; load_file owns the parse).
                        let src_text = std::fs::read_to_string(&file).unwrap_or_default();
                        if json_errors {
                            eprint!(
                                "{}",
                                operon::diag::render_json(&l.interp.file, &src_text, &s, &[])
                            );
                        } else {
                            eprint!(
                                "{}",
                                operon::diag::render(
                                    &l.interp.file,
                                    &src_text,
                                    &s,
                                    operon::diag::color_enabled(std::io::stderr().is_terminal()),
                                    &[]
                                )
                            );
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
                            if !s.chain.is_empty()
                                && s.chain.last().map(|(n, _)| n.as_str()) != Some("main")
                            {
                                eprintln!("  at main");
                            }
                        }
                        // W007 chain rendering moved into the W101 block above
                        // (capped at 64, "at main" tail preserved). In
                        // --json-errors mode the chain is a JSON array field.
                        // dx-r1 (parity audit W2): a failing program must not
                        // report success, CI/shell pipelines trusted rc=0
                        // from scripts that died. 1 = uncaught top-level stress.
                        // W095: frames collected so far are still diagnostics,
                        // drain before the exit.
                        write_grn_trace(&l.interp, &trace_grn_path);
                        tools::flush_notes(&l, opts.quiet);
                        if debug_protocol_mode {
                            l.interp.debug_drain_print_sink();
                        }
                        // ast-grep-ignore: no-std-process-exit-in-core
                        std::process::exit(1);
                    }
                }
            }
            write_grn_trace(&l.interp, &trace_grn_path);
            tools::flush_notes(&l, opts.quiet);
            if debug_protocol_mode {
                l.interp.debug_drain_print_sink();
            }
            let strict_cell = l
                .interp
                .cell
                .get("wobble.strict")
                .map(|v| v == "true")
                .unwrap_or(false);
            if (strict || strict_cell) && l.interp.notes.iter().any(|n| n.rung >= 3) {
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(3);
            }
        }
        "check" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("check needs a file"),
            };
            let rep = tools::check(&file, &opts, nmd, purge);
            // W48 (ROADMAP-100): check = correctness. Its slice of the shared
            // rule engine is wrong-arity + const-reassign; phantoms, NMD and
            // cell keys stay check-side (tools.rs). The style/quality stream
            // moved to `operon lint` and is NOT duplicated in the default
            // diag output: a labeled count + pointer keeps the data visible
            // without doubling the stream; --style inlines it.
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            let prog = parser::parse(&src);
            // TYPED-MODE: T-series findings join the check stream under --typed
            let typed_findings: Vec<operon::lint::Finding> = if typed {
                operon::typeck::check_program(&prog)
            } else {
                Vec::new()
            };
            let mut findings = operon::lint::lint_correctness(&prog);
            operon::lint::apply_allows(&mut findings, &src);
            let mut style = operon::lint::lint_style(&prog);
            operon::lint::apply_allows(&mut style, &src);
            operon::lint::apply_cli_allows(&mut style, &lint_allows);
            // W41 (ROADMAP-100): diag is the default: sectioned diagnostics
            // (error/warning/repair/style) with a summary line; --format score
            // keeps the school-grade banner for one transition cycle.
            if check_format == "diag" && !json {
                let style_inline: Vec<operon::lint::Finding> = if check_style {
                    style.clone()
                } else {
                    Vec::new()
                };
                print_diag(
                    &file,
                    &src,
                    &rep,
                    &findings,
                    &style,
                    &style_inline,
                    &typed_findings,
                );
                let hard = findings.iter().any(|f| f.sev == operon::lint::Sev::Error)
                    || typed_findings
                        .iter()
                        .any(|f| f.sev == operon::lint::Sev::Error);
                let typed_soft = typed_findings
                    .iter()
                    .any(|f| f.sev == operon::lint::Sev::Warning);
                if hard || (strict && (rep.wobbles > 0 || rep.fallbacks > 0 || typed_soft)) {
                    // ast-grep-ignore: no-std-process-exit-in-core
                    std::process::exit(3);
                }
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(0);
            }
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
                    .map(|p| {
                        let sug: Vec<String> = p
                            .suggestions
                            .iter()
                            .map(|s| format!("\"{}\"", tools::json_escape(s)))
                            .collect();
                        let fix = p
                            .fix
                            .as_ref()
                            .map(|f| f.to_json())
                            .unwrap_or_else(|| "null".into());
                        format!(
                            "{{\"name\":\"{}\",\"line\":{},\"suggestions\":[{}],\"fix\":{}}}",
                            tools::json_escape(&p.name),
                            p.line,
                            sug.join(","),
                            fix
                        )
                    })
                    .collect();
                // W48: the findings array carries CHECK-owned rules only
                // (correctness); style findings live in `operon lint --json`.
                // TYPED-MODE: T-series findings ride the same array under --typed.
                let all_findings: Vec<&operon::lint::Finding> =
                    findings.iter().chain(typed_findings.iter()).collect();
                let f_json: Vec<String> = all_findings
                    .iter()
                    .map(|f| {
                        format!(
                            "{{\"code\":\"{}\",\"severity\":\"{}\",\"line\":{},\"rule\":\"{}\",\"message\":\"{}\"}}",
                            f.code,
                            f.sev.name(),
                            f.line,
                            f.rule,
                            tools::json_escape(&f.message)
                        )
                    })
                    .collect();
                println!(
                    "{{\"file\":\"{}\",\"score\":{},\"letter\":\"{}\",\"notes\":{},\"wobbles\":{},\"fallbacks\":{},\"phantoms\":[{}],\"nmd\":[{}],\"findings\":[{}]}}",
                    tools::json_escape(&file),
                    rep.score,
                    rep.letter,
                    rep.notes,
                    rep.wobbles,
                    rep.fallbacks,
                    ph_json.join(","),
                    nmd_json.join(","),
                    f_json.join(",")
                );
            } else {
                println!(
                    "operon check: {}, score {}/100 (grade {})",
                    file, rep.score, rep.letter
                );
                if rep.wobbles > 0 || rep.fallbacks > 0 {
                    println!(
                        "  repairs: {} wobble, {} fallback",
                        rep.wobbles, rep.fallbacks
                    );
                }
                if !rep.phantoms.is_empty() {
                    let names: Vec<String> = rep.phantoms.iter().map(|p| p.name.clone()).collect();
                    println!("  phantom calls: {}", names.join(", "));
                }
                for (k, m) in &rep.nmd {
                    println!("  nmd[{}]: {}", k, m);
                }
            }
            if strict && (rep.wobbles > 0 || rep.fallbacks > 0) {
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(3);
            }
        }
        "test" => {
            let paths: Vec<String> = if positional.is_empty() {
                vec!["tests".to_string()]
            } else {
                positional.clone()
            };
            // W49 (ROADMAP-100): --list enumerates discovered files + proof
            // frame counts without executing; --filter substr narrows the
            // selection (substring match on the file path: proof frames are
            // anonymous in the grammar, the file is the selectable test
            // unit); --repeat N re-runs and pins byte-identical results.
            // No flags: discovery unchanged, byte-identical to the old runner.
            let files = tools::select_test_files(&paths, test_filter.as_deref());
            if list_only {
                let mut total = 0usize;
                for f in &files {
                    let n = tools::count_proof_frames(f);
                    total += n;
                    println!("{}  {} proof frame(s)", f, n);
                }
                println!("{} file(s), {} proof frame(s) total", files.len(), total);
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(0);
            }
            let rep = tools::run_tests(&files, &opts, json);
            if repeat > 1 {
                for run in 2..=repeat {
                    let again = tools::run_tests(&files, &opts, false);
                    if again.proofs != rep.proofs
                        || again.passed != rep.passed
                        || again.failed != rep.failed
                        || again.asserts != rep.asserts
                    {
                        eprintln!(
                            "determinism: run {} diverged (proofs {}/{} passed {}/{} failed {}/{})",
                            run,
                            again.proofs,
                            rep.proofs,
                            again.passed,
                            rep.passed,
                            again.failed,
                            rep.failed
                        );
                        // ast-grep-ignore: no-std-process-exit-in-core
                        std::process::exit(1);
                    }
                }
                eprintln!(
                    "determinism: {} runs identical ({} proofs, {} asserts)",
                    repeat, rep.proofs, rep.asserts
                );
            }
            if json {
                let fails: Vec<String> = rep
                    .failures
                    .iter()
                    .map(|f| format!("\"{}\"", tools::json_escape(f)))
                    .collect();
                println!(
                    "{{\"files\":{},\"proofs\":{},\"passed\":{},\"failed\":{},\"asserts\":{},\"failures\":[{}]}}",
                    rep.files,
                    rep.proofs,
                    rep.passed,
                    rep.failed,
                    rep.asserts,
                    fails.join(",")
                );
            }
            if rep.failed > 0 {
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(1);
            }
        }
        "fmt" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("fmt needs a file"),
            };
            // W47: layered formatter config. Precedence: flags > fmt file
            // (.operon-fmt.toml, or --fmt-config PATH) > operon.toml [fmt]
            // (project manifest, current directory) > defaults. Malformed
            // sections and bad values fall back with a stderr note, never a
            // crash; absent files change nothing (byte-identical output).
            let mut cfg = tools::FmtConfig::default();
            if let Ok(text) = std::fs::read_to_string("operon.toml") {
                let (section, sec_notes) = tools::extract_toml_section(&text, "fmt");
                for n in &sec_notes {
                    eprintln!("fmt: operon.toml: {n}");
                }
                if !section.is_empty() {
                    let (file_cfg, unknown) = tools::parse_fmt_config_from(cfg, &section);
                    cfg = file_cfg;
                    for u in &unknown {
                        eprintln!("fmt: ignoring unknown [fmt] key in operon.toml: {u}");
                    }
                }
            }
            let cfg_path = fmt_config_path
                .clone()
                .unwrap_or_else(|| ".operon-fmt.toml".to_string());
            if let Ok(text) = std::fs::read_to_string(&cfg_path) {
                let (file_cfg, unknown) = tools::parse_fmt_config_from(cfg, &text);
                cfg = file_cfg;
                for u in &unknown {
                    eprintln!("fmt: ignoring unknown config key in {cfg_path}: {u}");
                }
            } else if fmt_config_path.is_some() {
                die(&format!("fmt: config file not found: {cfg_path}"));
            }
            if let Some(n) = fmt_indent {
                cfg.indent = n;
            }
            if let Some(q) = fmt_quotes {
                cfg.quotes = q;
            }
            // W47-v2: flag > file > default. 0 means explicitly off.
            if let Some(n) = fmt_width {
                cfg.width = if n == 0 { None } else { Some(n) };
            }
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            let prog = parser::parse(&src);
            let out = tools::format_program_with(&prog, &cfg);
            if write {
                std::fs::write(&file, out).expect("write failed");
                eprintln!("fmt: {} rewritten", file);
            } else {
                print!("{}", out);
            }
        }
        // W65 (ROADMAP-100): the automated migrator. fix = legacy-surface
        // migrations (const→let, s:: → dot access) + Total Grammar
        // canonicalization, through the same formatter `operon fmt` uses.
        // Dry-run by default (prints a per-line diff); --write applies.
        // The canonical MEANING of the program never changes, pinned by
        // tests/fix_corpus.rs (corpus-wide: canonical(fix(x)) == canonical(x)).
        "fix" => {
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("fix needs a file"),
            };
            let src = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| die(&format!("cannot read {}: {}", file, e)));
            let (out, rep) = tools::fix_source(&src);
            let changed = out != src;
            if json {
                println!(
                    "{{\"file\":\"{}\",\"changed\":{},\"migrations\":{{\"canonicalize\":{},\"const_to_let\":{},\"s_dot\":{}}}}}",
                    tools::json_escape(&file),
                    changed,
                    rep.canonicalized,
                    rep.const_to_let,
                    rep.s_dot
                );
            } else if !changed {
                println!("fix: {}, already canonical", file);
            } else if write {
                std::fs::write(&file, &out)
                    .unwrap_or_else(|e| die(&format!("write failed: {}", e)));
                eprintln!(
                    "fix: {} rewritten ({} canonicalized, {} const→let, {} s::→dot)",
                    file, rep.canonicalized, rep.const_to_let, rep.s_dot
                );
            } else {
                // dry-run: show the diff, change nothing. Per changed line:
                // the old line, then the new line, with 1-based line numbers.
                println!("fix: {}, dry run (pass --write to apply)", file);
                let old_lines: Vec<&str> = src.lines().collect();
                let new_lines: Vec<&str> = out.lines().collect();
                let mut shown = 0usize;
                for k in 0..old_lines.len().max(new_lines.len()) {
                    let o = old_lines.get(k).copied().unwrap_or("");
                    let nw = new_lines.get(k).copied().unwrap_or("");
                    if o != nw {
                        if o.is_empty() {
                            println!("{:>5}: + {}", k + 1, nw);
                        } else if nw.is_empty() {
                            println!("{:>5}: - {}", k + 1, o);
                        } else {
                            println!("{:>5}: - {}", k + 1, o);
                            println!("{:>5}: + {}", k + 1, nw);
                        }
                        shown += 1;
                    }
                    if shown >= 200 {
                        println!("  … more lines not shown (use --write to apply all)");
                        break;
                    }
                }
            }
        }
        "rna" => {
            // W068 safety mode: default = checked dry-run (writes nothing);
            // --write applies the same engine; exit 1 on any miss so scripts notice.
            // W067 v2: a patch whose first content line is `syntax: v2` dispatches
            // to the node-addressed engine (parse → edit AST → canonical reprint,
            // all-or-nothing); absent header keeps v1 text semantics.
            if positional.len() < 2 {
                die("rna needs: operon rna <file.op> <patch.rna> [--write] [--check] [--json] [--allow-comment-drop]");
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
            // W68: `--check` validates the patch against the target and writes
            // NOTHING, ever (`--check --write` is refused up front). It reports
            // the engine detected (v1 header-less vs v2 `syntax: v2`), whether
            // every referenced span/node resolves (ambiguity included), the v2
            // comment preflight, and whether a real apply with the same flags
            // would succeed — by running the exact apply machinery in memory
            // (both engines are pure; file writes only ever happen in this CLI
            // layer, and check never reaches them). Exit 0 = would apply
            // cleanly, 1 = validation failure, 2 = usage/fatal (die()).
            if rna_check && write {
                die("rna: --check never writes; drop --write");
            }
            if rna_check {
                if rna2::is_v2_patch(&patch_src) {
                    let rep = rna2::check_rna_v2(&src, &patch_src, allow_comment_drop);
                    let applied = rep.rules.iter().filter(|r| r.applied).count();
                    let missed = rep.rules.iter().filter(|r| !r.target_found).count();
                    let would_change = rep.would_apply && applied > 0;
                    let preflight = if rep.comment_refusal.is_some() {
                        "refused"
                    } else if rep.comment_lines.is_empty() {
                        "ok"
                    } else {
                        "allowed"
                    };
                    if json {
                        let rows: Vec<String> = rep
                            .rules
                            .iter()
                            .map(|r| {
                                format!(
                                    "{{\"verb\":\"{}\",\"target\":\"{}\",\"target_found\":{},\"applied\":{},\"detail\":\"{}\"}}",
                                    r.verb,
                                    tools::json_escape(&r.target),
                                    r.target_found,
                                    r.applied,
                                    tools::json_escape(&r.detail)
                                )
                            })
                            .collect();
                        let lines: Vec<String> =
                            rep.comment_lines.iter().map(|l| l.to_string()).collect();
                        let mut out = format!(
                            "{{\"engine\":\"v2\",\"check\":true,\"file\":\"{}\",\"patch\":\"{}\",\"rules\":[{}],\"applied\":{},\"missed\":{},\"would_change\":{},\"would_apply\":{},\"would_write\":false,\"comment_preflight\":\"{}\",\"comment_lines\":[{}]",
                            tools::json_escape(&file),
                            tools::json_escape(&patch_path),
                            rows.join(","),
                            applied,
                            missed,
                            would_change,
                            rep.would_apply,
                            preflight,
                            lines.join(",")
                        );
                        if rep.would_apply {
                            out.push('}');
                        } else {
                            out.push_str(&format!(
                                ",\"refused\":true,\"reason\":\"{}\"}}",
                                tools::json_escape(
                                    rep.reason.as_deref().unwrap_or("would not apply")
                                )
                            ));
                        }
                        println!("{}", out);
                    } else {
                        println!(
                            "rna check: {} <- {}, engine v2, {}",
                            file,
                            patch_path,
                            if rep.would_apply {
                                format!(
                                    "would apply cleanly ({} rule(s), nothing written){}",
                                    applied,
                                    if would_change { "" } else { " (no change)" }
                                )
                            } else {
                                "REFUSED, nothing written".to_string()
                            }
                        );
                        if let Some(reason) = &rep.reason {
                            println!("  reason: {}", reason);
                        }
                        for r in &rep.rules {
                            if !r.target_found {
                                println!("  MISS [{}] {}", r.target, r.detail);
                            } else {
                                println!("  ok   [{}] {}", r.target, r.detail);
                            }
                        }
                        if rep.comment_refusal.is_none() && !rep.comment_lines.is_empty() {
                            println!(
                                "  note: plain '#' comments at lines [{}] will be dropped (--allow-comment-drop)",
                                rep.comment_lines
                                    .iter()
                                    .map(|l| l.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        }
                    }
                    if !rep.would_apply {
                        // ast-grep-ignore: no-std-process-exit-in-core
                        std::process::exit(1);
                    }
                    return;
                }
                // v1 (header-less) check: the same deprecation metadata as the
                // apply path (stderr note + `deprecated:true` in --json), the
                // same per-edit fate, nothing ever written.
                eprintln!(
                    "rna: note: header-less (v1 span) patches are deprecated (info, W63 step 1), add 'syntax: v2' and node-addressed rules; see docs/design/RNA-V2.md"
                );
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
                        "{{\"engine\":\"v1\",\"deprecated\":true,\"check\":true,\"file\":\"{}\",\"patch\":\"{}\",\"edits\":[{}],\"applied\":{},\"missed\":{},\"would_change\":{},\"would_apply\":{},\"would_write\":false}}",
                        tools::json_escape(&file),
                        tools::json_escape(&patch_path),
                        rows.join(","),
                        report.applied(),
                        report.missed(),
                        report.would_change(),
                        report.missed() == 0
                    );
                } else {
                    println!(
                        "rna check: {} <- {}, engine v1 (deprecated), {}",
                        file,
                        patch_path,
                        if report.missed() == 0 {
                            format!(
                                "would apply cleanly ({} edit(s), nothing written){}",
                                report.applied(),
                                if report.would_change() {
                                    ""
                                } else {
                                    " (no change)"
                                }
                            )
                        } else {
                            format!("WOULD MISS ({} missed, nothing written)", report.missed())
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
                if report.missed() > 0 {
                    // ast-grep-ignore: no-std-process-exit-in-core
                    std::process::exit(1);
                }
                return;
            }
            if rna2::is_v2_patch(&patch_src) {
                let report = match rna2::apply_rna_v2(&src, &patch_src, allow_comment_drop) {
                    Ok(r) => r,
                    Err(refusal) => die(&refusal),
                };
                if json {
                    let rows: Vec<String> = report
                        .rules
                        .iter()
                        .map(|r| {
                            format!(
                                "{{\"verb\":\"{}\",\"target\":\"{}\",\"target_found\":{},\"applied\":{},\"detail\":\"{}\"}}",
                                r.verb,
                                tools::json_escape(&r.target),
                                r.target_found,
                                r.applied,
                                tools::json_escape(&r.detail)
                            )
                        })
                        .collect();
                    match &report.new_text {
                        Some(new_text) => {
                            println!(
                                "{{\"engine\":\"v2\",\"file\":\"{}\",\"patch\":\"{}\",\"rules\":[{}],\"applied\":{},\"missed\":{},\"would_change\":{}}}",
                                tools::json_escape(&file),
                                tools::json_escape(&patch_path),
                                rows.join(","),
                                report.applied(),
                                report.missed(),
                                report.would_change()
                            );
                            if write {
                                std::fs::write(&file, new_text).expect("write failed");
                                eprintln!("rna v2: {} rewritten", file);
                            }
                        }
                        None => {
                            // all-or-nothing refusal, nothing written
                            println!(
                                "{{\"engine\":\"v2\",\"file\":\"{}\",\"patch\":\"{}\",\"rules\":[{}],\"applied\":0,\"missed\":{},\"would_change\":false,\"refused\":true}}",
                                tools::json_escape(&file),
                                tools::json_escape(&patch_path),
                                rows.join(","),
                                report.missed()
                            );
                        }
                    }
                } else {
                    match &report.new_text {
                        Some(_) => println!(
                            "rna v2: {} <- {}, {} applied, {} missed{}",
                            file,
                            patch_path,
                            report.applied(),
                            report.missed(),
                            if report.would_change() {
                                ""
                            } else {
                                " (no change)"
                            }
                        ),
                        None => println!(
                            "rna v2: {} <- {}, REFUSED (all-or-nothing): {} missed",
                            file,
                            patch_path,
                            report.missed()
                        ),
                    }
                    for r in &report.rules {
                        if !r.target_found {
                            println!("  MISS [{}] {}", r.target, r.detail);
                        } else {
                            println!("  ok   [{}] {}", r.target, r.detail);
                        }
                    }
                    if let Some(new_text) = &report.new_text {
                        if write {
                            std::fs::write(&file, new_text).expect("write failed");
                            eprintln!("rna v2: {} rewritten", file);
                        }
                    }
                }
                if report.missed() > 0 {
                    // ast-grep-ignore: no-std-process-exit-in-core
                    std::process::exit(1);
                }
                return;
            }
            // W067 stage 3, step 1 (W63 deprecation policy: Info first):
            // a header-less patch runs the v1 span engine. Text semantics are
            // byte-compatible and UNCHANGED, the deprecation surfaces only as
            // tool metadata (this stderr note + `deprecated:true` in --json).
            // Scoped to the `operon rna` file-editor path on purpose: the
            // `run --rna` pre-parse path must stay note-free so Rust/oracle
            // differential stderr parity is never at risk.
            eprintln!(
                "rna: note: header-less (v1 span) patches are deprecated (info, W63 step 1), add 'syntax: v2' and node-addressed rules; see docs/design/RNA-V2.md"
            );
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
                    "{{\"engine\":\"v1\",\"deprecated\":true,\"file\":\"{}\",\"patch\":\"{}\",\"edits\":[{}],\"applied\":{},\"missed\":{},\"would_change\":{}}}",
                    tools::json_escape(&file),
                    tools::json_escape(&patch_path),
                    rows.join(","),
                    report.applied(),
                    report.missed(),
                    report.would_change()
                );
            } else {
                println!(
                    "rna: {} <- {}, {} applied, {} missed{}",
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
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(1);
            }
        }
        "build" => {
            // W086 build-contract honesty: refuse the planned modes explicitly
            // instead of silently ignoring the flags (help text matches reality).
            if build_native {
                die("operon build --native: native executable compilation is not implemented yet (W85; build today emits specialized source, see SPEC tools section)");
            }
            if build_bundle {
                die("operon build --bundle: single-file bundling is not implemented yet (W87 deferred; design: docs/design/BUNDLE.md)");
            }
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
            // W097-A: Chrome-trace export. Written BEFORE any stdout output;
            // the operator notice rides stderr when --json keeps stdout
            // machine-pure, stdout otherwise (human-readable mode).
            if let Some(path) = &chrome {
                let (total, dropped) = tools::write_chrome_trace(&l, &file, path);
                let notice = format!(
                    "chrome trace: {} ({} span(s), {} dropped past the {} cap)",
                    path,
                    total,
                    dropped,
                    interp::SPAN_CAP
                );
                if json {
                    eprintln!("{}", notice);
                } else {
                    println!("{}", notice);
                }
            }
            let mut rows: Vec<(String, u64, f64)> = l
                .interp
                .bk_slots
                .iter()
                .map(|s| {
                    let t = l.interp.call_time_self.get(&s.name).cloned().unwrap_or(0.0);
                    (s.name.clone(), s.count, t)
                })
                .collect();
            rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            // W096: machine-readable profile. Self-describing (units, version,
            // per-gene flags). NOTE: Chrome-trace format is deliberately NOT
            // emitted yet, the profiler records aggregate self-time only;
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
                let mature = l.interp.bk_slots.len().min(total_defined);
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
                let mature = l.interp.bk_slots.len().min(total_defined);
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
            let max_calls = l.interp.bk_slots.iter().map(|s| s.count).max().unwrap_or(0);
            for g in &l.interp.defined_genes {
                let calls = l.interp.bk_count_for(g);
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
        "doc" => {
            // W073: markdown/JSON API reference from the AST, parse-only
            // (like check/fmt/graph), no run, no capabilities beyond reading
            // the input files. Directories expand to their top-level *.op
            // files (sorted, deterministic output order).
            let mut md_out: Vec<(String, String)> = Vec::new();
            let mut json_out: Vec<String> = Vec::new();
            let mut targets: Vec<String> = Vec::new();
            for p in &positional {
                let meta = std::fs::metadata(p);
                match meta {
                    Ok(m) if m.is_dir() => {
                        let mut entries: Vec<String> = std::fs::read_dir(p)
                            .map(|rd| {
                                rd.filter_map(|e| e.ok())
                                    .map(|e| e.path())
                                    .filter(|pt| pt.extension().map(|x| x == "op").unwrap_or(false))
                                    .map(|pt| pt.to_string_lossy().to_string())
                                    .collect()
                            })
                            .unwrap_or_default();
                        entries.sort();
                        targets.extend(entries);
                    }
                    _ => targets.push(p.clone()),
                }
            }
            if targets.is_empty() {
                die("doc needs a file or directory");
            }
            for t in &targets {
                let src = std::fs::read_to_string(t).unwrap_or_default();
                let prog = parser::parse(&src);
                if json {
                    json_out.push(tools::doc_json(t, &prog));
                } else {
                    md_out.push((t.clone(), tools::doc_markdown(t, &prog)));
                }
            }
            if json {
                println!("[{}]", json_out.join(","));
            } else if !outfile.is_empty() {
                // -o DIR: write one markdown file per input module
                if let Err(e) = std::fs::create_dir_all(&outfile) {
                    die(&format!("doc -o: cannot create '{}': {}", outfile, e));
                }
                for (t, md) in &md_out {
                    let stem = std::path::Path::new(t)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| t.clone());
                    let dest = format!("{}/{}.md", outfile, stem);
                    if let Err(e) = std::fs::write(&dest, md) {
                        die(&format!("doc -o: cannot write '{}': {}", dest, e));
                    }
                    println!("  wrote {}", dest);
                }
            } else {
                for (_, md) in &md_out {
                    print!("{}", md);
                }
            }
        }
        "watch" => {
            // W072: re-run on change. v1: mtime polling (200 ms, no external
            // deps) over the entry file + its local (non-std) import tree;
            // each iteration is a fresh `operon run` child, so fuel/caps/
            // interpreter state reset per run, no cross-run contamination.
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
            // W094: static regulate-network export, parse-only (like
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
        "disasm" => {
            // W10: bytecode listing of compiled gene bodies. Parse-only
            // (like check/fmt/graph/doc): compile_body never executes,
            // no capabilities beyond reading the file.
            let file = match positional.first() {
                Some(f) => f.clone(),
                None => die("disasm needs a file"),
            };
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            let prog = parser::parse(&src);
            if json {
                println!("{}", vm::disassemble_program_json(&prog));
            } else {
                print!("{}", vm::disassemble_program(&prog));
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
                        "operon crispr: knocked out '{}', {}/{} proof(s) survived",
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
                "operon bench: {} × {} iters, min {:.3} ms · avg {:.3} ms",
                file, rep.iters, rep.min_ms, rep.avg_ms
            );
        }
        other => {
            // dx-r3 (re-audit): an unknown subcommand says so, a silent
            // usage dump reads like a broken flag
            die(&format!(
                "unknown command '{}', run `operon` with no arguments for usage",
                other
            ));
        }
    }
}

/// W095: drain the interpreter's GRN tick-stream buffer into the operator's
/// file. The interpreter itself never touches the filesystem, this is the
/// only I/O point. Frames are JSONL (one self-describing object per line).
fn write_grn_trace(interp: &operon::interp::Interp, path: &Option<String>) {
    if let (Some(p), Some(frames)) = (path, &interp.trace_grn) {
        let mut out = String::new();
        for f in frames {
            out.push_str(f);
            out.push('\n');
        }
        if let Err(e) = std::fs::write(p, out) {
            eprintln!("grn trace: cannot write {}: {}", p, e);
        } else {
            eprintln!("grn trace: {} frame(s) -> {}", frames.len(), p);
        }
    }
}

// ------------------------------------------------------------ repl
fn repl() {
    use std::io::{BufRead, Write};
    println!(
        "Operon {} repl, gene-expression shell (:help for commands, :quit to leave)",
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
            spans: false,
            stdout_sink: None,
            use_vm: true,
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
    // sec-r3 (re-audit #9): the REPL shares the run-wide fuel pool too,
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
                        println!(":proof [f]   run proof frames, this session's, or file f's");
                        println!(":genes       list genes defined so far");
                        println!(":doc name    show the ## doc comment of a declaration (W074)");
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
                                spans: false,
                                stdout_sink: None,
                                use_vm: true,
                            };
                            let rep = tools::run_tests(&[arg.to_string()], &opts, false);
                            println!(
                                "  {}: {}/{} proof(s) passed ({} assertion(s))",
                                arg, rep.passed, rep.proofs, rep.asserts
                            );
                        }
                    }
                    "doc" => {
                        // W074: print the `##` doc comment attached to a
                        // declaration in this session (metadata only, this
                        // never executes anything).
                        if arg.is_empty() {
                            println!("  usage: :doc <gene|phenotype|splice|fate name>");
                        } else {
                            let prog = parser::parse(&session);
                            let mut found = false;
                            let print_doc = |doc: &[String], label: &str| {
                                if doc.is_empty() {
                                    println!("  {}, no ## doc comment", label);
                                } else {
                                    for dl in doc {
                                        println!("  {}", dl);
                                    }
                                }
                            };
                            for st in &prog.stmts {
                                match st {
                                    Stmt::Gene(g) | Stmt::Seq(g)
                                        if g.name.as_deref() == Some(arg) =>
                                    {
                                        print_doc(&g.doc, arg);
                                        found = true;
                                    }
                                    Stmt::Pheno(p) if p.name == arg => {
                                        print_doc(&p.doc, arg);
                                        found = true;
                                    }
                                    Stmt::Splice(sp) if sp.root == arg => {
                                        print_doc(&sp.doc, arg);
                                        found = true;
                                    }
                                    Stmt::Fate(f) if f.name == arg => {
                                        print_doc(&f.doc, arg);
                                        found = true;
                                    }
                                    Stmt::Pheno(p) => {
                                        // phenotype methods are not top-level
                                        for m in &p.methods {
                                            if m.name.as_deref() == Some(arg) {
                                                print_doc(&m.doc, arg);
                                                found = true;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if !found {
                                println!("  no declaration named '{}' in this session", arg);
                            }
                        }
                    }
                    "genes" => {
                        let mut names = l.interp.defined_genes.clone();
                        if names.is_empty() {
                            println!("  (no genes defined yet, try: gene hi() {{ return 1 }})");
                        } else {
                            names.sort();
                            println!("  {}", names.join(", "));
                        }
                    }
                    "symbols" => {
                        // sec-r2: a real consumer of the intern table, the
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
                            .map(|k| k.to_string())
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
                                spans: false,
                                stdout_sink: None,
                                use_vm: true,
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
                    other => println!("  unknown command ':{}', try :help", other),
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

// :proof (no file), run every proof frame the session has defined so far,
// against the live interpreter state, mirroring `operon test` semantics:
// a proof must run to completion AND exercise at least one assertion.
fn repl_proof_session(l: &mut tools::Loaded, session: &str) {
    if session.trim().is_empty() {
        println!("  (empty session, define some code first)");
        return;
    }
    let prog = parser::parse(session);
    if prog.proofs.is_empty() {
        println!(
            "  no proof frames in this session, add one: frame proof {{ assert(1 == 1, \"ok\") }}"
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
                        "  session proof #{}: FAILED (vacuous, no assertion exercised)",
                        i + 1
                    );
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
            Ok(_) => {
                println!(
                    "  session proof #{}: FAILED (exited early, return/break inside proof)",
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

/// dx-r1 (audit W4): print only the notes this REPL input produced,
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
        // W101 slice 7: the same derived E2xxx code the run-path flush uses,
        // so a repair reads identically in the REPL and in a script run.
        let code = operon::diag::note_code(&n.message);
        let tag_s = if code.is_empty() {
            tag.to_string()
        } else {
            format!("{} {}", tag, code)
        };
        // A13 (dx-r2): real locations when the note carries a line
        if n.line > 0 {
            println!("  [{}] {}:{}: {}", tag_s, l.interp.file, n.line, n.message);
        } else {
            println!("  [{}] {}", tag_s, n.message);
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
                    // REPL, the variant value passes through, tagged honestly.
                    Err(st) if st.prop.is_some() => {
                        println!("  [propagate] {} passes through", st.prop.unwrap().repr())
                    }
                    Err(st) => println!("  [{}] {}", st.kind, st.message),
                }
            } else {
                if let Err(st) = l.interp.exec_stmt(&env, stmt) {
                    // W06 (D-014): REPL top-level propagation, value passes
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
    // expression mode: a bare `1 + 2 * 3` is not a statement, evaluate it
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
            // W06 (D-014): REPL expression-mode propagation, same
            // passes-through contract as statement mode.
            if let Some(v) = st.prop {
                println!("  [propagate] {} passes through", v.repr());
            } else {
                println!("  [{}] {}", st.kind, st.message);
            }
        }
    }
    // dx-r1 (parity audit W4): notes were recorded but never shown, the
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

/// W41 (ROADMAP-100): sectioned diagnostics renderer, the check output that
/// treats programmers as adults (what's wrong + the fix), with the school
/// grade preserved under `--format score` for CI compatibility.
///
/// W48 stream split: `findings` carries the correctness stream; `style` is
/// the lint-owned style stream (counted, not shown by default); when
/// `--style` was passed, `style_inline` repeats the list to render inline
/// under a clearly labeled header. Default output keeps a labeled count +
/// pointer line — the split is about command purpose and defaults, not about
/// hiding data.
#[allow(clippy::too_many_arguments)] // TYPED-MODE: the typed stream is the 8th
fn print_diag(
    file: &str,
    src: &str,
    rep: &tools::CheckReport,
    findings: &[operon::lint::Finding],
    style: &[operon::lint::Finding],
    style_inline: &[operon::lint::Finding],
    typed: &[operon::lint::Finding],
) {
    use operon::lint::Sev;
    let color = operon::diag::color_enabled(std::io::stdout().is_terminal());
    let mut errors: Vec<&operon::lint::Finding> = Vec::new();
    let mut warnings: Vec<&operon::lint::Finding> = Vec::new();
    for f in findings {
        match f.sev {
            Sev::Error => errors.push(f),
            Sev::Warning => warnings.push(f),
            Sev::Style => {} // unreachable in the correctness stream, kept for safety
        }
    }
    // TYPED-MODE: T-series findings get their own sectioned stream; errors
    // escalate the summary counts, warnings stay advisory. W101 slice 2:
    // findings render as located blocks (SPEC 9a.1) via diag::render_finding.
    let t_errors: Vec<&operon::lint::Finding> =
        typed.iter().filter(|f| f.sev == Sev::Error).collect();
    let t_warnings: Vec<&operon::lint::Finding> =
        typed.iter().filter(|f| f.sev == Sev::Warning).collect();
    let section = |name: &str, items: &[&operon::lint::Finding]| {
        if items.is_empty() {
            return;
        }
        println!("{}:", name);
        for f in items {
            print!(
                "{}",
                operon::diag::render_finding(
                    file, src, name, f.code, &f.rule, f.line, &f.message, color
                )
            );
        }
    };
    let mut n_errors = errors.len() + t_errors.len();
    if !rep.parsed {
        println!("error:");
        println!("  error[E00]: file could not be read");
        n_errors += 1;
    }
    section("error", &errors);
    if !typed.is_empty() {
        section("type-error (TYPED-MODE)", &t_errors);
        section("type-note (TYPED-MODE)", &t_warnings);
    }
    let n_warnings = warnings.len() + rep.phantoms.len() + t_warnings.len();
    if !rep.phantoms.is_empty() {
        println!("warning:");
        for p in &rep.phantoms {
            // W101 slice 6: phantoms carry their call line now (Expr::Call
            // stamps it), so they render as located blocks; a phantom with no
            // honest line stays header-only. The message carries the
            // did-you-mean shortlist; the machine-applicable fix renders as a
            // second help line (the structured edit rides the --json shape).
            let mut msg = format!(
                "phantom call '{}' is called but not defined in this file",
                p.name
            );
            if let Some(h) = operon::diag::suggestion_help(&p.suggestions) {
                msg.push_str(&format!(" ({})", h));
            }
            if p.line > 0 {
                let line_text = src.lines().nth(p.line - 1).unwrap_or("");
                let labels = operon::diag::Span::of_token_word(p.line, line_text, &p.name)
                    .map(|sp| {
                        vec![operon::diag::Label {
                            span: sp,
                            text: String::new(),
                            primary: true,
                        }]
                    })
                    .unwrap_or_default();
                print!(
                    "{}",
                    operon::diag::render_finding_labeled(
                        file,
                        src,
                        "warning",
                        "W01",
                        "phantom-call",
                        p.line,
                        &msg,
                        color,
                        &labels
                    )
                );
                if let Some(f) = &p.fix {
                    println!("help: machine-applicable fix: {}", f.note);
                }
            } else {
                println!("  warning[W01]: {}", msg);
            }
        }
    }
    section("warning", &warnings);
    if rep.notes > 0 {
        println!("repair:");
        println!(
            "  {}: {} note(s), {} wobble(s), {} fallback(s); run `operon explain {}` for the play-by-play",
            file, rep.notes, rep.wobbles, rep.fallbacks, file
        );
    }
    // W48: the style stream is lint-owned. Default: one labeled line with the
    // count and the pointer; --style renders the full section inline.
    if !style_inline.is_empty() {
        println!("style (owned by `operon lint`, advisory):");
        for f in style_inline {
            print!(
                "{}",
                operon::diag::render_finding(
                    file, src, "style", f.code, &f.rule, f.line, &f.message, color
                )
            );
        }
    } else if !style.is_empty() {
        println!(
            "style (owned by `operon lint`): {} style finding(s), not shown here; run `operon lint {}` to see them",
            style.len(),
            file
        );
    }
    println!(
        "summary: {} error(s), {} warning(s), {} style, {} repair note(s)",
        n_errors,
        n_warnings,
        style.len(),
        rep.notes
    );
}

fn usage() {
    eprintln!(
        "Operon {}, the gene-expression language (Total Grammar)
usage:
  operon run f.op [--entry g] [--variant v] [--cell c] [--rna r] [--frame name] [--ires] [--strict] [--quiet]
                  [--fuel steps] [--allow-read path] [--allow-write path] [--allow-net host:port]
                  [--allow-run cmd] [--allow-py module] [--allow-exit] [--allow-env var] [--allow-all]
                  [--typed]  # TYPED-MODE: compile-time type gate; errors abort before execution
  operon check f.op [--format diag|score] [--nmd | --nmd=purge] [--json] [--style] [--strict] [--typed]
                  # W48 split: check = CORRECTNESS (wrong-arity, phantom-call, const-reassign,
                  # cell keys, NMD). diag default shows correctness sections; the style stream is
                  # summarized with a pointer to `operon lint` (--style inlines it).
                  # --typed: static type checking (TYPED-MODE, T-series findings)
  operon lint f.op [more.op ...] [--strict] [--allow rule1,rule2] [--cell c] [--json]
                  # W48 split: lint = STYLE/QUALITY only (unused-gene, unused-import,
                  # unused-binding, dead-const, shadowed-binding, constant-condition,
                  # infinite-loop-suspect, unreachable-code, duplicate/unreachable-match-arm).
                  # --strict: any finding = exit 3; --allow: file-wide suppression on top of
                  # '// allow: rule' comment suppression; --json: findings carry code/severity/line/rule/message
  operon test [paths...] [--json] [--filter substr] [--list] [--repeat n]
  operon fmt f.op [--write] [--indent N] [--quotes single|double] [--width N] [--fmt-config f]
  operon fix f.op [--write] [--json]   # migrate legacy surface (const→let, s::→dot), dry-run default
  operon ast f.op [--json]
  operon explain f.op [--json] [--strict]
  operon keywords [--json]
  operon repl
  operon debug f.op --break N   # W08r debugger (interp lane): breaks at line N, then
                  # c | s(tep-into) | n(ext, step-over) | fin(ish, step-out) |
                  # until N | b N | b del N | b list | bt | vars | p EXPR | q
                  # (--protocol=json serves the same state over NDJSON)
  operon dap f.op   # W08r stage 3: the Debug Adapter Protocol adapter on stdio
                  # (Content-Length framing; drives editors/ide debug clients)
  operon new NAME [--lib] [--here]   scaffold a project (operon.toml + src + a green smoke test)
  operon add NAME[@REQ] [--registry FILE]   pull a dependency from the registry
                  # REQ (item 4, ai/ecosystem-r3): ^1.2 ~1.2 >=1 <2 =X.Y.Z 1.x *
                  # (AND-lists ok). The HIGHEST registry version satisfying REQ wins;
                  # the req lands in operon.toml, the exact version in operon.lock,
                  # and --locked re-proves the pin still satisfies it.
                  # chain: --registry > OPERON_REGISTRY > operon.toml [registry] > bundled seed;
                  # an explicit --registry on a project without a pin is recorded in operon.toml
  operon remove|update|install|tree|verify|publish   the rest of the package verbs
                  # update/install accept --registry too; verify pins --locked (manifest/lock drift = error)
  operon registry init|serve|default   stand up a read-only HTTP registry (W21-r1)
  operon mod ...         the same package verbs, explicit spelling (W19/W20/W23;
                  operon.toml manifest + operon.lock; docs/specs/REGISTRY.md)
  operon build f.op [--variant v] [-o out.op] [--bundle] [--native]
                  # --bundle/--native refuse honestly (W87/W85 planned; build emits specialized source)
  operon rna f.op patch.rna [--write] [--check] [--json] [--allow-comment-drop]
                  # --check (W68): validate the patch against the target, report
                  # span/node resolution + ambiguity + comment preflight +
                  # would_apply (--json); writes NOTHING, ever; exit 1 on any
                  # validation failure
  operon graph f.op [--json]
  operon run f.op --trace-grn trace.jsonl   # W095: JSONL GRN tick-stream
  operon doc f.op|dir [...] [-o outdir] [--json]
  operon watch f.op [args...]
  operon profile f.op [--chrome trace.json]
                  # --chrome (W096/W097-A): write per-call spans as a Chrome Trace
                  # Format .json (about://tracing / ui.perfetto.dev render it);
  operon ir|disasm f.op [--json]  W10: annotated OIR1 listing of compiled genes
  operon crispr f.op --knockout gene [--json]
  operon bench f.op [--iters n]
  operon run f.op --vm            A2/A3: bytecode VM on gene bodies
  operon run f.op --opt 1         W11: fold/thread/DCE passes (2 = full, 0 = off)
  operon run f.op --opt-passes fold,dce   W011: run exactly these passes
  operon version",
        env!("CARGO_PKG_VERSION")
    );
    // ast-grep-ignore: no-std-process-exit-in-core
    std::process::exit(2);
}
