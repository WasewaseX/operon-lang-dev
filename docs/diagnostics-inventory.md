# Diagnostics emit-site inventory (W101, SPEC 9a.1)

The complete map of every diagnostic surface the toolchain emits: which
channel it flows through, which error code derives from it, what location
data it carries, and who consumes it. This file is the stage-1 deliverable
of the W101 errors epic; the render contract lives in SPEC 9a.1, the user
guide lives in docs/errors.md.

Rules the inventory obeys:

- Codes are stable strings. New families append, never renumber.
- Stress codes (E1xxx) derive from `(kind, message)` via `code_for` in
  src/diag.rs; note codes (E2xxx) derive from the message alone via
  `note_code` in the same module. Both are pure functions, so new emit
  sites inherit codes without touching the catalog.
- Unknown location data is `null` / line 0 / no caret — never guessed.
- The differential harness compares stdout only; every stderr rendering
  here is presentation, owned by the W101 engine (the stress VALUES and
  messages themselves are the parity surface, mirrored by the oracle).

## Channel map

| Channel | Where | Shape | Codes |
|---|---|---|---|
| Fatal stress block | `run`/`debug` stderr | located rustc block, W007 chain after | E1xxx |
| Fatal JSON | `run --json-errors` stderr | one object: code/kind/message/file/line/column/length/chain/help/labels/suggestions | E1xxx |
| Finding block | `check`/`lint` stdout | located severity block (`error[E01]:`, `warning[W07]:`, `style[N12]:`) | W041-era rule codes |
| Phantom block | `check` warning section | located block + did-you-mean + fix | W01 |
| Check JSON | `check --json` stdout | per-file report; phantoms are `{name,line,suggestions,fix}` objects | W01 |
| Note flush | `run`/`debug` stderr, after output | `[tag code] file:line: message` lines | E2xxx |
| Explain | `operon explain` stdout / `--json` | `[tag code] line N: message` / `{line,rung,rung_name,code,message}` | E2xxx |
| Contained stress | top-level containment note (stderr) | `[contained] [kind] file:line: message` + chain | kind only (W007 shape, pre-W101) |
| CLI usage errors | `die()` paths, stderr | one-liners (usage, unreadable file, missing arg) | none (arg parsing, rc 2) |

## E1xxx — fatal stress catalog (code_for, src/diag.rs)

| Code | Kind / verb | Meaning |
|---|---|---|
| E1000 | any unclassified | the kind is the truth; no family matched |
| E1002 | `missing` | absent value: key not found, index out of range, file errors |
| E1003 | `unfolded` | type/shape mismatch incl. soft annotation failures |
| E1004 | `unwrap` | unwrap of none/err without a fallback |
| E1010 | `read` denied | capability denial, read family |
| E1011 | `write`/`append` denied | capability denial, write family |
| E1012 | `net`/`http`/`serve` denied | capability denial, network family |
| E1013 | `exit`/`env` denied | capability denial, process/environment family |
| E1014 | `spawn` denied | capability denial, child process family |
| E1015 | `py` denied | capability denial, Python bridge family |
| E1016 | `clock` denied | capability denial, time family |
| E1019 | other interference | denial whose verb has no family yet |
| E1020 | `overflow` | arithmetic overflow / step budget / recursion (W32 contract) |
| E1021 | `burned` | fuel exhausted (regex/json/spawn ceilings) |

Location data on the fatal surface: raise line always (stresses carry it);
the caret row/JSON column appears only for capability denials whose failing
call name is found on the raise line (`locate_span`), because the AST is
line-only today (dx-r2 spans). Suggestions appear when the failing name has
did-you-mean neighbors (see slice 6 contract in SPEC 9a.1).

## E2xxx — parse/repair note catalog (note_code, src/diag.rs)

Notes are the Total Grammar transparency channel: the lexer and parser
REPAIR instead of reject, and every repair emits a note. Codes derive from
the message at flush time; a note whose message matches no family renders
uncoded (honest absence). The 10,000-note cap (SPEC §9b table) applies to
every emitter; the cap notice itself is a coded note.

| Code | Family | Rung | Emitted by |
|---|---|---|---|
| E2000 | note cap reached, further notes suppressed | 4 | lexer, parser, interp |
| E2001 | single-quoted string repaired to double quotes | 4 | lexer |
| E2002 | single-quoted bytes literal repaired | 4 | lexer |
| E2003 | malformed `\x` escape kept verbatim | 4 | lexer |
| E2004 | non-ASCII char in bytes literal encoded as UTF-8 | 4 | lexer |
| E2005 | unclosed bytes literal consumed to end of input | 4 | lexer |
| E2006 | unclosed multiline / raw string consumed | 4 | lexer |
| E2007 | stray `@` skipped | 4 | lexer |
| E2008 | integer literal out of range treated as 0 | 4 | lexer |
| E2009 | malformed number treated as 0 | 4 | lexer |
| E2010 | synonym repaired (`elseif` → `elif`, keyword synonyms) | 2 | parser |
| E2011 | wobble: unknown gene/mark repaired to nearest | 3 | parser, interp |
| E2012 | unmatched `}` skipped | 4 | parser |
| E2013 | block auto-closed at EOF (trait/match/mark payloads) | 4 | parser |
| E2014 | generic skipped token/line (catch-all, matched last) | 4 | parser |
| E2015 | `let`/`const` without value binds null | 4 | parser |
| E2016 | annotation payload missing (`needs ...`), missing `in` | 4 | parser |
| E2017 | construct "treated as" something else (bare block, bare name block) | 4 | parser |
| E2018 | unknown mark `@x` skipped | 4 | parser |
| E2019 | expression replaced with null (unexpected token) | 4 | parser, interp |
| E2030 | phantom call to `x`; result null (runtime) | 4 | interp |

Runtime operational notes that intentionally render uncoded today (they are
operator-facing status, not repairs): cell config unreadable, rna patch
applied/matched-nothing, grant changes, bridge timeouts, NMD markers. New
families get the next free E20xx id in src/diag.rs `note_code` and a row
here, in the same commit.

## Finding streams (W041/W048-era codes)

The `check` correctness stream and `operon lint` style stream carry their
own stable rule codes (E00/E01..., W01–W09, N12...): W01 phantom-call,
W04 wrong-arity, W07 unused-binding, E00 unreadable file, and the rest
documented at the top of src/lint.rs and in the `operon lint/check` usage
text. Their rendering went through the W101 engine in slice 2 (located
blocks, severity palette, rule provenance note); slice 6 added evidence
(call line, suggestions, machine-applicable fix) to phantoms.

## Machine-applicable fixes

`SuggestedFix` (src/diag.rs) is the structured edit: line, char column,
length, replacement, note. Produced today by the phantom sweep when the
unknown call token is locatable on its call line (word-boundary locating:
`min` inside `admin` never underlines). Consumers: `check --json`
(`fix` field per phantom). The `--allow-*` grant suggestions on denial
help lines are CLI-flag advice, deliberately NOT SuggestedFix — they edit
the command line, not the source.
