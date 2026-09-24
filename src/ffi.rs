//! ffi.rs — Rust bindings to the C runtime kernel + C++ codon kernel.
//! Every identifier interned by the lexer lives in the C table; bit-parallel
//! edit distance and codon scoring run in the C++ kernel. C/C++ are
//! load-bearing, not decorative.

use std::os::raw::c_char;

extern "C" {
    pub fn rt_intern(s: *const u8, n: usize) -> u32;
    pub fn rt_intern_count() -> u32;
    pub fn rt_now_ns() -> f64;
    pub fn rt_arena_used() -> usize;
    pub fn rt_alloc_count() -> u64;
    pub fn rt_edit_distance(a: *const c_char, la: usize, b: *const c_char, lb: usize) -> i32;
    pub fn rt_codon_score(s: *const c_char, n: usize) -> i32;
}

/// Intern a Rust string, returning the stable id.
pub fn intern(s: &str) -> u32 {
    unsafe { rt_intern(s.as_ptr(), s.len()) }
}

/// Edit distance between two Rust strings (C++ bit-parallel kernel).
pub fn edit_distance(a: &str, b: &str) -> i32 {
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

/// Monotonic nanoseconds (C kernel).
pub fn now_ns() -> f64 {
    unsafe { rt_now_ns() }
}
