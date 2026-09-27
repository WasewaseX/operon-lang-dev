# VALIDATION, the scientific validation layer (W093)

Every biological model in Operon is a MODEL, useful exactly to the degree
its relationship to the modeled mechanism is explicit. The modeling contract
(`docs/spec/BIO-CONTRACT.md`) labels each mechanism REAL / APPROXIMATION /
ABSTRACTION / FICTION; this file is the NUMERICAL half of that contract: for
each model where the literature defines expected behavior, it records the
source model, the mapping into Operon's parameterization, the numerical
tolerance, and the test that enforces it.

## Rules

1. **One row per validated model**, a claim without a test row is a wish;
   a test without a row is an undocumented constraint. Both are bugs.
2. **Tolerances are absolute** (`abs(actual - expected) < eps`) with the eps
   recorded per row. `1e-12` rows are bit-order-exact: the engine computes
   them with the same op order the expectation is written in (repeated
   multiplication, never `powi`, the W090 IEEE-754 parity contract).
3. **Tolerance idiom**, until `std/testing.op` grows a first-class
   `approx_eq(a, b, eps)` helper (dev-2 std lane, tracked below), tests use
   the `abs(x - e) < eps` idiom directly; this file is the registry of which
   eps each test's idiom binds.
4. **Fails-pre discipline**, every validation test fails on a core that
   changed the underlying model (integration order, gate semantics, curve
   form), not just on a wrong constant. A validation row that cannot fail
   is decoration, not validation.

## Registry

| # | Model (Operon surface) | Source model | Mapping | Tolerance | Enforcing test |
|---|------------------------|--------------|---------|-----------|----------------|
| V1 | Hill dose-response, thresholded activating edge (`regulate { a activates b strength 1.0 threshold t }`, n=2 default) | Hill–Langmuir input function; EC50 = half-max point (Alon, *An Introduction to Systems Biology*, ch. 2) | `influence = s·Lⁿ/(Lⁿ + tⁿ)`, n=2 → threshold t IS the EC50; canonical points t/3→10%, t/2→20%, t→50%, 2t→80%, 3t→90% (the 10–90 span) | `1e-12` (op-order exact) | `tests/sci_ec50_hill.op` |
| V2 | Repressilator ring period (`repressilator p -> q -> r`, T2d discrete integration) | Elowitz & Leibler (2000), *Nature* 403:335, synthetic three-repressor ring | `dA/dt = α/(1+R⁴) − γA` (α=10, γ=1, n=4), 20 Euler substeps × dt=0.05 per ring tick → 1 tick = 1.0 dimensionless time; peak-to-peak period of node p = **6 ticks** (stable across ≥4 consecutive cycles after 3-tick warmup) | period pinned exactly via peak-count-in-window (`peaks.len() == 5` in a 30-tick window fails for period 5 or 7); amplitudes pinned stateless-fold-identical (W089) only, relaxation from the initial condition is documented, not a published-number claim | `tests/sci_repressi_period.op` |
| V3 | Ring kinetics parameters (`repressilator ... alpha a gamma g hill n basal b noise s seed k`) | same as V2, parameter overlay layers onto the shared engine state; last declaration wins per field | documented in SPEC §11 (repressilator row); differential tests pin parameter plumbing | op-order exact (oracle-mirrored) | `tests/repressi_params.op`, `tests/repressi_alpha.op` |
| V4 | trp attenuation (`attenuates` edge, leader-peptide/terminator-hairpin outcome) | prokaryotic transcription attenuation (trp operon, Yanofsky 1981) | modeled as an RNA-level veto with dose threshold, mechanism labeled APPROXIMATION in BIO-CONTRACT (no anti-terminator structure modeling) | behavioral (containment/pass assertions) | `tests/trp_attenuator.op` |
| V5 | cis riboswitch (`@riboswitch(ligand, sense, threshold)`) | cis-acting metabolite-binding aptamers: TPP/purine/SAM `off` class vs adenine/glycine `on` class (Serganov & Nudler 2013 review) | per-gene 5'UTR sensor over a cell-wide metabolite pool; bound-state polarity per class | behavioral | `tests/riboswitch_cis.op` |
| V6 | stoichiometric RISC silencing (`silence old -> new strength s sites n`) | RNAi dose-response: per-site capture probability with multiplicative site composition, survival = (1−s)ⁿ | `sites n` clamped 1..=64, `strength s` clamped 0..=1; `strength 1.0` × 1 site = legacy binary redirect (no entropy draw) | op-order exact | `tests/silence_dose.op` |
| V7 | occupancy repression (`occupy` inhibiting edges, reg-bio-2 D2b) | thermodynamic occupancy survival Kⁿ/(Kⁿ+Rⁿ), repression cannot overshoot, full occupancy = full silencing | multiplicative composition `Π(1 − influence)` vs the legacy single subtraction; both curves pinned at the 0.5/0.6-class points | `1e-12` | `tests/grn_occupy.op` |
| V8 | gene dosage (`@copies n`, reg-bio-3 C10) | copy-number amplification of transcript concentration feeding GRN edges (not return values) | copies clamp 1..=64 at parse; amplification pinned at the level layer | behavioral | `tests/copies_dose.op` |

## Follow-ups (tracked, not yet validated)

- `std/testing.op` `approx_eq(a, b, eps)` first-class helper, dev-2 std lane
  (kept out of the W093 wave to avoid colliding with the open tooling PR).
- Two-tier translation (C1) period/amplitude reproduction against a published
  incoherent-FFL pulse-window parameterization (DinJ–YafN analog), needs a
  citable discrete parameterization before a number can be pinned; the
  behavioral window is already pinned in `tests/ffl_incoherent_pulse.op`.
- Quorum-sensing medium (C8) and ligand bath (A4) dose curves, sources
  identified, tolerances not yet pinned.
- Rho termination catch probability (loop-10), the entropy-stream parity is
  differentially pinned (`bootstrap/harness.py` granted targets); the
  per-cistron catch curve against a published Rho-attenuation dose dataset
  is open.
