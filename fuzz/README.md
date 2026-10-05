# fuzz/ — the in-process layer (F5, issue #49)

One libFuzzer target per surface, in-process and coverage-guided — the
class the black-box lanes (scripts/fuzz/*) cannot reach: parser
state-machine corners behind complex input shapes. The C1 multibyte
panic needed pure luck black-box; coverage guidance makes it cheap.

## Targets

| target | surface | sanitizer |
|---|---|---|
| `parse` | lexer + parser (`parser::parse`) | ASan + leak-check |
| `check` | parse + `typeck::check_program` | ASan + leak-check |
| `run` | the exact CLI run sequence in-process (`tools::load_file` + `tools::run_entry`, default-deny caps, stdout sunk, entry fuel 200k E2, run-wide pool 200M, VM lane on `vm_opt 0`) | coverage-only, `-detect_leaks=0` (rationale in FUZZING.md) |

Every input runs on a **big-stack worker** (512 MB,
`fuzz_targets/common.rs`) with panic re-propagation, so the ASan build
fits the SAME shipped 4096 nesting thresholds the CLI ships — the engine
source sees ZERO delta and differential parity is untouched by
construction.

## Run it

```sh
cargo fuzz build                        # nightly; all three targets
# libFuzzer: FIRST dir is the writable corpus, the rest are read-only seeds
cargo fuzz run parse  fuzz/corpus  tests/ examples/ fuzz_corpus/ -- -max_total_time=300 -timeout=10
cargo fuzz run check  fuzz/corpus  tests/ examples/ fuzz_corpus/ -- -max_total_time=300 -timeout=10
cargo fuzz run run    fuzz/corpus  tests/ examples/ fuzz_corpus/ -- -max_total_time=900 -timeout=30 -detect_leaks=0
```

Seeds = tests/ + examples/ + the committed crash corpus (fuzz_corpus/),
passed as read-only dirs — no seed files are duplicated into this crate.
The writable corpus dir is gitignored (coverage accumulates per session).

CI runs the same three targets nightly (non-blocking, date-derived seed,
replay-gated artifacts) — see .github/workflows/fuzz-inproc.yml.
