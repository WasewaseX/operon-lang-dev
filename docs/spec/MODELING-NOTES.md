# OPERON MODELING NOTES — the biology track

W091 of the M100 program · v1.0.0 · 2026-09-27 · owner: sz (dev-3)
Status: normative for the MODELING track. `SPEC.md` remains the only normative document
for the LANGUAGE track. Companion documents: `docs/spec/BIO-CONTRACT.md` (the honesty
labels), `docs/spec/VALIDATION.md` (reference datasets), `docs/spec/DETERMINISM.md`
(replay contracts), `docs/spec/BIO-LAYER-POLICY.md` (the C++ boundary rule).

---

## 0. The two tracks (read this first)

Operon borrows molecular-biology vocabulary for programming semantics. That borrowing
lives on two separate tracks, and this document is the second one:

1. **The language track** — `SPEC.md`. What the program DOES: syntax, exact semantics,
   gates and their order, clamps and defaults, entropy discipline, the `.cell` knobs, and
   the tests that pin every one of those statements. A change here asserts behavior and
   requires evidence (proof / differential / cargo test; oracle parity where semantics are
   mirrored).
2. **The modeling track** — this document plus `BIO-CONTRACT.md`. What the mechanism
   APPROXIMATES biologically: the real-biology context behind each name, the term audits
   that police word choice, the not-modeled lists that say where the analogy stops, and
   the audit trail that graded every mechanism. A change here must not alter behavior by
   construction, and needs no behavior gates.

The review rule that enforces the boundary lives in `CONTRIBUTING.md` §8b. Its one-line
form: **a bio-analogy change can no longer silently be a language change, and a language
change can no longer hide behind a bio-analogy rewording.**

The audience rule D-008 is unchanged: CS engineers first — biology is an intuition aid,
never a prerequisite. Nothing in this track is needed to write, read, or verify an Operon
program.

## 1. Why an appendix (the W091 split, what moved, what stayed)

Before W091, SPEC sections 10–18 mixed both tracks in one place: a mechanism's contract
sentence and its biology rationale shared a bullet, so a rewording of the analogy could
drift the contract (or vice versa) without any gate noticing. The W091 restructure moved
every biology-rationale fragment out of SPEC into this appendix; SPEC keeps markers of the
form `[MN-name]` at each extraction point, and this document's §2 is keyed by the same
names.

Coverage of the §10–§18 scan (the level's assignment):

| SPEC section | Verdict | Where the modeling content lives |
|---|---|---|
| §10 Builtins | contract-only; the bio builtins are textbook transformations | honesty grades: `BIO-CONTRACT.md` §2 (SIMPLIFICATION rows) |
| §11 Regulation layer | **split** — contract stays in SPEC, rationale moved | this doc §2 (the largest extraction) |
| §12 Frames/proofs | contract-only (test machinery) | — |
| §13 Concurrency | contract-only (snapshot semantics) | — |
| §14 Telemetry | contract-only (metric definitions; retired-velocity honesty note stays in SPEC) | grades: `BIO-CONTRACT.md` §2 |
| §15 Toolchain | contract-only (CLI) | — |
| §16 Biology ↔ feature map | **moved wholesale** | this doc §3 (SPEC keeps a numbered stub) |
| §17 Version / §18 Verification | contract-only | — |
| §19 Memory model | contract-only | — |

## 2. Term audits and modeling rationale, per mechanism

Markers `[MN-*]` in SPEC §11 resolve here. Provenance IDs (reg-bio, reg-bio-2/-3, C1…C11,
A1…A7, F-1…F-8, 12-c/12-d, 13a) are the loop/audit wave that introduced the mechanism; the
jury and reg-bio audit files themselves are listed in §4.

### MN-enhance (`enhance`, super-enhancer cluster; reg-bio)
Real enhancer strength varies with binding-site number and affinity. Operon's boost is a
single flat dose (default 0.25, one `.cell` knob) — the honest ABSTRACTION is "more
enhanced = lower effective threshold", not a binding-site model. Graded in
`BIO-CONTRACT.md` as REAL (boost direction) + ABSTRACTION (runtime multiplier).

### MN-@acetylate (`@acetylate`; term audit, reg-bio; D-005)
Acetylation is permissiveness — neutralized lysine charges open the chromatin — not
dispatch priority. The mark's honest effect is immunity: exclusion from silence rewriting
and from the silencing gates. "Open chromatin wins" is the ruling name (D-005), not a
chromatin simulation. There is no chromatin object in the interpreter.

### MN-@methylate (`@methylate`, graded silencing; reg-bio)
Histone marks compete on the same chromatin: an `@acetylate` definition relaxes what an
`@methylate` definition deepens. The lattice is integer 0..=3 with a threshold gate — no
nucleosome positioning, no mark propagation along a polymer, no reader/writer complexes.
Silencing direction is the REAL part; the integer lattice and next-call gating are the
APPROX/ABSTRACTION part.

### MN-@m6a (`@m6a`, dispatch priority + redefinition resistance; term audit, reg-r4)
The honest analogy for "the marked original wins over the unmarked new copy" is
prokaryotic **DNA** m6A: Dam methylation marks the parent strand so mismatch repair knows
which base is the error. Eukaryotic RNA-m6A "stability" is reader-dependent and is the
wrong analogy for this mechanism (the READER fate layer, where RNA-m6A belongs, is a
separate mechanism — see MN-m6a-readers). The mark is a level 0..=3 (MN-m6a-write); the
Dam-style reading applies to the resistance semantics, not to any repair process (none is
modeled).

### MN-silence (`silence`, RISC; reg-bio-3, C9)
Real RISC/miRNA destroys the transcript — there is no replacement gene — which is why the
target-less form is pure degradation. Real RNAi is dose-dependent: limiting RISC complexes
give fractional knockdown, and multiple target sites compound; that is the per-site
capture probability `1 − Π(1−sᵢ)^sitesᵢ`. Operon draws once per call attempt on the shared
mirrored stream; no guide/target kinetics, no off-target effects, no amplification
(secondary siRNA) are modeled.

### MN-regulate (`regulate`, GRN persistence and decay; term audit, reg-r3)
Persistence is a latch; decay-to-zero is dilution/degradation. Neither is "homeostasis",
which requires a regulated setpoint — the word is banned from these layers unless a
setpoint exists (loop-10 term-audit wave). Levels are normalized 0..1 fractions, not
concentrations; wave propagation `strength^wave` (≤ 10 waves) and once-per-fire inhibition
are the APPROX parts of an otherwise REAL direction/causality mirror.

### MN-toggle (`toggle`; term audit, reg-r3)
A declared boolean invariant, not rate-based bistability: no cooperativity, no hysteresis
in the `toggle` statement itself. The Gardner–Cantor–Collins toggle switch is the
biological inspiration, not a simulated model. Rate-based bistability IS expressible one
layer down (two mutual inhibiting edges with `hill ≥ 2`); the cooperativity-dependent
partial bands are proven in `tests/grn_bistability.op`, and hysteresis is NOT yet
demonstrated by a sweep test (loop-8 honesty note, carried in SPEC).

### MN-repressilator (`repressilator`; term audit, reg-r3; F-5; 12-d D6; V2)
The published system is the two-state mRNA+protein model of Elowitz & Leibler (Nature
2000) with Hill n = 2, α ≈ 250, β = 5 and a basal term. Operon's ring is the one-state
protein-only reduction with h = 4 and no basal term, which oscillates robustly in discrete
time (20 substeps of dt = 0.05 per tick). The `basal` knob exists because real repressed
promoters never lose their leak entirely. The noise kick is multiplicative and dt-aware
because real expression noise is multiplicative and strictly positive; the original
additive kick rectified upward through the zero clamp (12-d D6 finding). The reduction is
graded REAL (topology) + APPROX (kinetics); the reference-values pin is VALIDATION.md V2
(peak-to-peak = 6 ticks against the 2000 discrete parameterization) and the stochastic
mode's seeded telegraph noise is covered by MN-telegraph.

### MN-telegraph (telegraph promoter, bursting; reg-bio, F-1; F-2; F-3; 12-c C3)
Real promoters switch between active and inactive states; transcription happens in bursts;
and noise is what lets bistable cells switch fate. Operon's two-state Markov promoter
(seeded, replayable per DETERMINISM.md) models exactly that phenomenology. Per-gene
promoter identity (`@burst kon koff`) mirrors that different promoters have different
(kon, koff) — that is their identity. The worker-cell RNG derivation from the task id
(`DEFAULT_SEED ^ id·GOLDEN`) exists because the old shared default seed synchronized
promoter bursts across cells — perfect correlation, the exact OPPOSITE of extrinsic noise
(12-c C3). Burst telemetry's finite-sample bias (trailing ON-run uncounted) is documented
in SPEC rather than hidden.

### MN-translates (`translates`, the two-tier layer; reg-bio-2, C1)
Real expression is two coupled tiers: transcripts accumulate fast and bursty; proteins
accumulate slower (translation), lag, and smooth the bursts. Term audit: Δcalls counts
EXECUTED CALLS, not molecules — transcripts-as-calls is the language's own unit; the
smoothing/lag/persistence phenomenology is what this layer models. No ribosome counting,
no codon-speed modeling, no protein folding or transport (the loop-8 "honest reduction
ends here" verdict covers the two-tier state).

### MN-occupy (`occupy`; term audit; reg-bio-2, D2b)
The multiplicative survival form `child *= 1 − influence` is the thermodynamic
Kⁿ/(Kⁿ+Rⁿ) occupancy reduction. Term audit: occupancy shapes the fire-phase dose
arithmetic; the call-gate veto remains threshold-based — occupancy is expressed through
the levels it produces, not through a second veto mechanism.

### MN-decoy (`decoy`; reg-bio-2, C11)
Real TF sequestration / decoy-site titration: a sponge absorbs regulator without producing
output, so overexpression closes gates and emptying restores them. Operon's version is a
deterministic free-fraction subtraction (`max(0, level(tf) − c·level(d))`) — no diffusion,
no site-multiplicity modeling.

### MN-attenuates (`attenuates`; term audit; reg-bio-2, A5)
Term audit: the ribosome-stalling mechanics are not modeled; the metabolite-threshold
outcome is. The edge reports the RNA-level mechanism ("leader terminated") — the OUTCOME
of ribosome-coupled leader attenuation (trp operon), not TF occlusion. No codon-level
ribosome kinetics, no leader sequence object.

### MN-decay_clock (`decay_clock`; term audit, C4; reg-bio-2, C2)
Real transcripts decay per unit time, not only when someone fires the network — hence the
call-clock cadence. Term audit, C4: call-clock time is an explicit design choice — genes
are closures the programmer invokes; a Gillespie SSA scheduler would invert the language's
own metaphor. The repressilator ring remains the continuous-time enclave. Single-knob
decay is the APPROX; no mRNA half-life tables.

### MN-operon-unit (polycistronic `operon`; reg-bio-3, A1/A7; 13a; F-7; F-8)
The namesake construct: ONE promoter drives N cistrons on ONE polycistronic mRNA, cistron
order load-bearing (RBS gradient + polarity exposure). Not modeled: Rho loading kinetics,
RNAP velocity, rut-site sequence strength, antitermination, tmRNA/SsrA rescue — the
per-cistron threshold-draw abstraction is the model (F-7). The ribosome-queue shield
(F-8) is an INITIATION-FLUX abstraction, not elongation pile-up: occupancy shields the
failure POINT, not the naked runway past it (R10-M4/M6 boundaries pinned in
`tests/granted/rho_queue_shield.op`). Term audit, 13a: documented as PROKARYOTE-specific —
eukaryotes lack polycistronic transcription; the construct is not "call these genes
together" sugar: the unit-level transcript counter, per-cistron gradient, and polarity are
the biology.

### MN-passage (`passage`, epigenetic inheritance; reg-bio-3, B2/B6)
Real epigenetic marks are maintained across replication only by maintenance machinery
(DNMT1-style copying); without it, marks dilute ~50% per generation — dilution is the null
model. The `.cell methyl.maintenance` fraction is that maintenance dial (1.0 = perfect
maintenance, 0.5 = pure dilution, 0.0 = instant loss) with half-down rounding so a diluted
mark never reads as MORE repressed. The 1,000,000-division clamp exists because a culture
that old is not a useful model. No methylation-pattern copying (per-site maintenance), no
chromatin replication modeling.

### MN-m6a-write (`m6a_write`/`m6a_erase`; reg-bio-3, B3)
Quantitative site density under the Dam-style analogy (MN-@m6a) — no eukaryotic
mRNA-reader drift in the resistance semantics. Writer/eraser dose shifts the 0..=3 level;
`.cell m6a.decay` is density dilution per decay tick. Standalone cadence (loop-9) keeps
the decay observable without a GRN clock.

### MN-splice_shift (`splice_shift`; loop-9, F-4)
Splicing factors (SR proteins, hnRNPs) change which splice site wins at runtime. The
trans-acting shift slots between the operator pins and the @m6a bias: a bound factor beats
a basal inclusion bias, the operator still has the final word. No R-spliceosome modeling,
no site-strength scoring.

### MN-@riboswitch (`@riboswitch`, cis; loop-9, F-5; A2 finding)
The aptamer lives on the transcript it controls — that is what CIS means here: the
metabolite pool is cell-wide (`ligand_level`), the SENSOR is per-gene. `off` class
(TPP/purine/SAM): ligand bound → terminator hairpin → transcription OFF. `on` class
(adenine/glycine activators): unbound → RBS sequestered → OFF. The trans ligand EDGE
remains the protein-free metabolite-sensor analog; the older "riboswitch-style" label for
that edge was wrong (the A2 agent's finding, loop-9) and is retired from SPEC. Ligand
polarity examples in SPEC (allolactose on LacI, tryptophan on TrpR) are the canonical real
systems behind the inducer/cofactor polarity rule.

### MN-m6a-readers (m6A reader fate; loop-9, F-6)
At density ≥ 2 the eukaryotic READER fate engages on `translates` edges: YTHDF2-like decay
routing and YTHDF1/3-like attenuation. This is where RNA-m6A biology belongs (reader-
dependent effects); the RESISTANCE semantics stay Dam-style (MN-@m6a). Reader effects are
directional mirrors, not kinetic models (BIO-CONTRACT grading).

### MN-quorum (quorum sensing; loop-9, C8)
Real bacteria share diffusible autoinducers: Vibrio fischeri LuxI → AHL, with LuxR·AHL
activating the lux operon at a density threshold; AiiA quenches (lactonase); S. aureus agr
and E. coli AI-2 are the other canonical systems. The signal is EXTRACELLULAR and SHARED —
my secretion raises YOUR activation — so the medium is process-global (deliberately not
part of the spawn snapshot; workers inherit frozen cytoplasm but a LIVE medium). Integer
molecule counts make concurrent secretions commute; `quorum("ahl", t)` reads
molecules/1e9. Dilution via `passage(n)` is per-division binary-exact halving. No
diffusion geometry, no spatial structure, no signal chemistry beyond the count.

### MN-@copies (`@copies`; reg-bio-3, C10)
Copy-number variation: dose amplifies the CONCENTRATION the gene feeds its GRN edges,
saturating on the 0..1 lattice like real transcript dose under titration. Copies change
transcript amount, never the call's return value (a call is a transcription event; its
return is the per-transcript product). No recombination, no expression noise from copy
number.

### MN-hygiene (level hygiene + determinism hardening; reg-bio-2, D2c/D7/D9)
A binding weight is not an amplifier (strength clamps 0..=1); a level is a concentration
fraction (fire influence clamps at 1.0). Sorted-key emission exists because HashMap
iteration order varies per process (a proof-frame and oracle-parity hazard), and
`burst_total` accumulates in sorted order because float addition is not associative.
These are language-track rules living in SPEC §11; their modeling side — a level is a
fraction, noise must decorrelate cells — is recorded here so the WHY survives edits to
the WHAT.

## 3. Biology ↔ feature map

Moved verbatim from SPEC §16 by W091 (the section number stays in SPEC as a numbered
stub pointing here). For the honesty grading of every row, see `BIO-CONTRACT.md` §2; for
numerical reference pins, `VALIDATION.md`.

| Mechanism (real molecular biology) | Operon feature |
|---|---|
| Wobble base pairing (redundant codon recognition) | 4-rung Total Grammar + synonym table |
| Codon optimality | `codon()` scoring in check grading |
| Overlapping reading frames | `frame proof` — tests and code in one sequence |
| Alternative splicing | `splice { variant }` + `--variant` / `.cell` selection |
| Runtime splice regulation (splicing factors) | `splice_shift(root, variant)` — trans-acting shift between the operator pins and the @m6a bias |
| m6A reader fate (YTHDF2 decay / YTHDF1-3 attenuation) | density >= 2 engages reader factors on `translates` (`.cell m6a.reader.*`) |
| Phenotypic state and differentiation | `phenotype` classes: `new`, `init`, `self`, inheritance `from` |
| Polypeptide elongation (values produced one at a time) | `sequence` generators + `yield` / `.next()` / `.collect()` on worker cells |
| RNA editing | `.rna` hot patches (`edit/replace`) |
| Upstream ORF repression | `guard (cond) else { }` leading clauses |
| DNA methylation / epigenetics | `.cell` config layer + `methyl()` |
| TADs + CTCF anchors | `tad` domains + `anchor export/import` |
| Super-enhancers | `enhance` clusters (dose via `.cell enhance.delta`) |
| Histone acetylation / methylation | `@acetylate` (permissiveness, not priority) / `@methylate` (graded marks) |
| Nonsense-mediated decay | NMD sweep (`--nmd`) |
| miRNA → RISC silencing / transcript degradation | `silence old -> new;` (allele replacement) / `silence old;` (pure degradation) |
| Two-tier expression (transcription → translation) | `a translates p rate r decay d;` — protein nodes lag & smooth transcript bursts |
| Small-molecule allostery (lac inducer / trp corepressor) | `ligand x;` + `bind tf inducer|cofactor lg k v;` — affinity modulation, level untouched |
| Riboswitches / protein-free metabolite gating | a ligand named as an edge source reads its pool directly |
| Quorum sensing (LuxI/LuxR AHL, agr, AI-2) | `autoinducer ahl;` + `secrete`/`quorum`/`quench` — process-global integer molecule pool, signal species as an edge source |
| Transcription attenuation (trp leader) | `x attenuates y threshold t;` — RNA-level veto with leader-termination report |
| TF sequestration / decoy binding sites | `decoy d for tf capacity c;` — free-TF titration |
| Enhanceosome synergy (cooperative pooling) | `sum` edge keyword — pooled Hill input, super-additive with hill > 1 |
| Thermodynamic occupancy repression | `occupy` edge keyword — multiplicative Kⁿ/(Kⁿ+Rⁿ) survival |
| Time-based degradation (half-lives) | `decay_clock(n, f)` / `.cell grn.decay_calls` — decay on the call clock |
| Extrinsic noise (cell-to-cell variation) | worker RNG streams derived from task id (decorrelated bursting) |
| m6A modification (quantitative site density) | `@m6a` dispatch priority + `m6a_write`/`m6a_erase` levels 0..=3 + `.cell m6a.decay` (Dam-style analogy — §11 term audit) |
| Polycistronic operons (lacZYA / trpEDCBA) | `operon lac { lacZ rbs 1.0; lacY rbs 0.6; }` — unit-level gate, one transcript, cistron order load-bearing |
| RBS strength gradient (per-cistron translation efficiency) | per-cistron `rbs r` multiplier on `translates` rates — the lacZYA stoichiometric ratio |
| Transcriptional polarity (expected read-through loss) | upstream degraded/methylated cistrons scale downstream yield — per-member expected factor `surv + (1−surv)·polarity` (loop-9 weighted rule; methylation contributes `polarity` outright) |
| Rho-dependent termination + ribosome-queue coupling shield | a failed upstream translation leaves naked RNA; Rho loads and chases — termination probability compounds per cistron of naked runway `1−(1−catch)^d` (loop-10, opt-in); ribosome occupancy (queue ≥ floor) occludes rut sites — translated cistrons shield, drained queues re-expose |
| Epigenetic maintenance vs dilution (DNMT1 / passaging) | `passage(n)` + `.cell methyl.maintenance` — half-down dilution, `generation` counter |
| Gene copy-number variation (dosage) | `@copies n` — read-side dose amplification, saturating on the 0..1 lattice |
| Stoichiometric RISC (dose-dependent knockdown, multi-site) | `silence old -> new strength s sites n;` — per-call capture `1 − Π(1−s)^n` |
| IRES cap-independent entry | `ires name;` + `--ires` |
| UPR / ISR stress programs | `stress { } rescue { }` containment |
| Fate landscapes (valley semantics) | `fate` state machines |
| Gene regulatory networks | `regulate` + `grn_fire/grn_state` |
| Cooperative binding (Hill exponent / ultrasensitivity) | per-edge `hill n` dose-response + `motif_hill` |
| Cis-regulatory input functions (AND / OR promoters) | thresholded edges: conjunctive by default, `any` = alternative activator |
| Network motifs (autoregulation, coherent/incoherent FFL, toggle) | `std/motifs` runnable circuits |

## 4. Evidence base (the modeling track's audit trail)

The modeling track is not sustained by this document alone — every claim above descends
from a recorded audit. The canonical evidence corpus lives in the coordination vault
(`WasewaseX/project-vault`, `collab/audits/`):

- **Jury round r9**: `jury-r9-kinetics.md`, `jury-r9-noise.md`, `jury-r9-quorum.md`,
  `jury-r9-rna.md`, `jury-r9-verification.md` — the loop-9 wave's external review
  (promoter identity, telemetry boundaries, quorum medium semantics, cis-riboswitch).
- **Jury round r10**: `jury-r10-biology.md`, `jury-r10-kinetics.md`, `jury-r10-parity.md`,
  `jury-r10-stdlib.md`, `jury-r10-verification.md` — the loop-10 wave's review
  (Rho/queue coupling, composition rules, parity discipline).
- **reg-bio-4 register**: `reg-bio-4-kinetics.md`, `reg-bio-4-motifs.md`,
  `reg-bio-4-noise.md`, `reg-bio-4-quorum.md`, `reg-bio-4-rna.md`,
  `reg-bio-4-weakness-register.md` — the standing weakness register that feeds waves.

In-repo companions:

- `docs/spec/BIO-CONTRACT.md` — the four-label honesty grading of every live mechanism
  (REAL / APPROX / ABSTRACTION / SIMPLIFICATION), per aspect, with output meaning.
- `docs/spec/VALIDATION.md` — the scientific validation registry: reference models with
  numerical tolerances and provenance (EC50/Hill identity V1, repressilator period V2,
  plus registry-linked suite pins V3–V8).
- `docs/spec/DETERMINISM.md` — replay contracts (seeded RNG, draw-count invariance,
  bit-identity discipline) that make the stochastic modeling layers auditable at all.
- `docs/spec/BIO-LAYER-POLICY.md` — the C++ kernel boundary rule (no biological semantics
  ever enter C++).
- `docs/design/RNA-V2.md`, `docs/design/HOT-RELOAD.md`, `docs/design/MEM-PROFILER.md` —
  as-built design notes for mechanisms this track touches.

## 5. Change classes (what an edit to each file means)

| Edit | Track | Requirements |
|---|---|---|
| SPEC.md §1–§15, §17–§19 (contract) | language | behavior evidence: proof/differential/cargo test as applicable; oracle parity where mirrored; gates per Iron Rule 1 |
| SPEC.md §11 `[MN-*]` marker text | language | markers must resolve to §2 of this doc; renaming a key = editing both files in one PR |
| SPEC.md §16 stub | language | stub must link this doc (check_docs_sync fails otherwise) |
| this doc §2/§3 (rationale, map) | modeling | must not alter behavior; docs checks only (`check_docs_sync.py`, `doc_api_check.sh`) |
| `BIO-CONTRACT.md` (grades) | modeling | same as above; grade changes should cite an audit file from §4 |
| `VALIDATION.md` (reference pins) | modeling | tolerance/provenance discipline per its own header; failures are release blockers once stable |

A PR mixing the tracks must label which hunks are which; reviewers enforce the boundary
at lane check (CONTRIBUTING §8b).
