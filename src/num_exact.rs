//! Z-130-EXACT2: exact Int<->Float mixing above 2^53 (extends #103).
//!
//! Two laws, both mirroring Python's semantics for i64-ranged integers —
//! the oracle IS Python, so the engines align TO it:
//!
//! L1 (true division): Int / Int is computed EXACTLY and rounded ONCE to
//! the nearest double (Python `long_true_divide`), never via lossy `as f64`
//! operand conversion. `9007199254740993 / 3` must be `3002399751580331.0`
//! (the exact quotient), not the `...330.5` produced when the numerator
//! collapses to `9007199254740992.0` before the divide.
//!
//! L2 (ordering): mixed Int<->Float comparison is EXACT (Python
//! `float_richcompare`). `9007199254740993 > 9007199254740992.0` must be
//! TRUE — the two values genuinely differ by one unit in the last integer
//! place, but narrow both to f64 and they compare equal.
//!
//! What stays lossy (deliberately, Python-parity): mixed Int/Float
//! Add/Sub/Mul/Pow/Mod and any Float/Float or Int/Float division — Python
//! converts the int operand to double there too. Only int/int true division
//! and int-vs-float ordering/equality carry exact laws (#103 delivered the
//! equality half via int_float_exact_eq).
//!
//! Both helpers are total and branch-cheap on the common in-range path;
//! the engines call them from `apply_binop` (shared by the tree-walk and
//! the VM) and from the VM's constant folder, so every lane stays
//! byte-identical by construction.

/// Correctly-rounded f64 quotient of two i64s (L1).
///
/// Caller guarantees `b != 0` (the runtime zero-divisor stress contract is
/// enforced at the call site, preserving the exact Stress kind/message).
/// Adapted from CPython's `long_true_divide` restricted to i64 operands:
/// one normalized u128 division yields the 53-bit quotient prefix plus an
/// exact remainder, and the remainder drives round-half-even — so the
/// result is the double nearest to the true rational a/b.
pub fn div_i64_i64_exact(a: i64, b: i64) -> f64 {
    let sign = if (a < 0) == (b < 0) { 1.0 } else { -1.0 };
    let a = a.unsigned_abs(); // u64; |i64::MIN| = 2^63
    let b = b.unsigned_abs();
    if a == 0 {
        return 0.0 * sign; // ±0.0 per the sign law
    }
    // Normalize: park a's leading bit at 2^126, so av/bv = (a/b) * 2^shift_back.
    // a < 2^64 and shift_back <= 126 keep av < 2^128 (u128-safe).
    let abits = 64 - a.leading_zeros() as i32; // ∈ [1, 64]
    let shift_back = (126 - (abits - 1)) as u32; // ∈ [63, 126]
    let av = (a as u128) << shift_back;
    let bv = b as u128;
    let q = av / bv; // exact floor
    let r = av % bv; // exact remainder < bv <= 2^63
                     // Round q (+ tail r/b) to 53 significant bits, half-even on the exact tail.
    let ql = 128 - q.leading_zeros() as i32; // ∈ [63, 127] by construction
    let discard = (ql - 53).max(0) as u32;
    let mut cand = q >> discard; // <= 2^53, exactly representable in f64
    if discard > 0 {
        let frac = q & ((1u128 << discard) - 1);
        // 2*(frac + r/b) vs 1  ⟺  2*frac*bv + 2r vs bv<<discard — all u128-safe:
        // frac < 2^discard <= 2^11, bv <= 2^63 → lhs < 2^77; rhs < 2^75.
        let lhs = (frac << 1) * bv + (r << 1);
        let rhs = bv << discard;
        if lhs > rhs {
            cand += 1;
        } else if lhs == rhs {
            cand += cand & 1; // exact tie → round to even
        }
    } else {
        // ql <= 53: cand == q exactly; the tail is r/b alone.
        let lhs = r << 1;
        if lhs > bv {
            cand += 1;
        } else if lhs == bv {
            cand += cand & 1;
        }
    }
    // cand <= 2^53 (a +1 can carry at most to 2^53, itself exact in f64).
    // exp ∈ [-126, 1]; the product is exact (power-of-two scaling, and the
    // magnitude stays far from subnormal/overflow: |a/b| ∈ [2^-63, 2^63]).
    let exp = discard as i32 - shift_back as i32;
    sign * cand as f64 * 2f64.powi(exp)
}

/// Floored float remainder (Python `%` for floats, issue #131,
/// Z-131-FLOORMOD): CPython's own `float_rem` algorithm — C fmod
/// (truncated remainder, sign follows the DIVIDEND) adjusted into the
/// divisor's sign class. The previous engine formula
/// `a.rem_euclid(b.abs()) * b.signum()` looked like the floored law but
/// computes a euclidean-magnitude x divisor-sign hybrid: 5.5 % -2.0 was
/// -1.5 where the SPEC (and Python, and the engine's own Int `%`) say
/// -0.5.
///
/// The issue's suggested `a - (a/b).floor()*b` is ALSO wrong at the IEEE
/// edges: for b = +/-inf it produces NaN where Python propagates the
/// fmod image (5.5 % -inf = -inf, 5.5 % inf = 5.5), and for a = inf it
/// NaNs where Python also NaNs (inf % finite = NaN) — only the fmod+adjust
/// form matches Python on every edge: NaN in -> NaN, -0.0 dividend
/// preserved (m != 0.0 is false for -0.0, so no adjustment touches it).
///
/// The caller owns the zero-divisor stress ("modulo by zero"), exactly
/// as for the Int arm; divmod inherits this law by construction (it
/// reuses the `%` operator for r).
pub fn mod_f64_floored(a: f64, b: f64) -> f64 {
    let m = a % b; // Rust f64 Rem == C fmod
    if m != 0.0 {
        if (m < 0.0) != (b < 0.0) {
            m + b
        } else {
            m
        }
    } else {
        // CPython's zero law: the zero remainder takes the DIVISOR's
        // sign (0.0 % -2.0 is -0.0, -0.0 % 2.0 is 0.0) — floored law
        // carried into the zeros, exactly like the Copysign branch of
        // CPython's float_rem.
        m.copysign(b)
    }
}

/// Python's complete `float_divmod` (CPython algorithm, issue #131
/// follow-through): the q half is NOT `floor(a / b)` — that rounds
/// through the division's rounding error and lands on the wrong integer
/// whenever a/b sits on a boundary (1.0 // 1e-10 must be 9999999999,
/// but floor(1.0/1e-10) = 1e10 because the division rounded up).
/// CPython derives q from the EXACT remainder: `div = (a - r) / b`,
/// floored, then a > 0.5 fixup against the corrected-remainder
/// quotient. r is the [`mod_f64_floored`] law bit-for-bit, so the
/// `//` operator, the `%` operator, and `divmod` are one consistent
/// Python-parity triple by construction.
///
/// Zero divisors never reach here (both operator arms stress first,
/// preserving the exact Stress kind/message).
pub fn divmod_f64(a: f64, b: f64) -> (f64, f64) {
    let mut m = a % b; // raw fmod, sign follows the dividend
    let div = (a - m) / b; // raw (unfloored) quotient
    let mut q = div.floor();
    if div - q > 0.5 {
        q += 1.0;
    }
    if m != 0.0 {
        if (m < 0.0) != (b < 0.0) {
            m += b;
            q -= 1.0;
        }
    } else {
        m = m.copysign(b);
    }
    // Zero-quotient sign law: a zero q takes the sign of the true
    // quotient a/b (|true q| < 1 preserves it; the IEEE (a - m)
    // subtraction loses it — x - x = +0.0 even for -0.0 dividends, and
    // a m-adjustment that lands q on zero inherits the sign of the
    // pre-adjustment (a - m), not of a/b). Python: divmod(-1.0, -2.0)
    // = (+0.0, -1.0), divmod(0.0, -1.0) = (-0.0, 0.0).
    if q == 0.0 {
        q = 0.0_f64.copysign(a / b);
    }
    (q, m)
}

/// Exact three-way comparison of an i64 against an f64 (L2).
///
/// Python `float_richcompare` law: decompose the double into its exact
/// form (sign × integer mantissa × 2^exp) and compare against the int with
/// integer arithmetic. NaN never reaches the ordering path (the runtime
/// NaN guard returns false for every ordering operator before this call);
/// ±inf and the ±2^63 magnitude boundaries are short-circuited exactly.
pub fn cmp_i64_f64_exact(a: i64, b: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if b.is_nan() {
        // Unreachable by the runtime contract; keep the total function honest
        // (the old lossy path also collapsed NaN comparisons to Equal here).
        return Ordering::Equal;
    }
    if b == f64::INFINITY {
        return Ordering::Less;
    }
    if b == f64::NEG_INFINITY {
        return Ordering::Greater;
    }
    // b >= 2^63 sits strictly above every i64 (2^63 > i64::MAX).
    if b >= 9223372036854775808.0 {
        return Ordering::Less;
    }
    // b < -2^63 sits strictly below every i64; b == -2^63 IS i64::MIN and
    // must fall through to the exact path (the asymmetric guard is the law).
    if b < -9223372036854775808.0 {
        return Ordering::Greater;
    }
    let bits = b.to_bits();
    let bneg = bits >> 63 == 1;
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let mant = (bits & ((1u64 << 52) - 1)) as i128;
    // b = ±mb * 2^eb exactly; here |b| < 2^63.
    let (mb, eb) = if raw_exp == 0 {
        (mant, -1074i32) // subnormal (also ±0.0 with mb == 0)
    } else {
        (mant | (1i128 << 52), raw_exp - 1075)
    };
    if eb >= 0 {
        // b is an integer < 2^63 in magnitude: plain i128 comparison.
        let target = mb << eb; // < 2^63 by the short-circuit above
        let b_int = if bneg { -target } else { target };
        return (a as i128).cmp(&b_int);
    }
    // eb < 0: b = ±mb / 2^k with k = -eb ∈ [1, 1074].
    let k = (-eb) as u32;
    if k > 63 {
        // |a| * 2^k >= 2^k > 2^63 > 2^53 > mb for a != 0, so magnitudes
        // decide by sign alone; a == 0 has the smaller magnitude (or ties
        // exactly when b == ±0.0, mb == 0).
        if a == 0 {
            return if mb == 0 {
                Ordering::Equal // b is ±0.0 and a == 0 — the one true tie
            } else if !bneg {
                Ordering::Less // 0 < +tiny
            } else {
                Ordering::Greater // -tiny < 0
            };
        }
        // a != 0: |a| >= 1 > |b| — a's magnitude wins, sign of a decides.
        return if a > 0 {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    // 1 <= k <= 63: exact integer comparison a * 2^k vs ±mb — for the
    // negative-float side the compared target is -mb (av vs -mb), NOT
    // (-av) vs mb (that inverts the ordering: -1 vs -0.5 must be Less).
    let av = (a as i128) << k; // |a| < 2^63 → av < 2^126
    if bneg {
        av.cmp(&(-mb))
    } else {
        av.cmp(&mb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- L1: the issue's table (ground truth computed by Python) ----
    #[test]
    fn div_issue_table() {
        assert_eq!(div_i64_i64_exact(9007199254740993, 3), 3002399751580331.0);
        assert_eq!(div_i64_i64_exact(9007199254740993, 7), 1286742750677284.75);
        assert_eq!(
            div_i64_i64_exact(9007199254740993, 7).to_string(),
            "1286742750677284.8"
        );
    }

    #[test]
    fn div_exact_and_common() {
        assert_eq!(div_i64_i64_exact(9, 2), 4.5);
        assert_eq!(div_i64_i64_exact(-9, 2), -4.5);
        assert_eq!(div_i64_i64_exact(9, -2), -4.5);
        assert_eq!(div_i64_i64_exact(-9, -2), 4.5);
        assert_eq!(div_i64_i64_exact(1, 3), 1.0 / 3.0);
        assert_eq!(div_i64_i64_exact(-1, 3), -(1.0 / 3.0));
        assert_eq!(div_i64_i64_exact(0, 5), 0.0);
        assert_eq!(div_i64_i64_exact(0, -5), 0.0 * -1.0); // -0.0
        assert!(div_i64_i64_exact(0, -5).is_sign_negative());
        // powers of two stay exact through the whole range
        assert_eq!(
            div_i64_i64_exact(1, 4611686018427387904),
            1.0 / 2f64.powi(62)
        );
        assert_eq!(
            div_i64_i64_exact(4611686018427387904, 1),
            4611686018427387904.0
        );
        assert_eq!(div_i64_i64_exact(1, 3), 1.0 / 3.0);
    }

    #[test]
    fn div_i64_min_edges() {
        // |i64::MIN| = 2^63 is exactly a power of two — division by odd
        // small ints must still be correctly rounded from the u128 path.
        assert_eq!(div_i64_i64_exact(i64::MIN, 1), -9223372036854775808.0);
        assert_eq!(div_i64_i64_exact(i64::MIN, -1), 9223372036854775808.0); // 2^63, exact in f64
        assert_eq!(div_i64_i64_exact(i64::MIN, 2), -4611686018427387904.0);
        // Python: -9223372036854775808 / 3 == -3.0744573456182586e+18
        assert_eq!(
            format!("{:.7e}", div_i64_i64_exact(i64::MIN, 3)),
            "-3.0744573e18"
        );
        assert_eq!(
            div_i64_i64_exact(i64::MIN, 3).to_bits(),
            (-3074457345618258602.6666666666f64).to_bits()
        );
    }

    // ---- L2: the issue's table ----
    #[test]
    fn cmp_issue_table() {
        assert_eq!(
            cmp_i64_f64_exact(9007199254740993, 9007199254740992.0),
            std::cmp::Ordering::Greater
        );
        // sanity: the lossy cast IS the bug — 9007199254740993i64 as f64
        // collapses onto 9007199254740992.0, which is why as_floats compared
        // these two distinct values as equal
        assert_eq!(9007199254740993i64 as f64, 9007199254740992.0f64);
        assert_eq!(
            cmp_i64_f64_exact(9007199254740992, 9007199254740992.0),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn cmp_boundaries() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_i64_f64_exact(i64::MAX, 9.223372036854776e18), Less);
        assert_eq!(cmp_i64_f64_exact(i64::MAX, f64::INFINITY), Less);
        assert_eq!(cmp_i64_f64_exact(i64::MIN, f64::NEG_INFINITY), Greater);
        assert_eq!(cmp_i64_f64_exact(i64::MIN, -9223372036854775808.0), Equal);
        assert_eq!(
            cmp_i64_f64_exact(i64::MIN + 1, -9223372036854775808.0),
            Greater
        );
        assert_eq!(cmp_i64_f64_exact(0, 0.0), Equal);
        assert_eq!(cmp_i64_f64_exact(0, -0.0), Equal);
        assert_eq!(cmp_i64_f64_exact(1, 1.0), Equal);
        assert_eq!(cmp_i64_f64_exact(1, 0.5), Greater);
        assert_eq!(cmp_i64_f64_exact(-1, -0.5), Less);
        assert_eq!(cmp_i64_f64_exact(0, 1e-300), Less);
        assert_eq!(cmp_i64_f64_exact(0, -1e-300), Greater);
        assert_eq!(cmp_i64_f64_exact(5, 2.5), Greater);
        assert_eq!(cmp_i64_f64_exact(2, 2.5), Less);
        assert_eq!(cmp_i64_f64_exact(-5, -2.5), Less);
    }
}
