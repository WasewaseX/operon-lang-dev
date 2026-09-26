# Compatibility & deprecation policy (W63 + W64)

## Compatibility contract

1. **2.x accepts all 2.x programs.** Total Grammar makes this load-bearing: a program that ran
   on 2.0 runs on every later 2.x (modulo runtime containment policy changes, which are
   documented per release). This generalizes D-002 (v2.2 back-compat) to the whole minor line.
2. **Deterministic output is part of compatibility.** Same source + same seed + same language
   version ⇒ byte-identical outputs (draw-count invariance; loop-9 discipline). Engine
   refactors that change the RNG stream require a language-version bump.
3. **Breaking changes require 3.0** and a DECISIONS entry listing every breakage with a
   migration path (mechanized where possible — `operon fix`, W65).
4. **Version strings move only with milestones** (D-009): Cargo.toml carries the last tagged
   release; SPEC Status carries the in-development label (`vX.Y.0-dev`). Enforced mechanically
   by `scripts/check_docs_sync.py` (W53/W54).

## Deprecation lifecycle (W64)

A feature (keyword, synonym, builtin, config key) is retired in four stages:

| stage | meaning | enforcement |
|---|---|---|
| 1. `deprecated` | still works; warning emitted | `check`/`lint` finding naming the replacement |
| 2. window | ≥2 minor releases in stage 1 | warning text carries the closing version |
| 3. strict-fail | works only without `--strict` | `--strict` exits 3 on use |
| 4. removed | repair table entry maps old → new spelling OR hard error | Total Grammar keeps nothing silent |

### First entries

| feature | status | replacement | since | window closes |
|---|---|---|---|---|
| `const` as a synonym of `let` | stage 1 (planned with W05 immutability) | `let` (then `const` means immutable binding) | v2.3.0-dev | v2.5.0 |
| `s::` static-access teaching syntax | documented as unsupported (dx-r3) | dot access `s.field` | v2.2 | v2.4 |

New deprecations append here + a `src/lint.rs` registry row + a `check` warning in the same PR.
