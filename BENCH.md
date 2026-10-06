# BENCH, the Operon benchmark suite

Owner: builder-B (B1). Baseline: **v2.2.0** (main @ `95ffef7`). This file is the
measured answer to "how fast is Operon, where does the time go, and what should
the v3.0 Ribosome VM fix first?" It is regenerated per release; historical rows
accumulate in *Baseline tracking* below.

Audience note (D-008): everything here is programmer-first. `gene` ≈ a function,
`regulate` ≈ a runtime feature flag with thresholds, no biology required to
read a single line below.

---

## How to run

```sh
bash scripts/build.sh                 # produce bin/operon (or download a release)
bash scripts/bench.sh                 # named workloads: fib/loops/strings/collections/recursion/grn
bash scripts/bench.sh --micro         # per-construct micro fixtures (scripts/bench/micro/)
bash scripts/bench.sh --json out.json # machine-readable results
bash scripts/bench.sh --quick         # reduced iterations for a fast pass
```

Fixtures live in `scripts/bench/*.op`; correctness twins for every workload
live in `tests/bench/bench_correctness.op` and run in the proof suite on every
CI push, a benchmark that measures the wrong answer fails CI before it can
mislead anyone.

## Methodology (and honesty notes)

- **Three runners on identical algorithms:**
  - **operon**, the Rust core, `bin/operon run <fixture>`.
  - **oracle**, `bootstrap/oracle.py` running the *same .op file*; the CPython
    tree-walking semantic mirror used for differential parity. It shares the
    architecture (assoc maps, tree-walk), so it is an architecture reference,
    not a speed target.
  - **native-py**, the same algorithm hand-written in pure CPython
    (`scripts/bench_compare.py`, `native_*`). This is the real "CPython-level
    speed" bar v3.0 targets; it is what `op/py` measures.
- **Timing**: end-to-end process wall time, **min over N runs** (1 warmup;
  N=5 default, 3 for the oracle). Min-of-N is the standard for subtracting
  scheduler noise; medians land in the `--json` output.
- **Startup floor**: timings include process startup. Measured floor:
  **0.9 ms** for operon (`m_empty`), **~54 ms** for the oracle (CPython
  startup). The native-py column has no process floor. Numbers ≥ 30 ms are
  floor-clean; treat sub-5 ms rows (strings native) as ratio-indicative only.
- **Fixture sizes** are chosen so operon runtime is 25–600 ms; every workload
  prints a closed-form-verifiable result (see `tests/bench/bench_correctness.op`).
- **ns/op** in the micro table divides by documented op counts
  (`bench_compare.py` `MICROS`), indicative, not a hardware benchmark.

## Environment (this baseline)

| field | value |
|---|---|
| date | 2026-09-24 (re-measured post audit-wave 1) |
| host | c-6ab50512-14810412 (containerized CI-class VM) |
| cpu | Intel(R) Xeon(R) Processor |
| os | Linux 5.10.134 x86_64, glibc 2.41 |
| python | 3.12.14 |
| operon | main @ `e757b4d` + B1 (`c7d4dd2`), release build, rustc 1.98.1 |

Rerun on your machine before quoting ratios in an argument, absolute times
move, the *relative shape* (maps ≫ calls ≫ loops) does not.

## Results, named workloads (v2.2.0 baseline, re-measured on `e757b4d`)

Audit-wave 1 (security/dx/reg merges d0ce9e5/b83c0a2/e757b4d) re-verified: every
row moved ≤3% vs the `95ffef7` first measurement, the containment guards (push
mem-charge, DP ceiling) cost nothing measurable on honest workloads.

| workload | operon (ms) | oracle (ms) | native-py (ms) | op/py | op/oracle | calls/iters |
|---|---:|---:|---:|---:|---:|---:|
| fib25 | 127.7 | 2405.4 | 12.8 | **10.0x** | 18.8x | 242,785 calls |
| loops | 62.3 | 871.1 | 11.2 | **5.6x** | 14.0x | 200,000 iters |
| strings | 26.0 | 116.9 | 0.7 | **35.3x** | 4.5x | 4,000 replaces |
| collections | 329.5 | 22503.7 | 5.3 | **61.8x** | 68.3x | 20k map ops + 20k pushes |
| recursion | 228.8 | 4923.8 | 23.7 | **9.6x** | 21.5x | 369,511 calls |
| grn | 33.2 | 514.6 | 8.6 | **3.9x** | 15.5x | 60,000 gated calls |

**W083 real-program workloads** (landing wave; numbers to be re-measured in the next full bench pass, sizing verified, ratios below are placeholders until then):

| workload | what it exercises | native mirror | notes |
|---|---|---|---|
| json | json_str/json_parse round-trip over a 120-row nested doc ×200 | CPython json.dumps/loads (algorithm parity) | serializer + map/list allocation |
| regex | 5 patterns × 10 strings × 600 passes (re_match + re_find) | CPython re (algorithm parity) | regex engine incl. 2M-step ceiling |
| seq | 6k-base LCG genome: GC%, 3-mer scan, motif locate | CPython (algorithm parity) | seeded LCG, no random(), fully reproducible |
| large_map | 6k-key map build + full lookup pass | CPython dict (algorithm parity) | sized for the oracle pass (~18 s/iter) |
| file_io | 300 × write+read 2 KiB, content-verified | CPython open/write/read (shape parity) | the ONE bench needing grants, runner passes `--allow-write /tmp --allow-read /tmp` itself; oracle side runs granted too. NOISE (2026-09-27): I/O-bound, shared-runner spread across identical code measured 61–250 ms in one hour, so the perf gate pins this workload at a 100% threshold (`perf_gate.py` NOISY_THRESHOLDS) instead of the 20% default; still catches 2–3x catastrophic regressions, ignores runner jitter |
| modules | six local modules imported + 20k cross-module calls | CPython cached import machinery (**shape-compare only, not algorithm parity**) | exercises the SPEC §8 resolution table + module cache |

`op/py` = operon vs native CPython, **the v3.0 gap to close**.
`op/oracle` = how much faster the Rust core already is than its Python mirror.

## Results, micro (per construct — full table re-measured on the P1 merge head `2bd29b0`, 2026-10-05, builder-E, canonical iters in foreground chunks; m_mapset/m_mapget re-confirmed on this head)

> NOTE (2026-10-05, refresh): the P5/P6 survey's stale-row promise ("the map/call
> rows are stale and refresh with P1's landing") is now settled — every row below
> comes from ONE run on ONE head (`2bd29b0`), so within-run ratios are the
> comparables. Cross-session absolute drift on this sandbox is real: the same
> untouched fixtures (m_while, m_intadd) and even the oracle legs moved +8–17%
> vs the 2026-10-04 recordings on a different box allocation — do NOT read
> cross-session absolute deltas as regressions. Structural findings survive the
> drift unchanged: map rows hold at **3.5–3.7x** (vs 70–80x pre-fix), and the
> call row is consistent with the P4 profile below (m_call 202 ns/op here vs
> fib25's 406 ns/call — the nop-call shape is shallower than deep recursion).
> Survey evidence archive: `docs/bench/2026-10-04-p5p6-survey.md`.

| micro | operon (ms) | oracle (ms) | native-py (ms) | op/py | op ns/op | py ns/op |
|---|---:|---:|---:|---:|---:|---:|
| m_empty (startup) | 1.0 | 90.6 | — | — | — | — |
| m_call | 60.5 | 1035.3 | 8.7 | **7.0x** | 202 | 29 |
| m_forrange | 57.3 | 738.3 | 7.9 | **7.3x** | 96 | 13 |
| m_while | 66.0 | 1563.8 | 11.1 | **5.9x** | 110 | 19 |
| m_varread | 61.3 | 787.3 | 8.0 | **7.7x** | 102 | 13 |
| m_intadd | 93.3 | 1119.9 | 15.0 | **6.2x** | 104 | 17 |
| m_listpush | 19.1 | 310.5 | 2.3 | **8.3x** | 127 | 15 |
| m_listidx | 41.8 | 702.7 | 7.6 | **5.5x** | 139 | 25 |
| m_mapset | 18.4 | 61915.0 | 5.3 | **3.5x** | 230 | 66 |
| m_mapget | 29.1 | 82289.3 | 7.8 | **3.7x** | 189 | 51 |
| m_strcat | 10.9 | 138.4 | 0.7 | **15.6x** | 454 | 29 |

## Reading the numbers

- **Maps are fixed — the emergency is retired.** At `e757b4d` the association-list store scanned linearly with `deep_eq` per element: `m_mapset`/`m_mapget` ran **70–80x** slower than native CPython (the oracle's cProfile showed **58,025,002 `deep_eq` calls, 42 s of 84 s** on the collections fixture). dx-r3 added the hash memo prefilter, W-L1 removed the per-lookup String clone, and P1 (2026-10-04) rebuilt the memo as a single-hash open-addressed table with proven miss-trust (docs/bench/2026-10-04-p1-map.md): the build path dropped from O(N^2) deep_eq scans to O(N) (8k-key build 149→3.7 ms, t(2N)/t(N) 4.19→1.99), absent-key lookup from a full scan to O(1) (50k misses 1084→153 ms), batch del from quadratic rebuilds to linear (261→22 ms), and the standing micros now sit at **3.5–3.7x** native (re-confirmed on the merge head `2bd29b0`, 2026-10-05). The residual map gap is per-op interpreter dispatch + `str()` key materialization — P4 dispatch surface, not the store.
- **Calls are the chronic gap.** Naive recursion (fib25, C(20,10)) lands at
  **~9.5x**; an empty-ish gene call costs ~202 ns/op vs CPython's 29 (this
  table, 2026-10-05). The call path's alloc/clone story is now fully
  decomposed in the P4 section below: the W011 rounds killed the per-call
  name clones (tb/promo counters read ZERO), the regulatory gates cost 7.5
  ns/call, bookkeeping 1.6, the fuel tick 9.8 — and the remaining 388.5
  ns/call floor is frame-Env + slot-Vec allocation (env_new = 1.5/call,
  exact) plus 7 VM instruction dispatches.
- **Plain loops are close-ish.** `for`/`while` arithmetic sits at 5–6x, the
  dominant cost is a fresh `Env` (a new `Rc` + `HashMap`) allocated **per loop
  iteration** to bind one variable, plus `tick()` bookkeeping per node.
- **Strings leak allocations.** `s = s + "x"` reallocates a fresh `String` per
  op (15.6x on the 2026-10-05 run), and the `strings` fixture (template + replace + concat) hits
  33.5x, but note its native-py time (0.8 ms) is at the startup-noise floor;
  trust m_strcat's ~454 ns/op instead.
- **The v2.2 honesty machinery is cheap.** The GRN-gated workload is the
  *closest* to native (3.8x). `operon profile` on `grn.op` shows the three
  regulated genes at ~2.1 ms self each for 20k calls (~107 ns/call including
  the gate walk), T2a/T2e gating does not need rescuing for v3.0; do not let
  the VM design regress it either.

## Top-10 interpreter hot-path targets (input to the v3.0 Ribosome VM design)

Ordered by expected payoff. Line references are read-only analysis anchors on
main @ `e757b4d` (`src/interp.rs`); no src/* changes are proposed for v2.2.

1. **`Value::Map` as an association list**, `map_insert` (interp.rs:956) and
   every map read do a linear `deep_eq` scan. Replace with a real hash map over
   scalar keys (SPEC already requires scalar keys via `key_scalar`), keeping
   insertion order for deterministic iteration (§19 snapshot iteration).
   Expected: map-heavy code 5–15x. *Highest single payoff.*
2. **Env allocation per loop iteration**, `Stmt::For` (interp.rs:566–620)
   builds `Env::new(Rc + HashMap)` per item to bind one variable. Slot-based
   locals, or a reused single-slot iteration frame. Expected: loops ~2x.
3. **Per-call `String` churn in the call path**, `call_gene_inner`
   (interp.rs:1805+) clones the callee name 2–3x per call for `call_counts` /
   `gene_buckets` keys. Intern gene names (symbol table → index), make
   telemetry arrays index-addressed. Expected: call-heavy code ~1.5–2x.
4. **`for` over a list materializes a full clone**, `l.borrow().clone()`
   (interp.rs:589) copies the entire list before the first iteration. Iterate
   the `Rc` with an index instead.
5. **`tick()` per eval node and per iteration**, interp.rs:368, called first
   thing in `eval` (968) and in every loop body: a modulo + branch per AST node
   for fuel accounting. Charge fuel in batches (branch-free decrement, check on
   underflow) or only at loop back-edges and calls. Expected: ~5–10% across
   the board.
6. **String concatenation reallocates per op**, `apply_binop` Add
   (interp.rs:~1317): `format!` + per-op `mem_charge`. A rope / amortized
   builder for the `s = s + x` accumulation shape. Expected: m_strcat ~5x.
7. **String literals clone per evaluation**, `Expr::Str(s.clone())`
   (interp.rs:975). Pool literals per program (`Rc<str>`); the parser already
   walks them once.
8. **GRN gate per-call costs**, `grn_veto` (interp.rs:1767) walks all edges
   filtered per callee and scans `enhanced` per call; `grn_fire`
   (interp.rs:2766) clones the whole `grn_levels` map twice per fire (2780, 2834).
   Index edges by callee, precompute the enhanced set, mutate levels in place.
   Cheap today (3.8x overall), keep it cheap as nets grow.
9. **`silences` linear scan + clone per call site**, `Expr::Call`
   (interp.rs:~1081) scans the silence list (and clones the match) on every
   call. Resolve callees once per site (inline cache), invalidate on
   `silence`/`acetylate` mutations.
10. **Builtin dispatch by string `match` per call**, `call_builtin`
    (interp.rs:2195) matches `&str` each call. Intern builtin names at parse
    time (or perfect-hash) → enum dispatch.

**The v3.0 design takeaway:** targets 2, 3, 5, 7, 9, 10 are exactly the costs a
bytecode VM with stack frames, slot variables, interned constants, and inline
caches dissolves *by construction*. Target 1 is independent of bytecode-vs-tree
(the Value representation must change either way) and is the one thing that can
ship early without waiting for the VM. Targets 4, 6, 8 are tree-walk
mitigations worth doing only if the tree walker stays as the fallback core
after v3.0, which the differential harness needs it to.

## The --vm lane (W09, measured 2026-09-28, native-calls wave)

The OIR1 bytecode machine runs the FULL differential corpus byte-identically
against the same oracle outputs (vm lane 213/213; the differential harness
prints the split). The design's fib25 gate asked for a 2x speedup over the
tree-walk. Honest measurement, median of 7 runs, same box:

| config | fib25 (ms) | vs tree-walk |
|---|---:|---:|
| tree-walk (default) | 145.3 | 1.00x |
| --vm | 164.7 | **0.88x (slower)** |
| --vm --opt 1 | 165.0 | 0.88x |

A5 campaign update (2026-09-30, pre-flip): the call-path mallocs are gone
(def-name re-clone per call, SipHash on the pointer-keyed code cache, the
silences empty-gate entries collect, the call-counter entry clones) and the
machine sits at parity end-to-end, median of 5:

| config | fib25 | loops | collections | recursion |
|---|---:|---:|---:|---:|
| tree-walk | 139.3ms | 67.0ms | 49.4ms | 256.6ms |
| VM (default since v2.6.0) | 141.2ms | 64.6ms | 49.4ms | 261.5ms |
| ratio | 0.99x | 1.04x | 1.00x | 0.98x |

A6 flip (2026-09-30, v2.6.0): the VM is the run default, `--interp` (or
`--no-vm`) escapes to the tree-walk, the banner carries the `-vm` suffix.
The >=3x stretch target stays OPEN under W11/A5; parity is the shipped
floor, not the ceiling.

## The W011 stage-3 dispatch work (measured 2026-10-01, median of 7, same box)

Both remaining §2c items landed with the full differential gate (vm_parity
3537/0 all four axes, redteam 109/0, fuzz_diff 600x5 0 findings, clippy 0,
fmt clean) and the fib25 row the design note demanded:

| config | fib25 (ms) | delta |
|---|---:|---:|
| trivial-gene dispatch in (item 1, 4834575), mono-cache absent | 143.9 | 1.00x |
| mono-cache active (item 2, stash-verified rebuild) | 145.9 | 1.01x (within run noise) |

The honest verdict repeats the W011 matrix conclusion: **dispatch is
semantics-bound, not lookup-bound.** The mono-cache skips exactly one
env-chain walk (frame miss -> global hit) per call and pays one hash probe +
generation check for it; the funnel's mem_charge/cycle-note security charges
remain the floor, and fib25's 242,785 calls amortize both to noise. The items
stay in the language because the enumerated W011 list is now fully delivered
with parity proof, the machinery is real (2 chain-node lookups saved per
global-gene call, more on deep chains), and the cache is invalidation-complete
by construction (generation bumps inside Env's three mutators; param binding
exempt with the shadowing-safety argument in DEF_GEN's doc). fib(25) output
pinned 75025 in every run of both configs.

The gate is **honestly missed** at stage A2, and the campaign that measured it
found and fixed three real machine regressions along the way:

1. the compiled body was deep-cloned (Vec of instructions + consts + names)
   on EVERY call; the cache now hands out Rc<GeneCode> (refcount bump).
2. calls were bridge-only: every Expr::Call round-tripped through the
   tree-walk. Bare-identifier calls now compile to a native CallNamed
   instruction that rides the SHARED named-call funnel (RISC gate included),
   so the machine and the tree-walk cannot disagree on gates or notes.
3. a fresh operand Vec was allocated per call; stacks are now pooled per
   interpreter (stacks over 64 slots drop, memory stays flat).

The structural reason the gate is still missed: A2 shares the call funnel, the
gate chain and every note with the tree-walk (that sharing is WHY the vm lane
is byte-identical), and the machine pays one fuel tick per instruction where
the tree-walk pays one per statement/expression node, so a flattened body of
~15 instructions pays ~1.5x the tick volume of the same body walked as ~10
nodes. On call-free workloads the machine sits at parity (loops 1.01x,
collections 1.02x, large_map 1.00x).

Next levers, in the audit's own order (VM -> profiling -> opt -> JIT): W011
depth (superinstructions for LoadName/Push/Bin triples, which cuts both the
instruction count and the tick volume), then W012 (JIT, owner-gated) which is
where the 2x class of speedup has always lived. The 0.88x is recorded here so
nobody re-discovers it by surprise.

## The W011 per-pass matrix (measured 2026-10-01, dev-1)

The toggle matrix's measured answer (`bash scripts/bench_opt_passes.sh`,
min-of-5 process wall time, identical binaries; the differential
requirement — byte-identical stdout+rc across EVERY config — is enforced
inside the script and rode vm_parity's 3536-program corpus gate the same
day). Workloads: fib25 (recursion/call-funnel), lists.op (NEW, the
resolution-chain workload: 500k push/len/pop through
CallNamed -> call_named -> call_builtin), loops (arithmetic while).

| workload | tree-walk | vm | stage1 | all(+prop) | fold | fold,prop | thread | dce |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| fib25 | 141ms | 143ms | 143ms | 144ms | 144ms | 143ms | 143ms | 144ms |
| lists | 427ms | 434ms | 436ms | 437ms | 435ms | 432ms | 432ms | 433ms |
| loops | 66ms | 66ms | 64ms | 64ms | 64ms | 65ms | 64ms | 65ms |

Honest reading (the delta rows the W011 done-when asks for):

- **The passes are semantics-cheap and perf-neutral at these workloads**
  (within +/-1-2ms run-to-run noise). fib25 is call-funnel-bound: the gate
  funnel + env machinery dominate, and folding/propagation shave
  instructions that are a single-digit percentage of one recursion step.
  loops ties because its loop bodies are already LoadBinImm superinsns.
- **lists.op is dispatch-bound, not lookup-bound**: after the W011
  resolution cache (the per-call linear scans over BUILTIN_NAMES ~90
  strcmps and BUILTIN_SYNONYMS became OnceLock hash lookups), the
  remaining per-call cost is the SEMANTIC contract itself — mem_charge +
  cycle-note insertion on push (sec-r1/reg-bio-2, security-mandated), the
  env-chain shadow check (a user gene may shadow any builtin), and the
  per-tick accounting. That is the floor the contract sets, not overhead.
- The >=3x stretch stays where the A5 campaign left it: gated on the call
  funnel (W12 JIT class), not on more local passes. W011's value is the
  instruction-count reduction + the toggle/bench/differential
  infrastructure, shipped honest.

## W009-A — VM call-path investigation (2026-10-02, builder-A)

First measured attribution of the fib25 call-heavy gap (W009-A child task,
owner directive). Everything below is reproducible: `scripts/w009a_ablation_matrix.sh`
drives a runtime-gated ablation harness (`src/w009a.rs`, inert without
`OPERON_W009A_ABLATE`/`OPERON_W009A_COUNTS`); raw evidence in
`docs/bench/2026-10-02-w009a-ablation.txt`, suite baseline in
`docs/bench/2026-10-02-w009a-baseline.json`.

**Sandbox baseline** (2-core Xeon VM, slower than the 10.0x recorded rows —
machine variance, noted honestly; ratios are internally consistent within
the run): fib25 144.5 ms vs native-py 13.1 ms = **11.0x** (242,785 calls,
~595 ns/call vs CPython's ~54 ns). Call-heavy workloads cluster: recursion
11.3x, seq 11.4x — while call-free loops sits at 6.0x. Per-call overhead is
the structural cost, not body execution.

**Counters** (`OPERON_W009A_COUNTS=1`, one fib25 run): 1,699,496 VM
instructions (~7.0/call), 364,180 `Env::new` (~1.5/call — the if-block scope
env counts too), 242,786 bookkeeping calls, 242,786 funnel name clones,
242,782 mono-cache hits.

**Ablation matrix** (`operon bench --iters 30`, in-process min, base 143.7 ms;
each config verified to print exactly `fib(25) = 75025` before timing):

| config | min ms | delta | share | verdict |
|---|---:|---:|---:|---|
| bk — skip call bookkeeping (call_counts + burst bins + clock) | 126.9 | −16.8 ms | 11.6% | measured cost |
| gates — skip whole regulatory block (GRN/methyl/ribo/promoter/RHO/transcripts, incl. bookkeeping) | 119.5 | −24.1 ms | 16.8% | measured cost |
| tb — lazy traceback frame (no happy-path String clone) | 136.3 | −7.4 ms | 5.2% | measured cost |
| promo — promoter_veto checks `expr_stochastic` before cloning the name | 138.0 | −5.7 ms | 4.0% | measured cost |
| tick — skip per-instruction fuel tick | 143.8 | +0.1 ms | ~0 | **refuted** (not a suspect) |
| decay — skip m6a/grn decay tickers | 143.3 | −0.3 ms | ~0 | refuted |
| pool — Env map/consts free-list recycling | 152.3 | +8.6 ms | −6.0% | **refuted** (TLS round-trip costs more than glibc small-alloc) |
| SAFE = gates,tb,promo (semantics-preserving bundle) | 111.4 | −32.3 ms | **22.4%** | realistic fix ceiling |
| ALL | 120.2 | −23.5 ms | 16.4% | pool drags the bundle (consistent) |

**Lane/optimizer context** (end-to-end, min of 5): VM default 144.2 ms,
`--opt 2` 143.2, `--opt-passes all` 143.2, `--no-vm` 145.3 — the optimizer
passes do not move fib25, and the VM does not beat the tree-walk on
call-heavy code. The bottleneck is the **shared call funnel + per-body name
resolution**, not codegen, not dispatch, not fuel.

**Interpretation** (carried into `docs/vm-design.md` §12): measured,
fixable-shape costs account for ~22%; the remaining ~78% (≈460 ns/call on
the SAFE floor of 111.4 ms) is structural — the ~8-deep Rust call chain,
string-keyed SipHash env-chain resolution (~6+ hashes per body: 2 `fib`
lookups × 2 frames + 2 `n` reads), scope-env machinery, args plumbing.
Ranked fix candidates for a W009-B: (1) cached per-gene clean-regulation bit
skipping veto gates + cheaper bookkeeping (~17% proven shape), (2) kill the
happy-path String clones (~9% for 2 of 3 sites), (3) slot-indexed locals
(the vm-design §3 defer — targets the dominant residual), (4) collapsed
mono-hit call lane. Refuted by measurement: fuel-tick charging, decay
tickers, naive Env pooling.

**fib27 cross-check (2026-10-03, dev-1/builder-A).** The ytdl-bench M8 row
(fib27: operon 0.447 s vs CPython 0.027 s = 16.5x) prompted a per-call
scaling check on the same 2.7.0-vm binary: fib25 = 170 ms / 242,785 calls
= 700 ns/call; fib27 = 447 ms / 635,621 calls = 703 ns/call — the per-call
floor is DEPTH-FLAT (no deep-recursion penalty; CPython is likewise flat at
41–43 ns/call). The W009-A attribution carries over unchanged: fib27 is the
~0.7 µs/call funnel floor times the call count, and the 11.0x-vs-16.5x
spread between sessions is box-state variance in BOTH numerator and
denominator (CPython 13.1 ms then vs 10.0 ms now; operon 144.5 ms then vs
170 ms now), not a code change. Startup floors re-pinned the same day
(scripts/bench_startup.sh, N=30): exec-only 1.5 ms, run-hello 1.9 ms; full
352-line ytdl app via launcher 4 ms; the 934-line DEEP app direct
invocation 3 ms — see the reconciliation note in docs/BENCHMARK-DEEP.md
(the DEEP suite's recorded 27.2 ms startup row was box contention, ~+25 ms
on every host incl. bash).

## Baseline tracking

| version | commit | date | fib25 op/py | loops op/py | collections op/py | grn op/py |
|---|---|---|---:|---:|---:|---:|
| v2.2.0 | 95ffef7 | 2026-09-24 | 10.0x | 5.5x | 60.0x | 3.8x |
| v2.2.0+audit1 | e757b4d | 2026-09-24 | 10.0x | 5.6x | 61.8x | 3.9x |
| 2.8.0+W011-s3 | 3fe8dff+s3 | 2026-10-04 | 7.8x | 6.0x | 9.6x | 4.3x |

(Add a row per release; ratios from the default `--iters 5` run.)

## W011-r2 — the call-funnel round (2026-10-03)

Owner directive: "optimize the language to make it faster, check benchmarks
every time, verify with the ytdl app." Method: the W009-A ablation harness
first (apportion, then cut), an **interleaved A/B runner**
(`scripts/bench_core.py` — baseline and patched binaries alternate run-by-run
so box contention hits both sides; this box showed ±30% swings between
minutes, which would have made sequential measurement lie both ways), and
the full parity battery after every round.

Changes landed (all output-identical, gated per round):

1. **FxHasher** (`src/fxhash.rs`, zero crates) on the INTERNAL maps the
   call funnel touches per call: `builtin_synonym_map` (SipHash→~3ns),
   `mono_cache`, `call_counts`, `gene_buckets` (+ nested bin map),
   `methyl_levels`. **Deliberately NOT on `Env`**: a measured experiment
   showed Fx on env maps REGRESSED loops.op to 0.90x — deterministic
   hashing gives short hot names ("acc"/"i") systematic probe collisions,
   while RandomState's per-process seed spreads them; env maps stay on the
   default hasher (loops 1.00x, fib keeps its win).
2. **rho_knobs cached**: the RHO gate re-parsed four `.cell` strings
   (hash + `parse::<f64>`) on EVERY call; the cell map is never mutated
   after construction (audited), so the derived tuple is cached on first
   use. Was the single largest gate cost.
3. **promoter_veto reorder**: the `!expr_stochastic` early-out moved ABOVE
   the eager `def.name.clone()` — every non-bursting call paid a malloc for
   a name the gate never read.
4. **Lazy traceback frame** (W007 frame): `call_gene` cloned the gene name
   per call purely to carry it to an error path that honest workloads never
   hit; the clone now happens only when a chain frame is actually appended
   (`call_gene_inner` borrows the def, which also kills the funnel's own
   per-call name clone).
5. **Methylation empty-map guard**: with no level table and a positive
   threshold, the per-call hash lookup is provably inert and is skipped
   (threshold<=0 re-enables it — the guard reads current values).
6. **VM dispatch loop**: process-constant ablation flags hoisted out of the
   loop (two atomic loads per instruction removed); `tick()`'s cancel
   observation precomputes chain liveness (`cancel_live`, refreshed at the
   three wholesale assignment sites; `cancel_suppressed` stays live).
7. **Frame-push gating**: the per-call `call_stack` push (String malloc +
   Vec push) is skipped unless profiling or a debugger surface is live
   (`frame_trace_live`; the push's only readers; close_timing's pop is
   conditional, so the pairing is exact; DAP stopOnEntry/protocol modes arm
   it — caught by the DAP e2e gate during bring-up, which is why the
   debugger e2e gates run in the main battery).

Measured (interleaved A/B, medians, same box minutes apart):

| fixture | baseline | W011-r2 | speedup |
|---|---:|---:|---:|
| fib27 | 375.4 ms | 299.4 ms | **1.25x** |
| fib25 | 145.0 ms | 115.7 ms | **1.25x** |
| recursion | 269.4 ms | 220.8 ms | **1.22x** |
| grn | 43.1 ms | 35.0 ms | **1.23x** |
| loops | 66.7 ms | 67.5 ms | 0.99x |
| collections | 51.5 ms | 50.9 ms | 1.01x |
| strings | 27.1 ms | 27.8 ms | 0.97x |

fib27 op/py moved 9.2x → **7.3x** (CPython 41.2 ms on the same window).
App-level (ytdl deep suite, checksums 10/10 IDENTICAL): table -13.7%,
lines -17.8%, table300 -15.7%, lines200k -16.5%; spawn/queue unchanged
(child-process-bound, as designed). Gates at close: cargo 319, vm_parity
3555/0, differential 3484/0, compat matrix 3204x5 identical, redteam
109/0, debug+protocol+DAP e2e green, clippy 0, fmt clean.

**Next lever (boarded, not started):** the residual ~460ns/call is the
funnel walk itself (10 nested Rust frames per call) + per-call frame env
alloc + operand-stack arg collect — the classic fix is VM-to-VM call
fusion with frame-slot locals (W011 stage-2 "locals-in-frame" from the
W009-A follow-up); it needs its own gated session with the redteam
probes re-run (gate funnels get duplicated).

## v2.8.0 release verification (measured 2026-10-03, builder-A)

The owner's standing rule — "check benchmarks every time, and check with the
yt downloader app" — run once more on the exact release tree (post compat-r2:
ffi test move, test-runner VM-default flip, harness UTF-8 stdout; none touch
the run-path hot loops, and the numbers say so):

- Language suite (`scripts/bench_compare.py`, median of 5): **fib25 116 ms**
  (242,785 calls; 170 ms at the 2.7.0-era baseline, the W011-r2 win held),
  loops 68 ms, collections 51.9 ms, recursion 222 ms, grn 35.6 ms — every
  workload within noise of the W011-r2 measurements, op/py gap on fib25
  down to 9.0x (was ~16x when the fib-27 investigation opened).
- Deep cross-language bench (`scripts/bench_deep.py`, 11 workloads):
  all medians within ±3% of the 13:40 UTC re-run; **every workload's
  checksum IDENTICAL** across operon/python/bash. Full table:
  `apps/ytdl/bench/report_v280.md` (+ machine-readable `results_v280.json`).
- App battery: `apps/ytdl/test/run_tests.sh` **35/35** (operon e2e over
  deterministic mocks + cross-language decision differential byte-identical).

INFRA: the deep bench's spawn/queue workloads ERROR for every language if
`mockspawn`/`mocksleep` lack their exec bit — the index said 100644 and every
sandbox clone lost the locally-chmod'd bit (the 534cba6 lesson, reapplied via
`git update-index --chmod=+x`; this time recorded in-index so it sticks).

## Reproducing

```sh
bash scripts/bench.sh --json /tmp/bench.json        # named workloads
bash scripts/bench.sh --micro --json /tmp/micro.json
./bin/operon profile scripts/bench/grn.op           # per-gene self time
python3 bootstrap/oracle.py run scripts/bench/collections.op   # oracle-side
python3 -m cProfile -s tottime bootstrap/oracle.py run scripts/bench/collections.op
```

Profiling beyond the aggregate table (per-call spans, Chrome Trace Format,
the W009-A counters, and the full reproducible measurement procedure):
see **docs/PROFILING.md**.

Fixture → correctness-proof mapping: `tests/bench/bench_correctness.op`.
Questions / profile requests: `[builder-B -> sz]` thread in
`project-vault/collab/COMMS.md`.

## W011 stage-2 — frame-slot locals, write-through (2026-10-04)

The boarded stage-2 lever ("locals-in-frame", vm-design §3/§6, W009-A
candidate (c)) lands in its safe first shape: **param slots for the sync VM
lane**. The compiler's binder walk marks every let/for/match binder in a
gene body; a param is slot-eligible when it is default-free, unshadowed by
any of those binders, and the gene's cached IR contains no bridge
instruction (bridged nodes evaluate the original AST through the env
chain, which cannot see slot-only values). Eligible frames bind params
into a slot Vec **write-through** (the env copy stays authoritative for
the funnel: callee resolution by name, RISC/toggle immunity checks, the
mono-cache origin discipline); reads (LoadName, LoadNameQuiet, LoadBinImm,
RetName) and writes (AssignName) hit the Vec first, env unchanged below.
Kill switch: `OPERON_VM_SLOTS=0`; engagement counter:
`OPERON_W009A_COUNTS=1` prints `slot_frames`.

**Correctness from the analysis, not luck**: every binder form the parser
has is marked by the walker (over-marking is free, under-marking is a
bug); `StoreName` is deliberately never slot-routed (a slot name is
never let-bound by definition); the const-stress check runs before a
routed write exactly as before; the fiber lane keeps complete env frames
(parked frames must be self-contained). The first smoke run CAUGHT the
design error of skipping write-through (`twice(f, v)` → `f(v)` resolved
the callee through the env chain → phantom nulls) — the proof-frame
corpus would have caught it too, the harness ran before the corpus did.

- **Measured** (interleaved A/B, `scripts/bench_core.py` N=9, same binary
  with `OPERON_VM_SLOTS=0` as baseline): fib27 **1.02x** (307.3→300.3 ms),
  fib25 **1.03x** (119.2→116.1), recursion **1.03x** (226.7→219.9),
  loops/collections/grn 1.00–1.01x (within noise). Full suite re-run:
  fib25 113.7 ms, recursion 218.4 ms — best recorded numbers; op/py at
  **8.9x** on fib25.
- **Honest scope note**: the write-through design keeps the frame-env
  allocation per call (~100 ns of the residual) because eliminating it
  requires slot-routing the callee resolution AND the RISC/toggle
  immunity env reads — measured shape ≈1–2% more, funnel-signature
  change, deferred. The slot path removes the per-READ SipHash probes
  (3–5 per fib call), which is the part the corpus would see.
- **Deep cross-language bench** (`scripts/bench_deep.py`, 11 workloads,
  `apps/ytdl/bench/results_w011s2.json`): every checksum IDENTICAL across
  operon/python/bash; medians within the box's noise band of the v2.8.0
  snapshot (sequential cross-session numbers on this box swing ±3–5%).
- **App battery**: `apps/ytdl/test/run_tests.sh` **35/35** with slots live.
- **New lane-parity tests**: `tests/vm_slots.rs` (recursion, nested-scope
  reassign, shadowed-param fallback, higher-order param callee,
  missing/extra-arg stress) — both lanes byte-equal.
- **Gates at close**: cargo test green (123 lib + vm_slots 5/5 + all
  integration sets) · vm_parity 3555 identical/0 divergent (4 lanes) ·
  differential VM lane vs oracle 3484/3484 · redteam 109/0 · clippy 0 ·
  fmt clean.

## W011 stage-3 — the bookkeeping consumer gate (2026-10-04)

The W009-A ablation said the remaining call-path shape was `bk` — the
per-call `call_counts`/`gene_buckets` maintenance (13% on fib25, measured
again this session: `bk` 100 ms vs base 115 ms; the decay tickers were
re-confirmed refuted as a cost, −0.3 ms, and the frame-env residual stays
the honest 1–2% the stage-2 note recorded). s3 lands the measured-first
conclusion: **the counters are write-only state unless a reader exists**,
so a program-global consumer scan decides whether the maintenance runs.

- **s3a (decay inert gate):** `bump_call_bookkeeping` skips
  `m6a_decay_own`/`grn_decay_tick` when no decay surface exists (runtime
  `decay_clock` disarmed — read LIVE, it is a runtime switch — and no .cell
  `grn.decay_calls` / positive `m6a.decay` — cached post-load facts, the
  same audited cell-immutability premise `rho_cache` rides). Standalone
  effect: sub-noise (the tickers were refuted as a cost), kept because it
  removes the last per-call config re-reads for clean programs.
- **s3b (consumer scan):** `analyze_bookkeeping_consumers` walks the loaded
  AST once at load (exhaustive, compiler-enforced over Stmt/Expr/Pat — no
  wildcard arms) and flags (1) any name within edit distance 2 of
  `fingerprint` (the funnel's own wobble radius: direct calls AND
  wobble-repairable typos reach the builtin; string literals
  over-approximate on purpose), (2) any `Stmt::Regulate` anywhere (it arms
  `trans_edges` mid-run, and trans_integrate's first delta counts ALL prior
  calls — a runtime-state precondition is unsound by construction), (3) any
  `use` import (a module file may carry consumers its host never sees),
  (4) `profiling`/`frame_trace_live` (the profile command + DAP/protocol
  eval can read counts on a source-clean program). Clean → `bk_fast`: only
  `call_clock` keeps advancing. Non-analyzed paths (REPL, direct Interp
  users) never see `bk_fast = true` — default false is the pre-s3b
  contract. Kill switch: `OPERON_VM_BKFAST=0`.
- **Measured** (interleaved A/B, `scripts/bench_core.py` N=9, same-box
  minutes apart): fib25 **1.14x** (114.6→100.4 ms), recursion **1.11x**
  (217.9→196.7), loops/collections/grn 1.00x. Full suite: fib25 **99.9 ms**
  (op/py **7.8x**, best recorded; was 9.0x at session start, ~16x when the
  fib-27 investigation opened), recursion 196.2 ms (8.3x). Micro: m_call
  229→204 ns/call. Deep cross-language bench
  (`apps/ytdl/bench/results_w011s3.json`): every checksum IDENTICAL across
  operon/python/bash.
- **Soundness probes** (byte-identical vs the baseline binary): a direct
  `fingerprint()` call, a wobble-repairable `fingrprint()` call, and a
  mid-run `regulate` arming with translate decay. Pinned as
  `src/interp.rs::w011_s3b_tests` (6 tests: direct call, wobble names,
  nested regulate, clean program, literal over-approximation, and the
  gate-independent 67-call fingerprint count).
- **Gates at close**: cargo test 330 green (324 + 6 new) · clippy 0 · fmt
  clean · proof suite 3422 files / 141 proofs / 2244 asserts · timing
  4f/5p, async 7f/7p + wake-order determinism 3 identical · granted lanes
  green · vm_parity 3555 identical/0 divergent · differential VM lane vs
  oracle 3476/3476 (compat 3204 + non-compat 272) · redteam 109/0 · ytdl
  battery 35/35 (from the repo root — the battery's `use std/args`
  resolves std/ via the CWD-relative candidate; running it from
  `apps/ytdl/` fails module resolution for EVERY binary, a pre-existing
  quirk, not a gate failure).
- **Honest residual**: the remaining call-path cost is the structural
  funnel floor (the ~10-deep Rust call chain, W12-JIT-class work) plus the
  1–2% frame-env residual — both correctly out of s3's reach. The next
  measurable call-path lever stays gated on the roadmap (P4) and its §34
  verdict.

## P2 — sort2k complexity + callback-cost audit (2026-10-04, builder-E)

The roadmap P2 charter (lane E, §34 APPROVED measure-first): determine
whether the comparator-driven sort — `sorted(xs, cmp)` / `xs.sort(cmp)`,
an insertion sort with a user-gene callback per comparison
(`src/interp.rs`, the `.sort()` contract block) — is algorithmic
(O(n log n)), quadratic, or dominated by language callback overhead.
Fixtures (lane-exclusive paths): `scripts/bench/sort_scale.op` (operon),
`sort_scale_py.py` (CPython mirrors), `sort_scale_rs.rs` (native Rust),
`sort_scale.sh` (driver), `sort_scale_profcheck.op` (profiler cross-check).

**Design.** LCG seed-42 data (`x = (x*1103515245+12345) mod 2^31`,
`v = x mod 1e6`) — the identical stream in all three engines. Legs per
n ∈ {1k, 2k, 4k, 8k}: insertion timed min-of-3; exact comparator-call
count (in-language push-cell, `n ≤ 4000`); an in-engine algorithmic
control (`std/heap` `heap_from` + `heap_sorted` with the SAME comparator
gene — O(n log n) comparisons, identical per-callback machinery); CPython
floors (`sorted` builtin = Timsort at C speed; `cmp_to_key` = Timsort +
Python callback); native-Rust insertion with a closure comparator +
`sort_unstable` floor. One operon process per size: the 200M run-wide
step budget (SPEC §9b) cannot fit a multi-size study in one invocation —
a 3-rep 8k leg alone is E1020 (`step budget exhausted`), itself a
workload-shaping property worth knowing.

**Comparator-call counts are EXACT and cross-engine identical** (same
LCG → same permutation → same comparison sequence): 251,830 (1k) →
1,011,638 (2k) → 4,048,443 (4k) → 15,997,582 (8k), verified equal on
operon/CPython/Rust at every measured size (operon counted ≤ 4k; the 8k
count transfers under the differential law). Growth ratio 4.02 / 4.00 /
3.95 per doubling — quadratic, matching the insertion-sort expectation
(n²/4; theory 250k vs measured 251,830 at 1k). The profiler independently
reports `<lambda> 251830` calls at 1k — span count equals the in-language
count exactly (two instruments, one number).

**Wall time, min-of-3, ms** (main `d5483a6`, this box — the BENCH.md
environment table above):

| n   | operon insertion | CPython insertion | Rust insertion | operon heap (control) | CPython builtin | CPython cmp_to_key |
|-----|-----------------|-------------------|----------------|----------------------|-----------------|--------------------|
| 1k  | 95.0            | 47.4              | 0.116          | 68.2                 | 0.088           | 1.07               |
| 2k  | 381.5           | 197.1             | 0.432          | 166.4                | 0.187           | 2.35               |
| 4k  | 1531.6          | 800.3             | 1.693          | 439.0                | 0.418           | 5.55               |
| 8k  | 6118.6          | 3175.1            | 6.568          | 1204.6               | 0.921           | 10.92              |

Per-engine insertion growth per doubling: operon 4.02 / 4.01 / 3.99,
CPython 4.16 / 4.06 / 3.97, Rust 3.72 / 3.92 / 3.88 — quadratic in all
three; the ALGORITHM sets the growth everywhere.

**Diagnosis (the charter question).** Both terms are real and separable:
- **Algorithmic term (dominant):** ×4.0 time and comparison growth per
  doubling in every engine. The in-engine control proves it without
  leaving operon: the heap path with the same comparator gene (and ~4×
  the per-comparison gene-call depth) is already 1.4× faster at 1k and
  5.1× faster at 8k (68.2 vs 95.0; 1204.6 vs 6118.6) purely on n log n
  vs n² comparisons (16,881 vs 251,830 at 1k; 2.23× growth per doubling).
- **Callback term (constant):** per comparison at 8k: operon 382 ns,
  CPython callback analog 198 ns, native Rust closure 0.41 ns — operon is
  1.93× the pure-CPython analog and ~930× native on the matched
  algorithm. For contrast, CPython's `cmp_to_key` (Timsort + Python
  callback, 117 ns/callback) pays 93,049 callbacks at 8k — 172× fewer
  than the insertion contract — and finishes in 10.9 ms.
- **Verdict:** the comparator sort is quadratic BY ALGORITHM with the
  per-comparison constant set by VM callback overhead. Neither factor
  alone explains the gap to production floors (CPython builtin is
  6641× faster at 8k: 0.921 ms vs 6118.6 ms); together they compound.
- **Evidence-backed fix direction (owner-gated, not claimed here):** a
  stability-preserving merge sort behind the same comparator contract
  (`cmp(a,b)` true-when-before, ties keep original order — the differential
  corpus pins stability-observable outputs, so the replacement must be
  stable like insertion sort) cuts comparisons from n²/4 to ~n log n;
  at the measured per-comparison cost that projects 8k to ~40–60 ms
  (~100–150×), with the callback term as the follow-on lever (P4-class).

**Honesty notes.** Min-of-3 in-process `clock()` deltas (sort isolated
from generation and checksum); generation is O(n) and uncharged. The
counted legs run untimed (the push-cell adds O(1) per comparison).
Rust 1k→2k ratio dips to 3.72 (cache effects at 4-byte elements). The
8k operon counted leg is omitted (16M-cell list vs the run-wide budget)
and transfers via the verified differential law instead. Env knobs:
`SORT_SCALE_N` / `SORT_SCALE_REPS` with `--allow-env`; the driver drops
to 2 reps at 8k to stay inside the step budget.
## P3 — loop-memory scaling audit (2026-10-04, builder-E)

Roadmap P3 charter (lane E, §34 APPROVED measure-first): compare
`while i < N` vs `for i in range(N)` at 1M/2M/4M, measure RSS for nested
loops, and classify the growth — live retention / eager collection
materialization / scope allocation / allocator high-water / something
else — with the explicit guardrail "do not call it a leak until the
scaling experiment supports that conclusion." Fixtures (lane-exclusive):
`scripts/bench/loop_mem.op` + `loop_mem_py.py` (CPython mirror) +
`loop_mem_rs.rs` (typed floor) + `loop_mem_driver.py`.

**Design.** Five shapes × {1M, 2M, 4M} iterations, one process per leg
(the 200M run-wide step budget again): `acc_list` (while + push — the
live-retention leg), `transient_while` (no-retention baseline),
`transient_forrange` (the eager-materialization probe),
`transient_while_let` (per-iteration `let` — scope-allocation probe),
`nested` (1000 × n/1000 transient while — the charter's nested ask).
RSS instrument: each leg's peak VmHWM, self-reported by the CPython/Rust
fixtures and driver-polled from `/proc/<pid>/status` for operon — post-exec
accounting, because wait4 `ru_maxrss` carries the spawner's fork floor
(~9.5 MB here) and masks every delta below it. Checksums are cross-engine
identical per shape at every size (differential contract held throughout).

**Peak RSS delta over each engine's own transient baseline (MB):**

| shape             | n   | operon | CPython | Rust (typed) |
|-------------------|-----|--------|---------|--------------|
| acc_list          | 1M  | 33.0   | 39.8    | 8.8          |
| acc_list          | 2M  | 63.6   | 78.9    | 16.8         |
| acc_list          | 4M  | 127.1  | 159.4   | 32.3         |
| transient_while   | 4M  | 0.05   | 0.2     | 0.0          |
| transient_forrange| 1M  | 64.0   | 0.0     | 0.0          |
| transient_forrange| 2M  | 126.0  | 0.0     | 0.0          |
| transient_forrange| 4M  | 251.9  | 0.1     | 0.0          |
| transient_while_let| 4M | 0.0    | 0.1     | 0.0          |
| nested (1000×n/1k)| 4M  | 0.0    | 0.1     | 0.0          |

**Charter classification (measured, not assumed):**
- **Eager collection materialization — CONFIRMED, operon-only.**
  `for i in range(N)` grows RSS linearly at **~66 B/elem** (64.0 → 126.0 →
  251.9 MB) for a body that retains NOTHING, while CPython's and Rust's
  ranges are FLAT across the same sizes (lazy). The ~2× gap vs the
  retained-list cost (33 B/elem) is consistent with a `Vec<Value>`
  materialization riding amortized-doubling high-water (old + new buffer
  live at the copy). This is the memory-side counterpart of the P5/P6
  survey's range() finding (interp.rs:7240, eager materialization, 11.4
  ns/elem) — measured here independently from RSS at charter scale.
  A lazy Range (the survey's smallest-delta candidate) would zero the
  251.9 MB leg, not just shrink it.
- **Live retention — CONFIRMED, linear, constant per-element cost.**
  acc_list: 33.5 B/elem operon (boxed `Value` + Vec slot, matching the
  survey's 32 B/elem boxed-numerics claim), **41.6 B/elem CPython**
  (8 B slot + 28 B int object — CPython retains MORE than operon per
  element), **8.4 B/elem Rust `Vec<u64>`** — the typed-array floor the
  P6 design space targets, now measured from the memory side.
- **Scope allocation — REFUTED as a growth source.** A per-iteration
  `let` inside the loop (transient_while_let) is flat: 3.6 → 3.8 MB
  across 4× the work.
- **Allocator high-water / leak — REFUTED.** Every non-retaining shape is
  flat (±0.05 MB over 4× iterations), nested loops included (charter's
  nested ask: 1000 × 4000 flat). Nothing grows without corresponding live
  data — the scaling experiment supports NO leak conclusion.
- **Time-memory trade note:** the eager materialization is not free speed
  either way — in-process time at 1M: operon forrange 292.9 ms vs while
  414.8 ms (the materialized Vec iterates faster than the while
  machinery); CPython shows the same direction (46.4 vs 66.1 ms) WITHOUT
  paying memory for it. A lazy range must not regress the 1.42× time win
  while removing the 251.9 MB.

**Instrument honesty notes.** operon's `memory()` builtin (arena_bytes /
allocs) does NOT move on any leg — the counters track a small-object
arena path, not the VM's per-iteration global-allocator traffic, so VmHWM
carries the verdict and memory() is reported as static (a real limitation
of the in-process accounting surface for this class of work). The
driver-poll interval is 4 ms; legs end in a flat checksum loop, so the
last-sample-equals-peak assumption is safe at these shapes. Side
observation for the DX queue: `read_file("/proc/self/status")` returns
empty (procfs reports st_size=0; a stat-sized read gets nothing) — bit
the instrumentation once this session.

Fix directions are NOT claimed here (P6 is the survey's owner-gated lane):
the evidence files are the lazy-Range candidate (eager materialization,
66 B/elem) and the typed-array floor (8.4 vs 33.5 B/elem retention).

## P4 — call-overhead profile: same-binary ablation ladder (2026-10-05, builder-E)

**Charter (ROADMAP-BIO-COMPUTATIONAL §P4, §34-AMENDED):** "Profile: gene call
frames, environment lookup, argument binding, telemetry counters, fuel
charging, stack allocation/reuse, bridged calls. Target call-heavy workloads
such as fib25/fib30." The amendment gates any deeper call-frame redesign
(lazy-fenv-class) on **fresh same-binary interleaved evidence** — this section
is that evidence. Every leg below runs the SAME release binary (2.8.0-vm @
main `0f20cec`) over the IDENTICAL workload (`scripts/bench/call_fib.op`,
`CALL_FIB_N` parameterized), interleaved round-robin, min-of-3, in-process
clock — process-level configs differ only in `OPERON_W009A_ABLATE` /
`OPERON_W009A_COUNTS` (the W009-A harness, read at process start, inert at
one AtomicBool load per site otherwise).

**Fixtures (lane-exclusive):** `scripts/bench/call_fib.{op,py}` + `call_fib_rs.rs`
(the charter's named target, timed legs + analytic count legs);
`call_ablate.sh` (the ladder driver + counters); `call_args.{op,py,rs}` +
`call_args.sh` (arity ladder, slot vs env lanes); `call_bridged.{op,py}`
(direct-builtin vs gene-wrapped vs pure-gene). Zero Rust delta.

### The differential anchor: call counts and shape constants

| quantity | fib25 | fib30 | law |
|---|---:|---:|---|
| gene calls C(n) | 242,785 | 2,692,537 | C(n)=2·F(n+1)−1, engine-independent (count legs byte-match in op/py/rs) |
| `bookkeep` counter | 242,786 | 2,692,538 | exactly C(n)+1: main is invoked as a gene (entry) |
| `slot_frames` | 242,785 | 2,692,537 | exactly C(n): every call rode the W011-s2 slot path |
| VM instructions | 1,699,525 | 18,847,789 | **7.00 instrs/call** at both sizes |
| `env_new` | 364,181 | 4,038,809 | **exactly 1.5 env allocs/call**: 1 frame env per call + 1 block env per leaf (the `n<2` branch body) + 2 run-wide (364181 = 242786 + 121393 + 2 ✓) |
| `mono_hits` | 242,782 | 2,692,534 | C(n)−3: 3 misses to prime the mono-cache, then all hits |
| `tb_clones` / `promo_clones` | 0 / 0 | 0 / 0 | the W011-r2 lazy-traceback + promo-early-return optimizations are LIVE — zero happy-path String clones remain |

### Ladder — removable per-call work (min-of-3 ms, deltas in ns/call)

| cfg | fib25 ms | fib30 ms | Δ vs base (ns/call, from n=30) |
|---|---:|---:|---:|
| base | 98.464 | 1093.148 | — (406.0 ns/call) |
| `tick` (no per-instr step check) | 96.590 | 1066.879 | **−9.8** |
| `gates` (no regulatory block) | 96.725 | 1073.026 | **−7.5** |
| `bk` (no counters/buckets/clock) | 98.184 | 1088.732 | **−1.6** |
| `decay` (no m6a/grn tickers) | 99.350 | 1090.287 | −1.1 (subset of bk) |
| `tb` / `promo` (no clones) | 98.560 / 99.445 | 1092.402 / 1091.880 | ≈ 0 (already optimized away) |
| `pool` (env recycling OFF) | 104.584 | 1162.063 | **+25.6** — the recycling's banked win |
| `workless` (tick+gates+bk+decay+tb+promo) | 94.487 | 1045.981 | **floor = 388.5 ns/call** |
| `all` (workless + pool off) | 100.840 | 1125.067 | mixed direction, sanity-consistent |

**Verdict on the removable layer:** after W009-A→W011-r2→s2→s3, only
**~17.5 ns of the 406 ns/call (4.3%)** is still removable via the ablation
flags — fuel tick 9.8, regulatory gates 7.5, bookkeeping 1.6 (decay inside
it 1.1), clones 0. The remaining **388.5 ns/call floor is the call machinery
itself**: frame-Env allocation (1.5 envs/call measured above), the slot-Vec
alloc, arg clone/define (`resolve_param`), and the 7 VM instruction
dispatches. The same-binary evidence the amendment asked for, in one line:
**the lazy-fenv-class question is now a question about the 388 ns floor, not
about the gate/counter layers** — and the measured 1.5 envs/call is the
biggest identifiable chunk inside it (the pool result — recycling the same
structures is already worth 25.6 ns/call — bounds the alloc cost from below).

### Cross-engine anchors (same box, same day)

| engine | fib25 | fib30 | ns/call | op/py |
|---|---:|---:|---:|---:|
| operon 2.8.0-vm | 98.46 ms | 1093.15 ms | 405.5 / 406.0 (linear ✓) | — |
| CPython 3.12 | 10.32 ms | 114.30 ms | **42.5 / 42.5** (exactly linear) | **9.5x / 9.6x** |
| native Rust (`-O`, `#[inline(never)]`) | 0.172 ms | 1.897 ms | 0.71 / 0.70 | ~570x |

CPython's per-call cost is *exactly* size-independent (42.5 both sizes) — a
clean interleaved anchor. The operon/CPython gap has narrowed from the
W009-A investigation's ~11x (595 vs 54) to **9.5x** via the W011 rounds, all
of it now in the floor. Coherence check with P2: the sort comparator
callback (382 ns/comparison) and the fib call (406 ns/call) are the same
call-path constant measured through two different doors.

### Argument binding: arity ladder (200k calls/leg, min-of-3)

| arity | operon env-lane | operon slot-lane | CPython | rs floor |
|---|---:|---:|---:|---:|
| 0 | 544.5 | 547.5 | 87.2 | (folded) |
| 1 | 691.5 | 703.6 | 104.6 | (folded) |
| 2 | 825.5 | 831.2 | 123.3 | 0.56 |
| 4 | 1135.8 | 1156.4 | 169.7 | 0.60 |
| 8 | 1799.4 | 1872.0 | 261.0 | 1.02 |

- **Per-arg marginal binding cost: ~166 ns/arg (slot) / ~157 ns/arg (env)**
  vs CPython's **21.7 ns/arg** — **7.2-7.6× steeper per argument**. Arity-0
  call: 545 vs 87 ns = 6.3×. Every extra parameter costs real time in
  resolve_param + define/write-through + the extra slot.
- **Slot-vs-env reversal (shape-dependent, both directions reported):** on
  fib25 (deep recursion, 1 param) the slot path WINS by 10.3 ns/call (98.65
  vs 101.15 min). On the 200k shallow-loop ladder the slot lane LOSES at
  every arity ≥ 1 (e.g. 1872.0 vs 1799.4 at arity 8). The write-through
  design (slot copy AND `define_param` per arg) pays a double-binding tax
  that only pays off when param reads dominate (deep recursion), and loses
  when calls dominate over reads (shallow bodies). Classification evidence
  for the fix phase — not a fix claim.

### Bridged calls: builtin vs gene hop (200k calls, min-of-3)

| leg | operon ns/call | CPython ns/call | op/py |
|---|---:|---:|---:|
| direct builtin (`abs(-i)`) | 550.8 | 105.6 | 5.2x |
| gene-wrapped builtin (`wrap(i)→abs`) | 804.4 | 119.7 | 6.7x |
| pure gene (`idneg(i)→-i`) | 754.6 | 117.2 | 6.4x |

- **One gene-call hop costs +253.6 ns** on top of the same builtin body
  (804.4 − 550.8); CPython's def hop is +14.1 ns — an **18× steeper hop**.
- **The bridge itself costs +49.8 ns** inside an otherwise identical gene
  body (804.4 − 754.6): Value build + arg Vec + dispatch + Int unbox on the
  way into the builtin.

### Honesty notes

- Machine noise ±1-2% run-to-run despite min-of-3 interleaving; single-flag
  deltas < 2 ms on fib25 are inside the noise band (promo/decay/tb rows) and
  the fib30 legs are the resolution anchor — every headline delta quoted
  comes from n=30 where the same effect is 11× larger.
- `ticks` counter reads 0 in the VM lane: it counts tree-walk eval ticks;
  the VM lane charges steps in its dispatch check — which the `tick`
  ablation still removes (−9.8 ns/call, real).
- The rs arity-0/1 legs are constant-folded by LLVM (pure callees hoisted
  out of the loop despite `inline(never)`); only the 2/4/8 rows are genuine
  native floors.
- The bridged probe uses Int-only bodies by design (no alloc noise); real
  workloads with String/Vec args pay more.
- Counters runs (`OPERON_W009A_COUNTS=1`) add per-call atomic fetch_adds;
  they are reported separately and never mixed into timed rows.
- Fix directions are NOT claimed here (L-002/`src/interp.rs` is builder-A's
  ACTIVE P4-safe window): the evidence files are (1) the 1.5 envs/call +
  388 ns floor for the lazy-fenv-class redesign (owner-gated per §34), (2)
  the per-arg 157-166 ns and the slot write-through reversal for the binding
  path, (3) the +49.8 ns bridge delta for builtin-call batching.

## P2+P3+P5 — the fix wave (2026-10-06, builder-A)

The three evidence-backed fix directions from the audits above landed as one
wave (owner-authorized "start optimization"), zero contract drift — the
differential corpus (3,615 programs × 2 lanes), vm parity (3,575 programs ×
4 modes), the compat matrix (3,204 × 5 engines), proofs (143/143, 2,285
asserts, both engines), redteam (109/0) and cargo tests (129/0) all green on
the wave head. Pins: `tests/differential/p2p3p5_opt_pins.op` (byte-identical
on both engines, including the tie-order proof below).

### P2 — comparator sort: insertion → tie-exact merge

`sorted(xs, cmp)` / `xs.sort(cmp)` now run a bottom-up merge sort above
n = 32 (≤ 32 keeps the original insertion walk VERBATIM, so small pinned
programs and side-effect-ordered comparators keep their exact call order).
The merge's tie rule — take right iff NOT cmp(left, right) — reproduces the
insertion walk's tie semantics byte-for-byte (a tie swaps in the walk, so
tie groups end up reversed; the oracle still runs the insertion walk, so the
differential IS the tie-rule proof, at n = 50/64/100 with 7-way tie groups).

| n | old (insertion) | new (merge) | speedup | comparator calls old → new |
|---|---:|---:|---:|---|
| 1k | 95.0 ms | 3.57 ms | **26.6×** | 251,830 → 8,713 |
| 2k | 381.5 ms | 7.87 ms | **48.5×** | 1,011,638 → 19,452 |
| 4k | 1,531.6 ms | 17.4 ms | **87.8×** | 4,048,443 → 42,920 |
| 8k | 6,118.6 ms | 38.2 ms | **160×** | 15,997,582 → ~88k (transfer law) |

Growth per doubling is now 2.20× / 2.22× / 2.19× (O(n log n), was 4.0×);
checksums are byte-identical to the audit's recorded values at every size.
vs CPython: the comparator sort was 1.93× SLOWER than CPython's own
insertion analog at 8k — it is now **83× faster** (38.2 ms vs 3,175 ms).
The in-engine heap control (1,189 ms at 8k) is now 31× slower than the new
path — it served its purpose as the algorithmic witness and retires.

### P3 — `for i in range(...)`: eager materialization → lazy bounds

When the iterable IS a range call, the loop iterates the bounds directly
(O(1) memory) with the builtin's exact observables: same note on non-int
args (E2016, stamped at the call line), same `unfolded` stress on step 0,
same `overflow` stress above the 10M ceiling (O(1) count check, i128 math),
same per-iteration tick and binding; every other surface (`len(range(..))`,
indexing, passing to list-expecting builtins) still goes through the eager
builtin unchanged.

| leg (4M iterations) | old peak RSS | new peak RSS |
|---|---:|---:|
| transient for-range | **251.9 MB** over baseline | **~0 MB** (12.7 MB total, = the while baseline) |

The 66 B/elem materialize-then-clone double copy is gone. Time: the m_forrange
micro shows no regression (55.4 ms / 200k vs 57.3 recorded pre-wave; the
known ±8–17% cross-session drift band applies); the materialized Vec's
contiguous-iteration edge at multi-million scale narrows to ~±17% vs while
(4M wall: 1,070 ms lazy vs 911 ms while on this box) — the trade is 252 MB
for ≤ noise at corpus scale.

### P5 — string accumulation: clone + format! + rebind → in-place append

The accumulation shapes now append IN PLACE to the bound `Value::Str` slot
(amortized O(|rhs|) per op): `s += x` unconditionally (the general path
already read the slot after the rhs eval), and `s = s + <pure rhs>` behind a
static side-effect-freedom gate (`expr_is_pure`: literals/idents/arithmetic
only — no calls), with the exact Add-arm ceiling (512 MiB) and mem-charge
amounts at the exact positions. The VM compiles both shapes to the new
`AppendName` opcode (SPEC §15 row added; fallback reproduces the replaced
LoadName(Quiet) + Bin + AssignName sequence for every non-Str shape, unbound
names, const targets and slot targets — both machines). `Value::Str` is an
owned `String` (never Rc-shared), so in-place mutation is invisible to every
other holder by construction.

| micro | old | new | |
|---|---:|---:|---|
| m_strcat (12k `s = s + "ab"`) | 10.9 ms / 454 ns-op | **2.69 ms / ~112 ns-op** | **4.05×** |
| Str+Str Add (general path) | `format!` per op | exact-capacity single alloc | lower constant everywhere |

The survey's 4–13× projection lands at 4.05× on the 12k fixture (the
quadratic term is still modest there; the win grows with N).

### Real bug found by the wave's pins (fixed)

`tests/differential/p2p3p5_opt_pins.op` exposed a LATENT oracle divergence:
the oracle's `range()` builtin crashed on any non-int argument (Python
TypeError, rc=1) where the Rust engine notes + returns `[]` — the corpus
never generated that shape. The oracle now mirrors the note + `[]` exactly.

### Wave-era VM note

rt_p17b_scope_cancel joined the vm_parity RACY_NOTE class: the payload's
busy loop rebinds a name every iteration and cancel lands at a timing-
dependent position, so the count of loop-emitted notes is 0..N (the P3 lazy
range shifted the race window enough to expose the latent flake in the
sweep; 9/9 serial runs byte-identical). stdout+rc stay strictly compared;
only the loop-emitted note lines are dropped for that payload.
