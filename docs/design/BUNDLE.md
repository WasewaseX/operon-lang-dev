# BUNDLE — the `operon bundle` single-file artifact (W087, design note)

Status: design. Implementation is gated on W009 (bytecode VM) for the
bytecode payload and W023 (package manager) for the dependency-closure
semantics; the ARTIFACT LAYOUT and LOADER CONTRACT are pinned here so the
W009/W023 waves build against a fixed target. Preceded by `operon build`
(source bake — shipped) and `operon build --target=bytecode` (stage 1,
post-W009). Contract parent: `docs/spec/BUILD-CONTRACT.md` §4 stage 3.

## Why a bundle (and why not a zip of the project)

A bundle answers ONE operational need: hand a non-author a single file that
runs with the toolchain they already have — no project tree, no module
search path, no `.cell` archaeology, no "works from the repo root only".
It is the distribution unit between "a loose `.op` file" (too fragile for
multi-module apps) and "a native executable" (stage 2, heavier). The
content-addressed stage of W023 (registry install) then uploads/downloads
the same artifact, so `bundle` is also the registry's unit of storage.

## Artifact layout

A bundle is a single `.opb` file: a small deterministic text envelope with
embedded, clearly-fenced payload sections. Text envelope (not binary zip)
keeps the artifact diff-able, grep-able, and byte-stable under the same
normalization rules as `operon build` (W088 — path/timestamp exclusion is
by construction: the envelope carries no paths and no timestamps).

```
operon-bundle v1
meta {
  format: 1
  operon: 2.2.0            # toolchain version that produced the bundle
  name: myapp              # from operon.toml [package] when present
  version: 0.1.0           # ditto
  entry: main              # resolved entry gene
  source_hash: sha256-...  # of the entry source AS EMBEDDED
}
cell {
  # runtime-config keys the AUTHOR ships (behavior defaults ONLY — see
  # Security: never allow.* keys; the loader refuses them)
}
modules {
  module seq/seq.op        # use-path, closure order (topological)
  --BEGIN--
  <canonical source of the module>
  --END--
  module main.op
  --BEGIN--
  <canonical source of the entry file>
  --END--
}
```

- **Source is the v1 payload.** Bytecode (W009) rides behind `payload: bytecode`
  later; until the VM passes the full differential campaign, shipping source
  is the honest choice (the tree-walk interpreter is the only executor).
- **Module closure** comes from the W069 resolution algorithm: every `use`
  path in the entry, transitively, resolved against the standard roots at
  BUNDLE time and embedded with their `use`-path key. `std/*` modules are
  NOT embedded — the loader resolves them against the RUNNING toolchain's
  std (a bundle pins the toolchain MINOR, not a std snapshot; mismatch is a
  loud loader error, not a silent drift).
- **Closure order** is topological by first-import; cycles are refused at
  bundle time (the loader's cyclic-import placeholder semantics are for
  live runs, not for packaging — a cyclic bundle is a design smell caught
  early).
- **`.rna` patches are not applied in bundles** — a bundle is a snapshot;
  hot patches apply to source projects. (A future `bundle --rna` bakes the
  patch into the embedded source via the W067 v2 engine before sealing.)
- **Metadata source of truth**: `operon.toml` (W019/W022) when present;
  `--name`/`--version` CLI overrides for bundles outside a project. `.cell`
  contributes behavior defaults to the `cell` section under the W66
  package-keys rule (package-like keys are refused by the bundler, matching
  the W48 lint).

## Loader contract (changes live in `tools::load_file` — dev-2 lane)

1. Extension `.opb` → parse the envelope header (first line MUST be
   `operon-bundle v1`; anything else = refuse with a version hint).
2. Verify `source_hash` per module (refuse on mismatch — truncated or
   hand-edited bundles fail loudly, never partially).
3. Materialize modules into the interp's module cache under their `use`
   path keys BEFORE the entry executes — `use` statements then resolve
   from the cache without touching the filesystem (module-search rules of
   W069 are bypassed, not violated: the bundle IS its own search root).
4. `cell` section loads as an EXPLICIT cell (the operator handed over the
   file) — but with the same allow.* refusal as auto-detected cells:
   capability grants come from the operator's CLI/--cell at RUN time, never
   from inside an artifact. A bundle with `allow.*` keys is refused at
   bundle time already; the loader refusal is the belt to the bundler's
   suspenders.

## Determinism (W088 §builds contract)

`operon bundle app.op` twice, same toolchain + same inputs → byte-identical
`.opb`. Achieved by: canonical source (the W047 formatter runs on every
embedded module), no timestamps, no absolute paths, stable envelope field
order, closure order from the deterministic W069 algorithm. The existing
build-normalization test extends to bundles (`scripts/` — build twice,
byte-compare) once the format lands.

## CLI surface (v1)

```
operon bundle app.op [-o myapp.opb] [--name n] [--version v]
operon bundle --check myapp.opb     # header + hashes + closure lint, no run
operon run myapp.opb [--allow-*]    # loader path above; grants via CLI only
```

## Why this waits for W009/W023 (and what does NOT wait)

- **W009 (bytecode VM)**: the `payload: bytecode` section is only real once
  a VM executes it and the differential campaign passes on the VM — the
  W009 gate. Source payload is shippable before that.
- **W023 (package manager)**: dependency closure for EXTERNAL packages
  (registry deps in `operon.toml`) needs the registry's naming/resolution
  contract. v1 bundles only close FIRST-PARTY modules (the app's own tree
  + std) — external deps are refused with a clear message until W023
  defines `deps` resolution.
- The layout, hashing, cell rules, and loader contract do NOT wait — they
  are implementable against the current tree-walk interpreter, and the
  W009/W023 waves plug payloads and deps into this envelope.
