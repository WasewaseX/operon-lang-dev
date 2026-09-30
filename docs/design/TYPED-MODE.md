# TYPED-MODE.md — the Operon static type system (typed mode)

Status: ACTIVE design (builder-B, session-8). Implemented by `src/typeck.rs`.
Law: **additive**. The dynamic side is untouched — typed mode is a compile-time
gate, never a new runtime. Programs without annotations check trivially clean
of obligations they did not state.

## 1. Goal

Catch before runtime what the dynamic side catches at runtime (as recoverable
Stress) or never catches at all:

```
x = 10
x.name()        # static error: int has no member 'name'
```

The check is opt-in per invocation (`operon run --typed`, `operon check
--typed`) and per-file enforcement can be driven from CI. REPL and scripting
keep today's dynamic semantics byte-for-byte.

## 2. Surface syntax (all of it already soft-parses or is added additively)

```text
gene add(a: Int, b: Int) -> Int { return a + b }     # capitalized aliases OK
gene first<T>(xs: list<T>) -> T { return xs.get(0) } # generic gene
gene m<T: numeric>(a: T, b: T) -> T { ... }          # bounded type parameter
let n: int = 3                                       # W01 annotated binding
let s: list[str] = ["a"]                             # generic annotation
let r: result[int, str] = ok(1)                      # Result annotation
let o: int? = none()                                 # Option shorthand
map[str, int]                                        # generic annotation
```

Capitalization is normalized (`Int`≡`int`, `Str`≡`str`, ...): the user-facing
canonical form is capitalized; the runtime W01 soft contract accepts both
after this design (both cores normalized identically — oracle parity kept).

## 3. The type lattice (checker-internal, `Ty`)

`Null Bool Int Float Str Bytes List(T) Map(K,V) Gene Seq Channel Weak
Pheno(name) Option(T) Result(T,E) Union(..) Param(name) Any Never`

- `Union` is flattened/deduped; `Any` absorbs; `Never` is the bottom
  (unreachable / always-stress) and absorbs into anything.
- Options/Results are structural (`Option(int)`), not nominal: `none()` is
  `Option(Any)` until unified, `ok(v)` is `Result(T, Any)` until an `err`
  pins `E`.

## 4. Annotation grammar → Ty

| annotation              | Ty                        | notes                        |
|-------------------------|---------------------------|------------------------------|
| `int`, `Int`            | Int                       | aliases, case-insensitive    |
| `float str bool null any bytes gene sequence channel weak phenotype` | primitive | |
| `t?`                    | Option(inner)             | W01 shorthand                |
| `option[t]`             | Option(t)                 |                              |
| `result[t, e]`          | Result(t, e)              |                              |
| `list[t]`               | List(t)                   | bare `list` = List(Any)      |
| `map[k, v]`             | Map(k, v)                 | bare `map` = Map(Any, Any)   |
| `list`, `map` (bare)    | element-free              | matches any instance         |
| `Some(x)` family        | n/a (patterns only)       | constructors are builtins    |
| unknown capitalized     | Pheno(name) if declared, else Any + finding | soft, never a reject |
| unknown lowercase       | Any + finding T04         | soft, never a reject         |

## 5. Constructors and destructurers (runtime forms, statically known)

- Constructors: `some(v)`, `none()`, `ok(v)`, `err(e)` builtins;
  `Variant` values carry them at runtime (W06).
- `e?!` propagation (W06): requires `Option(_)`/`Result(_, _)`/`Any`;
  yields the payload type. On a non-variant type: finding T06.
- Patterns: `Some(p)`, `None`, `Ok(p)`, `Err(p)`, list/map/lit/bind/or/guard.

## 6. Inference

Constraint-based, statement-local, flow-joined:

- `let x = e` binds `x: infer(e)`. `let x: T = e` unifies `infer(e)` with `T`
  (finding T01 on mismatch) and binds `x: T`.
- Assignment to an existing binding: unify with the binding's type; a
  type-CHANGE (int → str) is finding T09 (strict), int → float widening is
  legal (runtime numerics already widen).
- Branch join: `if`/ternary produce `Union(then, else)`; `a and b` /
  `a or b` → `Union(a, b)`; `a ?? b` → `payload(a) ∪ b`.
- Unannotated genes: parameters are `Any` at body-check time (fresh TyVars,
  never unified across calls — one body, one solving, call sites check
  against the inferred signature). Return type = union of all `return`
  expressions + implicit Null when fallthrough is possible.
- Recursion: unannotated recursive genes see their own return as `Any`
  (no fixpoint iteration; documented limit, annotations lift it).
- Binaries: the exact dynamic lattice from `interp.rs` (Div→Float,
  FloorDiv→Int, Pow int**int≥0→Int else Float, Add Str/Str→Str,
  List+List→List, Mul Str×Int→Str, bitops need Int/Bool). A binary that can
  only stress at runtime (`int + str`) is finding T03.
- Calls: arity vs known defs (hard), argument types unified with formal
  annotations (T03), generic call sites solve the parameter map, then bounds
  are re-checked on the SOLUTION (T08). Return = formal ret with the solved
  map applied.
- Methods: builtin per-type method table mirrored from `call_method`
  (str/bytes/list/map/seq surfaces). Unknown member on a known non-phenotype
  receiver = finding T02 (the `x = 10; x.name()` catch). Phenotype receivers
  check their lineage + implemented traits (T02 on real misses; `Any`
  receivers are never flagged — dynamic escape hatch).

## 7. Generics

Declared `gene first<T>(xs: list<T>) -> T`. `<...>` after the gene name is
parsed only in signature position (unambiguous). Type parameters:

- scope: the gene body + its annotations;
- bounds: `T: numeric` or `T: SomeTrait` (builtins: `numeric`, `comparable`);
- solving: call-site unification (arg types vs formal types); after solving,
  bounds re-checked (T08); unknown/unsolved parameter defaults to `Any`
  (soft).
- Bounds:
  - `numeric` — Int, Float (and Any/TyVar passes).
  - `comparable` — Int, Float, Str, Bytes, Bool, lists/maps of comparables.
  - trait bound — the argument type must be a phenotype that (transitively)
    implements the trait, or provides the trait's required methods.

## 8. Traits (static side)

Trait declarations (W04 runtime) now carry static contracts:

- A phenotype `implements T` must define every REQUIRED trait method with a
  compatible signature — checked at the phenotype definition site (T07:
  missing; T03-compatible: signature mismatch on annotated ones).
- Trait DEFAULT methods are callable on implementing phenotypes statically.
- Trait names usable as annotation types (`let x: Drawable = ...`) — the
  value must be a phenotype implementing it (or Any).

## 9. Match exhaustiveness

Per scrutinee static type:

- `Option(t)` must cover `Some(_)` and `None`; `Result(t, e)` must cover
  `Ok(_)` and `Err(_)`. Bind/Wild cover everything. Or-patterns contribute
  all their alternatives; guarded arms contribute NOTHING (condition may be
  false). Missing variants: finding T05 naming the uncovered constructor(s).
- Bool-literal scrutinee: `true` and `false` must be covered.
- Union scrutinee: each member must be covered by some arm (literal or
  bind).
- List/Map/literal matches: no exhaustiveness law (bind covers the rest);
  impossible arms after a catch-all are W06's existing job (lint stream).
- Scrutinee `Any`: never flagged.

## 10. Findings (T-series, W041 code scheme, `check` stream)

| code | rule                  | meaning                                           |
|------|-----------------------|---------------------------------------------------|
| T01  | type-mismatch         | inferred type contradicts an annotation           |
| T02  | unknown-member        | member/method does not exist on the receiver type |
| T03  | arg-type-mismatch     | argument cannot satisfy the formal (incl. impossible binary) |
| T04  | unknown-type          | annotation names nothing known (soft: treated as any) |
| T05  | non-exhaustive-match  | variant/literal cases uncovered                   |
| T06  | propagate-non-variant | `?!` on a type that cannot be Option/Result       |
| T07  | trait-method-missing  | implements-declaration lacks a required method    |
| T08  | bound-violation       | generic argument violates its declared bound      |
| T09  | assign-type-change    | assignment changes a binding's static type        |
| T10  | return-missing        | annotated `-> T` gene can fall through to Null    |

Severity: T01/T02/T03/T05/T07/T10 = Error; T04/T06/T08/T09 = Warning
(strict mode `--typed --strict` promotes warnings to errors at exit time;
exit codes reuse `check`'s: 3 = findings, 0 = clean).

## 11. CLI surface

- `operon run --typed file.op` — static check first; any ERROR (or warning
  under `--strict`) aborts BEFORE execution. Otherwise run (dynamic).
- `operon check --typed file.op` — check stream includes T-series.
- `--typed` is inert on files that opt nothing in — zero-cost adoption.

## 12. Oracle parity

- Runtime `ann_matches` normalization (capitalized aliases) lands in BOTH
  cores byte-identically; the differential harness re-proves 1:1.
- The static pass is Rust-side tooling (like lint): the oracle does not
  re-implement it; no dynamic behavior change exists to mirror.
- Parser additions (`[...]` type args, `<T>` type params) are Total-Grammar
  soft: they only engage in annotation/signature positions, and existing
  programs never lex there, so parse trees of old programs are untouched.

## 12a. Span discipline (v1)

Expression-borne findings (T02 member/method, T03 binaries/calls, T06
propagation, T05 on match) carry EXACT source lines: `Call`, `Index`,
`Binary`, `Propagate`, `Method`/`MethodSafe` and `Match` all stamp their
line in the AST (A13 span parity). Pure statement-level findings (T01 on
`let`-annotations, T09 assignments, T10/T05 edge forms whose scrutinee has
no stamp) are best-effort line 0 until statements gain their own stamps
(parser work queued behind this design, not a language-semantics change).

## 13. Non-goals (this iteration)

- No type-directed evaluation, no monomorphization, no runtime casts.
- No closure-signature annotations (lambda inference = call-shape only).
- No cross-module import types (module values are `Any` at the boundary).
- No flow-sensitive narrowing inside match arms beyond the arm's own bind.
