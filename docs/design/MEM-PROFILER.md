# MEM-PROFILER — design sketch (W097-v2, deferred)

Status: design note only — no code in this document is implemented. The
`memory()` builtin's exact accounting is pinned in SPEC §10 (W097): the
symbol/intern table gauge (`arena_bytes`, `interns`, `allocs`). This note
describes what a REAL memory profiler would add, and why it is deferred.

## What exists today (pinned, W097)

- `memory()` → `{arena_bytes, interns, allocs}` — process-wide, mutex-guarded,
  monotonic except `arena_bytes` (live bytes rise and fall with interning).
- `:symbols` (REPL) → the canonical spellings, oldest first.
- Interp-level value storage is refcounted ordinary Rust allocation — there is
  no arena to sum; today's gauge intentionally does not pretend otherwise.

## Why a real profiler is deferred

1. **No arena to instrument.** Values are plain `Rc<RefCell<...>>`; accurate
   per-value accounting requires either a custom allocator (global — affects
   every allocation, including the parser's) or a size-estimation pass
   (recursive Value sizing — costs a full walk, changes what it measures).
2. **The VM changes the substrate (W009).** Per-frame/per-gene memory
   attribution built on the tree-walker would be invalidated by the bytecode
   VM's frame layout. Building it once, after the VM, is half the work.
3. **Demand is hypothetical** — no current M100 level requires per-gene
   allocation attribution; the redteam mem ceilings (rt_p2h) use RSS-style
   process caps, which are orthogonal.

## Design sketch (for W097-v2, post-W009)

- **Global counting allocator** (`#[global_allocator]` wrapper, feature-gated
  `mem-profile`): tracks live bytes + allocation counts per call-site bucket
  at coarse granularity (lexer / parser / interp-values / module-cache /
  regex-engine). Per-bucket, not per-value: cheap (thread-local counters),
  no value walks.
- **Attribution windows**: `memory(true)` starts a window; `memory(false)`
  closes it and returns the bucket deltas for the window. Windows compose
  with `profile` (self-time table gains a `bytes` column).
- **Per-gene attribution** (approximate, honest): allocations inside a gene
  call's dynamic extent are attributed to that gene (call funnel stamps the
  thread-local current-gene slot; spawned workers attribute to their task).
  Documented as attribution-by-extent, NOT ownership — a value returned from
  a gene keeps its allocation bucket after return.
- **Output**: `operon profile --json` gains a `memory` section
  (`{"buckets": {...}, "peak_window": {...}}`); Chrome-trace mem events ride
  W096's trace format when per-call spans land.

## Acceptance criteria (for the eventual level)

- Zero overhead when the feature gate is off (release builds unchanged).
- Bucket deltas are deterministic for a deterministic program (DETERMINISM.md
  §5 float policy does not apply; allocation order may vary across threads —
  document which buckets are order-sensitive).
- No false promises: the report says "attribution by extent", never ownership.
