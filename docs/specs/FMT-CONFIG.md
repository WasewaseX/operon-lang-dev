# Formatter configuration (W47, ROADMAP-100)

Normative for `operon fmt`. Conflicts resolve toward SPEC.md, then this file.

## Precedence

```
flags  >  .operon-fmt.toml (or --fmt-config PATH)  >  defaults
```

A config file is optional; when neither file nor flags are given the defaults
apply. Explicit `--fmt-config PATH` that does not exist is a hard error; the
default `.operon-fmt.toml` path is only read when present (so formatters stay
invocable from any directory).

## Keys

| key | values | default | meaning |
|---|---|---|---|
| `indent` | integer 1..=16 | `2` | spaces per nesting level. Default is 2 because that is the de-facto house style of every checked-in `.op` file — a formatter whose default reformats the whole corpus is a broken default. |
| `quotes` | `double` \| `single` | `double` | how plain string literals re-emit. |
| `width` | integer 0..=10000 | `0` (off) | soft line-width limit. `0` = off (the historical behavior; also what LSP formatting uses). When ≥ 1, lines longer than `width` wrap at parser-safe comma points — see the next section. |

Unknown keys and out-of-range values are **reported on stderr and ignored**
(Total Grammar spirit: forward-compatible, never silently swallowed).
Section headers (`[fmt]`) and `#` comments are accepted.

Example `.operon-fmt.toml`:

```toml
indent = 4
quotes = "double"
width = 80
```

## Quote semantics (honest notes)

- `double` is the canonical form (SPEC §3) and the default.
- `single` re-emits `'…'` **only when it is byte-lossless**: the string value
  contains no `'`, no backslash, no brace, no newline/tab — i.e. both
  spellings denote the identical value with zero escaping. Anything else
  (interpolated strings included) stays double.
- Single-quoted output is **lossless but non-canonical**: re-parsing it
  yields the lexer's quote-repair wobble note (rung 3, "single-quoted string
  repaired to double quotes") per literal. That is by design and documented;
  the value and token stream are unchanged.
- `preserve` is impossible **by design**: the AST stores the string's VALUE,
  not which quote character the source used, so there is nothing to preserve.

## Deliberately not shipped (honesty ledger)

- `--canonical` would be a no-op flag: the parser repairs synonym spellings
  into the canonical AST and fmt prints the AST, so keyword canonicalization
  is already inherent. Not shipped.

## `width` — the W47-v2 safe-break contract

The width pass is a **post-print wrapper**: it runs after the canonical AST
render and inserts newlines ONLY at comma positions the parser provably
tolerates — the element loops that call `eat_newlines_inline()` at their top:
bare-call args, method and `?.` args, gene/sequence parameter lists (with
annotations and defaults), and list literals. It is a pure function of
(canonical text, width, indent), so it cannot oscillate.

Breakable: a comma whose enclosing bracket stack contains ONLY `(`/`[`.
Chosen first: the OUTERMOST such group; its commas all break (hanging-indent
style, continuation indent = line indent + depth × `indent`). Continuation
segments that still overflow recurse into deeper groups. Closers stay glued
to the last element.

Never breakable (pinned by tests/fmt_width.rs):

- **openers** — the first element stays on the opener line;
- **depth-0 commas** — multi-assign target/value lists (`let a, b = 1, 2`):
  a newline there is not parser-tolerated;
- **anything under `{`** — a text pass cannot distinguish a block brace from
  a map-literal brace, so both are poisoned (map literals never wrap);
- **string contents** — including interpolation regions, which contain real
  code; the break lands after the whole string or not at all;
- **directly inside index brackets or parenthesized single expressions** —
  those parsers have no newline tolerance; nested CALL commas inside them
  may break (tolerated by the call-arg loop), the bracket itself never does.

A line with no breakable comma stays long — visibly, honestly.

Laws enforced over the whole fmt corpus (std/ + tests/ minus redteam +
examples/ + apps/) at widths 20/40/60/80 by `tests/fmt_width.rs`:

1. **AST identity** — the AST of the wrapped output equals the AST of the
   canonical render (`operon ast` Debug dump).
2. **zero new notes** — the wrapped output re-parses with exactly the
   canonical render's notes (by construction: none). A break the parser did
   not tolerate would surface here as a rung-3/4 repair note.
3. **idempotence** — `fmt(w) ∘ fmt(w) == fmt(w)`, byte-exact per config.

LSP formatting (and `format_program`) never sets `width`: editors keep their
own wrap policy and never inherit a width-driven diff.

## Byte-stability law (enforced by tests)

`fmt(fmt(x, cfg), cfg) == fmt(x, cfg)` for every supported config, enforced
corpus-wide (std/ + tests/ minus redteam + examples/ + apps/) by
`tests/fmt_idempotence.rs` in both the default config and the extreme config
(indent 4 + single quotes), on every platform — the test runs inside the
standard `cargo test` gate including the blocking Windows job.

Additionally: fmt output re-parses with **no rung-4 fallback ever**, and with
no rung-3 note at all under the default config (the single-quote repair note
is the one documented exception under `quotes = single`). Config changes may
rearrange whitespace and quote spelling; they may never alter meaning.

## Library surface

`tools::FmtConfig { indent, quotes, width }`, `tools::QuoteMode::{Double, Single}`,
`tools::format_program_with(prog, &cfg)`; `tools::format_program` is the
default-config wrapper (used by the LSP's `textDocument/formatting`, which
always formats canonical — editor formatting never injects wobble notes and
never wraps). `tools::parse_fmt_config(&str)` is the zero-dependency config
parser.
