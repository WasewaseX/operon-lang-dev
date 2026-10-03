//! M100 W076 — CI-verified embedding example (external-style crate).
//!
//! Depends on `operon` via `path = "../.."` — the same shape a downstream
//! embedder's Cargo.toml has. Mirrors docs/EMBEDDING.md §2 (tool-level) and
//! §3 (core-level). Built and run by `scripts/embed_example_check.sh` in CI
//! so the guide cannot drift from the public API.
//!
//! Run locally:  cargo run --manifest-path examples/embed/Cargo.toml

use operon::interp::{Caps, Env, Interp};
use operon::parser;
use operon::tools::{self, Opts};

fn main() {
    // ---- Core level: parse + evaluate a string (Total Grammar visible) ----
    // parse() NEVER fails on syntax; repairs are reported as notes (rung 1-4).
    let src = "gene add(a, b) { return a + b }\npromote(add(40, 2))";
    let prog = parser::parse(src);
    for n in &prog.notes {
        eprintln!("[repair rung {}] {}", n.rung, n.message);
    }
    let mut interp = Interp::new();
    let env = Env::new(None); // fresh root scope
    for stmt in &prog.stmts {
        if let Err(st) = interp.exec_stmt(&env, stmt) {
            eprintln!("stress contained: [{}] {}", st.kind, st.message);
            break;
        }
    }

    // ---- Tool level: run a file exactly like the CLI does ----
    // Caps::default() is the default-deny sandbox — the same contract as
    // `operon run` with no --allow flags (THREAT-MODEL.md). There is no
    // embedder backdoor.
    let demo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("demo.op");
    let sink = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let opts = Opts {
        cell: None,
        variant: None,
        rna: None,
        entry: Some("main".to_string()),
        use_ires: false,
        frame: None,
        args: vec![],
        quiet: true,
        caps: Caps::default(),
        profile: false,
        spans: false,
        stdout_sink: Some(sink.clone()),
    };
    let path = demo.to_str().unwrap_or("demo.op").to_string();
    let mut loaded = match tools::load_file(&path, &opts) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("load failed: {e}");
            std::process::exit(1);
        }
    };
    if let Err(st) = tools::run_entry(&mut loaded, &opts) {
        eprintln!("uncaught stress: [{}] {}", st.kind, st.message);
        std::process::exit(1);
    }
    tools::flush_notes(&loaded, true); // repair/semantic notes, CLI-shaped

    // Program promote() output was captured into the sink instead of hitting
    // process stdout — print it tagged so the CI gate can assert it.
    for line in sink.borrow().iter() {
        println!("captured: {line}");
    }
}
