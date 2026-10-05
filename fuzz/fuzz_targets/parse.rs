//! F5 (issue #49) — in-process coverage-guided target: lexer + parser.
//!
//! Contract (scripts/fuzz/TRIAGE.md vocabulary):
//!   * parse returning a Program with embedded diagnostics = clean
//!     (the Total Grammar parses-or-noted, never rejects)
//!   * a panic / signal / in-process exit = crash = BUG
//!
//! Seeds: tests/ + examples/ + fuzz_corpus/ (passed as read-only corpus
//! dirs to libFuzzer; see fuzz/README.md). The committed fuzz_corpus/
//! crash inputs must NOT crash here — the Span::of_token_word fix
//! (docs/FUZZING.md findings) is pinned by tests/diagnostics/ too.
//!
//! Inputs run on the big-stack worker (common.rs): same engine, same
//! shipped 4096 nesting threshold, a stack that fits the ASan build.

#![no_main]
use libfuzzer_sys::fuzz_target;

mod common;

fuzz_target!(|data: &[u8]| {
    // from_utf8_lossy keeps every byte string meaningful: invalid UTF-8
    // becomes U+FFFD, which is itself a multibyte shape worth fuzzing
    // (the C1 finding was a multibyte-boundary panic).
    let src = String::from_utf8_lossy(data).into_owned();
    common::with_big_stack(move || {
        let _prog = operon::parser::parse(&src);
    });
});
