#![no_main]
// z-fuzz-libfuzzer (#49): in-process target for the typed checker surface.
// check_program() must NEVER panic on any parseable input (findings are
// typed Sev, not panics; the gate is a hard-abort only via the CLI wrapper
// after the check returns).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(src) = std::str::from_utf8(data) {
        let prog = operon::parser::parse(src);
        let _ = operon::typeck::check_program(&prog);
    }
});
