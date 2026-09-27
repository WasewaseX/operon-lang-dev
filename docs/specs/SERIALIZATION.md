# SERIALIZATION contract (W34, ROADMAP-100)

Normative for the serialization surface of Operon. Conflicts resolve toward
SPEC.md (§10 builtins, §9 error hierarchy), then this file.

## Scope and staging

- **Stage 1 (landed)** — the value layer: `null`, `bool`, `int`, `float`,
  `str`, `list`, `map`, and the Option/Result wrappers. Surface:
  `std/serialize.op` (the dispatcher) over the `json_str` / `json_parse`
  builtins and std/csv semantics.
- **Stage 2 (landed 2026-09-27)** — phenotype instances, as a STRUCTURAL
  default (see below). The W04 trait hook below stays specified and frozen
  as the future OVERRIDE layer; code that adopts `std/serialize` keeps
  working unchanged when it lands.

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
| phenotype instance | exact, recursively (stage 2) | emits the canonical wire map — field map + hidden `"#phenotype"` identity key, the SAME shape the `spawn` boundary uses (SPEC §7a); `from_json` rebuilds a REAL instance (class must be declared in the deserializing program; unknown name → `err(...)`). Reconstruction is DATA restore: `init` does NOT re-run and field defaults do NOT apply — the wire is the truth; absent fields read as null + note |

`roundtrip(value, fmt)` implements this table as a runnable check; the
`tests/std_serialize.op` proof pins every value-layer row and
`tests/std_serialize_pheno.op` pins the instance rows. Pinned limitations:
`nan` / infinite floats are the one value-layer shape that does not
round-trip, and the proof asserts the REPORTING of that, not a silent false
promise — the same honesty applies to non-finite fields INSIDE instances
(`roundtrip()` reports false; the wire nulls them).

## Stage 2: the structural default (landed) and the reserved key

An instance serializes as its field map plus the hidden `"#phenotype"`
identity key written FIRST — byte-identical to what `spawn` already puts on
the wire (one canonical phenotype wire format across the language, not two):

```text
to_json(new Point())          == {"#phenotype":"Point","x":0,"y":0,"label":"origin"}
from_json(that text)          == a real Point (methods dispatch; area() works)
```

Rules the structural default obeys (pinned in `tests/std_serialize_pheno.op`):

1. **`"#phenotype"` is a RESERVED wire key.** A plain map carrying it
   rebuilds as an instance (spawn already implies this reservation); an
   instance field cannot use the name — the phenotype surface has no way to
   declare such a field, and the dispatcher refuses the collision outright
   (`unfolded` stress shaped into an `err(...)` at the API line) rather than
   silently dropping or renaming.
2. **Reconstruction is data restore, not construction.** `init` does not
   re-run; field defaults do not apply; the wire carries what the instance
   carried. A rebuilt instance dispatches methods (they ride the class
   definition).
3. **Equality honesty.** `==` on instances is DATA equality: same class
   name AND deep-equal field values, on both engines — dev1's parity
   resolution, answered from THIS module's stage-2 finding (before the
   ruling, Rust compared the shared class definition — any two same-class
   instances were `==` regardless of their fields — while Python fell
   through to identity, so the engines DISAGREED and `==` was treated as
   unspecified here). `roundtrip()` uses the same structural `==` for
   every shape, instances included: the rebuild is a distinct object with
   equal fields, so it `==` the original. The explicit escape hatch
   `object_fields(a) == object_fields(b)` stays valid for programs that
   want the field map as a value.
4. **The primitives are public builtins** (SPEC §10): `is_object(v)` (silent
   predicate), `object_fields(o)` (shallow field-map copy; non-instance →
   null + note), `object_from_map(m)` (the rebuild; unknown class →
   catchable `unfolded` stress). Any caller can compose its own format from
   the same shape.

## The W04 trait hook (frozen — future OVERRIDE layer)

When W04 traits land, phenotype instances may declare the hook to CUSTOMIZE
their value-layer projection — the structural default above remains the
fallback for every phenotype that does not, and no signature changes:

```operon
trait Serializable {
    gene to_map()      # phenotype -> value-layer map
}

# dispatcher behavior, stage 2:
#   serialize(instance, "json")  ->  to_json(instance.to_map())
```

Rules the hook must obey (checked at the W04-stage review):

1. **One method, `to_map()`, returning the value layer.** No
   `to_json()`/`to_csv()` per format on the trait — formats compose from
   maps; N formats × M types stays N + M.
2. **Round-trip ownership is the phenotype's.** A `Serializable` that
   promises `from_map()` round-trips; the dispatcher cannot enforce it and
   must not pretend to (the `roundtrip()` helper stays value-layer for
   trait-projected instances).
3. **Option/Result fields keep the wire shape above.**
4. **The tag stays.** A trait-projected map is still emitted with the
   `"#phenotype"` identity key (the class name); `from_map()`-style custom
   reconstruction is the trait's promise to deliver, not the dispatcher's.

## Downstream rule

Code that serializes phenotype instances today can use this module directly
— the structural default is live. When the W04 hook lands, a phenotype that
wants a CUSTOM projection declares `Serializable`, implements `to_map()`,
and the dispatcher prefers it; call sites do not change.
