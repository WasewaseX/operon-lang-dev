//! F5 (issue #49) — in-process coverage-guided target: the correctness
//! grader over parsed input.
//!
//! Contract: parse + `typeck::check_program` must never panic on any byte
//! string. Findings (errors/warnings) are the checker WORKING — only a
//! panic / signal / in-process exit is a crash (TRIAGE vocabulary).
//!
//! parse and check stay leak-checked (acyclic ASTs; the Rc-cycle REMAIN
//! documented in FUZZING.md harness note 2 lives on the run surface).

#![no_main]
use libfuzzer_sys::fuzz_target;

mod common;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data).into_owned();
    common::with_big_stack(move || {
        let prog = operon::parser::parse(&src);
        let _findings = operon::typeck::check_program(&prog);
    });
});
