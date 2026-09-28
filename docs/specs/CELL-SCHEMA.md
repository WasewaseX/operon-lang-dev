# `.cell` formal schema (W066)

`.cell` is RUNTIME/ENVIRONMENT configuration, never a package manifest (that is
`operon.toml`, W19/W22). Loaded explicitly via `--cell`; an auto-detected cell cannot widen a
sandbox (README security section, SPEC §15). The engine consumes a flat `key = value` map
parsed by `src/genes.rs::parse_cell` (INI-like: `#` comments, `[section]` headers compose
`section.key`, values are taken verbatim, surrounding double quotes are stripped, last
assignment wins).

## One story: parse lane and check lane agree (W10/W11)

Two validators look at every `.cell` file and they use the SAME vocabulary:

- **Check time** — `operon lint f.op --cell c.cell` (src/lint.rs): unknown key →
  `cell-unknown-key` (W10) warning; a numeric key with a non-numeric value →
  `cell-type-mismatch` (W11) warning.
- **Parse time** — `src/genes.rs::parse_cell_checked`: the same two rule names, one advisory
  note per finding, with richer detail (the unknown-key note carries a closest-known-key
  typo hint, the type-mismatch note names expected vs got). Notes are returned to the
  caller (loader/lint); `parse_cell` keeps the frozen silent behavior every existing caller
  and the differential parity depend on.

Both are advisory (Total Grammar): a configuration problem NEVER rejects a run. Unknown keys
are delivered to the map and ignored by the engine — which is exactly why the note exists:
a typo'd threshold must not look like a configured one. `allow.*` values are additionally
redacted from `methyl()` reads (security contract, SPEC §9b).

## The schema (source of truth: `src/genes.rs::CELL_SCHEMA`)

38 declarations: 35 exact keys + 3 prefix families (`allow.*`, `ligand.*`, `variant.*`).
Mechanical sync between this table and `CELL_SCHEMA` is pinned by `tests/cell_schema.rs`
(`schema_doc_lists_every_declared_key`); the two must move in the same PR.

| key | type | default when absent | effect | since |
|---|---|---|---|---|
| `allow.exit` | bool | `false` | boolean exit capability; only an explicit `--cell` may grant (SPEC §9b) | sec-r2 (audit C-11) |
| `allow.*` | str | nothing granted | capability grant family: `read/write/net/run/py/env` take comma-separated values, `exit` is boolean; auto-detected `operon.cell` grants are ignored with an `[info]` note (SPEC §9b, §15) | pre-M100 (sec-r2) |
| `cli.variant` | str | — | pins splice-variant selection for every gene (CLI `--variant` is staged in as this key; selection: cell > @m6a > first declared) (SPEC §15) | pre-M100 |
| `entry` | str | `main` / ires | entry gene override (CLI `--entry` wins over it) (SPEC §15) | pre-M100 |
| `enhance.delta` | number (0..=1) | `0.25` | enhancer dose: threshold reduction applied by `enhance` (SPEC §11) | reg-bio (F-6) |
| `expression.koff` | number (0..=1) | `0.1` | telegraph promoter OFF probability per call attempt (SPEC §11) | reg-bio (F-1) |
| `expression.kon` | number (0..=1) | `0.3` | telegraph promoter ON probability per call attempt (SPEC §11) | reg-bio (F-1) |
| `expression.seed` | integer | `0` (golden-ratio constant) | reseeds the shared mirrored xorshift64\* stream behind promoter draws (SPEC §11) | reg-bio (F-1) |
| `expression.stochastic` | bool | `false` | enables per-call telegraph promoter draws; the deterministic contract holds otherwise (SPEC §11) | reg-bio (F-1) |
| `grn.decay` | number (0..=1) | `0.0` (no decay) | GRN level dilution per `grn_fire` pulse / time tick (SPEC §11) | A10 / reg-bio-2 (C2) |
| `grn.decay_calls` | integer (>=1) | — (event-driven only) | fires one GRN decay step every N calls when set (SPEC §11) | reg-bio-2 (C2) |
| `ligand.*` | number (0..=1) | `0.0` | `[ligand.<name>]` bath default per species; the runtime `ligand_set` pool wins over it (SPEC §11) | reg-bio-2 (A4) |
| `m6a.decay` | number (0..=1) | `0.0` (no decay) | standalone m6A density decay fraction per cadence tick (SPEC §11) | loop-9 (P0-4) |
| `m6a.decay_calls` | integer (>=1) | `1` | standalone m6A decay cadence in calls (SPEC §11) | loop-9 (P0-4) |
| `m6a.reader.decay` | number (0..=1) | `0.25` | YTHDF2 fate: extra decay on marked transcripts (SPEC §11) | loop-9 (F-6) |
| `m6a.reader.min_level` | integer (0..=3) | `2` | reader engagement threshold on the mark-density lattice (SPEC §11) | loop-9 (F-6) |
| `m6a.reader.translation` | number (0..=1) | `0.10` | YTHDF1/3 fate: translation attenuation on marked transcripts (SPEC §11) | loop-9 (F-6) |
| `methyl.maintenance` | number (0..=1) | `0.5` | maintenance factor applied to methylation levels per `passage(n)` (SPEC §11) | pre-M100 |
| `methylate.quiet` | bool | `false` | suppresses the per-call methylation growth notes (SPEC §11) | pre-M100 (A12) |
| `methylate.threshold` | integer (>=0) | `3` | graded silencing gate: calls blocked when the methylation counter >= threshold (SPEC §11) | pre-M100 (T2b) |
| `modules.visibility` | str | default visibility | `"strict"` enables W24 strict module export visibility (private containment) | W24 |
| `operon.polarity` | number (0..=1) | `0.5` | transcriptional polarity survival factor for upstream cistrons (SPEC §11) | loop-9 (P0-1) |
| `py.timeout_ms` | integer | `10000` (clamp 1..=300000) | `py()` wall-clock cap in milliseconds (SPEC §9b) | substrate-r1 |
| `quorum.dilution` | number (0..=1) | `0.5` | signal-medium dilution per `passage(n)` division (SPEC §11) | loop-9 (C8) |
| `rho.catch` | number (0..=1) | `0.5` | Rho catch-up probability base (distance decay `q = 1-(1-catch)^d`) (SPEC §11) | loop-10 (F-7) |
| `rho.queue_floor` | number | `0.5` | rut-site occlusion floor for Rho termination (SPEC §11) | loop-10 (F-7) |
| `rho.termination` | bool | `false` | arms Rho-dependent termination; opt-in, legacy runs draw nothing (SPEC §11) | loop-10 (F-7) |
| `ribosome.drain` | number | `0.5` | ribosome-queue drain rate per call (SPEC §11) | loop-10 (F-8) |
| `ribosome.queue_cap` | number | `1.0` | per-cistron ribosome-queue shield cap (`0.0` = unshielded) (SPEC §11) | loop-10 (F-7) |
| `repressi.alpha` | number (>0) | `10.0` | repressilator production alpha (SPEC §11) | reg-bio (F-5) |
| `repressi.basal` | number (>=0) | `0.0` | basal promoter leak (SPEC §11) | reg-bio (F-5) |
| `repressi.gamma` | number (>=0) | `1.0` | repressilator degradation gamma (SPEC §11) | reg-bio (F-5) |
| `repressi.hill` | integer (1..=8) | `4` | Hill coefficient (SPEC §11) | reg-bio (F-5) |
| `repressi.noise` | number (0..=1) | `0.0` (off) | Euler substep kick amplitude (SPEC §11) | reg-bio (F-5) |
| `repressi.seed` | integer | `0` (golden-ratio constant) | seeds the repressilator noise stream (SPEC §11) | reg-bio (F-5) |
| `run.timeout_ms` | integer | `10000` (clamp 1..=300000) | `run()` child wall-clock cap in ms; a timed-out child is killed and reported (SPEC §9b) | sec-r2 (audit A14) |
| `scope.cancel_on_error` | bool | `true` | cancel child scopes on a stress unwind (`off` disables) | pre-M100 (scope/cancel lane) |
| `variant.*` | str | (@m6a > first declared) | `variant.<root>` pins the splice variant per gene root (SPEC §15) | pre-M100 |
| `wobble.strict` | bool | `false` | cell-side `--strict`: `operon run` exits 3 when rung >= 3 repairs occurred | W37 |

## Special surfaces (not schema rows)

- **`methyl(key, default?)` reads ANY key** (the introspection door, interp.rs): a program
  can read its effective cell, with two guardrails — `allow.*` reads are REDACTED (the
  program gets its `default` argument, capability grants are the operator's business), and
  values come back typed (`true`/`false` → bool, integer → int, numeric → float, else str).
- **`[section]` composition**: `[rho]` + `termination = true` is the key `rho.termination`;
  a mangled header line (`6a]` instead of `[m6a]`) silently demotes the following keys to
  top level — the unknown-key note is what makes that class of accident visible.

## Rules

1. **Unknown keys are ignored silently by the engine** and noted by BOTH validators
   (`cell-unknown-key`, W10) with a closest-key typo hint at parse time. Never a rejection.
2. **Type mismatches are advisory** (`cell-type-mismatch`, W11): expected kind vs got, from
   the `type` column above. The engine's own loader keeps its per-key range notes
   ("needs a number in 0..=1", …); W11 catches the kind, the loader note catches the range.
3. **`allow.*` values are redacted from `methyl()` reads** and cannot come from an
   auto-detected cell (security contract, SPEC §9b).
4. **Mirrors.** `src/genes.rs::CELL_SCHEMA` is the runtime source of truth; this table is
   its human mirror (sync pinned by `tests/cell_schema.rs`); `src/lint.rs::CELL_KEYS` is
   the check-time mirror. Known drift: lint declares the 23 exact keys + `allow.*` that
   existed when W10/W11 landed; `expression.stochastic`, `ligand.*`, `m6a.decay`,
   `m6a.decay_calls`, `methyl.maintenance`, `methylate.quiet`, `modules.visibility`,
   `operon.polarity`, `py.timeout_ms`, `scope.cancel_on_error`, `variant.*`, `wobble.strict`
   are schema-declared here but not yet lint-checked — lint lane catch-up, one PR.
5. **Future evolution**: a `schema = 1` key reserves a version stamp so a 2.x loader can
   reject (or migrate) newer files loudly instead of guessing. Not declared in
   `CELL_SCHEMA` today (the engine does not read it yet), so setting it would draw an
   unknown-key note — the stamp lands in the schema the same PR the loader learns it.
6. **Runtime surfacing**: parse-time notes are an ADDITIVE channel (they do not touch the
   frozen `parse_cell` map, stdout parity, or the oracle); the loader (`src/tools.rs`)
   decides where they appear. The Rust-lane pins live in `tests/cell_schema.rs`, the
   both-core behavior pin (known keys act, unknown keys read as absent, `allow.*`
   redacted) in `tests/granted/cell_schema.op` + `.cell`.
