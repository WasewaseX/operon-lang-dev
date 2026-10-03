# Core/bio boundary contract (W036)

Normative for grammar growth. Conflicts resolve toward SPEC.md, then
[BIO-LAYER-POLICY.md](BIO-LAYER-POLICY.md) (the freeze law, W36 + W81), then
this file. Owner directive D-008 (2026-09-24) froze the v3.5 "Epigenome"
roadmap level and locked the audience: CS engineers who know zero biology;
gene vocabulary is an intuition aid, never a prerequisite.

## The rule

**After the v3.5 freeze (D-008), no new biology-flavored syntax enters the
core language: new biological or domain mechanisms land as `std/*.op` modules
and library genes (or as `.cell` configuration on an existing engine
capability), never as new grammar.**

BIO-LAYER-POLICY.md states this as Rule 1 (the reserved set does not grow) and
Rule 2 (libraries first, `.cell` tuning second, builtins last and only with a
DECISIONS entry). This page is the inventory and the enforcement story the
critique asked for (CRITIQUE-LOOP.md, W036: "the boundary is a convention, not
a written contract with a lint rule that fires when a PR crosses it").

## What is core grammar today (frozen)

35 of the 60 reserved keywords. Counts are as of this writing;
[docs/KEYWORDS.md](../KEYWORDS.md) (generated from `src/parser.rs::KEYWORDS`)
is the live truth.

| family | keywords |
|---|---|
| control flow | `if` `elif` `else` `while` `loop` `for` `in` `break` `continue` `return` `match` `case` `guard` |
| bindings | `let` `as` (plus contextual `mut`, contextual `pub`: W05/W24, not reserved) |
| functions and generators | `gene` `self` `sequence` `yield` `collect` `new` |
| abstraction | `phenotype` `from` `trait` |
| errors | `raise` `stress` `rescue` |
| modules | `use` `import` `tad` `anchor` `export` |
| concurrency | `scope` (plus the `spawn` builtin) |
| verification | `proof` `frame` |

Non-keyword core surface: the literal and logical words `true false null and
or not` (recognized in expression positions, SPEC §3), the 4-rung Total Grammar
ladder itself, the builtin call surface, `##` doc comments (W074), and the
module resolution algorithm (§8).

## What is the frozen biology layer (grandfathered)

The remaining 25 keywords, plus marks, plus contextual spellings. All of it
predates the freeze and stays for back-compat (see Exceptions).

| family | keywords | programmer meaning |
|---|---|---|
| regulation / GRN | `regulate` `activates` `inhibits` `strength` `threshold` `enhance` `silence` `decoy` `bind` `toggle` | feature-flag network gating calls, with edge weights and cutoffs |
| signal pools | `ligand` `inducer` `cofactor` `autoinducer` | named values in a global pool that edges read |
| operons and oscillators | `operon` `period` `repressilator` | polycistronic batch; 3-node ring oscillator |
| runtime variant swap | `splice` `variant` `replace` | swap a gene's implementation at runtime |
| fate machines | `fate` `state` `enter` | a plain state machine on a value (SPEC §11) |
| entry and editing | `ires` `edit` | cap-independent run entry; the `.rna` patch verb |

Plus, all under the same freeze:

- **Marks** (declaration-attached): `@acetylate` `@methylate` `@m6a` `@copies`
  `@riboswitch` `@burst`.
- **Contextual spellings** inside regulation statements (recognized
  positionally, not reserved): `translates` `attenuates` `secrete` `quorum`
  `quench`, and the edge modifiers `sum` `any` `occupy` `hill`, per-cistron
  `rbs`.
- **`.cell` configuration keys** (`rho.*`, `quorum.*`,
  `methylate.threshold`, `enhance.delta`, `m6a.reader.*`, `grn.decay_calls`,
  `modules.visibility`, ...): the sanctioned growth surface for mechanism
  tuning, because tuning-not-syntax is the interface (BIO-LAYER-POLICY Rule 2,
  preference 2). The schema lives in [CELL-SCHEMA.md](CELL-SCHEMA.md) (W66).
- **The C++ codon kernel** (`runtime/codon_kernel.cpp`): its own boundary is
  BIO-LAYER-POLICY Rule 5 (W81): no new biological semantics, extensions need
  a benchmark demonstrating 2x over the Rust path plus a DECISIONS entry.

## What is the std library (the growth surface)

22 modules, pure `.op` by decision (no new Rust builtins without a DECISIONS
entry, D-010). New biological or domain mechanisms default here
(BIO-LAYER-POLICY Rule 2, preference 1), with proofs plus STDLIB.md rows.

`args` `bigint` `bio` `collections` `csv` `deque` `fmt` `fs` `heap` `iter`
`json` `math` `motifs` `path` `random` `seq` `serialize` `set` `strings`
`testing` `time` `unicode`

Two of these are the biology mechanism libraries: `std/bio.op` (sequence
utilities layered on the native kernels, codon usage and friends) and
`std/motifs.op` (the canonical GRN network-motif circuits, runnable over the
`regulate` layer). Everything else is programmer-first vocabulary, which is
the D-008 point.

## Enforcement story (honest)

1. **Generated inventory plus stats gate.** `scripts/gen_doc_stats.py`
   regenerates docs/KEYWORDS.md and docs/stats.json from
   `src/parser.rs::KEYWORDS`; `scripts/check_docs_sync.py` recomputes both and
   fails CI on drift, and its forbidden-pattern list blocks hand-typed counts
   (including the keyword count in SPEC). Consequence: a keyword cannot be
   added without the generated count and table changing in the same diff, so
   the PR is review-visible and the reviewer can ask the one question that
   matters: is this keyword biology?
2. **Process rules.** Grammar lives in `src/parser.rs` (dev-1 lane). The Iron
   Rules (CONTRIBUTING §1) require the oracle mirror and SPEC text in the same
   PR (rules 6 and 7), so a new grammar form without them is an incomplete
   change by rule, not by goodwill. A biology-flavored form additionally needs
   the Rule 4 escape hatch (below).
3. **Existing check surface.** `operon check`/`lint` findings (`src/lint.rs`),
   the migrator laws (tests/fix_corpus.rs), the LSP advertisement pin
   (tests/lsp_smoke.py), and the redteam suite police behavior. None of them
   currently fires on a new biology keyword.
4. **Honest gap.** Enforcement is partly procedural today. There is no
   automated lint that diffs the parser's keyword set against the frozen
   inventory in this file and fails a crossing PR; that checker rule is
   exactly what the W036 critique asks for and it is the named follow-up
   (natural home: `check_docs_sync.py`, which already recomputes the keyword
   list). Until it lands, enforcement is the generated-inventory diff plus
   review discipline.

**For new contributors**: if your feature seems to need a new keyword, it is a
boundary-crossing PR by definition. Default to `std/*.op`. If you believe a
library cannot express it, write the Rule 4 statement first; a PR that adds a
keyword without a DECISIONS entry gets bounced in review.

## Grandfathered exceptions

- **The whole existing biology layer above** (25 keywords, 6 marks, the
  contextual statement spellings): grandfathered because it predates the
  freeze; removing it would break every v2.2 program that uses the mechanisms
  (contradicting D-002) and would discard the metaphor the language is named
  for. Several of these are biology-named but semantically generic (`fate`
  `state` `enter` are a state machine; `splice` `variant` `replace` are
  runtime implementation swap; `ires` is entry-point selection): they keep
  their spellings for back-compat, and a NEW equivalent would ship as a
  library, not as a second spelling.
- **The synonym/wobble repair tables** may keep mapping biology-flavored
  misspellings (`@acetylat` → `@acetylate`); adding repair entries is the
  repair-only class of COMPATIBILITY.md. A synonym must never map a new
  spelling to new semantics.
- **The codon kernel**: a native component predating the freeze, walled off by
  the W81 Rule 5 boundary rather than by this page.

## What would ever justify a new keyword (governance)

Only the escape hatch, all four conditions (BIO-LAYER-POLICY Rule 4): a
DECISIONS entry with the owner's sign-off, a programmer-first analogy (D-008),
differential tests plus the oracle mirror in the same PR, and a written
statement of why a std library, a `.cell` key, and a builtin are all
impossible for it. No exceptions in silence. The track record so far: every
post-2.2 amendment (match v2, soft annotations, traits, const bindings, `::`
sugar, bytes) is programmer vocabulary, and zero biology keywords have been
added since the freeze; the hatch has never been used.
