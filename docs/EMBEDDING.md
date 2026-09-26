# OPERON EMBEDDING GUIDE (Rust public API)

W076 of the M100 program · v1.0.0 · 2026-09-26 · owner: sz (dev-3)
Baseline: main @ dd76caa · `operon` is a plain library crate: every module is public
(`src/lib.rs`: `ast ffi genes graph interp lexer ls parser tools value`).

---

## 0. What embedding gets you today

The **entire language core** runs inside your Rust process with no subprocess and no CLI:
parse (Total Grammar — every input is accepted at some repair rung), evaluate, capture
diagnostics/notes, and drive the interpreter's gene-expression machinery programmatically.
The binary `operon` is a thin shell over exactly these entry points
(`src/main.rs` is ~1200 lines of CLI wiring you can skip).

Versioning: the API is de-facto stable but NOT yet semver-hardened — pin an exact `operon`
version in your `Cargo.toml` (compat policy: `docs/specs/COMPATIBILITY.md`, D-011).
A curated `prelude` + semver pledge is the W076 follow-up once the type system (W001)
settles value signatures.

## 1. Two integration levels

| Level | Use when | Entry points |
|-------|----------|--------------|
| **Tool-level** (recommended) | you want "run this file/string like the CLI does" — file loading, `.cell`, std resolution, entry-gene dispatch, notes flush | `tools::load_file`, `tools::run_entry`, `tools::Opts` |
| **Core-level** | you want to drive parse trees / interpreter yourself (custom REPLs, analysis, sandboxes) | `parser::parse`, `interp::Interp::new`, `Interp::exec_stmt`, `genes::load_module` |

## 2. Tool-level: run a file like the CLI

```rust
use operon::tools::{self, Opts};

// Opts is Clone (no Default yet — fields are public, build it explicitly)
let opts = Opts {
    cell: None,           // Some(".cell".into()) to override discovery
    variant: None,        // splice variant selection
    rna: None,            // .rna patch path (run-time edit surface)
    entry: None,          // Some("main".into()) to dispatch an entry gene
    use_ires: false,
    frame: None,
    args: vec![],
    quiet: false,
    caps: operon::interp::Caps::default(), // default-deny sandbox (CLI equivalent: no --allow flags)
    profile: false,       // true = time top-level statements during load
    stdout_sink: None,    // Some(Rc<RefCell<Vec<String>>>) to capture promote() output
};
let mut loaded = match tools::load_file("app/main.op", &opts) {
    Ok(l) => l,
    Err(e) => { eprintln!("load failed: {e}"); return; }
};
// .cell resolved, modules loaded, top-level executed (profile-gated as in the CLI)
match tools::run_entry(&mut loaded, &opts) {
    Ok(_value) => {}
    Err(stress) => { eprintln!("uncaught stress: [{}] {}", stress.kind, stress.message); }
}
tools::flush_notes(&loaded, false); // repair/semantic notes -> stderr, CLI-shaped
```

Notes:
- `load_file` is the SAME path the CLI uses — exe-relative std resolution, module loading,
  capability wiring. Sandbox defaults (default-deny) apply exactly as documented in
  THREAT-MODEL.md; there is no "embedder backdoor" (deliberately).
- Notes ordering and shapes are byte-stable per DETERMINISM.md — safe to diff in tests.

## 3. Core-level: parse + evaluate a string

```rust
use operon::parser;
use operon::interp::Interp;

let prog = parser::parse("gene add(a, b) { return a + b }\npromote(add(40, 2))");
for n in &prog.notes {
    eprintln!("[repair rung {}] {}", n.rung, n.message); // Total Grammar transparency
}
let mut interp = Interp::new();
let env = operon::interp::Env::new(None); // fresh root scope
for stmt in &prog.stmts {
    if let Err(st) = interp.exec_stmt(&env, stmt) {
        eprintln!("stress contained: [{}] {}", st.kind, st.message);
        break;
    }
}
```

Notes:
- `parser::parse` NEVER fails on syntax (Total Grammar); inspect `prog.notes` for what was
  repaired. Rung meanings: SPEC §2 (canonical → repairable → recoverable → warning → hard).
- `Interp::new()` gives default caps/fuel; the struct fields are public — read
  `src/interp.rs`'s head for the current knobs (fuel, mem ceiling, profiling flags).
  Setting caps programmatically is equivalent to the CLI flags; same contracts.

## 4. Diagnostics & introspection

- `interp.notes` — all repair/semantic notes (rung, line, message).
- `interp.call_counts` / `interp.call_time_self` — the profiler's data (see `profile --json`
  shape, W096).
- `operon::graph::collect(&prog.stmts)` — static regulate-network export (W094), useful for
  pipeline tooling.
- `operon::genes::apply_rna_checked` — the `.rna` patch engine with a full per-rule report
  (W068) — embeddable for editor/automation tooling.

## 5. What is deliberately NOT exposed yet

1. **Registering Rust functions as builtins** (host functions): the plumbing exists
   internally (`Native` values) but the public signature is not frozen — W076 follow-up
   after W004 (traits) stabilizes the calling convention. Until then, bridge via `.op`
   genes or the `py()` substrate.
2. **C ABI** (W077): design note stage — `docs/design/` per TODO-100.
3. **Sandbox profiles as data**: caps are settable per-field; a profile builder API waits
   for W066's `.cell` schema to define the shared vocabulary.

## 6. CI-verified embedding example

`examples/embed/` (an external-style crate depending on `operon` via path) is the M100
follow-up for this level — it will be compiled in CI to keep this guide honest. Until it
lands, the snippets above are exercised implicitly by `tests/repl.rs` and the tool layer.
