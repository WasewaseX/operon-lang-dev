# CI MIRROR of the R0.10 lock ledger — DO NOT EDIT DIRECTLY
# Canonical: project-vault/collab/LOCKS.md (builder-F, roadmap §23, syncs this
# file in the same session as any ledger change; chatroom posts record both SHAs).
# Parsed by scripts/collision_guard.py (roadmap §24.4).

---

## Active locks

| lock | agent | task | paths | status | claimed | released |
|---|---|---|---|---|---|---|
| L-002 | builder-A | P4-safe | src/interp.rs | ACTIVE | 2026-10-04 (chatroom 14:58 post, amendment-bounded) | — |
| L-004 | builder-B | W006-C2 | examples/result_pipeline.op | RELEASED | 2026-10-04 (PR #78 VERIFY) | PR #78 / 73daded |
| L-005 | builder-B | Q1 | docs/KEYWORDS.md, docs/STATS.md | 2026-10-04 (PR #70 VERIFY) | 2026-10-05 (Q1 landed as PR #70 @ ee54e19 — API-verified MERGED 2026-10-05T06:49:44Z; stats pair landed consistent 167/3562 per B's refresh-9 receipts; keeper release by builder-F — guard had flagged this stale row against PR #127's stats regen) |
| L-006 | builder-B | S4 | docs/SPEC.md, docs/SPEC_AUDIT.md | 2026-10-04 (PR #64 VERIFY) | 2026-10-05 (S4 landed as PR #64 @ 6d21b5d — API-verified MERGED 2026-10-05T06:46:30Z; keeper release by builder-F) |
| L-007 | builder-F | W061-D | scripts/check_composition_pin.py, README.md | ACTIVE | 2026-10-04 (standing checker duty) | — |
| L-008 | builder-F | W061-J | scripts/install.sh, scripts/release_smoke.sh, .github/workflows/release.yml | ACTIVE | 2026-10-04 (release lane) | — |
| L-012 | builder-F | F-CI-INFRA | scripts/redteam.sh, scripts/fuzz_parser.py | RELEASED | 2026-10-05 (coordinator digest-6 assignments: redteam.sh bash-3.2 fix + windows fuzzer TIMEOUT calibration; C veto-at-review) | PR #109 / 5a08fe8 |
| L-014 | builder-C | R0.6 | scripts/validation/**, tests/validation/**, docs/spec/VALIDATION.md | RELEASED | 2026-10-05 (issue #139 + chatroom post; Agent-C opener per roadmap §27); RELEASED 2026-10-05 at landing — PR #140 MERGED rebase 9cccb2f (API truth; coordinator-merged on 15-green + 1 documented skip, 15-file scope == declared Paths, teeth independently reproduced by F session-14, fresh-binary regen 3568) | PR #140 / 9cccb2f |
| L-019 | Super Z | Z-129-FOLDSEM | src/vm.rs | RELEASED | 2026-10-05 (digest-11 claim under owner ALL-word; sweep-3 #129 fold semantics; PR #143). RENUMBERED twice by coordinator ruling R34-A final: L-015 lost to E-SWEEP-STD1 (092b595 18:32Z), L-017 lost to builder-B S3-136H (9abc50d 19:26Z) — L-019 first free | 2026-10-06 RELEASED at landing: PR #143 MERGED rebase 7c1b474 (API truth; F rule-8 independent repro CONFIRMED on record comment 6001934571; CI 14 green + documented nightly skip; scope 2 files == Paths) | PR #143 / 7c1b474 |
| L-015 | builder-E | E-SWEEP-STD1 | std/iter.op, std/json.op, tests/differential/keyword_sweep_std_pin.op | RELEASED | 2026-10-05 (chatroom post; digest-10 ALL-word queue items #136.1 unique + #136.2 json_compact, differential pins ride the fix) | 2026-10-06 RELEASED at landing: PR #142 MERGED rebase 3cc1116 (API truth; F strict review: contracts 3-lane byte-identical, differential 3497/0 + 3489/0, cargo rc=0, CI 14 green + skip; base-refresh rebase after #141 landing to drop patch-identical segment) | PR #142 / 3cc1116 |
| L-018 | builder-E | E-SWEEP-STD2 | std/math.op, tests/differential/ | RELEASED | 2026-10-05 (chatroom post; digest-10 ALL-word queue item #133 isqrt/lcm overflow, division-based rewrites, pin rides) | 2026-10-05 RELEASED-YIELDED post-ruling R34-A: F holds std/math.op first-canonical (L-016/F-SWEEP-MATH1, vault 888775b 19:01Z); E draft PR #145 closed unmerged 19:29Z, division-based fix + 3-lane pin preserved there for F reference |
| L-013 | builder-F | F-128-TRACERACE | tests/grn_trace.rs, tests/sec_regression.rs, tests/profile_spans.rs, tests/rna_v2.rs | RELEASED | 2026-10-05 claim (digest-8 #128 race); RELEASED 2026-10-05 at landing — PR #135 MERGED rebase e220929 (API truth; owner-merged via coordinator, class closure verified W59-green + 4-file scope) | PR #135 / e220929 |
| L-016 | builder-F | F-SWEEP-MATH1 | std/math.op, std/seq.op, std/fmt.op, tests/differential/math_overflow_pin.op | RELEASED | 2026-10-06 (chatroom post; digest-10 ALL-word queue item #133 — full S1+S2 family isqrt/lcm/digits/round_to/fmt_fixed/fmt_thousands/fib_seq, differential pin rides; PR #141) | 2026-10-06 RELEASED at landing: PR #141 MERGED rebase 90b8cc0 (API truth; guard-lesson absorbed: README rider dropped, Paths shrunk; head receipts: differential 3502/0, family probes 3-lane byte-identical, cargo 331/0, CI 14 green + skip) | PR #141 / 90b8cc0 |
| L-017 | builder-B | S3-136H | std/strings.op, std/collections.op, std/set.op, std/bigint.op, std/hashing.op, std/serialize.op, std/binary.op, std/unicode.op, std/result.op, std/csv.op, std/heap.op, std/args.op, tests/differential/s3_136h_pool_pin.op, docs/STDLIB.md, docs/STATS.md, docs/stats.json | RELEASED | 2026-10-06 (draft PR #144 re-scoped; digest-10 ALL-word queue item #136 hardening pool minus E/F-owned files — S1 headlines yielded to E-SWEEP-STD1/L-015; rows 4/5/10/12 excluded as E/F-filed) | 2026-10-06 RELEASED at landing: PR #144 MERGED rebase 27a29a9 (API truth; owner merge-word; F strict review: 22-file scope == claim, pool pin 3-lane rc=0, differential 3501/0 + 3493/0, cargo 331, std_bigint MIN contract 3-lane; CI 14 green + 1 skip) | PR #144 / 27a29a9 |
| L-021 | builder-B | S3-136T2 | std/iter.op, std/json.op, tests/differential/s3_136t2_pin.op, docs/STDLIB.md, docs/STATS.md, docs/stats.json, docs/api/iter.md, docs/api/json.md | RELEASED | 2026-10-06 (draft PR #149 claim-before-edit; #136 rows 3-iter-half/4/5 released by E-SWEEP-STD1 landing; declared next-pull per d602e69) | 2026-10-06 RELEASED at landing: PR #149 MERGED rebase-merge 42844e9 (API truth 12:26:29Z; refresh-1 onto post-release main ab3a3c4 after the release-round stats collision; CI 14 pass + 1 documented skip on 0bbc330; full corpus 3505/0 + 3497/0 both lanes; cargo 331/0; redteam 109/0; pin 3-lane byte-identical re-verified on post-merge main 42844e9 incl. #152 src/value.rs) | PR #149 / 42844e9 |
| L-020 | builder-C | F5 (#49 cargo-fuzz in-process) | fuzz/, .github/workflows/fuzz-inproc.yml, docs/FUZZING.md | ACTIVE | 2026-10-06 (chatroom post; branch reliab/f5-cargo-fuzz-inproc pushed @ 38d219e, draft PR #147; guard green post body-claim patch; L-012 covers scripts/fuzz_parser.py only — no overlap) | #49 |

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
| L-001 | builder-A | P1-batch | src/value.rs, src/vm.rs | 2026-10-04 (chatroom 14:58/16:05 posts) | 2026-10-05 (P1 fix cycle landed on main @ 2bd29b0 — bbf1c67 rework + 5fe1a11 D1 counter + 2bd29b0 slot ownership; owner-merged PR #100 06:29Z; F's independent probe battery green on the fix head pre-merge) |
