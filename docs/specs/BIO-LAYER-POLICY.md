# Biological layer policy, the syntax freeze (W36 + W81)

**Audience lock (D-008):** Operon is for CS engineers who know zero biology. Gene vocabulary is
flavor, not prerequisite. This policy exists so the metaphor stays an asset instead of becoming
a growth path for parser keywords.

## Rule 1, the core keyword surface is FROZEN

The reserved keyword set (generated: [docs/KEYWORDS.md](../KEYWORDS.md), source
`src/parser.rs::KEYWORDS`) does not grow except by the escape hatch in Rule 4. Today's biology
syntax (`regulate`, `splice`, `operon`, `repressilator`, `ligand`, `quorum`-family, …) is the
FINAL grammar-level biology surface.

## Rule 2, new mechanisms land as libraries, not keywords

A future mechanism (epigenetics, stochastic kinetics, population models, …) ships in one of
three places, in preference order:

1. **`std/*.op` library** (pure Operon, proofs + STDLIB.md rows), default.
2. **`.cell` configuration** on an existing engine capability, when tuning, not syntax, is
   the interface (precedent: `rho.termination`, `quorum.dilution`, `methylate.threshold`).
3. **Builtins**, only with a DECISIONS entry proving a library cannot express it and the
   oracle mirror is specified (lane rule: no new Rust builtins otherwise).

## Rule 3, library-eligible inventory

Existing surface classified (loop-9/10 state): grammar-level = frozen per Rule 1;
builtin/config-level = extendable under Rule 2; already-library = `std/motifs.op`,
`std/bio.op`, `std/random.op`, `std/testing.op`. Candidates for library migration keep their
back-compat aliases until the COMPATIBILITY.md window closes them.

## Rule 4, the escape hatch

A grammar-level biology addition requires ALL of: a DECISIONS entry with the owner's sign-off,
a programmer-first analogy (D-008), differential tests + oracle mirror in the same PR, and a
statement of why Rules 1–3 are impossible for it. No exceptions in silence.

## Rule 5, the C++ codon kernel boundary (W81)

No biological semantics ever enter `runtime/codon_kernel.cpp`. Kernel additions require a
benchmark demonstrating ≥2× over the Rust path and a DECISIONS entry. The kernel stays tiny,
pointer-free, budget-guarded, ASan-smoked.

## Language vs modeling separation (points at W91/W92, sz lane)

Biological mechanisms have TWO contracts: what the PROGRAM does (SPEC language sections,
testable, differential) and what it MODELS (biology analogy, approximation with an honest
label). The modeling track's labels and jury evidence live in `project-vault/collab/audits/`;
a biology-analogy change must never silently become a language change.
