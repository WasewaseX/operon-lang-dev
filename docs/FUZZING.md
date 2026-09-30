# FUZZING — the standing lane (S7)

Total Grammar promises "every input must not crash." The proof suite holds
that promise against inputs people WROTE; the fuzz lane holds it against
inputs nobody wrote YET. This page is the lane's contract and inventory;
the vocabulary (clean / contained / crash / hang) is defined at the top of
scripts/fuzz/fuzz.py and in scripts/fuzz/TRIAGE.md.

**The rule: a finding is a BUG to fix, not a number to brag about.**
"0 findings" is a property hold; "1 finding" is the fuzzer doing its job.

## The tools

| Tool | Method | Surface | Determinism |
|---|---|---|---|
| scripts/fuzz/fuzz.py (W051) | mutation-based black-box | check, ast --json, fmt, explain [--json] | --seed S |
| scripts/fuzz_parser.py | grammar-aware generation | parser via exec engines | --seed S |
| scripts/fuzz/fuzz_diff.py (S7 s2) | grammar-aware differential, at scale | run (VM + tree-walk) vs oracle, check rc contract, check --json shape parity | --seed S |
| scripts/gen_corpus.py | grammar-aware corpus generator | compat matrix input | --seed S |
| scripts/compat_matrix.sh | differential matrix | tree-walk / VM / VM-opt / debug / oracle | fixed corpus + seed |

## Surface inventory (S7 stage 1)

Fuzzed TODAY (parse/tooling layer, all inputs must end clean or contained):

- `operon check` — correctness grader
- `operon ast --json` — AST dump
- `operon fmt` — canonical formatter (parse+reprint)
- `operon explain [--json]` — repair play-by-play (S7 slice 1: joined the
  target row; was excluded as a batch-2 WIP lane)

Fuzzed by the grammar generator (fuzz_parser.py): lexer + parser + the
exec engines in agreement mode (tree-walk vs oracle).

NOT YET fuzzed (the S7 roadmap, in order):

- `operon run` on generated programs under a default-deny, fuel-capped,
  timed profile (the exec-surface property: contained-or-clean, never
  panic/hang; slice 3)
- `rna --check`, `doc`, `graph --json`, `crispr`, `disasm` tooling surfaces
  (slice 3)
- differential fuzzing at scale: Rust vs Python oracle byte-identical
  stdout over THOUSANDS of generated programs per round, plus
  `check --json` shape parity — DONE (slice 2, scripts/fuzz/fuzz_diff.py;
  reuses gen_corpus.py's bucket generators verbatim, so
  fuzz_diff(seed, i) == gen_corpus(seed, i) program for program)
- redteam-directed generation: capability-escape attempts seeded from the
  denial vocabulary (slice 3)

## Running it

```
python3 scripts/fuzz/fuzz.py --time-budget 120 --execs 300 --seed 20260930
python3 scripts/fuzz_parser.py --n 500 --seed 20260930 --exec
python3 scripts/fuzz/fuzz_diff.py --n 2400 --seed 20260930 --time-budget 480
FAST=1 bash scripts/compat_matrix.sh        # 10% sample
```

Exit code 1 = findings exist. They are SAVED under fuzz_corpus/ with a
MANIFEST.jsonl line each: triage them (scripts/fuzz/TRIAGE.md), do not
gloat, and land the fix with a regression .op so the finding can never
reoccur invisibly.

## Baseline (recorded)

- fuzz.py seed 20260930, 300 execs (check/ast/fmt/explain x5 targets):
  clean=219 contained=81 crash=0 hang=0
- fuzz_parser.py seed 20260930, 200 inputs, exec mode: no panics, no
  hangs, no breaches, engines agree
- fuzz_diff.py seed 20260930, 2400 generated programs x 5 surfaces
  (run VM, run tree-walk, oracle, check, check --json) = 12,000 execs in
  244s: 0 divergences, 0 rc-contract violations, 0 shape-parity
  violations, 0 panics/hangs; all 12 buckets x 200

## fuzz-r3 deep sweep (record)

The one-off deep sweeps behind the current baseline, run on the FIXED
binary (see the finding below):

- fuzz.py seed 20261030, 3000 execs: clean=2213 contained=787 crash=0
  hang=0, no findings
- fuzz_parser.py seed 777, 1000 inputs, exec mode: no panics, no hangs,
  no breaches, engines agree

## Findings (the fuzzer doing its job)

- fuzz-r3 (found by fuzz.py seed 20260930, execs 896/1051/1606, crash
  corpus preserved in fuzz_corpus/ with MANIFEST.jsonl): `operon check`
  panicked (rc 101) in `Span::of_token_word` — the phantom-label scan
  sliced at byte indexes that can sit INSIDE a multibyte character
  (a sigma, U+2028, and a CJK char in the three inputs; all from
  unicode_splice mutations of real test files). Delta-minimized to the
  5-byte program `字d(`. Fixed in src/diag.rs: char-boundary-safe
  slicing plus advance-by-character restart, unit-tested with the
  minimized case, pinned as tests/diagnostics/char_boundary_phantom.op
  in the diag_golden gate. All three crash inputs now exit 0.
