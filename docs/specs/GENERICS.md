# GENERICS staged plan (W03, ROADMAP-100)

Normative for how Operon grows generic programming. Conflicts resolve
toward SPEC.md, then this file. Status: **stage 1 is the shipped reality;
stages 2–3 are specified and deliberately unscheduled**, stdlib growth
must never block on them (the W03 decision).

## What "generic" means here

Operon genes are first-class values and containers are untyped, so the
language is already generic in the way Go 1 maps and Python callables
are: **any function that only uses the operations its body performs
works for every element type that supports them.** What Operon lacked,
and what this plan stages, is (a) proof that the std APIs really are
callable-generic with zero duplication, (b) documentation-only type
annotations so signatures can say `map<T, U>(f, l)`, and (c) a real
parametric layer that never lies about being checked.

## Stage 1, duck typing + callable-generic std (LANDED, this document
is part of the W03 gate)

The contract the stdlib already upholds, now pinned as policy:

- **Std APIs accept callables as plain gene values.** `iter.map`,
  `iter.filter`, `iter.fold`, `iter.scan`, `iter.take_while`,
  `iter.drop_while`, `iter.find_first`, `iter.sort_by_key`,
  `collections.group_by`, `heap` comparators (`gene (a, b) => a < b`),
  and `seq` combinators all take a gene where mainstream libraries take
  a function object. One implementation serves every element type,
  there is no `map_int`/`map_str` duplication and none may be added.
- **Duck typing is the semantic contract.** A callable is generic over
  exactly the operations it performs. `iter.map(nums, gene (x) => x * 2)`
  and `iter.map(strs, gene (s) => len(s))` share one `map`. The proof
  corpus is `tests/std_generics.op`; the differential pin is
  `tests/differential/generics.op` (byte-identical on both engines).
- **Total Grammar applies.** A callable that performs an operation its
  argument does not support surfaces the mismatch by tier, exactly as
  the SPEC's error hierarchy demands: a missing method is the SOFT tier
  (null + note, the note names the missing method), while a real type
  mismatch (`1 + "s"`) is a catchable `unfolded` stress at the use site.
  Nothing is ever a parse rejection, and there is no template
  instantiation error, the same containment shape as every other
  runtime mismatch.
- **Phenotypes are NOT generics.** A phenotype is a record with methods
  (SPEC §8); it does not abstract over its field types and must never be
  presented as a generic mechanism (the W03 audit finding, restated).

## Stage 2, documentation-only type parameters (specified; rides W01
stage-2 annotations)

`gene map<T, U>(f, l)` parses; the annotations are checked by the same
L2c soft-contract machinery as `gene f(x: int) -> int`, catchable
`unfolded` stress on violation, never a parse rejection, and the
`<T, U>` spellings are PURELY descriptive (they name slots for readers
and the LSP hover; they do not monomorphize, they do not reject calls
that would work). Rules, fixed now so code written against stage 2
survives stage 3:

1. Type parameters may appear anywhere a type annotation is legal today
   (params, return, `let`).
2. A type parameter matches by the SAME rules as `type_name()` equality
  , a call-site value whose `type()` spelling equals the parameter's
   binding at that call satisfies the annotation; `any` accepts all.
3. Signatures stay documentation: a violation is a catchable stress
   raised by the existing annotation checker, identical in kind and
   containment to W01 stage 1. No new execution semantics.
4. The fmt round-trip (W47) preserves parameter lists byte-exactly.

## Stage 3, real parametric polymorphism (specified; deliberately
unscheduled)

Only worth building if a concrete duplication burden materializes in std
(motivated use-case: a typed `Set<T>` that needs per-type hash/equality
dispatch). Shape, so nobody re-derives it under pressure:

- **Monomorphization at module load**: each distinct call-site type
  instantiation compiles one specialization of the gene body; the cache
  key is the ordered tuple of `type()` spellings of the type arguments.
- **Explicit instantiation only**: `map<int, int>(f, l)`, no inference
  (inference over duck-typed bodies is unbounded, and unbounded magic is
  the failure mode this plan exists to avoid).
- **Trait bounds ride W04**: `gene sort<T: Ordered>(l)` requires the
  trait's required methods present at instantiation; the check is the
  W04 construction-time contract check, reused unchanged.
- **Both engines, always**: the oracle implements the identical cache and
  identical failure notes, or the feature does not land (the differential
  rule is absolute).
- **Entropy discipline**: instantiation order must never reorder random
  draws (the loop-10 rule); monomorphization happens at load, before any
  statement executes.

## What this file forbids

- Adding per-type duplicated std functions "because generics are not
  ready", stage 1 makes them unnecessary (W03 gate).
- Presenting stage-2 annotations as compile-time checking in any user-
  facing doc, they are contracts with the same catchable-stress
  semantics as every W01 annotation.
- Pulling stage 3 forward without a DECISIONS entry naming the concrete
  std duplication it removes.
