# Static-Analysis Battery — Triage Ledger

Release base: `main` @ `84411fd` (v2.3.0 candidate), branch `b2/release-v2.3.0`.
Date: 2026-09-28. Operator: builder-B (dev 2). This ledger records every tool,
every finding class, and the verdict for each — the honest counterpart to the
raw SARIF reports in `reports/`.

## Why this battery

CodeQL (already run every session, 0 findings) is flow analysis. These tools
add orthogonal lenses: structural search (ast-grep), pattern packs (semgrep),
Python linting (bandit), NASA Power-of-Ten style reliability rules (spc).

## Tool-by-tool results

### 1. ast-grep (structural, Rust core) — 90 findings → 9 info, 0 warn, 0 err

Rules live in `sg-rules/`, project config in `sgconfig.yml` (repo root).
Raw report: `reports/ast-grep-84411fd.sarif` (final state).

| rule | before | after | verdict |
|------|--------|-------|---------|
| no-std-process-exit-in-core | 26 | 0 | all 26 justified: CLI exit-code plumbing (main.rs, operon-ls.rs), the documented `die()`/`die_pkg()` helpers, the debug-REPL quit, pkg subcommand status exits, and the capability-gated `exit` builtin (interp.rs:6458 IS the gate). Each site carries `// ast-grep-ignore` + reason. |
| no-unwrap-in-src | 49 | 0 | every site audited one by one: guard-protected (`is_some()` guards, `num()` checks), invariants by construction (symmetric stack push/pop in codegen, `rest` starting with the matched needle in pkg.rs, parser rejecting empty variant lists at parser.rs:2304), or `#[cfg(test)]` code. Mutex/Condvar poisoning unwraps moved to their own info rules. |
| no-raw-thread-spawn | 5 | 0 | all 5 are internals of governed paths: capped stdout/stderr drainers (64 MiB, sec-r4) in interp.rs + pybridge.rs, and the bounded HTTP accept loop (MAX_CONNECTIONS = 256). The operon-level spawn governance sits above these. |
| no-unsafe-block-in-src | 2 | 0 | both in ffi.rs (C++ kernel FFI). Fixed properly: SAFETY comments documenting pointer validity, lifetime, and the budget caps. Suppressed with trailing directives. |
| no-todo-in-src | 2 | 0 | false positives: `\uXXXX` comment text and a `TODO-100` board-filename reference. Rule regex tightened to require a non-`-` terminator and XXX dropped. |
| conventional-mutex-unwrap (info) | — | 8 | accepted class, tracked permanently. |
| conventional-condvar-unwrap (info) | — | 1 | accepted class, tracked permanently. |

**Real bug found and fixed on main:** `scan_width_breaks` (src/tools.rs, the
W47-v2 `--width` soft-wrap pass) treated `#`-comment lines as code — the `(`
inside a `##` doc comment opened a depth frame, the comment's comma became a
break candidate, and the wrapped continuation lost its `##` prefix and
re-parsed as CODE (`...a caller bug, say so with a value, not a crash`
produced a `say()` call). Caught by dev1's own `fmt_width_corpus_laws` test,
which was red on main — invisible because the GitHub Actions outage kept CI
from ever running on the recent M100 pushes. Fix: the scanner stops at `#`
(a comment runs to end of line; no break may exist past it). fmt_width 14/14.

### 2. semgrep 1.178.0 (packs: p/rust, p/security-audit, p/secrets) — 8 findings, all justified

Raw report: `reports/semgrep-84411fd.sarif`.

| rule | count | verdict |
|------|-------|---------|
| current-exe | 4 | exe-relative std-tree resolution is the documented dx-r5 fix (install layout …/bin/operon + …/std); current_exe is used to locate bundled std/, not to trust content. |
| args | 2 | `std::env::args()` in the two CLI binaries — that is their job. |
| unsafe-usage | 2 | the same two ffi.rs blocks, now with SAFETY comments. |

secrets pack: 0 findings (no credentials in tree).

### 3. bandit 1.9.4 (Python: bootstrap/, scripts/) — 30 findings, all benign tooling classes

Raw report: `reports/bandit-84411fd.json` (bandit has no native SARIF; JSON).

| rule | count | verdict |
|------|-------|---------|
| B603 subprocess_without_shell | 11 | harness.py/oracle.py/bench_compare.py intentionally spawn the two engines; argument-list form, no shell=True anywhere. |
| B404 subprocess import | 9 | same files, same reason. |
| B108 hardcoded /tmp | 4 | bench tooling scratch space; not security-relevant (bench_compare.py). |
| B311 pseudo-random | 3 | Monte Carlo bench workloads — randomness is the point, crypto is not involved. |
| B607 partial path | 2 | invoking `operon`/`python` from PATH inside the harness scripts. |
| B112 try/except/continue | 1 | benchmark loop resilience. |

### 4. spc (space-proof-code v1.5.0, NASA Power-of-Ten style) — 1419 style flags, standing debt ledger

Raw report: `reports/spc-src-84411fd.sarif`. Top classes on the interpreter
core: complex_flow 535, nested_conditionals 308, dynamic_memory 234 (Rc/RefCell
value model — inherent to the interpreter's design), multiple_returns 116,
unbounded_loops 68 (eval loops by nature), unsafe_file_op 45 (the capability-
gated fs builtins), set_timeout 30, try_catch 19, global_vars 17, recursion 14,
exceeds_max_func_lines 14, race_condition 10 (Rust's compiler prevents data
races; the heuristic flags shared-mutation patterns).

Verdict: these are architectural style rules aimed at aerospace C; applied to
a 300k-line interpreter they describe a long-horizon refactoring program, not
release blockers. Recorded as the quality-debt baseline so future passes can
measure the trend. No action in v2.3.0.

### 5. CodeQL (per-session standing directive) — run on the release merge; see reports/

CLI 2.27.1, `rust-security-and-quality.qls` suite. Previous scans: 0 findings
at PR #28 head and at f0527e5. Fresh scan over the v2.3.0 release commit is
committed to `reports/` when it lands (see COMMS for the run record).

### 6. Not runnable in this environment (recorded, not skipped silently)

- **CodeSonar**: commercial license required — not available.
- **SonarQube CE**: requires a long-running server + scanner pair; no server
  in this environment. The workflow skeleton in static-analysis.yml keeps the
  hook point ready.

## Supply-chain hardening status in this release

- `softprops/action-gh-release` was already SHA-pinned (D-6/sec-r5 pass) —
  verified again here: release.yml, ci.yml, perf.yml, and the new
  static-analysis.yml carry zero floating action refs.
- New `.github/workflows/static-analysis.yml` runs ast-grep + bandit + semgrep
  on every push/PR as a NON-BLOCKING job (the B1 bench-job precedent:
  continue-on-error, evidence over gates) and uploads SARIF artifacts, so the
  next environment reset or CI outage cannot silently re-redden main. Both of
  its actions are SHA-pinned (checkout v4.2.2, upload-artifact v4.6.2).
