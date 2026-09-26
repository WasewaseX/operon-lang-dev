# SERIALIZATION contract (W34, ROADMAP-100)

Normative for the serialization surface of Operon. Conflicts resolve toward
SPEC.md (§10 builtins, §9 error hierarchy), then this file.

## Scope and staging

- **Stage 1 (this document, landed)** — the value layer: `null`, `bool`,
  `int`, `float`, `str`, `list`, `map`, and the Option/Result wrappers.
  Surface: `std/serialize.op` (the dispatcher) over the `json_str` /
  `json_parse` builtins and std/csv semantics.
- **Stage 2 (gated on W04 traits)** — phenotype instances. The hook is
  specified below and frozen NOW so code that adopts `std/serialize` keeps
  working when it lands.

## Naming contract

The dispatcher owns exactly two verbs per format, one shape each:

| verb | signature | failure shape |
|---|---|---|
| `to_<fmt>(v)` | value → text | **total** for supported shapes (returns text or `err(...)` — never a stress across the API line, never a silent null) |
| `from_<fmt>(text)` | text → value | **Result**: `ok(value)` / `err(message)` (D-014 — expected failures are values) |
| `serialize(v, fmt)` / `deserialize(text, fmt)` | format-generic mirror of the pair | `unsupported format` → `err(...)` |

Names are the `to_x` / `from_x` pair, snake_case, matching `to_csv`-class
conventions already in std. `supported_formats()` returns the inventory so
callers branch instead of guessing.

## Error contract (why Result, not null+note)

Pre-W34 helpers returned null on parse failure, which conflates "the text
encodes null" with "the text is junk". Under D-014 the dispatcher returns
`ok/null` from `from_json(null-literal)` and `err(...)` for junk — the tag
IS the distinction. `--strict` stays out of it: a bad payload is an expected
failure, not a contract violation.

## Round-trip table (json format)

| shape | round-trip | notes |
|---|---|---|
| null, bool, int | exact | |
| float | exact when finite | non-finite floats emit `null` (RFC 8259 honesty in `json_str`) — `roundtrip()` reports **false** for them instead of pretending |
| str | exact | UTF-8 text; escapes canonicalized |
| list | exact, recursively | |
| map | exact, recursively | keys are strings; re-parse preserves document order so structural `==` holds |
| option / result | **wire fidelity only** | `{"ok":…}` / `{"err":…}` / `null`; the tag is NOT rebuilt on parse — a single-key map is indistinguishable from a map BY DESIGN. Callers that need the tag rebuild it (`is_ok`/`has_key`) — see the pinned proof |

`roundtrip(value, fmt)` implements this table as a runnable check; the
`tests/std_serialize.op` proof pins every row. Pinned limitation: `nan` /
infinite floats are the one value-layer shape that does not round-trip, and
the proof asserts the REPORTING of that, not a silent false promise.

## The W04 trait hook (frozen now)

When W04 traits land, phenotype instances serialize through the same
signature — no new verb, no breaking change:

```operon
trait Serializable {
    gene to_map()      # phenotype -> value-layer map
}

# dispatcher behavior, stage 2:
#   serialize(instance, "json")  ->  to_json(instance.to_map())
```

Rules the hook must obey (checked at stage-2 review):

1. **One method, `to_map()`, returning the value layer.** No
   `to_json()`/`to_csv()` per format on the trait — formats compose from
   maps; N formats × M types stays N + M.
2. **Round-trip ownership is the phenotype's.** A `Serializable` that
   promises `from_map()` round-trips; the dispatcher cannot enforce it and
   must not pretend to (the `roundtrip()` helper stays value-layer).
3. **Option/Result fields keep the wire shape above.**

## Downstream rule

Code that serializes phenotype instances today should convert to maps
explicitly and call this module. When stage 2 lands, the explicit
`to_map()` call site simply becomes the trait method — the diff is
mechanical and the `operon fix` migrator (W65) is the designated tool for
it.
