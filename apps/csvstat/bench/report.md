# csvstat cross-language bench — Operon vs Python (CPython 3.12)

Fixture: `test/data/big.csv` (5000 rows x 6 cols, deterministic LCG,
contains quoted fields: `"gpu, pro"`, `say "hi"`, so the CSV parser
exercises both the bulk path and the quoted-region segment feed).
Method: N timed runs per engine per workload, median reported; sha256
over stdout captured per run — the cross-engine checksum contract must
hold on every workload. Machine: 2-core Xeon cloud VM (see results JSON
meta). All numbers in milliseconds.

## Results (results_v1.json, 2026-10-04)

| workload   | operon (v1) | operon (hybrid csv_parse) | python | ratio now | ratio v1 |
|------------|-------------|---------------------------|--------|-----------|----------|
| startup    | 2.7         | 2.8                       | 20.4   | 0.14x     | 0.13x    |
| info_big   | 331.0       | 158.0                     | 33.5   | 4.7x      | 9.9x     |
| stats_big  | 369.1       | 190.9                     | 38.2   | 5.0x      | 9.7x     |
| top_big    | 218.3       | 51.2                      | 26.4   | 1.9x      | 8.3x     |
| table_big  | 210.7       | 42.2                      | 24.9   | 1.7x      | 8.4x     |
| stats_small| 6.0         | 4.5                       | 20.2   | 0.22x     | 0.30x    |

("ratio now" = operon/python after the std/csv hybrid parser landed;
"v1" = the original pure state-machine parser. startup and
stats_small lead CPython outright; the 5000-row workloads went from
~8-10x slower to 1.7x-5.0x slower.)

## What moved the needle

- **std/csv hybrid parser (this batch):** quote-free lines parse via one
  bulk `split` each; quoted regions feed the original character machine
  on whole segments (runs between quote chars), exploiting the fact
  that the field buffer is join("")-accumulated so chunk granularity is
  free. Equivalence is pinned three ways: 58 asserts in
  `tests/std_csv.op` (contract + hybrid-vs-machine corners), a 327-case
  randomized differential fuzz (scripts/fuzz_csv_equivalence.py), and
  the 6/6 checksum contract here.
- csv_parse self-time: ~200 ms -> ~35 ms per 5000-row document (profile).

## Known residual (parked, P-task ledger material)

- csv_records: 45 ms for 5000 header-keyed maps — map insert cost, the
  P1 (map representation) surface. Parked per the roadmap activation law.
- The app's own stats walk (~90 ms on stats_big) is app-level code, not
  stdlib; optimizing it further is app work, deliberately left as the
  regression baseline for future runtime wins.
