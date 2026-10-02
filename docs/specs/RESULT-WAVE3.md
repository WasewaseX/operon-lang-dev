# RESULT-WAVE3 — W006 stage 2 wave 3: candidates, contracts, and the
# Builder-A core-dependency report.

Status: **PREPARED (stdlib side complete; core runtime changes PENDING)**.
Owner lane: Builder-B (stdlib/ecosystem). Task: W006-A.
Baseline: main @ 44c822d, v2.7.0. Waves 1-2 shipped 10 try_* builtins
(PR #43, merge c4d258f); this doc prepares the REMAINder recorded in
TODO-100 W006: "further families (IO read_file stress-class, sqrt domain)
behind per-function compat notes".

## 1. Audit method

Every builtin failure mode was extracted mechanically from the oracle's
builtin dispatch (`scripts/w006a_audit.py`, Builder-B session tooling:
walks `if name == "..."` arms, classifies `raise Stress(kind, ...)` and
note+null returns). 91 of ~150 builtins have observable failure modes; the
language-level operator stresses (indexing, arithmetic, ordering —
`oracle.py` eval paths, NOT builtins) are grammar semantics and are out of
scope for every wave.

## 2. Classification

### 2a. Already migrated (waves 1-2, no further work)
`try_num` `try_index` `try_get` `try_pop` `try_first` `try_last`
`try_char_at` `try_env` `try_json_parse` `try_re_groups`. Their remaining
`unfolded` stresses are ARITY errors on malformed calls — by contract these
stay stresses (a malformed call is not an expected failure).

### 2b. Wave-3 numeric family — STDLIB SIDE SHIPPED in this task
`std/try_numeric.op` delivers `try_sqrt` / `try_floor` / `try_ceil` without
a core change (validate-arg-then-rescue; Err payloads are fixed strings, so
the engine-neutral payload law holds by construction). Pins:
`tests/std_try_numeric.op` (30 asserts, both engines) +
`tests/differential/try_numeric_std.op` (byte-identical 3-lane).
Legacy stresses stay pinned UNCHANGED in the same proof frame.

| wrapper | ok side | Err payload (fixed) |
|---|---|---|
| `try_sqrt(x)` | `sqrt(x)` exact value | `"sqrt of negative number"` |
| `try_floor(x)` | `floor(x)` exact int | `"float too large for floor/ceil to int"` |
| `try_ceil(x)` | `ceil(x)` exact int | `"float too large for floor/ceil to int"` |
| type abuse (any of the three) | — | `"try_<name> expects a number, got <type>"` |

### 2c. Wave-3 IO family — CORE DEPENDENCY (Builder-A), NOT stdlib-side
The IO builtins fail with `missing`-class stresses whose payload embeds the
OS error text (`"read_file '<path>': {e}"`) — engine-specific by nature.
A stdlib rescue wrapper CANNOT deliver the engine-neutral payload law here
without swallowing capability denials: the same rescue that catches
`missing` catches `interference` (containment), and dynamic re-raise is NOT
usable (finding F2 below). Therefore the IO family requires NATIVE core
arms, exactly like waves 1-2:

| required builtin | ok side | Err payload (engine-neutral, fixed) | stays Stress |
|---|---|---|---|
| `try_read_file(path)` | file bytes/str | `"read_file '<path>' failed"` | interference (containment) |
| `try_read_file_bytes(path)` | bytes | `"read_file_bytes '<path>' failed"` | interference |
| `try_read_dir(path)` | entry list | `"read_dir '<path>' failed"` | interference |
| `try_run(prog)` | exit/streams | `"run '<prog>' failed"` | interference |
| `try_http_get(host, port, path)` | body | `"http_get failed"` | interference |
| `try_str_from_bytes(b)` | str | `"str_from_bytes: invalid UTF-8 at byte N"` (N deterministic both engines) | — |

Second tier (writes/mutations — same shape, lower priority):
`try_write_file` `try_append_file` `try_write_file_bytes` `try_fs_delete`
`try_fs_rename` `try_fs_mkdir` (all: `"<name> '<path>' failed"`, interference
stays Stress). Builder-A notes: mirror in `bootstrap/oracle.py` builtin()
byte-for-byte (stdout-visible payloads), add each to the BUILTINS registry
in BOTH engines, granted-lane any capability-dependent pin, and keep the
legacy builtins byte-identical (additions, not replacements).

### 2d. Keep-as-stress (deliberate, documented — do NOT migrate)
- `assert` -> `burned` (test machinery IS the failure surface).
- Capability denials -> `interference` (membrane containment law, SPEC §7).
- `send` on closed channel -> `closed_channel`; `send` depth -> `overflow`;
  `select` with no buffered receive in the oracle -> `blocked` (channel
  contracts, W016/W018 lanes).
- `unwrap` on Err/plain -> `unwrap` (the documented panic surface; the
  payload reader is `match`, not unwrap).
- `abs(i64::MIN)` -> `overflow`: i64::MIN is UNREACHABLE from source (the
  literal overflows the parser, E2008, and every arithmetic path to it
  overflows first) — `try_abs` would have a dead error path; skip.
- `weak`/`strengthen`/bytes-family arg-type abuse -> `unfolded`
  (programming errors, not expected failures).
- The null+note lifecycle family (`task_state`/`cancel`/`join`/
  `wait_all`/`wait_any` on unknown/finished ids) and the null+note
  collection helpers (`zip`/`chunk`/`take`/`drop`/`unique`/`flatten`/
  `sorted`/`reversed`/`enumerate`/`clamp`/`round` on wrong-shaped args):
  null+note is the PINNED legacy contract; a wave-4 candidate list may
  propose `try_*` twins, but nothing in this wave touches them.

## 3. Findings filed to other lanes (from this audit, NOT fixed here)

- **F1 (core lane): oracle crashes on sqrt type abuse.** `sqrt("x")` is a
  contained `unfolded` stress on the Rust VM and the tree-walk
  ("numeric op needs numbers, found str and int") but a raw Python
  `ValueError` (rc=1) in the oracle (`float(args[0])` unguarded,
  oracle.py:6871). The differential corpus never feeds sqrt a non-number,
  so parity never observed it. Fix is one guard in the oracle's sqrt arm;
  the corpus avoidance then stops being load-bearing.
- **F2 (language lane): dynamic re-raise collapses the message.**
  `raise e.kind, e.message` yields `kind=unfolded, message=unfolded`
  IDENTICALLY on all three engines: the raise grammar binds its name slot
  from an IDENT token, so a member-expression first argument silently
  degrades (the literal form `raise "kind", "msg"` works). Either the
  grammar should accept expression kinds with fixed semantics, or the
  degraded form should be a documented note. All three engines agree today,
  so no differential hazard — but the semantics are surprising.
- **F3 (type lane): Result annotation sugar.** `ok`/`err` values are
  plain variants to the checker; `result[T, E]`-style annotations and
  Result-aware inference are recorded as the honest open degrade that made
  std/result.op ship with bare (erased) parameters.

## 4. Handoff summary

- Stdlib side: COMPLETE (std/result.op, std/try_numeric.op + 4 pin files).
- Builder-A dependencies: §2c native arms + oracle mirrors (IO family),
  optionally native `try_sqrt`/`try_floor`/`try_ceil` arms to retire the
  rescue-based wrappers — the differential pins already enforce
  byte-identity, so a native swap is gate-protected.
- Core-lane bug: F1. Language-lane quirk: F2. Type-lane degrade: F3.
- Nothing in this task modified core runtime files.
