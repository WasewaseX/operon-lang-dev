# Reading Operon's errors

Operon never rejects a program for a repairable problem: the lexer and
parser fix what they can and TELL you what they fixed. When something
fatal does happen, the diagnostic is designed to answer three questions
without re-running anything: what broke, where exactly, and what to try
next. This page explains every shape you will see. The render contract
(SPEC 9a.1) and the full emit-site inventory
(docs/diagnostics-inventory.md) are the machine-level references.

## The fatal block

An uncaught error at the top level renders as a located block on stderr:

```
error[E1010]: read denied, no capability grant covers 'config.json' (grant with --allow-read or --allow-all)

  --> app.op:2
   |
2 |   data = read_file("config.json")
   |          ^^^^
   |
   = no capability grant covers 'config.json' (grant with --allow-read or --allow-all)

help: run with:
      --allow-read
      --allow-all
```

Reading it top to bottom:

- `error[E1010]` — the severity and the stable error code. Codes never
  renumber; new families append. The table below explains each one.
- The message — verbatim from the failure, including the capability verb
  and the path it wanted.
- `--> app.op:2` — file and line. The caret row (`^^^^`) underlines the
  exact call when it can be located honestly; for CJK source text the
  padding uses display width, so the caret stays under its target.
- The `=` row — the provenance: for denials it repeats the grant clause,
  otherwise the stress kind.
- `help:` — the `--allow-*` flags extracted from the denial itself, so
  the advice can never disagree with the failure. When a name looks
  like a typo, the help section leads with a did-you-mean line instead.

If the failure unwound through genes, the call chain prints after the
block, innermost frame first (`at boom (app.op:5)`), capped at 64 frames.

## Error codes (E1xxx, fatal stresses)

| Code | Meaning |
|---|---|
| E1000 | unclassified stress; the `kind` field is the truth |
| E1002 | something needed was missing (key, index, file, entry gene) |
| E1003 | type or shape mismatch |
| E1004 | unwrap of none/err without a fallback |
| E1010-E1019 | capability denials: read, write, net, exit/env, spawn, py, clock; E1019 covers verbs with no family yet |
| E1020 | overflow: arithmetic, recursion depth, step budget |
| E1021 | fuel burned (regex/json/spawn ceilings) |

## Note codes (E2xxx, repairs)

Repairs are not errors. The run flushes them as `[tag code] file:line:`
lines after the program's output (and the REPL prints the same shape):
`info` (rung 1, canonical), `synonym` (rung 2, a legacy spelling repaired),
`wobble` (rung 3, the nearest plausible name), `fallback` (rung 4, the
parser's honest recovery). Codes group the families: E2000 the 10,000-note
cap, E2001-E2009 lexer repairs (quote/escape/number fixes), E2010-E2019
parser repairs (synonyms, auto-closed blocks, null substitutions), E2030
a phantom call that evaluated to null. The full table, including the
families that intentionally render uncoded, is
docs/diagnostics-inventory.md.

`operon explain app.op` prints the play-by-play of every repair with its
code; `--json` adds the code as a field.

## Check and lint findings

`operon check` (correctness) and `operon lint` (style) render findings as
the same style of block, severity-prefixed: `error[E04]:`, `warning[W07]:`,
`style[N12]:`. Phantom calls (a call to a name this file never defines)
carry the most evidence:

```
warning[W01]: phantom call 'maiin' is called but not defined in this file (some similar names: 'main', 'min')

  --> app.op:2
   |
2 |   maiin()
   |   ^^^^^
   |
   = rule: phantom-call
help: machine-applicable fix: replace 'maiin' with 'main'
```

The suggestion shortlist mirrors the runtime wobble ladder exactly (same
edit-distance thresholds), so check never suggests a name the interpreter
would not have repaired to. A typo'd `--entry` is fatal (rc 1) with the
same did-you-mean treatment.

## Machine-readable output

`operon run app.op --json-errors` replaces the block with ONE JSON object
on stderr:

```json
{"code": "E1010", "kind": "interference", "message": "read denied, ...",
 "file": "app.op", "line": 2, "column": 10, "length": 9,
 "chain": [{"gene": "main", "line": null}],
 "help": ["--allow-read", "--allow-all"],
 "labels": [{"line": 2, "column": 10, "length": 9, "text": "", "primary": true}],
 "suggestions": []}
```

Fields a caller cannot know are `null`, never guessed. `labels` carries
every underline; `suggestions` is the did-you-mean shortlist. `check
--json` reports phantoms as `{name, line, suggestions, fix}` objects,
where `fix` is a machine-applicable edit (`{line, column, length,
replacement, note}`) an editor can apply.

## Color and exit codes

ANSI color is automatic and stream-aware: terminals get color, pipes and
files get byte-exact plain text, and `NO_COLOR` always suppresses. Exit
codes: 0 success, 1 uncaught fatal, 2 CLI usage, 3 a check hard error or
a lint `--strict` escalation.

## The golden fixtures

Every shape on this page is pinned byte-for-byte by
scripts/diag_golden.sh (part of the standard gate suite) against the
programs in tests/diagnostics/. The expected files are re-derived from
real runs, never hand-patched, so the documented shapes above cannot
drift silently.
