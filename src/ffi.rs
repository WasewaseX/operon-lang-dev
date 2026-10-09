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
/// rt_reset). compat-r2 (2026-10-03): no longer #[cfg(test)] — the
/// intern-table contract tests live in tests/intern_table.rs, their OWN
/// cargo test binary (cargo runs test binaries serially, so that process
/// has the table to itself; inside it, the tests serialize on their own
/// mutex). The lexer interns from other tests' threads inside the lib
/// test binary, so absolute assertions can never live there.
#[doc(hidden)]
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

/// Char-based Levenshtein distance over two Rust strings (pure Rust two-row
/// DP, no kernel hop). Z-119 (#119.3): the `distance`/`similar` BUILTINS are
/// char-based — the oracle's `edit_distance` walks Python str indices, i.e.
/// CHARACTERS, so `distance("héllo", "hello")` is 1 (one char deletion), not
/// the byte kernel's 2 (é is two UTF-8 bytes). The byte kernel above stays:
/// its only other callers are the parser's wobble ladder and the diagnostics
/// nearest-match, whose candidates are ASCII keywords (char law == byte law
/// there) and which need the fuel-blind kernel hop.
///
/// Budget discipline: the caller enforces the 10M-cell ceiling and the
/// per-operand cap on CHAR counts BEFORE calling (the builtins raise the
/// catchable `overflow` Stress; this fn is unguarded by design). Two rows of
/// `b.len()` cells, `i32` — the caller's budget makes overflow unreachable.
pub fn edit_distance_chars(a: &str, b: &str) -> i32 {
    if a == b {
        return 0;
    }
    let bc: Vec<char> = b.chars().collect();
    let lb = bc.len();
    if lb == 0 {
        return a.chars().count() as i32;
    }
    let mut prev: Vec<i32> = (0..=lb as i32).collect();
    let mut cur: Vec<i32> = vec![0; lb + 1];
    for (i, ca) in a.chars().enumerate() {
        cur[0] = i as i32 + 1;
        for j in 1..=lb {
            let cost = if ca == bc[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[lb]
}

/// Edit distance between two Rust strings (C++ bit-parallel kernel).
pub fn edit_distance(a: &str, b: &str) -> i32 {
    if a.len().saturating_mul(b.len()) > DP_CELL_BUDGET
        || a.len() > FFI_OPERAND_CAP
        || b.len() > FFI_OPERAND_CAP
    {
        return DP_BUDGET_SENTINEL;
    }
    // SAFETY: rt_edit_distance reads exactly a.len()/b.len() bytes from each
    // pointer for the duration of the call only; both pointers come from live
    // `&str` borrows held across the call, so they are valid and aligned.
    // Cell-count budget (DP_CELL_BUDGET) and per-operand caps (FFI_OPERAND_CAP)
    // are enforced above, so the kernel cannot over-read or overflow its
    // bit-parallel DP buffers.
    unsafe {
        rt_edit_distance(
            a.as_ptr() as *const c_char,
            a.len(),
            b.as_ptr() as *const c_char,
            b.len(),
        )
    } // ast-grep-ignore: no-unsafe-block-in-src
}

/// Codon-usage style score 0..100 (C++ kernel).
pub fn codon_score(s: &str) -> i32 {
    // SAFETY: rt_codon_score reads exactly s.len() bytes from the pointer for
    // the duration of the call only; the pointer comes from a live `&str`
    // borrow held across the call, and the kernel performs no writes.
    unsafe { rt_codon_score(s.as_ptr() as *const c_char, s.len()) } // ast-grep-ignore: no-unsafe-block-in-src
}

#[cfg(test)]
mod tests {
    use super::*;

    // compat-r2 (2026-10-03): the intern-table contract tests MOVED to
    // tests/intern_table.rs — a dedicated cargo test binary. History: the
    // table is process-global and the lexer interns into it from every
    // parse; TABLE_TEST_LOCK only ever serialized the ffi tests against
    // EACH OTHER, never against the lexer. The macOS arm64 scheduling
    // flare (2026-09-29, table_bytes saw 4114) and the linux CI flare
    // (bytes=16) are the same residual race. cargo executes test binaries
    // one at a time, so the dedicated binary is the only place absolute
    // assertions can hold exactly. The clock/kernel tests below stay —
    // they assert nothing about the table.

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
