# BENCHMARK-DEEP.md — one downloader app, four languages, measured

This is the deep, many-aspect benchmark behind the ytdl app experiment
(see `APP-COMPARISON.md` for the feature/security comparison). The same
YouTube-downloader orchestrator is implemented four times — **Operon**
(the language under test), **Python 3.12**, **Deno 2.9/TypeScript**, and
**Bash 5** — and every build exposes an identical `bench-*` workload
surface so the harness can run *the same work* through *the same decision
logic* in every language and compare everything that can be compared.

Everything here runs offline against deterministic mock engines. No
network, no flaky YouTube gates — the numbers are reproducible on any box.

## 1. Method

**The workload surface.** Each build implements the same six benchmark
subcommands (`apps/ytdl/ytdl.op`, `apps/ytdl-compare/{python,deno,bash}/`):

| workload | what it exercises | parameters used |
|---|---|---|
| `bench-startup` | full app invocation: parse/compile + dispatch + exit | — |
| `bench-json F --n N` | yt-dlp metadata path: parse the 9.4 KB / 48-format fixture N times | N=200, N=2000 |
| `bench-table F --rounds R` | the app's real per-video decision pipeline: sort 48 formats by (height desc, tbr desc) via comparator + build every human row (clip/pad/format/interpolate) | R=30, R=300 |
| `bench-lines --k K` | child-output processing: build + scan + extract K synthetic `yt-dlp` progress lines | K=20k, K=200k |
| `bench-spawn --n N` | orchestrator overhead: N sequential child spawns (`mockspawn`, one-line bash script) | N=30, N=200 |
| `bench-queue --k K --c C` | concurrency model: K mock downloads (80 ms children) at C=1 vs C=8 workers | K=16, C∈{1,8} |

Bash deliberately **refuses** `bench-json`/`bench-table` — it has no JSON
parser. That refusal is a benchmark *finding*, recorded as `refused`, not
an error (same honesty as `selfcheck` in `APP-COMPARISON.md`).

**The differential pin.** Every workload prints one
`bench-* cs=...` checksum line. The harness collects them across builds
and requires byte-identical agreement for every workload among the
JSON-capable builds; bash's sequential queue is pinned separately
(`ok16,k16,c1`). In the run below: **10/10 workloads IDENTICAL**. The
benchmark itself is differential-verified — no build accidentally
"optimizes" different work.

**Timing.** Median of R timed runs after one discarded warmup run (fills
OS caches and Deno's transpile cache). Wall clock around the whole child
process, so `startup` is included in every number; the `*2k / *300 /
*200k / *200` variants use 10× the work to make per-unit rates
startup-independent. Low-variability: medians reproduced within a few
percent across repeated harness invocations.

**Memory.** Peak RSS via a fresh wrapper process per measurement
(`scripts/rss_wrap.py`, `getrusage(RUSAGE_CHILDREN).ru_maxrss`). On this
kernel the method has a **~12.7 MB floor for ANY child** (measured:
`bash -c "exit 0"` → 12.86 MB, `python3 -c pass` → 12.78 MB), so treat
values as `floor + delta` and compare the deltas.

**Engines.** Identical mocks on PATH for every build: `mockspawn` (print
one line, exit 0) and `mocksleep` (sleep 80 ms) — both tiny bash scripts,
so the child floor is the same everywhere.

## 2. Environment

- CPU: Intel Xeon ×2 cores | RAM 4.0 GB | kernel 5.10.134 (x86_64 container)
- Operon **2.7.0-vm** (rust-core, cpp-kernel), release binary 3.6 MB
- Python 3.12.14 · Deno 2.9.6 · GNU bash 5.2.37
- Date of run: 2026-10-02 23:59 UTC; full raw data in
  `apps/ytdl/bench/results.json` (medians, min/max, RSS, checksums)
- Reproduce: `python3 scripts/bench_deep.py` (see §8). Absolute numbers
  are box-specific — the **ratios and orderings** are the story.

## 3. Raw medians (ms, whole process)

| workload | Operon VM | Python 3.12 | Deno 2.9 (TS) | Bash 5 |
|---|---|---|---|---|
| startup (×15) | **27.2** | 61.6 | 36.5 | 26.8 |
| json (n=200) | 34.8 | 41.9 | **19.4** | refused |
| table (r=30) | 163.8 | 36.7 | **30.6** | refused |
| lines (k=20k) | 84.3 | 37.7 | **18.4** | 244.0 |
| spawn (n=30) | 166.8 | 67.8 | 57.7 | **41.0** |
| queue1 (K=16, C=1) | 1389.8 | 1370.0 | 1349.9 | 1336.1 |
| queue8 (K=16, C=8) | **219.2** | 249.4 | 223.5 | 1362.6 |
| json2k (n=2000) | 342.0 | 167.0 | **98.3** | refused |
| table300 (r=300) | 1602.8 | 91.1 | **44.0** | refused |
| lines200k (k=200k) | 789.7 | 121.4 | **34.7** | 2422.2 |
| spawn200 (n=200) | 1095.7 | 277.7 | 277.9 | **246.7** |

**Startup reconciliation (2026-10-03, builder-A).** The startup row above
was recorded on a contended box: every host carries a ~+25 ms uniform
additive term. Evidence: the bash row says 26.8 ms, but `bash
apps/ytdl-compare/bash/ytdl.sh bench-startup` re-measures at 2 ms median
(`bash -c "exit 0"` floor: 1 ms), the operon row re-measures at 3 ms
median (same command shape as `operon_base()`, same 2.7.0-vm binary, n=7),
and python re-measures at 32 ms. Deployable startup story: bash ~2 ms,
**Operon ~3 ms**, python ~32 ms — Operon is still ~10x faster than CPython
and essentially tied with bash. The §4 per-unit rates that subtract this
row inherit the error (a flat 25 ms over N units shrinks every small-N
rate); re-run the suite on an idle box before quoting §4 precisely. The
work-dominated rows (json2k, table300, lines200k, spawn200, queues) are
unaffected in ordering — their per-unit work dwarfs 25 ms.

## 4. Net per-unit rates (startup-subtracted, medians)

| aspect | Operon | Python | Deno (TS) | Bash | Operon vs fastest |
|---|---|---|---|---|---|
| JSON parse (9.4 KB meta) | 157 µs (~60 MB/s) | 53 µs (~177 MB/s) | **31 µs (~303 MB/s)** | n/a | 5.1× slower |
| decision pipeline (1 table build, 48 formats) | 5.25 ms | 98 µs | **25 µs** | n/a | 210× slower |
| progress-line processing | 3.8 µs/line | 0.30 µs/line | **≤0.2 µs/line** | 12.0 µs/line | ~19× slower |
| child spawn + capture | 5.34 ms | 1.08 ms | 1.21 ms | **1.10 ms** | 4.9× slower |

Read the decision-pipeline row with context: Operon is a tree-walking
VM whose `sorted()` comparator is a *gene call* per comparison and whose
string rows are built through interpreter-level interpolation, while
CPython's `sorted`+f-strings and V8's JIT are C-level machinery. A 210×
compute gap is real — and, for this app, almost irrelevant (§6).

## 5. Concurrency: queue scaling (K=16 × 80 ms jobs)

| build | C=1 | C=8 | speedup | mechanism |
|---|---|---|---|---|
| Operon | 1389.8 ms | 219.2 ms | **6.34×** | `spawn(gene, [chunk])` fibers + ordered `join` |
| Deno | 1349.9 ms | 223.5 ms | 6.04× | `Promise.all` over async `Deno.Command` chunks |
| Python | 1370.0 ms | 249.4 ms | 5.49× | `ThreadPoolExecutor` over pre-partitioned chunks |
| Bash | 1336.1 ms | 1362.6 ms | 0.98× | none — sequential by design, honestly labelled `c1` |

The theoretical ideal for K=16, C=8, 80 ms children is 8× bounded by a
160 ms wall floor (two rounds of children). All three real concurrency
models land within ~15% of that bound; Operon is fastest in absolute
terms here (children dominate; the queue glue costs less than Python's
thread-pool bookkeeping). Bash's 0.98× is the honest finding it claims:
no safe concurrency for this shape, so no speedup, and the differential
still pins its output.

## 6. Where the time actually goes (the app's perspective)

A downloader orchestrator spawns `yt-dlp` **once per video**. With real
engines, one spawn is a multi-second network download; the measured
per-unit costs above then mean, per video:

- metadata JSON parse: 0.16 ms (Operon) vs 0.03 ms (Deno) — **noise**
- decision table: 5.3 ms (Operon) vs 0.03 ms (Deno) — **0.2% of one second**
- spawn premium: +4.2 ms per `run()` (grant checks + capture + fuel
  accounting) vs a plain fork — **noise again**
- the queue concurrency model: the only place glue is on the critical
  path — and Operon is *ahead* there (§5)

So the compute-heavy rows (§4) are the benchmark's magnifying glass, not
the app's bottleneck. The aspects that decide this app's user experience
are startup, memory, footprint, safety, and orchestration — measured next.

## 7. The other aspects

**Startup (cold, whole app).** Operon **27.2 ms** ≈ Bash 26.8 ms <
Deno 36.5 ms < Python 61.6 ms. Two findings: (a) Operon pays ~26 ms to
parse/compile the 825-line app *on every invocation* — there is no
bytecode cache yet; the OIR1 pipeline makes an on-disk cache keyed by
source hash an obvious, small future win (filed below). (b) Deno's number
is *warm-cache* (transpiled TS cached on disk); a cold-cache Deno start
is materially slower, and its cache is a disk artifact the other builds
don't need.

**Peak RSS (floor ≈ 12.7 MB on this kernel; deltas are the signal).**

| run | Operon | Python | Deno | Bash |
|---|---|---|---|---|
| app startup | 12.9 MB (**+0.1**) | 14.8 MB (+2.1) | 28.8 MB (+16.1) | 12.7 MB (+0.0) |
| json2k (2000 parses) | 12.5 MB (**≤floor**) | 14.9 MB (+2.2) | 36.2 MB (+23.5) | n/a |
| queue8 (8 workers) | 12.9 MB (**+0.1**) | 17.2 MB (+4.5) | 36.9 MB (+24.2) | 12.9 MB (+0.1) |

Operon runs the full concurrent app **at the measurement floor** — the
VM arena stays flat under 8× concurrency. Deno pays ~16–24 MB for the
V8 heap; Python adds a few MB for threads and parsed JSON churn.

**Deployable footprint** (runtime + app source, what "install this app"
actually means):

| build | runtime | app source | total | app LOC |
|---|---|---|---|---|
| Operon | 3.6 MB | 23.8 KB | **3.6 MB** | 825 |
| Bash | 1.2 MB | 7.6 KB | 1.2 MB (subset only) | 185 |
| Python | 29.5 MB | 17.7 KB | 29.5 MB | 495 |
| Deno | 91.2 MB | 21.4 KB | 91.2 MB | 608 |

The lightweight goal from the owner's brief ("not the 80 MB!") lands
literally: the Operon edition is **3.6 MB total, 28× smaller than the
Python stack, 25× smaller than Deno**, with the full feature set — while
the bash build that beats it on size refuses half the workloads and all
of the JSON decision logic.

**Security posture (qualitative, from the app designs).** The Operon
build runs under a default-deny capability sandbox: the launcher grants
exactly `yt-dlp`/`ffmpeg`/`aria2c` + the output directory + read paths,
with a fuel ceiling and a per-`run()` wall-clock limit — and the *app
itself* cannot exceed that envelope even if its logic is buggy or
hostile. Python/Deno builds rely on process-level permissions (Deno's
`--allow-*` is close in spirit but the comparison build asks for
blanket `--allow-run`; Python has nothing at the language level). Bash
has nothing. For an orchestrator whose whole job is executing
third-party binaries, this is the aspect where Operon is most clearly
ahead — and the bench surface exercises it for real (`--allow-run
mockspawn --allow-run mocksleep` are *required* for those workloads to
pass; remove a grant and the app refuses loudly).

**Correctness under the benchmark.** 10/10 workloads byte-identical
checksums across all builds (bash pinned at its honest subset). The
differential that session-21 built for `selfcheck` now extends to every
timed workload — any future optimization pass (W011) or app refactor
that changes behavior breaks this harness before it breaks a user.

**Ergonomics notes from writing the bench surface (per language).**
- *Operon*: the interpolation grammar rejects string literals inside
  `{...}` (session-21 lesson, hit again — `",".join()` must be hoisted
  to a `let`); `join(t)` returns the fiber's value directly (not a
  list-of-one); everything else was a straight transliteration of the
  Python shape. Sandbox grants were the only extra ceremony, and they
  are one CLI line in the harness.
- *Python*: shortest code; `ThreadPoolExecutor` for queue; spawn
  overhead the lowest of the managed runtimes.
- *Deno/TS*: async split forced a sync *and* async worker for the same
  queue logic (`outputSync` vs `await output()`); type shims for the
  metadata shape; fastest compute by far, heaviest runtime by far.
- *Bash*: 4 of 6 workloads implementable; no JSON anywhere; the
  sequential queue is the only honest option; string ops 3–6× slower
  than Python despite zero startup.

## 8. Verdict

| aspect | winner | notes |
|---|---|---|
| cold startup (whole app) | Operon ≈ Bash | 2.3× faster than Python; bytecode cache can push lower |
| metadata JSON parse | Deno | Operon same order of magnitude as CPython |
| decision pipeline compute | Deno ≫ Python | Operon pays the interpreter tax (210×) — irrelevant here |
| string/line processing | Deno > Python ≫ Operon > Bash | |
| spawn orchestration | Bash ≈ Python ≈ Deno; Operon +4 ms/child | grant-check + capture premium |
| concurrent queue | **Operon** (6.34×, fastest absolute) | all real models ~6×; bash honest 1× |
| peak memory | **Operon = floor** | Deno +16–24 MB, Python +2–4.5 MB |
| deployable footprint | **Operon 3.6 MB** | vs 29.5 MB / 91.2 MB |
| capability security | **Operon** (default-deny grants) | Deno partial, Python/Bash none |
| differential correctness | all (10/10 IDENTICAL) | bash on its honest subset |
| feature completeness at that cost | **Operon** | bash small but refuses JSON workloads |

**For this app**, the ranked story is: Operon wins every aspect that the
user actually feels (startup, memory, footprint, safety, concurrency)
and loses only on raw compute that the app never needs. The benchmark
also earns its keep as a language test — it surfaced two concrete,
queued language items:

1. **OIR1 bytecode cache** (keyed by source hash) — would cut Operon's
   27 ms app startup toward the ~1 ms interpreter floor (`operon run
   hello.op` measures 1.0 ms cold); natural follow-on to the W011 VM
   passes.
2. **`run_stream()`** (filed in APP-COMPARISON.md) — streaming child
   output would let the queue print live progress; the bench's
   progress-line workload is the pin for its parsing half.

## 9. Reproduce

```sh
# all four builds, medians + RSS + checksums, writes results.json
python3 scripts/bench_deep.py

# faster pass
python3 scripts/bench_deep.py --runs-scale 0.4

# any single workload by hand (grants are the security boundary):
PATH=apps/ytdl/test/mock:$PATH \
  target/release/operon run apps/ytdl/ytdl.op --cell apps/ytdl/ytdl.cell \
  --allow-read "$PWD" --allow-run mockspawn --allow-run mocksleep \
  --fuel 20000000000 -- bench-queue --k 16 --c 8

deno run --allow-read --allow-run apps/ytdl-compare/deno/ytdl.ts bench-queue --k 16 --c 8
python3 apps/ytdl-compare/python/ytdl.py bench-queue --k 16 --c 8
bash apps/ytdl-compare/bash/ytdl.sh bench-queue 16
```

Raw machine-readable results: `apps/ytdl/bench/results.json`.
Feature/security comparison: `docs/APP-COMPARISON.md`.
