# CLAIMS, the machine-readable claim/evidence layer (R0.7)

Every biological model in Operon makes a CLAIM about a mechanism: that it
mirrors some piece of biology, under stated assumptions, with a specific
validation class, backed by named evidence. Until R0.7 that claim lived in
three places at once (BIO-CONTRACT labels, VALIDATION.md prose rows, the
numerical registry) and nothing checked that the three agreed. This file
defines the claim record that unifies them; `bootstrap/claim_registry.json`
is the machine-readable registry of those records;
`scripts/check_claims.py` is the standing gate with teeth.

## The law

1. **A claim without evidence is a wish; evidence without a claim is an
   undocumented constraint.** Every numerical entry in
   `bootstrap/validation_registry.json` MUST be claimed exactly once in
   `bootstrap/claim_registry.json`, and every claim MUST point at evidence
   that exists on disk. The checker enforces both directions.
2. **Magnitudes live in the validation registry and nowhere else.** Claims
   carry PROVENANCE (where a parameter comes from) and ASSUMPTIONS (what the
   claim rests on), never expected values or tolerances — W093's
   single-source-of-truth rule is inherited, not re-implemented. A claim
   that restates a magnitude has created a second source of truth and the
   checker rejects the shape that invites it.
3. **No unstated assumptions.** Every claim lists at least one assumption.
   For deterministic-core claims that is usually the determinism premise
   (`docs/spec/DETERMINISM.md` section 1) plus the W090 op-order parity
   contract; stochastic claims add the seeded-stream premise.
4. **Unregistered mechanisms stay visible.** A biological claim whose
   mechanism has no BIO-CONTRACT row yet must say so explicitly
   (`bio_contract_labels: []` + a `bio_contract_note` that names the gap).
   Silence is not a state; an empty label list without the note fails the
   gate.
5. **Deny-by-default.** Unknown fields, unknown enum values, dangling
   references, and missing files all fail. The checker self-tests its own
   teeth (`--negative-selftest`): if a mutation cannot make it fail, the
   gate is decoration and must be repaired before it ships.

## Claim record (claim-v1)

| field | required | rules |
|---|---|---|
| `id` | yes | unique; biological claims derived from their registry row (`C-<Vid>`), analytic/RNG claims `C-R06-*` / `C-R05-*` |
| `model` | yes | one-line model statement (what is claimed about the mechanism) |
| `model_kind` | yes | `biological` (BIO-CONTRACT governs) or `mathematical-reference` (pure-math substrate: R0.6 analytic distributions, R0.5 RNG) |
| `surface` | yes | Operon surface tokens (keywords/constructs) the claim is about; each must appear in generated `docs/KEYWORDS.md` or `docs/spec/` |
| `bio_contract_labels` | biological | subset of `REAL`, `APPROX`, `ABSTRACTION`, `SIMPLIFICATION` (the four labels `docs/spec/BIO-CONTRACT.md` defines); each word must occur in that file; `[]` only with the pending note (rule 4) |
| `bio_contract_note` | if labels empty | states the gap honestly (e.g. row not yet filed) |
| `primary_sources` | yes | non-empty; literature/spec sources for the model, not for Operon |
| `equations` | yes | non-empty; the model equations as used, in the registry's mapping language |
| `parameter_provenance` | yes | non-empty list of `{param, source, kind}`; `kind` ∈ `spec-default` / `literature` / `registry-row` / `test-canonical`; **no `value` field** (rule 2) |
| `assumptions` | yes | non-empty (rule 3) |
| `validation_class` | yes | `ANALYTIC` (R0.6 closed-form harness) / `ORACLE` (mirrored-expectation registry rows) / `DIFFERENTIAL` (two-lane parity) / `BEHAVIORAL` (containment/pass assertions) / `BENCHMARK` (measured, regression-guarded) |
| `evidence` | yes | object of path/id lists: `validation_registry_ids` (must exist in `bootstrap/validation_registry.json`), `enforcing_tests`, `fixtures` (must be in `tests/validation/MANIFEST.sha256`), `programs`; at least one item in total; every path exists on disk |
| `status` | yes | `VALIDATED` (evidence green on main) or `PROVISIONAL` (claim registered ahead of its evidence; must become VALIDATED before any R1+ row depends on it) |

## Adding a claim

1. Land the model's evidence first (registry row + enforcing test, or an
   R0.6 fixture set, per VALIDATION.md rules).
2. Append the claim record to `bootstrap/claim_registry.json` with full
   provenance and assumptions.
3. Run `python3 scripts/check_claims.py` — green, then `--negative-selftest`
   (teeth intact), then the standard battery.
4. If the mechanism lacks a BIO-CONTRACT row, file the row (or the note)
   in the same PR. Claims are allowed to expose doc gaps; they are not
   allowed to hide them.

## Negative-testing the gate

`python3 scripts/check_claims.py --negative-selftest` mutates an in-memory
copy of the registry (drops an assumption, dangles an evidence path, invents
an unknown class, unclaims a registry entry, empties a label list without a
note) and asserts every mutation FAILS with a named finding. The selftest is
part of the gate: a silent checker is a failed check.
