# C-ABI — design note (M100 W077)

owner: sz (dev-3) · status: deferred-by-design (implement after W076 settles + W009 VM)
see also: `docs/EMBEDDING.md` (Rust embedding, landed), `THREAT-MODEL.md`,
`docs/design/BUNDLE.md`, `docs/spec/BUILD-CONTRACT.md` §4.

## 1. Why a C ABI at all

The Rust embedding API (W076) covers in-process hosts that speak Rust. A C ABI makes the
same capability reachable from every FFI-capable ecosystem — Python (cffi/ctypes), Node
(N-API/node-ffi), Go (cgo), Java (JNI), C/C++ directly, and every language that can bind
a C symbol — without each one depending on `cxx`/`cbindgen` Rust-toolchain knowledge.
It is the layer that turns "operon is a Rust crate" into "operon is a library, full stop".

## 2. Why NOT yet (sequencing, deliberately)

1. **The Rust surface is not semver-hardened.** W076's own guide pins an exact version
   and defers the semver pledge until the type system (W001) and traits (W004) settle.
   A C ABI is a harder commitment than the Rust API — C symbols, once shipped, are
   effectively forever (every downstream binding bakes them in). Freezing C symbols on
   top of a moving Rust surface guarantees breakage or permanent shims.
2. **Host builtins are not public yet.** The most valuable C-ABI use case — registering a
   native function as an Operon builtin — needs the `Native` value calling convention to
   be public and stable (W076 follow-up, post-W004). Exposing `operon_register_builtin`
   before the convention is frozen would mint a compatibility debt no deprecation ladder
   can pay down.
3. **Crash containment needs the VM.** An in-process C-ABI consumer shares the process
   with the interpreter. Today a fuel/mem exhaustion is contained (Stress), but a kernel
   or interp panic unwinds through `catch_unwind` only if we draw that boundary
   deliberately; a native segfault kills the host. The bytecode VM (W009) is where
   execution gets a hard restart boundary (interpret loop, no unwinding through the
   embedder). W077 lands with/after W009, not before.

## 3. Proposed symbol surface (v1 sketch)

```c
/* operon.h — sketch, not frozen. All functions are thread-local to the
   interpreter handle; no global mutable state. */

/* Opaque handle: one interpreter + loaded program. NOT thread-safe —
   one handle per thread (Interp is !Sync today; see §5). */
typedef struct operon_handle operon_handle;

/* Diagnostics: repair/semantic notes from the last operation. */
typedef struct {
    uint32_t line;
    uint8_t  rung;      /* 1 canonical, 2 synonym, 3 wobble, 4 fallback */
    uint8_t  _pad[3];
    const char* message; /* UTF-8, NUL-terminated, borrowed until next call/free */
} operon_note;

/* Run a source string. Returns NULL on containment failure (check
   operon_last_status); returned Value is a UTF-8 rendering of the result
   (display-channel form, identical to CLI promote semantics). */
const char* operon_run_source(
    operon_handle* h,
    const char* source,        /* UTF-8 Operon source */
    const operon_caps* caps,   /* NULL = default-deny (the ONLY safe default) */
    const char* entry          /* NULL = .cell/main discovery, CLI-equivalent */
);

/* Lifecycle */
operon_handle* operon_new(const operon_caps* caps);
void           operon_free(operon_handle* h);
void           operon_string_free(const char* p);   /* frees returned buffers */
int32_t        operon_last_status(const operon_handle* h); /* 0 ok, else Stress kind code */
const operon_note* operon_notes(const operon_handle* h, size_t* count);
const char*    operon_version(void);                /* semver, static */
```

Non-goals for v1 (explicit): no `Value` introspection beyond the rendered string
(structured value access waits for the Rust surface freeze), no builtin registration
(post-W004), no streaming/two-phase compile+run (post-W009 bytecode), no Windows
`wchar_t` surface (source is UTF-8; callers convert).

## 4. Ownership rules (the whole contract in four lines)

1. **Handles** are created by `operon_new` and destroyed by `operon_free` — exactly once,
   from any thread (interior `Arc` teardown is the only cross-thread step).
2. **Strings returned to the caller** are owned by the handle until the next call on that
   handle OR until `operon_string_free` — documented per-symbol; borrowed pointers are
   never invalidated mid-call.
3. **Strings passed in** are borrowed for the duration of the call only.
4. **No panics across the boundary**: every exported fn wraps its body in
   `catch_unwind` → `operon_last_status` codes; abort-on-panic is the release profile
   decision to revisit at implementation time (W009 dependency, §2.3).

## 5. Capability + threat-model deltas

- `operon_caps` mirrors the Rust `Caps` struct 1:1 (read/write/run/net/env/py grants +
  enabled flag). **NULL means default-deny** — the C ABI must make the safe choice the
  lazy choice (THREAT-MODEL.md's "no embedder backdoor" rule applies verbatim).
- New attack surface vs W076: a C caller is exactly as trusted as a Rust embedder (same
  process, same privileges) — no new sandbox boundary is claimed. `operon_run_source`
  inherits every containment guarantee and every residual risk listed in THREAT-MODEL.md;
  the C note adds: untrusted callers must not be hosts (a host that links this library
  and runs untrusted source with expanded caps owns the consequences — same as Rust).
- rt-payload plan: extend `tests/redteam/` with an FFI-shaped smoke once the ABI exists
  (garbage source, missing NUL discipline via length-bounded copy-in, double-free of
  returned strings under ASan).

## 6. Implementation order (when unblocked)

1. `operon-sys`-shaped C header + cbindgen config checked into `include/operon.h`.
2. Export the four lifecycle symbols over the existing `tools::` layer (§ W076 guide).
3. ASan/UBSan smoke mirroring `tests/smoke_codon.cpp` (`tests/smoke_cabi.c`).
4. One FFI-consumer example (`examples/embed-ffi/`, cffi or ctypes) + CI step.
5. Semver pledge: symbols carry `#[no_mangle] pub extern "C"` + a
   `docs/specs/COMPATIBILITY.md` (D-011) section declaring the frozen set.
