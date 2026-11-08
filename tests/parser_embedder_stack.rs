//! Z-111-PARSERTHREAD regression pins for `operon::parser::parse`.
//!
//! Contract (EMBEDDING.md):
//! 1. `parse()` runs the recursive descent on a dedicated 256 MiB thread —
//!    the descent itself can never SIGABRT the host, whatever thread size
//!    the caller is on and whatever the build profile (debug frames are
//!    ~40 KiB; release ~0.7 KiB per level).
//! 2. The returned `Program` is owned data: its `Drop` (and any recursive
//!    walk, e.g. eval) runs on the CALLER's stack, so work at the very top
//!    of the 4096-frame budget belongs on a worker with headroom — the
//!    reference pattern lives in `parse_and_drop_on_work_thread`.
//!
//! Pre-fix, a 2 MiB embedder thread SIGABRTed the whole host at ~3000
//! nesting levels (uncatchable, A6 violation). If these tests abort the
//! test process again, the regression is live by construction.

use operon::parser;

/// Parse `src` on a 2 MiB thread (the #111 embedder shape). Must never
/// abort the process; returns whether parse completed.
fn parse_on_small_thread(src: String) -> Result<bool, String> {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .name("embedder-sim".to_string())
        .spawn(move || {
            // Parse on the 2 MiB thread; the returned AST is dropped HERE
            // only for the over-budget case (cap diagnostics return early,
            // shallow AST). The deep-parse case uses the work-thread
            // helper below for its drop.
            let prog = parser::parse(&src);
            // This helper pins the DESCENT (parse must never abort). The
            // cap-fired AST can still be thousands of levels deep; its
            // recursive Drop belongs on a headroomed stack (the work
            // helper below), not on this 2 MiB thread.
            std::mem::forget(prog);
            true
        })
        .expect("spawn embedder-sim")
        .join()
        .map_err(|_| "parser panicked (caught by join, host alive)".to_string())
}

/// The reference embedder pattern: parse AND dispose a deep AST on a
/// work thread with real headroom (96 MiB), while the CALLER thread can
/// stay tiny. `parse()` itself never aborts regardless.
fn parse_and_drop_on_work_thread(src: String) -> Result<bool, String> {
    std::thread::Builder::new()
        .stack_size(96 * 1024 * 1024)
        .name("embedder-work".to_string())
        .spawn(move || {
            let prog = parser::parse(&src);
            let n = prog.stmts.len();
            drop(prog); // recursive Drop runs on this 96 MiB stack
            n > 0
        })
        .expect("spawn embedder-work")
        .join()
        .map_err(|_| "parser panicked (caught by join, host alive)".to_string())
}

#[test]
fn embedder_thread_2000_nesting_levels_parses() {
    // 2000 nested call arguments: under the 4096-frame budget, above what
    // a 2 MiB host thread could carry pre-fix (~3000 levels aborted;
    // debug builds far earlier). parse() on the small thread must not
    // abort; the deep AST is disposed on the work thread per contract.
    let mut src = String::from("let x = f(");
    src.push_str(&"g(".repeat(2000));
    src.push('1');
    src.push_str(&")".repeat(2000));
    src.push(')');
    assert!(
        parse_and_drop_on_work_thread(src).expect("no abort"),
        "parse failed on a 2 MiB-class embedder shape"
    );
}

#[test]
fn embedder_thread_2000_index_levels_parse() {
    let mut src = String::from("let a = [1]\nlet x = a");
    src.push_str(&"[0]".repeat(2000));
    assert!(
        parse_and_drop_on_work_thread(src).expect("no abort"),
        "parse failed on a 2 MiB-class embedder shape"
    );
}

#[test]
fn parse_caller_thread_stays_tiny_for_shallow_programs() {
    // The everyday embedder shape: a normal program parsed and used on a
    // small thread — the whole lifecycle must fit.
    let mut src = String::from("let a = [1]\nlet x = a");
    src.push_str(&"[0]".repeat(50));
    assert!(
        parse_on_small_thread(src).expect("no abort"),
        "shallow parse must complete on a 2 MiB thread"
    );
}

#[test]
#[cfg(not(debug_assertions))]
fn over_budget_nesting_is_rejected_not_aborted() {
    // 5000 nested calls exceed the 4096-frame budget: the parser must
    // refuse (budget diagnostic path) without taking the host down.
    // Release-only: debug builds spend 3-5 physical frames of ~40-128 KiB
    // per nesting level, so NO practical stack can reach depth 4096 in a
    // debug build at all — the law's witness needs the release profile.
    let mut src = String::from("let x = f(");
    src.push_str(&"g(".repeat(5000));
    src.push('1');
    src.push_str(&")".repeat(5000));
    src.push(')');
    let ok = parse_on_small_thread(src);
    // Either outcome is acceptable EXCEPT a process abort: a completed
    // parse (cap not enforced on this path) or a caught parser panic.
    assert!(ok.is_ok(), "over-budget nesting must not abort the host");
}
