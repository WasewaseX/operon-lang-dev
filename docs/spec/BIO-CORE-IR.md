# BIO-CORE IR, the shared biological computational substrate (R0.1)

Status: DESIGN + MINIMAL PROTOTYPE (R0.1, ROADMAP-BIO-COMPUTATIONAL §"R0.1").
The prototype is executable today: `std/biocore.op` (class C4, pure .op, zero
engine delta) with its proof (`tests/biocore.op`, 22 asserts) and the
byte-identical differential pin (`tests/differential/biocore_pin.op`).
Author: builder-B (dev-2). Vocabulary: Lowering.md §1 (the nine abstraction
classes), MODELING-NOTES §2 (the MN keys), per the R0.9 composition
direction in the coordinator's 2026-10-08 posts.

## 0. What this is, and is not

The roadmap asks for "the shared biological computational substrate" as a
DESIGN with a minimal prototype — "without implementing the whole biology
program" and with "no production commitment to every future row". This
document is that design. The substrate is deliberately SMALL: one state
shape, one scheduler, five laws, five invariants. It is the vocabulary new
biological mechanisms lower onto (class C4/C5 landings), and the lens
existing mechanisms are described through (§4's mapping table) — it does not
replace any engine-native mechanism, and no engine source changes here.

## 1. The IR data model

A **core** is a value (a plain Operon map — serializable, .cell-compatible,
engine-neutral):

```
{
  "species":      { NAME: number, ... },          # named levels
  "reactions":    [ REACTION, ... ],              # declaration order = application order
  "interactions": [ EDGE, ... ],
  "events":       [ EVENT, ... ],                 # append-only (INV5)
  "t":            int                             # ticks applied
}
```

- **Species** — a named numeric level. Every persisted bio quantity in the
  engine unifies in this vocabulary: a GRN level (`grn/<gene>`, MN-regulate),
  a methylation lattice rung (`methyl/<gene>`, MN-@methylate), a quorum pool,
  a fate clock. Levels are numbers; the lattice's 0..=3 bound is a GUARD
  convention of its mechanism, not a substrate rule.
- **Reaction** — `{name, law, species, reads, writes, guard, rate, weight}`.
  The `law` names one member of the closed vocabulary (§2). `reads`/`writes`
  are DECLARED species sets: the guard/law may read only `reads` (plus the
  interaction's `from`), and write only `writes` (INV4).
- **Interaction (edge)** — `{from, to, sign, weight}`: a signed, weighted
  dependency between species. `sign` is documentation at the IR level (the
  DYNAMICS live in the reactions); edges feed the edge-driven laws.
- **Event** — `{t, reaction, species, before, after}`: one applied effect,
  appended to the log. The log is the PARITY ARTIFACT (INV5): engines prove
  equivalence by trace identity, the same law the differential harness
  applies to stdout.

## 2. The law vocabulary (closed, one lowering each)

Law L1 (Lowering.md) applies inside the substrate too: a mechanism whose
dynamics cannot be stated as one of these laws plus declared data does not
enter the IR — it stays a stdlib module (C4) or research material (C9) until
it can. The vocabulary is SMALL by design; extending it is a contract change
to this file plus the `biocore_laws()` list plus pins, never a per-mechanism
engine edit.

| law             | stated arithmetic (per tick, per candidate)         | class | example mechanism it models |
|---|---|---|---|
| `decay`         | `lvl -= rate * snapshot(lvl)`, floored at 0         | C4 | MN-regulate decay half (dilution/degradation) |
| `produce`       | `lvl += rate`                                       | C4 | constant-flux synthesis, fate-clock advance |
| `activate`      | `to += weight * snapshot(from)` via the edge        | C4 | MN-regulate activation half (latch feed) |
| `inhibit`       | `to = max(0, to - weight * snapshot(from))`         | C4 | repressor edge (the repressilator ring's cut) |
| `stoch_produce` | draw `random()` once per candidate; `lvl += weight` on success | C5 | MN-telegraph promoter firing (seeded, replay-pinned) |

`snapshot(v)` always means the tick-START value (INV2). A candidate is a
reaction whose guard holds on the snapshot; `stoch_produce` draws exactly one
mirrored-stream number per candidate per tick, so consumption order is
declaration order and the replay law holds.

## 3. Semantic invariants

- **INV1 — Determinism.** Same core + same seed state + same tick count =
  identical event trace, byte-for-byte, on every engine. Where the lowered
  form is stochastic, the SEED REPLAY is the pinned surface (L2, the
  telegraph precedent). Pinned by `tests/biocore.op` (replay equality) and
  `tests/differential/biocore_pin.op` (3-lane byte identity).
- **INV2 — Snapshot isolation.** Every guard and every law's read side sees
  the tick-start snapshot. No reaction observes another's mid-tick write.
  Candidacy is therefore order-independent; only EFFECTS are ordered.
- **INV3 — Declared-order effects.** Effects apply to the working copy in
  declaration order, unconditionally — the scheduler has zero reordering
  freedom. A reaction whose guard reads a species another reaction lifts in
  the same tick decides on the snapshot but writes after (the pin's
  `overflow` row demonstrates both halves: it fires on snapshot 4.5 and adds
  10 on top of feed's 9.0).
- **INV4 — Declared writes.** A reaction may write only species it declared.
  Violations are refused EAGERLY at `biocore_add_reaction` (never discovered
  at tick time), with a refusal naming INV4 — the .cell-schema discipline
  (W66) applied to dynamics.
- **INV5 — Append-only, replayable log.** Events only append; inputs are
  never mutated (every tick/add returns a NEW core — the stdlib copy law,
  the rand_shuffle precedent). The trace is the parity artifact and the
  profiling surface (class C6 consumers read it, they do not perturb it).

## 4. The lowering boundary (what exists, what lands here)

The substrate is the SHARED vocabulary; engine-native mechanisms keep their
classes (Lowering.md §1 is the authority). The mapping — every row cites its
class and MN key per the composition direction:

| engine surface | lowers to (IR view) | class | MN key |
|---|---|---|---|
| `regulate` GRN persistence + decay | species `grn/<gene>` + `decay`/`activate` reactions | C1 (frozen keyword) | MN-regulate |
| `@methylate` graded silencing | species `methyl/<gene>` (lattice rung) + threshold guard | C2 mark | MN-@methylate |
| `@acetylate` immunity | guard exemption recorded on the silencing reaction | C2 mark | MN-@acetylate |
| `@m6a` dispatch priority | consult-point record, not a species | C2 mark | MN-@m6a |
| `silence` (RISC capture) | `stoch_produce`-class seeded draw on the mirrored stream | C1 keyword + C5 | MN-silence |
| `telegraph` promoter | the canonical `stoch_produce` law | C5 | MN-telegraph |
| `repressilator` (builtin ring) | the proof's ring SHAPE (produce/activate/inhibit ring) — engine-native law stays | C1 | MN-repressilator (SPEC §11) |
| Rho termination / queue shield | consult-point pins, outside the level substrate | C5 | Rho family rows |
| `std/random` (R0.5) | the mirrored stream every seeded law draws from | C4 | — |
| future mechanisms | land HERE first (C4 module + laws/guards/edges), engine-native only via the C5 ladder | C4/C5 | per-mechanism |

The boundary rule (the substrate's version of L2): **new biological dynamics
land as IR data + the five laws unless a mechanism proves it needs engine
native code** — and that proof runs through the C-ladder (library → .cell →
mark → kernel → never grammar), not around it.

## 5. The prototype (what executes today)

- `std/biocore.op` — the substrate: `biocore_new`, `biocore_add_reaction`,
  `biocore_add_interaction`, `biocore_validate`, `biocore_tick`, `biocore_run`,
  `biocore_species`, `biocore_trace`, `biocore_render`, plus the closed
  `biocore_laws()` list. Pure .op, zero engine delta (class C4), both engines
  byte-identical by construction.
- `tests/biocore.op` — the invariants as executable assertions (22 asserts:
  construction refusals, exact law arithmetic, INV2 snapshot isolation with
  the guard/working-copy split, INV3 declared-order composition, INV4 eager
  refusal, INV5 append-only + input immutability, INV1 seeded replay, and the
  ring oscillation shape: bounded, peak past start).
- `tests/differential/biocore_pin.op` — 4-tick deterministic program + the
  seeded stochastic replay, byte-identical vm/tree-walk/oracle.

## 6. Non-commitments (the honest scope)

This prototype commits to the SHAPE, not the surface area: no claim that
every future mechanism already fits the five laws; no .cell grammar for cores
(the map literal is the format until a need is proven); no engine-native
acceleration (the C7 ladder exists if a profile demands it); no migration of
the existing engine-native mechanisms (they keep their classes and pins).
The next bio-semantics task may extend the law vocabulary or replace the
prototype wholesale — the invariants are the durable part, not the file.
