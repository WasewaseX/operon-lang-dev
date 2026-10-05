# Operon keywords, GENERATED from src/parser.rs, do not hand-edit

The parser's reserved set (source of truth: `KEYWORDS` in `src/parser.rs`).

Count: **60**. Regenerate: `python3 scripts/gen_doc_stats.py`.

Analogy voice per D-008 (zero biology assumed; one-line programmer meaning).

| keyword | programmer analogy |
|---|---|
| `activates` | positive regulation edge |
| `anchor` | explicit export anchor |
| `as` | alias binding |
| `autoinducer` | quorum-sensing counter (population medium) |
| `bind` | attach a binding site |
| `break` | leave loop |
| `case` | match arm |
| `cofactor` | ligand modifier |
| `collect` | comprehension body |
| `continue` | next iteration |
| `decoy` | decoy binding site (absorbs interference) |
| `edit` | in-file edit block (load-time .rna metadata) |
| `elif` | else-if branch |
| `else` | fallback branch |
| `enhance` | feature-flag boost (lowers a gene's gate threshold) |
| `enter` | fate's initial-state entry |
| `export` | export declaration |
| `fate` | state-machine tag on a value |
| `for` | iteration |
| `frame` | proof/measurement window |
| `from` | inheritance (phenotype C from P) |
| `gene` | named function (def) |
| `guard` | early-exit clause |
| `if` | conditional |
| `import` | synonym of use |
| `in` | membership / loop binder |
| `inducer` | activating ligand |
| `inhibits` | negative regulation edge |
| `ires` | secondary entry point |
| `let` | variable binding |
| `ligand` | named signal value (global pool) |
| `loop` | infinite loop (break to exit) |
| `match` | pattern switch |
| `new` | constructor call |
| `operon` | polycistronic unit (batch of genes) |
| `period` | oscillator timing |
| `phenotype` | class (fields + methods) |
| `proof` | named assertion block (test) |
| `raise` | throw a stress |
| `regulate` | feature-flag network gating calls |
| `replace` | edit-block value-swap arm |
| `repressilator` | 3-node oscillator (negative-feedback ring) |
| `rescue` | catch handler |
| `return` | exit with value |
| `scope` | structured-concurrency block (children joined at exit) |
| `self` | method receiver |
| `sequence` | generator function |
| `silence` | disable a gene (soft-off) |
| `splice` | runtime variant swap for a gene |
| `state` | declare state cell |
| `strength` | regulation edge weight |
| `stress` | throwable error value (exception) |
| `tad` | module boundary (TAD-insulated exports) |
| `threshold` | gate cutoff |
| `toggle` | boolean gate switch |
| `trait` | interface (required + default methods) |
| `use` | import module |
| `variant` | named alternative implementation |
| `while` | condition loop |
| `yield` | generator yield |

Literal words `true false null` and logical `and or not` are recognized in
expression positions but are not part of the reserved table (SPEC §3).

<!-- Q1AUDIT: hand-maintained below; the generator preserves this tail verbatim -->

## Keyword × behavior × proving test (the Q1 audit, 2026-10-02)

One row per reserved keyword (parser `KEYWORDS`, 60), per mark (parser
`MARKS`, 7), and per literal/logical word (6). "Actual behavior" states the
CURRENT engine behavior — probed against the binary where the generated
analogy alone was not enough; dead-by-design forms say so. "Proving test"
names the corpus evidence: differential files are byte-compared on all
three engines by the harness, proof frames run their asserts under both
proof runners (walker + oracle), cargo tests cover the compiled side.
Kept green by the W55b docs-sync gate (every word here must keep a row
with a non-empty proving-test cell) and by future S4 sweeps.

Findings register (filed to vault REVIEWS.md):
- Q1-1: `operon run --ires` selects the first registered ires entry on both
  Rust engines (with the 'cap-independent entry via --ires' note) but the
  oracle CLI accepts the flag and ignores it (its resolve_entry implements
  the selection; the run path never passes it) — flag-leg parity gap,
  oracle lane (builder-A W006-B/D are in flight; not touched cross-lane).
- Q1-2 (design note, no action): the `edit NAME { replace FROM -> TO }`
  in-file form is inert metadata on purpose — patches belong to load-time
  .rna (interp.rs Stmt::Edit) — and both engines agree; pinned so any
  future wiring change trips the audit.
- Q1-3 (fixed here): the generated D-008 analogy for `enter` said
  "scope-entry hook"; the actual behavior is a fate block's initial-state
  entry — analogy corrected in the generator's table.

| word | class | actual behavior | proving test |
|---|---|---|---|
| `activates` | keyword | GRN activation edge in a regulate block: `a activates b strength S threshold T`; level flows caller to callee with hop attenuation | tests/autoreg_hysteresis.op, tests/copies_dose.op (proof frames) |
| `anchor` | keyword | export/import anchor: `anchor export a, b;` names the module's export table; any anchor makes the module anchored-only (anchor-export-wins) | tests/mechanisms.op, tests/differential/keyword_tad.op (in-tad form) |
| `as` | keyword | use-path alias binder (`use std/strings as st`); only valid in use paths | tests/caps_policy.op, tests/async_caps.op |
| `autoinducer` | keyword | quorum-sensing counter: population-medium signal feeding quorum gates | tests/quorum_basic.op, tests/differential/quorum_state_pin.op |
| `bind` | keyword | binding-site declaration; ligand occupancy feeds the gate math (the occupy-sum combination rule) | tests/lac_gate.op, tests/differential/occupy_sum_combo_pin.op |
| `break` | keyword | leaves the enclosing loop; a frame-level break in a proof is an integrity failure (runner FAILs it) | tests/compat/control_0000.op; scripts/proof_rules_e2e.sh (negative shape) |
| `case` | keyword | match arm introducer: `match v { case Pat { ... } }` | tests/compat/matchpat_0000.op, tests/bytes.op |
| `cofactor` | keyword | ligand modifier: modulates binding/occupancy effects | tests/trp_attenuator.op, tests/lac_gate.op |
| `collect` | keyword | comprehension body: `collect { expr }` builds the collected list | tests/control_flow.op, tests/sequences.op |
| `continue` | keyword | next loop iteration | tests/compat/control_0000.op |
| `decoy` | keyword | decoy binding site: absorbs interference without signaling | tests/grn_decoy.op |
| `edit` | keyword | in-file `edit NAME { ... }` blocks are load-time METADATA: the interpreter no-ops them by design (patches belong to load-time .rna; the target is a bare name) | tests/differential/keyword_edit_replace.op (inert parity, 3-lane); tests/rna_v2.rs (the .rna machinery, cargo suite) |
| `elif` | keyword | else-if branch | tests/compat/control_0001.op |
| `else` | keyword | fallback branch of if/elif chains | tests/compat/control_0000.op |
| `enhance` | keyword | feature-flag boost: `enhance g;` lowers g's activating thresholds by ENHANCE_DELTA (0.25) | tests/enhance_boost.op (fail-pre proven) |
| `enter` | keyword | fate block's initial-state entry: `enter off` puts the fate machine in state `off` | tests/stress_fate.op |
| `export` | keyword | export declaration feeding the module export table; interacts with strict visibility (pub-marked modules) | tests/namespaces2.op, tests/granted/visibility_strict.op |
| `fate` | keyword | named state machine: `fate X { state a -> b ... enter a }` | tests/stress_fate.op |
| `for` | keyword | iteration over lists, maps (keys), strings | tests/control_flow.op |
| `frame` | keyword | measurement/proof window: `frame NAME { }`; `frame proof { }` is the walker-counted proof form | tests/core_values.op (161 corpus proof frames) |
| `from` | keyword | inheritance introducer: `phenotype C from P` | tests/differential/pheno_equality.op, tests/const_freeze.op |
| `gene` | keyword | named function definition; anonymous lambda form `gene (x) { ... }` | tests/operon_unit.op (corpus-wide) |
| `guard` | keyword | match-arm guard clause (early-exit condition on an arm) | tests/differential/guard_arms.op |
| `if` | keyword | conditional | tests/compat/control_0000.op |
| `import` | keyword | synonym of `use` (module import) | tests/differential/mod_res.op, tests/mechanisms.op |
| `in` | keyword | membership test and the for-loop binder | tests/control_flow.op |
| `inducer` | keyword | activating ligand (occupies binding sites, drives gates) | tests/lac_gate.op, tests/trp_attenuator.op |
| `inhibits` | keyword | GRN inhibition edge (negative regulation) | tests/differential/occupy_sum_combo_pin.op, tests/grn_bistability.op |
| `ires` | keyword | secondary entry registration: `ires NAME;` registers NAME; INERT in default runs (pinned 3-lane); `operon run --ires` runs the first registered entry with a note on both Rust engines — the oracle CLI ignores the flag (Q1-1) | tests/differential/keyword_ires.op (registration inertness) |
| `let` | keyword | variable binding; `=` rebinds | tests/core_values.op (corpus-wide) |
| `ligand` | keyword | named signal value in the global pool; occupancy math reads it | tests/differential/occupy_sum_combo_pin.op, tests/autoreg_hysteresis.op |
| `loop` | keyword | infinite loop; `break` is the only exit | tests/autoreg_hysteresis.op, tests/async/async_channels.op |
| `match` | keyword | pattern switch with `case` arms | tests/compat/matchpat_0000.op, tests/match_v2.op |
| `new` | keyword | constructor call on a phenotype | tests/channels.op, tests/differential/channels.op |
| `operon` | keyword | polycistronic unit: a named top-level batch of genes | tests/operon_unit.op, tests/core_values.op |
| `period` | keyword | oscillator timing window over a repressilator | tests/sci_repressi_period.op |
| `phenotype` | keyword | class declaration: fields + methods (+ `type Name = ann` aliases intercept the synonym) | tests/differential/pheno_equality.op |
| `proof` | keyword | named assertion block: `frame proof { assert(...) }`; asserts are COUNTED — vacuous proofs and frame-level returns FAIL the runner (S4-6b) | tests/core_values.op; scripts/proof_rules_e2e.sh |
| `raise` | keyword | throws a stress (`raise KIND, MSG`) | tests/bytes.op, tests/burst_modulate.op |
| `regulate` | keyword | GRN edge declaration block | tests/autoreg_hysteresis.op, tests/regulation.op |
| `replace` | keyword | edit-block arm `replace FROM -> TO`; INERT in-file by design (edit blocks are load-time metadata, Q1-2) — both engines agree, and the pin trips if that ever changes | tests/differential/keyword_edit_replace.op |
| `repressilator` | keyword | 3-node negative-feedback oscillator declaration | tests/regulation.op, tests/differential/noise_pin.op |
| `rescue` | keyword | catch handler of a stress block (`rescue (e) { ... }`; kind-filtered forms pass wrong kinds through) | tests/bytes.op, tests/stress_fate.op |
| `return` | keyword | exit the gene with a value | tests/operon_unit.op (corpus-wide) |
| `scope` | keyword | structured-concurrency block: children joined at exit | tests/async/async_scope.op |
| `self` | keyword | method receiver inside phenotype methods | tests/differential/pheno_equality.op |
| `sequence` | keyword | generator function; creation honors ALL gates (GRN veto, methylation, toggle) like any transcript | tests/seq_gates.op |
| `silence` | keyword | disable a gene (soft-off: calls gate to null) | tests/granted/rho_readthrough.op (cell-granted leg) |
| `splice` | keyword | runtime variant swap for a gene | tests/mechanisms.op, tests/differential/splice_m6a.op |
| `state` | keyword | state-cell declaration; inside a fate, `state a -> b` declares transitions | tests/stress_fate.op, tests/async/async_basics.op |
| `strength` | keyword | regulation edge weight | tests/autoreg_hysteresis.op, tests/copies_dose.op |
| `stress` | keyword | throwable error value + containment block; contained stresses unfold (never crash) | tests/bytes.op (corpus-wide stress/rescue) |
| `tad` | keyword | module-boundary insulation: members bind into the ENCLOSING environment (not a child scope) and `anchor export` INSIDE a tad feeds the module export table (tad_exports merged with anchor_exports) | tests/differential/keyword_tad.op + keyword_tad_lib.op (new pin, both halves) |
| `threshold` | keyword | gate cutoff on regulation edges/genes | tests/autoreg_hysteresis.op, tests/copies_dose.op |
| `toggle` | keyword | boolean gate switch (`toggle a, b` + toggle_on/toggle_off builtins) | tests/grn_bistability.op, tests/seq_gates.op |
| `trait` | keyword | interface: required + default methods | tests/traits.op, tests/differential/traits.op |
| `use` | keyword | module import: resolves FILE-FIRST (bare names beside the importing file), then descends exported nested tables longest-prefix-first | tests/differential/namespaces2.op |
| `variant` | keyword | named alternative implementation, selected at runtime | tests/differential/splice_m6a.op, tests/mechanisms.op |
| `while` | keyword | condition loop | tests/compat/control_0000.op |
| `yield` | keyword | generator yield inside a sequence | tests/seq_gates.op, tests/operon_unit.op |
| `acetylate` | mark | clears methylation (resets the silencing counter) | tests/methyl_gate.op, tests/methylate_api.op |
| `m6a` | mark | variant-level silencing mark honored in variant selection | tests/differential/splice_m6a.op, tests/m6a_levels.op |
| `burst` | mark | bursty expression: `@burst(kon, koff)` in 0..=1 (defaults 0.3/0.1) | tests/burst_identity.op, tests/burst_modulate.op |
| `copies` | mark | copy-number dose scaling of GRN levels | tests/copies_dose.op |
| `deprecated` | mark | metadata-only deprecation payload `@deprecated("msg", since="2.4")`; the runtime never reads it, the wobble repair never mangles it | tests/deprecate.op |
| `methylate` | mark | graded silencing: per-call counter, at/over threshold blocks the call | tests/methyl_gate.op, tests/differential/grn_methyl_combo.op |
| `riboswitch` | mark | ligand-gated cis switch: needs a ligand name and 'on'/'off' | tests/riboswitch_cis.op |
| `and` | literal | logical and (truth table pinned; `and`/`or`/`not` are expression-position words, not reserved) | tests/core_values.op |
| `false` | literal | boolean literal | tests/core_values.op |
| `not` | literal | logical negation | tests/core_values.op |
| `null` | literal | the unbound/missing value: unbound names, missing map keys, gated calls and unexported reads all collapse to null (the null-law) | tests/core_values.op, tests/differential/builtin_state_introspection.op |
| `or` | literal | logical or | tests/core_values.op |
| `true` | literal | boolean literal | tests/core_values.op |

Audit method: usage inventory over the whole walker corpus (3438 .op files,
161 proof frames) against parser.rs KEYWORDS/MARKS, then per-word behavior
verification against the parser/interp sources and live 3-lane probes for
every word whose corpus coverage was zero or ambiguous (tad, ires,
edit/replace). The three new differential pins exist because those probes
found real behavior no test had ever exercised.
