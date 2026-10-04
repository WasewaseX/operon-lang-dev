# SPEC-vs-code audit (S4), 2026-10-02

Auditor: builder-B (S4 pull, dynamic queue). Scope: MASTER-PLAN S4 done-when —
"section-by-section audit (§1–§19): every claim either has a test or is marked
aspirational in a clearly-labeled 'not yet' list; zero unmarked drift."
Baseline: main `44c822d` (v2.7.0), binary `Operon 2.7.0-vm` rebuilt fresh this
session (stale-binary trap honored), probes run on all three implementations
(Rust VM, tree-walk `--interp`, Python oracle) where behavior was contested.

Method, three passes:

1. **Artifact sweep** — every repo path SPEC.md cites (131 unique tokens
   extracted mechanically) verified to exist: 104 real paths all present;
   27 raw hits were pseudo-paths (LSP method names, grammar forms,
   illustrative paths) or globs (`tests/granted/rho_*.op`, 5 files exist).
   One ghost citation found: **S4-1** (below).
2. **Live probes** — 40+ claims exercised against the real binary (+ oracle
   for parity where the claim is cross-engine): precedence, truthiness,
   equality, ordering stress, `num` fallback, sqrt/pow domain, depth limit,
   swap/multi-assign, memory model (§19a), `in`, `??`, ternary, parser
   nesting cap, json depth cap, `--typed` flagship, dx-r6 argv, banner,
   version status, grading top-of-scale, fixer const behavior, proof-runner
   integrity rules.
3. **Corpus mapping** — every section's claims mapped to their named pins
   (130 proof files, 59 differential targets, 112 redteam payloads, e2e
   scripts) and to the SPEC's own honesty markers.

## Findings register

| ID | Section | Finding | Disposition |
|---|---|---|---|
| S4-1 | §13 | Ghost citation: `tests/redteam/rt_p21a_channels.op` — the channel blocking-shape evidence is `tests/redteam/rt_p21a.op` (content verified: close-while-recv, cancel-while-recv, leak storm) | **FIXED in-lane** (SPEC path corrected) |
| S4-2 | §17 | Banner contract stale: SPEC pinned `Operon 2.7.0 (rust-core, cpp-kernel)`; both version surfaces print `Operon 2.7.0-vm (rust-core, cpp-kernel)` since the 2.6.0 A6 VM-default flip (src/main.rs:509,515). Unpinned by any gate — docs-sync checks version numbers, not the banner suffix | **FIXED in-lane** (SPEC text corrected to the printed bytes, suffix explained; core lane may consider a banner pin in a future gate) |
| S4-3 | §9b/§15/§17 | Internal contradiction on the step budget: §9b table + §15 said default 200,000,000; §17 amendment 4 says one run-wide pool of 500M. Code truth: the run-wide pool (500M default) is set unconditionally on the run/repl/debug paths (main.rs:538, 893, 2297) and is the operative budget; interp.rs:902 retains a 200M per-interpreter default that the pool overrides. The main.rs comment (":SPEC §9b: 200M steps per run") cites the stale table while setting 500M | **FIXED in-lane** (§9b row + §15 line truthed to 500M run-wide). The stale main.rs comment is **reported to the core lane**, not touched (lane law) |
| S4-4 | §15 vs §7d | `operon fix` line still advertised `const→let`; the migration is RETIRED (W05, §7d). Live probe: `const_to_let: 0`, `changed: false` on a const program — the field is the always-0 JSON-shape stub | **FIXED in-lane** (§15 fix line now names the retirement; behavior already correct) |
| S4-5 | §10 | **Genuine 3-lane divergence (value level):** `pow(-2, 0.5)` — Rust engines return `nan` as an ordinary Float (f64::powf IEEE semantics); the oracle's Python `**` produces a **complex number** that escapes the oracle's value model and renders as the unmodeled-value fallback `<?>` (oracle.py:473). Zero corpus coverage of the negative-fractional domain (grep `sqrt\(-|pow\(-` over tests/ = empty). Not a crash — a silent value disagreement that the differential harness never sees because no target exercises the shape | **REPORTED to core lane** — one guard in bootstrap/oracle.py (return `float('nan')` when base < 0 and exponent non-integer) + a differential pin; same file and class as W006-D's F1 guard, filed as the W006-D scope extension (F1b). NO SPEC contract sentence for pow's domain until the fix lands — a divergence is never contract-ized |
| S4-6 | §12 | Proof-runner integrity rules (vacuous proof / exited early) implemented in src/tools.rs since W12-era but NEVER negatively exercised — the debug_e2e rot class. Also: a loop-internal `break` is normal completion (the loop consumes Flow::Brk); only a frame-level `break`/`return` exits early | **FIXED in-lane** — new standing gate `scripts/proof_rules_e2e.sh` (positive control + 3 negative shapes with exact runner messages + output-capture check), joined into scripts/test.sh |
| S4-6b | §12 | Runner-contract divergence at the ENFORCEMENT level: the Python oracle's `test` runner does not enforce the two integrity rules (no assert-count check, Flow-swallowing exec_block) — a vacuous or early-returning proof passes silently on the oracle while failing on the canonical Rust runner. Suite-level integrity is unaffected (the Rust side gates), and bootstrap/oracle.py is core-lane | **REPORTED to core lane** (candidate for the W006-D file scope; low priority — enforcement lives on the canonical runner) |
| S4-7 | Status/§17 | Version status stale: SPEC said "Last tagged release: 2.5.0 … 2.7.0 = … untagged"; v2.7.0 is tagged (c4d258f) and released (published 2026-10-01, verified via API) | **FIXED in-lane** (Status line truthed) |
| S4-8 | §10 | sqrt domain under-documented: the negative-argument stress (`unfolded`, "sqrt of negative number") is implemented identically on all three engines (probed) but SPEC §10 said only "→ Float", and zero corpus coverage existed | **FIXED in-lane** — §10 row now states the domain contract; pinned by new proof assertions (tests/l1a_num_builtins.op, 20→24 asserts) + new differential target tests/differential/math_domain.op (byte-identical on all 3 lanes, verified this session) |
| S4-9 | §15 | Opcode table documents 18 of 22 mnemonics on main (BinImm, LoadBinImm, RetName, Nop missing; verified present in src/vm.rs) | **Known drift, repair in flight** — builder-D's W010-A (PR #53) completes the table 22/22 with the bidirectional SPEC↔machine reconciliation gate. Not touched here (no double-fix) |
| S4-10 | §8 | Board note, not SPEC drift: `docs/specs/RESULT-WAVE3.md` (cited by the W006-B queue row) is absent on main because it rides the open W006-A PR #44; it lands with the merge | **No action** (dependency tracked on the board row) |

## Section-by-section verdicts

Legend: **pinned** = named test/corpus evidence verified present; **probed** =
exercised live this session (3-lane where noted); **marked** = the SPEC's own
honesty marker verified present and accurate; **fixed** = drift repaired in
this sweep; **reported** = finding filed to the owning lane.

| § | Claims (abridged) | Verdict | Evidence |
|---|---|---|---|
| 1 Identity | extensions, Total Grammar design law, no-scientist-names law | pinned | corpus-wide (Total Grammar is the parser's standing behavior, wobble_total.op + every suite run); naming law is a repo-wide convention (docs track) |
| 2 Values | 9 kinds; truthiness table; deep `==`; Int/Float numeric compare; incompatible ordering → catchable `unfolded` | pinned + probed | core_values.op, edge_semantics.op; probed 3-lane: truthiness/equality/`in`/ordering stress all agree |
| 3 Lexical | escapes, interpolation, raw/multiline strings, byte strings (W029), char-vs-byte-vs-grapheme table (W28), normalization/casefold stage-2 (generated tables), identifiers, radix forms + `_` separators, saturate-to-0 literals | pinned | tests/unicode.op + unicode_depth.op + differential (byte-identical), tests/bytes.op + differential + rt_p19a_bytes, tests/string_literals.op + differential, tests/numeric_literals.op + differential; generator-verification proof (scripts/gen_unicode_tables.py) |
| 4 Total Grammar | 4-rung ladder, synonym table, edit-distance repair, rung-4 fallback, runtime soft laws, phantom calls | pinned | wobble_total.op; corpus-wide; check's phantom sweep pinned by W101 slice-6 work (typeck/diag goldens) |
| 5 Statements | let/destructuring/multi-define/swap, compound ops, control flow, `match`, `use`, raise forms, stress/rescue, pattern soft-miss law | pinned + probed | l1a_destructure.op, control_flow.op, match_v2.op + differential; probed: swap (RHS-first), short-RHS null, both agree |
| 5a match-v2 | variant/list/map/or/guard patterns, legacy forms, unreachable-arm lint (W02 stage 2) | pinned | tests/match_v2.op + differential/ match_v2.op; lint rules W05/W06 in src/lint.rs (check goldens) |
| 6 Expressions | precedence 1–15, `??` null-only coalescing, `?!` binding, `?.` silent-null | pinned + probed | arith_pinned.op + differential; probed 3-lane: `2**3**2`=512, `-2**2`=-4, `??`, ternary associativity — all agree |
| 7 Genes | marks, guard, closures by reference, defaults, extra/missing args notes, depth 10_000 → overflow, collect | pinned + probed | genes_closures.op, guard_arms.op (differential); probed: depth limit fires `overflow` (3-lane) |
| 7a Phenotypes | init lineage root-first, field defaults, dispatch, type(o), instance equality (deep, class-name-part-of-value) | pinned | phenotypes.op, pheno_equality.op + differential (cycle-safe) |
| 7b Sequences | worker-cell bodies, lazy pull, membrane crossing, yield-outside note, honesty note (oracle buffered) | pinned + marked | sequences.op, seq_gates.op; the honesty note (oracle buffered first-pull) is accurately marked |
| 7c Soft annotations | param/return/let contracts, widening rule, staged model (stage 2 LANDED), W026 graph sugar | pinned + marked | type_anns.op, type_aliases.op, type_generics.op + differentials, std_graph.op; the List<T> inference degrade is accurately marked known-open (W001 REMAIN) |
| 7d const | deep freeze, both walls, frozen stress kind, spawn-boundary rule, `const→let` fix retirement, `let mut` doc-only | pinned + fixed | const_freeze.op + differential; fixer retirement probed live (`const_to_let: 0`), §15 line corrected (S4-4) |
| 8 Modules | resolution roots 1–7, W070 failure classes, W25 namespace spellings, W24 visibility, W19/W20/W23 packages, tads/anchors, cyclic use | pinned + marked | namespaces.op/namespaces2.op/mod_res.op + differentials, visibility_default.op + granted/visibility_strict (.cell), pkg_e2e.sh; roots 4–5 runtime-only is accurately marked (oracle source inspected: base_dir + OPERON_STD only); S4-10 noted (RESULT-WAVE3 rides PR #44) |
| 8b Traits | required vs default methods, contract check at construction, virtual dispatch, redefinition | pinned | traits.op (10 asserts) + differential/traits.op |
| 9 Stress | 4-tier hierarchy, Option/Result constructors + predicates + unwrap law, try_* family (waves 1–2), propagation `?!`, stress kinds table, rescue binding, top-level containment | pinned + probed | try_family.op + differential/try_family.op + try_wave2.op + differential + granted/try_env_wave2; option_result.op + differentials; probed: top-level containment shape (sqrt probe), rc=0 with note |
| 9a Tracebacks | chain capture at call funnel, 64-frame cap, e.chain field order, uncaught rendering, oracle mirror | pinned | traceback_shape.op, differential/traceback_chain.op |
| 9a.1 Error codes | E1xxx stability, --json-errors, caret rules, W101 slices 2/3/5/6, entry validation rc | pinned | scripts/diag_golden.sh + tests/diagnostics/ fixtures (byte-exact) |
| 9b Security | default-deny builtins list, grant flags, symlink/TOCTOU (sec-r5), py bridge + W078/W079 contracts, .cell grant rules, import gating, ceilings table, W32 overflow contract, DAG/monotonic containment, CI pinning | pinned + fixed + probed | security_caps.op, caps_policy.op, rt_p15a–c, rt_p19a/b, overflow_contract.op (both engines, byte-identical), pybridge granted lane + pybridge_off.op; ceilings probed: json 512 ✓, parser 4096 ✓ (§17 #7); step-budget row truthed (S4-3) |
| 10 Builtins | core/random/iteration builtin contracts (incl. the new sqrt domain sentence), method tables, bio-layer boundary note | pinned + fixed + probed | l1a_*.op, l1a_iter_builtins.op, collections.op; sqrt domain now stated + pinned (S4-8); `num` fallback probed; pow negative-fractional = S4-5 divergence REPORTED (never contract-ized) |
| 11 Regulation | §11a contract header/freeze/boundary, gates + order, clamps, entropy discipline, repressilator ODE + kinetics, operon units + rbs + polarity + Rho termination + queue shield | pinned | grn_*.op (9 files), methyl_*.op, m6a_*.op, repressi_*.op, operon_unit.op, quorum_*.op, lac_gate.op, trp_attenuator.op, splice_*.op, silence_*.op, ffl_*.op, granted/rho_*.op (5), rt_p14e; entropy discipline pinned via noise_pin.op + worker_seed_pin.op |
| 12 Frames/proofs | proof frames skipped by run / run by test, completion + ≥1-assert rules, report fields, GENERATED stats | pinned + fixed | docs/STATS.md generated (docs-sync green); the two integrity rules now negatively pinned by proof_rules_e2e.sh (S4-6); oracle-side enforcement gap reported (S4-6b) |
| 13 Concurrency | spawn/join/cancel/task_state, inheritance, task groups, channels + select + wire membrane + stress families + fuel-accounted blocking, scope { } + cancel-on-error, honesty notes (oracle sequential) | pinned + fixed | threads.op, channels.op + differential, cancel.op + differential, task_groups.op + differential, scope.op + differential, timing/ lanes (cancel_timing, wait_any, scope_timing, channel_cross), rt_p20a, rt_p21a (S4-1 path corrected), worker_ring.op; oracle `blocked`-stress honesty note verified accurate |
| 14 Telemetry | fingerprint() keys + bin method, retired v2.0 keys, profile table, memory() gauge boundary | pinned | burst_identity.op, burst_modulate.op, expr_burst.op, burst_bins_pin.op (differential), enhance_boost.op; retired-keys note is accurate (no spliced/unspliced/velocity emitters remain) |
| 15 Toolchain | debug REPL (W08), VM default + bridge + opcode table, CLI verbs, operon-ls surface, grading, --fuel, --strict, stdout discipline | pinned + fixed | debug/protocol/DAP e2es, vm stability tests, differential dual-lane harness, lsp contract (W62 versioning doc); grading probed (clean file → 100/A, --json valid); opcode table 18/22 = S4-9 (PR #53); fix line + --fuel line corrected (S4-4, S4-3) |
| 16a Typed mode | --typed gate (exit 3), T01–T10, flagship T02, inference lattice, aliases, erasure, non-goals | pinned + probed | tests/typecheck.rs (41 cases verified), examples/typed/ good+bad corpus, typeck_e2e.sh; probed: T02 gate exit 3 with the exact finding text |
| 16 Bio map | moved to modeling track (W091) | marked | docs/spec/MODELING-NOTES.md §3 exists; boundary wording present |
| 17 Version | wave-3 hardening amendments (1–8), new surface (regex/time/repeat/join/REPL), semantics pinning block, banner | pinned + fixed | rt_p* ceilings, regex_corpus.op + differential, regex_time.op, std_time.op + differential/time.op, repl.rs (16 asserts: :proof/:load/:genes/:reset), arith_pinned.op; banner + status truthed (S4-2, S4-7) |
| 18 Verification | proof/differential/redteam/playground status, GENERATED counts | pinned | docs/STATS.md regenerated by gate; CI enforces oracle proof parity |
| 19 Memory | scalar copy / container share, args, closure capture, spawn snapshot membrane, cycles + W013 gauge contract, weak refs (W013), engine-truth note | pinned + probed + marked | memory_model.op (differential), weak_refs.op + differential, rt_p22a; probed: scalar copy vs shared handle (3-lane agree); the Rust-lane-only weak-ref shapes are accurately marked as recorded divergence |

## Honesty-marker verification (the "not yet" list is healthy)

Every deliberately-open claim in the SPEC carries its marker, verified present
and accurate this sweep: List<T> sugar degrade (§7c/§16a), typed-collections
mechanical pass pending (§7c), oracle roots 4–5 runtime-only (§8), oracle
buffered sequences (§7b), oracle `blocked` stress (§13), mutexes/atomics open
on W15 (§13), VM-offset breakpoints deferred to A-track (§15), BigInt/Decimal
deferred with std/bigint as the sanctioned hatch (§9b), compatibility
decompositions out of scope (§3), full case folding as casefold (§3),
exhaustiveness not claimed by the W06 lint (§5a), MEM-PROFILER deferred
(§10/§14), tracing GC rejected (§19e). Zero unmarked drift found beyond the
findings register.

## Probe log (this session)

- 3-lane parity (VM / --interp / oracle), stdout+stderr captured separately:
  `scripts/s4_probes/probes_core.op` (40+ claim probes; one oracle divergence
  found → S4-5; note-path stderr shape is the known stdout-only parity
  surface, §18 — not a finding).
- Isolated domain probes: sqrt(-1) → contained stress, statement aborted,
  rc=0, identical on all 3 lanes; pow(-2, 0.5) → S4-5 divergence.
- Caps: 5000-deep expression → rung-4 truncate note, rc=0; 513-deep JSON →
  contained `unfolded`, message names 512.
- Toolchain: `--typed` flagship exit 3 + finding text; dx-r6 `--` argv
  surfaces (`argv()` builtin + std/args helpers) verified end-to-end;
  `operon fix` const neutrality; `operon version`/`--version` banner bytes;
  clean-file grading 100/A + valid --json.
- Corpus: 131 SPEC-cited artifacts checked; 130 proof files, 59 differential
  targets, 112 redteam payloads inventoried against their sections.
| S4-11 | §17 | **S4-2 class REGRESSION on main's 2.8.0 re-pin:** the version-string refresh (0b04c70-era) rewrote the banner claim as `Operon 2.8.0 (rust-core, cpp-kernel)` — no `-vm` suffix — and dropped the suffix-explaining parenthetical, while src/main.rs:545,551 still prints `Operon {version}-vm (rust-core, cpp-kernel)` in BOTH engine variants (live probe: `operon version` → `Operon 2.8.0-vm (rust-core, cpp-kernel)`). Unpinned by any gate — docs-sync checks version numbers, not the banner suffix (same hole S4-2 named) | **FIXED in-lane** during the 2026-10-04 VERIFY-refresh merge resolution (SPEC §17 line corrected to the printed bytes, parenthetical restored, src-line pointer updated; core lane may consider a banner pin in a future gate — the class has now bitten twice) |
