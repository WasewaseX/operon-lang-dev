//! asyncrt.rs — W16: the async runtime (green threads over the VM loop).
//!
//! The model (docs/specs/ASYNC.md): execution units are fiber frames, not
//! OS threads. A fiber is a parked VM frame chain plus a fresh worker
//! `Interp` built with the EXACT snapshot machinery the thread path uses
//! (spawn membrane, regulation freeze, capability inheritance, shared
//! run-wide fuel pool, derived RNG stream, cancel chain). Fibers only exist
//! under `.cell io.pool` + `--vm`; without them `spawn` keeps its thread
//! path byte-for-byte (the compatibility rule: existing spawn semantics are
//! unchanged, fibers are an opt-in execution substrate for the SAME
//! observable contract, proven by test rather than promise).
//!
//! Suspension contract: a fiber parks ONLY at a native `CallNamed` position
//! calling an await-shaped builtin (`sleep`, `recv`, `select`). The park is
//! requested inside `call_builtin` (the single builtin funnel), unwinds as
//! an internal marker stress NO user containment can catch (NUL-prefixed
//! kind — the lexer refuses NUL in source, so no program can name it), and
//! every VM frame on the way out saves itself. A suspension requested from
//! a BRIDGED (tree-walk) position falls back to the builtin's existing
//! blocking behavior: the Rust call stack there cannot rewind safely, and
//! the blocking shape is today's documented, byte-parity-tested behavior.
//!
//! Determinism: one scheduler per run, pumped only from the host thread
//! (host blocking ops + joins). Ready order is FIFO; wake scans process
//! parked fibers in registration order; same-deadline sleeps wake FIFO.
//! Timing-dependent orderings (different sleep durations) are the same
//! documented class as `wait_any` (Rust-lane evidence, never the oracle).
//!
//! Fuel: per-run, shared across fibers (the same `fuel_pool` the thread
//! path drains). Sleep charges its whole ms×1000 at park (the exact sleep
//! shape); channel parks charge the wake-slice shape on the same cadence
//! the blocking builtin would, so a parked-forever fiber drains the run
//! budget into the catchable `overflow` exactly like a blocked worker.
//! Capability inheritance: a fiber gets the spawning cell's caps (the
//! worker-cell rule; the sandbox is unchanged). Diagnostics: fiber notes
//! are prefixed `[task <id> <name>]` at completion so containment names the
//! fiber with its W18 task id.

use crate::genes::SendValue;
use crate::interp::{Env, Interp, TaskState};
use crate::value::{ChannelShared, Value};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// The internal park marker kind. NUL-prefixed: unreachable by any
/// source-level stress name or builtin. Excluded explicitly at every
/// containment site (rescue arms, pattern/guard containment) so user
/// `stress {} rescue {}` can never observe, contain, or spoof it.
pub const FIBER_PARK: &str = "\u{0}fiber-park";

pub fn is_fiber_park(s: &crate::value::Stress) -> bool {
    s.kind == FIBER_PARK
}

/// Why a fiber is parked. `order` pins the deterministic wake scan;
/// `since` paces the fuel wake-slice charges for channel parks.
pub enum ParkReq {
    /// awaitable sleep: the whole ms×1000 charge happened at park time.
    Sleep {
        wake_at: std::time::Instant,
        order: u64,
    },
    /// recv on an empty-and-open channel: wake when a value lands or close.
    Recv {
        ch: Arc<ChannelShared>,
        order: u64,
        since: std::time::Instant,
    },
    /// select with nothing ready and at least one channel open.
    Select {
        chans: Vec<Arc<ChannelShared>>,
        order: u64,
        since: std::time::Instant,
    },
}

impl ParkReq {
    pub fn order(&self) -> u64 {
        match self {
            ParkReq::Sleep { order, .. } => *order,
            ParkReq::Recv { order, .. } => *order,
            ParkReq::Select { order, .. } => *order,
        }
    }
    fn touch(&mut self) {
        let now = std::time::Instant::now();
        match self {
            ParkReq::Sleep { .. } => {}
            ParkReq::Recv { since, .. } | ParkReq::Select { since, .. } => *since = now,
        }
    }
}

/// The completion outcome a fiber writes into its registry handle (the same
/// surface a thread worker sends through its channel: value or stress map,
/// plus the notes for the joining interpreter).
pub struct FiberOutcome {
    pub value: SendValue,
    pub notes: Vec<crate::ast::Note>,
}

/// Registry entry in the SPAWNING interpreter's `fiber_tasks` (the same
/// per-interp namespace the thread `tasks` map uses). The pump never looks
/// at registries: it talks to the Arcs, exactly the way a thread worker
/// sets its phase before its result leaves.
pub struct FiberHandle {
    pub cancel: Arc<AtomicBool>,
    pub state: Arc<Mutex<TaskState>>,
    pub done: Arc<Mutex<Option<FiberOutcome>>>,
    /// scheduler key (diagnostics/debug; the program-visible id is the
    /// registry key here, same namespace as thread task ids).
    pub global: u64,
}

/// One green thread: the parked frame chain (outermost first) + the worker
/// interpreter that owns the gene's cell state.
pub struct Fiber {
    pub global: u64,
    pub name: String,
    pub interp: Interp,
    /// outermost (entry gene) first, innermost last. Empty only BEFORE the
    /// first segment (the entry runs through call_named for the full gate
    /// funnel); after the first park the chain lives here.
    pub frames: Vec<crate::vm::FrameState>,
    pub park: Option<ParkReq>,
    /// value/error to deliver into the next resumed frame's parked call.
    pub resume: Option<Result<Value, crate::value::Stress>>,
    /// first-segment entry state (before the gene body ever ran).
    pub entry: Option<FiberEntry>,
    /// completion Arcs (mirrors of the registry handle).
    pub state: Arc<Mutex<TaskState>>,
    pub done: Arc<Mutex<Option<FiberOutcome>>>,
    pub finished: bool,
}

pub struct FiberEntry {
    pub genv: Rc<Env>,
    pub task_name: String,
    pub args: Vec<Value>,
}

/// The scheduler. Lives for the whole run on the host thread (TLS); every
/// fiber segment runs inside the pump on that thread, so single-thread
/// types are sound and there is nothing to poison. Thread workers never
/// see it (their thread's TLS is empty) — the thread path is untouched.
#[derive(Default)]
pub struct AsyncRt {
    pub fibers: HashMap<u64, Fiber>,
    /// FIFO ready queue of scheduler-global fiber ids.
    pub ready: VecDeque<u64>,
    pub next_order: u64,
    pub next_global: u64,
}

thread_local! {
    static SCHED: RefCell<Option<Rc<RefCell<AsyncRt>>>> = const { RefCell::new(None) };
}

fn with_sched<R>(f: impl FnOnce(&mut AsyncRt) -> R) -> Option<R> {
    SCHED.with(|s| {
        let rc = s.borrow().as_ref().cloned()?;
        let mut rt = rc.borrow_mut();
        Some(f(&mut rt))
    })
}

fn ensure_sched() {
    SCHED.with(|s| {
        if s.borrow().is_none() {
            *s.borrow_mut() = Some(Rc::new(RefCell::new(AsyncRt::default())));
        }
    });
}

/// True when this thread pumps a scheduler with at least one live fiber.
/// Cheap gate for the host blocking ops (recv/select/sleep/join/wait_any):
/// zero cost when no async run is active, full behavior when fibers exist.
pub fn has_fibers() -> bool {
    SCHED.with(|s| match s.borrow().as_ref() {
        Some(rt) => !rt.borrow().fibers.is_empty(),
        None => false,
    })
}

/// Spawn decision: `--vm` + `.cell io.pool` on + this thread can pump.
/// Worker THREADS never fiber (fresh interps carry no .cell and their
/// thread's TLS scheduler is absent) — the thread path stays the default.
pub fn fiber_mode(interp: &Interp) -> bool {
    if !interp.vm {
        return false;
    }
    // the host reads the `.cell io.pool` key; fibers carry the lane flag
    // (workers inherit no raw .cell) so nested spawns stay on the substrate
    let on = interp.io_pool
        || interp
            .cell
            .get("io.pool")
            .map(|v| v == "on" || v == "true" || v == "1")
            .unwrap_or(false);
    if !on {
        return false;
    }
    ensure_sched();
    SCHED.with(|s| s.borrow().is_some())
}

/// Live fiber count (for the shared live-task cap).
pub fn live_fibers() -> usize {
    SCHED.with(|s| match s.borrow().as_ref() {
        Some(rt) => rt.borrow().fibers.len(),
        None => 0,
    })
}

/// Build the registry handle + the scheduler-side fiber from the parts the
/// spawn lane prepared (the worker interp is pre-configured by spawn_task:
/// snapshot, regulation, caps, fuel pool, medium, RNG, cancel chain).
pub struct FiberParts {
    pub name: String,
    pub interp: Interp,
    pub entry: FiberEntry,
    pub cancel: Arc<AtomicBool>,
}

/// Create the fiber + its registry handle. Returns (local-id is assigned by
/// the caller's next_task_id flow) the handle for the spawning registry.
pub fn launch(parts: FiberParts) -> FiberHandle {
    ensure_sched();
    let global = with_sched(|rt| {
        rt.next_global += 1;
        rt.next_global
    })
    .unwrap_or(0);
    let handle = FiberHandle {
        cancel: parts.cancel.clone(),
        state: Arc::new(Mutex::new(TaskState::Running)),
        done: Arc::new(Mutex::new(None)),
        global,
    };
    let fiber = Fiber {
        global,
        name: parts.name,
        interp: parts.interp,
        frames: Vec::new(),
        park: None,
        resume: None,
        entry: Some(parts.entry),
        state: handle.state.clone(),
        done: handle.done.clone(),
        finished: false,
    };
    with_sched(|rt| {
        rt.fibers.insert(global, fiber);
        rt.ready.push_back(global);
    });
    handle
}

// ---------------------------------------------------------------- pump

/// Run ONE scheduler step: wake scan (registration order), then run the
/// front ready fiber to completion-or-park. Returns false when nothing was
/// runnable (callers do their own slice wait, exactly like today's
/// blocking builtins).
///
/// Borrow discipline: the running fiber is REMOVED from the scheduler map
/// before its segment executes (phase 2 runs with the RefCell borrow
/// RELEASED) — a fiber body spawns fibers, joins, and parks, and each of
/// those re-enters the scheduler. A re-entrant `borrow_mut` would panic;
/// the take-run-return protocol is what makes nesting sound.
pub fn step_once() -> bool {
    // ---- phase 1: wake scan + dequeue (under the borrow)
    let running = with_sched(|rt| {
        let now = std::time::Instant::now();
        // wake pass in registration order (deterministic FIFO ties)
        let mut parked: Vec<(u64, u64)> = rt
            .fibers
            .iter()
            .filter(|(_, f)| f.park.is_some())
            .map(|(g, f)| (f.park.as_ref().unwrap().order(), *g))
            .collect();
        parked.sort_unstable();
        let mut wake_ids: Vec<u64> = Vec::new();
        for (_o, g) in parked {
            let f = match rt.fibers.get_mut(&g) {
                Some(f) => f,
                None => continue,
            };
            // cancel beats every wake reason: a cancelled park wakes with
            // the catchable `cancelled` stress at the suspension point
            if f.interp
                .cancel_chain
                .iter()
                .any(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            {
                f.park = None;
                f.resume = Some(Err(crate::value::Stress::new(
                    "cancelled",
                    "task cancelled",
                )));
                wake_ids.push(g);
                continue;
            }
            // decide the wake action WITHOUT mutating, then apply (the
            // park request stays borrowed while the channel state is read)
            enum Wake {
                None,
                Ready(Value),
                Charge(u64),
            }
            let action = match &f.park {
                None => Wake::None,
                Some(ParkReq::Sleep { wake_at, .. }) => {
                    if *wake_at <= now {
                        Wake::Ready(Value::Null) // sleep returns null
                    } else {
                        Wake::None
                    }
                }
                Some(ParkReq::Recv { ch, since, .. }) => {
                    let mut st = ch.state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(sv) = st.queue.pop_front() {
                        Wake::Ready(crate::genes::from_send(sv))
                    } else if st.closed {
                        Wake::Ready(Value::Null) // closed+empty = null
                    } else {
                        // still parked: keep the wake-slice fuel cadence the
                        // blocking builtin would have paid (containment: a
                        // never-fed channel drains the budget into overflow)
                        drop(st);
                        if now.duration_since(*since).as_millis() as u64 >= Interp::RECV_SLICE_MS {
                            Wake::Charge(Interp::RECV_SLICE_MS)
                        } else {
                            Wake::None
                        }
                    }
                }
                Some(ParkReq::Select { chans, since, .. }) => {
                    let mut ready_idx: Option<usize> = None;
                    let mut any_open = false;
                    for (i, c) in chans.iter().enumerate() {
                        let st = c.state.lock().unwrap_or_else(|e| e.into_inner());
                        if !st.queue.is_empty() {
                            ready_idx = Some(i);
                            break;
                        }
                        if !st.closed {
                            any_open = true;
                        }
                    }
                    if let Some(i) = ready_idx {
                        Wake::Ready(Value::Int(i as i64))
                    } else if !any_open {
                        Wake::Ready(Value::Int(-1))
                    } else if now.duration_since(*since).as_millis() as u64
                        >= Interp::SELECT_SLICE_MS
                    {
                        Wake::Charge(Interp::SELECT_SLICE_MS)
                    } else {
                        Wake::None
                    }
                }
            };
            match action {
                Wake::None => {}
                Wake::Ready(v) => {
                    f.park = None;
                    f.resume = Some(Ok(v));
                    wake_ids.push(g);
                }
                Wake::Charge(ms) => {
                    // containment: a failed charge (the run-wide pool or the
                    // fiber's own budget exhausted) WAKES the fiber with the
                    // overflow stress at its suspension point — a parked
                    // fiber can never outlive the run's budget
                    match charge_wake_slice(&mut f.interp, ms) {
                        Ok(()) => {
                            if let Some(p) = f.park.as_mut() {
                                p.touch();
                            }
                        }
                        Err(s) => {
                            f.park = None;
                            f.resume = Some(Err(s));
                            wake_ids.push(g);
                        }
                    }
                }
            }
        }
        for g in wake_ids {
            rt.ready.push_back(g);
        }
        // dequeue the next ready fiber and TAKE it out (the borrow releases
        // before the segment runs)
        rt.ready
            .pop_front()
            .and_then(|g| rt.fibers.remove(&g).map(|f| (g, f)))
    });
    // ---- phase 2: run the segment (NO scheduler borrow held — nested
    // spawn/join/park re-enter freely)
    let Some((g, mut f)) = running.flatten() else {
        return false; // nothing runnable
    };
    run_segment(&mut f);
    if f.finished {
        // record the terminal phase (the worker-side rule: BEFORE the
        // result becomes visible)
        let cancelled = matches!(
            &*f.done.lock().unwrap_or_else(|e| e.into_inner()),
            Some(FiberOutcome {
                value: SendValue::Stress(k, _),
                ..
            }) if k == "cancelled"
        );
        *f.state.lock().unwrap_or_else(|e| e.into_inner()) = if cancelled {
            TaskState::Cancelled
        } else {
            TaskState::Done
        };
        // phase 3: the finished fiber is dropped (outcome already published)
        true
    } else {
        // phase 3: return the parked fiber to the scheduler
        with_sched(|rt| {
            rt.fibers.insert(g, f);
        });
        true
    }
}

/// The wake-slice charge: the exact `blocking_wake` shape (shared pool +
/// step budget + cancel observation) applied to the parked fiber's own
/// interpreter. A failed charge is the fiber's problem at its next segment
/// (overflow surfaces there); the scan keeps scanning.
fn charge_wake_slice(interp: &mut Interp, slice_ms: u64) -> Result<(), crate::value::Stress> {
    let charge = slice_ms.saturating_mul(1000);
    interp.steps = interp.steps.saturating_add(charge);
    if let Some(pool) = &interp.fuel_pool {
        let left = pool.fetch_sub(charge as i64, std::sync::atomic::Ordering::Relaxed);
        if left <= charge as i64 {
            return Err(crate::value::Stress::new(
                "overflow",
                "run-wide step budget exhausted (channel wait)",
            ));
        }
    }
    if interp.steps > interp.step_budget {
        return Err(crate::value::Stress::new(
            "overflow",
            "step budget exhausted (channel wait)",
        ));
    }
    Ok(())
}

/// Earliest pending sleep deadline (for host-side slice waits).
pub fn next_wake() -> Option<std::time::Instant> {
    SCHED.with(|s| {
        let rc = {
            let b = s.borrow();
            b.as_ref().cloned()?
        };
        let rt = rc.borrow();
        let mut min: Option<std::time::Instant> = None;
        for f in rt.fibers.values() {
            if let Some(ParkReq::Sleep { wake_at, .. }) = &f.park {
                min = Some(match min {
                    Some(m) if *wake_at >= m => m,
                    _ => *wake_at,
                });
            }
        }
        min
    })
}

/// Run one segment of a fiber: resume the innermost frame (or run the
/// entry call for a fresh fiber) until it parks, completes, or fails.
fn run_segment(f: &mut Fiber) {
    f.interp.fiber_active = true;
    // fresh fiber: run the entry through the FULL call funnel (gates, param
    // binding, guards all run in the shared code before the body), so a
    // fiber's first segment is behavior-identical to a thread worker's body.
    if f.frames.is_empty() {
        if let Some(entry) = f.entry.take() {
            let FiberEntry {
                genv,
                task_name,
                args,
            } = entry;
            let result = f.interp.call_named(&genv, &task_name, args);
            adopt_parked(f);
            match result {
                Ok(v) => {
                    finish(f, Ok(v));
                    return;
                }
                Err(s) => {
                    if !crate::asyncrt::is_fiber_park(&s) {
                        finish(f, Err(s));
                        return;
                    }
                    // parked inside the body: the frames were adopted above
                    // and the wake request rides on the interpreter — take
                    // it so the scheduler can wake this fiber (the resume
                    // path does the same after ITS park).
                    f.park = f.interp.fiber_park.take();
                    // no VM frame arrived would mean the hook parked at
                    // suspend_depth > 0 (it refuses to) — fail safe rather
                    // than lose the fiber silently.
                    if f.frames.is_empty() {
                        finish(
                            f,
                            Err(crate::value::Stress::new(
                                "overflow",
                                "fiber parked frameless",
                            )),
                        );
                    }
                    return;
                }
            }
        }
    }
    // resume path: innermost frame, completion chains outward
    loop {
        let resume = f.resume.take();
        let Some(mut st) = f.frames.pop() else {
            finish(f, Ok(Value::Null)); // defensive: frames drained, no Ret
            return;
        };
        st.was_resumed = true;
        let code = st.code.clone();
        let r = crate::vm::exec_frame(&mut f.interp, &code, &mut st, resume);
        if let Err(s) = &r {
            if crate::asyncrt::is_fiber_park(s) {
                // this frame parked at its own level: save it first, then
                // adopt any deeper frames the unwind collected through its
                // nested call chain (outermost-first append order).
                f.frames.push(st);
                adopt_parked(f);
                f.park = f.interp.fiber_park.take();
                return;
            }
        }
        match r {
            Err(s) => {
                // a real failure: outermost frame → the fiber dies; else
                // deliver the error into the caller frame's parked call
                if f.frames.is_empty() {
                    finish(f, Err(s));
                    return;
                }
                f.resume = Some(Err(s));
            }
            Ok(flow) => {
                // frame completed: Ret(v) / Norm (fell off) / Brk / Cont —
                // at gene boundaries every non-Ret flow means an implicit
                // null (the tree-walk contract at gene scope). A frame that
                // ever parked never ran call_gene's tail, so its return
                // annotation applies here (the same shared check).
                let v = match flow {
                    crate::interp::Flow::Ret(v) => Some(v),
                    _ => None,
                };
                if st.was_resumed {
                    // the same shared check call_gene's tail would have run
                    // (a parked frame's tail never executed)
                    if st.ret_ann.is_some() {
                        if let Some(name) = st.name.as_ref() {
                            let checked = match &v {
                                Some(v) => {
                                    f.interp.check_ret_ann("gene", name, &st.ret_ann, v, true)
                                }
                                None => f.interp.check_ret_ann(
                                    "gene",
                                    name,
                                    &st.ret_ann,
                                    &Value::Null,
                                    false,
                                ),
                            };
                            if let Err(s) = checked {
                                if f.frames.is_empty() {
                                    finish(f, Err(s));
                                    return;
                                }
                                f.resume = Some(Err(s));
                                continue;
                            }
                        }
                    }
                }
                let v = v.unwrap_or(Value::Null);
                if f.frames.is_empty() {
                    finish(f, Ok(v));
                    return;
                }
                f.resume = Some(Ok(v));
            }
        }
    }
}

/// Adopt the frame chain the park unwind collected into the fiber's
/// interpreter (`exec_gene_code` pushes each parking frame at index 0, so
/// the unwind leaves the vec outermost-first — append order preserved).
fn adopt_parked(f: &mut Fiber) {
    let collected = std::mem::take(&mut f.interp.fiber_frames);
    f.frames.extend(collected);
}

/// Fiber completion: convert exactly the way the thread worker does (a
/// propagated variant IS the return value, D-014), prefix the notes with
/// the task tag, publish the outcome + phase.
fn finish(f: &mut Fiber, result: Result<Value, crate::value::Stress>) {
    f.interp.fiber_active = false;
    let sv = match result {
        Ok(v) => crate::genes::to_send(&v),
        Err(s) => {
            if let Some(v) = s.prop {
                crate::genes::to_send(&v)
            } else {
                SendValue::Stress(s.kind.clone(), s.message.clone())
            }
        }
    };
    let global_note = format!("[task {} {}]", task_display_id(f), f.name);
    let mut notes = std::mem::take(&mut f.interp.notes);
    for n in notes.iter_mut() {
        n.message = format!("{} {}", global_note, n.message);
    }
    *f.done.lock().unwrap_or_else(|e| e.into_inner()) = Some(FiberOutcome { value: sv, notes });
    f.finished = true;
}

fn task_display_id(f: &Fiber) -> i64 {
    f.interp.fiber_task_id
}

// ------------------------------------------------- suspension hook

/// The await-shaped builtin hook, called at the TOP of `call_builtin` for
/// fiber interps at native positions. Returns:
/// - `None` → not an awaitable / not parked / bridged position: the builtin
///   runs its normal (blocking) path unchanged;
/// - `Some(Ok(v))` → the operation completed without parking (buffered
///   recv, ready select, zero sleep) — the builtin returns this;
/// - `Some(Err(marker))` → the fiber parks: the marker unwinds to the
///   scheduler through the native call chain.
pub fn fiber_suspend_point(
    interp: &mut Interp,
    name: &str,
    args: &[Value],
) -> Option<Result<Value, crate::value::Stress>> {
    if !interp.fiber_active || interp.vm_suspend_depth > 0 {
        return None;
    }
    match name {
        "sleep" => {
            let ms = match args.first() {
                Some(Value::Int(i)) => (*i).max(0) as u64,
                Some(Value::Float(f)) => (*f).max(0.0) as u64,
                _ => 0,
            };
            let ms = ms.min(60_000);
            // the exact sleep charge shape (the builtin shares this helper)
            if let Err(s) = interp.sleep_charge(ms) {
                return Some(Err(s));
            }
            let order = next_order();
            interp.fiber_park = Some(ParkReq::Sleep {
                wake_at: std::time::Instant::now() + std::time::Duration::from_millis(ms),
                order,
            });
            Some(Err(crate::value::Stress::new(FIBER_PARK, "")))
        }
        "recv" => {
            let ch = match args.first() {
                Some(Value::Channel(c)) => c.clone(),
                _ => return None, // the builtin raises the type stress
            };
            let mut st = ch.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(sv) = st.queue.pop_front() {
                return Some(Ok(crate::genes::from_send(sv)));
            }
            if st.closed {
                return Some(Ok(Value::Null));
            }
            drop(st);
            // first wake slice charged at park (the blocking cadence starts)
            if let Err(s) = charge_wake_slice(interp, Interp::RECV_SLICE_MS) {
                return Some(Err(s));
            }
            let order = next_order();
            interp.fiber_park = Some(ParkReq::Recv {
                ch,
                order,
                since: std::time::Instant::now(),
            });
            Some(Err(crate::value::Stress::new(FIBER_PARK, "")))
        }
        "select" => {
            let mut chans: Vec<Arc<ChannelShared>> = Vec::new();
            for a in args {
                match a {
                    Value::Channel(c) => chans.push(c.clone()),
                    _ => return None, // the builtin raises the type stress
                }
            }
            if chans.is_empty() {
                return None; // the builtin notes + answers -1
            }
            // declaration-order poll (the builtin's law, shared shape)
            let mut ready_idx: Option<usize> = None;
            let mut any_open = false;
            for (i, c) in chans.iter().enumerate() {
                let st = c.state.lock().unwrap_or_else(|e| e.into_inner());
                if !st.queue.is_empty() {
                    ready_idx = Some(i);
                    break;
                }
                if !st.closed {
                    any_open = true;
                }
            }
            if let Some(i) = ready_idx {
                return Some(Ok(Value::Int(i as i64)));
            }
            if !any_open {
                return Some(Ok(Value::Int(-1)));
            }
            if let Err(s) = charge_wake_slice(interp, Interp::SELECT_SLICE_MS) {
                return Some(Err(s));
            }
            let order = next_order();
            interp.fiber_park = Some(ParkReq::Select {
                chans,
                order,
                since: std::time::Instant::now(),
            });
            Some(Err(crate::value::Stress::new(FIBER_PARK, "")))
        }
        _ => None,
    }
}

fn next_order() -> u64 {
    with_sched(|rt| {
        rt.next_order += 1;
        rt.next_order
    })
    .unwrap_or(0)
}

// ------------------------------------------------- host-side pump gates

/// Pump fibers while a blocking host operation waits. One step per call;
/// the host wait loops call this between their own slice waits. No-op when
/// no fibers exist (the common case: zero behavior change).
pub fn pump_step() {
    if has_fibers() {
        step_once();
    }
}

/// Pump until the fiber's completion outcome lands (join's fiber lane).
/// The OUTCOME, not the phase, is the join signal: `cancel()` flips the
/// phase immediately (the thread-path contract) while the fiber still has
/// to unwind; the outcome lands exactly when the worker would have sent on
/// its channel. Honors the join ceilings by returning on `deadline`.
pub fn pump_until_done(
    done: &Mutex<Option<FiberOutcome>>,
    deadline: Option<std::time::Instant>,
) -> bool {
    loop {
        {
            let d = done.lock().unwrap_or_else(|e| e.into_inner());
            if d.is_some() {
                return true;
            }
        }
        if let Some(d) = deadline {
            if std::time::Instant::now() >= d {
                return false;
            }
        }
        // a segment can BLOCK past the deadline (bridged awaits fall back
        // to the blocking builtins) — an outcome that lands during such a
        // step still answers null: the join timeout contract says "join(id,
        // ms) returns null when the worker exceeds it", the fiber stays
        // joinable and a later join picks the value up.
        if let Some(d) = deadline {
            if std::time::Instant::now() >= d {
                return false;
            }
        }
        let had_outcome = {
            let d = done.lock().unwrap_or_else(|e| e.into_inner());
            d.is_some()
        };
        if had_outcome {
            return true;
        }
        if !step_once() {
            // nothing runnable: sleep to the next wake or a bounded slice
            let now = std::time::Instant::now();
            let until = next_wake()
                .map(|w| w.min(now + std::time::Duration::from_millis(Interp::RECV_SLICE_MS)))
                .unwrap_or(now + std::time::Duration::from_millis(Interp::RECV_SLICE_MS));
            let now2 = std::time::Instant::now();
            if until > now2 {
                std::thread::sleep(until - now2);
            }
        }
        // the step may have overshot the deadline (a blocking segment): an
        // outcome landing past the deadline answers null per the contract
        if let Some(d) = deadline {
            if std::time::Instant::now() >= d {
                return false;
            }
        }
    }
}
