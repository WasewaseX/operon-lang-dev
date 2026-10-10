#![no_main]
// z-fuzz-libfuzzer (#49): in-process target for the evaluator surface,
// fuel-capped. The interpreter's own step budget (the same fuel bound the
// CLI applies) contains runaways; a panic/abort escaping exec_block is the
// finding. Bytes that are not valid UTF-8 are skipped (the file layer
// already rejects them — see #113's read-path contract). The root env is
// the interpreter's own global (the same shape main() drives proofs with).
//
// LEAK POLICY (measured, deliberate): run this target with
// -detect_leaks=0. The value graph is process-lifetime by design — gene
// closures hold Rc env cycles the CLI process never frees (exit wipes
// them), so LeakSanitizer flags the interpreter's OWN teardown model, not
// a finding (first 40s run: 1785 bytes in 8 allocations on an
// empty-ish input — the global env itself). Panics/aborts/timeouts remain
// the contract; leaks are out of scope for a process-lifetime runtime.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(src) = std::str::from_utf8(data) {
        let prog = operon::parser::parse(src);
        let mut interp = operon::interp::Interp::new();
        // Fuel cap (the issue's "run-fuel-capped"): the documented 20M-step
        // run bound (fuzz_parser.py's TIMEOUT comment: the cap sits ABOVE
        // this bound; 25s wall > 20M steps on the CI runners). Interp::new()
        // defaults to 200M (the REPL pool) — an unbounded-feel target would
        // burn 2.5 minutes per adversarial input instead of ~15s.
        interp.step_budget = 20_000_000;
        let genv = interp.global.clone();
        // exec_block returns Err(Stress) on containment — an expected
        // outcome, never a finding. A panic escaping this call IS one.
        let _ = interp.exec_block(&genv, &prog.stmts);
    }
});
