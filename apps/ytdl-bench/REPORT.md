# ytdl — Cross-Language Deep Benchmark Report

**App**: a lightweight video-download orchestrator (doctor / info / get / meta /
queue) driving `yt-dlp` + `ffmpeg` + `ffprobe` from PATH, implemented four
times with an identical CLI contract:

| Port | Language | Runtime | Files |
|------|----------|---------|-------|
| operon/ | **Operon** (genes, 100% of app logic) | Operon VM (small Rust binary, `target/release/operon`) | `ytdl.op` + `ytdl.cell` + bash launcher |
| python/ | Python 3.12 | CPython (`subprocess` + `ThreadPoolExecutor`) | `ytdl.py` |
| rust/ | Rust (std-only, **zero crates**) | native binary (`std::process` + scoped threads) | `main.rs` + `mini_json.rs` |
| node/ | JavaScript (ESM) | Node 24 (`execFile` + worker pool on the event loop) | `ytdl.mjs` |

**Method**: benchmarks run offline against deterministic shims
(`bench/shims/` — fixed latencies, fixed 1 MiB payloads, URL-driven failure)
so every number isolates **host + orchestration** cost, not the network.
Host: 2-core / 3.9 GiB container, Linux 5.10, x86_64. All four ports pass the
same functional smoke (doctor, info, get, meta, queue 32/32, fail-queue rc=1).
The Operon VM here is the current origin/ai/ecosystem build — it includes the
landed W011 optimizer pipeline (constant folding, jump threading, DCE,
block-local constant propagation, engine fast paths; default `--opt=1`).
Stability: the full suite was run twice end-to-end; every metric moved within
±3%, and fib(27) moved 1.6% (0.450 s → 0.443 s) — consistent with W011's own
honest micro-corpus finding (±1%; Env HashMap traffic dominates, not compute).

---

## 1. Headline answer — how much Operon is in it?

**The Operon app is 100% Operon at the application layer.**

- `ytdl.op`: 294 code lines — **every line of app logic is Operon** (genes).
- 20 lines of bash in the launcher whose only job is passing the sandbox
  grants (`--allow-read`, the operator cell, fuel). Zero Rust at app level.
- The VM underneath is Rust (26.6k lines), but it is the *platform*, shared by
  every future Operon app — the marginal Rust cost of the next app is zero.
  This is exactly the CPython relationship to a `.py` file.

**Composition of the whole benchmark suite** (code-only lines, all four ports):

| Language | Code LOC | Share |
|----------|---------:|------:|
| Rust (incl. 145-line hand-rolled JSON parser) | 529 | 44.3% |
| **Operon** | 294 | **24.6%** |
| Node/JS | 193 | 16.2% |
| Python | 178 | 14.9% |
| **Total** | 1,194 | 100% |

The 2.5x spread (Python 178 vs Rust 529) is itself a finding: in Python/Node,
JSON, threads and subprocesses are batteries; in std-only Rust the app must
carry its own JSON parser, and the borrow checker forces structure. Operon
lands in between: `run`/`json_parse`/`spawn`/`join` are builtins (like
Python), but there is no `elif`/`switch`, no tuple unpacking, no
dict-comprehension — the CLI dispatch tower costs ~40 extra lines.

## 2. Benchmark results (8 metrics)

### M3 — Cold start (hidden `startup` mode; 25 reps; mean / min / stdev, s)

| Host | mean | min | vs Rust |
|------|-----:|----:|--------:|
| Rust native | 0.0013 | 0.0012 | 1.0x |
| **Operon VM** | **0.0050** | **0.0047** | **3.8x** |
| Node 24 | 0.0288 | 0.0270 | 22.2x |
| CPython 3.12 | 0.0430 | 0.0411 | 33.1x |

The Operon VM starts in ~5 ms — that includes parsing the 352-line gene
file, registering all genes, optimizing, initializing the default-deny
capability sandbox, and executing. It beats CPython by ~8.6x and V8 by ~5.8x
on the metric that decides whether a CLI feels instant.

### M4 — Spawn overhead (`spawnbench 100`: 100 sequential `yt-dlp --version` children)

| Host | total (s) | ms/child | overhead vs best |
|------|----------:|---------:|-----------------:|
| CPython | 3.121 | 31.2 | baseline |
| Rust | 3.188 | 31.9 | +2.1% |
| Node | 3.194 | 31.9 | +2.3% |
| **Operon** | 3.414 | 34.1 | **+9.4%** |

The child's own startup (~29 ms of Python) dominates; the Operon dispatch
adds ~2.9 ms per spawn (fork/exec + capability check + env scrub + fuel
accounting + result map construction). For an orchestrator that spawns
children measured in *seconds*, a 2.9 ms tax per child is noise.

### M5 — Concurrent queue (32 shim jobs x {0.2 s, 1 MiB write}; workers 1/4/8; 3 reps)

| Host | w=1 (s) | w=4 (s) | w=8 (s) | speedup 1→8 |
|------|--------:|--------:|--------:|------------:|
| CPython | 7.557 | 2.002 | 1.137 | 6.6x |
| Rust | 7.497 | 1.981 | 1.111 | 6.7x |
| Node | 7.548 | 2.017 | 1.122 | 6.7x |
| **Operon** | **7.635** | **1.978** | **1.159** | **6.6x** |

At w=1 all four are within 1.8% — the workload is child-bound, which is the
honest description of what a downloader app does all day. At w=4 all four hit
~3.8x and at w=8 ~6.6x (the shim is sleep-dominated, so oversubscribing 2 cores
still scales). **Operon's spawn/join map-reduce model is within 2-4% of
native Rust** — its workers are real OS threads, with results crossing the
thread boundary through serialized values and channels, never shared state.

### M6 — Host process peak RSS (VmHWM during the w=8 queue)

| Host | peak RSS | vs Rust |
|------|---------:|--------:|
| Rust native | 1.64 MB | 1.0x |
| **Operon VM** | **4.73 MB** | **2.9x** |
| CPython | 15.88 MB | 9.7x |
| Node 24 | 54.54 MB | 33.3x |

The Operon VM orchestrates 8 concurrent worker threads and 32 children from
under 5 MB of RAM — 3.4x leaner than CPython and 11.5x leaner than V8, while
carrying a parser, optimizer, sandbox, fuel meter and regulation engine.

### M7 — Error handling (2-job queue against deterministically failing children)

| Host | wall (s) | exit code | behavior |
|------|---------:|----------:|----------|
| Rust | 0.033 | 1 | `Result` propagation, fast fail |
| **Operon** | **0.042** | **1** | `run` returns `ok=false`; queue exits 1 |
| Node | 0.068 | 1 | rejected promise per job |
| CPython | 0.077 | 1 | `SubprocessError` caught per job |

All four fail loudly and correctly. Operon is second-fastest on the error
path: failure is an in-VM value (`ok=false` map / catchable `missing` stress),
not an OS-level surprise. Notably, a missing binary in Operon is *not*
`ok=false` — it raises a catchable `missing` stress, because in a
default-deny world "cannot execute" is a containment event. That cost one
`stress/rescue` in `probe()` and is exactly the kind of error model
difference the app exists to expose.

### M8 — In-host compute (recursive fib(27), inside each runtime)

| Host | wall (s) | vs Rust |
|------|---------:|--------:|
| Rust native | ~0.0005 | 1x |
| Node (V8 JIT) | 0.002 | 4x |
| CPython | 0.027 | 54x |
| **Operon** | **0.443** | **~900x** |

The honest weak spot: Operon's interpreter is ~16x slower than CPython on
tight numeric recursion. The W009-A ablation attribution (BENCH.md) pins
the mechanism: a ~0.7 µs/call floor in the SHARED call funnel — per-call
regulatory gate walks (16.8%), call bookkeeping (11.6%), traceback/name
String clones (9.2%), and the structural residual of string-keyed
env-chain resolution + per-call Env allocation (fuel ticks, decay
tickers, and naive Env pooling were all REFUTED by ablation). Per-call
cost is depth-flat (700 ns/call at fib25 == 703 ns/call at fib27), so
fib27 is simply 635,621 calls x the floor. The W011 optimizer does not
move this row (measured: passes shave single-digit % of a ~7-instruction
body — the funnel dominates), so the fix ladder is W009-B (cached
clean-regulation bit + happy-path clone removal, 22.4% measured SAFE
ceiling), then slot-indexed locals, then the owner-gated W012 JIT tier.
It does not surface in M4/M5 because the app is orchestration-bound, but
it sets the VM roadmap's priority order.

### M1 — Lines of code

| Port | total | code-only | notes |
|------|------:|----------:|-------|
| Python | 224 | 178 | batteries included |
| Node | 222 | 193 | batteries included |
| **Operon** | 352 | 294 | no `switch`, dispatch tower costs ~40 lines |
| Rust | 576 | 529 | + 145 of those are the hand-rolled JSON parser |

### M2 — What you actually ship

| Port | App artifact | Runtime needed | Total to ship |
|------|-------------|----------------|--------------:|
| **Rust** | 568 KB standalone binary | none | **568 KB** |
| **Operon** | 9.9 KB `ytdl.op` | 2.5 MB VM binary (ship once, share across all Operon apps) | **2.53 MB** |
| Python | 6.7 KB `ytdl.py` | system CPython (or ~10-15 MB bundled) | ~10-15 MB bundled |
| Node | 7.0 KB `ytdl.mjs` | system Node (or ~100 MB bundled) | ~100 MB bundled |

The lightweight requirement is comfortably met: the Operon port ships 2.53 MB
total — no Deno, no aria2, no bundling of yt-dlp/ffmpeg — and the Rust port
proves the whole app fits in 568 KB.

## 3. How the Operon app works differently from the Python and Rust ones

**Execution model.** The Python app *is* the process — CPython interprets
`ytdl.py` directly, and whatever the script can express, the process can do:
full parent environment, any path, any exit, no gatekeeper. The Rust app
*is* the machine — compiled to native code, the OS trusts it completely.
The Operon app is neither: it is guest code in a 2.5 MB VM that owns every
side effect. The app declared "I want to run yt-dlp/ffmpeg/ffprobe" and
nothing else happens without an operator cell granting it.

**Concurrency shape.** Three different philosophies of parallelism:
- Python: `ThreadPoolExecutor` with a shared jobs list and a per-future
  result loop — mutable shared state, made safe by the GIL.
- Rust: scoped threads over *contiguous chunk partitions* — no shared
  mutation because the borrow checker forbids it; results returned per chunk.
- Operon: `spawn(worker, [chunk])` / `join(id)` — every worker is a real OS
  thread, but it can *only* receive serialized values and return one result.
  Shared mutable state is not dangerous; it is *impossible*. The map-reduce
  shape is forced by the language, which is why the Operon port has no
  mutex/no lock/no race story to tell at all.

**Environment and containment.** The Python and Rust ports pass the full
parent environment to every child by default — a `SHIM_FAIL=1` knob "just
works" (and so would a leaked `AWS_SECRET_ACCESS_KEY`). The Operon VM scrubs
child environments to OS essentials; our first error-handling benchmark
silently *did not fail* in Operon because `SHIM_FAIL` never reached the
child — the sandbox ate it, exactly as designed (secrets cannot leak to
effects). The benchmark was fixed by moving the failure signal into the URL.
That incident is the single most Operon thing in this report: the language's
default behavior caught a cross-port experiment design flaw.

**Progress semantics.** Python/Rust/Node stream child stdout as it exits;
Operon's `run()` captures output until process end (with a 64 MiB cap and a
wall-clock timeout charged as fuel), so job lines appear at `join` time. For
real yt-dlp runs the other ports show incremental progress; the Operon port
currently reports completion-state per job. Fixing this means adding a
streaming `run` variant to the VM — filed as follow-up work.

**Error models.** Python: exceptions + tracebacks (richest message, slowest
path here). Rust: `Result` through explicit plumbing (fast, compile-checked).
Node: rejected promises (fast, silent-unless-awaited risk). Operon: `run`
returns `{code, stdout, stderr, ok}` for process outcomes but raises
catchable *stresses* (`missing`, `interference`, `overflow`) for containment
events — failures that in the other ports would be indistinguishable from
ordinary errors are *typed by cause* in Operon.

**Build loop.** Python/Node/Operon: edit → run (0 s). Rust: edit → `cargo
build` (3.7 s on this box) — and the compiler caught 3 bugs (integer/usize
mismatch, closure move, slice lifetime) before the app ever ran, while the
interpreted ports shipped their bugs (float job index in Operon, shadowed
PATH in the launcher) to runtime.

## 4. Ratings for this specific app (subprocess-orchestration CLI)

Scored 1-10 per dimension for THIS workload, from the measured numbers above.

| Dimension | Operon | Python | Rust | Node |
|-----------|-------:|-------:|-----:|-----:|
| Cold start | 8.5 | 5.5 | 10 | 6 |
| Host memory | 8.5 | 6 | 10 | 4 |
| Spawn dispatch overhead | 9 | 9.5 | 9.5 | 9.5 |
| Queue throughput (child-bound) | 9.5 | 9.5 | 10 | 9.5 |
| In-host compute | 3 | 8 | 10 | 9.5 |
| LOC / expressiveness | 7 | 9 | 5 | 9 |
| Error-path speed & clarity | 9 | 6.5 | 10 | 7 |
| Safety / containment by default | **10** | 3 | 8 | 4 |
| Distribution artifact | 8.5 | 6 | 10 | 5 |
| Dev loop speed | 8 | 9 | 6 | 9 |
| **Total (of 100)** | **81** | **72** | **88.5** | **72.5** |

**Reading the ratings.** For this specific app:

- **Rust wins overall (88.5)** — and it should: the app *is* the artifact,
  with native speed, the smallest footprint, and compile-time guarantees.
  Its costs are developer-visible: 44.3% of the suite's code for 25% of the
  ports, a hand-rolled JSON parser, and the slowest dev loop.
- **Operon is second (81)** — remarkable for a pre-1.0 language. It wins the
  dimension nobody else contests (containment: 10/10), is within 2-4% of Rust
  on orchestration throughput with 3.4x less RSS than Python, starts ~8.6x
  faster than Python, and ships a 2.5 MB artifact. Its compute tier (3/10)
  is the documented debt (W011 stage-2 locals-in-frame, then W012 JIT).
- **Python and Node tie (~72)** — the fastest to write, the slowest to
  start, the fattest to ship, and both hand every child the full parent
  environment by default. For a one-off script that is fine; for a
  distribution-grade orchestrator it is the weak flank.

## 5. Language-specific findings the app caught (verified behaviors)

1. Operon: a *missing binary* raises catchable `missing` stress — `run` never
   returns `ok=false` for "could not execute" (containment is typed).
2. Operon: `/` is always float division (Python semantics) — `jobs[k]` with a
   float index fails loudly; integer chunk math needs `//`.
3. Operon: unknown `--flags` before the POSIX `--` separator die loudly at the
   host CLI (typo armor) — program flags must follow `--`.
4. Operon: child environments are scrubbed to essentials; env-dependent test
   hooks must be granted (`--allow-env`) or, better, encoded in the payload.
5. All ports: `ffmpeg`/`ffprobe` require `-version` (not `--version`, exit 8)
   — a cross-language footnote that cost the same one-line fix four times.

## 6. Threats to validity

- The queue benchmark is child-bound by design; on a network-bound real
  download all four hosts would converge to identical wall times, and
  M4/M5's host differences would matter even less.
- M6 measures the host process (VmHWM), excluding children — the honest
  comparison of *runtime* cost; a "total tree RSS" metric would favor Rust
  and Operon equally (both keep children lean) and is harder to sample portably.
- The box has 2 cores / 3.9 GiB; absolute numbers are container-relative,
  but host-to-host ratios are stable across the 3 reps (stdev ≤ 4%).
- fib(27) is a worst-case micro-benchmark for an interpreter (deep recursion,
  pure arithmetic); string/collection workloads are far kinder to Operon.
- Operon totals use the tree-walk + W011-optimized engine as shipped
  (default `--opt=1`); `--opt=0` and `--opt=2` were verified output-identical
  by the optimizer's own parity gates (1308/1308 byte-identical).

## 7. Reproduce

```bash
cargo build --release                                   # Operon VM
cargo build --release --manifest-path apps/ytdl-bench/rust    # Rust port
apps/ytdl-bench/bench/bench.sh                                # full suite (offline)
apps/ytdl-bench/ytdl doctor                                   # real tools smoke
```

Results land in `apps/ytdl-bench/bench/results/` (`summary.tsv`, `m1_loc.csv`,
`m2_artifact.csv`, `run.log`).
