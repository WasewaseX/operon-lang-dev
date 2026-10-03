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
| Rust native | 0.0014 | 0.0013 | 1.0x |
| **Operon VM** | **0.0047** | **0.0044** | **3.4x** |
| Node 24 | 0.0285 | 0.0262 | 20.4x |
| CPython 3.12 | 0.0431 | 0.0418 | 30.8x |

The Operon VM starts in ~4.7 ms — that includes parsing the 352-line gene
file, registering all genes, initializing the default-deny capability
sandbox, and executing. It beats CPython by 9x and V8 by 6x on the metric
that decides whether a CLI feels instant.

### M4 — Spawn overhead (`spawnbench 100`: 100 sequential `yt-dlp --version` children)

| Host | total (s) | ms/child | overhead vs best |
|------|----------:|---------:|-----------------:|
| CPython | 3.090 | 30.9 | baseline |
| Rust | 3.101 | 31.0 | +0.4% |
| Node | 3.133 | 31.3 | +1.4% |
| **Operon** | 3.345 | 33.5 | **+8.3%** |

The child's own startup (~29 ms of Python) dominates; the Operon dispatch
adds ~2.5 ms per spawn (fork/exec + capability check + env scrub + fuel
accounting + result map construction). For an orchestrator that spawns
children measured in *seconds*, a 2.5 ms tax per child is noise.

### M5 — Concurrent queue (32 shim jobs x {0.2 s, 1 MiB write}; workers 1/4/8; 3 reps)

| Host | w=1 (s) | w=4 (s) | w=8 (s) | speedup 1→8 |
|------|--------:|--------:|--------:|------------:|
| CPython | 7.528 | 2.003 | 1.127 | 6.7x |
| Rust | 7.473 | 1.964 | 1.124 | 6.6x |
| Node | 7.576 | 1.991 | 1.114 | 6.8x |
| **Operon** | **7.577** | **1.999** | **1.176** | **6.4x** |

At w=1 all four are within 1.4% — the workload is child-bound, which is the
honest description of what a downloader app does all day. At w=4 all four hit
~4x and at w=8 ~6.7x (the shim is sleep-dominated, so oversubscribing 2 cores
still scales). **Operon's spawn/join map-reduce model is within 2-4% of
native Rust** — its workers are real OS threads, with results crossing the
thread boundary through serialized values and channels, never shared state.

### M6 — Host process peak RSS (VmHWM during the w=8 queue)

| Host | peak RSS | vs Rust |
|------|---------:|--------:|
| Rust native | 1.45 MB | 1.0x |
| **Operon VM** | **4.64 MB** | **3.2x** |
| CPython | 15.81 MB | 10.9x |
| Node 24 | 54.62 MB | 37.7x |

The Operon VM orchestrates 8 concurrent worker threads and 32 children from
under 5 MB of RAM — 3.4x leaner than CPython and 11.8x leaner than V8, while
carrying a parser, sandbox, fuel meter and regulation engine.

### M7 — Error handling (2-job queue against deterministically failing children)

| Host | wall (s) | exit code | behavior |
|------|---------:|----------:|----------|
| Rust | 0.034 | 1 | `Result` propagation, fast fail |
| **Operon** | **0.041** | **1** | `run` returns `ok=false`; queue exits 1 |
| Node | 0.070 | 1 | rejected promise per job |
| CPython | 0.078 | 1 | `SubprocessError` caught per job |

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
| **Operon** | **0.450** | **~900x** |

The honest weak spot: Operon's tree-walking interpreter is ~17x slower than
CPython on tight numeric recursion (fuel checks on every step, checked i64
arithmetic, `Rc<RefCell>` value model, no bytecode tier yet). It does not
surface in M4/M5 because the app is orchestration-bound, but it sets the
priority for the VM roadmap (bytecode + constant folding + jump threading +
DCE — W011 — and later a JIT tier).

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
  dimension nobody else contests (containment: 10/10), is within 8% of Rust
  on orchestration throughput with 3.4x less RSS than Python, starts 9x
  faster than Python, and ships a 2.5 MB artifact. Its compute tier (3/10)
  is the documented debt, already queued (W011 optimizer, bytecode).
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

## 7. Reproduce

```bash
cargo build --release                                   # Operon VM
cargo build --release --manifest-path apps/ytdl/rust    # Rust port
apps/ytdl/bench/bench.sh                                # full suite (offline)
apps/ytdl/ytdl doctor                                   # real tools smoke
```

Results land in `apps/ytdl/bench/results/` (`summary.tsv`, `m1_loc.csv`,
`m2_artifact.csv`, `run.log`).
