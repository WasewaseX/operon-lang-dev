#![no_main]
// z-fuzz-libfuzzer (#49): in-process coverage-guided target for the parser.
// lib.rs's module tree backs this crate; parse() is the exact entry the CLI
// uses. Panics/aborts here are real parser bugs the black-box lane could
// only find by luck (the C1 Span::of_token_word multibyte panic class).
// Seeds: tests/ + examples/ via the corpus dir (cargo fuzz runs -merge with
// the seed corpus once, then the nightly job explores).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = std::str::from_utf8(data);
    if let Ok(src) = src {
        // depth caps + fuel live inside the parser; a panic escaping this
        // call is the finding (libFuzzer saves the input + the backtrace).
        let _ = operon::parser::parse(src);
    }
});
