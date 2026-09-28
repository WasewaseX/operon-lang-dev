//! ffi.rs, native substrate of the Operon toolchain (sec-r2, audit A15).
//!
//! History: interning/arena/clock lived in a C kernel (runtime/operon_rt.c).
//! Audit wave 1 proved that kernel's raw pointers were the single largest
//! memory-safety surface of the whole project, two ASan-confirmed UAF/SEGV
//! classes (C-5/C-6) traced to it, AND that the lexer discards every intern
//! result (write-only). The evidence-based verdict (MASTER-PLAN A15) was to
//! port the table into Rust and delete the C kernel outright: the entire UAF
//! class is now structurally impossible, and the Win32/POSIX shim layer went
//! with it.
//!
//! What remains native and load-bearing:
//!   * codon_kernel.cpp, bit-parallel Myers edit distance + codon scoring.
//!     Pure computation, no allocation, no pointers retained, guarded by
//!     DP_CELL_BUDGET before every hop. This kernel EARNED its place.
//!
//! The Rust symbol table below is the canonical record of every identifier
//! the lexer has seen. Consumers: the `memory()` builtin (interns/bytes/
//! allocs), the REPL `:symbols` inspector, and A8 LSP goto-definition will
//! build on it. It is no longer write-only.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

// --------------------------------------------------------------- symbol table

#[derive(Default)]
struct InternTable {
    /// canonical spelling -> stable id (ids start at 1; 0 is reserved/invalid)
    map: HashMap<Box<str>, u32>,
    /// id -> canonical spelling (dense index; slot id-1)
    slots: Vec<Box<str>>,
    /// live bytes held by the table (sum of slot capacities)
    bytes: usize,
    /// allocation ops served (mirrors the old rt_alloc_count semantics)
    allocs: u64,
}

static INTERN: OnceLock<Mutex<InternTable>> = OnceLock::new();

fn table() -> &'static Mutex<InternTable> {
    INTERN.get_or_init(|| Mutex::new(InternTable::default()))
}

/// Intern a string, returning its stable id. Equal bytes => equal id.
/// Memory safety: no raw pointers, no arena, no manual reclaim, the table
/// is ordinary Rust ownership, so use-after-free cannot occur by construction.
pub fn intern(s: &str) -> u32 {
    let mut t = table().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&id) = t.map.get(s) {
        return id;
    }
    let id = (t.slots.len() + 1) as u32;
    let owned: Box<str> = s.into();
    t.bytes += owned.len();
    t.allocs += 1;
    t.slots.push(owned);
    t.map.insert(s.into(), id);
    id
}

/// Number of distinct interned strings in this process.
pub fn intern_count() -> u32 {
    table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .slots
        .len() as u32
}

/// Live bytes held by the symbol table (the `memory()` builtin's "arena").
pub fn table_bytes() -> usize {
    table().lock().unwrap_or_else(|e| e.into_inner()).bytes
}

/// Allocation ops served by the table (the `memory()` builtin's "allocs").
pub fn table_allocs() -> u64 {
    table().lock().unwrap_or_else(|e| e.into_inner()).allocs
}

/// Canonical spellings, oldest first (REPL `:symbols` inspector).
pub fn symbols() -> Vec<String> {
    table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .slots
        .iter()
        .map(|b| b.to_string())
        .collect()
}

/// Reset the table (run-scoped semantics preserved from the C kernel's
/// rt_reset; used by tests to prove reset leaves a clean table).
#[cfg(test)]
pub fn reset_for_tests() {
    let mut t = table().lock().unwrap_or_else(|e| e.into_inner());
    *t = InternTable::default();
}

// --------------------------------------------------------------------- clock

static EPOCH: OnceLock<std::time::Instant> = OnceLock::new();

/// Monotonic nanoseconds since first use in this process.
/// Instant is guaranteed monotonic on every supported platform, replacing
/// the hand-written QPC/clock_gettime shim (and its Win32 branch) entirely.
pub fn now_ns() -> f64 {
    let epoch = EPOCH.get_or_init(std::time::Instant::now);
    epoch.elapsed().as_nanos() as f64
}

// ------------------------------------------------------- C++ codon kernel ABI

use std::os::raw::c_char;

extern "C" {
    pub fn rt_edit_distance(a: *const c_char, la: usize, b: *const c_char, lb: usize) -> i32;
    pub fn rt_codon_score(s: *const c_char, n: usize) -> i32;
}

/// DP cell budget for the C++ edit-distance kernel (sec-r1, audit C-1/C-8).
/// An FFI call is fuel-blind, no step/fuel accounting can interrupt it, so
/// the O(la*lb) budget must be enforced BEFORE the hop. One guard here
/// protects `distance()`, `similar()`, the wobble-repair paths, and the
/// parser's suggestion engine, which all funnel through this function.
pub const DP_CELL_BUDGET: usize = 10_000_000;

/// Returned instead of a distance when the input pair exceeds the budget.
/// No real distance can be i32::MAX, and every comparison site treats a
/// bigger distance as a worse match, so over-budget pairs simply never
/// win a "nearest" contest.
pub const DP_BUDGET_SENTINEL: i32 = i32::MAX;

/// sec-r3 (re-audit #8): per-operand cap. The cell budget alone admits
/// pathological shapes (10 MB × 1 byte = 10 M cells) whose bit-parallel
/// wavefront costs tens of ms per hop, fuel-blind. Identifiers and
/// suggestion candidates are tiny; 64 KiB per operand is generous headroom.
pub const FFI_OPERAND_CAP: usize = 64 * 1024;

/// Edit distance between two Rust strings (C++ bit-parallel kernel).
pub fn edit_distance(a: &str, b: &str) -> i32 {
    if a.len().saturating_mul(b.len()) > DP_CELL_BUDGET
        || a.len() > FFI_OPERAND_CAP
        || b.len() > FFI_OPERAND_CAP
    {
        return DP_BUDGET_SENTINEL;
    }
    unsafe {
        rt_edit_distance(
            a.as_ptr() as *const c_char,
            a.len(),
            b.as_ptr() as *const c_char,
            b.len(),
        )
    }
}

/// Codon-usage style score 0..100 (C++ kernel).
pub fn codon_score(s: &str) -> i32 {
    unsafe { rt_codon_score(s.as_ptr() as *const c_char, s.len()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_ids_are_stable_and_equal_bytes_get_equal_ids() {
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
        reset_for_tests();
        assert_eq!(intern(""), 1); // empty string is internable
        let u1 = intern("基因");
        let u2 = intern("基因");
        assert_eq!(u1, u2);
        let big = "x".repeat(4096);
        assert_eq!(intern(&big), intern(&big));
        assert_eq!(intern_count(), 3);
    }

    #[test]
    fn clock_is_monotonic() {
        let t0 = now_ns();
        let t1 = now_ns();
        assert!(t1 >= t0, "clock must never go backwards");
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(now_ns() - t1 >= 1_000_000.0, "2ms elapsed must show up");
    }

    #[test]
    fn edit_distance_kernel_still_sane() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", "abc"), 0);
    }
}
