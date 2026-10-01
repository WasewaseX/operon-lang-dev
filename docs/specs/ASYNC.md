# ASYNC, the async model spec (W16, ROADMAP-100)

Status: **IMPLEMENTED (W16, 2026-10-01)** — the model below is normative
AND live: `.cell io.pool = fiber` runs spawned tasks as fibers on the VM
loop (src/sched.rs + the fiber machine in src/vm.rs). The implementation
contract is §9 at the bottom of this file; conflicts resolve toward
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

## 9. Implementation contract (W16, the shipped behavior)

- **Lane gate.** `spawn` splits AFTER the shared pre-flight (weak-handle
  refusal, depth cap, id claim, seed derivation, membrane snapshot): with
  `.cell io.pool = fiber` (and the VM lane, the default) the task becomes a
  FIBER; every other spawn rides the OS-thread worker exactly as before.
  The snapshot membrane, caps/fuel/medium/cancel inheritance and the task
  registry are the same code on both lanes — identical by construction,
  not by promise (tests/async_parity.rs pins byte parity per program:
  stdout, notes, rc, lifecycle).
- **The frame stack.** A fiber owns `Vec<VmFrame>` — the heap frame stack
  vm-design §6 reserved. Named-gene calls inside a fiber push frames
  through the ONE hook at call_gene_inner's body-exec point, so RISC ->
  toggle -> GRN -> methylation -> riboswitch -> promoter -> RHO, param
  binding, uORF guards and call bookkeeping run in the SHARED funnel; the
  fiber machine never reimplements a gate. `fiber_state`, `wake_deadline`
  and `cancel_flag` are real fields now.
- **The scheduler is a plain queue.** FIFO ready; run-to-completion until
  a suspension point; a send wakes the FIRST-parked waiter on that channel;
  selects re-poll in declaration order (leftmost ready wins) and re-park
  at the back; timers wake in deadline order, ties in park order. Wake
  order is pinned by tests (tests/async_parity.rs wake-order repeat pin,
  src/sched.rs unit pins).
- **The clock is virtual.** `sleep` parks with `wake_deadline`; when
  nothing is ready the scheduler jumps to the earliest deadline. Parked
  receivers are charged 1000 steps per virtual ms — the thread lane's
  exact rate — so fuel stays the liveness guard: a program that would
  deadlock dies with the identical overflow stress, deterministically.
  Sleep charges ms*1000 at the call, on both lanes; suspension never
  refunds fuel.
- **Suspension points.** `sleep`, `recv`, `select` park when reached at a
  compiled call site of the fiber's own frames. Reached inside bridged
  tree-walk code, uORF guards or default-arg evaluation they degrade to
  the thread lane's blocking behavior (blocking mid-turn cannot reorder a
  cooperative scheduler — it is deterministic, just not interleaved).
- **Cancellation (W18) is unchanged.** `cancel()` sets the shared flag;
  ticks observe it; a chan-parked fiber unblocks at the next boundary and
  fails with the catchable `cancelled` stress; the phase register names
  `cancelled` (everything else is `done`), both lanes.
- **Join/wait/task_state are lane-blind.** join(id) drains the scheduler;
  the thread lane's 300 s wall ceiling becomes 300_000 virtual ms with the
  same observable (null + note + the task stays joinable). wait_all/
  wait_any/scope reaping ride the same drain. task_state reads the same
  phase register.
- **Unjoined tasks at program end are abandoned** on both lanes (threads
  are detached; the scheduler drops with the interp) — deterministic
  programs never observe the difference, racy ones cannot be pinned
  anyway.
- **Parity evidence.** tests/async_parity.rs: the same program on
  `io.pool = fiber` and `io.pool = thread` produces identical stdout,
  identical notes and identical lifecycle observables — join ordering,
  cancellation, membrane refusals, fuel-exhaustion wire shape, note
  prefixes, FIFO wake stability across repeats.
