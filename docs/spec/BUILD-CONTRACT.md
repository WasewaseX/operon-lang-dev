# OPERON BUILD CONTRACT

W086 of the M100 program · v1.0.0 · 2026-09-26 · owner: sz (dev-3)
Baseline: main @ dd76caa · `operon build f.op [--variant v] [-o out.op]`

---

## 0. What `operon build` IS today (the honest contract)

`operon build` is a **source-to-source specialization** step. It produces a regular
`.op` source file — not bytecode, not IR, not a native executable. The audit's complaint
("the name suggests a compiler while the implementation is a source transformation") is
correct; this document is the fix in the meantime, W085/W087 own the future.

Pipeline, exactly as implemented (`src/main.rs`, `build` arm):

```
input source
  → (optional) apply .rna patch        genes::apply_rna(&src, &patch, &stem)
  → parse                              parser::parse
  → bake --variant (if given)          keep only the selected splice variant body per
                                       Splice statement; drop the others
  → strip proof frames                 all Stmt::Frame { is_proof: true } removed
  → canonical format                   tools::format_program (the fmt canonical form)
  → write                              <stem>.built.op  (or -o path)
```

## 1. Guarantees

1. **Semantics-preserving modulo specialization**: the baked program behaves identically
   to running the original with the same `--variant`, minus proof frames (which are
   verification scaffolding, not semantics) and minus unselected splice bodies (which the
   runtime could never take under that variant).
2. **Byte-stable**: same input bytes + same variant + same operon version ⇒ byte-identical
   output (DETERMINISM.md §6). No timestamps, no paths, no environment values are embedded.
3. **Canonical form**: output is formatter-canonical — feeding it back through
   `operon fmt` is a no-op.
4. **Standalone source**: the artifact needs nothing from the build step; it runs on any
   operon binary of a compatible version (compat policy: W63).

## 2. Non-guarantees (explicit)

1. **NOT a compiler**: no bytecode (W009), no IR (W040), no native code (W085 roadmap).
2. **No dependency bundling**: `use` statements are untouched; modules are resolved at
   run time exactly as in the unbaked program (resolution: W069).
3. **No minification promise**: canonical formatting may expand or shrink text; size is
   not a goal.
4. **`.rna` application is best-effort text-level patching today**: misses and multi-hits
   behave exactly as the runtime `.rna` surface behaves (W067/W068 cover the safety mode
   and the future AST-based model).

## 3. What it is FOR

1. **Shipping cleaner programs**: strip verification scaffolding (proof frames) before
   distributing an app or embedding an example in docs/playground.
2. **Variant snapshots**: bake a specific splice variant so downstream users do not need
   to pass `--variant`.
3. **Canonicalization as a build step**: normalize team code to the canonical form.
4. **Deterministic artifact for tests**: the byte-stable output makes build a fixture
   generator for round-trip tests.

## 4. Roadmap (W085/W087 — owned by sz, M100)

1. **Stage 1 — bytecode bundle (post-W009)**: `operon build --target=bytecode` emits the
   VM bundle; the tree-walk source remains the default target until the VM passes the full
   differential campaign (W009 gate).
2. **Stage 2 — true standalone executable**: embed the bundle into a released operon
   runtime binary (`operon build --target=exe`, Rust embed, no external toolchain needed
   by the end user). Native codegen only if W011 profiling demands it (audit ordering).
3. **Stage 3 — single-file bundle (W087)**: `operon bundle app.op` — source + bytecode +
   stdlib deps + metadata in one portable artifact; design note: `docs/design/BUNDLE.md`.
4. Until Stage 1 lands, `--target` is not accepted and this document is the contract.
