# `.cell` schema (W66)

`.cell` is RUNTIME/ENVIRONMENT configuration, never a package manifest (that is
`operon.toml`, W19/W22). Loaded explicitly via `--cell`; an auto-detected cell cannot widen a
sandbox (README security section). The engine consumes a flat `key = value` map
(`src/genes.rs::parse_cell`).

Validated by `operon lint f.op --cell c.cell`: unknown keys → `cell-unknown-key` warning,
non-numeric value on a numeric key → `cell-type-mismatch` warning. Findings are advisory (Total
Grammar: configuration problems never reject a run, an unknown key is ignored, which is
exactly why the lint warning exists).

## Keys (generated sweep of `cell.get` call sites; source of truth: `src/lint.rs::CELL_KEYS`)

| key | type | what it tunes |
|---|---|---|
| `cli.variant` | str | variant selection pinned for the whole run |
| `entry` | str | entry gene override |
| `enhance.delta` | num | `enhance` threshold reduction amount |
| `expression.kon` / `expression.koff` / `expression.seed` | num | per-gene telegraph bursting params / RNG seed |
| `grn.decay` / `grn.decay_calls` | num | GRN level homeostasis decay (opt-in) |
| `methylate.threshold` | num | calls blocked when methylation counter ≥ threshold |
| `m6a.reader.decay` / `m6a.reader.min_level` / `m6a.reader.translation` | num | m6A reader effects (opt-in) |
| `quorum.dilution` | num | population-medium dilution per `passage(n)` |
| `rho.catch` / `rho.queue_floor` / `rho.termination` | num/bool | Rho termination sensitivity + ribosome-queue shield |
| `ribosome.drain` / `ribosome.queue_cap` | num | ribosome queueing (opt-in) |
| `repressi.alpha` / `repressi.basal` / `repressi.gamma` / `repressi.hill` / `repressi.noise` / `repressi.seed` | num | repressilator kinetics |
| `run.timeout_ms` | num | child-process wall-clock cap for `run()` |
| `allow.*` | str (family) | capability grants (`allow.read/write/net/run/py/env/exit`) |

## Rules

1. Unknown keys are **ignored silently by the engine** (Total Grammar) and **warned by lint**,
   a typo'd threshold must not look like a configured one.
2. `allow.*` values are redacted from `methyl()` reads (security contract preserved).
3. Future evolution: `schema = 1` key reserves a version stamp so a 2.x loader can reject (or
   migrate) newer files loudly instead of guessing.
4. This table is hand-maintained against the generated `CELL_KEYS` constant; the two must move
   in the same PR (checked by review; automated check tracked as W53 follow-up).
