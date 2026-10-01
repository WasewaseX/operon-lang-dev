# TYPE-SYSTEM.md — the Operon static type system (W01, normative)

Status: **landed** on `ai/type-system` (stage 1 runtime annotations on main;
stage 2 check-time layer + aliases here). Normative for every type-related
surface. Conflicts resolve toward SPEC.md (§7c soft contracts, §16a typed
mode), then this file, then code comments. The design rationale lives in
[docs/design/TYPED-MODE.md](../design/TYPED-MODE.md); the stdlib callable-
generics track is [GENERICS.md](GENERICS.md) (W03) and stays separate.

## 1. The prime directive: additive, never destructive

The dynamic side is the product; the type system is a **compile-time gate**
over it. Three consequences that every change must preserve:

1. **Dynamic semantics are frozen.** The evaluator, the VM, and the oracle
   never consult the static checker. `operon run f.op` (no flag) and the
   REPL behave byte-for-byte as they always did.
2. **The runtime soft annotations (W01 stage 1) are the compatibility
   fallback.** Annotated params/returns/lets keep raising the catchable
   `unfolded` Stress at the boundary; the static checker ADDS precision but
   is never the thing that makes an annotated program run.
3. **Total Grammar holds.** No type syntax is ever a parse rejection;
   malformed or unknown type forms degrade with notes (SPEC §7a family).

The flagship catch the system exists for:

```text
x = 10
x.name()        # T02 unknown-member at compile time (was: null + note at runtime)
```

## 2. Layout (module contract)

| module | owns | never does |
|---|---|---|
| `src/types.rs` | the `Ty` lattice, assignability (`ty_compat`), unions, bounds, the builtin method surface, trait tables | AST walking, findings |
| `src/typeck.rs` | the `Checker` pass, lexical `Env`, `check_program` → `Vec<Finding>` | evaluation, mutation, rejecting parses |
| `src/ast.rs` | `TypeAnn` (Named/Union/Optional/Generic/**Alias**), `Stmt::TypeAlias` | — |
| `src/parser.rs` | annotation grammar, `gene f<T: bound>` signatures, `type Name = ann` resolution, type-param shadowing | type checking |
| `bootstrap/oracle.py` | the op-for-op mirror of EVERY law above | divergence |

`check_program` is pure: it reads the AST and emits `lint::Finding`s in the
check stream (T-codes, W041 scheme). It never rejects a program the parser
accepted.

## 3. Surface (all of it)

```text
gene add(a: Int, b: Int) -> Int { return a + b }      # the user-facing form
gene first<T>(items: list[T]) -> T? { ... }           # generic gene, bracketed args
gene m<N: numeric>(a: N, b: N) -> N { ... }           # bounded parameter
type Metrics = map[str, float]                        # type alias (W01-s2)
type Ids = list[int]      type Row = Metrics          # alias-of-alias
let n: int = 3            let xs: list[str] = [...]   # annotated bindings
let r: result[int, str] = ok(1)    let o: int? = none()   # Option/Result
```

Capitalized primitives are normalized in BOTH cores (`Int` ≡ `int`; the
capitalized form is the user-facing canonical).

## 4. Runtime laws (the compatibility fallback, stage 1 + aliases)

- **Boundary contract.** Param annotations check at every call funnel,
  return annotations on the produced value (including `?!`-propagated
  variants and guard branches). Violation = catchable `unfolded` Stress,
  never a hard failure.
- **Matching** is by `Value::type_name()`, with the documented relaxations:
  `any` accepts all; `float` accepts int (widening); `int` REFUSES float
  (no silent narrowing); `option[t]`/`result[t, e]` match the variant
  FAMILIES; `t?` accepts null + the inner law + the Option family.
- **Generic annotations are shallow at runtime.** `list[int]` enforces
  "is a list", `map[k, v]` "is a map"; element types are the checker's
  business (empty lists match vacuously).
- **Type parameters are erased.** A bare `T` satisfies anything at runtime;
  the checker owns the real constraint.
- **Aliases resolve at parse time.** The runtime never sees the alias NAME:
  matching is the target's law. The `type` statement itself is inert (the
  VM bridges it to the tree-walk, which no-ops it).
- **Declare before use.** An annotation naming an alias before its
  declaration stays a plain `Named` — the typo-armor rule applies (unknown
  names match nothing, so the boundary surfaces the mistake loudly).
  Duplicate alias = rung-2 note, first declaration wins. A gene's type
  parameters shadow aliases in annotation position.

## 5. Static laws (the checker, stage 2)

- **Inference:** unannotated code infers by a flow-joined statement walk;
  branch joins union. The binary lattice mirrors `apply_binop` exactly
  (`/` → Float, `//` → Int, int**int → Int, `str + str` → Str). `Any` is
  the universal escape hatch: dynamic code inside a typed file checks
  clean, there are no false positives by design.
- **Generics:** call-site solving unifies actuals against formals; solved
  parameters substitute into the return type; declared bounds
  (`numeric`, `comparable`, trait names) are verified against the solution
  (T08).
- **Traits:** `implements` contracts are enforced statically — a phenotype
  missing a required trait method is T07 at the definition; trait names
  are annotation types (`let d: Drawable = ...`).
- **Exhaustiveness:** a match over Option/Result/Bool/union scrutinees must
  cover every case; `case _` and bind arms cover all; guarded arms cover
  nothing (the condition may be false) — T05.
- **Option/Result are structural:** `none()` is `Option(Any)` until
  unified, `ok(v)` is `Result(T, Any)` until an `err` pins `E`; `?!`
  propagation through non-variants is T06.
- **Assignability** (`ty_compat`) is the ONE subtyping law: exact match,
  widening int→float, union membership, Option/Result/List/Map structural
  recursion, Never bottom, Any top, phenotype derivation, trait
  satisfaction. Everything else is a mismatch (T01/T03).

## 6. Findings (T-codes, W041 scheme)

| code | rule | severity |
|---|---|---|
| T01 | type-mismatch | error |
| T02 | unknown-member | error |
| T03 | arg-type-mismatch | error |
| T04 | unknown-type (unknown annotation name) | warning |
| T05 | non-exhaustive-match | error |
| T06 | propagate-non-variant | warning |
| T07 | trait-method-missing | error |
| T08 | bound-violation | error |
| T09 | assign-type-change | warning |
| T10 | return-missing | warning |

CLI contract: `operon run --typed f.op` = compile gate before execution
(exit 3 on type errors, execution never starts); `operon check --typed
f.op` = T-series findings in the check stream (diag + `--json`); `--strict`
refuses type warnings.

## 7. Oracle parity (the gate that keeps it honest)

Every law in §4 is mirrored op-for-op in `bootstrap/oracle.py`
(`ann_matches`, `ann_is_typaram`, `ann_render`, the parser's alias table +
type-param stack, the inert `typealias` execution). The differential
harness holds both cores to byte-identical stdout AND notes on:

- `tests/type_anns.op` + `tests/differential/type_anns.op` (stage 1)
- `tests/type_generics.op` + `tests/differential/type_generics.op`
- `tests/type_aliases.op` + `tests/differential/type_aliases.op`
- `tests/typecheck.rs` (41 checker cases, Rust-side)
- `examples/typed/` (good-path corpus + `bad_*` negatives pinned by the CLI gate)

A type-system change that moves one core without the other is a bug by
definition; the harness is the arbiter.

## 8. Non-goals

No new runtime types, no runtime generic dispatch, no nominal
Option/Result wrappers, no reflection over annotations, no type-directed
optimization. If a proposal needs any of those, it is a new proposal —
this document does not authorize them.
