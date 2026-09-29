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

## v2.4.0 addendum — the poisoning class is retired

The 9 info findings accepted in v2.3.0 (8 `conventional-mutex-unwrap`, 1
`conventional-condvar-unwrap`) were the standing debt of this ledger. In
v2.4.0 they are FIXED, not re-accepted:

- Every `.lock().unwrap()` (src/genes.rs x4, src/interp.rs x4) became
  `.lock().unwrap_or_else(|e| e.into_inner())`.
- The `wait_timeout` site (src/interp.rs, builtin_recv slice loop) became
  `.wait_timeout(..).unwrap_or_else(|e| e.into_inner())`.

Why this is a real robustness win, not cosmetics: a panic inside a worker
while holding a channel-state or task-phase lock poisons it; under `unwrap`
the NEXT thread touching that lock panics too, cascading one controlled
failure into a thread-crash storm. `into_inner()` recovery keeps every
unaffected invariant alive; the original panic still surfaces through its
own join handle, so error attribution is unchanged. The Rust core behavior
contract is untouched — differential 221/0 and vm 215/0 re-confirm parity at
v2.4.0, proofs 129/129, cargo 184/0, redteam 106/0.

Policy change: both sg-rules graduated from info/accepted to warning/
forbidden. They stay in the ruleset as regression guards — any new
poisoning unwrap is now a finding, not a convention.

Battery refresh at v2.4.0: ast-grep 0 findings (rules as guards), semgrep
0 unjustified, bandit 0 real, spc trend unchanged (see reports/ for the raw
SARIF/JSON at the release SHA).

## Session-8 addendum — high-severity class audit + batch-1 drift (main @ ec32c0c + corpus hygiene)

The v2.3.0/v2.4.0 ledger recorded spc counts as raw numbers. This pass audits
every sev-3+ class sample-by-sample/sample-by-sample and measures the drift
that dev1's batch-1 units introduced.

### Drift since v2.4.0 (1419 → 1445, +26)

All +26 are in batch-1's lane and in structural/style classes only:
complex_flow +15, dynamic_memory +5, nested_conditionals +4,
multiple_returns +2 (vm.rs +22, rna2.rs +3, main.rs +1 — the new W11
superinstruction dispatch arms are big match tables; that is what fused ops
look like to a complexity counter). ZERO growth in any security-relevant
class: unsafe_file_op, race_condition, eval_usage, unsafe_input,
unbounded_loops all unchanged.

### High-severity class audit (sev 3–5)

- **eval_usage (2, sev 5) — 0 true.** Both hits are the ffi.rs `unsafe {}`
  blocks (rt_edit_distance / rt_codon_score); spc's eval rule misfires on
  `unsafe {` + kernel symbol names. Both sites carry SAFETY docs and the
  DP_CELL_BUDGET / FFI_OPERAND_CAP caps.
- **race_condition (10, sev 3) — 0 true.** All ten are
  `Ordering::Relaxed` atomics: cancel-flag polls (no ordering-dependent
  data; polled in loops, so visibility is eventual by construction), the
  fuel-pool fetch_sub (exact-count atomic RMW; ordering irrelevant to a
  monotone budget), and the ALLOC_BYTES ceiling (monotone fetch_add with a
  saturating compare). Relaxed is the correct memory order at every site.
- **unsafe_input (2, sev 4) — 0 true.** REPL stdin read_line and
  `env::args` — the input surface of a CLI toolchain; the language's
  capability system governs what programs can do with them.
- **unbounded_loops (68, sev 5) — 0 true, sampled per file class.** Parser
  loops break on Tok::Eof and advance pos with a no-progress skip; the VM
  dispatch loop pays `interp.tick()?` (fuel) every iteration and falls off
  code end; the stress wait loop is deadline-bounded (timeout capped at
  300 s); the stdout/stderr drainers are 64 MiB-capped; the REPL is
  user-driven. Containment on hostile input is redteam's job — 106/0.
- **unsafe_file_op (45, sev 3) — 0 true, sampled.** All sampled sites are
  the stdlib FS builtins, which sit behind the capability gate
  (caps_policy.op / security_caps.op pin this).
- **set_timeout (30, sev 4) — 0 true.** spc's JS heuristic misreads
  bounded `Duration`/`thread::sleep` poll schedules.

### Real find of the pass: corpus-walker sandbox hygiene (fixed)

fix_corpus law1 went RED in this sandbox: `examples/cookbook/fsm_turnstile.op`
— an UNTRACKED draft left by a parallel lane — was swept by the corpus
walker, and its (draft-quality) source red the meaning-preservation law. CI
never sees this (clean checkout), which is exactly why it must be fixed
here: any developer with stray drafts gets a false red. All three law
walkers (fix_corpus.rs, fmt_idempotence.rs, fmt_width.rs) now filter to
`git ls-files`-tracked .op files, degrading to include-all when git is
absent (source tarballs). Also removed a stale ast-grep suppression
directive in vm.rs left behind by the batch-1 refactor (stale suppressions
hide future real findings).

### Battery at ec32c0c + hygiene fix

ast-grep 0 · semgrep 0 unjustified · bandit 0 real · spc 1445 (see drift
above) · **CodeQL 0 findings** (rust-security-and-quality, 0 extraction
errors) · cargo 199/0 · proofs 132/132 (2025 asserts) · redteam 106/0 ·
harness vm lane 224/0 · clippy/fmt clean.
