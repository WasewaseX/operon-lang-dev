# NATIVE-FFI — design note (M100 W080)

owner: sz (dev-3) · status: deferred-by-design (after W077 C-ABI + W009 VM; py() and the
embedding API deliberately come first)
see also: `docs/design/C-ABI.md`, `THREAT-MODEL.md`, `docs/EMBEDDING.md`,
`docs/spec/BIO-CONTRACT.md`, TODO-100 W081 (kernel boundary standing rule).

## 1. What "native FFI" means here (and what it does not)

Goal: an Operon program can call a **native shared library** (`libfoo.so`/`.dylib`/`.dll`)
exporting C-ABI functions, declared in-source. This is the classic "escape hatch to
native speed / existing ecosystem" found in mainstream languages (Python `ctypes`,
LuaJIT FFI, Julia `ccall`, Rust `extern`).

NOT in scope, ever, for this layer: loading Operon *into* other processes (that is the
reverse direction — W076/W077), and moving biological mechanisms into the C++ kernel
(W081 standing rule: no bio mechanism into the kernel without a ≥2x benchmark proving it
AND a security re-audit — the kernel stays a load-bearing arithmetic/interning engine).

## 2. Proposed grammar sketch (Total-Grammar compliant)

```
foreign "c" gene erf(x: float) -> float
    -- declaration only; resolved at call time against the granted library

foreign "c" lib "libstat" gene churn(n: int, seed: int) -> float
    -- library-qualified form when symbols collide across granted libraries
```

- **Declaration is grammar; execution is capability.** Both forms parse everywhere (the
  Total Grammar accepts them at the canonical rung); a `foreign` call without the
  matching capability grant raises contained Stress `interference` at the fence (C-7
  preserved: a denied call is data — catchable — never a process kill).
- Doc comments (`##`, W074) attach to `foreign` genes like any other; `operon doc` (W073)
  renders them with a `foreign` mark so generated docs never disguise native calls as
  pure Operon.

## 3. Capability gate

- New caps field `ffi: Vec<String>` alongside read/write/run/net/env/py (Caps is already
  the one struct, THREAT-MODEL §defense-inventory). Grants are **exact-match per library
  file name** (`--allow-ffi libstat`) — the same per-module trust shape as `py()` grants
  (substrate-r1), NOT all-or-nothing `dlopen`-anything.
- CLI: `--allow-ffi <lib>` (repeatable). `.cell`: stays runtime-config-only (W22/W66 —
  capability grants are CLI/runtime posture, not package metadata).
- Default state: OFF. Zero grants ⇒ every `foreign` call is a contained, catchable
  Stress, byte-identical behavior with or without the declaration present in the source
  (mirrors the W078 py()-off parity contract).

## 4. Marshalling (v1 — deliberately boring)

| Operon type      | C side                        | Notes |
|------------------|-------------------------------|-------|
| int              | `int64_t`                     | single integer width, like the interp |
| float            | `double`                      | W090 determinism note applies past the boundary too |
| bool             | `uint8_t` (0/1)               | |
| string           | `const char*` + `int64_t` len | copy-in/copy-out; callee never retains |
| list of numbers  | caller-allocated `T*` + len   | copy-out on return; no borrowed pointers in v1 |
| struct/map       | NOT SUPPORTED v1              | flatten to positional args; `#[repr(C)]`-style struct marshalling is v2 (needs the type system, W001) |

- **No host callbacks in v1.** A C function that calls back into the interpreter requires
  re-entrant handles + the W004 calling convention; sequencing it after traits lands.
- **No pointer types visible to Operon programs** — the language never sees an address,
  only values. This keeps the capability story auditable: a grant authorizes *symbol calls
  into one named library*, not memory access.
- Symbol resolution: `dlopen`/`LoadLibraryW` at first granted call, per-run cache,
  `dlclose` at teardown; resolution failure = contained Stress `missing` with the
  attempted-name detail shaped like W070 import diagnostics.

## 5. Why this is LAST in the FFI sequence (threat reasoning)

1. **py() first (already landed, substrate-r1):** subprocess isolation — a Python child
   crash is contained by the OS process boundary. Its grant model (per-module, default
   off, catchable denial) is the trust vocabulary FFI reuses.
2. **C-ABI of the core next (W077):** makes operon *callable* — still the host's choice,
   host's process, host's risk.
3. **Native FFI (this note):** makes *unvetted machine code* callable FROM the sandboxed
   interpreter. In-process, there is no isolation: a segfault kills the host, a heap
   corruption corrupts it. That is only acceptable after:
   - W009 VM provides a hard execution boundary (fuel-charged interpret loop, no unwind
     through embedder frames) so containment claims stay honest;
   - THREAT-MODEL.md gains the `ffi` caps row with evidence payloads (redteam:
     garbage-library grant, symbol-confusion, arg-count mismatch, oversize return
     buffer, non-UTF-8 returns);
   - an isolation alternative is evaluated honestly (wasm callee as the sandboxed
     long-term path — native FFI stays "trusted first-party libraries only" until then).
4. Resource charges: every `foreign` call burns fuel (W009 pricing table TBD) — a native
   call must never be a way to buy unlimited host time for one unit of Operon fuel.

## 6. Implementation order (when unblocked)

1. `Caps.ffi` + fence + redteam payloads (can land independently, default-deny, zero
   behavior change for grant-less programs — mirrors how `py` landed behind `--allow-py`).
2. Lexer/parser/AST arm for `foreign` declarations + SPEC §grammar row + oracle mirror +
   differential pins (declaration-only, no execution).
3. Marshalling layer (§4 table) + symbol resolution + W070-shaped diagnostics.
4. `examples/ffi/` + CI smoke on Linux only (cgo-free tiny `libstat.so` built in-job).
5. Docs: README capability table row, SPEC §caps row, GenomeLab unaffected (bio layer
   consumes stdlib only — W081 boundary).
