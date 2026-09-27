# Operon keywords — GENERATED from src/parser.rs, do not hand-edit

The parser's reserved set (source of truth: `KEYWORDS` in `src/parser.rs`).

Count: **59**. Regenerate: `python3 scripts/gen_doc_stats.py`.

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
| `edit` | in-place value edit |
| `elif` | else-if branch |
| `else` | fallback branch |
| `enhance` | feature-flag boost (lowers a gene's gate threshold) |
| `enter` | scope-entry hook |
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
| `replace` | swap an implementation |
| `repressilator` | 3-node oscillator (negative-feedback ring) |
| `rescue` | catch handler |
| `return` | exit with value |
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
| `trait` | *(analogy pending — file a docs finding)* |
| `use` | import module |
| `variant` | named alternative implementation |
| `while` | condition loop |
| `yield` | generator yield |

Literal words `true false null` and logical `and or not` are recognized in
expression positions but are not part of the reserved table (SPEC §3).

NOTE: 1 keyword(s) lack a D-008 analogy: trait.
