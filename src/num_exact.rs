//! Exact Int<->Float numeric primitives (issue #130, Z-130-EXACT2).
//!
//! Sweep-3's last open item: #103 made Int<->Float *equality* exact, but
//! true division and ordering still rounded through lossy `as f64`
//! conversions above 2^53, so `9007199254740993 > 9007199254740992.0`
//! was false and `9007199254740993 / 7` divided the wrong numerator.
//! The oracle (Python) is exact on both axes; the engines align TO it.
//!
//! Two laws, one module, so the VM fold and the tree-walk interpreter
//! byte-match each other by construction:
//!
//! * [`div_i64_i64_exact`] — Python `long_true_divide` for i64/i64:
//!   the correctly-rounded f64 of the exact rational, computed with a
//!   single rounding (never the double rounding of
//!   `f64(x) / f64(y)`).
//! * [`cmp_i64_f64_exact`] — Python `float_richcompare` for i64 vs f64:
//!   exact ordering via the trunc decomposition (`f = trunc(f) + frac`,
//!   `|frac| < 1` decides ties without any 53-bit rounding).
//!
//! Deliberately NOT here: mixed Int/Float Add/Sub/Mul/Mod and float
//! division stay lossy — Python converts there too, and oracle parity
//! is the contract (issue #130's fix scope: Int/Int true division and
//! mixed ordering only).

/// The correctly-rounded f64 image of the rational `a / b` (Python
/// `int.__truediv__` semantics for 64-bit operands).
///
/// The caller owns the zero-divisor stress (interp apply_binop raises
/// "division by zero" before reaching here; the fold refuses to fold).
///
/// Fast paths:
/// * exact integer quotient — the i128 quotient converts with one
///   rounding (`i64 as f64` rounds-to-nearest);
/// * both magnitudes <= 2^53 — both images are exact f64s, so the IEEE
///   quotient of the two exact operands is itself correctly rounded.
///
/// General path: binary long division of |a|/|b| generates the top 54
/// significand bits plus a sticky bit, then rounds half-to-even exactly
/// like IEEE 754. The result magnitude is bounded by 2^63 and 2^-64, so
/// no overflow, no subnormal, and the final `m53 * 2^k` assembly is an
/// exact multiplication of a 53-bit integer by a power of two.
pub fn div_i64_i64_exact(a: i64, b: i64) -> f64 {
    debug_assert!(b != 0, "zero divisor is the caller's stress");
    // -(2^63) / -1 overflows every integer quotient type; Python yields
    // exactly 2^63, which IS an exact f64. Handle before `a % b`, whose
    // own (MIN, -1) remainder is a release-wrap/debug-panic trap.
    if a == i64::MIN && b == -1 {
        return 9223372036854775808.0;
    }
    // Exact integer quotient (one rounding on the exact value). The
    // i128 division cannot overflow: only the (MIN, -1) pair escapes
    // i64, and it is handled above. IEEE zero-sign law: 0 / -5 is -0.0
    // (the integer quotient loses the sign the float must carry).
    if a % b == 0 {
        let q = (a as i128) / (b as i128);
        if q == 0 && (a < 0) != (b < 0) {
            return -0.0;
        }
        return q as f64;
    }
    let na = a.unsigned_abs() as u128;
    let nb = b.unsigned_abs() as u128;
    const EXACT: u128 = 1 << 53;
    if na <= EXACT && nb <= EXACT {
        // Both images exact: the IEEE quotient is the correctly rounded
        // rational. Hot path — byte-identical to the pre-#130 behavior
        // for every small-magnitude division.
        return (a as f64) / (b as f64);
    }
    // General: single-round |a|/|b|, sign applied at the end (IEEE:
    // the sign of a quotient is the XOR of the operand signs).
    let negative = (a < 0) != (b < 0);
    let q = na / nb; // integer part
    let r = na % nb; // in (0, nb)
    if q > 0 {
        // e = floor(log2(v)): v in [q, q+1) shares q's top bit.
        let e = 127 - q.leading_zeros() as i32;
        let mag = if e >= 53 {
            // Top 54 bits all live in q. The dropped tail is
            // (q mod 2^(e-53)) + r/nb > 0 (r > 0), so sticky is set.
            let mant54 = (q >> (e - 53)) as u64;
            round54(mant54, true, e)
        } else {
            // 54 bits span q's integer bits and (53-e) fraction bits:
            // mant54 = floor(v * 2^(53-e)) = (q << t) | floor(r*2^t/nb).
            let t = (53 - e) as u32;
            let shifted = r << t; // r < 2^64, t <= 53 -> fits u128
            let frac = (shifted / nb) as u64;
            let rem2 = shifted % nb; // tail in [0, nb) -> sticky
            let mant54 = ((q << t) as u64) | frac;
            round54(mant54, rem2 > 0, e)
        };
        if negative {
            -mag
        } else {
            mag
        }
    } else {
        // v = r/nb in (0,1): double r until it crosses nb (k doublings
        // -> leading bit at 2^-k), then generate the 53 fraction bits
        // below the leading 1 by binary long division.
        let mut k = 0u32;
        let mut rem = r;
        while rem < nb {
            rem <<= 1;
            k += 1;
        }
        let e = -(k as i32);
        let mut rem = rem - nb; // the leading 1 is emitted; keep the tail
        let mut mant54: u64 = 1;
        for _ in 0..53 {
            mant54 <<= 1;
            rem <<= 1;
            if rem >= nb {
                mant54 |= 1;
                rem -= nb;
            }
        }
        let mag = round54(mant54, rem > 0, e);
        if negative {
            -mag
        } else {
            mag
        }
    }
}

/// Round a 54-bit fixed-point image (`mant54` = the top 54 significand
/// bits, top bit set) to the 53-bit f64 significand, half-to-even.
/// `sticky` ORs any tail below the guard bit; `e` is the exponent
/// context (the value's top bit sits at 2^e). Returns `m53 * 2^(e-52)`,
/// exact because a 53-bit integer times a power of two inside the
/// normal exponent window is representable.
fn round54(mant54: u64, sticky: bool, e: i32) -> f64 {
    let guard = mant54 & 1 == 1;
    let mut m53 = mant54 >> 1;
    let mut exp = e;
    if guard && (sticky || m53 & 1 == 1) {
        m53 += 1;
        if m53 == 1 << 53 {
            m53 = 1 << 52;
            exp += 1;
        }
    }
    (m53 as f64) * two_pow(exp - 52)
}

/// Exact power of two for the assembly range (|exp| <= 120 here, far
/// inside the normal f64 exponent window).
fn two_pow(exp: i32) -> f64 {
    f64::from_bits(((exp + 1023) as u64) << 52)
}

/// Exact i64-vs-f64 ordering (Python `float_richcompare` for
/// int<->float pairs, issue #130 hole 2).
///
/// NaN mirrors the old `partial_cmp().unwrap_or(Equal)` fallback
/// (Equal); the boolean operators guard NaN to all-false BEFORE the
/// compare, and sort call sites keep NaN->Equal byte-for-byte. +inf
/// orders above every i64, -inf below.
///
/// Exact core: with `t = trunc(f)`, `f = t + d` where `|d| < 1`. Any
/// integer at least 1 away from `t` decides on the i128 comparison
/// alone; `i == t` defers to the sign of `d` (f strictly above t means
/// the integer sitting exactly at t is BELOW f, and vice versa).
/// `|t| >= 2^127` cannot share range with any i64 (and every f64 that
/// far out is integral, so no fractional case hides there).
pub fn cmp_i64_f64_exact(i: i64, f: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if f.is_nan() {
        return Ordering::Equal;
    }
    if f.is_infinite() {
        return if f > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let t = f.trunc();
    // 2^127 as an f64 literal (exactly representable, power of two).
    const TWO_POW_127: f64 = 1.7014118346046923e38;
    if t.abs() >= TWO_POW_127 {
        return if f > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let ti = t as i128;
    match (i as i128).cmp(&ti) {
        Ordering::Equal => {
            let d = f - t;
            if d > 0.0 {
                Ordering::Less
            } else if d < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        ord => ord,
    }
}
