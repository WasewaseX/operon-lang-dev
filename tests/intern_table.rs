//! Intern-table contract tests — compat-r2 (2026-10-03).
//!
//! This file is its OWN cargo test binary. Cargo executes test binaries
//! one at a time, so this process has the process-global intern table to
//! itself: no lexer from a sibling parse-heavy test can intern between
//! our reset and our assertions. Inside the binary the three tests
//! serialize on a mutex (cargo still runs one binary's tests on parallel
//! threads). Absolute assertions are therefore exact here — the same
//! assertions were flaky inside src/ffi.rs (macOS arm64 2026-09-29:
//! table_bytes saw 4114; linux CI 2026-10-03: table_bytes saw 16).

use operon::ffi::{intern, intern_count, reset_for_tests, symbols, table_allocs, table_bytes};
use std::sync::{Mutex, OnceLock};

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[test]
fn intern_ids_are_stable_and_equal_bytes_get_equal_ids() {
    let _guard = lock();
    reset_for_tests();
    let a = intern("promoter");
    let b = intern("promoter");
    let c = intern("terminator");
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, 0, "id 0 is reserved/invalid");
    assert_eq!(intern_count(), 2);
    // id determinism within a process: re-interning anything returns
    // the same id it had before
    assert_eq!(intern("promoter"), a);
}

#[test]
fn intern_table_tracks_bytes_allocs_and_symbols() {
    let _guard = lock();
    reset_for_tests();
    intern("abcd"); // 4 bytes, 1 alloc
    intern("abcdefgh"); // 8 bytes, 1 alloc
    assert_eq!(table_bytes(), 12);
    assert_eq!(table_allocs(), 2);
    let syms = symbols();
    assert_eq!(syms, vec!["abcd".to_string(), "abcdefgh".to_string()]);
}

#[test]
fn intern_handles_empty_and_unicode_and_long() {
    let _guard = lock();
    reset_for_tests();
    assert_eq!(intern(""), 1); // empty string is internable, ids start at 1
    let u1 = intern("基因");
    let u2 = intern("基因");
    assert_eq!(u1, u2);
    let big = "x".repeat(4096);
    assert_eq!(intern(&big), intern(&big));
    assert_eq!(intern_count(), 3);
    // the tracked bytes are exact: "" (0) + "基因" (6 UTF-8 bytes) + 4096
    assert_eq!(table_bytes(), 4102);
}
