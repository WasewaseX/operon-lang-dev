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

## Results, micro (per construct, re-measured on `e757b4d`)

| micro | operon (ms) | oracle (ms) | native-py (ms) | op/py | op ns/op | py ns/op |
|---|---:|---:|---:|---:|---:|---:|
| m_empty (startup) | 0.9 | 52.6 |, |, |, |, |
| m_call | 63.3 | 761.7 | 8.4 | **7.5x** | 211 | 28 |
| m_forrange | 50.9 | 614.1 | 8.9 | **5.7x** | 85 | 15 |
| m_while | 59.9 | 1315.0 | 11.9 | **5.0x** | 100 | 20 |
| m_varread | 56.7 | 657.1 | 9.1 | **6.3x** | 94 | 15 |
| m_intadd | 86.0 | 945.6 | 13.6 | **6.3x** | 96 | 15 |
| m_listpush | 16.3 | 226.9 | 2.3 | **7.1x** | 108 | 15 |
| m_listidx | 38.0 | 579.7 | 7.7 | **5.0x** | 127 | 26 |
| m_mapset | 445.0 | 31634.9 | 5.3 | **83.6x** | 5563 | 67 |
| m_mapget | 583.2 | 40460.4 | 7.8 | **74.5x** | 3787 | 51 |
| m_strcat | 10.0 | 96.2 | 0.7 | **14.7x** | 416 | 28 |

## Reading the numbers

- **Maps are the emergency.** `m_mapset`/`m_mapget` run **70–80x** slower than
  native CPython; the mixed `collections` workload is **60x**. Maps are stored
  as association lists (`Vec<(Value, Value)>`) scanned linearly with
  `deep_eq` per element, O(n) per insert/lookup, and keys are re-hashed
  strings. The oracle agrees with this diagnosis the hard way: cProfile shows
  **58,025,002 `deep_eq` calls (42 s of 84 s)** for the 20k-op collections
  fixture. The fix is representational, not micro-tuning.
- **Calls are the chronic gap.** Naive recursion (fib25, C(20,10)) lands at
  **~10x**; an empty-ish gene call costs ~214 ns/op vs CPython's 28 ns. The
  call path allocates: the callee's `name.clone()` is cloned up to three times
  per call for the counters/burst bins before the body even starts.
- **Plain loops are close-ish.** `for`/`while` arithmetic sits at 5–6x, the
  dominant cost is a fresh `Env` (a new `Rc` + `HashMap`) allocated **per loop
  iteration** to bind one variable, plus `tick()` bookkeeping per node.
- **Strings leak allocations.** `s = s + "x"` reallocates a fresh `String` per
  op (14.7x), and the `strings` fixture (template + replace + concat) hits
  33.5x, but note its native-py time (0.8 ms) is at the startup-noise floor;
  trust m_strcat's 412 ns/op instead.
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

## Baseline tracking

| version | commit | date | fib25 op/py | loops op/py | collections op/py | grn op/py |
|---|---|---|---:|---:|---:|---:|
| v2.2.0 | 95ffef7 | 2026-09-24 | 10.0x | 5.5x | 60.0x | 3.8x |
| v2.2.0+audit1 | e757b4d | 2026-09-24 | 10.0x | 5.6x | 61.8x | 3.9x |

(Add a row per release; ratios from the default `--iters 5` run.)

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
