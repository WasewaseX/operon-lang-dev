# loglens — access-log analytics (Lane-A test app #4)

A web-server access-log analytics CLI, written in Operon, with three
byte-identical reference engines. It is deliberately the room's weak-spot
workload: per-line string splitting (P5 surface), map get/set + iteration
(P1 surface), comparator sorting (P2 surface), and bulk line streaming.

## Engines

| engine | file | role |
|---|---|---|
| Operon | `loglens.op` | the app under test |
| Python 3 | `loglens.py` | Bar B (mainstream interpreter baseline) |
| Node.js | `loglens.js` | mainstream runtime baseline (V8) |
| Rust | `loglens.rs` | Bar C (native algorithmic ceiling), `rustc -O` |

All four must print byte-identical stdout for every subcommand (sha256
contract, asserted on every bench run — a VM optimization that changes a
single output byte fails the bench).

## Subcommands

```
loglens stats FILE          records/hosts/urls/bytes census + status histogram
loglens top FILE FIELD N    frequency table for host|url|status
loglens errors FILE [N]     >=400 census + top-N error urls (default N=10)
loglens table FILE [N]      first N records as a table (default N=10)
```

## Run

```
./loglens.sh stats test/data/big.log          # launcher (sandbox grants)
operon run apps/loglens/loglens.op --cell apps/loglens/loglens.cell \
    --allow-read $PWD -- stats apps/loglens/test/data/big.log
python3 loglens.py stats test/data/big.log
node loglens.js stats test/data/big.log
rustc -O loglens.rs -o /tmp/loglens_rs && /tmp/loglens_rs stats test/data/big.log
```

## Bench

```
python3 bench/bench_loglens.py     # needs rustc for the Bar-C column
```

Results: `bench/results_v1.json`, medians over 7 timed runs (15 for
startup/small), sha256 of stdout per run.

## Fixture

`test/gen_fixture.py` — LCG seed 42, Common Log Format, exactly two `"`
per line, canonical integer status/bytes, \n only, one trailing newline.
`small.log` 150 records, `big.log` 50,000 records (~4.1 MB). Regenerating
must be byte-identical (generate small THEN big — global LCG stream).

## Gates

- proof frame in `loglens.op` (49 asserts) — runs under `operon test apps/`
- 4-way byte parity on small + big fixtures (see bench checksum contract)
- launcher e2e incl. missing-file exit 2 (test/run_tests.sh)

## fmt hazard RESOLVED (2026-10-04, W47-v3)

The fmt Lambda bug caught here on 2026-10-04 (`fmt_body_inline` rendering
multi-statement closure bodies as `gene(..) => null`, silent source
corruption) is FIXED in src/tools.rs (`fmt_lambda`): a body renders inline
`gene(..) => e` only when it is exactly one `return e`; every other body
round-trips through the braced form. Pinned by
tests/fmt_idempotence.rs#fmt_multistmt_lambda_body_survives;
`fmt --write` on `loglens.op` now round-trips byte-identically
(comments are still normalized by canonical fmt, unchanged behavior).
