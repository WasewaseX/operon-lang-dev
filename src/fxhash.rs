//! fxhash.rs — W011-r2: a zero-crate FxHasher (the rustc-hash algorithm)
//! for the interpreter's INTERNAL hot maps. Performance infrastructure
//! only: hasher choice changes nothing a program can observe.
//!
//! Why this is parity-safe by construction: Rust's default RandomState is
//! seeded PER PROCESS, so HashMap iteration order already differs between
//! two runs of the same binary. Every output path that could have exposed
//! map order (grn_state, profile rendering, items() semantics) therefore
//! already canonicalizes (sorts) or is order-insensitive — the differential
//! harness (byte-identical stdout/stderr/rc across engines AND processes)
//! proves it continuously. Swapping SipHash for Fx changes iteration order
//! and nothing else; the same corpus gates this change exactly like any
//! other. What it buys: equal-key lookups on short identifier strings drop
//! from ~15-25ns (SipHash-1-3, per-process keyed) to ~2-4ns (multiply-
//! rotate-xor), on the maps the call funnel touches 5-6 times per call.

use std::hash::BuildHasherDefault;
use std::hash::Hasher;

/// 64-bit multiplicative seed (rustc-hash constant).
const SEED64: u64 = 0x51_7c_c1_b7_27_22_0a_95;
/// 32-bit multiplicative seed for the width-specific writes.
const SEED32: u32 = 0x9e_37_79_b9;

#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add_to_hash(&mut self, i: u64) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(SEED64);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // as_chunks::<8>: zero-copy &[u64] view; the remainder loop covers
        // the short-string tail (names are 1-8 bytes on the hot paths).
        let (chunks, rem) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add_to_hash(u64::from_ne_bytes(*c));
        }
        if !rem.is_empty() {
            // zero-padded tail: deterministic for a given byte length,
            // which is all an internal map ever needs
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            self.add_to_hash(u64::from_ne_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add_to_hash((i as u64) << 32 | SEED32 as u64);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_i8(&mut self, i: i8) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_i16(&mut self, i: i16) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.add_to_hash((i as u64) << 32 | SEED32 as u64);
    }

    #[inline]
    fn write_i64(&mut self, i: i64) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn write_isize(&mut self, i: isize) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// The builder type used across the interpreter's internal maps.
pub type FxBuild = BuildHasherDefault<FxHasher>;

/// Convenience constructor: an empty Fx-hashed map (mirrors HashMap::new).
#[allow(dead_code)]
pub fn fx_map<K, V>() -> std::collections::HashMap<K, V, FxBuild> {
    std::collections::HashMap::with_hasher(FxBuild::default())
}

/// Convenience constructor: an empty Fx-hashed set.
#[allow(dead_code)]
pub fn fx_set<T>() -> std::collections::HashSet<T, FxBuild> {
    std::collections::HashSet::with_hasher(FxBuild::default())
}
