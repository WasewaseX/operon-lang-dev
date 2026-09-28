# CONTRIBUTING.md, the collaboration constitution

This repository is built by **one human owner and multiple AI agents working concurrently**.
Everything an agent needs to not step on another agent is in this file. If a rule here conflicts
with habit or convenience, the rule wins. This document **grows over time** (see §14): every
incident that two agents resolved by improvisation becomes a rule here so nobody improvises
twice.

Current living state is in **§10 ("Always know")**, re-read it at the start of every session.

---

## 1. Who works here

| Identity | Role | Branch prefix | Touches |
|---|---|---|---|
| **owner** (human, WasewaseX) | final authority, merges PRs, owns decisions D-##, owns secrets |, | everything |
| **builder-A** | lead implementer: Rust core, interpreter, parser, VM, LSP | `builder/*` | `src/**`, `bootstrap/**`, `tests/**`, `SPEC.md`, `.github/**` |
| **sz** | reviewer/QA: diff audits, differential oracle, SPEC drafts, release engineering, docs | `sz/*` | `bootstrap/**`, `.github/**`, `scripts/**`, docs, `tests/**`, plus read-access to everything |
| **builder-B** | mainstreaming: benchmarks, stdlib (.op), cookbook, playground | `b2/*` | `scripts/bench*`, `std/**`, `tests/std_*`, `examples/cookbook/**`, `web/playground/**`, `BENCH.md`, `STDLIB.md` |

Tasks are claimed in `project-vault/collab/TASKS.md` (the queue) and discussed in
`project-vault/collab/COMMS.md` (the message board). Reviews live in
`project-vault/collab/REVIEWS.md`. Locked decisions live in `project-vault/collab/DECISIONS.md`.

**One claimant per task.** Before starting work, write your claim (edit TASKS.md and post a COMMS
note). If a task is claimed, do not start it, pick another or split with a COMMS proposal.

---

## 2. Iron rules (violating any of these = the merge gets reverted)

1. **Never push to `main` directly.** All work lands through a pull request, even "one-line" changes.
2. **Open the PR immediately**, the moment your branch is pushed, open a PR (mark it `WIP:` in the
   title if unfinished). Two agents once raced a rebase against another's merge because the work
   existed only as a branch; a visible PR would have prevented it. A pushed-but-un-PR'd branch is a landmine.
3. **CI green before merge-request.** No "it passed on my machine" merges. If CI is red, your next
   action is fixing it, not merging.
4. **Invariants are load-bearing** (see §6). If any invariant fails after your change, stop, do not
   push, post to COMMS.
5. **Bug fixes require fail-pre tests**: a test that demonstrably fails on the pre-fix binary and
   passes on the post-fix binary. State this in the commit message (`fails-pre verified`).
6. **Rust ↔ oracle mirror**: every semantic change to the interpreter must be mirrored in
   `bootstrap/oracle.py` in the same PR. The differential harness (`bootstrap/harness.py`) must end
   with `0 diverge`. A semantic change without an oracle mirror is an incomplete change.
7. **SPEC travels with code**: behavior changes update `SPEC.md` in the same PR, written
   programmer-first (D-008: the audience is CS engineers with zero biology; gene vocabulary is an
   intuition aid, never a prerequisite).
8. **Back-compat default**: any new gating semantics must default to old behavior (e.g. threshold 0
   = always-on). Old `.op` programs must run byte-identically unless the queue explicitly says
   otherwise and DECISIONS.md records it.
9. **Secrets never enter the repo or chat.** Tokens live in the owner's secret store. If a token
   appears in any output, file, or message: rotate immediately (owner action) and scrub the artifact.
10. **Honesty over marketing** (D-008): a keyword must do what its name promises. If you find a
    mechanism that only *sounds* like it works, file it as a finding in REVIEWS.md, this project's
    credibility was nearly destroyed once by five such mechanisms (T2a–T2e, all fixed).
11. **Do not rewrite pushed history** on shared branches. Rebase only your own unmerged branches,
    and `--force-with-lease`, never `--force`.
12. **Verify before claiming done.** "Done" in TASKS.md requires the task's acceptance gate green
    *on the merged target* (not just your branch), with the evidence (counts, run IDs) pasted in.

---

## 3. Branches and commits

**Branch naming**

```
builder/<task>-<slug>     e.g. builder/t2a-grn-gating
sz/<task>-<slug>          e.g. sz/r3-oracle, sz/c-contributing
b2/<task>-<slug>          e.g. b2/b1-benchmarks
```

Branches are deleted after merge. Do not pile multiple tasks into one branch; small PRs merge fast
and conflict less.

**Commit message format**

```
<area>(<scope>): <imperative summary, lower-case, no period>

<body: what changed and why, the reviewer reads only this + the diff>

Verification: <the exact commands run and their results>
```

- areas: `interp`, `parser`, `lexer`, `genes`, `tools`, `ls`, `repl`, `oracle`, `harness`, `std`,
  `tests`, `ci`, `release`, `docs`, `bench`, `playground`, `spec`, `t2x…` (task ids)
- example: `t2a(interp): GRN level now gates gene calls` + body + `Verification: proofs 23/267, oracle 29/29, fails-pre verified`

**What never goes in a commit**: generated artifacts (`target/`, `bin/`, `build/`, `dist/`), secrets,
editor config, or files outside your lane (§7) without a COMMS agreement first.

---

## 4. Pull request protocol

1. **Title**: `[task-id] plain-language what it does`, e.g. `[T4] REPL completion: :load/:proof/:genes/:vars/:reset`.
2. **Body must contain**: what + why; conflict-resolution notes if you rebased; the verification
   block (commands + results); which TASKS item this closes; any SPEC/behavioral impact.
3. **Who merges**: the owner, or an agent explicitly delegated in COMMS for that PR. The *reviewer*
   (sz by default) approves in REVIEWS.md before merge-requesting.
4. **Merge style**: merge commits (history keeps task grouping). Squash only for single-commit PRs.
5. **After merge**: delete the branch, mark TASKS done with the merge SHA, post a COMMS note. The
   R-gate (§5) reviews it within one sz session.
6. **Rebase etiquette**: rebase onto `origin/main` before requesting merge. If conflicts touch
   another lane's files, resolve minimally and note it in the PR body, do not silently absorb
   someone else's in-flight work.
7. **WIP → ready**: when CI is green and the task's acceptance gate passes, remove `WIP:`, comment
   `ready for merge` in COMMS with evidence.

**Emergency hotfix path** (CI broken on main / release-blocking bug): branch `hotfix/<slug>`,
smallest possible diff, COMMS `@owner HOTFIX` note immediately, merge after one green CI run.

---

## 5. Review protocol (the R-gate)

- **Standing rule R4**: *every merge to main gets a REVIEWS.md entry within one sz session.*
  A review without findings is still an entry (that's what "clean" means).
- Findings are graded: **HIGH** (wrong behavior, broken invariant, dishonest mechanism),
  **MEDIUM** (spec drift, missing mirror, fragile test), **LOW** (style, docs), **POSITIVE**
  (worth copying elsewhere). Every HIGH/MEDIUM gets a required action with a deadline of
  *one builder session*.
- Reviews audit **diffs, not intentions**: read the merged code as if you wrote an adversarial
  test suite for it. Check: does the code do what the commit message and SPEC claim? Does a test
  fail without the change (fail-pre)? Is the oracle mirrored? Is the D-008 tone kept?
- builder-A/builder-B respond in COMMS within one session: fix, contest with evidence, or split
  into a follow-up task. Silence = the finding stands.

---

## 6. CI and invariants

CI (`ci.yml`) runs on every PR and every main push:
`cargo build --release` → `cargo test` (unit + .op proof suite + version gate) → `clippy -D warnings`
→ `cargo fmt --check` → differential harness → redteam suite → LSP smoke.

**Invariants (red = global stop, everyone stops feature work until green):**

| Invariant | Current bar (2026-09-24) | Where checked |
|---|---|---|
| Proof suite | 26/26 files, 281 assertions | `cargo test`, `operon test tests/` |
| Differential oracle | harness 38/38 MATCH, 0 diverge | `python3 bootstrap/harness.py` |
| Redteam containment | 59 contained, 0 breached | `bash scripts/redteam.sh` |
| LSP smoke | full session green | `tests/lsp_smoke.py` |
| Clippy / fmt | 0 warnings / 0 diffs | `cargo clippy -- -D warnings`, `cargo fmt --check` |
| Version gate | Cargo.toml == SPEC == banners | `cargo test` version gate |

Update the table in §10 when the bar moves, that edit is part of the PR that moves it.

---

## 7. File ownership lanes (default; change only by COMMS agreement)

| Path | Owner lane | Notes |
|---|---|---|
| `src/interp.rs`, `src/parser.rs`, `src/lexer.rs`, `src/ast.rs`, `src/genes.rs`, `src/value.rs`, `src/lib.rs`, `src/main.rs`, `src/ls.rs`, `src/ffi.rs`, `build.rs` | builder-A | core semantics; sz may patch bugs it found, with a COMMS note |
| `bootstrap/oracle.py`, `bootstrap/harness.py`, `tests/differential/**` | sz | the differential truth; builder-A mirrors semantics here |
| `tests/**` (proofs, redteam, contract) | claiming agent | fail-pre rule (§2.5) applies to bug fixes |
| `SPEC.md` | sz drafts / builder-A applies | same-PR rule (§2.7) |
| `.github/workflows/**`, `scripts/install.sh` | sz | release engineering |
| `std/**`, `STDLIB.md` | builder-B | .op level only; no new Rust builtins without DECISIONS |
| `examples/cookbook/**` | builder-B | |
| `web/playground/**` | builder-B | |
| `scripts/bench*`, `BENCH.md` | builder-B | |
| `docs/**` (site), `TUTORIAL.md`, `README.md`, `CONTRIBUTING.md` | sz | README factual sections co-owned with builder-A |
| `runtime/codon_kernel.cpp` | builder-A | the C++ codon kernel is load-bearing for wobble + codon scoring; the legacy C kernel was deleted in sec-r2 (audit A15), interning lives in Rust `src/ffi.rs` |

Cross-lane edits: propose in COMMS, wait for the lane owner's 👍 (or the owner's), keep the diff
minimal, credit in the commit body.

---

## 8. Language-specific ground rules (.op)

- `guard (expectation) else { fallback } { body }`, the **second** block runs when the guard is
  TRUE; the first block is the fallback. Re-verify against SPEC before writing proofs around it.
- `/` is float division; `//` is integer division. `%` is **floored** toward negative infinity per
  SPEC §19 divisor-sign rule (`7 % -3 == -2`), do not "fix" this back to truncated.
- Regulation semantics post-T2: `regulate` gates calls via GRN level ≥ threshold (default 0 =
  always-on); `@methylate` accumulates a per-gene counter (default threshold 3 blocks calls);
  `@acetylate` clears it; variant-level `@m6a` selects the variant; `enhance` reduces the
  activating threshold by 0.25; `repressilator` is a driven-oscillator ring with phase lag.
- Every std addition (builder-B) needs: unit proofs, a STDLIB.md row, zero Rust changes.

### 8a. The docs-gate contract (W075), examples are tests are docs

Operon documentation is enforced at three points; "I updated the docs" is not
one of them. The chain: **doc example → executable example → test → rendered
doc.**

1. **Cookbook gate (`scripts/cookbook.sh`)**, every `examples/cookbook/*.op`
   runs against its frozen `examples/cookbook/expected/*.out`, byte-exact, on
   every CI pass. A doc that shows cookbook output that no longer matches the
   program cannot merge: fix the program or the frozen output IN THE SAME PR
   (never hand-edit the .out to silence a failure, re-derive it and diff it).
2. **Apps gate (`operon test apps/`)**, the flagship app's proof frame runs
   in CI like any proof. Docs describing app behavior must survive the app's
   own proofs.
3. **API docs gate (`operon doc` + `scripts/doc_api_check.sh`, W073)**,
   `docs/api/*.md` is GENERATED from the AST (signatures, marks, `##` doc
   comments). The check re-renders and diffs; drift fails. `##` doc comments
   live WITH the code they describe, a signature change without updating its
   doc comment fails the regen-check by construction. Playground-side
   consumers use `operon doc --json`.

Rules of the chain:
- A doc example in README/TUTORIAL/STDLIB must reference a verified program
  (cookbook entry, app, or proof), no copy-paste originals that rot.
- W074 doc comments are the ONLY doc source rendered from the AST; keep prose
  docs (SPEC/BENCH/design notes) OUT of `##` comments (they are API reference,
  not narrative).
- The differential harness doubles as the truth-pass for behavioral claims in
  SPEC: a SPEC sentence describing deterministic output should be able to name
  the differential/proof program that pins it.

---

## 9. Security and capability model

- The interpreter is default-deny for filesystem/process caps (`.cell` grants). Any new builtin
  touching fs/net/process must: require an explicit grant, add a redteam test proving denial and
  containment, and appear in SPEC §9b.
- Fuel limits exist for recursion, spawn, match, JSON depth, uORF, do not add unbounded loops to
  std. Std code runs under the same caps as user code.
- Never weaken a containment test to make CI pass. If a redteam test is wrong, prove it and move it
  in the same PR with the reasoning in the body.

---

## 10. Always know (living facts, re-read every session)

**State**: v2.2.0 tagged at main `0e3d379` (2026-09-24). Milestone "Transcription" complete:
G1 honesty (T2a–T2e) ✓, G2 Cargo+CI ✓, G3 release pipeline ✓, G4 REPL ✓, G5 operon-ls seed ✓.
Next version line: v3.0 "Ribosome VM" (bytecode interpreter, CPython-parity target). v3.5
"Epigenome" is **FROZEN** (D-008) pending owner re-scope, do not plan or build it.

**Repo map** (the 30-second tour):
- `src/parser.rs` (~1.1k lines), grammar, marks, splice variants. Landmark: variant marks were
  hardcoded `m6a:false` until T2c; variant marks now parse.
- `src/interp.rs` (~3.4k lines), execution. Landmarks: `grn_veto` (call funnel for GRN gating +
  enhance), `methyl_levels` counter + gate, `grn_fire` two-phase Hill n=2, repressilator driven
  levels, RISC redirect, NMD sweep, fingerprint/burst index.
- `src/genes.rs`, .cell loading, RNA patches, variant choice, TAD export insulation, managed-tree
  imports, threads (`spawn`/`sequence`).
- `src/tools.rs`, `check` scoring (phantom/wobble/NMD/fallback), `profile`, `crispr`,
  `check_source()` shared by CLI + LSP, `TestReport` (has `asserts` now).
- `src/ls.rs` + `src/bin/operon-ls.rs`, stdio JSON-RPC LSP; hover + diagnostics; zero deps.
- `bootstrap/oracle.py` + `harness.py`, the Python differential truth. **Every builtin must be
  registered in BUILTINS**, wave-3 builtins once shipped dead because of this exact hole.
- `std/*.op`, math, strings, collections, iter, seq, bio. Self-host ratio ~7% and climbing (B2).
- `tests/`, proof files (`operon test tests/`), `redteam/` (59 containment), `differential/`
  (oracle goldens), `repl.rs`, `lsp_smoke.py`, `cargo_proof.rs`.

**Landmines (things that already bit someone):**
- **Terminal display corruption**: an output sanitizer may eat the two-character sequence `[m`,
  so `[main]` can *display* as `ain]`. Before filing any "corrupted text" finding, verify the real
  bytes (print character ordinals via Python: `[ord(c) for c in line]`). One false HIGH finding was
  born this way; do not make it two.
- **`bin/` is gitignored**, the harness wants `./bin/operon`; copy `target/release/operon bin/operon`
  in your sandbox before running it. CI builds via cargo paths instead.
- **cargo not on PATH** in fresh sandboxes: `export PATH="$HOME/.cargo/bin:$PATH"`.
- **The proof suite counts asserts, not just proofs**, when quoting state, quote both
  (`26 proofs / 281 asserts`).
- **Windows**: `/dev/null` doesn't exist; REPL and load paths have a hand-built `Loaded` fallback,
  keep that path compiling (it is why `load_file` errors are caught there).

**Counts that must be quoted exactly** (update on change): proofs 26/26 · asserts 281 · harness
38/38 · redteam 59/0 · clippy 0 · fmt clean.

**Key decisions quick-reference** (full text in DECISIONS.md): D-001 vault = single source; D-004
labels are patch/minor per content; D-008 audience = CS engineers, zero biology, gene words =
intuition only, v3.5 frozen; D-009 2.2.0 = the gated Transcription milestone (2.1.1 never tagged).

---

## 11. Verification cheat sheet

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release                       # core build (build.rs compiles the C++ codon kernel)
cargo test --release                        # unit + proof suite + version gate
cargo clippy -- -D warnings && cargo fmt --check
cp target/release/operon bin/operon         # harness expects ./bin/operon
python3 bootstrap/harness.py                # differential oracle, must end 0 diverge
bash scripts/redteam.sh                     # containment suite, must end 0 breached
python3 tests/lsp_smoke.py                  # LSP session smoke
./target/release/operon test tests/         # proof suite directly (generated counts: docs/STATS.md)
```

For interpreter behavior questions: write a tiny `.op`, run
`./target/release/operon run file.op`, and **read what it actually prints**, never trust memory
over the binary (guard-arm and `/`-vs-`//` surprises above were both caught this way).

---

## 12. Release protocol

1. Release work (`release.yml`) is sz's lane. Tags `v*` trigger the 5-target build matrix
   (linux x86_64+arm64, macOS x86_64+arm64, windows-msvc); **the proof suite gates every target**
   before packaging.
2. Tagging requires: all milestone gates green (TASKS), REVIEWS.md clean-or-waived for the merge
   set, owner's go in COMMS, and DECISIONS.md/README/SPEC version fields already bumped in the
   tagged commit (the version gate in CI enforces this).
3. After tag: verify assets appear (5 archives + 5 sha256), spot-check one sha256, verify
   `scripts/install.sh` URL pattern matches the asset names, then post the release note in COMMS.
4. Never re-tag a version. If a release is broken, fix forward: `vX.Y.Z+1`.

---

## 13. Disagreements, blockers, escalation

- **CI red on main** = global stop for feature work; the reporting agent posts the failure to
  COMMS; nearest-lane agent fixes or the PR author reverts their merge.
- **Design disagreement**: propose a `PROPOSAL:` block in DECISIONS.md. Opposing evidence in one
  session. Owner decides; the decision becomes D-## and is final until the owner revisits.
- **Blocked >1 session**: mark the TASKS item `blocked` with the blocker named, post COMMS, pick
  another task. Do not silently sit on a claim.
- **Found something broken outside your lane**: REVIEWS.md finding + COMMS note. Do not fix across
  lanes unannounced.
- **Owner directives override everything**; when one lands, it becomes a DECISIONS entry the same
  session (that is how D-008 happened).

---

## 14. Growing this document

This constitution is alive. The rule: **every incident that required improvisation becomes a
bullet here, in the same session that resolved it.**

- Anyone may propose an amendment: COMMS note `AMEND: CONTRIBUTING §<n>` + the diff as a PR to
  this file (lane: sz, but any agent may draft).
- The changelog table below is appended, never rewritten.
- §10 (living facts) is updated by whichever PR changes the facts, stale counts in §10 are a
  MEDIUM review finding.

| Date | Change | By |
|---|---|---|
| 2026-09-24 | initial constitution: 12 iron rules, lanes, R-gate, invariants, landmines from the first three-agent week (PR race, display-corruption false positive, dead builtin dispatch, bin/-gitignore trap) | sz |

---

*The point of all of this: the owner should be able to read COMMS.md + TASKS.md + REVIEWS.md in
five minutes and know exactly what state the language is in. If your session made that harder,
you did it wrong.*
