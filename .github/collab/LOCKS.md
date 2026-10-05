# CI MIRROR of the R0.10 lock ledger — DO NOT EDIT DIRECTLY
# Canonical: project-vault/collab/LOCKS.md (builder-F, roadmap §23, syncs this
# file in the same session as any ledger change; chatroom posts record both SHAs).
# Parsed by scripts/collision_guard.py (roadmap §24.4).

**What this is:** the mechanical half of multi-agent collision prevention —
the executable ledger behind the §24.3 claim protocol and the §24.4 CI guard
(`operon-lang-dev/scripts/collision_guard.py` + `collision-guard.yml` workflow).

**Keeper:** builder-F (roadmap §23, collab/** upkeep). Ledger changes land as
vault commits; the keeper syncs the CI mirror `.github/collab/LOCKS.md` in
operon-lang-dev in the same session and the chatroom post records both SHAs.

---

## Task/claim format (the contract)

**1. Before touching code (roadmap §24.3, condensed):**

1. Read `collab/TASKS.md` (is the task READY/unclaimed?).
2. Read this file — confirm no ACTIVE row owns an overlapping path.
3. Create the branch, open a **draft PR immediately**.
4. Put the claim in the PR body (format below) and record the lock here.

**2. PR body contract** (parsed by the CI guard; keys are case-insensitive):

```
Task-ID: <task id from collab/TASKS.md or roadmap §34 — e.g. W061-L, P1-batch, R0.10>
Paths: <comma-separated exclusive paths this task touches — files or dir/ prefixes>
```

- `Task-ID` is REQUIRED for any change under a core path (src/, std/, tests/,
  examples/, apps/, bootstrap/, scripts/, docs/, packaging/, web/, labs/,
  .github/, README.md, SPEC.md, and the other top-level docs/Cargo files —
  the full list is `CORE_PATHS` in the guard script).
- `Paths` is advisory-but-expected; it lets the guard warn at DRAFT time,
  before any file changes, when a declared path already belongs to someone else.

**3. Overlap rule:** the first canonical GitHub claim for a path set wins
(§24.3). The guard rejects a changed file covered by an ACTIVE lock whose task
differs from the PR's Task-ID. A conflicting worker stops before editing and
chooses another READY task — or negotiates openly in the chatroom (the owner's
standing note: cross-agent help is endorsed when claimed openly first).

**4. Lock lifecycle:** ACTIVE → RELEASED. A lock releases when the task's PR
merges, when the chatroom records a handoff/close, or by owner override.
Released rows move to the Released table (audit trail, never deleted).
The ledger is self-checked by the guard: two ACTIVE rows covering one path
is a loud ledger defect that fails every PR until the keeper fixes it.

**5. Grandfather clause:** PRs opened before the guard's landing date
(2026-10-05) get WARN instead of FAIL, so the in-flight review queue is
reported but not churned.

---

## Active locks

| lock | agent | task | paths | status | claimed | released |
|---|---|---|---|---|---|---|
| L-002 | builder-A | P4-safe | src/interp.rs | ACTIVE | 2026-10-04 (chatroom 14:58 post, amendment-bounded) | — |
| L-004 | builder-B | W006-C2 | examples/result_pipeline.op | ACTIVE | 2026-10-04 (PR #78 VERIFY) | — |
| L-005 | builder-B | Q1 | docs/KEYWORDS.md, docs/STATS.md | ACTIVE | 2026-10-04 (PR #70 VERIFY) | — |
| L-006 | builder-B | S4 | docs/SPEC.md, docs/SPEC_AUDIT.md | ACTIVE | 2026-10-04 (PR #64 VERIFY) | — |
| L-007 | builder-F | W061-D | scripts/check_composition_pin.py, README.md | ACTIVE | 2026-10-04 (standing checker duty) | — |
| L-008 | builder-F | W061-J | scripts/install.sh, scripts/release_smoke.sh, .github/workflows/release.yml | ACTIVE | 2026-10-04 (release lane) | — |
| L-012 | builder-F | F-CI-INFRA | scripts/redteam.sh, scripts/fuzz_parser.py | ACTIVE | 2026-10-05 (coordinator digest-6 assignments: redteam.sh bash-3.2 fix + windows fuzzer TIMEOUT calibration; C veto-at-review) | — |

Notes:
- **Not locked (claims invited, not yet ACKed in the room):** P2/P3
  (profiling lane, builder-E invited 2026-10-04 14:58 — P2 executed via G9
  announcement and merged as PR #96; P3 claim announced 2026-10-04 16:45),
  S7 security re-aim (owner ratification), W006-E (owner ratification).
  Compat-matrix/environment-pair: REASSIGNED to builder-F 2026-10-04
  (digest-4, C veto-at-review) — claimed and executed same day as PR #94.
- The perf lanes interleave on src/interp.rs and src/vm.rs frequently; the
  P1/P4 locks above cover the ACTIVE claim windows only — landed work
  releases locks the same day (see Released table). Keep claims narrow.

## Released locks

| lock | agent | task | paths | claimed | released |
|---|---|---|---|---|---|
| L-010 | builder-A | fmt-P0 (W47-v3) | src/tools.rs | 2026-10-04 | 2026-10-04 (main d5483a6) |
| L-011 | builder-A | W011-s3 salvage (PR #90) | tests/differential/bk_slots_pin.op | 2026-10-04 | 2026-10-04 (merged, arbitration digest-3) |
| L-003 | builder-A | P5-P6-survey | docs/bench/ | 2026-10-04 (chatroom 15:40 post, measure-only) | 2026-10-05 (survey merged as PR #92 @ 52830cd — fix phases re-claim per survey-then-fix) |
| L-009 | builder-F | R0.10 | collab/** (vault), .github/collab/**, scripts/collision_guard.py, .github/workflows/collision-guard.yml | 2026-10-05 (this ledger) | 2026-10-05 (CI half merged as PR #93 @ c1e5039 — keeper sync continues under WORKER-BEHAVIOR rule 7) |
| L-001 | builder-A | P1-batch | src/value.rs, src/vm.rs | 2026-10-04 (chatroom 14:58/16:05 posts) | 2026-10-05 (P1 fix cycle landed on main @ 2bd29b0 — owner-merged PR #100 06:29Z; F's independent probe battery green on the fix head) |
