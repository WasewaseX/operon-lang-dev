# Biological lowering + abstraction contract (R0.9)

Roadmap row R0.9 (§34 APPROVED 2026-10-04; §27 language/DX deliverable,
coordinator-executed under the digest-4 team-note precedent). Normative for
every biological mechanism Operon carries today or adds later. Precedence:
SPEC.md wins, then BIO-LAYER-POLICY.md (the freeze law), then this file, then
CORE-BIO-BOUNDARY.md (the inventory), MODELING-NOTES.md (the term audits) and
BIO-CONTRACT.md (the honesty labels). Conflicts between the subordinate files
resolve upward, never sideways.

The problem this contract closes: the language grew its biology over many
waves (reg-bio .. reg-r4, loop-9, A1/A7, F-1..F-8, C1..C11), and each wave
documented its mechanism locally. Nothing states the SHARED rule for how a
biological surface becomes executable semantics — so nothing stops the next
mechanism from requiring private semantics in every engine. R0.9 is that
shared rule.

## 0. The two laws

**Law L1 — one lowering, declared.** Every biological surface (keyword, mark,
cell key, builtin, module function) lowers into a defined CORE surface —
environment records, value fields, gate consults, table lookups, seeded noise
draws — and the lowering is stated once, here, in §2. A mechanism whose
lowering cannot be stated in core terms does not enter the language; it stays
a stdlib module (class C4) or research material (class C9) until it can.

**Law L2 — engines implement the lowered form, never the biology.** The
tree-walk interpreter, the VM, and the Python oracle each implement the
LOWERED form of L1. No engine carries a private biological model. Where the
lowered form is deterministic, engines are byte-parity-pinned by the
differential harness; where it is seeded-stochastic, the SEED REPLAY is
byte-parity-pinned (the telegraph precedent). A bio feature without its pin
is not landed, regardless of how correct one engine looks.

## 1. Abstraction classes (the nine)

The roadmap's menu — "a biological mechanism may become core syntax, a
declarative construct, a type-system concept, a stdlib abstraction, a runtime
mechanism, a profiler primitive, a native kernel, a validation benchmark, or
research material" — is closed. Every surface maps to exactly one primary
class below. The class is chosen by computational value, never biological
prestige; D-008 voice applies to every class (programmer-first naming, zero
biology required to use the language).

**C1 — core syntax (FROZEN).** Reserved keywords and their grammar. Carries
evaluation semantics in every engine. Post-freeze (D-008): CLOSED to growth;
the 25 grandfathered bio keywords are back-compat obligations, not growth
room. Examples: `regulate`, `splice`, `fate`, `operon`, `repressilator`.

**C2 — declarative marks.** `@name` annotations attached to declarations.
The mark is a record the runtime consults at a defined consult-point (§2);
it never changes parse structure. Adding a mark is a semantic change (it
must lower per L1) but NOT new grammar — the `@` machinery already exists.
Examples: `@methylate`, `@acetylate`, `@m6a`, `@copies`, `@riboswitch`.

**C3 — type-system concept.** The mechanism becomes a typing, checking, or
inference rule. Empty today by design (the type layer stays deliberately
thin); a future bio mechanism lands here only with a typeck design note and
DECISIONS entry.

**C4 — standard-library abstraction.** `std/*.op` modules and library genes.
The DEFAULT landing zone for new biological computation post-freeze (W036
Rule 2: libraries first). Examples: `std/bio.op`, `std/motifs.op`,
`std/seq.op`, `std/random.op` (the R0.5 deterministic RNG).

**C5 — runtime mechanism (.cell-configured).** An engine capability whose
behavior is present in every engine but whose activation/tuning rides
`.cell` configuration — the mechanism lowers to runtime state (flags, pins,
caches) read at defined consult-points. Examples: `telegraph` promoter
layer, `decay_clock` override, Rho termination pins, quorum species,
ribosome-queue shield, `io.pool = fiber`.

**C6 — profiler / analysis primitive.** The mechanism exists to be measured,
not executed: `memory()` accounting, the NMD sweep's findings, stack report,
`operon check` graders. Lowers to instrumentation hooks; parity is claimed
on the FINDINGS, not on timing.

**C7 — specialized native kernel.** C/C++ compiled kernels behind the C ABI
for genuinely hot paths, exposed through builtins/stdlib. The kernel and the
Rust fallback must agree on outputs (the smoke gate compiles and runs the
kernel; the differential harness pins the builtin's behavior). Examples:
`rt_edit_distance` (the wobble engine's scorer, `distance()`/`similar()`),
`rt_codon_score` (the `codon()` builtin).

**C8 — validation benchmark.** The mechanism exists as a known-answer or
known-behavior workload for the verification lanes: R0.6 analytic fixtures,
repressilator reproduction tests, V-row models. A "mechanism" that lives
only in fixtures is still a legitimate landing class — it computes value
without touching engine semantics.

**C9 — research-only material.** Term audits, not-modeled lists, rejected
analogies, the deep-research synthesis. Lives in MODELING-NOTES.md §2 and
the vault; carries ZERO runtime obligation. Guarded by check_docs_sync's
marker discipline (a `[MN-*]` key must resolve to its audit).

Growth rule (the one-line version of §3): post-freeze, new mechanisms enter
at C4 first, C5 second, C2 only with a lowering note in this file, C7 only
with a hot-path measurement, and NEVER at C1.

## 2. Lowering rules (the frozen inventory, classified)

Every live biological surface, its class, and its lowered form. Engine locus
names where the lowered form lives (tw = tree-walk interpreter, vm = VM,
py = Python oracle — all three when the surface is pinned). Labels cite
BIO-CONTRACT.md's vocabulary. MN keys cite MODELING-NOTES.md §2.

### 2.1 GRN family (class C1, frozen)

| surface | class | lowered form | locus | label | MN key |
|---|---|---|---|---|---|
| `regulate { a activates/inhibits b (strength, threshold) }` | C1 | GRN edge table (source, target, sign, weight); on gene activation the runtime consults the table, computes the gated level via dose-response `s·p²/(p²+t²)`, propagates ≤ 10 waves at `strength^wave`, applies inhibition once per fire | tw, vm, py | REAL (direction) + APPROX (wave math) | MN-regulate |
| `enhance g` / `enhance g, n` | C1 | lowers the named gene's gate threshold (cluster = super-enhancer analogy); a level mutation on the GRN record, not a call-path change | tw, vm, py | ABSTRACTION | MN-enhance |
| `silence g` (RISC) | C1 | stoichiometric suppression record: the target's next `n` activations are consumed by the silencing pool before the gate fires | tw, vm, py | REAL (targeted degradation) + APPROX (guides consumed) | MN-silence |
| `decoy site` / `bind site` | C1 | interference-absorbing table rows: decoys absorb interference before it reaches real sites (C11 crossfire analog) | tw, vm, py | ABSTRACTION | MN-decoy |
| `toggle` | C1 | two-state GRN edge pair with hysteresis — sugar over a mutual-inhibition edge pair | tw, vm, py | ABSTRACTION | MN-toggle |
| `occupy` | C1 | site-occupancy record on the binding-site eval path: occupied sites gate competing effectors (D2b) | tw, vm, py | APPROX | MN-occupy |
| `attenuates` | C1 | graded edge modifier on an existing regulation edge (A5) | tw, vm, py | APPROX | MN-attenuates |
| `translates` (verb in `regulate`: `a translates p rate r decay d`) | C1 | protein node in the GRN level map integrating one Euler step of the two-tier ODE at every update point (`p += rate·Δcalls − decay·p`, clamped 0..1); gates may regulate on PROTEIN, giving the two-tier delay signature | tw, vm, py | REAL (two-tier delay) + APPROX (Euler) | MN-translates |

### 2.2 Signal pools (class C1, frozen)

| surface | class | lowered form | locus | label | MN key |
|---|---|---|---|---|---|
| `ligand` / `inducer` / `cofactor` / `autoinducer` | C1 | named values in the global signal pool; edges read pool entries at consult time; `autoinducer` additionally feeds the quorum layer (2.5) | tw, vm, py | ABSTRACTION | MN-quorum (pool side: term audits reg-bio) |

### 2.3 Marks (class C2)

| surface | class | lowered form | locus | label | MN key |
|---|---|---|---|---|---|
| `@methylate` / `@acetylate` | C2 | integer level record (+1 per application, saturating −1) on the declaration; consulted at the NEXT call's gate; `methylate()/demethylate()` mutate the record | tw, vm, py | REAL (direction) + ABSTRACTION (levels) | MN-@methylate, MN-@acetylate |
| `@m6a` | C2 | level 0..=3 record; dispatch priority + redefinition resistance read it (reg-r4); writers are `m6a_write/m6a_erase` (2.5) | tw, vm, py | REAL (stability effects) + APPROX (levels) | MN-@m6a, MN-m6a-write, MN-m6a-readers |
| `@copies n` | C2 | N-way expansion of the declaration into distinct instances at bind time (C10); level hygiene (D2c/D7/D9) keeps marks per-instance and determinism hardening applies to the expansion order | tw, vm, py | ABSTRACTION | MN-@copies, MN-hygiene |
| `@riboswitch` | C2 | cis ligand gate on the marked gene's own transcript: the gate reads the ligand pool entry the transcript is defined against; the cis requirement is enforced (loop-9 B4) | tw, vm, py | REAL (cis aptamer) + APPROX (level switch) | MN-@riboswitch |
| `@burst(kon, koff)` | C2 | per-gene bursty-expression parameters (defaults 0.3/0.1, clamped 0..=1) riding the telegraph promoter layer: the mark lowers to that gene's two-state promoter rates; burst telemetry feeds fingerprint() | tw, vm, py | REAL (bursts) + APPROX (two-state, seeded) | MN-telegraph |
| `@deprecated(...)` (W64, non-bio precedent) | C2 | pure metadata; tooling owns behavior (`check` warning W12) — the neutrality pattern every pure mark must match | tw (neutral), check | ABSTRACTION | — |

### 2.4 Variants, machines, entry (class C1, frozen)

| surface | class | lowered form | locus | label | MN key |
|---|---|---|---|---|---|
| `splice` / `variant` / `replace` | C1 | variant table on the gene record; `choose_variant` selects the active implementation at call time (splice fuel charges; site syntax is text-level, loop-9 F-4 adds splice_shift) | tw, vm, py (genes.rs) | REAL (exon joining) + ABSTRACTION | MN-splice_shift (+ term audit) |
| `fate` / `state` / `enter` | C1 | a plain state machine on a value: states are strings, `enter` names the initial state, transitions are ordinary calls (SPEC §11 — no hidden semantics) | tw, vm, py | ABSTRACTION | term audit reg-bio |
| `ires` | C1 | entry flag on the program: cap-independent run entry bypassing the 5'-cap gating step (binary on/off) | tw, vm, py | REAL (bypass) + ABSTRACTION (binary) | term audit |
| `edit` | C1 | load-time `.rna` patch verb: `apply_rna_checked` rewrites source before parse (counted, reported); NOT a runtime mechanism — lowers to a transformed AST, and parity holds post-patch | tw (load), py (load) | ABSTRACTION | term audit |
| `operon { a, b, c }` (polycistronic) | C1 | one promoter record fanning activation to all listed genes per firing (A1/A7 co-transcription); Rho/attenuation interplay via 2.5 mechanisms | tw, vm, py | REAL (co-transcription) + ABSTRACTION | MN-operon-unit |
| `repressilator` | C1 | sugar expanding to the published 3-gene ring (Elowitz–Leibler) with parameters α/β, basal leak, and the deterministic-noise mode for byte-parity | tw, vm, py | REAL (topology) + APPROX | MN-repressilator |
| `period`, `telegraph` parameters | C1/C5 | oscillator phase parameter; the telegraph two-state Markov promoter charges seeded noise into expression (F-1/F-2/F-3) | tw, vm, py | REAL (bursts) + APPROX (seeded) | MN-telegraph |

### 2.5 Runtime mechanisms (class C5, .cell-configured)

| surface | class | lowered form | locus | label | MN key |
|---|---|---|---|---|---|
| telegraph promoter layer | C5 | runtime state: seeded two-state promoter per gene (virtualized where the lane is deterministic); activation draws ride the seeded stream | tw, vm, py | REAL (bursts) + APPROX (two-state, seeded) | MN-telegraph |
| `decay_clock` builtin | C5 | live-read runtime override `decay_clock_n`/`decay_clock_f` on the decay rate consult (n=0 = off); the cell-derived booleans cache, the clock does not | tw, vm, py | ABSTRACTION | MN-decay_clock |
| Rho termination | C5 | termination pressure grows with naked runway: catch probability `1−(1−catch)^d` consulted during transcription-long operations; pins cached per cell (loop-10 W2 direction fix) | tw, vm, py | REAL (direction) + APPROX (probability) | MN-operon-unit (F-7) |
| ribosome-queue shield | C5 | queued ribosomes reduce the decay rate of the transcript while the queue is live (F-8); a decay-path multiplier, not a separate process | tw, vm, py | REAL + APPROX | MN-operon-unit (F-8) |
| quorum sensing | C5 | population = worker/toggle layer; density-dependent switch reads the autoinducer pool species (C8); no diffusion/geometry | tw, vm, py | REAL (density switch) + ABSTRACTION | MN-quorum |
| `m6a_write` / `m6a_erase` | C5 | quantitative site-density writes under the Dam-style analogy (B3); readers (F-6) apply directional stability/translation effects | tw, vm, py | REAL (effects) + APPROX (density) | MN-m6a-write, MN-m6a-readers |
| `passage` | C5 | epigenetic inheritance across explicit passage events (B2/B6): mark levels snapshot/restore under the passage protocol | tw, vm, py | ABSTRACTION | MN-passage |
| uORF fuel | C5 | upstream-ORF repressive fuel charge on the translation path (rt_p2f) | tw, vm, py | REAL (repression) + APPROX | term audit |
| fiber lane (`io.pool = fiber`) | C5 | non-bio PRECEDENT for the class: a .cell key selects the execution substrate; determinism rides the virtual clock — the pattern bio C5 rows follow | tw, vm | ABSTRACTION | — |

### 2.6 Native kernels (class C7)

| surface | class | lowered form | locus | label | MN key |
|---|---|---|---|---|---|
| `rt_edit_distance` | C7 | bit-parallel edit-distance wavefront behind the wobble engine (rung 3) and `distance()`/`similar()`; single-word ≤ 64 O(n), block path beyond; Rust fallback must agree | vm (parse), std | ABSTRACTION (naming aid) | — |
| `rt_codon_score` | C7 | identifier read as 3-letter groups scored against standard genetic-code usage weights → `codon()` builtin + `check` grader; Rust fallback must agree | tw, vm, check | ABSTRACTION | — |

### 2.7 Standard-library bio (class C4)

| surface | class | lowered form | locus | label | MN key |
|---|---|---|---|---|---|
| `std/bio.op` | C4 | sequence utilities layered on the native kernels (codon usage counting, etc.) — ordinary Operon code, no engine privileges | std | ABSTRACTION | — |
| `std/motifs.op` | C4 | motif search over sequences; pure library code on core data structures | std | ABSTRACTION | — |
| `std/seq.op` | C4 | sequence operations (the post-freeze growth surface for sequence computation) | std | ABSTRACTION | — |
| `std/random.op` (R0.5) | C4 | mirrored xorshift64* with stable state format and seeded streams — the deterministic-RNG substrate every stochastic bio mechanism MUST draw from (no second RNG may appear) | std, py | ABSTRACTION | — |

### 2.8 Analysis + validation (classes C6/C8)

| surface | class | lowered form | locus | label | MN key |
|---|---|---|---|---|---|
| NMD sweep | C6 | static finding pass over the program: called/enhanced sets → premature-stop purge candidates (`purge_premature_stops`); parity claimed on findings | tw, vm, py | REAL (NMD analogy) + ABSTRACTION | term audit |
| `memory()` accounting | C6 | allocation-accounting hooks (W097 SPEC §10 pin); numbers are contracts, not timings | tw, vm | ABSTRACTION | — |
| R0.6 analytic fixtures | C8 | known-answer models with independent reference calculations (VALIDATION.md registry); the standard for any future stochastic R-row | tests/validation, py | ABSTRACTION | — |
| repressilator reproduction tests | C8 | published-behavior reproduction pins (`tests/repressi_params.op`, `tests/repressi_alpha.op`) | tests | REAL (reproduction) | MN-repressilator |

Anything not listed here and not addable under §3's ladder does not lower —
it is C9 research material by default.

## 3. Reserved-word policy (how keyword growth is controlled)

The freeze (W036/D-008, stated in BIO-LAYER-POLICY.md as Rules 1–2 and
inventoried in CORE-BIO-BOUNDARY.md) is restated here as the R0.9 growth
ladder. When a new biological mechanism is proposed, it enters at the
LOWEST acceptable rung, and each rung up requires strictly more process:

1. **Library function (C4).** `std/*.op` module or library gene. No engine
   delta, no parity obligation beyond the module's own tests. This is the
   default and usually the end.
2. **`.cell` configuration on an existing engine capability (C5).** Only
   when the mechanism needs engine-visible state. Requires: the consult
   point named in this file's §2.5 table, determinism classification
   (deterministic / seeded-replay-pinned), and honest D-008 naming.
3. **Mark (C2).** Only when the mechanism is declarative metadata consulted
   at defined points. Requires: a §2.3 row here, the MN-key, the
   BIO-CONTRACT label, and the neutrality argument (what the mark does NOT
   change — the `@deprecated` precedent).
4. **Native kernel (C7).** Only with a measured hot path (profile receipts)
   and the Rust-fallback agreement pin.
5. **Core syntax (C1).** CLOSED. A new reserved word requires a DECISIONS
   entry, an owner word, and a SPEC amendment — the W036 freeze has no
   routine path here, by design.

Two standing rules bind every rung: (a) `docs/KEYWORDS.md` is GENERATED from
`src/parser.rs::KEYWORDS` — hand-editing it is a docs-sync failure; (b) the
deprecation/removal path is the compatibility ladder (docs/specs/
COMPATIBILITY.md) via the `@deprecated` machinery — bio surfaces retire the
same way core surfaces do, with migration text, never silently.

The 60-keyword reserved set is frozen at the W036 inventory: 35 core
keywords (control flow, bindings, functions/generators, abstraction, errors,
modules, concurrency, verification) + 25 grandfathered bio keywords (the
GRN, signal-pool, operon/oscillator, variant-swap, fate-machine, and
entry/editing families in CORE-BIO-BOUNDARY.md's tables). Counts are live
truth in KEYWORDS.md; this file's tables must never disagree with it
(check_lowering.py enforces).

## 4. Per-feature spec template (mandatory for every new bio mechanism)

Every new biological mechanism — at ANY class — fills this template across
the two-track documentation (W091: language contract in SPEC §11,
modeling rationale in MODELING-NOTES.md). A section with no content is
filled with the honest "none" and a reason; empty-by-omission is a review
blocker. The house check (scripts/check_lowering.py) enforces the
section vocabulary's presence in this file and the marker discipline's
continuation.

Template (section order fixed, names exact):

1. **`[MN-<key>]`** — the mechanism key, coined once, used identically in
   SPEC §11 markers, MODELING-NOTES §2 heading, BIO-CONTRACT table row, and
   the §2 lowering row here. Renaming = one PR touching all four.
2. **Surface** — the exact syntax: keyword(s), mark spelling, cell keys,
   builtin signature. Verbatim, with the grammar production if C1/C2.
3. **Lowering (L1)** — the core surface it lowers to, one paragraph, plus
   the §2 table row landed in the same PR. If it cannot be stated in core
   terms, the mechanism does not land (Law L1).
4. **Engine locus + parity plan (L2)** — tw/vm/py implementation sites; the
   pin file that arbitrates byte-parity (deterministic) or seed-replay
   parity (stochastic); which differential cell/grant applies.
5. **Determinism class** — `deterministic` | `seeded-stochastic (stream:
   std/random)` | `load-time` | `tooling-only`. Stochastic without a named
   seeded stream is rejected.
6. **BIO-CONTRACT label** — exactly one primary label from {REAL, APPROX,
   ABSTRACTION, SIMPLIFICATION}, with the compound form allowed
   (`REAL (X) + APPROX (Y)`); the honesty sentence in the same row.
7. **Not-modeled list** — what the mechanism deliberately does NOT model
   (lives verbatim in MODELING-NOTES §2; the template row cites it).
8. **Cell keys + grants** — every `.cell` key with type/range/defaults, and
   the grants the mechanism's tests need (zero-grant suite first).
9. **Freeze verdict** — the rung from §3 this mechanism entered at, and why
   no lower rung suffices.

The template's own placement rules: sections 2–4 land in SPEC §11
(language track), sections 1/6/7 land in MODELING-NOTES + BIO-CONTRACT
(modeling track), the §2 row lands here, and the pin lands in tests/.
A PR that lands one without the others is incomplete (two-track rule,
CONTRIBUTING §8b).

## 5. Total Grammar interaction rules

Bio constructs are full citizens of the 4-rung parse ladder (SPEC §4) and
obey its notes/repair discipline:

- **Rung interaction.** Bio keywords wobble-repair like any keyword
  (`@acetylat` → `@acetylate`, edit distance ≤ 2, ≤ 1 if length ≤ 4); mark
  spellings are rung-3 candidates exactly when their keyword forms are.
  Ambiguity between two bio keywords resolves to rung 4 (an error note),
  never to a coin flip.
- **Doc comments (W074).** `##` docs attach to bio declarations
  (`gene`/`splice`/`fate`/...) by the same hug-rule as core declarations;
  `@mark` lines above a declaration do not steal the attachment. Docs on
  bio surfaces are pure metadata — parity is not claimed on doc content.
- **Repair-invariance.** A bio construct that survives the ladder must
  evaluate identically to its canonical spelling (`fmt∘fmt = fmt` holds
  corpus-wide; wobble-repaired marks are the same declaration). The
  differential corpus carries repaired-vs-canonical pins for the mark
  families.
- **The boundary test (W091).** A construct is biological modeling (and its
  rationale lives in the modeling track) iff its behavior cannot be
  predicted without SPEC §11. Everything else is language track. The test
  is per-construct, not per-file.
- **Two-track review (CONTRIBUTING §8b).** Mixed-track PRs label hunks; a
  bio-analogy rewording can never justify a behavior change and vice versa.
  The §2 rows here are language-track contract; the MN audits are
  modeling-track rationale.
- **Checker discipline.** check_docs_sync.py guards the [MN-*] marker →
  MODELING-NOTES §2 resolution and the SPEC two-track pointers;
  scripts/check_lowering.py guards THIS file's coverage obligations
  (keyword/class 1:1, MN-key 1:1, label vocabulary, template presence).
  The two checkers compose; neither substitutes for review.
