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
| fuzz/ (F5, #49) | cargo-fuzz libFuzzer, in-process, coverage-guided | parse (lexer+parser), check, run (VM-load: load+top-level+entry, default-deny, fuel-capped) | -seed=N (date-derived in CI) |
| scripts/fuzz_parser.py | grammar-aware generation | parser via exec engines | --seed S |
| scripts/fuzz/fuzz_diff.py (S7 s2) | grammar-aware differential, at scale | run (VM + tree-walk) vs oracle, check rc contract, check --json shape parity | --seed S |
| scripts/fuzz/fuzz_exec.py (S7 s3) | grammar-aware + redteam-directed escape generation | run default-deny + fuel cap (E1/E1b/E2), rna --check / doc / graph / disasm / crispr (E3) | --seed S |
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

Fuzzed by the exec-surface lane (S7 slice 3, fuzz_exec.py):

- `operon run` under a default-deny profile (zero grants) with a fuel
  cap: the contained-or-clean property (E1) plus the breach sentinel
  (E1b) — escape-shaped programs print EXECBREACH only on the
  effect-success path, so an ungranted effect that ever executes is a
  finding independent of the exit code
- the capability funnel itself: programs seeded from the denial
  vocabulary (read_file/write_file/append_file/read_file_bytes/
  write_file_bytes/exists/file_size/read_dir/fs_delete/fs_rename/
  fs_mkdir/http_get/run/py/env/exit) with adversarial argument shapes
  (traversal, escape paths, symlink names, hosts, commands, modules),
  bare and stress/rescue-wrapped and loop-wrapped (E2: fuel exhaustion
  is the expected end, never a hang), spliced into real bucket programs
- the tooling surfaces (E3): `rna <f> <patch> --check` (both text and
  --json; plus the never-writes property — input bytes hashed
  before/after must match), `doc [--json]`, `graph [--json]`,
  `disasm [--json]`, `crispr --knockout G [--json]` on generated and
  byte-mutated programs

NOT YET fuzzed (the S7 roadmap, in order):

- differential fuzzing at scale: Rust vs Python oracle byte-identical
  stdout over THOUSANDS of generated programs per round, plus
  `check --json` shape parity — DONE (slice 2, scripts/fuzz/fuzz_diff.py;
  reuses gen_corpus.py's bucket generators verbatim, so
  fuzz_diff(seed, i) == gen_corpus(seed, i) program for program)
- redteam-directed generation: capability-escape attempts seeded from the
  denial vocabulary — DONE (slice 3, the escape leg of fuzz_exec.py)
- libFuzzer in-process targets (cargo-fuzz) for the Rust core: the deeper
  layer W051's done-when names; the black-box lanes above are the
  zero-setup daily driver that runs in CI today — DONE (F5, issue #49:
  the fuzz/ crate, three targets, nightly workflow
  fuzz-inproc.yml; see "The in-process layer" below)

## The in-process layer (F5, issue #49)

The fuzz/ crate drives the Rust core IN-PROCESS with coverage guidance —
the class the black-box lanes cannot reach (parser state-machine corners
behind complex input shapes; the C1 multibyte panic needed pure luck
black-box, coverage guidance makes it cheap). One target per surface:

- `parse` — lexer + parser (`parser::parse`). ASan + leak-checked.
- `check` — parse + `typeck::check_program`. ASan + leak-checked.
- `run` — the exact CLI run sequence in-process (`tools::load_file` +
  `tools::run_entry`): default-deny caps, stdout sunk (dx-r3), entry leg
  fuel-capped at 200k (E2 profile), VM lane on. Coverage-only
  (`--sanitizer none`) + `-detect_leaks=0` — see the harness notes.

Seeds = tests/ + examples/ + fuzz_corpus/, passed to libFuzzer as
read-only dirs (no seed files are duplicated into fuzz/). Every input
runs on a big-stack worker thread (fuzz/fuzz_targets/common.rs, 512 MB)
so the ASan build fits the SAME shipped nesting thresholds the CLI ships
— the engine source sees zero delta and differential parity is untouched
by construction.

Harness notes, recorded honestly (each found by this layer on day one):

1. ASan stack economics: 4096-deep `[[[[…]]]]` truncates contained on
   the CLI (rc 0 repair note) but SIGSEGV'd the ASan fuzz target — ASan
   inflates frames several-fold and 4096 nesting × the ~5-frame
   recursive-descent chain no longer fits the stack. The big-stack
   worker keeps the shipped threshold under test; the plain CLI is
   byte-identical in behavior (verified on the crash input).
2. Rc reference cycles: a malformed `gene f( {` leaves gene/env
   references in a cycle; LSan flags it on the run target. The shipped
   CLI model (one program → exit → OS reclaims) never exposes it; a
   Weak-based cycle break is an engine-lane follow-up, so the run target
   runs with -detect_leaks=0 and this REMAIN is tracked here. parse and
   check stay leak-checked (acyclic ASTs).
3. Recursion + rescue: `gene f` calling itself inside `rescue` (and the
   `stress`+`rescue` double-pump variant) pumps the SPEC §7
   catch-and-retry loop; the CLI contains it at rc 1 (E1020) at
   fuel ∈ {200k, 1M, 200M} but the ASan build overflowed even a 512 MB
   stack. Coverage-only for run removes the false-positive flood this
   class would feed the run target; the containment is pinned by the
   CLI cross-checks.
4. `#` comment splice slow-unit: an unbounded top-level loop contained
   via the 10k note cap (CLI rc 0, 4.3 s) burns the run target's quiet
   budget for ~20 s. Reproducible-slow class — the nightly workflow
   replay-gates every artifact SOLO before it counts as a finding, so
   known-slow transients are recorded without crying wolf.

## Running it

```
python3 scripts/fuzz/fuzz.py --time-budget 120 --execs 300 --seed 20260930
python3 scripts/fuzz_parser.py --n 500 --seed 20260930 --exec
python3 scripts/fuzz/fuzz_diff.py --n 2400 --seed 20260930 --time-budget 480
python3 scripts/fuzz/fuzz_exec.py --n 900 --seed 20261001 --time-budget 600
FAST=1 bash scripts/compat_matrix.sh        # 10% sample
```

In CI (S7 stage 5): the FAST pre-merge `fuzz` job runs all three lanes
with fixed seed 20261001 and time-boxed budgets — a finding FAILS the
run; the nightly `fuzz-nightly` job (schedule + workflow_dispatch) runs
the full deep sweeps on a date-derived seed recorded in the log.

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
- fuzz_exec.py seed 20261001, 900 programs (run:301, escape:301,
  tooling:298 legs): E1/E1b/E2/E3 all hold, 0 findings
- fuzz_exec.py seed 20261030 deep sweep, 9000 programs (3001/3001/2998)
  in 59s: 0 findings — default-deny run path, fuel counter and tooling
  surfaces hold contained-or-clean

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
