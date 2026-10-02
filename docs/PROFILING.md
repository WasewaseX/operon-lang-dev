# Profiling & benchmark instrumentation guide

*W097-A (builder-E, profiling lane). The canonical procedure for measuring
Operon runtime behavior — one entry point for every instrument the project
ships, with the discipline rules that keep a measurement honest. Bench
tables themselves live in `BENCH.md`; the VM design's measured problem is
`docs/vm-design.md` §1, and its measured attribution is §12.*

## 0. The discipline rules (read before trusting any number)

1. **Rebuild before you measure.** After any source edit, `bash
   scripts/build.sh` and check `./bin/operon --version` matches
   `Cargo.toml`. A stale binary is a STALE MEASUREMENT — every claim it
   produces is void (the same law that voids stale gate results).
2. **Verify correctness inside the timing loop's workload.** fib25 must
   print `fib(25) = 75025` before any timing is quoted from it. A fast
   wrong answer is not a measurement (the W009-A method, adopted here).
3. **Median of N, whole process.** Time the full binary invocation
   (process start + parse + run), never a single wall-clock reading, and
   report the median. The W082 lesson: this sandbox family has an
   I/O-bound noise floor (`file_io` spread 61–250 ms on identical code);
   `scripts/perf_gate.py` carries `NOISY_THRESHOLDS` with the measured
   evidence — a 20% gate is meaningless for noisy workloads, and so is a
   single-run claim.
4. **Name the lane.** The VM is the default engine; `--no-vm` is the
   tree-walk lane; the Python oracle is the differential reference (its
   side of a comparison: `python3 -m cProfile -s tottime
   bootstrap/oracle.py run …`). A number without its lane is not a
   number.
5. **Differential law first.** Any instrumentation you add must be
   default-off and byte-silent on the standard lanes. Spans, counters,
   and traces are operator diagnostics; the corpus runs none of them.

## 1. Instruments at a glance

| Instrument | Surface | Grain | Overhead |
|---|---|---|---|
| `scripts/bench.sh` (+ `--micro`) | named workloads, JSON out | workload wall time | none (separate runs) |
| `operon bench f.op --iters n` | one program, internal loop | program wall time | none |
| `scripts/perf_gate.py` | regression gate vs `BENCH.md` thresholds | workload median vs threshold | none |
| `operon profile f.op` | per-gene aggregate table | gene, exclusive self-µs + call counts | +28% on fib25 (measured below) |
| `operon profile f.op --json` | same, machine-readable (`operon-profile` schema) | gene aggregates | same |
| `operon profile f.op --chrome t.json` | **per-call spans**, Chrome Trace Format | every gene call: ts, dur, depth | +83% on fib25 on top of aggregate (measured below) |
| `OPERON_W009A_COUNTS=1` (builder-A, PR #55, scratch class) | VM instruction/env/clone counters | per-mechanism totals | see PR #55 |
| `memory()` builtin | symbol-table gauges (`arena_bytes`, `interns`, `allocs`, `cycles`) | process-cumulative | negligible |
| `operon run f.op --trace-grn t.jsonl` | GRN tick-stream (W095) | one JSONL frame per engine update point | none unless flagged |
| W08r debugger | stepping, breakpoints, NDJSON/DAP | statement grain | debug session only |

## 2. Micro/macro benchmarks (the BENCH.md pipeline)

```sh
bash scripts/build.sh                              # rule 1: fresh binary
bash scripts/bench.sh --json /tmp/bench.json       # named workloads
bash scripts/bench.sh --micro --json /tmp/micro.json
./bin/operon bench examples/fib.op --iters 5       # one program, N iters
```

Publish a BENCH.md table row per release (see the "Version history"
table there) and run `scripts/perf_gate.py` for the regression gate.
Correctness inside the bench fixtures is pinned by
`tests/bench/bench_correctness.op`. The oracle side of any comparison:
`python3 bootstrap/oracle.py run <file>` (and cProfile for its hot path).

**The W009-A baseline** (builder-A, PR #55) is the standing call-path
reference: fib25 = 144.2 ms VM == 145.3 ms tree-walk == 143.2 ms `--opt2`
on the reference runner — the bottleneck is the SHARED call funnel, not
the bytecode machine; ~11.0x CPython on fib25. Evidence artifacts:
`docs/bench/2026-10-02-w009a-{baseline.json,ablation.txt}`; the full
attribution and the refuted suspects are recorded in `docs/vm-design.md`
§12 and `BENCH.md` ("W009-A"). Re-derive against it before claiming any
regression or improvement.

## 3. Aggregate profiling (`operon profile`)

```sh
./bin/operon profile examples/fib.op               # human table
./bin/operon profile examples/fib.op --json        # "operon-profile" schema
```

The table prints per-gene **exclusive self-time µs** (children
subtracted — a caller never inflates itself with its callees' cost),
call counts, regulation flags (`enhanced active repressed`), the
`mature · nascent · maturation` summary, and enhance candidates. The
`--json` schema is self-describing and byte-stable (pinned by tests);
`--chrome` does not change it (§4).

## 4. Per-call spans → Chrome Trace (`--chrome`, W096/W097-A)

```sh
./bin/operon profile examples/fib.op --chrome /tmp/fib25.json
# open ui.perfetto.dev (or chrome://tracing) and load the file
```

What you get: one `ph="X"` complete event per gene call — `{name,
cat:"gene", pid:1, tid:1, ts, dur, args:{depth}}` — plus two `ph="M"`
metadata events, on ONE logical timeline. The interpreter is
single-threaded (fibers ride the deterministic virtual clock on the same
thread; workers would be attributed by W098's WORKER-TELEMETRY surface,
not invented here). `ts`/`dur` are microseconds on the same monotonic
clock the aggregate table uses, so trace intervals and the self-time
table reconcile. `otherData` self-describes the file: format tag,
version, source file, unit, clock, `total_spans`, `dropped_spans`,
`span_cap`.

Contract highlights (normative wording in SPEC §11 telemetry; shape
pinned by `tests/profile_spans.rs`):

- **Capture arms only under `--chrome`, before load** — top-level calls
  executed during `load_file` are in the timeline (the dx-r1 rule).
- **Nesting**: `depth` = number of live ancestor frames at call time
  (outermost gene call = 0). Every depth-d span is interval-contained in
  some depth-(d−1) span — the property Perfetto uses to rebuild the
  call tree, and the pinned cross-check.
- **Cap**: the span log holds at most 1,000,000 spans; past the cap,
  spans are DROPPED and counted in `dropped_spans`. A trace never lies
  silent about its own limits.
- **`--json` stays byte-compatible.** With `--json --chrome`, the
  operator notice (`chrome trace: … (N span(s), M dropped past the
  1000000 cap)`) rides stderr so the JSON stream stays parseable.
- **Ordering**: events appear in completion order (chronological by
  end time). Renderers sort by `ts`; consumers that need start order
  sort themselves.

Measured shape on `examples/fib.op` (fib25, release binary, this
instrument, 2026-10-02): **242,786 spans** (fib 242,785 = C(25) exactly +
main 1), depth histogram peaking at 18 (52,666 calls at depth 18), the
textbook recursion profile — 2^k calls at depth k+1 until the quadratic
crossover. `main`'s inclusive duration is the whole program's call-tree
time.

**Instrument overhead, measured (median of 5 whole-process runs, same
session, same binary):**

| mode | fib25 median | delta |
|---|---|---|
| `run` (unprofiled) | 142.1 ms | — |
| `profile` (aggregate) | 182.6 ms | +28% (the pre-existing now_ns bookkeeping) |
| `profile --chrome` | 333.4 ms | +83% spans on top of aggregate (~0.6 µs/call) |

The unprofiled lane is byte-silent and cost-free (the capture branch is
one bool check inside the already-profiling-gated `close_timing` block).
The spans mode is a **diagnostic instrument, not a timing harness** —
quote its ts/dur for attribution and shape, never as performance
numbers; for numbers use §2 with the profiler off. Aggregate mode's +28%
is the pre-existing profiler cost, unchanged by this work.

## 5. Ablation counters (builder-A's scratch class)

`OPERON_W009A_COUNTS=1` (PR #55) exposes per-mechanism totals
(instructions, env operations, clones) for the W009 call-path
investigation. Different grain from spans: counters answer "how many of
mechanism X ran", spans answer "when and how deep did each call go".
They compose: a spans capture localizes WHERE, the counters attribute
WHAT. The counters are scratch-class evidence tooling, not a contract —
see PR #55 and `docs/vm-design.md` §12 before relying on a field.

## 6. Memory gauges

`memory()` returns process-cumulative symbol-table gauges:
`arena_bytes` (live interned-symbol bytes — NOT process RSS, NOT
interpreter values), `interns` (interned canonical spellings),
`allocs` (monotonic allocation count), `cycles` (live detected
reference cycles, W013, contract in SPEC §19e). Honest limit: a
symbol-table gauge, not a heap profiler — per-gene allocation
attribution is the MEM-PROFILER design
(`docs/design/MEM-PROFILER.md`), deferred post-W009 by design.

## 7. Regulation + debugger observation surfaces

`--trace-grn` (W095) emits the GRN tick-stream: one JSONL frame per
engine update point (`{"tick":N,"phase":"fire"|"decay","levels":{…}}`),
deterministic, 200k-frame cap, drained after the run. The W08r debugger
(statement stepping, breakpoints, NDJSON protocol, DAP) is the
interactive observation surface; `docs/DEBUGGER.md` is its manual.
Both are default-off diagnostics under the same differential law as §0.5.

## 8. A worked, reproducible session

```sh
bash scripts/build.sh && ./bin/operon --version          # rule 1
./bin/operon run examples/fib.op                          # rule 2: fib(25) = 75025
for i in 1 2 3 4 5; do ./bin/operon bench examples/fib.op --iters 5; done   # rule 3
./bin/operon profile examples/fib.op --chrome /tmp/fib25.json
# ui.perfetto.dev ← /tmp/fib25.json        (attribution + shape)
./bin/operon profile examples/fib.op --json > /tmp/agg.json
# cross-check: sum(gene.calls) == otherData.total_spans (minus metadata events)
```

Cross-check law: the aggregate view and the span view must agree on call
COUNTS (they share `close_timing`). If they ever disagree, the
instrument is broken — stop and report, do not average it away.
