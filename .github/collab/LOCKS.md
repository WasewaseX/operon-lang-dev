# CI MIRROR of the R0.10 lock ledger — DO NOT EDIT DIRECTLY
# Canonical: project-vault/collab/LOCKS.md (builder-F, roadmap §23, syncs this
# file in the same session as any ledger change; chatroom posts record both SHAs).
# Parsed by scripts/collision_guard.py (roadmap §24.4).

# OPERON LOCKS — active path ownership (R0.10, APPROVED 2026-10-04)

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
| L-002 | builder-A | P4-safe | src/interp.rs | RELEASED | 2026-10-04 (chatroom 14:58 post, amendment-bounded) | 2026-10-06 RELEASED at landing: PR #152 MERGED rebase-merge eb9aa25 (API truth 12:25:46Z). Independent F strict-review receipts on 0b4c37d: diff line-audited (position_str memo hit deep_eq-verified, Str-specialized exact-scan collision fallback incl. the fnv1a("2")==fnv1a(Float(2.0)) craftable shape, miss-trust absence unchanged; member_value Map+Obj arms rewired, missing-notes preserved); fresh release build rc=0; differential 3,497 x 2 lanes 0 diverge (p4_member_read_pin.op rides); same-binary interleaved bench reproduced A's claims: late 10,425.4 -> 520.6 ns = 20.0x, miss 3,819.6 -> 782.0 = 4.9x, controls early/obj1/obj8 flat within ~1%; CI 14 success + 1 documented skip, mergeable CLEAN | PR #152 / eb9aa25 |
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
| L-022 | builder-C | R0.7 (claim/evidence schema) | docs/spec/CLAIMS.md, bootstrap/claim_registry.json, scripts/check_claims.py, scripts/test.sh | ACTIVE | 2026-10-06 (chatroom post + TASKS row; R0.6 follow-on per §27 verification/science lane; no overlap with L-007/L-008/L-020/L-021/L-002) | — |
| L-020 | builder-C | F5 (#49 cargo-fuzz in-process) | fuzz/, .github/workflows/fuzz-inproc.yml, docs/FUZZING.md | ACTIVE | 2026-10-06 (chatroom post; branch reliab/f5-cargo-fuzz-inproc pushed @ 38d219e, draft PR #147; guard green post body-claim patch; L-012 covers scripts/fuzz_parser.py only — no overlap) | #49 |
| L-023 | Super Z | Z-134-RANGEGUARD | src/interp.rs, tests/differential/range_step_wrap_pin.op | RELEASED | 2026-10-07 (chatroom digest-14 post; digest-10 ALL-word queue item #134 — S1 builtin range step-wrap, differential pin rides; unclaimed per F session-16 scan + room ruling; the lazy for-loop site is already guarded by the P5 wave, the materialized builtin is the remaining site) | 2026-10-08 RELEASED at landing (keeper repair): PR #164 MERGED rebase 49bc669 — the W071 mirror sync recorded the release in the REPO mirror but the vault-side flip was lost in the parallel-lane union; row otherwise verbatim |
| L-024 | Super Z | Z-130-EXACT2 | src/num_exact.rs, src/interp.rs, src/vm.rs, src/lib.rs, tests/num_exact.rs, tests/differential/int_float_exact2_pin.op, docs/stats.json, docs/STATS.md | RELEASED | 2026-10-08 (digest-16 follow-up: my L-023 post declared #130 next in my queue after #134 lands; #134 landed via PR #164 — claim exercised; sweep-3's LAST remaining item; no ACTIVE lock overlaps: L-007/L-008 are F's standing checker/release duties, all others RELEASED) | PR #165 — head advanced 650d8d6 -> 16a9bdb (rebased onto v2.9.3 main) -> 5f6a0e3 (the parallel session's correct stats-3580 fix, force-pushed to my branch, adopted) -> 785a10f (the session-W generated CPython ground-truth table folded in, credited, 339/339 bit-exact on both implementations) — merge on required-legs green |
| L-026 | Super Z | Z-131-FLOORMOD | src/num_exact.rs, src/interp.rs, bootstrap/oracle.py (AMENDED at PR open: the oracle's float % arm carried the same hybrid bug and the i64::MIN corner misfire — moved with the law per the declared-before-landing rule), tests/num_exact_mod.rs, tests/differential/float_mod_floored_pin.op, docs/stats.json, docs/STATS.md | RELEASED | 2026-10-08 (digest-17 addendum follow-up: my #130 numeric-exactness territory; float % + divmod floored law via the CPython fmod+sign-adjust algorithm — the issue's suggested floor formula breaks on infinite divisors vs Python; no ACTIVE overlaps: L-024 touches interp Div/compare, this touches interp Mod/divmod) | 2026-10-08 RELEASED at landing: PR #167 rebase-merged as eaf089b (CI 15/15 green incl. the windows required leg via the NaN-payload contract fix 8abef0c and the collision guard after 4 real-ledger-defect reds; scope 7 files == amended Paths; 3028 mod + 1905 divmod rows bit-exact vs CPython) |
| L-025 | builder-B | W036-freeze-lint (D-008 follow-up) | scripts/check_docs_sync.py, docs/specs/CORE-BIO-BOUNDARY.md, TODO-100.md | RELEASED | 2026-10-08 (worklog session-35; the W036-named follow-up: automated lint diffing parser KEYWORDS+marks against the frozen inventory; no overlap with L-007/L-008/L-020/L-022/L-023/L-024 — docs/specs/ is not docs/spec/, and test.sh is not touched) | 2026-10-08 RELEASED at landing: W069 MERGED direct to main @ 85a0d38 (push API truth 29e2d2b..85a0d38, WasewaseX identity verified on the head commit; gate receipts in the commit message: positive run green 60kw/7marks, negative tests crossing/vanished/count-drift/mark-drift all fire; CI watch noted in session-35 worklog) |
| L-027 | Super Z | Z-R09-LOWERING | docs/spec/LOWERING.md, scripts/check_lowering.py, docs/spec/MODELING-NOTES.md, docs/spec/BIO-CONTRACT.md, docs/specs/CORE-BIO-BOUNDARY.md | RELEASED | 2026-10-08 (digest-19 claim under the owner directive "do some of the bio roadmap"; roadmap R0.9 lowering/abstraction contract, §34 APPROVED 2026-10-04, §27 proposed for builder-D's language/DX lane which has not pulled it — coordinator executes under the digest-4 team-note precedent with C/D veto-at-review; ZERO src/ delta, docs+checker only; no ACTIVE overlaps: L-020 is fuzz/, L-022 is CLAIMS.md+claim_registry+check_claims.py+test.sh — scripts/test.sh deliberately NOT in my Paths; scripts/check_lowering.py vs L-022's scripts/check_claims.py are disjoint files) || 2026-10-08 RELEASED at landing: PR #168 MERGED rebase 8214e15 (API truth; CI 15/15 — zero fails, one documented nightly-fuzzer skip; windows required leg + all 6 compat matrix legs green; guard green on the pre-synced mirror c8e75ab; scope 5 files == declared Paths via API files endpoint; check_lowering 32kw/41rows/23MN + selftest 4/4; docs-sync fast OK, doc versions OK, doc_api_check rc=0; ZERO src/ delta — Rust battery transfers 1:1; composition pin 429,533/60,093/9,573 verified untouched) |
| L-030 | Super Z | Z-R04-NUMPOLICY | docs/spec/DETERMINISM.md | RELEASED | 2026-10-08 (digest-19 follow-up claim; roadmap R0.4 deterministic numerical policy — its §5 amendment restates the sweep-3 numeric laws as policy: the num_exact shared-law module, the fold-semantics contract (Z-129), the generated-CPython-referee-table house pattern, the payload-blind NaN platform law; docs-only rider completing R0.4; no ACTIVE overlaps — B's L-028 is src/typeck.rs+ls.rs lane, B's L-029 is the R0.1 files; RELEASED at landing: PR #171 rebase-merged as 326f7d45, 15/15 CI green, 1 file == Paths, docs-only) | 326f7d45 |
| L-031 | Super Z | Z-137-SORTSTABLE | src/interp.rs, bootstrap/oracle.py, tests/differential/p2p3p5_opt_pins.op, tests/differential/sort_stability_pin.op, SPEC.md, docs/stats.json, docs/STATS.md | RELEASED | 2026-10-09 (R35 session, owner directive "countinue never stop" — sweep-3 unclaimed queue per digest-18/19; issue #137 S2 comparator-sort stability: both engine paths (insertion <=32, merge >32) swap equal-key pairs, oracle builtin path deliberately mirrors the unstable walk while the oracle method path (cmp_to_key) is Python-stable — so method-form ties DIVERGE engine-vs-oracle TODAY; fix = one stable rule at the shared comparator_sort choke point (swap/take-right iff cmp(second, first) strictly-before), oracle builtin+method paths mirror the same walk byte-for-byte (also unifying side-effect comparator call order across all four surfaces), P2 pin regenerated to the stable law, NEW stability pin (dup-key records, multi-key sort trick, both path regimes, call-count row), SPEC list-methods line amended to pin stability; no ACTIVE overlaps: src/interp.rs free (L-001 RELEASED), bootstrap/oracle.py free, SPEC.md free — L-020 is fuzz/, L-022 is CLAIMS+check_claims paths) | — |
| L-032 | Super Z | Z-115-LEXZERO | src/lexer.rs, src/parser.rs, tests/differential/lex_zero_pin.op, docs/stats.json, docs/STATS.md | ACTIVE | 2026-10-09 (R35 continuation, sweep-2 unclaimed queue; issue #115 S1: the three numeric lexer error arms note "treated as 0" but push NO token while the oracle pushes INT 0 — live differential break (rust empty line/null vs oracle 0/0/0) and the parser Null-substitution fallback consumes Newline, merging the next statement into the broken one; fix = push Tok::Int(0) in all three Err arms (radix overflow, malformed float, decimal overflow — oracle-parity incl. INT-0-for-malformed-float) + the parse_primary fallback never consumes Tok::Newline (reserved for the statement layer) + differential pin for each shape; no ACTIVE overlaps: L-031 is interp.rs+oracle.py (mine, PR #174 open — disjoint files), A's Z-SWEEP4-S0-PARSER touched parser.rs but that push LANDED (de9d493) and its follow-up options are src/tools.rs or tests/fix_corpus.rs, not the lexer arms; RELEASED at landing: PR #174 rebase-merged (1b5c47c + stats 3c41534), 7 files == Paths, issue #137 auto-closed via the body keyword) | 1b5c47c |
| L-033 | Super Z | Z-SWEEP4-FIXCORPUS-CONTAIN | tests/fix_corpus.rs | ACTIVE | 2026-10-09 (S0 incident unblock per the coordinator standing offer in the cfe5beb incident post — A has not surfaced and main is red on 6 legs; option (b) the containment precedent: law1_fix_never_changes_canonical_meaning_corpus_wide containment-checks the two parser depth pins (their fix_source-transformed output re-parses deeper than any stack and a stack overflow is an uncatchable process abort — the raw parse+format pipeline passes in a 2MB thread, probe-proven), asserting their parse-side contract loudly instead; vm_parity rt_p4b containment precedent; A option (a) fixer depth-discipline remains open and supersedes — this patch reverts when (a) lands; C veto-at-review; single file, zero src/ delta) | — |

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
| L-028 | builder-B | W45-v3 (inlay hints) | src/typeck.rs, src/ls.rs, src/bin/operon-ls.rs, tests/ls_inlay_hints.rs, tests/lsp_smoke.py, SPEC.md, README.md, docs/specs/LSP-VERSIONING.md, TODO-100.md | RELEASED | 2026-10-08 (board-truthing session: the W45 goal list's inlay-hints item is the last unclaimed LSP-breadth piece; PR #169 open; no ACTIVE overlaps — src/ls.rs + src/bin/operon-ls.rs are covered by no other row, L-007/L-008/L-020/L-022 are F/C standing lanes on other paths) | PR #169 |
| L-029 | builder-B | R0.1 | docs/spec/BIO-CORE-IR.md, std/biocore.op, tests/biocore.op, tests/differential/biocore_pin.op, STDLIB.md, docs/api/biocore.md, docs/api/url.md, docs/stats.json, docs/STATS.md, docs/coverage.md | RELEASED | 2026-10-08 (task cell normalized to the bare Task-ID per the b61628c lesson; the parenthetical '(Bio-Core IR)' lives here in the claim note; owner directive post 17:0x UTC assigns @builder-B R0.1; coordinator's 18:1x post declares it unblocked with the R0.9 vocabulary — IR rows cite LOWERING.md §1 classes + MN keys; zero src/ delta, pure-.op C4 PoC; no ACTIVE overlap: docs/spec/BIO-CORE-IR.md is a NEW path distinct from L-027's LOWERING.md/MODELING-NOTES.md/BIO-CONTRACT.md, std/*.op + tests/ are my lane defaults) | PR pending |
