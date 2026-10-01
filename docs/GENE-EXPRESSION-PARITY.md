# GENE EXPRESSION PARITY — Operon the language vs real molecular biology

v1.0.0 · 2026-10-01 · owner: dev-1 (builder-A)
Companion to `docs/spec/BIO-CONTRACT.md` (the four-point honesty grades), `docs/spec/MODELING-NOTES.md`
(term audits and the §3 biology ↔ feature map), and `docs/spec/VALIDATION.md` (literature-anchored
numerical pins). This document is the beginner-friendly, end-to-end comparison the map alone does not
give: real biology first, the language's mirror next, the honest divergences last. It adds no
mechanisms and changes no semantics — documentation only (W091 separation).
Audience rule D-008 holds throughout: CS engineers first; the biology is an intuition aid, never a
prerequisite. Every biology claim below is textbook-level (Lewin's *Genes*, Alberts, or the cited
primary papers); every language claim is backed by a test you can run (§10).

---

## 0. The answer first

Operon is **not a cell simulator**. It is a deterministic programming language that borrowed
molecular biology's *control plane* — the machinery living cells use to decide which genes are
expressed, how much, and when — and made it executable language semantics. The core execution model
(genes are functions, calls are expression events) is a deliberate metaphor. The regulation layer on
top of it, however, is quantitatively anchored to real molecular biology: Hill dose–response curves,
thermodynamic occupancy, two-state telegraph promoters, cis riboswitch classes, Rho-termination
pressure, and the published repressilator all appear with their real directions, and several carry
numerical pins against the primary literature.

The verdict in one paragraph: **the direction and causality of every borrowed mechanism is
faithful; the kinetics are honest simplifications; the execution substrate is declared metaphor.**
Of the 25 live mechanisms in `BIO-CONTRACT.md` §2, 23 carry a REAL grade for the aspect they model
(the remaining two are declared design decisions: the Hill edge graded APPROX at its primary grade,
and the two-tier mRNA/protein state graded ABSTRACTION by the loop-8 "honest reduction ends here"
verdict). On the "how it is computed" side, the grades split 16 APPROX, 8 ABSTRACTION, and
1 SIMPLIFICATION (the central-dogma string utilities, which are textbook-correct but use one fixed
genetic-code table). That distribution is not an accident — it is enforced by the contract's
per-mechanism grading and by the term-audit waves that banned words like "homeostasis" wherever no
regulated setpoint exists.

What this buys you as a programmer: expression-control programs — gates, gradients, oscillators,
switches, dose responses — that behave the way the corresponding biology behaves in *direction*,
while staying byte-identical across runs, engines, and platforms (the parity invariants). What it
does not buy you: laboratory predictions. Levels are normalized 0..1 fractions, not concentrations;
time is discrete semantic steps, not seconds; noise is seeded, not physical. Section 6 lists every
divergence honestly; Section 8 lists what real biology has that Operon does not model at all.

---

## 1. The real biology, in brief

This section is the reference picture against which the language is judged. It is deliberately
compact — enough real molecular biology to make every later comparison checkable.

### 1.1 The central dogma

Information in a cell flows DNA → RNA → protein. **Transcription** copies a gene's DNA into
messenger RNA (mRNA) via RNA polymerase (RNAP); **translation** reads that mRNA in 3-base codons
via ribosomes and tRNAs to build a polypeptide. In prokaryotes (bacteria) the two processes are
*spatially coupled* — ribosomes start translating an mRNA while RNAP is still transcribing it —
and this coupling is not decoration: it drives attenuation, polarity, and Rho shielding below. In
eukaryotes the two processes are separated by the nuclear membrane, and the RNA between them is
heavily processed: capped, spliced, poly-adenylated, chemically modified.

### 1.2 What an operon actually is (Jacob & Monod, 1961)

An operon is a bacterial DNA cluster in which **one promoter drives several structural genes into
one polycistronic mRNA**. The pieces: a **promoter** (where RNAP + sigma factor bind, with
consensus −10/−35 boxes); an **operator** (where a regulatory protein binds to help or block); the
**structural genes** (the protein-coding cistrons, each with a **ribosome-binding site** — RBS —
whose Shine-Dalgarno strength sets per-cistron translation efficiency); and usually a
**regulator gene** elsewhere in the genome, *trans*-acting, with its own promoter. Because the
mRNA is polycistronic, an event at the promoter gates **every** cistron at once; because each
cistron's RBS differs, the proteins are produced in fixed **stoichiometric ratios**; and because
transcription and translation are coupled, trouble upstream **degrades** what downstream cistrons
get (**polarity**). The name Operon (the language) is taken from exactly this construct.

### 1.3 The lac operon: dual control with the right polarities

`lacZYA` is the textbook operon. Control is **negative inducible**: the LacI repressor sits on the
operator and blocks transcription; the inducer **allolactose** (or the synthetic IPTG) binds LacI
*allosterically* — LacI's abundance never changes, its DNA affinity does — so the repressor lets
go. On top of that sits **catabolite repression**: when glucose is scarce, cAMP rises, and
CRP–cAMP binds upstream as an **activator** that helps RNAP. The operon therefore expresses only
when *both* signals say so: inducer present (lactose available) AND cAMP high (glucose absent).
Getting *both* polarities right — ligand *reduces* the repressor's affinity, cofactor *requires*
the activator's binding — is the famous bit, and it is the exact behavior pinned in the language's
`tests/lac_gate.op` (§3 below).

### 1.4 The trp operon: repression plus attenuation

`trpEDCBA` is **negative repressible**: tryptophan acts as a **corepressor** — it binds the TrpR
aporepressor and *enables* DNA binding (the mirror image of lac: ligand ON means repression ON).
On top of that, the trp operon has **attenuation** (Yanofsky): a leader peptide with two Trp
codons is translated *while* transcription proceeds. If tryptophan is abundant, the ribosome runs
fast, a 3:4 terminator hairpin forms, and transcription aborts early; if Trp is scarce, the
ribosome stalls, the 2:3 antiterminator pairs instead, and transcription continues. The outcome is
RNA-level and metabolite-threshold-shaped; the mechanism is ribosome-speed-coupled.

### 1.5 The RNA control layer: riboswitches, Rho, RBS gradients, polarity

**Riboswitches** are RNA elements in their own transcript's 5′ end that bind small metabolites
directly (no protein required). They come in polarity classes: TPP, purine-G and SAM riboswitches
are **off-class** (ligand bound → terminator hairpin → transcription OFF); adenine and glycine
riboswitches are **on-class** (unbound → RBS sequestered → OFF). **Rho-dependent termination**:
Rho is a hexameric ATP-powered helicase that loads onto *naked* RNA at rut sites, chases RNAP
5′→3′, and catches it at pauses — terminating upstream of untranslated cistrons. Because
translating ribosomes shield the RNA from Rho loading, translation failures create naked runway,
and the termination pressure **grows with the length of that runway** — this is the mechanical
basis of polarity. RBS strength gradients (lacZYA expresses Z > Y > A) set stoichiometry.

### 1.6 The eukaryotic layer: splicing, m6A, RISC, epigenetics, domains

Eukaryotes add controls bacteria do not have. **Alternative splicing** recombines exons into
different mRNAs from one gene, regulated by splicing factors (SR proteins, hnRNPs). **m6A**
(N6-methyladenosine) is the most abundant internal mRNA modification; reader proteins (YTHDF2
routes to decay, YTHDF1/3 modulate translation) give it its effects. **RNA interference**: miRNA
guides RISC/Argonaute to transcripts for degradation — stoichiometric, dose-dependent, and
compounding across binding sites. **Epigenetics**: cytosine methylation at CpG islands and
histone acetylation (HATs open chromatin, HDACs close it) make stable, partially heritable
expression states; **maintenance methyltransferases** (DNMT1-style) copy marks at replication,
and without them marks dilute roughly 50% per generation. Chromatin folds into **TADs** bounded
by CTCF/cohesin anchors, and **super-enhancers** (dense enhancer clusters) drive cell-identity
genes with high, threshold-like doses.

### 1.7 The network layer: motifs, toggle, repressilator, quorum

Above single operons sit **gene regulatory networks** with recurring motifs: negative feedback
(homeostasis-like damping), positive feedback / mutual inhibition (**bistability** — the
Gardner–Cantor–Collins synthetic toggle switch, 2000), feed-forward loops, and the
**repressilator** (Elowitz & Leibler, *Nature* 2000): three transcriptional repressors wired in a
ring, oscillating with a period set by Hill coefficient, α, and decay ratios. And populations
coordinate across cells by **quorum sensing**: secreted autoinducers (Vibrio fischeri LuxI → AHL;
LuxR·AHL activates lux at a density threshold; AiiA quenches) make gene expression depend on
*population density* — my signal raises your activation.

---

## 2. The central dogma, mapped

| Real biology | Operon language surface | Notes |
|---|---|---|
| DNA sequence | text in the `.op` program (plus demo-sequence ops) | the genome-as-data is deliberately thin; see §6 |
| Transcription (DNA → mRNA) | calling a gene = one transcription attempt; `fingerprint().transcripts` counts unit-level transcripts | suppressed calls produce **no** transcript — suppressed expression is not invisible expression |
| Translation (mRNA → protein) | `a translates p rate r decay d;` two-tier protein nodes | protein tier lags and smooths transcript bursts (§5 row 24) |
| Polypeptide elongation (one residue at a time) | `sequence` generators + `yield` / `.next()` / `.collect()` | values produced one at a time |
| Genetic code table | `transcribe` / `translate` / `reverse_complement` / `gc_content` / `find_orf` builtins | textbook-correct, one fixed table (SIMPLIFICATION) |
| Wobble base pairing | 4-rung Total Grammar + synonym table | the parser accepts synonyms the way codon recognition accepts near-matches |
| Codon optimality | `codon()` scoring in check grading | scoring aid, not kinetics |
| Alternative splicing | `splice { variant }` + `--variant` / `.cell` selection | eukaryotic mechanism, present as first-class construct |
| Splicing factors (SR/hnRNP) shifting site choice | `splice_shift(root, variant)` | trans-acting shift between operator pins and the @m6a bias |
| RNA editing | `.rna` hot patches (`edit` / `replace`) | post-transcriptional repair, language analog |
| Nonsense-mediated decay | NMD sweep (`--nmd`) | quality-control sweep over the corpus |
| IRES (cap-independent entry) | `ires name;` + `--ires` | binary in Operon; real IRES strength varies |
| uORF repression | `guard (cond) else { }` leading clauses | upstream-clause repression mirror |

The reading the table should produce: **every stage of the dogma has a language mirror, and the
mirrors are graded individually** — the string utilities are the most literally true thing in the
language (they are exactly what a textbook table says), while the two-tier translation state is the
most explicitly declared abstraction in it. That inversion (most-true = the boring string ops,
most-abstract = the interesting dynamics) is what honest grading looks like.

---

## 3. Side by side: the lac operon

Real biology first, in one breath: LacI blocks `lacP`; IPTG/allolactose binds LacI and releases
the DNA *without changing how much LacI exists*; CRP helps RNAP only when cAMP is present; so
`lacZYA` expresses only under *inducer AND low glucose*; remove the inducer and repression snaps
back. Now the language program — `tests/lac_gate.op`, lightly abridged:

```text
ligand iptg
ligand camp
regulate {
    lacI inhibits lacZ strength 0.9 threshold 0.3
    bind lacI inducer iptg k 0.2
    crp activates lacZ strength 0.9 threshold 0.4
    bind crp cofactor camp k 0.05
}
gene lacZ() { return "beta-gal" }
```

The test then drives the four canonical states and pins the two polarity facts the real operon is
famous for:

1. **No ligands**: `lacZ()` returns null — the repressor sits on the operator and the cofactor is
   absent. Dual control gates in the AND direction, exactly as in §1.3.
2. **IPTG added**: the call is *still* null (CRP–cAMP off — the glucose-high state), and the test
   asserts `grn_get("lacI") == 1.0` to the last digit: **the repressor's level never moved.** The
   ligand changed the *binding affinity*, not the *abundance* — allostery, not dilution. This
   single assertion is the difference between modeling the lac operon and merely writing a gate.
3. **IPTG + cAMP**: `lacZ()` returns `"beta-gal"` — the classic expressing state.
4. **IPTG removed**: the call goes null again — repression snaps back when the inducer leaves.

Every step is an `assert` inside a `frame proof` block, so the biology is not a comment; it is an
enforcing test in the CI set. The VALIDATION track carries the same allostery discipline for the
trp system: in the trpR model, the *regulator level* never moves either — the ligand flips the
binding state (`v4_trpR_unchanged` pin). The language got this polarity right because an early
audit (reg-bio-2, A4) checked it, not because the name "lac" makes it so.

---

## 4. Side by side: the namesake construct — the polycistronic operon

The construct the language is named after is modeled as a *unit*, not as sugar. From
`tests/operon_unit.op`:

```text
regulate {
    activator activates lac strength 0.9 threshold 0.5
    lacZ translates pz rate 1.0
    lacY translates py rate 1.0
    lacA translates pa rate 1.0
}
gene lacZ() { return "z" }
gene lacY() { return "y" }
gene lacA() { return "a" }
operon lac { lacZ rbs 1.0; lacY rbs 0.6; lacA rbs 0.3; }
```

The test pins the three biology-carrying properties that make this a real mirror and not
"call these genes together" syntax:

- **The unit gate.** Before induction, *every* cistron call returns null — an edge targeting the
  unit `lac` vetoes all members, because in real bacteria the promoter drives the whole
  transcript. The test also asserts `fingerprint().transcripts.lac == 0`: suppressed calls make
  **no transcripts**, so a gated unit is observably silent, not silently empty.
- **The RBS gradient.** After induction and `grn_fire`, the protein tier holds exactly
  `pz = 1.0`, `py = 0.6`, `pa = 0.3` — the per-cistron Shine-Dalgarno strength gradient that
  gives real operons their stoichiometry (lacZYA-style Z > Y > A), pinned to `1e-12`.
- **Polarity compounds.** Degrading the first cistron (`silence lacZ;` — RISC capture) reduces
  downstream yield by the polarity factor, and degrading the second reduces what remains:
  Y goes 0.6 → 0.9 (0.6 + 0.6·0.5) and A goes 0.3 → 0.45 → 0.525 (0.45 + 0.3·0.25). Upstream
  trouble scales *everything below it*, multiplicatively — the read-through-loss rule
  `surv + (1−surv)·polarity` per member.

Not modeled, honestly (MODELING-NOTES MN-operon-unit): Rho *loading kinetics*, RNAP velocity,
rut-site sequence strength, antitermination, tmRNA/SsrA rescue. The per-cistron threshold-draw
abstraction is the model; the ribosome-queue shield (§5 row 13) adds the initiation-flux
protection mechanism on top. The term audit also records that the construct is documented as
PROKARYOTE-specific — eukaryotes lack polycistronic transcription, and the doc says so rather
than pretending the metaphor is universal.

---

## 5. Mechanism by mechanism: the full comparison

The canonical per-mechanism grades live in `BIO-CONTRACT.md` §2; the term audits live in
`MODELING-NOTES.md`; the numerical pins live in `VALIDATION.md`. This table is the readable
merger — what the real mechanism is, what the language does, and where the two part ways.

| # | Real mechanism | Operon surface | What is faithful (REAL) | What is approximated (how-side) | Grade |
|---|---|---|---|---|---|
| 1 | Transcription-factor networks | `regulate { a activates/inhibits b }` + `grn_fire`/`grn_state` | direction, gating, causality of edges | levels are 0..1 fractions, not concentrations; wave propagation `strength^wave` ≤ 10; once-per-fire inhibition | REAL + APPROX |
| 2 | Hill dose–response (ligand-receptor binding) | per-edge `hill n`, threshold `t` | the curve's shape family and EC50 semantics: `s·Lⁿ/(Lⁿ+tⁿ)`, n=2 → t IS the EC50 | fixed shapes, normalized range; V1 pins t/3→10%, 2t→80%, 3t→90% against the Hill–Langmuir function | APPROX |
| 3 | DNA methylation / histone acetylation | `@methylate` (graded 0..=3 lattice, threshold gate), `@acetylate` (silencing immunity — "open chromatin wins") | silencing/activation *direction*; marks compete on one substrate | integer lattice, no nucleosome positioning, no mark propagation, no reader/writer complexes; heritability only via §5 row 23's passage machinery | REAL + ABSTRACTION |
| 4 | m6A (N6-methyladenosine) | `@m6a` marks 0..=3, `m6a_write`/`m6a_erase`, `.cell m6a.decay`; reader fate at density ≥ 2 on `translates` (YTHDF2-like decay, YTHDF1/3-like attenuation) | resistance = Dam-style parent-strand-priority (prokaryotic DNA m6A); reader effects = eukaryotic RNA m6A — both, in their honest places | level-based, reader effects are directional mirrors not kinetics | REAL + APPROX |
| 5 | RNAi / miRNA → RISC | `silence old -> new strength s sites n;` (allele replacement) / `silence old;` (pure degradation) | targeted transcript destruction; stoichiometric, dose-dependent, compounding: `1 − Π(1−sᵢ)^sitesᵢ` | no guide/target kinetics, no off-target effects, no secondary-siRNA amplification | REAL + APPROX |
| 6 | Alternative splicing | `splice { variant }`, `--variant` / `.cell` selection | exon recombination into distinct products | splice fuel charges; site syntax is text-level; introns are not modeled as objects | REAL + ABSTRACTION |
| 7 | Splicing factors (SR proteins, hnRNPs) | `splice_shift(root, variant)` | trans-acting factors change which site wins at runtime | a bound factor beats a basal @m6a bias; the operator pins still win; no R-spliceosome model | REAL + APPROX |
| 8 | uORF repression | `guard (cond) else { }` leading clauses | upstream-clause repressive effect | single-knob fuel model | REAL + APPROX |
| 9 | IRES | `ires name;` | cap-independent initiation bypasses 5′-cap gating | binary on/off; real IRES strength is a spectrum | REAL + ABSTRACTION |
| 10 | Cis riboswitches | `@riboswitch(ligand, sense, threshold)` per gene; ligand pool cell-wide | the aptamer lives on the transcript it controls (cis is *enforced*); off-class (TPP/purine-G/SAM) vs on-class (adenine/glycine) polarities are distinct | ligand-level switch; no hairpin-folding geometry | REAL + APPROX |
| 11 | Polycistronic operons (lacZYA, trpEDCBA) | `operon name { cistron rbs r; ... }` | ONE promoter → ONE transcript → N cistrons; unit-level gating; order matters | per-cistron threshold draws; translation coupling simplified | REAL + ABSTRACTION |
| 12 | Attenuation (trp leader) | `x attenuates y threshold t;` | ribosome-stall → early-termination *outcome*, reported as "leader terminated" | no codon-level ribosome kinetics, no leader-sequence object; two-state outcome | REAL + APPROX |
| 13 | Rho-dependent termination + ribosome-queue shielding | failed upstream translation → naked RNA → Rho loads and chases; catch `1−(1−catch)^d` per naked cistron; queued ribosomes shield rut sites | termination pressure GROWS with naked runway (direction fixed by the loop-10 jury); translation shields the failure point | catch probability is a per-cistron draw, not helicase kinetics; no RNAP velocity | REAL + APPROX |
| 14 | Transcriptional polarity | per-member yield factor `surv + (1−surv)·polarity` | upstream loss scales downstream yield, compounding | default 0.5 polarity constant | REAL + APPROX |
| 15 | RBS strength gradients | per-cistron `rbs r` multiplier on `translates` rates | the lacZYA stoichiometric gradient (Z > Y > A) | linear multiplier on a 0..1 tier | REAL + APPROX |
| 16 | The repressilator (Elowitz–Leibler 2000) | 3-repressor ring, 20 substeps of dt=0.05 per tick, h=4, `basal` knob, seeded noise kick | topology; robust oscillation; V2 pins peak-to-peak = 6 ticks against the published discrete parameterization; `basal` exists because real repressed promoters leak | one-state protein-only reduction of the published two-state (mRNA+protein) model; no basal term in the original reduction | REAL + APPROX |
| 17 | Telegraph promoters / transcriptional bursting | `@burst kon koff` two-state Markov promoter, seeded per cell | promoters DO switch states; bursting IS the phenomenology; per-gene (kon,koff) IS promoter identity; worker RNG streams decorrelate cells (extrinsic-noise mirror) | two-state Markov, discrete rounds; seeded ⇒ deterministic replay | REAL + APPROX |
| 18 | Quorum sensing (LuxI/LuxR, agr, AI-2) | `autoinducer ahl;` + `secrete` / `quorum` / `quench` | signal is EXTRACELLULAR and SHARED — my secretion raises your activation; density threshold; integer counts make concurrent secretions commute | process-global pool, no diffusion geometry, no signal chemistry | REAL + ABSTRACTION |
| 19 | Small-molecule allostery (lac inducer, trp corepressor) | `ligand x;` + `bind tf inducer\|cofactor lg k v;` | ligand modulates binding AFFINITY, never the regulator's level (the §3 polarity discipline) | two-tier binding math | REAL + APPROX |
| 20 | Thermodynamic occupancy repression | `occupy` edges, multiplicative survival `Π(1 − influence)` | repression cannot overshoot; full occupancy = full silencing; V7 pins the Kⁿ/(Kⁿ+Rⁿ) reduction | thermodynamic derivation reduced to the multiplicative form | REAL + APPROX |
| 21 | Enhanceosomes / super-enhancers | `enhance` clusters, `sum` pooling edges, dose via `.cell enhance.delta` | cooperative pooling; `sum` with `hill > 1` is super-additive | flat dose = "lower effective threshold", not a binding-site model | REAL + ABSTRACTION |
| 22 | TF sequestration / decoy sites | `decoy d for tf capacity c;` | a sponge absorbs regulator without producing output; emptying restores gates | deterministic free-fraction subtraction `max(0, level − c·level_d)`; no diffusion | REAL + APPROX |
| 23 | Epigenetic maintenance vs dilution | `passage(n)` + `.cell methyl.maintenance` | DNMT1-style maintenance copies marks; without it marks dilute ~50%/generation; half-down rounding so dilution never reads as MORE repressed | single maintenance dial, not per-site copying; 10⁶-division clamp | REAL + APPROX |
| 24 | Gene dosage / copy-number variation | `@copies n` (1..=64) | dose amplifies the CONCENTRATION feeding GRN edges, saturating on the 0..1 lattice; copies never touch return values | no recombination, no dosage-dependent expression noise | REAL + APPROX |
| 25 | Two-tier expression (transcript → protein) | `translates` layer: proteins lag and smooth transcript bursts | the two-tier shape of real expression | the loop-8 verdict: the honest reduction ENDS here — no ribosome counting, no folding, no transport | ABSTRACTION (declared) |

Two structural remarks the table should leave you with. First, the language consistently mirrors
*polarity* — which ligand turns which regulator which way — because that is where real biology is
falsifiable and where shallow metaphors usually get it wrong (the §3 lac walk-through is the
showcase). Second, the consistent "not modeled" pattern is *kinetics at the molecular scale*:
RNAP velocities, ribosome codon times, helicase translocation, hairpin folding. Every one of
those is replaced by a discrete threshold draw whose *outcome curve* is pinned against literature
where a dataset exists (V1 EC50 points, V2 period, V5 classes, V7 occupancy points, V8 dose
saturation, the Rho per-cistron catch curve).

---

## 6. The honest divergences (where the metaphor is deliberately false)

1. **A gene is a function, not a DNA region.** Calling `lacZ()` evaluates a body and returns a
   value; it does not produce a molecule. The biology mapping is: one call attempt = one
   transcription attempt; its return value = the per-transcript product. That unit choice is
   documented (MN-@copies: "a call is a transcription event; its return is the per-transcript
   product") and it is a language decision, not a claim about cells.
2. **Levels are fractions, not concentrations.** 0..1 clamped, normalized. `grn_state()` is an
   Operon state vector; a reading of 0.7 does not mean 0.7 µM of anything.
3. **Time is discrete and semantic.** `grn_fire` steps, repressilator ticks, burst rounds, and
   decay clocks count calls and rounds — not seconds, not cell-cycle phases. The call-clock
   cadence is an explicit design choice (MN-decay_clock): genes are closures the programmer
   invokes; a Gillespie-style event scheduler would invert the language's own metaphor.
4. **Determinism beats realism.** Where real expression is stochastic, Operon seeds the noise
   (DETERMINISM.md §4): same seed, same bytes, every run, every platform. The repressilator even
   ships a deterministic-noise mode so byte-parity holds across engines. The cost: variance
   statistics from a single run mean nothing; the seeded replays are the honest unit of evidence.
5. **Epigenetic state is not automatically heritable.** Marks persist across *passages* only with
   the maintenance dial configured (`.cell methyl.maintenance`, 1.0 = perfect DNMT1-style
   copying); the default null model is dilution — exactly like real marks without maintenance
   machinery, and exactly unlike the common "methylation = permanent" shorthand.
6. **"Homeostasis" is banned where no setpoint exists.** Persistence is a latch; decay is
   dilution; neither is homeostasis (reg-r3 term audit). This is the discipline that keeps the
   vocabulary from overclaiming: names anchor intuition, they do not assert mechanisms
   (D-008's "names are anchors, not claims").
7. **The sandbox is biological in metaphor, computer-science in teeth.** `stress { } rescue { }`
   mirrors the UPR/ISR containment programs and the membrane is deny-by-default — but the
   enforcement is a real interpreter sandbox (fuel, membranes, caps), not a simulation of
   molecular containment. Proof frames mirror *overlapping reading frames* (tests and code in
   one sequence) — a structural metaphor; kinetic proofreading (Hopfield-style fidelity
   amplification) is NOT claimed and NOT modeled.
8. **No genome as first-class data.** Sequence utilities (`transcribe`, `translate`,
   `reverse_complement`, `find_orf`, `codon()`, the C++ `distance`/`similar` kernels) operate on
   demo strings; CRISPR tooling operates on demo sequences. The language models *expression
   control*, not genomics.
9. **Eukaryotic and prokaryotic mechanisms coexist by design.** Real biology separates these
   (polycistronic operons are bacterial; splicing is eukaryotic). Operon offers both in one
   language because the goal is expressiveness of the control plane, not organism fidelity —
   and each construct is tagged with its domain of validity (MN-operon-unit, 13a).

---

## 7. The fidelity scorecard

Counting the 25 live mechanisms of `BIO-CONTRACT.md` §2 (inventory basis noted there):

- **Primary grades**: 23/25 mechanisms carry **REAL** for the aspect they model (direction,
  gating, or causality). The two exceptions are declared, not hidden: the Hill edge is graded
  APPROX at primary (a curve family, not a mechanism), and the two-tier transcript/protein state
  is graded ABSTRACTION as the explicit loop-8 design verdict.
- **Secondary (how-side) grades**: 16 APPROX, 8 ABSTRACTION, 1 SIMPLIFICATION.
- **Numerical pins**: 8+ literature-anchored validation entries (V1 Hill/EC50 points, V2
  repressilator period, V4 trpR allostery state, V5 riboswitch classes, V7 occupancy curve,
  V8 dosage saturation, the Rho per-cistron catch dataset, the repressilator basal floors) with
  tolerances at `1e-12` where the derivation is exact.

How to read this honestly: the scorecard measures *faithfulness of direction and shape*, not
predictive power. A mechanism graded REAL + APPROX behaves like the biology at the level a
programmer reasons about — "induction relieves repression", "occupancy cannot overshoot", "naked
runway raises termination pressure" — while a mechanism graded REAL + ABSTRACTION keeps the
causality but replaces the mechanism with a language-defined device. Nothing in the language is
graded as if it were wet-lab evidence, and BIO-CONTRACT §3 forbids presenting outputs as
biological truth.

---

## 8. What real biology has that Operon does not model (at all)

- **Molecular-scale kinetics**: RNAP velocity, ribosome codon-transit times, Rho helicase
  translocation, hairpin folding thermodynamics, rut-site sequence strength, tmRNA/SsrA rescue.
- **Molecular inventory**: no metabolite chemistry beyond named ligand pools, no energy carrier
  accounting (ATP), no resource allocation between expression and growth (fuel and
  `stress/rescue` are containment approximations, not metabolism).
- **Chromatin as a polymer**: no nucleosome positioning, no 3D contact maps, no
  compartmentalization; TADs exist as `tad` domains with CTCF-style `anchor` semantics only.
- **Intrinsic vs extrinsic noise separation**: extrinsic (cell-to-cell) decorrelation is modeled
  via per-worker RNG derivation; intrinsic (same-gene burst variance) exists only through the
  seeded telegraph promoter — no joint decomposition, no Gillespie trajectories.
- **Evolution**: no mutation-selection dynamics across passages; `passage` dilutes marks, it
  does not evolve genomes.
- **Cell cycle and growth**: division is `passage(n)`; there is no replication timing, no
  sequestration window (Dam/SeqA-style), no size control.
- **Spatial structure**: quorum sensing is a well-mixed global pool by declared design; no
  gradients, no compartments beyond the cell abstraction, no transport.
- **Post-translational regulation**: protein modification (phosphorylation cascades,
  ubiquitination) has no layer; the two-tier state is the declared reduction floor.

Each of these is a boundary the repo documents rather than fudges: the loop-8 "honest reduction
ends here" verdict, the syntax-freeze discipline (BIO-CONTRACT §4 — new biology needs a written
justification and a contract row), and the per-mechanism not-modeled lists in MODELING-NOTES are
the governance that keeps this list true.

---

## 9. Why the divergences are the right call for a programming language

A programming language has invariants a cell does not. Operon's governing contract is
**byte-identical behavior** — same program, same seed, same bytes on tree-walk interpreter,
bytecode VM, both engines, every platform, every optimizer setting (the differential harness,
3520+ corpus programs across the axes). Real expression is inherently stochastic and
environment-sensitive; a faithful *kinetic* simulation would break the parity guarantees that
make the language reliable. So Operon draws the line at: **borrow the control structures and
their polarity curves; seed (or remove) the noise; declare every departure.** The result is a
language where a systems-biology intuition ("this service should gate like lac: induction plus
cofactor") becomes a deterministic program with enforcing tests, and where the vocabulary stays
honest because every term audit can point at the exact mechanism it describes. D-008 states the
final rule plainly: biology is an intuition aid, never a prerequisite — a CS engineer who has
never seen a western blot can read `regulate`/`translates`/`silence` programs and be correct.

---

## 10. Run the evidence yourself

```bash
bash scripts/build.sh                                   # build ./bin/operon (Rust) and the Python oracle
./bin/operon test tests/lac_gate.op                     # §3: dual control, allostery-not-dilution
./bin/operon test tests/operon_unit.op                  # §4: unit gate, RBS gradient, polarity
./bin/operon test tests/repressi_params.op              # row 16: the published parameterization
./bin/operon test tests/repressi_alpha.op               # row 16: alpha sweep
./bin/operon test tests/riboswitch_cis.op               # row 10: cis enforcement + off/on classes
./bin/operon test tests/grn_occupy.op                   # row 20: occupancy cannot overshoot
./bin/operon test tests/copies_dose.op                  # row 24: dose saturation
./bin/operon test tests/grn_bistability.op              # §1.7: cooperativity-dependent partial bands
python3 bootstrap/validation_report.py --entry V2       # the repressilator period pin
python3 bootstrap/validation_report.py --entry V1       # the Hill/EC50 pin
python3 bootstrap/validation_report.py --entry V5       # the riboswitch class pin
```

Every command above is part of the standing test/validation sets; the VALIDATION entries print
their independent re-derivation (e.g., V2's alpha-surface value is re-folded in plain Python and
compared bit-for-bit). Cross-checks: `docs/spec/BIO-CONTRACT.md` for the canonical grades,
`docs/spec/MODELING-NOTES.md` for the term audits and the §3 map this document narrates, and
`docs/spec/VALIDATION.md` for the numerical pins and their tolerances.
