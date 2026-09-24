# BENCH — the Operon benchmark suite

Owner: builder-B (B1). Baseline: **v2.2.0** (main @ `95ffef7`). This file is the
measured answer to "how fast is Operon, where does the time go, and what should
the v3.0 Ribosome VM fix first?" It is regenerated per release; historical rows
accumulate in *Baseline tracking* below.

Audience note (D-008): everything here is programmer-first. `gene` ≈ a function,
`regulate` ≈ a runtime feature flag with thresholds — no biology required to
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
CI push — a benchmark that measures the wrong answer fails CI before it can
mislead anyone.

## Methodology (and honesty notes)

- **Three runners on identical algorithms:**
  - **operon** — the Rust core, `bin/operon run <fixture>`.
  - **oracle** — `bootstrap/oracle.py` running the *same .op file*; the CPython
    tree-walking semantic mirror used for differential parity. It shares the
    architecture (assoc maps, tree-walk), so it is an architecture reference,
    not a speed target.
  - **native-py** — the same algorithm hand-written in pure CPython
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
  (`bench_compare.py` `MICROS`) — indicative, not a hardware benchmark.

## Environment (this baseline)

| field | value |
|---|---|
| date | 2026-09-24 |
| host | c-6ab50512-14810412 (containerized CI-class VM) |
| cpu | Intel(R) Xeon(R) Processor |
| os | Linux 5.10.134 x86_64, glibc 2.41 |
| python | 3.12.14 |
| operon | v2.2.0 release binary = main @ 95ffef7 |

Rerun on your machine before quoting ratios in an argument — absolute times
move, the *relative shape* (maps ≫ calls ≫ loops) does not.

## Results — named workloads (v2.2.0 baseline)

| workload | operon (ms) | oracle (ms) | native-py (ms) | op/py | op/oracle | calls/iters |
|---|---:|---:|---:|---:|---:|---:|
| fib25 | 128.0 | 2434.2 | 12.8 | **10.0x** | 19.0x | 242,785 calls |
| loops | 62.9 | 865.8 | 11.5 | **5.5x** | 13.8x | 200,000 iters |
| strings | 25.2 | 115.4 | 0.8 | **33.5x** | 4.6x | 4,000 replaces |
| collections | 319.1 | 22537.4 | 5.3 | **60.0x** | 70.6x | 20k map ops + 20k pushes |
| recursion | 230.9 | 4969.9 | 23.6 | **9.8x** | 21.5x | 369,511 calls |
| grn | 32.7 | 515.4 | 8.5 | **3.8x** | 15.7x | 60,000 gated calls |

`op/py` = operon vs native CPython — **the v3.0 gap to close**.
`op/oracle` = how much faster the Rust core already is than its Python mirror.

## Results — micro (per construct, v2.2.0 baseline)

| micro | operon (ms) | oracle (ms) | native-py (ms) | op/py | op ns/op | py ns/op |
|---|---:|---:|---:|---:|---:|---:|
| m_empty (startup) | 0.9 | 54.4 | — | — | — | — |
| m_call | 64.2 | 766.9 | 8.4 | **7.6x** | 214 | 28 |
| m_forrange | 52.7 | 624.6 | 9.0 | **5.9x** | 88 | 15 |
| m_while | 60.9 | 1323.4 | 11.9 | **5.1x** | 102 | 20 |
| m_varread | 59.1 | 653.5 | 9.0 | **6.5x** | 98 | 15 |
| m_intadd | 85.4 | 950.1 | 13.6 | **6.3x** | 95 | 15 |
| m_listpush | 16.5 | 230.0 | 2.3 | **7.3x** | 110 | 15 |
| m_listidx | 37.5 | 581.2 | 7.6 | **4.9x** | 125 | 25 |
| m_mapset | 429.6 | 30280.9 | 5.3 | **80.7x** | 5370 | 67 |
| m_mapget | 571.6 | 40277.3 | 7.8 | **73.7x** | 3712 | 50 |
| m_strcat | 9.9 | 91.9 | 0.7 | **14.7x** | 412 | 28 |

## Reading the numbers

- **Maps are the emergency.** `m_mapset`/`m_mapget` run **70–80x** slower than
  native CPython; the mixed `collections` workload is **60x**. Maps are stored
  as association lists (`Vec<(Value, Value)>`) scanned linearly with
  `deep_eq` per element — O(n) per insert/lookup, and keys are re-hashed
  strings. The oracle agrees with this diagnosis the hard way: cProfile shows
  **58,025,002 `deep_eq` calls (42 s of 84 s)** for the 20k-op collections
  fixture. The fix is representational, not micro-tuning.
- **Calls are the chronic gap.** Naive recursion (fib25, C(20,10)) lands at
  **~10x**; an empty-ish gene call costs ~214 ns/op vs CPython's 28 ns. The
  call path allocates: the callee's `name.clone()` is cloned up to three times
  per call for the counters/burst bins before the body even starts.
- **Plain loops are close-ish.** `for`/`while` arithmetic sits at 5–6x — the
  dominant cost is a fresh `Env` (a new `Rc` + `HashMap`) allocated **per loop
  iteration** to bind one variable, plus `tick()` bookkeeping per node.
- **Strings leak allocations.** `s = s + "x"` reallocates a fresh `String` per
  op (14.7x), and the `strings` fixture (template + replace + concat) hits
  33.5x — but note its native-py time (0.8 ms) is at the startup-noise floor;
  trust m_strcat's 412 ns/op instead.
- **The v2.2 honesty machinery is cheap.** The GRN-gated workload is the
  *closest* to native (3.8x). `operon profile` on `grn.op` shows the three
  regulated genes at ~2.1 ms self each for 20k calls (~107 ns/call including
  the gate walk) — T2a/T2e gating does not need rescuing for v3.0; do not let
  the VM design regress it either.

## Top-10 interpreter hot-path targets (input to the v3.0 Ribosome VM design)

Ordered by expected payoff. Line references are read-only analysis anchors on
main @ 95ffef7 (`src/interp.rs`); no src/* changes are proposed for v2.2.

1. **`Value::Map` as an association list** — `map_insert` (interp.rs:956) and
   every map read do a linear `deep_eq` scan. Replace with a real hash map over
   scalar keys (SPEC already requires scalar keys via `key_scalar`), keeping
   insertion order for deterministic iteration (§19 snapshot iteration).
   Expected: map-heavy code 5–15x. *Highest single payoff.*
2. **Env allocation per loop iteration** — `Stmt::For` (interp.rs:566–620)
   builds `Env::new(Rc + HashMap)` per item to bind one variable. Slot-based
   locals, or a reused single-slot iteration frame. Expected: loops ~2x.
3. **Per-call `String` churn in the call path** — `call_gene_inner`
   (interp.rs:1779+) clones the callee name 2–3x per call for `call_counts` /
   `gene_buckets` keys. Intern gene names (symbol table → index), make
   telemetry arrays index-addressed. Expected: call-heavy code ~1.5–2x.
4. **`for` over a list materializes a full clone** — `l.borrow().clone()`
   (interp.rs:~597) copies the entire list before the first iteration. Iterate
   the `Rc` with an index instead.
5. **`tick()` per eval node and per iteration** — interp.rs:368, called first
   thing in `eval` (969) and in every loop body: a modulo + branch per AST node
   for fuel accounting. Charge fuel in batches (branch-free decrement, check on
   underflow) or only at loop back-edges and calls. Expected: ~5–10% across
   the board.
6. **String concatenation reallocates per op** — `apply_binop` Add
   (interp.rs:~1327): `format!` + per-op `mem_charge`. A rope / amortized
   builder for the `s = s + x` accumulation shape. Expected: m_strcat ~5x.
7. **String literals clone per evaluation** — `Expr::Str(s.clone())`
   (interp.rs:975). Pool literals per program (`Rc<str>`); the parser already
   walks them once.
8. **GRN gate per-call costs** — `grn_veto` (interp.rs:~1762) walks all edges
   filtered per callee and scans `enhanced` per call; `grn_fire`
   (interp.rs:~2718/2751) clones the whole `grn_levels` map twice per fire.
   Index edges by callee, precompute the enhanced set, mutate levels in place.
   Cheap today (3.8x overall) — keep it cheap as nets grow.
9. **`silences` linear scan + clone per call site** — `Expr::Call`
   (interp.rs:~1081) scans the silence list (and clones the match) on every
   call. Resolve callees once per site (inline cache), invalidate on
   `silence`/`acetylate` mutations.
10. **Builtin dispatch by string `match` per call** — `call_builtin`
    (interp.rs:2159) matches `&str` each call. Intern builtin names at parse
    time (or perfect-hash) → enum dispatch.

**The v3.0 design takeaway:** targets 2, 3, 5, 7, 9, 10 are exactly the costs a
bytecode VM with stack frames, slot variables, interned constants, and inline
caches dissolves *by construction*. Target 1 is independent of bytecode-vs-tree
(the Value representation must change either way) and is the one thing that can
ship early without waiting for the VM. Targets 4, 6, 8 are tree-walk
mitigations worth doing only if the tree walker stays as the fallback core
after v3.0 — which the differential harness needs it to.

## Baseline tracking

| version | commit | date | fib25 op/py | loops op/py | collections op/py | grn op/py |
|---|---|---|---:|---:|---:|---:|
| v2.2.0 | 95ffef7 | 2026-09-24 | 10.0x | 5.5x | 60.0x | 3.8x |

(Add a row per release; ratios from the default `--iters 5` run.)

## Reproducing

```sh
bash scripts/bench.sh --json /tmp/bench.json        # named workloads
bash scripts/bench.sh --micro --json /tmp/micro.json
./bin/operon profile scripts/bench/grn.op           # per-gene self time
python3 bootstrap/oracle.py run scripts/bench/collections.op   # oracle-side
python3 -m cProfile -s tottime bootstrap/oracle.py run scripts/bench/collections.op
```

Fixture → correctness-proof mapping: `tests/bench/bench_correctness.op`.
Questions / profile requests: `[builder-B -> sz]` thread in
`project-vault/collab/COMMS.md`.
