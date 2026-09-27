# MACROS, metaprogramming design for a Total Grammar language (W35, ROADMAP-100)

Status: DESIGN, no parser keyword is proposed, none may land until this doc
is ratified in DECISIONS (W36 discipline: the keyword surface is frozen;
macros are the pressure valve, so the valve itself must be designed under
pressure).

Author: builder-B (dev2). Reviewer queue: sz (ratification), builder-A
(parser-implementation feasibility).

## 1. The problem, measured

The biology mini-language is hardcoded as seven statement-level parser arms
today (`src/parser.rs`, 3,723 lines): `enhance` (873), `fate` (1003),
`regulate` (1082), `operon` (1465), `repressilator` (1541), `splice` (1703),
`ires` (1890). Each arm is a bespoke grammar fragment that parses special
surface syntax into AST variants. Three costs are already visible:

1. **Parser growth is linear in mechanism count.** BIO-LAYER-POLICY.md froze
   the syntax because every new mechanism used to buy a parser arm. The freeze
   stops the bleeding but does not remove the scar tissue: the seven arms
   still exist as unique code paths to keep byte-parity with the Python
   oracle, to keep SPEC sections honest, and to keep the repair ladder
   correct around them.
2. **Community mechanisms have no path in.** A user who wants a
   domain-specific block (state machines, unit-aware quantities, circuit
   diagrams-as-text) can only request a new keyword, exactly the growth the
   audit called the biggest design risk.
3. **The arms are grammar islands.** `regulate` networks, `operon` units and
   `splice` tables each hand-roll their own list/table syntax, which is why
   trap-class bugs (brace greed, trailing-comma drift) appear per-form rather
   than once.

Macros are the standard answer. For Operon, though, the standard answers
arrive with extra charges that a Total Grammar, differential-parity,
deterministic-output language must pay up front. This document prices the
two candidate models and recommends a staged path.

## 2. Model evaluation

### Model A, declarative rewrite rules

Surface: `macro <name>(<pattern>) → <template>`, applied **post-parse** as
AST→AST rewrites; repairs and wobble handling see only the canonical output.

- **Pros**
  - Auditable: a rewrite is data (a pattern and a template), reviewable like
    any diff; expansion can be dumped by `operon ast --json` (W39) and
    explained by `operon explain` (W38) with zero new tooling.
  - Deterministic by construction: expansion is a pure function of
    (AST, rule table) in a fixed order, the draw-count invariance and
    byte-stability contracts (DETERMINISM.md, W088–W090) are untouched
    because nothing runs, only rewrites.
  - Fits the repair pipeline: the rule engine runs where the synonym/wobble
    ladder already runs, so an unmatched pattern can fall through as ordinary
    code, **Total Grammar is preservable**: a macro match failure becomes a
    note, never a rejection.
  - No new identifier-resolution semantics: hygiene questions shrink to
    "which names in the template bind where", answered by the same scoping
    rules as the expanded form.
- **Cons**
  - A pattern language is still a language: naive glob-style patterns
    rust quickly (repressilator's cross-statement wiring does not fit a
    single-node pattern). Scope must be cut: pattern = one statement-shaped
    form with holes, NOT a full grammar.
  - Cross-statement transforms (collect `@burst` marks scattered in a file
    into one network) need a second, "gather" rule class, the classic
    hygiene trap of declarative systems.

### Model B, procedural quoting (`quote{…}` + `eval`)

Surface: Lisp-flavored; `quote{...}` yields AST as data; user code runs at
compile time to build new AST.

- **Pros**
  - Maximal power; cross-statement transforms are just loops.
  - One mechanism instead of two rule classes.
- **Cons, and for Operon they are structural, not cosmetic**
  - **A second interpreter.** Compile-time user code must execute somewhere.
    Executing it on the Rust core means maintaining compile-time parity with
    the Python oracle for every builtin the quote-language can call, the
    differential corpus doubles in the worst dimension (semantic surface,
    not output). Executing it in Python breaks the "oracle never ships"
    boundary and inverts the dependency arrow.
  - **Determinism exposure.** Compile-time code that reads the clock, env,
    or RNG breaks draw-count invariance silently; guarding every builtin at
    expansion time is a whole capability-sandbox re-implementation.
  - **The sandbox inversion.** Operon's security story (sec-r1/r2) is
    "nothing runs without a grant". Compile-time eval is code that runs
    before any grant context exists, either it runs un-granted (fine but
    then it is a crippled language) or pre-granted (a new trust boundary the
    threat model, THREAT-MODEL.md W100, does not currently model).
  - **Repair semantics freeze-in-time.** `quote{...}` captures raw tokens;
    the wobble/repair ladder currently repairs at parse. Quoted fragments
    would need "repair-on-quote or repair-on-eval" semantics, a NEW rung
    question the 4-rung contract (TOTAL-GRAMMAR-CONTRACT.md) does not answer.

### Verdict

**Model A, staged, with the rule-table twist in §4.** Model B is not
rejected forever; it is rejected until the day a mechanism genuinely cannot
be expressed as a rewrite, and any such proposal must come with a parity and
sandbox plan (the burden of proof sits with the proposer).

## 3. Interaction contracts (what a ratified macro system must promise)

1. **Total Grammar holds.** An unmatched macro pattern is an ordinary
   statement; a pattern that matches but fails to expand produces a NOTE
   (rung 3, wobble semantics) and leaves the source statement intact.
   Nothing in macro land may reject a program.
2. **Determinism holds.** Expansion is pure: no IO, no clock, no env, no RNG,
   no `py()`; rule tables come only from the module graph already governed by
   `use`. Byte-identical inputs give byte-identical outputs, corpus-wide.
3. **Oracle parity holds.** Every rule is mirrored op-for-op in
   `bootstrap/oracle.py` like every builtin; `tests/differential/` pins one
   byte-identical expansion per rule family. A rule without its differential
   pin does not ship.
4. **Tooling sees through it.** `operon ast` (W39) dumps POST-expansion by
   default with a `--pre-macro` switch; `operon explain` (W38) names the rule
   that fired and where it matched; lint (W42) runs post-expansion so rules
   never hide unreachable code from it.
5. **Error quality holds.** Because expansion rewrites the AST before
   lowering, spans must be preserved (the W007 statement-line discipline
   applies to template-emitted nodes: origin line, not template line,
   tracebacks point at the user's code).
6. **Keyword surface stays frozen.** Rule DECLARATION is not a keyword. If
   rules are data (§4), no new reserved word is required at any stage; the
   word `macro` never enters the parser's reserved set on the recommended
   path.

## 4. Recommended shape: rules as data, engine as law

The rule table is an Operon value (a list of maps: `match` template, `expand`
template, `where` guards limited to structural predicates), loaded from a
std module (`std/macros.op`) or a `.op` file's header. The ENGINE is a fixed,
small, Rust-side AST rewriter; it is a law of the language, not a library.

Why rules-as-data beats both classic variants here:

- vs Rust-coded rules: rule tables are revisable in std/ without a compiler
  release, reviewable as data diffs, and loadable per-module under existing
  `use` control (no new trust surface).
- vs `macro` keyword declarations: no new keyword (W36 discipline is
  structurally enforced), and the declaration syntax cannot drift from the
  parse of the templates themselves because templates ARE data.
- The engine stays ~500 lines of Rust with a fixed predicate set, small
  enough to differential-mirror completely.

Hygiene, stated plainly: template identifiers bind in the template's scope;
pattern variables bind at the match site; a template may not introduce a name
that shadows a pattern hole (engine-enforced). This is the minimal rule set
that survived review of the seven hardcoded families in §5.

## 5. First migration candidates (the biology families, in order)

Each migrates as its own PR, differential-pinned before and after (byte-
identical expansion vs the hardcoded arm it replaces):

| # | family | hardcoded arm | why it is first | pattern class needed |
|---|---|---|---|---|
| 1 | `enhance` | parser.rs:873 | single-statement annotation, trivial holes | statement + mark |
| 2 | `fate` | parser.rs:1003 | dispatch table → `match`-shaped expansion | table with holes |
| 3 | `ires` | parser.rs:1890 | polycistronic sugar → multi-gene emit | statement × N |
| 4 | `splice` | parser.rs:1703 | variant table → gene + variant records | table with holes |
| 5 | `operon` units | parser.rs:1465 | polycistronic + polarity, largest table, migrate after 1–4 de-risk the engine | table + marks |
| 6 | `regulate` networks | parser.rs:1082 | network node list, the gather-class rule pilot | statement + gather |
| 7 | `repressilator` ring | parser.rs:1541 | n-node unrolling, LAST (needs gather + arithmetic in guards) | gather + computed |

Stages:

- **Stage 0 (this doc, ratified):** freeze the contract list in §3; approve
  the migration order.
- **Stage 1 (engine, no user surface):** internal `apply_macros(prog, rules)`
  post-parse; rules authored as Rust constants byte-equivalent to the
  hardcoded arms; families 1–2 migrate; differential pins prove parity.
  Parser arms delete on migration, the win is net-negative parser code.
- **Stage 2 (rules become data):** engine reads rule tables from std/;
  families 3–5 migrate; `operon ast --pre-macro` + explain integration land
  here so tooling sees both worlds.
- **Stage 3 (public surface, gated):** user-land rule tables load from
  modules under `use` (capability already exists, no grant change);
  families 6–7 migrate if the gather class proves clean; community macros
  become possible, with the §3 contracts as the gate.

Each stage is independently valuable and independently revertible; none
changes runtime semantics, only who writes down the expansion.

## 6. Explicit non-goals (this document)

- No `quote`/`eval` (Model B), reconsidered only with a parity + sandbox
  plan per §2.
- No compile-time execution of user functions under any stage.
- No new reserved words at any stage of the recommended path.
- No macro-based deprecation shims for the frozen bio syntax: deprecation
  goes through the W64 lifecycle, not through rewrite tricks.

## 7. Open questions for sz

1. Does Stage 2's "rules live in std/" conflict with the std freeze discipline
   (W36's library-ization rule), or is it its natural continuation?
2. Is the gather rule class (families 6–7) worth its hygiene cost, or should
   `regulate`/`repressilator` remain permanent parser arms (the honest
   fallback, §5 order already de-risks without them)?
3. Should `operon check --strict` (W41) flag files whose expansion fired at
   all (a "macro-free purity" lint rule), or is that noise?
