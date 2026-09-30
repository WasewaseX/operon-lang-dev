# ASYNC, the async model spec (W16, ROADMAP-100)

Status: **implemented on `builder/w016-async`** (W016 stage 2; the dynamic
side is untouched — every law below is opt-in behind `.cell io.pool` +
`--vm`, and without them `spawn` is byte-for-byte the thread path it always
was). Conflicts resolve toward SPEC.md and docs/vm-design.md.

## Why async waits for the VM (the honest dependency)

Async without a runtime that owns frames is a lie: today's `spawn` runs
on real OS threads and every `sleep` blocks a thread, and that is
documented honestly. Green-thread async requires the scheduler to own
execution state, pause a gene mid-body, park its frame, resume it
later. The tree-walk interpreter cannot do that (state is the Rust call
stack); the bytecode VM (W09) can: a frame is already a first-class
struct with slots, a pc, and a fuel account. So A-track order is:
A2/A3 land the frame model → this spec's fields exist for real.

## The model: green threads over the VM loop

- **Execution units are fiber frames, not OS threads.** `spawn` keeps its
  signature and semantics (value snapshot membrane, join-before-read);
  under `.cell io.pool` the spawned task may run as a fiber on the VM
  loop instead of a thread. Callers cannot tell, determinism and the
  snapshot membrane are identical by test, not by promise.
- **Suspension points are explicit**: `await`-shaped builtins
  (`sleep`, `chan.recv`, `fs` calls, `py()`) yield the frame to the
  scheduler at the same fuel-tick boundaries that already exist. No
  preemptive context switches ever, the seeded-determinism story (§22)
  and entropy-order discipline (loop-10 rule) are non-negotiable.
- **The scheduler is a plain queue, not magic**: run-to-completion until
  a suspension point; FIFO wake-ups; wake order is pinned by the
  differential harness like any other observable.

## Frame fields the VM reserves (A2 obligation, docs/vm-design.md §frames)

Every OIR frame carries, from A2 onward (unused until W16 lands, cost =
one enum tag + one u32):

| field | purpose |
|---|---|
| `fiber_state: Running \| SuspendedOn(WakeReason)` | parked at builtin calls, never mid-instruction |
| `wake_deadline: Option<u32>` | timer wheel slot for `sleep` / timeout receipts |
| `join_parent: Option<FrameId>` | structured-concurrency scope membership (W17 rides the same fields) |
| `cancel_flag: bool` | W18 cooperative cancellation, checked at the same tick boundaries |

## Capability and safety posture (unchanged by async)

- **Default-deny stays.** A fiber has exactly the capabilities its
  spawning cell had; `.cell io.pool` is itself opt-in.
- **Fuel is per-run, shared across fibers**, a fiber cannot buy more
  steps than the run had. Suspension does not refund fuel.
- **Containment notes name the fiber.** An uncaught stress in a fiber
  renders with the same `[tag] file:line` shape plus the task id (W18's
  task_state vocabulary), so debugging stays mainstream.

## Explicitly out of scope (permanently unless superseded)

- Colored functions (async fn / sync fn split), callables do not
  change signature by suspension behavior; the builtin boundary is the
  only suspension point.
- M:N work-stealing, locks, atomics, message passing only (W15's
  channels are the coordination primitive; D-005/D-013 principles
  apply: choose the deterministic, testable mechanism).
- Real preemption, it would break byte-parity differential testing.

## 9. The W016 implementation contract (what landed)

**The substrate.** `spawn` keeps its signature and its prep byte-for-byte
(snapshot membrane, regulation freeze, capability inheritance, shared
run-wide fuel pool, live medium, derived RNG stream, cancel chain — the
SAME code runs before the lane fork). Under `.cell io.pool` + `--vm` the
prepared task becomes a **fiber**: a parked VM frame chain plus a worker
`Interp` running on the host thread's scheduler loop. Without that cell
the task is an OS thread, exactly as before. `src/asyncrt.rs` owns the
scheduler; `src/vm.rs` owns the resumable `FrameState`.

**Suspension points are explicit and narrow.** A fiber parks ONLY at a
native `CallNamed` position calling an await-shaped builtin — `sleep`,
`recv`, `select` — through the hook at the top of `call_builtin`. A park
requested from a BRIDGED position (inside `stress {}` windows, `match`
guards, comprehensions — anything the VM delegated to the tree-walk) is
REFUSED and the builtin takes its existing blocking path: the Rust call
stack there cannot rewind, and the blocking shape is today's
byte-parity-tested behavior. The park unwinds as an internal marker stress
whose kind is NUL-prefixed (`\u{0}fiber-park`) — unspeakable by any source
program — and every containment site (rescue arms, pattern/guard
containment, interpolation, the gene chain builder) explicitly lets it
pass, so a user `stress {} rescue any` can never observe, contain, or
spoof it.

**The scheduler is a plain queue.** One scheduler per run, pumped only from
the host thread (host blocking ops + joins). Ready order is FIFO;
run-to-completion until a suspension point; the wake scan processes parked
fibers in registration order; same-deadline sleeps wake FIFO. A running
fiber segment is taken OUT of the scheduler map while it executes (the
take-run-return protocol), so nested spawns/joins/parks re-enter freely.
Wake order for equal deadlines is deterministic; `--repeat` on the proof
runner pins byte-identical results
(tests/async/timing/async_wake_order.op).

**Fuel is per-run, shared across fibers** — the sleep charge is the EXACT
helper the blocking builtin uses (ms×1000 against the step budget AND the
shared pool), channel parks charge the wake-slice shape on the same
50 ms/10 ms cadence the blocking builtins pay, and a failed charge WAKES
the parked fiber with the catchable `overflow` stress at its suspension
point: a parked fiber can never outlive the run's budget. Suspension does
not refund fuel.

**Cancellation is the W18 contract verbatim.** `cancel(id)` sets the flag
(observed at wake/tick boundaries; nothing preempted). A fiber parked on
an awaitable wakes with the catchable `cancelled` stress at the suspension
point (a 10 s sleep cancels in ~2 ms); a running fiber observes at its
next tick; a pre-tick cancel dies with `[cancelled]` before the body runs.
Join returns the standard stress map; the tombstone says `cancelled`.
Cancellation chains inherit (cancelling a cell stops its descent).

**Capabilities and diagnostics.** A fiber has exactly the spawning cell's
grants (worker-cell rule; the sandbox is untouched). Fiber notes are
prefixed `[task <id> <name>]` at completion so containment names the fiber
with its W18 task id. `join`/`cancel`/`task_state`/`wait_all`/`wait_any`
treat fibers and threads identically; join timeouts answer null + note and
keep the fiber joinable (the deadline is enforced strictly even across
blocking segments).

**Containment.** The live-task cap (4096) spans both registries — the
fiber leak storm ends on the catchable overflow
(tests/redteam/rt_p24a_fiber_leak.op). The cancel storm proves no parked
fiber outlives its join (rt_p24b). The never-fed park drains the budget
into overflow (rt_p24c).

**Out of scope (unchanged from §8, plus what this landing deliberately
does not do):** no fs/py parking yet (they execute inline inside the
fiber's segment — bounded by the same fuel, deterministic, documented);
no colored functions; no work stealing; no preemption; the sequential
oracle mirrors async programs with children-born-finished inline, so the
differential corpus pins only the ordering-free shapes — the timing shapes
are Rust-lane evidence under tests/async/timing/.
