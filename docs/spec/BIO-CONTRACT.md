# OPERON BIOLOGICAL MODELING CONTRACT

W092 of the M100 program · v1.0.0 · 2026-09-26 · owner: sz (dev-3)
Companion to SPEC §11 (regulation layer, language contract) and to
`docs/spec/MODELING-NOTES.md` — the modeling-track appendix (W091) that holds the term
audits, the not-modeled lists, and the SPEC §16 biology ↔ feature map.
Audience rule D-008: CS engineers first — biology is an intuition aid, never a prerequisite.

---

## 0. Purpose

Operon borrows molecular-biology vocabulary for programming semantics. That borrowing is
powerful and dangerous: a reader may assume simulation outputs carry scientific weight they
do not have. This document labels **every live mechanism** on a four-point honesty scale and
states what outputs do and do not mean. It is the contract that lets the language evolve
biologically without pretending to be a wet-lab tool.

## 1. The honesty scale

| Label | Meaning |
|-------|---------|
| **REAL** | The biological phenomenon exists and the mechanism's role is faithfully mirrored (direction, gating, causality). |
| **APPROX** | Mathematical approximation of a real mechanism: simplified kinetics, normalized ranges, discrete time. |
| **ABSTRACTION** | Operon-specific semantic device using a biology name; behavior defined by Operon's spec, not by biology. |
| **SIMPLIFICATION** | Deliberate fiction kept for pedagogy/utility; would not survive peer review as a model. |

Every mechanism below is graded **per aspect** (what it models vs how). "Output meaning"
says what a user may conclude from watching levels/telemetry.

## 2. Mechanism inventory (all live mechanisms at dd76caa)

| Mechanism (Operon surface) | Biology it mirrors | Grade | Notes & honesty |
|---|---|---|---|
| `regulate { a activates b }` GRN | transcription-factor regulation networks | REAL (direction/causality) + APPROX (wave propagation `strength^wave` ≤ 10 waves, inhibition applied once per fire) | Levels are normalized 0..1, not concentrations. Persistence = latch (opt-in decay = dilution). Term audit reg-r3 bans the word "homeostasis" without a setpoint. |
| threshold edges (dose-response) | Hill-type dose response | APPROX (n=2 fixed: `s·p²/(p²+t²)`) | One Hill coefficient for all edges; reg-bio added per-edge cooperativity, still APPROX. |
| `@methylate` / `@acetylate` marks, `methylate()/demethylate()` | epigenetic regulation (DNA methylation, histone acetylation) | REAL (silencing/activation direction) + ABSTRACTION (integer levels +1 / saturating −1, gate at next call) | No chromatin, no CpG islands, no inheritance across "generations" (there are none). |
| `m6a` marks + m6A readers (F-6, F-1m) | N6-methyladenosine RNA modification | REAL (stability/translation effects) + APPROX (level-based effects, reader coupling) | Reader effects are directional mirrors, not kinetic models. |
| RISC (stoichiometric) | RNA interference / siRNA-guided degradation | REAL (targeted degradation) + APPROX (stoichiometric consumption: guides are used up) | No off-target modeling. |
| `splice()` / splice sites | eukaryotic RNA splicing | REAL (exon joining, uORF interplay) + ABSTRACTION (splice fuel charges; site syntax is text-level) | Introns are not modeled as objects. |
| uORF fuel (rt_p2f) | upstream ORF translational regulation | REAL (repressive effect) + APPROX | Single-knob model. |
| IRES | cap-independent translation initiation | REAL (bypasses 5'-cap gating) + ABSTRACTION | Binary on/off in Operon; real IRES strength varies. |
| riboswitch, cis (F-5) | metabolite-sensing RNA controlling its own transcript | REAL (cis-acting aptamer) + APPROX (ligand-level switch) | The aptamer lives on the transcript it controls, the cis requirement is enforced (loop-9 wave B4 fixed exactly this honesty point). |
| polycistronic operon construct (A1/A7) | bacterial operons (single promoter → multi-gene transcript) | REAL (co-transcription) + ABSTRACTION (translation coupling simplified) | Attenuation/termination interplay modeled separately. |
| attenuation (`trp_attenuator`) | ribosome-speed-coupled transcription attenuation | REAL (coupling direction) + APPROX (two-state outcome) | No codon-level ribosome kinetics. |
| Rho-dependent termination (F-7) | Rho helicase catching RNA polymerase | REAL (termination pressure grows with naked runway) + APPROX (catch probability `1−(1−catch)^d`) | Direction fixed in loop-10 W2, pressure GROWS with naked runway; jury-verified. |
| ribosome-queue coupling (F-8) | translation-coupled mRNA decay shielding | REAL + APPROX | Shield = reduced decay while queued; not a kinetic model of the nuclease. |
| repressilator | published 3-gene ring oscillator (Elowitz–Leibler 2000) | REAL (topology) + APPROX (basal leak, deterministic-noise mode, parameterized α/β) | Deterministic mode exists for byte-parity; stochastic mode uses seeded telegraph noise. Reproduction tests: `tests/repressi_params.op`, `tests/repressi_alpha.op`. |
| telegraph promoter | stochastic two-state promoter activity | REAL (expression bursts) + APPROX (two-state Markov, seeded) | The source of "operon noise"; seeded ⇒ deterministic replay per DETERMINISM.md. |
| quorum sensing (C8) | population-level autoinducer coordination | REAL (density-dependent switching) + ABSTRACTION (population = worker/toggle layer) | No diffusion/geometry. |
| ligand binding / allostery / occupancy / synergy / titration / decay-clock (reg-bio-2) | receptor and enzyme regulation | REAL (qualitative directions) + APPROX (two-tier binding math) | Two-tier state is the honest reduction floor (loop-8 verdict); deeper kinetics are deliberately out. |
| decoy sites | miRNA/RBP decoy sequestration | REAL (sequestration reduces effective regulator) + APPROX |, |
| enhancers (`enhance`) | enhancer elements boosting transcription | REAL (boost direction) + ABSTRACTION (runtime boost multiplier) | Runtime-effect verified by `tests/enhance_boost.op` (T2e defect class closed). |
| bursting | transcriptional bursting | REAL (bursty expression) + APPROX (burst state machine, seeded) |, |
| CRISPR (`crispr` CLI/gene tooling) | programmable DNA targeting | REAL (target/cut semantics) + ABSTRACTION (text-level genome edits in demos) | Not a genome editor; operates on demo sequences. |
| `std/motifs` (motif finders, memoized DP) | regulatory motif detection (Shannon entropy, information content) | REAL (the math is real math) + APPROX (scoring models simplified) | Pure computation, correctness proven, not biology-calibrated. |
| `distance()`/`similar()` (C++ codon kernel) | sequence similarity | REAL (edit-distance math) + APPROX (codon scoring style) | Bit-parallel Myers; 10M-cell ceiling, 64 KiB operand cap (THREAT-MODEL 4.3). |
| `transcribe/translate/reverse_complement/gc_content/find_orf` | central-dogma utilities | REAL (textbook-correct transformations) + SIMPLIFICATION (fixed genetic-code table; no codon ambiguity beyond the wobble note) | Exactly as true as a textbook table, the most scientifically solid layer. |
| mRNA/protein two-tier state | gene expression two-species reduction | ABSTRACTION (explicit modeling choice, loop-8 "honest reduction ends here" verdict) | Deeper layers (folding, transport) out of scope by decision. |

## 3. What simulation output means (and does not mean)

1. **Levels are normalized** (0..1 clamped). They are *not* concentrations, copy numbers,
   or probabilities of any biological event. `grn_state()` output is an Operon state vector.
2. **Time is abstract**: `grn_fire` steps, repressilator ticks, and burst rounds are
   discrete semantic steps, not seconds, not cell-cycle phases.
3. **No laboratory prediction**: outputs support intuition, teaching, algorithmic
   exploration, and deterministic testing. They must never be presented as biological truth.
4. **Determinism over fidelity**: where biology is stochastic, Operon seeds the noise
   (DETERMINISM.md §4). The model is reproducible first, realistic second.
5. **Names are anchors, not claims**: a gene called `lacI` behaves per Operon's spec, not
   per E. coli. The no-scientist-names mapping (D-008, now MODELING-NOTES.md §3) keeps vocabulary readable
   without implying simulation of the named system.

## 4. Governance (answers the audit's "do not keep adding biological syntax forever")

1. **Syntax freeze discipline (W36, dev-2 owns the SPEC side)**: new biological mechanisms
   land as (a) `.op` library modules, (b) declarative `.cell` configuration, or (c) new
   builtin functions, **parser keywords require a written justification** in DECISIONS.md
   showing why library/declarative forms cannot express the mechanism.
2. **Every new mechanism ships with a row in §2** (this file) in the same PR, no row, no
   merge (sz review checklist item).
3. **Fidelity work** (kinetics, coupling, direction fixes) is welcome and gated by the
   differential harness + jury reviews (loop-9/10 pattern), never by intuition.
4. **Semantics separation (W091)**: language semantics (SPEC §1–§10) do not change when
   bio modeling changes; bio modeling lives in §11/§16 + this contract. A modeling fix is
   never a breaking language change by construction.
5. **Validation layer (W093)**: reference-model tests with tolerances (repressilator
   period, attenuation outcomes, riboswitch cis behavior) are the bridge between this
   contract and executable proof; see `docs/spec/VALIDATION.md` as it lands.

## 5. Evidence index

- Gate behaviors: `tests/methyl_gate.op`, `tests/grn_gate.op`, `tests/seq_gates.op`, `tests/splice_silence.op`
- Dynamics: `tests/repressi_params.op`, `tests/repressi_alpha.op`, `tests/grn_decay.op`, `tests/repressi_osc.op`
- Dose/response: `tests/silence_dose.op`, `tests/silence_degrade.op`, `tests/trp_attenuator.op`, `tests/riboswitch_cis.op`
- Constructs: `tests/mechanisms.op`, `tests/methylate_api.op`, `tests/spawn_regulation.op`, `tests/worker_seed_pin.op`
- App-layer demo: `apps/genomelab/genomelab.op`
- History: reg-bio (1b9785d), reg-bio-2 (749ba68/9d614f5), reg-bio-3 (16eebae), loop-9 waves A/B1–B5, loop-10 waves R/R-b/R-c (commit subjects carry F-numbers)

**R0.9 pointer:** the class + lowering rule for every graded mechanism above
is normative in [LOWERING.md](LOWERING.md) (roadmap R0.9, §2); its checker
enforces that this file's label vocabulary is the only vocabulary the
lowering contract cites.
