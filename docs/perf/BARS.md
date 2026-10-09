# BARS — the R0.8 standard Bar A/B/C report (docs/perf/)

Task-ID R08-BARABC (L-039, sz lane; roadmap §10 R0.8 "Standardized Bar
A/B/C benchmarks and scaling tests", §34-APPROVED; team-note reassignment
from builder-E's lane, veto-at-review). This file is the STANDARD bar
structure digest-17 specified: per workload, **Bar A** = the previous
RELEASE binary, **Bar B** = CPython on the same machine, **Bar C** = a
native Rust floor — plus **scaling** (t(2N)/t(N)) and a reproducibility
half (`scripts/bench/bars_check.py`, the pointer audit that re-derives
every number on this page from the saved `bars.json`).

BENCH.md stays the full methodology + history file; this page is the
standing three-bar snapshot. The measurement half is
`python3 scripts/bench/bars.py`.

## This baseline (2026-10-09)

| pin | value |
|---|---|
| HEAD pin | main @ eb005ab (the JSON pin; the measured HEAD binary was built from f640787 — eb005ab is README-only, zero engine/fixture delta; the binary sha256 below is the provenance anchor) |
| HEAD binary | Operon 2.10.0-vm (rust-core, cpp-kernel), sha256 `37949c947ce725558dd3b2894ff86b6ebefbc956a375c04709cb215c39ecdcc7` |
| Bar A | release v2.10.0, asset `operon-2.10.0-x86_64-unknown-linux-gnu.tar.gz`, tarball sha256 `7d10356c4c4dbfafa324f384e3cac41bd206d8be2c0f0d6867374dc1a7d24efc` — verified against the release's own SHA256SUMS (the F-REL2100-POSTTAG chain) |
| Bar A binary sha256 | `d4e96ce5dd4af115e38bcbe747d1a4d8a4d733ef77fa453d7fa803021fa55f04` |
| Bar B | CPython 3.12.14 (the `bench_compare.py` native-py mirrors, IMPORTED — never reimplemented) |
| Bar C | `rustc -O` single-file twins, Rust 1.99.0 |
| host | x86-64 Linux sandbox, min over 5 runs, 1 warmup, end-to-end process wall (the bench_compare timing law) |

## Bars per workload (ms, end-to-end incl. startup)

| workload | HEAD ms | Bar A ms | Bar B ms | Bar C ms | op/py | HEAD/A |
|---|---|---|---|---|---|---|
| fib25 | 103.0 | 102.4 | 12.9 | 0.7 | 8.0x | 1.01 |
| loops | 55.2 | 54.9 | 12.1 | 0.7 | 4.6x | 1.00 |
| strings | 30.0 | 28.8 | 0.7 | 0.7 | 40.8x | 1.04 |
| collections | 31.7 | 31.1 | 5.2 | 2.5 | 6.1x | 1.02 |
| recursion | 203.6 | 205.9 | 23.9 | 0.8 | 8.5x | 0.99 |
| grn | 35.3 | 34.9 | 8.7 | 0.5 | 4.1x | 1.01 |
| json | 73.2 | 73.9 | 35.6 | refused | 2.1x | 0.99 |
| regex | 79.7 | 79.5 | 23.8 | refused | 3.3x | 1.00 |
| seq | 32.0 | 31.6 | 2.8 | 0.5 | 11.3x | 1.02 |
| large_map | 7.4 | 7.3 | 2.1 | 1.5 | 3.6x | 1.02 |
| file_io | 61.2 | 53.6 | 59.1 | 60.5 | 1.0x | 1.14 |
| modules | 62.8 | 61.8 | 1.8 | refused | 34.0x | 1.02 |

Reading notes:

- **op/py** = HEAD vs Bar B, the v3.0 "CPython-level speed" gap from
  BENCH.md, now on the standard bar surface. fib25 8.0x / recursion 8.5x
  are the deep-call overhead rows; `strings`/`seq` carry the quadratic
  `s = s + frag` idiom (see scaling notes).
- **HEAD/A** inside 0.99–1.14 everywhere — main and the release binary are
  the same engine at this baseline (Z-120/Z-125 touched parser/scheduler
  surfaces, not these loops). `file_io` 1.14 is the documented NOISY
  workload (W082); single-run rows there are indicative only.
- **Bar C refusals** (regex/json/modules) are findings, not gaps: there is
  no std-only Rust analog that keeps algorithm parity (a regex engine or a
  JSON library would be a different algorithm wearing the same name — the
  deep-bench refusal-is-a-finding honesty, BENCHMARK-DEEP §1). `modules`
  is additionally a documented shape-compare on Bar B (BENCH.md).

## Scaling t(2N)/t(N)

| workload | HEAD t(2N) | Bar A t(2N) | Bar C t(2N) |
|---|---|---|---|
| loops | 2.03 | 1.99 | 1.25 |
| strings | 1.41 | 1.38 | 1.20 |
| collections | 1.93 | 1.92 | 1.68 |
| grn | 1.95 | 1.97 | 1.00 |
| json | 1.94 | 1.95 | - |
| regex | 1.86 | 1.85 | - |
| seq | 2.80 | 2.81 | 1.02 |
| large_map | 1.93 | 1.92 | 1.67 |
| file_io | 2.47 | 2.26 | 2.42 |
| modules | 1.95 | 1.97 | - |

Scaling notes (the reason the table exists — algorithmic class, honestly
read):

- **Linear class confirmed**: loops 2.03/1.99, json 1.94, grn 1.95,
  modules 1.95, collections 1.93, large_map 1.93 — HEAD matches Bar A to
  within noise; no scaling regression.
- **`seq` 2.80 and `strings` 1.41**: the fixtures build their payload with
  the quadratic `s = s + frag` concat idiom; seq's genome build shows the
  clearest superlinear slope. Report-only (W083 fixture idiom, not an
  engine defect); a P-class candidate if the owner ever ranks it.
- **`regex` 1.86 / `file_io` 2.47**: regex's scan is step-charged with a
  corpus-length-dependent mix; file_io is the documented noisy row (and
  the twin agrees at 2.42, so the slope is real work, not jitter).
- **Bar C ratios are floor-masked for sub-ms twins**: the process-spawn
  floor (~0.3 ms) dominates a 0.5 ms twin, so loops/grn/seq twins read
  1.00–1.25 despite being perfectly linear in Rust. The ratio becomes
  honest only where work >> floor (file_io 2.42, large_map 1.67,
  collections 1.68).
- **fib25 + recursion are n/a by construction**: fib's call count grows
  x1.618 per +1 n and C(2n, n) grows ~x4 per +2 n — a "2x" reading would
  be a lie, so the table refuses it.

## Differential-verified benchmarking (the house rule, enforced)

Every runner prints the value the .op fixture promotes. bars.py refuses
to record ANY timing until HEAD == Bar A == (the imported py mirror's
value) == (the Bar C twin's value) on the same run — the deep-bench
checksum discipline applied to the standard suite. On this baseline every
row is parity-OK, and the gate EARNED its keep before landing:

> **The regex parity catch (disclosed fix):** the first bars run REFUSED
> the regex row — the py mirror counted 12000 hits while both engines
> counted 10800. Root cause was two bench-surface truth bugs, not an
> engine bug: (1) the fixture's `"[0-9]{2,4}-[a-z]+"` pattern is written
> in a double-quoted string, where `{2,4}` is INTERPOLATION — the engines
> were matching the mangled pattern `[0-9]2-[a-z]+` since W083 (SPEC §9.9
> already documents the escaped-brace form `\{2,4\}` for exactly this;
> single quotes are no escape — they are E2001-repaired to double
> quotes); (2) the py mirror used `re.fullmatch` where SPEC §9.9 pins
> `re_match` as an ANCHORED PREFIX test. Fixed in the same PR: the
> fixture now uses the SPEC-documented escaped-brace form, the mirror
> uses `re.match`. Engine regex semantics unchanged (zero src/ delta) —
> the corrected regex row measures 13200 hits on engine, mirror, and both
> bars.

## Reproducibility recipe

```sh
bash scripts/build.sh                                    # HEAD binary (rule 1)
python3 scripts/bench/bars.py --json docs/perf/bars.json # the measurement
python3 scripts/bench/bars_check.py                      # THIS audit (must stay green)
```

- Bar A resolves automatically: latest release asset for this platform,
  downloaded to `$OPERON_BARS_CACHE` (default /tmp/operon-bars-cache),
  sha256-verified against the release's own SHA256SUMS before use;
  `--bara PATH` pins an explicit binary.
- The Bar C twins compile on first use (`rustc -O`, cached by source
  mtime) from `scripts/bench/*_rs.rs`; sizes are passed EXPLICITLY so a
  twin-default drift can never silently change the measured work.
- `bars_check.py --print` emits the exact expected table rows (copy-paste
  re-pin, the composition-pin pattern); `--self-test` mutates a copy and
  verifies the checker notices drift.
- Noise honesty (from BENCH.md, unchanged): ±1–3% run-to-run, ±8–17%
  cross-session on this box class; a "regression" is only a regression
  after an interleaved A/B re-run confirms it outside the band.

## Bar C twins

Existing (untouched): `call_fib_rs.rs` (P4), `call_args_rs.rs` (P4),
`loop_mem_rs.rs` (W082), `sort_scale_rs.rs` (P2). New in this PR:
`loops_rs.rs`, `strings_rs.rs`, `collections_rs.rs`, `recursion_rs.rs`,
`grn_rs.rs`, `seq_rs.rs`, `large_map_rs.rs`, `file_io_rs.rs` — each
mirrors the bench_compare native mirror statement-for-statement (the
import-the-law discipline; the py mirror stays the reference law).
