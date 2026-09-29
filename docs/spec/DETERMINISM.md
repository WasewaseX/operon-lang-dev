# OPERON DETERMINISM CONTRACT

W088/W089/W090 of the M100 program · v1.0.0 · 2026-09-26 · owner: sz (dev-3)
Baseline: main @ dd76caa · evidence links point at committed tests, not prose promises.

---

## 0. Why this document exists

Operon's whole verification architecture, the differential harness (Rust core vs Python
oracle, byte-identical stdout), 83+ proof frames, and the redteam suite, only works because
runs are **reproducible**. This document states exactly what reproducibility is promised,
where it is deliberately NOT promised, and how to replay any pinned output. It exists so
that (a) contributors know which changes may alter outputs (and must therefore re-pin
corpus files), and (b) users know what they may build on top of a seeded run.

## 1. The promise, in one line

> Same binary version + same source + same seed + same flags + same `.cell` ⇒ **byte-identical
> stdout**. This is enforced continuously by the differential harness (128/128 MATCH at
> dd76caa) and has held through every loop since it was adopted.

## 2. Sources of nondeterminism and their policies

| Source | Policy | Evidence |
|--------|--------|----------|
| Wall-clock time | **Never pinned in corpus output.** `clock()`/`now()` are **monotonic** (SPEC §22); `unix_time`/`date_parts`/`date_fmt` are wall-clock, corpus files assert **shape only** (type, field count, format skeleton), never absolute values. | `tests/differential/builtin_time_shape.op`, SPEC §22 |
| RNG stream | Fully deterministic, see §4. | `tests/worker_seed_pin.op`, `tests/std_random.op` |
| Thread scheduling | Spawn/join **values** are deterministic; worker RNG seeds are **pinned per worker** (pin preference incl. worker-in-worker, loop-10 R10-c F-1m) so stream identity does not depend on scheduling. Print-ordering across workers is not promised. | `tests/worker_seed_pin.op`, loop-10 wave R |
| Map iteration order | **Insertion-ordered** map value type, iteration is deterministic and stable. | SPEC §5 (value model) |
| Float formatting | `str()`/display of floats follows one formatting rule shared byte-for-byte by both implementations (SPEC §19). | `tests/differential/arith_pinned.op`, `strings_pinned.op` |
| `clock()` under load | Monotonic but load-dependent, corpus may pin monotonicity relations (`t2 >= t1`), never magnitudes. | `builtin_time_shape.op` |
| `sleep(ms)` | Bounded ≤ 60,000 ms; used in corpus only for scheduling-shape tests, never timing assertions. | SPEC §10 |
| `memory()` counters | Reflect the running process; shape-pinned only (keys present, value types), never magnitudes. | SPEC §10 |
| GRN / repressilator dynamics | Deterministic given seed + parameters (repressilator supports deterministic-noise mode for parity). | `tests/repressi_params.op`, `tests/repressi_alpha.op` |

## 3. Integers and overflow

`Int` is **i64**. Integer literals beyond i64 **saturate to 0** with a lexer note, the same
behavior in both implementations (commit f0fe2ec closed the out-of-range parity edge).
Arithmetic behavior is pinned by `arith_pinned.op`. This is a documented compatibility
contract: code relying on saturation is portable across Operon versions until a major
release says otherwise (compat policy: W63).

## 4. Randomness determinism (W089)

1. **State**: `randomize(seed?)` resets a deterministic **xorshift** state. The generator is
   pure integer arithmetic (u64 shifts/xors), there is no platform-dependent component.
2. **Promise (single thread)**: same seed + same call sequence + same operon version ⇒ the
   same stream of `random()`/`random(n)` values, byte-identical, on any platform.
3. **Promise (workers)**: every spawned worker's RNG state is **pinned** (seed derived
   deterministically, including the worker-in-worker pin preference from loop-10). Spawn
   order does not change a worker's stream. Join values are therefore deterministic.
4. **`std/random.op`** provides named wrappers over the mirrored core RNG; it adds no
   entropy source of its own.
5. **Cross-implementation**: the Python oracle implements the identical xorshift; the
   differential corpus contains RNG programs precisely to keep the two implementations
   stream-identical. **A change to the RNG is a breaking change** (compat policy W63):
   it requires a SPEC changelog entry, an oracle lockstep update, and re-pinning of every
   seeded corpus file.
6. **What randomness does NOT promise**: statistical quality beyond the xorshift family
   (fine for simulations/teaching; not cryptographic, crypto-grade entropy needs a host
   facility, future `std/hashing`-adjacent work, W027).

## 5. Floating-point determinism (W090)

1. **Representation**: `Float` is **f64** everywhere, both implementations. No f32 fast
   paths, no extended-precision spill (Rust: strict IEEE-754 double semantics; Python
   floats are f64).
2. **Bit-identical, cross-platform, promised**: `+ - * / %`, comparisons, `floor`, `ceil`,
   `abs`, `min`, `max`, `clamp`, `divmod`, `round` (half-away-from-zero via one shared f64
   formula, `round(1.005, 2) == 1.0` because 1.005 is stored as 1.00499…; pinned in
   `arith_pinned.op`), `sqrt` (IEEE-754 correctly rounded ⇒ platform-independent), and
   float→string formatting (SPEC §19).
3. **NOT promised bit-identical cross-platform**: `pow` and any future libm-backed
   transcendental (log, exp, sin, …). These are correctly rounded only within their own
   libm; two platforms may differ in the last ulp. Policy: (a) the differential corpus does
   not pin such outputs to byte level unless both sides call the *same* libm family,
   (b) scientific tests use tolerance-based assertions (std/testing `expect_*`, W093),
   (c) this document is the disclosure.
4. **Compiled artifacts**: no fast-math, no FP contraction, no `-ffast-math`-style flags
   anywhere in the build (Rust default; the C++ codon kernel is integer-only bit-parallel
   arithmetic, no FP at all).
5. **GRN/repressilator note**: all kinetics use only the promised operations (mul/add/max/
   clamps), so biological simulations are bit-reproducible across the release matrix
   (linux x64/arm64, macos, windows), this is what makes `repressi_params.op` meaningful.

## 6. Build reproducibility (W088)

1. **`operon build`** (source specialization/baking, full contract:
   `docs/spec/BUILD-CONTRACT.md`) output depends on exactly: input source bytes,
   `--variant` selection, and the operon version. It embeds no timestamps, no absolute
   paths, no environment values.
2. **Promise**: building the same input twice with the same version yields **byte-identical
   output**. Test: CI runs the bake twice and byte-compares (landed with this wave).
3. **Excluded from the promise**: `bin/` layout of released archives (zip/tar metadata),
   platform-native line endings written by install scripts (not by build), and anything a
   future bundle format (W087) explicitly documents as metadata.
4. **Language-version component**: baked output records the operon version; a different
   version is a different output, the promise is scoped to "same version", matching §1.

## 7. Replay protocol (how to reproduce any corpus output)

```
git checkout <commit>            # e.g. dd76caa
cargo build --release            # pinned toolchain per .github/workflows/ci.yml
cp target/release/operon bin/operon
python3 bootstrap/harness.py     # runs the full differential corpus; prints N/N MATCH
```

A pinned corpus file's `frame proof` header names its expected invariants; the harness
compares Rust-core stdout against the Python oracle byte-for-byte. Any divergence = red
main = stop everything (CONTRIBUTING §1).

## 8. Change rules for contributors

| You changed… | You must… |
|--------------|-----------|
| the RNG | lockstep oracle update + re-pin every seeded corpus file + SPEC changelog entry (breaking per W63) |
| float formatting / `str()` of floats | re-pin `strings_pinned.op` + `arith_pinned.op` + SPEC §19 |
| any builtin's output text (notes included) | re-pin the affected differential files, notes are part of the byte contract |
| GRN/repressilator math | re-run `repressi_*`, `grn_*`, `silence_*` families + re-pin changed outputs |
| `operon build` output shape | byte-compare test in CI must be updated in the same PR |
| time builtins | shape-only rule still holds, never pin wall-clock values |

## 9. Evidence index

- Differential harness: `bootstrap/harness.py`, 128/128 MATCH @ dd76caa
- Pinned arithmetic/strings: `tests/differential/arith_pinned.op`, `tests/differential/strings_pinned.op`
- RNG + worker pinning: `tests/worker_seed_pin.op`, `tests/std_random.op`, `tests/differential/json_rng.op`
- Time shape: `tests/differential/builtin_time_shape.op`
- Repressilator parameterization: `tests/repressi_params.op`, `tests/repressi_alpha.op`
- Monotonic clock oracle fix (S3): `bootstrap/oracle.py` `time.monotonic()`, the bug that
  proved why this document needed to exist
