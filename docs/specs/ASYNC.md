# ASYNC, the async model spec sketch (W16, ROADMAP-100)

Status: **spec only, deliberately**. The roadmap's W16 gate for this cycle
is "spec accepted; the VM design reserves frame fields for it", no
implementation. Normative when the time comes; conflicts resolve toward
SPEC.md and docs/vm-design.md.

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
