# WORKER-TELEMETRY, W098 design note (documentation half delivered; builtin deferred)

Status: the telemetry that EXISTS today is documented here and in SPEC §13/§14;
the `workers()` introspection builtin is deferred to a dev-1-lane session (it
adds a builtin to the interp + oracle mirror + differential coverage, a full
parity wave, not a docs edit).

## What exists today (pinned)

- **`promoter_telemetry(name)`** (loop-9 F-3) → `{attempts, on_total, episodes,
  on_frac, burst_size}` per gene: the telegraph promoter layer's attempt
  counters. `burst_size = on_total/episodes` is the mean ON-run length (~1 =
  Poisson-like firing, >>1 = bursting). Zero attempts → all-zero map. A degraded
  (RISC) call never reaches these counters, suppressed calls are not expression.
- **`fingerprint()`** (§14) → call counts, mature/nascent/maturation, burst
  indices, transcript counts, generation.
- **Spawn snapshot membrane** (§13): workers inherit a frozen regulation state
  copy; parent changes do not propagate. Worker calls dispatch through the same
  funnel, gates and silencing apply inside the cell identically.
- **join(id)** semantics: second join → null + note; default 300 s wait ceiling;
  timed-out tasks stay joinable.

## Deferred builtin sketch: `workers()`

- Returns a List of Maps, one per task created this run:
  `{id, gene, spawned_at_tick, state: running|finished|joined|timed_out,
  result_kind: value|stress|null|none, wall_ms, fuel_used}`.
- Implementation surface: task registry already exists (join needs it); the
  additions are lifecycle stamps at spawn/finish/join/timeout (4 sites, all in
  the spawn/join funnel), plus a registry drain at introspection time.
- Oracle parity: the oracle's threading layer mirrors spawn/join; lifecycle
  stamps must be added op-for-op, and a differential program pins the SHAPE
  (fields present, state transitions legal), timing values are NOT pinned
  (wall-clock, machine-dependent, DETERMINISM §5-adjacent honesty).
- Blocked-time + per-task fuel attribution: deferred with MEM-PROFILER's
  counting-allocator work (same substrate reason, the W009 VM changes frame
  layouts; build both once, after).
