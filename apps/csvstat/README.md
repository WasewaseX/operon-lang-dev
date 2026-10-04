# csvstat — CSV analytics test app (lane A: test apps / benchmarking)

A pure-Operon CLI for CSV census, stats, frequency tables and pretty
tables. It exists for three reasons:

1. **Real-app regression vehicle.** Like apps/ytdl, it exercises the
   runtime the way actual programs do — file iteration, string
   splitting, map aggregation, table rendering — with a hermetic e2e
   (`test/run_tests.sh`, 24 checks) and a proof frame inside
   `csvstat.op` (43 asserts, runs under `operon test apps/`).
2. **Cross-language differential at the application level.** The Python
   reference `csvstat.py` must produce byte-identical stdout on every
   subcommand; the bench asserts sha256 equality per workload, so no
   optimization can buy speed with a semantics change. 6/6 identical on
   every run.
3. **A benchmark that amplifies the weak workloads.** The cross-language
   deep bench showed lines/table/csv-shaped work as Operon's weakest vs
   CPython; this app quantifies that gap in milliseconds on a real
   5000-row document and validates stdlib optimizations end-to-end.

## Usage

```sh
apps/csvstat/csvstat.sh info  FILE
apps/csvstat/csvstat.sh stats FILE
apps/csvstat/csvstat.sh top   FILE COL N
apps/csvstat/csvstat.sh table FILE [N]
```

The launcher grants read access to the repo tree only (default-deny
sandbox) and relocates to the repo root because std/ module resolution
is CWD-relative (known quirk, documented in BENCH.md).

## Files

| path | role |
|---|---|
| `csvstat.op` | the app (Operon) + proof frame |
| `csvstat.py` | the byte-identical Python reference |
| `csvstat.sh` / `csvstat.cell` | launcher + runtime contract |
| `test/run_tests.sh` | hermetic e2e (24 checks) |
| `test/gen_fixture.py` | deterministic fixture generator (seed 42) |
| `test/data/small.csv` / `big.csv` | 40-row / 5000-row fixtures |
| `bench/bench_csvstat.py` | cross-language benchmark |
| `bench/results_v1.json` | machine-readable results |
| `bench/report.md` | results narrative |

## Determinism contract

Integers render via `str(int)` on both engines; ratios render via
`render_ratio()`, whose float op sequence (`x = num/den`; `sc = 10^nd`
by repeated `*10.0`; `r = floor(x*sc + 0.5)`; integer split) is
mirrored op-for-op by the Python side — bit-identical inputs give
bit-identical integers, so the rendered bytes match. No float
`repr()`/`str()` is ever printed. No interpolation format specs are
used (the spec after `:` is currently inert in the runtime — filed as a
finding, see the chatroom session report).
