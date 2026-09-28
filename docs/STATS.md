# STATS — generated doc counts (W53)

Generated 2026-09-28 20:49:22Z by `scripts/gen_doc_stats.py`. **Do not hand-edit** —
regenerate with `python3 scripts/gen_doc_stats.py` and commit together with any
change that moves a count. `scripts/check_docs_sync.py` fails when README/SPEC
quote a number that contradicts `docs/stats.json`.

| metric | value |
|---|---|
| implementation version | 2.2.0 |
| proof suite (tests/) | 116 files / 100 proofs / 1,420 assertions (100 passed, 0 failed) |
| proof suite (apps/) | 1 files / 1 proofs / 7 assertions |
| granted-lane proofs | 7 files under explicit operator cells |
| differential harness | 147 match / 0 diverge (5 granted targets) |
| red-team suite | 100 attacks contained, 0 breached (97 committed payloads + runtime fixtures) |
| stdlib | 16 modules, 168 genes |
| keywords | 58 |
| LSP methods | 12 |

## std modules

`std/args`, `std/bio`, `std/collections`, `std/csv`, `std/fmt`, `std/fs`, `std/iter`, `std/json`, `std/math`, `std/motifs`, `std/path`, `std/random`, `std/seq`, `std/set`, `std/strings`, `std/testing`

## keyword inventory

`gene`, `let`, `if`, `elif`, `else`, `while`, `loop`, `for`, `in`, `return`, `break`, `continue`, `match`, `case`, `use`, `tad`, `anchor`, `export`, `import`, `enhance`, `silence`, `stress`, `rescue`, `raise`, `fate`, `state`, `regulate`, `activates`, `inhibits`, `strength`, `toggle`, `repressilator`, `period`, `frame`, `proof`, `guard`, `splice`, `variant`, `edit`, `replace`, `ires`, `as`, `collect`, `enter`, `phenotype`, `sequence`, `yield`, `new`, `threshold`, `from`, `self`, `decoy`, `ligand`, `autoinducer`, `bind`, `inducer`, `cofactor`, `operon`

## LSP methods

`exit`, `initialize`, `shutdown`, `textDocument/completion`, `textDocument/definition`, `textDocument/didChange`, `textDocument/didClose`, `textDocument/didOpen`, `textDocument/documentSymbol`, `textDocument/formatting`, `textDocument/hover`, `textDocument/publishDiagnostics`
