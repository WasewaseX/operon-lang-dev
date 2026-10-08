# NUMERIC-ARRAYS.md — the typed numeric array contract (R0.2)

Status: **normative** for `std/arrays.op` (roadmap R0.2 "Typed numeric
arrays", APPROVED 2026-10-04). Lowering class: **C4** (stdlib, pure `.op`,
zero engine delta) with the packed-buffer kernel declared as the future
**C7** native-kernel lowering (§7). Proof battery: `tests/arrays.op`;
byte pin: `tests/differential/arrays_pin.op` (vm / tree-walk / oracle
3-lane identical).

## 0. Why this module exists

The quantitative bio waves (BD1 promoter occupancy, BD2 stochastic
physics, BD2.3 rate-law stdlib) all need numeric vectors and matrices
whose element type, shape, and accumulation order are *declared*, not
accidental. A heterogeneous list cannot carry those guarantees, and a
silent float collapse at 2^62 would silently corrupt exact regulatory
arithmetic. This module is the semantic substrate: six laws, every one
executable, every refusal catchable with its exact text pinned.

## 1. Representation

An array is a map:

```text
{ "tag": "numarr", "dtype": "int" | "float",
  "shape": [d0] | [rows, cols],      # rank 1 or 2 in v1, every dim >= 1
  "data":  [e0, e1, ...] }           # flat row-major, len == product(shape)
```

`int` arrays preserve exact i64 arithmetic end to end. `float` arrays are
f64 everywhere (DETERMINISM §5.1). Empty vectors are refused (every dim
≥ 1): one law, no special case. Rank ≥ 3 is refused in v1 with an exact
refusal — the honest scope note, not a hidden limit.

## 2. The six laws

**A1 — type law.** Every stored element matches the declared dtype
exactly: a numarr never stores null, a string, or a mixed element.
Construction is the ONE declared coercion point: `arr_from_list(ls,
"float")` and `arr_new(..., "float")` promote Int elements to Float;
`arr_set` conforms strictly (convert explicitly, the refusal says so).
`arr_validate()` is the DEEP check — it walks every element; the other
entry points do the cheap structural gate (tag/dtype/shape/length) and
trust the type law between calls, exactly as `biocore_validate` relates
to `biocore_add_reaction`.

**A2 — shape law.** Element-wise operations require identical shapes;
the refusal names both shapes. `arr_matmul` requires `(m×k)·(k×n)`;
`arr_reshape` requires the same total size; `arr_get`/`arr_set` require
an index list of the array's rank with in-bounds components (negative
indices refused — no hidden wrap). Every refusal text is pinned in the
proof battery.

**A3 — host-mirror arithmetic.** Element-wise arithmetic NEVER invents
its own numeric semantics (the L2 discipline): each element operation is
literally the host's operator. Therefore int⊕int stays int, any float
operand yields float (including a float scalar RHS on an int array — the
host's `1 + 2.0` law), and division is the host's TRUE division: it
ALWAYS yields Float, even for `int / int` arrays, even when the quotient
is exact (`[4] / [2]` is `[2.0]`, not `[2]` — DETERMINISM §5.6). The one
declared deviation: **division by zero is refused eagerly** — scalar RHS
or per-element, flat index named — because A1 outranks the host's scalar
null-repair INSIDE a typed container; a numarr never stores the null the
host's contained stress would deliver. The host's repair law is untouched
for scalars; this is container law, and it is stated here, not discovered
at tick time (the INV4 precedent).

**A4 — order law.** Every reduction is ONE sequential left-to-right pass
in flat row-major order. No pairwise tree, no parallel reduction order —
the class of float drift that bit BD2.1's propensity design cannot even
form here. Int sums are EXACT (pinned at 2^62: no float collapse).
Float sums pin the host's f64 accumulation bit-for-bit (`0.1 + 0.2 +
0.3 == 0.6000000000000001`, CPython parity). `arr_mean` is ALWAYS Float:
the exact sum divided by the count through the host's true-division law,
one division, at the end — never running means. This module contains NO
transcendentals: every operation is inside DETERMINISM §5.2's
bit-identical-across-platforms promised set, so array programs are
byte-reproducible across the release matrix by construction.

**A5 — stability law.** `arr_argmin`/`arr_argmax` return the FIRST
occurrence in flat row-major order. Ties are not engine dice (the #137
stability lesson, stated before it can happen here).

**A6 — copy law.** Every mutating operation (`arr_set`, `arr_reshape`,
`arr_transpose`) returns a NEW array; the input is never mutated (the
stdlib copy law). The readers return COPIES of shape and data lists, so
a caller mutating `arr_shape(a)` cannot reach inside the array.

## 3. Determinism class

| Surface | Class | Note |
|---|---|---|
| construction, get/set, render | deterministic byte-exact | pure .op over host values |
| add/sub/mul, dot, matmul | deterministic byte-exact | promised set only (§5.2) |
| div | deterministic byte-exact | true division; IEEE-754 correctly rounded |
| sum/min/max/mean/argmin/argmax | deterministic byte-exact | A4 sequential order is part of the law |

## 4. Refusal inventory (all catchable `err(...)`, exact text pinned)

construction: unknown dtype · rank ≠ 1,2 · dim < 1 · dim not int ·
non-number fill/element · float into int (lossy) · empty vector ·
indexing: rank mismatch · non-int component · out of bounds ·
arithmetic: shape mismatch (both shapes named) · dot length mismatch ·
matmul inner dims · matmul/transpose rank · division by zero (scalar and
per-element, flat index named) · set: type conformance · non-number ·
reshape: size mismatch · validate: tag, dtype, data length, per-element
conformance (first offender named).

## 5. Function inventory

28 public genes: `arr_new`, `arr_zeros`, `arr_ones`, `arr_from_list`,
`arr_infer`, `arr_shape`, `arr_dtype`, `arr_to_list`, `arr_get`,
`arr_set`, `arr_add`, `arr_sub`, `arr_mul`, `arr_div`, `arr_neg`,
`arr_abs`, `arr_dot`, `arr_matmul`, `arr_transpose`, `arr_reshape`,
`arr_sum`, `arr_min`, `arr_max`, `arr_mean`, `arr_argmin`,
`arr_argmax`, `arr_validate`, `arr_render`. Private helpers carry the
`__ar_` prefix and are not API.

## 6. What this module is NOT (honesty)

It is not the packed execution kernel. The map-of-lists representation is
the C4 semantic prototype — the R0.1 biocore precedent: the substrate is
expressed in the language itself, exercising the exactness and
determinism machinery it will later carry. It is also not a broadcast
system: scalar RHS is the only broadcasting in v1, and rank is capped at
2. Both limits are refusal-with-exact-text, not silent behavior.

## 7. The C7 lowering path (declared, not built)

The future kernel is a VM-native packed i64/f64 buffer behind the SAME
gene names and the SAME law set: A1 becomes a bounds+dtype-checked
memory region, A4's sequential pass becomes the kernel's only sanctioned
accumulation order, A3's promotion table is unchanged, and every result
byte must match this module's — the differential pin and the proof
battery transfer 1:1 as the kernel's acceptance tests. Semantics that
have been validated are preserved; the representation that no longer
serves the model is replaced. That is the R0.9 representation-swap rule,
and it is the reason the contract lands BEFORE the kernel.
