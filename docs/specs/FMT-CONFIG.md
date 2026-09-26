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

Unknown keys and out-of-range values are **reported on stderr and ignored**
(Total Grammar spirit: forward-compatible, never silently swallowed).
Section headers (`[fmt]`) and `#` comments are accepted.

Example `.operon-fmt.toml`:

```toml
indent = 4
quotes = "double"
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
- `--width N` (soft wrapping) is deferred to W47-v2: wrapping changes token
  layout and must not ship before the byte-stability law below is proven
  over it.

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

`tools::FmtConfig { indent, quotes }`, `tools::QuoteMode::{Double, Single}`,
`tools::format_program_with(prog, &cfg)`; `tools::format_program` is the
default-config wrapper (used by the LSP's `textDocument/formatting`, which
always formats canonical — editor formatting never injects wobble notes).
`tools::parse_fmt_config(&str)` is the zero-dependency config parser.
