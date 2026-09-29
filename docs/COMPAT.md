# COMPAT.md — the Operon reliability contract

**The rule this document exists to enforce:**

> Every optimization must preserve semantics. Every engine pairing must
> agree byte-for-byte. Reliability outranks features.

This document defines the compatibility corpus, the engine axes, the gates
that bind them, and the known exclusions (each with a reason and an owner).
If a gate and this document disagree, the gate wins and this document gets
fixed in the same commit.

## 1. Engine axes

| # | Axis | Pairing | Gate |
|---|------|---------|------|
| A1 | tree-walk | Rust, release | every gate |
| A2 | bytecode VM | Rust, release, `--vm` | vm_parity, compat_matrix, fuzz `--exec` |
| A3 | optimized VM | Rust, release, `--vm-opt` | vm_parity, compat_matrix (rule R1) |
| A4 | build profile | Rust, debug build | compat_matrix |
| A5 | implementation | Python oracle | harness, compat_matrix |
| F  | hostile input | fuzzer classes R/U/T/D/S | fuzz_parser (`--exec` adds A1-vs-A2) |
| P  | platform | Linux / macOS / Windows, x86-64 / aarch64 / i686 | CI `compat.yml` |

The invariant: **stdout, stderr, and exit code are byte-identical across
every axis combination for every program in the corpus.** Not
"semantically equivalent" — byte-identical, including diagnostics.

## 2. The corpus

1. **Pinned corpus** (`tests/differential/`, `tests/`, `apps/`): hand-written
   feature pins, 217 programs. Every language feature lands together with
   its pin (this predates the reliability push and remains the rule).
2. **Generated corpus** (`tests/compat/`, 1,200 programs, committed):
   produced by `scripts/gen_corpus.py --seed 20260930 --count 1200` across
   12 buckets — arith, cmplogic, strings, lists, maps, control, funcs,
   matchpat, optres, stressfail, nums, mixed. Regenerate bit-identically
   from the seed; the manifest records provenance.
3. **Fresh corpus** (`tests/compat_fresh/`, not committed): 800 new
   randomized programs generated per run from a rolling daily seed
   (`GEN=1 scripts/compat_matrix.sh`). Yesterday's fresh corpus is today's
   regression surface only if it found something; the committed corpus is
   the permanent regression surface.

**Determinism contract for generated programs:** no clock, no rng, no
threads, no spawns/sequences (thread-cap trip order is load-dependent —
see §5), bounded loops and recursion, bounded allocation, and stress only
where deliberately wrapped in `stress/rescue` with fixed probes.

## 3. The gates

| Gate | Command | Contract |
|------|---------|----------|
| differential harness | `python3 bootstrap/harness.py` | Rust == oracle on every `.op` in the repo (stdout + exit code) |
| 3-way VM parity | `bash scripts/vm_parity.sh` | tree-walk == `--vm` == `--vm-opt` on every repo `.op` (stdout + stderr + exit code) |
| compat matrix | `bash scripts/compat_matrix.sh` | all 5 axes on the full generated corpus (+ debug build) |
| parser fuzzer | `python3 scripts/fuzz_parser.py --exec` | no panics, no hangs, no containment breaches; engines agree where execution is reachable |
| redteam | `bash scripts/redteam.sh` | 100 adversarial payloads contained, 0 breached |

Full sweep, all gates, ~4 minutes on a laptop. The CI workflow
(`.github/workflows/compat.yml`) runs all of them across
Linux/macOS/Windows, release/debug, and 32-bit/64-bit/ARM targets.

## 4. Rule R1: optimizations are semantics-preserving by construction

`--vm-opt` runs `compile::optimize()`. The pass is deliberately small and
each transform is safe by argument, not by tuning:

- **Constant folding** only folds ops that *cannot stress at runtime*
  (checked arithmetic that succeeded, non-zero divisors, scalar
  comparisons). An op whose runtime result would be a stress is left
  untouched, so the VM raises the identical stress at the identical point.
  Float arithmetic is never folded (platform conservatism).
- **Folding uses `Nop` replacement** — program counters never move, so no
  jump target ever needs rewriting; folds overlapping any control-transfer
  target are refused outright.
- **Jump threading** rewrites operands only (a jump whose target is an
  unconditional `Jmp` is re-pointed), fixpoint-bounded.
- **`Insn::Line` stamps are never folded** — the `cur_line` trajectory is
  identical with and without the pass, so tracebacks are byte-identical.

**The gate earned its keep on day one**: the first `--vm-opt` build
panicked (index past the end insn) on `rt_p5a_arith.op` from the existing
redteam corpus, because a jump-to-end target (`t == len`) is legal and the
threading loop indexed it. Reproduced, bounds-checked, re-greened — within
minutes of the pass existing.

## 5. Known exclusions (each with a reason and an owner)

| Program | Reason | Owner |
|---------|--------|-------|
| `tests/redteam/rt_p4a_threadbomb.op`, `rt_p4b_threadbomb_join.op` | Containment is deterministic (rc=1, `[contained]`), but *which* cap trips first (task cap 4096 vs OS thread cap 256) is scheduler-dependent under a parallel sweep. Verified serially: identical output across all engines, 3 runs each. | redteam.sh owns the payload contract |

The diagnostic channel (stderr notes) is byte-parity as of 2026-09-30:
parser and lexer notes carry `file:line` and the offending token text in
both engines (`tools::flush_notes` format). Known remaining gap: *runtime*
interp notes carry a line in Rust but not yet in the oracle — unreachable
by any current gate (no corpus program emits a runtime note), filed as
oracle-parity backlog.

## 6. Fuzzer contract

`scripts/fuzz_parser.py` — classes: **R**andom bytes, **U**nicode salad,
**T**oken salad, corpus m**u**tations (bit flips / splices / truncations /
brace storms), **D**epth bombs. Per-input wall-clock bound is 15 s —
deliberately *above* the 20M-step fuel bound (~5 s), so a fuel-contained
runaway counts as contained behavior and only genuinely unbounded
execution trips the gate. Findings reproduce from the printed seed; the
offending input is preserved under a temp dir and named in the finding.

## 7. What "reliable" means here

A language feature is not done when it works. It is done when:

1. it has a hand-written pin in the differential corpus,
2. the generator buckets can randomly recombine it without disagreement,
3. the VM executes it byte-identically to the tree-walk,
4. the optimizer preserves it byte-identically,
5. the debug build preserves it byte-identically,
6. the Python oracle reproduces it byte-for-byte from source,
7. the fuzzer cannot make it panic, hang, or leak,
8. and all of the above holds on every OS and word size CI can reach.

Anything less is listed in §5 with a reason, or it does not ship.
