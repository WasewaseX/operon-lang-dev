# Compatibility & deprecation policy (W63 + W64)

Normative for every change to the language surface. Conflicts resolve toward
SPEC.md, then this file. The policy generalizes D-002 (v2.2 back-compat:
breaking SPEC text is allowed, breaking old programs is not) to the whole 2.x
line, and it implements the D-009 labeling law (version strings move only with
the milestone).

## Change classes

Total Grammar (SPEC §4) means the parser never rejects a token stream, so a
"breaking change" in Operon terms is almost never grammar-level breakage: an
old spelling keeps parsing (rung 2/3 repair) and an unknown spelling degrades
to a note (rung 4). The real risk surface is semantics and observable output.
Three classes, in decreasing order of frequency:

**Additive change**: new grammar forms, value kinds, or builtins that no
pre-existing program can contain, so old programs are untouched by
construction. Post-2.2 examples: match-v2 pattern forms (W02 stage 1), soft
type annotations (W01 stage 1), traits (W04), the bytes type (W29), the `::`
separator sugar (W25). Additive is the default release currency inside a
minor.

**Semantic change**: redefines the meaning of a spelling that already parses.
This is the dangerous class, and D-002 is the gate it must pass. The house
example is the const binding (W05): `const` stopped being a `let` synonym and
became immutable binding + deep freeze. The change was allowed because it was
proven safe: programs whose `const` bindings are never reassigned and whose
containers are never mutated run byte-identically to the old synonym (the
freeze draws nothing from the entropy stream, corpus-verified), and the
`operon fix` const-to-let migration was retired because rewriting `const` to
`let` would unfreeze bindings and change meaning (fix_corpus law 1). A
semantic change without a proof that unaffected programs stay byte-identical
is not a compatible change, it is a 3.0 candidate (contract 3 below).

**Repair-only change**: a rung 2-4 rewrite, or a migrator output, that leaves
canonical meaning identical: canonical(fix(x)) == canonical(x) (tests/
fix_corpus.rs law 1, corpus-wide). Repair policy may change notes freely
(notes are not output); it may never change program output. Two worked
examples:

- The use-path fix rule: `operon fix` at one point migrated `/` separators in
  `use` paths to `::`. That churned the canonical form of
  tests/differential/namespaces.op for nothing: `/` is the canonical use-path
  spelling and `::` is exact sugar for it (W25), so the migration was retired
  and a pin keeps the fixer from ever touching use-path separators.
- The W01 stage 1 unknown-annotation typo armor: an unknown annotation name
  (`gene f(x: inr)`) parses and matches nothing instead of failing. The armor
  is deliberately permissive so a later stage (user-defined phenotype
  annotations) can give those names meaning additively, without a breaking
  cliff under programs that already contain misspelled annotations.

"Observable output" means everything a pinned program must reproduce: stdout
per mode, exit codes, frames/telemetry, draw counts. The bar for "unchanged"
is byte-identical.

## The 2.x contract

1. **2.x accepts all 2.x programs.** Total Grammar makes this load-bearing: a program that ran
   on 2.0 runs on every later 2.x (modulo runtime containment policy changes, which are
   documented per release). This generalizes D-002 (v2.2 back-compat) to the whole minor line.
2. **Deterministic output is part of compatibility.** Same source + same seed + same language
   version ⇒ byte-identical outputs (draw-count invariance; loop-9 discipline). Engine
   refactors that change the RNG stream require a language-version bump.
3. **Breaking changes require 3.0** and a DECISIONS entry listing every breakage with a
   migration path (mechanized where possible, `operon fix`, W65).
4. **Version strings move only with milestones** (D-009): Cargo.toml carries the last tagged
   release; SPEC Status carries the in-development label (`vX.Y.0-dev`). Enforced mechanically
   by `scripts/check_docs_sync.py` (W53/W54).

## Deprecation ladder

A deprecation climbs three rungs, each calendar-gated (pinned to a named
version in the table below, never "when someone gets to it"):

1. **Info note**: the feature still works and its output is byte-identical;
   the affected path says so once: a stderr note on the human/editor path plus
   a structured flag in the tool's `--json` output. At least one deliberately
   note-free run path stays alive so the differential harness keeps stderr
   parity.
2. **Warning**: the note escalates to a `check`/`lint` finding that names the
   replacement; `--strict` escalates it to a hard exit.
3. **Removal**: no earlier than one minor release after the warning opens; the
   closing version is named in this file when the warning lands. Removal maps
   the old spelling onto the repair table (old → new) or makes it a hard
   error. Total Grammar keeps nothing silent.

Minimum window: one minor between warning-open and removal. A feature may be
held longer; a window may be extended in this file, never shortened below the
minimum.

**Working precedent (W067 stage 3, `.rna` v1 patches)**: header-less v1
patches entered the ladder at Info: a stderr note on the editor path,
`engine:"v1"` plus `deprecated:true` in the `--json` report, file output
byte-identical (pinned by test), and `operon run --rna` deliberately note-free
for differential stderr parity (tests/rna_v2.rs, 18 tests at landing). Its
Warning → removal steps are calendar-gated by this policy: the closing version
is named here when the warning ships.

For language features, the rungs are enforced through the W064 lifecycle
below. The mark infrastructure that automates rung 2 is W064; until it lands,
rung 1/2 surfaces are hand-plumbed exactly like the precedent above.

### W064 lifecycle for language features

| stage | meaning | enforcement |
|---|---|---|
| 1. `deprecated` | still works; warning emitted | `check`/`lint` finding naming the replacement |
| 2. window | ≥2 minor releases in stage 1 (never less than the ladder minimum) | warning text carries the closing version |
| 3. strict-fail | works only without `--strict` | `--strict` exits 3 on use |
| 4. removed | repair table entry maps old → new spelling OR hard error | Total Grammar keeps nothing silent |

### First entries

| feature | status | replacement | since | window closes |
|---|---|---|---|---|
| `const` as a synonym of `let` | resolved, superseded by the W05 landing | none: `const` is live semantics (immutable binding + deep freeze), the planned const-to-let migration was retired (fix_corpus law 1 pin) | v2.3.0-dev | n/a, no removal window |
| `s::` static-access teaching syntax | documented as unsupported (dx-r3) | dot access `s.field` | v2.2 | v2.4 |
| header-less `.rna` v1 patches (W67 stage 3) | rung 1 (Info) | `syntax: v2` header | current cycle | named here at the Warning step |

New deprecations append here + a `src/lint.rs` registry row + a `check` warning in the same PR.

## How amendments stage (what "landed" means)

- SPEC marks features as stage N in their sections (for example §7c, "Staged
  model (this stage = stage 1)"; §5a match-v2). The board's REMAIN lists own
  what is not landed yet. A stage number is a scope promise, not a version
  claim.
- An amendment counts as **landed only when oracle parity and differential
  pinning exist in the same PR**. The Iron Rules (CONTRIBUTING §1) make this
  mechanical rather than aspirational: rule 6, every semantic change to the
  interpreter is mirrored in `bootstrap/oracle.py` in the same PR and the
  differential harness ends with 0 diverge; rule 7, SPEC travels with code in
  the same PR; rule 1, the full invariant sweep (differential ALL MATCH,
  proofs green, redteam 0 breaches, cargo test green, clippy 0, fmt clean,
  LSP smoke green) blocks the merge otherwise. D-003 says the same from the
  honesty side: new mechanisms land only with SPEC + test in the same commit.
- Between milestones, version strings do not move (D-009, contract 4): the
  last tag stays in Cargo.toml while SPEC Status carries `vX.Y.0-dev`, and
  `scripts/check_docs_sync.py` fails CI on drift.

## Post-2.2 amendments, classified

The policy's first worked example, per the board's done-when: every Track L
addition since the 2.2 tag, classified.

| amendment | class | stage | pinned by |
|---|---|---|---|
| match v2 patterns (W02) | additive | stage 1 (unreachable-arm detection in `check` is the remaining stage) | tests/match_v2.op, tests/differential/match_v2.op, tests/redteam/rt_p17a.op |
| soft type annotations (W01) | additive, with the unknown-name typo armor | stage 1 of L2c (check-time inference, `List<T>`/`Map<K,V>` sugar, aliases remain) | tests/type_anns.op, tests/differential/type_anns.op |
| traits (W04) | additive | landed (both cores) | tests/traits.op, tests/differential/traits.op |
| const bindings (W05) | semantic (D-002-gated, see Change classes) | landed (both cores, plus the rt_p18a hardening) | tests/const_freeze.op, tests/differential/const_freeze.op, tests/redteam/rt_p18a_frozen.op, tests/fix_corpus.rs pins |
| `::` namespace sugar (W25) | additive (exact sugar; both spellings pinned byte-identically) | stage 1 (nested sub-module declarations remain) | tests/differential/namespaces.op |
| bytes type (W29) | additive (new value kind + `b"..."` literals) | landed | tests/bytes.op, tests/differential/bytes.op, tests/redteam/rt_p19a_bytes.op |

## Limitations (what this policy does not promise yet)

- **No per-release compat changelog exists.** CHANGELOG.md does not exist in
  the repo, so the board's "each release ships a compat changelog section"
  criterion is unmet; until the release lane creates it, this file plus git
  tags are the record.
- **The ladder is enforced by hand.** W064 mark infrastructure
  (`@deprecated` with migration text, `--strict` failure) is not landed. The
  only live ladder entries are the W67 info note and `check` findings;
  calendar gates are promises in this file, not machine-checked.
- **Tooling JSON shape stability is ad hoc.** The differential harness pins
  program output, not tool output shapes; shape is preserved case by case
  (the `const_to_let` fixer report field stays, always 0, for `--json` shape
  stability).
- **Security tightening can change observable behavior.** The 2.x contract
  excepts documented runtime containment policy changes; a new redteam
  containment can break a program that relied on the hole. Such changes are
  documented per release, never silent.
- **std/*.op API stability is out of scope** except where the differential
  corpus pins it; STDLIB.md rows are the std contract.
- **v3.0 is unwritten.** The Ribosome VM (W09) is a new implementation
  substrate; its migration and back-compat promises get written when the
  bytecode format freezes, not before.
- **Ratification pending.** DECISIONS.md does not yet carry a D-number for
  this policy (noted in docs/specs/README.md).
