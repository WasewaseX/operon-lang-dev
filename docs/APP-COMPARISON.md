# APP-COMPARISON.md — one downloader app, four languages

**ytdl** is the same YouTube-downloader orchestrator written four times:
in **Operon** (the language under test), **Python 3**, **Deno/TypeScript**,
and **Bash**. The engines are identical everywhere — `yt-dlp` and `ffmpeg`
from PATH, `aria2c` when present — so every difference below is the
*language and runtime*, not the tooling.

This experiment exists to test Operon the honest way: by shipping a real
application and letting a real workload expose real gaps.

> **Measured follow-up:** the same four builds now carry a uniform
> `bench-*` workload surface and have been benchmarked across 10+
> aspects (startup, JSON parse, decision pipeline, string processing,
> spawn overhead, queue concurrency scaling, peak RSS, footprint, LOC,
> differential checksums). Numbers, analysis and verdict:
> **`docs/BENCHMARK-DEEP.md`** (harness: `scripts/bench_deep.py`,
> raw data: `apps/ytdl/bench/results.json`).

## What the app does (identical surface in every build)

- `doctor` — probe PATH engines, report versions, verdict
- `info URL` — `yt-dlp -J` metadata → format table (sort by height then bitrate, human sizes, comma-grouped views)
- `get URL` — download with `--quality TIER | --format EXPR | --audio KIND`, optional subs/aria2c, checkpoint-resume
- `queue FILE` — N concurrent workers over a URL list, deterministic stable output
- `selfcheck` — differential pin: a decision matrix (9 quality tiers × 5 audio kinds), a sort-order check, and two header lines that every JSON-capable build must print **byte-identically**

## The differential (the actual test)

`selfcheck` output from the Operon build and the Python build is
`diff`-verified byte-identical over the full matrix, the format sort order,
and the rendered header lines — same selector expressions, same ordering,
same human formatting, computed independently in two languages with two
runtimes. `apps/ytdl/test/run_tests.sh` runs this on every invocation
(currently **operon == python: identical**; the Deno build prints the same
matrix and runs wherever Deno is installed).

Bash deliberately ships a subset: it has no JSON parser, so it refuses
`selfcheck` with a clear error instead of faking it, and `info` uses
`yt-dlp --print` field extraction instead.

## Numbers (this machine, median of 10–20 runs, mock engines)

| runtime | size (MB) | cold start (ms) | doctor ×10 (ms) | selfcheck ×10 (ms) | app LOC |
|---|---|---|---|---|---|
| **Operon** | **3.6** | **1.0** | 19.3 | **4.4** | 747 |
| Python 3 | 29.5† | 15.0 | 35.0 | 34.6 | 472 |
| Deno | n/a (not installed here; upstream ~80–100 MB) | n/a | n/a | n/a | 500 |
| Bash | 1.2‡ | 0.7 | 8.5 | n/a (no JSON) | 163 |

† python3 binary only — a working python needs its stdlib tree on top.
‡ bash alone is not a JSON-capable runtime; its build is a deliberate subset.

Reading the table:

- **Ship size**: the Operon story is "one 3.6 MB binary + your PATH".
  There is no bundle step at all — the same binary runs every `.op`
  program. A bundled-Deno alternative was the original plan and it starts
  at ~80 MB before the app ships a single feature; this is why the
  lightweight edition exists.
- **Cold start**: 1.0 ms — an interactive CLI wrapper never makes you wait.
- **Orchestration cost**: the decision matrix (parse JSON, sort 10 formats
  by two keys, render 14 decision lines + 2 header lines) runs in 4.4 ms
  vs Python's 34.6 ms on identical input and identical output.
- **LOC honesty**: Operon uses more lines (747 vs 472) — explicit null
  guards (`has` + null checks, no silent optionals), no comprehension
  sugar, and the capability-grant boilerplate is in the launcher rather
  than hidden in magic. Density is the price of Total Grammar's
  no-surprise semantics; every line is boring on purpose.

## What the app caught in the language (the point of the exercise)

1. **`--` passthrough was broken (fixed here).** The host CLI kept
   matching its own flags *after* the POSIX `--` separator: a program
   invocation `operon run app.op -- get URL --out D` silently lost
   `--out D` to the host's build-output flag. Real-app arguments are the
   only workload that reliably exercises this; unit corpus never passed
   `--`+host-colliding names. Fixed in `src/main.rs` (passthrough now
   short-circuits every host arm), pinned by `tests/dx_passthrough.rs`.
2. **`.cell` repeated keys are last-wins.** `parse_cell` builds a
   `HashMap`, so two `allow.run` lines leave only the second grant. The
   launcher therefore passes repeatable `--allow-run` flags (which
   correctly *append*) and reserves the cell for `run.timeout_ms` +
   `allow.exit`. Documented; a multi-value grant syntax is a future item.
3. **`run()` has a hard 5-minute wall-clock ceiling** (containment by
   design). A big download gets killed mid-transfer and — notably —
   surfaces as *exit code 0* with a `.part` file. The app turns this into
   a feature: a **checkpoint-resume loop** that re-invokes `yt-dlp
   --continue` while a partial file exists, bounded by `--max-attempts`.
   The stall test in the battery simulates the kill deterministically.
4. **No streaming child output.** `run()` buffers stdout/stderr (64 MiB
   cap), so live progress bars are impossible today; the app prints
   destination/result lines after completion. A `run_stream()` builtin is
   the obvious future language item — filed, not faked.
5. **Interpolation format specs bind to simple expressions.**
   `{(f / N):.2}` does not apply the spec; binding to a variable first
   (`let g = f / N; "{g:.2}"`) does. Worked around in-app; worth a parser
   fix later.

## Feature matrix

| feature | Operon | Python | Deno | Bash |
|---|---|---|---|---|
| doctor probe | yes | yes | yes | yes |
| info (JSON metadata → table) | yes | yes | yes | `--print` fields only |
| quality tiers / raw format expr | yes | yes | yes | yes |
| audio extract (`-x`) | yes | yes | yes | yes |
| subtitles + srt convert | yes | yes | yes | passthrough |
| aria2c 16-connection accel | auto | auto | auto | not wired |
| checkpoint-resume loop | yes | yes | yes | retries only |
| concurrent queue (N workers) | yes (spawn/join OS threads) | yes (ThreadPool) | yes (event loop) | sequential only |
| deterministic output order | yes | yes | yes | yes |
| capability sandbox (default-deny) | **yes — grants in launcher** | no | no (opt-in flags exist) | no |
| differential selfcheck | yes | byte-identical | same matrix (runs where deno exists) | refuses honestly |

## Security posture (Operon build only)

The launcher (`ytdl.sh` / `ytdl.bat`) is the security boundary. The
program runs under Operon's default-deny capability system with exactly:
`allow.run yt-dlp/ffmpeg/aria2c`, `allow.read` (cwd + out dir + app dir),
`allow.write` (out dir), `allow.exit`, `run.timeout_ms 300000`,
fuel-pool headroom for long children. A queue file or output path outside
those grants is a catchable `interference` stress, not a surprise. The
Python/Deno/Bash builds run with the host's full authority — there is no
in-language way to narrow them.

## Reproduce

```sh
cargo build --release
bash apps/ytdl/test/run_tests.sh          # 35-check battery + differential
BIN=./target/release/operon bash scripts/vm_parity.sh
BIN=./target/release/operon bash scripts/redteam.sh
N=20 bash scripts/bench_ytdl.sh           # regenerate the numbers above
```

Real-network smoke: `yt-dlp` metadata on datacenter IPs is typically
bot-gated (YouTube returns null/429), so the deterministic mock battery is
the CI gate; on a residential connection the same command path runs
unchanged — `apps/ytdl/ytdl.sh doctor` then `apps/ytdl/ytdl.sh get URL`.
