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
   The TOLERANCE FRAMEWORK section below (W093 remainder) formalizes this
   per entry, adds the `exact` kind for discrete quantities, and pins the
   machine-readable registry (`bootstrap/validation_registry.json`) as the
   single source of truth for every magnitude.
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

## TOLERANCE FRAMEWORK (W093 remainder)

The Registry above says WHICH test enforces WHICH model; this section says
HOW TIGHT each claim is and WHY that tightness is honest. Every quantity in
the generated tables below carries its expected value with the source it is
derived from, a comparison kind, a tolerance magnitude, and a repro command
that re-derives the value end-to-end on this tree.

**The determinism premise.** The engine is deterministic: same binary
version + same source + same seed + same flags + same `.cell` gives
byte-identical stdout (`docs/spec/DETERMINISM.md` S1), and every numeric row
below uses only `+ - * /` in a fixed op order (bit-reproducible across the
release matrix, DETERMINISM S5). There is no run-to-run jitter to absorb.
Tolerances therefore exist to guard FUTURE REFACTORS: each magnitude is
chosen wide enough that a legitimate re-association of floating-point
operations (a few ulps) still passes, and many orders of magnitude narrower
than any change to the modeled system itself (curve form, constants,
integration order, gate semantics).

**Comparison kinds** (full definitions live in the registry's framework
block, mirrored here):

- `absolute`, pass iff `abs(observed - expected) < tolerance`. The house
  value is `1e-12`: expectations are written in the same op order the engine
  computes (repeated multiplication, never `powi`, the W090 IEEE-754 parity
  contract), so the true numerical noise floor against the derivation is
  about one ulp (1e-16 class for O(1) quantities). 1e-12 leaves four orders
  of magnitude of refactor headroom while staying far below the smallest
  modeled quantity.
- `relative`, pass iff `abs(observed - expected) < tolerance * abs(expected)`.
  For quantities whose scale is parameter-dependent. Not needed by any
  current row.
- `significant-figure`, pass iff observed and expected agree to N
  significant digits. The right kind for future rows whose expectation is a
  value PUBLISHED with limited digits (dose-curve datasets), where quoting
  more digits than the source would be false precision. Not needed by any
  current row.
- `exact`, pass iff observed and expected are identical (booleans,
  integers, and floats compared via their formatted value, bit-identical).
  For DISCRETE quantities: counts, booleans, peak grids, and stream values
  of the pure-integer xorshift. No numerical slack is defensible for a
  deterministic discrete quantity.

**Single source of truth.** The per-entry tables are GENERATED from
`bootstrap/validation_registry.json`; that file is the only place tolerance
constants live. `bootstrap/validation_report.py` consumes it (it re-runs
every enforcing test on both cores, runs the measurement probes, compares
observed vs expected, and emits a machine-readable JSON report), and
`--check-doc` re-verifies this generated block against the registry, so
this section cannot drift from the registry. The test files keep their
enforcing asserts; if a test file and the registry ever disagree, the
report goes red and the live run decides.

<!-- BEGIN GENERATED: tolerance-framework (source: bootstrap/validation_registry.json; regenerate: python3 bootstrap/validation_report.py --emit-table; drift gate: python3 bootstrap/validation_report.py --check-doc) -->
### V1, Hill dose-response, thresholded activating edge (n=2): the EC50 identity and the five canonical curve points

Source: Hill-Langmuir input function; EC50 = the half-max point (Alon, An Introduction to Systems Biology, ch. 2). Mapped by SPEC section 11: influence = strength * level^n / (level^n + threshold^n), n=2 default, so threshold t IS the EC50.

| Quantity | Expected | Comparison | Tolerance | Derivation |
|---|---|---|---|---|
| `v1_t3` | `0.1` | absolute | `1e-12` | level = t/3 = 0.1: 1/(1+9) = 0.1 (10% of max) |
| `v1_t2` | `0.2` | absolute | `1e-12` | level = t/2 = 0.15: 1/(1+4) = 0.2 (Hill-2 quarter point, NOT 10%) |
| `v1_t1` | `0.5` | absolute | `1e-12` | level = t = 0.3: 1/(1+1) = 0.5 (the EC50 identity: level == threshold is the half-max point) |
| `v1_2t` | `0.8` | absolute | `1e-12` | level = 2t = 0.6: 4/(4+1) = 0.8 |
| `v1_3t` | `0.9` | absolute | `1e-12` | level = 3t = 0.9: 9/(9+1) = 0.9 (the 10-90 span complete) |

Why this magnitude is defensible: The expectations are the closed form of the n=2 curve at the five canonical points (t/3, t/2, t, 2t, 3t with t = 0.3, strength 1.0), and the engine evaluates strength * L*L / (L*L + t*t) in exactly the op order the expectation is written in (repeated multiplication, no powi, W090). Live worst case is one ulp (the t/3 point reads 0.10000000000000002 against 0.1, a 1.4e-17 gap), so 1e-12 absolute is a refactor guard, not a jitter allowance. Fails-pre: any change to the influence formula, the gate semantics, or the level-latch behavior.

Enforcing tests: `tests/sci_ec50_hill.op`. Measurement probes: `bootstrap/validation_probes/v1_hill.op`.

Repro (end-to-end): `bash scripts/build.sh && ./bin/operon test tests/sci_ec50_hill.op && python3 bootstrap/validation_report.py --entry V1`

### V2, Repressilator ring period (repressilator p -> q -> r, T2d discrete integration): peak-to-peak period 6 ring ticks

Source: Elowitz & Leibler (2000), Nature 403:335, a synthetic three-repressor ring. Mapped by SPEC section 11 (T2d): dA/dt = alpha/(1+R^4) - gamma*A (alpha=10, gamma=1, h=4), 20 Euler substeps of dt=0.05 per ring tick, so one tick = 1.0 dimensionless time unit.

| Quantity | Expected | Comparison | Tolerance | Derivation |
|---|---|---|---|---|
| `v2_peak_count` | `5` | exact | bit-identical | local maxima of node p in the 30-tick window (ticks 6, 12, 18, 24, 30): 5 peaks = peak-to-peak period 6 ticks = 6.0 dimensionless time units |
| `v2_period_ticks_ok` | `true` | exact | bit-identical | every consecutive peak-index distance equals 6 ticks (structural period check, not just a count) |
| `v2_stateless_fold` | `true` | exact | bit-identical | repressi_state() twice at the same tick: identical p, q, r (the W089 stateless-fold determinism contract) |

Why this magnitude is defensible: The period is a DISCRETE structural quantity: 5 local maxima of node p in a fixed 30-tick window after a 3-tick warmup, consecutive peaks exactly 6 ticks apart (period 5 or 7 would give 6 or 4 peaks and fail). The Euler fold uses only +, -, *, / with a fixed op order, so the peak grid is bit-reproducible and no numerical slack is defensible: exact. Peak amplitudes are deliberately NOT pinned: they are still relaxing toward the limit cycle from the initial condition, a documented property, not a published-number claim (the enforcing test pins amplitudes only as stateless-fold identity, the W089 determinism contract). Fails-pre: a drifted period (integration changed), a hardcoded drive schedule, or a stateful fold.

Enforcing tests: `tests/sci_repressi_period.op`. Measurement probes: `bootstrap/validation_probes/v2_period.op`.

Repro (end-to-end): `bash scripts/build.sh && ./bin/operon test tests/sci_repressi_period.op && python3 bootstrap/validation_report.py --entry V2`

### V3, Ring kinetics parameters (alpha, gamma, hill, basal, noise, seed): the alpha surface and the basal-leak surface

Source: Same system as V2 (Elowitz & Leibler 2000) with the kinetic overlay documented in SPEC section 11; the defaults (alpha=10, gamma=1, h=4, basal=0, noise=0) are bit-identical to the historical constants.

| Quantity | Expected | Comparison | Tolerance | Derivation |
|---|---|---|---|---|
| `v3_alpha20_y2` | `3.6355225619570213` | absolute | `1e-12` | alpha=20 ring, node y after 2 ticks (the enforcing test's window is 3.6 < y < 3.7; the exact fold value is 3.6355225619570213) |
| `v3_basal_min_p` | `0.794555574088881` | absolute | `1e-12` | basal=0.5 ring, minimum of node p over 12 ticks (independent fold: 0.794555574088881) |
| `v3_basal_min_q` | `0.36236427270474075` | absolute | `1e-12` | basal=0.5 ring, minimum of node q over 12 ticks (independent fold: 0.36236427270474075) |
| `v3_basal_min_r` | `0.8578301366963166` | absolute | `1e-12` | basal=0.5 ring, minimum of node r over 12 ticks (independent fold: 0.8578301366963166) |
| `v3_basal_above_floor` | `true` | exact | bit-identical | all three minima above the 0.2 behavioral floor (the enforcing test's claim, mirrored exactly) |

Why this magnitude is defensible: The alpha-surface expectation is re-derived INDEPENDENTLY from the documented parameterization (alpha=20, gamma=1, h=4, 20 Euler substeps of dt=0.05, Jacobi update from the snapshot, init [5,0,0]): a plain Python fold of the published system gives 3.6355225619570213 and the live engine gives 3.6355225619570213, agreement to the last bit, so 1e-12 absolute guards refactors only. The basal minima carry the same derivation status (independent fold agrees bit-for-bit); the behavioral floor (every minimum above 0.2, repressed promoters never reach zero) mirrors the enforcing test as an exact boolean. Fails-pre: any change to the fold, the parameter plumbing, or the basal term.

Enforcing tests: `tests/repressi_params.op`, `tests/repressi_alpha.op`. Measurement probes: `bootstrap/validation_probes/v3_alpha.op`, `bootstrap/validation_probes/v3_basal.op`.

Repro (end-to-end): `bash scripts/build.sh && ./bin/operon test tests/repressi_params.op && ./bin/operon test tests/repressi_alpha.op && python3 bootstrap/validation_report.py --entry V3`

### V4, trp attenuation (attenuates edge) and corepressor polarity: outcome contract

Source: Prokaryotic transcription attenuation (trp operon, Yanofsky 1981); the corepressor polarity (binding INCREASES repressor DNA affinity) is the lac inverse. Mechanism labeled APPROXIMATION in docs/spec/BIO-CONTRACT.md (RNA-level veto with dose threshold, no anti-terminator structure modeling).

| Quantity | Expected | Comparison | Tolerance | Derivation |
|---|---|---|---|---|
| `v4_scarcity_trpE_on` | `true` | exact | bit-identical | pool 0.02: occupancy 0.02/0.12 < threshold 0.3, the aporepressor state, trpE expresses |
| `v4_scarcity_leader_on` | `true` | exact | bit-identical | pool 0.02 < attenuator threshold 0.5: the leader reads through |
| `v4_abundance_trpE_suppressed` | `true` | exact | bit-identical | pool 0.9: the cofactor binds, the inhibits edge vetoes the call |
| `v4_abundance_leader_suppressed` | `true` | exact | bit-identical | pool 0.9 >= attenuator threshold 0.5: the leader transcript terminates |
| `v4_trpR_unchanged` | `true` | exact | bit-identical | the regulator LEVEL never moved (allostery acts on the binding state, not dilution) |
| `v4_pool_readable` | `0.9` | absolute | `1e-12` | ligand("trp") reads back the stored 0.9 (pass-through value) |

Why this magnitude is defensible: This row's contract is behavioral: the observable is which calls express (a return value) and which are vetoed (null). Booleans compared exact, because a tolerance on a discrete outcome is meaningless. The one numeric quantity (pool readback after ligand_set 0.9) is a pass-through of a stored value, so 1e-12 absolute is generous and only guards a refactor of the pool plumbing. Fails-pre: a polarity flip (inducer semantics on a corepressor), a lost attenuates veto, or a gate-order change.

Enforcing tests: `tests/trp_attenuator.op`. Measurement probes: `bootstrap/validation_probes/v4_trp.op`.

Repro (end-to-end): `bash scripts/build.sh && ./bin/operon test tests/trp_attenuator.op && python3 bootstrap/validation_report.py --entry V4`

### V5, Cis riboswitch (@riboswitch ligand class threshold): per-gene 5'UTR sensor, off/on polarity and gate order

Source: Cis-acting metabolite-binding aptamers: TPP/purine/SAM off class vs adenine/glycine on class (Serganov & Nudler 2013 review). Per-gene sensor over a cell-wide metabolite pool; gate order methylation -> riboswitch -> promoter.

| Quantity | Expected | Comparison | Tolerance | Derivation |
|---|---|---|---|---|
| `v5_off_bound_suppressed` | `true` | exact | bit-identical | off class, pool 0.9 >= 0.5: terminator hairpin folds, call suppressed |
| `v5_off_released_expresses` | `true` | exact | bit-identical | off class, pool 0.1 < 0.5: hairpin released, transcript expresses |
| `v5_on_unbound_suppressed` | `true` | exact | bit-identical | on class, pool 0.1: RBS sequestered, call suppressed |
| `v5_on_bound_expresses` | `true` | exact | bit-identical | on class, pool 0.9: RBS exposed, transcript expresses |
| `v5_cis_plain_ignores_pool` | `true` | exact | bit-identical | the CIS property: the unmarked gene expresses under the same bound pool |
| `v5_cis_marked_responds` | `true` | exact | bit-identical | the CIS property: the marked gene under the same pool is suppressed |
| `v5_acetylate_no_immunity` | `true` | exact | bit-identical | gate order: acetylation (chromatin permission) does not immunize against the RNA hairpin |
| `v5_hairpin_released_expresses` | `true` | exact | bit-identical | pool released: the acetylated gene expresses normally |

Why this magnitude is defensible: Behavioral row: the contract is the bound-state polarity per class plus the cis property (an unmarked gene under the SAME pool is unaffected) plus the gate order (acetylation does not immunize against an RNA-level hairpin). All observables are discrete express/suppress outcomes, so every row is an exact boolean; a numeric tolerance would be decoration. Fails-pre: a polarity inversion (on/off classes swapped), a pool-wide (trans) leak into the unmarked gene, or a gate-order change.

Enforcing tests: `tests/riboswitch_cis.op`. Measurement probes: `bootstrap/validation_probes/v5_riboswitch.op`.

Repro (end-to-end): `bash scripts/build.sh && ./bin/operon test tests/riboswitch_cis.op && python3 bootstrap/validation_report.py --entry V5`

### V6, Stoichiometric RISC silencing (silence old -> new strength s sites n): multiplicative dose-response on a seeded stream

Source: RNAi dose-response: per-site capture probability with multiplicative site composition, survival = (1-s)^n. sites clamped 1..=64, strength clamped 0..=1; strength 1.0 with one site is the legacy binary redirect with no entropy draw.

| Quantity | Expected | Comparison | Tolerance | Derivation |
|---|---|---|---|---|
| `v6_capture_count` | `16` | exact | bit-identical | 20 calls at p = 1-(1-0.5)^2 = 0.75 under randomize(42): the deterministic capture count is 16 (parity pin) |
| `v6_binary_captured` | `true` | exact | bit-identical | strength 1.0, one site: capture WITHOUT any entropy draw (legacy binary redirect) |
| `v6_no_draw_binary` | `true` | exact | bit-identical | random() before and after the binary capture, same seed: identical value, the stream is untouched |
| `v6_stream_value` | `1.2732925824820995e-11` | exact | bit-identical | first random() after randomize(7): a pure-integer-stream value, bit-identical on every platform (DETERMINISM section 4) |
| `v6_immune_expresses` | `true` | exact | bit-identical | @acetylate immunity is checked BEFORE any draw: the immune transcript expresses |
| `v6_no_draw_immune` | `true` | exact | bit-identical | random() before and after the immune call, same seed: identical value, no entropy spent |

Why this magnitude is defensible: The capture count (16 of 20 calls at p = 1-(1-0.5)^2 = 0.75) is a PARITY PIN of the shared mirrored xorshift64* stream under randomize(42), not a statistic over random draws: the stream is pure integer arithmetic (DETERMINISM section 4), so the count is exact and no other tolerance is defensible. The stream value row (first draw after randomize(7)) pins the same property bit-identically. The no-draw rows pin entropy-stream discipline (binary capture and the @acetylate immune check must not consume a draw): exact booleans. Fails-pre: an entropy-stream change (a breaking change per DETERMINISM section 8), a lost immune pre-check, or a composition change.

Enforcing tests: `tests/silence_dose.op`. Measurement probes: `bootstrap/validation_probes/v6_silence.op`.

Repro (end-to-end): `bash scripts/build.sh && ./bin/operon test tests/silence_dose.op && python3 bootstrap/validation_report.py --entry V6`

### V7, Occupancy repression (occupy inhibiting edges): multiplicative thermodynamic survival K^n/(K^n+R^n)

Source: Thermodynamic occupancy survival: repression composed multiplicatively (each repressor contributes a survival factor 1 - influence), so it cannot overshoot and full occupancy approaches but never crosses zero. The legacy subtractive form (1 - sum of influences) is pinned alongside as the contrast.

| Quantity | Expected | Comparison | Tolerance | Derivation |
|---|---|---|---|---|
| `v7_sub_partial` | `0.2` | absolute | `1e-12` | subtractive legacy: 1 - 0.4 - 0.4 = 0.2 (live reads 0.19999999999999996, one-ulp class) |
| `v7_occ_partial` | `0.36` | absolute | `1e-12` | occupancy survival: 1 * 0.6 * 0.6 = 0.36 |
| `v7_sub_overshoot` | `0.0` | exact | bit-identical | two near-full subtractive repressors overshoot and clamp to a hard 0.0 (discrete clamp outcome) |
| `v7_occ_survival` | `9.802960494069226e-05` | absolute | `1e-12` | occupancy near-full survival: (1 - 1/1.01)^2, approaches but never crosses zero |

Why this magnitude is defensible: The expectations are closed forms: two partial repressors (level 1.0, strength 0.5, threshold 0.5, influence 0.4 each) give survival 1 * 0.6 * 0.6 = 0.36 and the legacy subtractive form gives 1 - 0.4 - 0.4 = 0.2; two near-full repressors (influence 1/(1+0.01) each) give survival (1 - 1/1.01)^2 (hand-derived 9.802960494069213e-05, live 9.802960494069226e-05, a 1.3e-17 gap) and the subtractive form overshoots to a hard-clamped 0.0. The engine composes in the derivation's op order, so observed gaps are one-ulp class; 1e-12 absolute guards re-association only. The hard-zero row is exact (the subtractive overshoot is a clamp, not a small number). Fails-pre: a composition change (multiplicative to subtractive or back), an overshoot into negatives, or an influence-formula change.

Enforcing tests: `tests/grn_occupy.op`. Measurement probes: `bootstrap/validation_probes/v7_occupy.op`.

Repro (end-to-end): `bash scripts/build.sh && ./bin/operon test tests/grn_occupy.op && python3 bootstrap/validation_report.py --entry V7`

### V8, Gene dosage (@copies n): read-side dose amplification on GRN edges

Source: Copy-number amplification of transcript concentration feeding GRN edges (not return values): n copies at level L read as min(1, n*L) to their targets; copies clamp 1..=64 at parse.

| Quantity | Expected | Comparison | Tolerance | Derivation |
|---|---|---|---|---|
| `v8_single_vetoed` | `true` | exact | bit-identical | single copy at level 0.3 < threshold 0.5: the gate vetoes the call |
| `v8_raw_single` | `0.3` | absolute | `1e-12` | grn_get("src") reads back 0.3: copies never touch the raw level (pass-through) |
| `v8_double_gate_open` | `true` | exact | bit-identical | two copies: 0.3 x 2 = 0.6 >= 0.5, the gate opens (dose is a READ-side effect) |
| `v8_saturated_gate_open` | `true` | exact | bit-identical | 64 copies at 0.02: 1.28 saturates at the 1.0 lattice ceiling, the gate still passes |
| `v8_raw_saturated` | `0.02` | absolute | `1e-12` | grn_get("src") still reads 0.02: saturation happens on the dose read, not the stored level |
| `v8_reset_vetoed` | `true` | exact | bit-identical | a plain redefinition restores single-copy dose: 0.3 < 0.5 is vetoed again |

Why this magnitude is defensible: The dose effect is observable as discrete gate outcomes (vetoed vs passes at threshold 0.5), so those rows are exact booleans: 0.3 single-copy is vetoed, 0.3 x 2 = 0.6 opens the gate, 0.02 x 64 saturates and still passes, and a plain redefinition restores the veto. The raw-level readbacks are pass-through stored values (copies never touch the raw level), so 1e-12 absolute is generous and only guards the readback plumbing. Fails-pre: dose applied to return values, to the raw level, or lost saturation/clamp.

Enforcing tests: `tests/copies_dose.op`. Measurement probes: `bootstrap/validation_probes/v8_copies.op`.

Repro (end-to-end): `bash scripts/build.sh && ./bin/operon test tests/copies_dose.op && python3 bootstrap/validation_report.py --entry V8`
<!-- END GENERATED: tolerance-framework -->

**How the report runs** (`python3 bootstrap/validation_report.py`, stderr
carries the human summary, stdout carries the JSON):

1. Enforcing tests: every entry's test file runs through BOTH normal
   runners (`./bin/operon test <file> --json` and
   `python3 bootstrap/oracle.py test <file>`), the same lanes
   `scripts/test.sh` drives.
2. Probe parity: every entry's measurement probe
   (`bootstrap/validation_probes/*.op`, programs that assert NOTHING and
   only print `RESULT name=value` observables) runs on both cores; the
   RESULT lines must be byte-identical, the differential discipline applied
   to the measurement instrument itself.
3. Comparison: each registered quantity's observed value is checked against
   the registry expectation with the registry's kind and magnitude; the
   report records entry id, expected, observed, tolerance, pass/fail, and
   exits 1 on any red row.

## Reproducibility notes

**Environment pinning.** A validation row is only as reproducible as the
toolchain that produced it:

- Rust: the stable toolchain, pinned by SHA in `.github/workflows/ci.yml`
  (dtolnay/rust-toolchain); this framework was pinned and verified at
  cargo/rustc 1.98.1. No fast-math, no FP contraction anywhere in the build
  (DETERMINISM S5/S6), which is exactly what makes the 1e-12 rows
  cross-platform claims instead of machine-local ones.
- Python: the oracle, the differential harness and the validation report
  use only the standard library; this framework was verified on CPython
  3.12.14 (CI uses the runner default `python3`).
- The C++ codon kernel is integer-only bit-parallel arithmetic (no FP at
  all, DETERMINISM S5) and is not on any validation row path; it is built
  by `scripts/build.sh` and smoke-tested by `scripts/test.sh` like every
  other gate.
- The report JSON records what it ran under (engine version, rust
  toolchain, python version, platform), so an archived report is
  self-identifying.

**UTC-only time contract.** Wall-clock values are NEVER pinned in any
corpus output (DETERMINISM S2): the date/time builtins compute the UTC
civil calendar (SPEC S22) and `std/time.op` is a UTC-only module (offset
suffixes rejected). The validation suite follows the same rule: probes and
enforcing tests print only deterministic quantities, and the report's
`generated_utc` field is informational metadata that nothing asserts. A
report archived in CI is comparable across machines and years precisely
because no local timezone, locale or wall-clock value can enter any
compared byte.

**CI invocation** (documentation only, no workflow is edited by this
change). After the existing build step, a validation lane is one command:

```yaml
- name: validation registry report (W093)
  run: python3 bootstrap/validation_report.py --out validation_report.json
```

The script needs `bin/operon` (produced by `scripts/build.sh`), prints the
JSON report to the file and the human summary to stderr, and exits 1 on any
red row, test failure or probe parity divergence, so it slots into an
existing job as a hard gate (upload `validation_report.json` as a workflow
artifact to archive it). `python3 bootstrap/validation_report.py
--check-doc` is the companion drift gate for this file. Full replay of any
pinned value follows the DETERMINISM S7 protocol (checkout the commit,
`cargo build --release`, run the harness).

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
