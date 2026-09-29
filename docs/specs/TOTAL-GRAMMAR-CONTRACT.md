# Total Grammar, semantic contract (W37)

Operon's defining promise is **nothing you write is ever rejected**. This document makes that
promise precise, so "accepted" never silently means "silently wrong". Five levels, in order of
severity. Levels 1–4 always run; level 5 is the only place execution refuses to start.

| level | name | what happens | user-visible | `--strict` |
|---|---|---|---|---|
| 1 | **Canonical** | exact keyword/grammar match | nothing | runs |
| 2 | **Repaired** | synonym/wobble table maps the token (`les→let`, `fn→gene`) | `[synonym]`/`[wobble]` note with file:line | exit 3 |
| 3 | **Recovered** | structure reconstructed (unclosed block, stray token reinterpreted) | `[fallback]` note with file:line | exit 3 |
| 4 | **Semantic warning** | parses + runs, but static analysis flags probable intent mismatch | `check`/`lint` finding (e.g. `wrong-arity`, `constant-condition`, unknown-identifier-as-string) | exit 3 on `error` severity |
| 5 | **Hard error** | CLI misuse (missing file, unknown flag), capability denial, containment kill | stderr message, exit 2/3 | unchanged |

## The rules

1. **A repair is always narrated.** Every level 2–3 rewrite emits a structured note (line,
   rung, message). `operon explain f.op` is the play-by-play; `operon check --format diag`
   groups them under `repair:`.
2. **Typos must not become data.** The historical behavior "unknown identifier becomes a
   string" is demoted from silent coercion to a **level-4 semantic warning** (`phantom-word`
   rule, planned): the program still runs (level 1–4 contract), but `check` flags it and
   `--strict` fails it. Landing the warning requires the oracle-mirrored differential pass,
   tracked in ROADMAP-100 W37 before any runtime text changes.
3. **Recovery is deterministic.** Two runs on the same source produce the same notes, byte for
   byte. Repairs never depend on environment or platform.
4. **Containment is not grammar.** Fuel exhaustion, memory ceilings and red-team containment
   are runtime policy (SPEC §9b), not grammar levels; they interact with `rescue`, not with
   the parser.
5. **The ladder is the compatibility boundary.** New syntax may only ADD rows to the level-1
   set (see BIO-LAYER-POLICY); synonyms may only be added to the level-2 table with a
   deprecation entry (COMPATIBILITY.md), never silently.

## Tooling map

| tool | what it shows you |
|---|---|
| `operon explain f.op` | every repair/recovery, rung names, strict verdict (W38) |
| `operon check --format diag` | level 4 findings grouped error/warning/repair/style (W41) |
| `operon lint f.op` | level-4 rule engine front door (W42/W43/W48) |
| `operon ast f.op` | the post-repair AST, what actually executed (W39) |
| `operon-ls` | notes with rung tags as editor diagnostics (W46 continues) |

## The sweep, as-built (W037, batch 3)

`scripts/tg_sweep.py [--bin PATH] [--timeout SECS] [--json OUT]` runs every parseable
program under tests/, examples/, std/, apps/ (redteam payloads excluded, they are
adversarial by design) and classifies each from REAL behavior only: clean (exit 0),
contained (exit 1-3, the documented failure fates), panic (signal, 101, or panic text in
the stream), hang (per-file wall-clock timeout). A parseable program must never panic or
hang; that is the line the sweep proves.

First full run (2026-09-28, batch 3): 1,071 programs, 1,052 clean, 19 contained, 0 panic,
0 hang. The corpus strip (10 hand-written nasty-but-parseable programs under
scripts/tg_sweep_corpus/) is clean-or-contained line by line. Sweep findings are bugs to
fix, not numbers to brag about; a panic row is a contract violation and blocks the verdict.

Operational note: the sweep's per-file timeout competes with concurrent load on the same
box (release builds, harness runs). task_groups.op, which spawns real worker threads, was
observed timing out at the 20 s default under load and passing cleanly at 45 s and
standalone. Run the sweep on a quiet box or with --timeout 45 before believing a hang row.
