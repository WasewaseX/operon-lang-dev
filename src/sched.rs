//! sched.rs, W16: the fiber scheduler (docs/specs/ASYNC.md).
//!
//! A plain queue, not magic. The scheduler owns fiber tasks — each a full
//! task Interp (the snapshot-membrane setup spawn has always built for
//! worker threads) plus a heap `Fiber` (the VM frame stack) — and runs them
//! on the host thread, one turn at a time, in FIFO order:
//!
//! ```text
//! VM
//!  │
//!  ├── Fiber A ──sleep──────┐
//!  ├── Fiber B ──chan.recv──┤
//!  ├── Fiber C ──running────┤
//!  │                        ▼
//!  │                    scheduler
//!  │                        │
//!  │               FIFO deterministic wake
//! ```
//!
//! Determinism contract (pinned by tests, not promise):
//! - ready fibers run to completion/suspension, FIFO from the front;
//! - a send wakes the FIRST-parked waiter on that channel (FIFO by park
//!   order); selects re-poll in declaration order (leftmost ready wins)
//!   and re-park at the BACK if still nothing;
//! - when nothing is ready, the virtual clock jumps to the earliest sleep
//!   deadline; ties wake in park order;
//! - cancel unblocks chan-parked fibers at the next boundary (the thread
//!   lane observes cancel per blocking slice — same contract);
//! - fuel is the liveness guard: every virtual slice charges exactly what
//!   the thread lane's real slice charged (1000 steps/ms), so a program
//!   that would deadlock dies with the identical overflow stress, just
//!   deterministically.
//!
//! The clock is virtual: `sleep` never blocks wall time inside the fiber
//! lane and tests run in microseconds.

use crate::ast::Note;
use crate::genes::SendValue;
use crate::interp::{Interp, TaskState};
use crate::value::{Stress, Value};
use crate::vm::{fiber_run, Fiber, FiberOutcome, PendingWake};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

type Chan = Arc<crate::value::ChannelShared>;

/// A channel event a fiber (or the host, or a worker thread) produced.
/// The scheduler consumes these FIFO at every drain boundary.
pub enum WakeReq {
    Sent(Chan),
    Closed(Chan),
}

/// One fiber task: the worker-thread setup made single-threaded. The task
/// interp carries the snapshot membrane, task-id-derived rng, host caps,
/// the shared fuel pool, the shared medium and the cancel chain — identical
/// to what spawn_task builds for an OS-thread worker, minus the thread.
pub struct FiberTask {
    pub id: i64,
    pub name: String,
    pub interp: Interp,
    pub fiber: Fiber,
    /// W18: the cooperative cancel flag (also lives at the end of the task
    /// interp's cancel chain; observed at ticks and at virtual slices)
    pub cancel: Arc<AtomicBool>,
    /// W18: lifecycle phase, set by the completion paths BEFORE the result
    /// is readable (the worker thread's own discipline)
    pub state: Arc<Mutex<TaskState>>,
    /// the joined result: (wire value, prefixed notes) — None until done
    pub result: Option<(SendValue, Vec<Note>)>,
    pub done: bool,
}

impl FiberTask {
    pub(crate) fn finish_ok(&mut self, v: Value) {
        let notes = Self::prefixed(&self.interp.notes, &self.name);
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = TaskState::Done;
        self.result = Some((crate::genes::to_send(&v), notes));
        self.done = true;
    }

    pub(crate) fn finish_err(&mut self, s: Stress) {
        let notes = Self::prefixed(&self.interp.notes, &self.name);
        // W18: a cancelled-run stress names the phase; everything else
        // counts as done (the worker thread's exact rule)
        let phase = if s.kind == "cancelled" {
            TaskState::Cancelled
        } else {
            TaskState::Done
        };
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = phase;
        self.result = Some((SendValue::Stress(s.kind, s.message), notes));
        self.done = true;
    }

    /// the worker thread's note discipline: every task note is prefixed so
    /// the host can name the cell it came from
    fn prefixed(notes: &[Note], name: &str) -> Vec<Note> {
        let global_note = format!("[task {}]", name);
        notes
            .iter()
            .cloned()
            .map(|mut n| {
                n.message = format!("{} {}", global_note, n.message);
                n
            })
            .collect()
    }

    /// Deliver the awaited value for a recv parked on `ch`: pop the front
    /// item (the scheduler is the ONLY popper for parked fibers), or the
    /// closed+empty null. Returns the value for the task's wake_result.
    fn recv_answer(ch: &Chan) -> Value {
        let mut st = ch.state.lock().unwrap_or_else(|e| e.into_inner());
        match st.queue.pop_front() {
            Some(sv) => crate::genes::from_send(sv),
            None => Value::Null,
        }
    }

    /// Re-poll a select's channel list in declaration order: the leftmost
    /// ready index, or -1 when every channel is closed; None = stay parked.
    fn select_answer(chans: &[Chan]) -> Option<Value> {
        let mut any_open = false;
        for (i, c) in chans.iter().enumerate() {
            let st = c.state.lock().unwrap_or_else(|e| e.into_inner());
            if !st.queue.is_empty() {
                return Some(Value::Int(i as i64));
            }
            if !st.closed {
                any_open = true;
            }
        }
        if any_open {
            None
        } else {
            Some(Value::Int(-1))
        }
    }
}

/// The scheduler state. Lives on the HOST Interp (`Interp::sched`); fiber
/// task interps reach it only through the shared `wake_reqs` queue (they
/// run on the same thread, but ownership rules keep them decoupled).
pub struct AsyncSched {
    pub ready: VecDeque<FiberTask>,
    /// parked on sleep, in park order (ties wake FIFO)
    pub timers: Vec<FiberTask>,
    /// parked on channels, in park order: (task, chans, is_select)
    pub chan_parks: Vec<(FiberTask, Vec<Chan>, bool)>,
    /// completed but not yet joined (join/wait_all/task_state read these)
    pub finished: Vec<FiberTask>,
    /// the virtual clock in milliseconds
    pub now_ms: u64,
    /// shared with every task interp (fiber AND thread workers): sends and
    /// closes land here, the scheduler consumes them at drain boundaries
    pub wake_reqs: Arc<Mutex<Vec<WakeReq>>>,
}

impl AsyncSched {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for AsyncSched {
    fn default() -> Self {
        AsyncSched {
            ready: VecDeque::new(),
            timers: Vec::new(),
            chan_parks: Vec::new(),
            finished: Vec::new(),
            now_ms: 0,
            wake_reqs: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl AsyncSched {
    /// Anything still owned live (running or parked)? Finished tasks wait
    /// in `finished` for their join and do not count.
    pub fn has_pending(&self) -> bool {
        !self.ready.is_empty() || !self.timers.is_empty() || !self.chan_parks.is_empty()
    }

    fn take_wake_reqs(&self) -> Vec<WakeReq> {
        let mut q = self.wake_reqs.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *q)
    }

    fn parks_on(cs: &[Chan], ch: &Chan) -> bool {
        cs.iter().any(|c| Arc::ptr_eq(c, ch))
    }

    /// Consume wake requests FIFO: a send wakes the first-parked recv
    /// waiter on that channel; a select touching it re-polls in declaration
    /// order (leftmost ready wins) and re-parks at the BACK when still
    /// nothing; a close wakes every recv waiter (closed+empty = null, the
    /// only null recv can produce) and re-polls selects.
    fn wake_on_requests(&mut self) {
        for req in self.take_wake_reqs() {
            match req {
                WakeReq::Sent(ch) => {
                    // Z-120 (#120): a RUNNING fiber's recv pops the queue
                    // directly (builtin_recv) WITHOUT consuming this request
                    // — a stale Sent whose value was already consumed must
                    // NOT wake a parked waiter: that wake's recv_answer
                    // would pop an EMPTY OPEN channel and deliver null, but
                    // closed+empty is the only null recv may produce (the
                    // pinned invariant). Drop the stale request; the waiter
                    // stays parked for the next real send (the scheduler's
                    // stall return keeps a fully-parked program honest),
                    // exactly the documented "send wakes the first-parked
                    // waiter" rule minus the consumed-value theft wake.
                    {
                        let st = ch.state.lock().unwrap_or_else(|e| e.into_inner());
                        if st.queue.is_empty() {
                            continue;
                        }
                    }
                    // first recv-parked waiter on this channel, FIFO
                    let recv_pos = self
                        .chan_parks
                        .iter()
                        .position(|(_, cs, sel)| !*sel && Self::parks_on(cs, &ch));
                    if let Some(pos) = recv_pos {
                        let (mut t, _, _) = self.chan_parks.remove(pos);
                        t.fiber.wake_result = Some(FiberTask::recv_answer(&ch));
                        self.ready.push_back(t);
                        continue;
                    }
                    // then the first select-parked waiter touching it
                    let sel_pos = self
                        .chan_parks
                        .iter()
                        .position(|(_, cs, sel)| *sel && Self::parks_on(cs, &ch));
                    if let Some(pos) = sel_pos {
                        let (mut t, chans, _) = self.chan_parks.remove(pos);
                        match FiberTask::select_answer(&chans) {
                            Some(ans) => {
                                t.fiber.wake_result = Some(ans);
                                self.ready.push_back(t);
                            }
                            None => self.chan_parks.push((t, chans, true)),
                        }
                    }
                }
                WakeReq::Closed(ch) => {
                    // wake every recv waiter: closed+empty reads null (or a
                    // value if one landed before the close)
                    let mut i = 0;
                    while i < self.chan_parks.len() {
                        let (is_recv, on_ch) = {
                            let (_, cs, sel) = &self.chan_parks[i];
                            (!*sel && Self::parks_on(cs, &ch), Self::parks_on(cs, &ch))
                        };
                        if is_recv && on_ch {
                            let (mut t, _, _) = self.chan_parks.remove(i);
                            t.fiber.wake_result = Some(FiberTask::recv_answer(&ch));
                            self.ready.push_back(t);
                        } else {
                            i += 1;
                        }
                    }
                    // selects re-poll too (all-closed answers -1)
                    let mut i = 0;
                    while i < self.chan_parks.len() {
                        let (is_sel, on_ch) = {
                            let (_, cs, sel) = &self.chan_parks[i];
                            (*sel && Self::parks_on(cs, &ch), Self::parks_on(cs, &ch))
                        };
                        if is_sel && on_ch {
                            let (mut t, chans, _) = self.chan_parks.remove(i);
                            match FiberTask::select_answer(&chans) {
                                Some(ans) => {
                                    t.fiber.wake_result = Some(ans);
                                    self.ready.push_back(t);
                                }
                                None => self.chan_parks.push((t, chans, true)),
                            }
                        } else {
                            i += 1;
                        }
                    }
                }
            }
        }
    }

    /// Charge a virtual slice to every chan-parked fiber (the thread lane
    /// charged per real 50ms blocking slice; the fiber lane charges per
    /// virtual slice — same 1000 steps/ms rate, deterministic). A budget
    /// death here fails that task with the identical overflow stress.
    fn charge_slices(&mut self, delta_ms: u64) {
        let mut i = 0;
        while i < self.chan_parks.len() {
            let r = self.chan_parks[i].0.interp.blocking_wake(delta_ms);
            if let Err(s) = r {
                let (mut t, _, _) = self.chan_parks.remove(i);
                t.finish_err(s);
                self.finished.push(t);
            } else {
                i += 1;
            }
        }
    }

    /// Run the queue until the stop predicate holds or the scheduler can
    /// make no progress (nothing ready, no timers, no requests: the caller
    /// falls back to the thread lane's own blocking semantics).
    pub fn drain(&mut self, stop: &mut dyn FnMut(&AsyncSched) -> bool) {
        loop {
            if stop(self) {
                return;
            }
            self.wake_on_requests();
            // cancel unblocks parked receivers at the next boundary (the
            // thread lane observes cancel per blocking slice); the woken
            // task's next tick raises the cancelled stress
            let mut i = 0;
            while i < self.chan_parks.len() {
                if self.chan_parks[i].0.cancel.load(Ordering::Relaxed) {
                    let (mut t, _, _) = self.chan_parks.remove(i);
                    t.fiber.wake_result = Some(Value::Null);
                    self.ready.push_back(t);
                } else {
                    i += 1;
                }
            }
            if self.ready.is_empty() {
                // virtual clock: jump to the earliest sleep deadline,
                // waking every timer at or before it in park order (FIFO)
                let earliest = self
                    .timers
                    .iter()
                    .map(|t| t.fiber.wake_deadline.unwrap_or(self.now_ms))
                    .min();
                if let Some(deadline) = earliest {
                    if deadline > self.now_ms {
                        let delta = deadline - self.now_ms;
                        self.now_ms = deadline;
                        self.charge_slices(delta);
                    }
                    let mut i = 0;
                    while i < self.timers.len() {
                        let due = self.timers[i].fiber.wake_deadline.unwrap_or(self.now_ms)
                            <= self.now_ms;
                        if due {
                            let mut t = self.timers.remove(i);
                            t.fiber.wake_deadline = None;
                            t.fiber.wake_result = Some(Value::Null);
                            self.ready.push_back(t);
                        } else {
                            i += 1;
                        }
                    }
                    continue;
                }
                return; // stalled: nothing can make progress
            }
            // run ONE ready turn (FIFO from the front)
            let mut t = match self.ready.pop_front() {
                Some(t) => t,
                None => return,
            };
            t.fiber.cancel_flag = t.cancel.load(Ordering::Relaxed);
            let out = fiber_run(&mut t.interp, &mut t.fiber);
            match out {
                Ok(FiberOutcome::Done(v)) => {
                    t.finish_ok(v);
                    self.finished.push(t);
                }
                Ok(FiberOutcome::Suspended(pw)) => match pw {
                    PendingWake::Sleep(ms) => {
                        t.fiber.wake_deadline = Some(self.now_ms.saturating_add(ms));
                        self.timers.push(t);
                    }
                    PendingWake::Chan { chans, select } => {
                        self.chan_parks.push((t, chans, select));
                    }
                },
                Err(s) => {
                    t.finish_err(s);
                    self.finished.push(t);
                }
            }
        }
    }

    /// task_state for a task anywhere in the scheduler (live or finished):
    /// the phase register the completion paths set before the result left
    pub fn phase_of(&self, id: i64) -> Option<TaskState> {
        for t in self.ready.iter().chain(self.timers.iter()) {
            if t.id == id {
                return Some(*t.state.lock().unwrap_or_else(|e| e.into_inner()));
            }
        }
        for (t, _, _) in &self.chan_parks {
            if t.id == id {
                return Some(*t.state.lock().unwrap_or_else(|e| e.into_inner()));
            }
        }
        for t in &self.finished {
            if t.id == id {
                return Some(*t.state.lock().unwrap_or_else(|e| e.into_inner()));
            }
        }
        None
    }

    /// Take a finished task's result (join consumes it)
    pub fn take_finished(&mut self, id: i64) -> Option<(SendValue, Vec<Note>)> {
        let pos = self.finished.iter().position(|t| t.id == id)?;
        let t = self.finished.remove(pos);
        t.result
    }

    /// Is the task still somewhere in the scheduler? (join on timeout
    /// keeps it joinable — the same rule the thread lane has)
    pub fn is_live(&self, id: i64) -> bool {
        self.ready.iter().any(|t| t.id == id)
            || self.timers.iter().any(|t| t.id == id)
            || self.chan_parks.iter().any(|(t, _, _)| t.id == id)
            || self.finished.iter().any(|t| t.id == id)
    }
}

// ========================================================== W16 sched tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::{fiber_call_begin, FiberBegin, VmProgram};
    use std::time::Instant;

    /// Parse a program, bind its top-level statements in the interp's own
    /// global env (a task-interp shape: vm lane on, own arena).
    fn setup(src: &str) -> Interp {
        let parsed = crate::parser::parse(src);
        let mut interp = Interp::new();
        interp.vm = true;
        interp.vm_program = Some(VmProgram::default());
        for s in &parsed.stmts {
            let g = interp.global.clone();
            let _ = interp.exec_stmt(&g, s);
        }
        interp
    }

    fn task(id: i64, name: &str, src: &str, entry: &str) -> FiberTask {
        let mut interp = setup(src);
        let g = interp.global.clone();
        let fiber = match fiber_call_begin(&mut interp, &g, entry, vec![]) {
            Ok(FiberBegin::Fiber(f)) => f,
            Ok(FiberBegin::Completed(_)) => panic!("{name}: test target must reach a body"),
            Err(s) => panic!("{name}: begin failed: {}: {}", s.kind, s.message),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        // the worker discipline: the task interp's chain ends with its OWN
        // flag so ticks and cancel observations see the shared truth
        interp.cancel_chain = vec![cancel.clone()];
        interp.cancel_live = true;
        FiberTask {
            id,
            name: name.to_string(),
            interp,
            fiber,
            cancel,
            state: Arc::new(Mutex::new(TaskState::Running)),
            result: None,
            done: false,
        }
    }

    fn run_to_end(sched: &mut AsyncSched) {
        let mut guard = 1_000_000usize;
        sched.drain(&mut stop_all);
        while sched.has_pending() && guard > 0 {
            sched.drain(&mut stop_all);
            guard -= 1;
        }
        assert!(guard > 0, "scheduler failed to converge");
        assert!(!sched.has_pending(), "tasks left pending");
    }

    /// One drain turn: runs until the scheduler stalls (parked tasks with
    /// no possible wake) — for park-then-act scenarios
    fn drain_to_stall(sched: &mut AsyncSched) {
        sched.drain(&mut stop_all);
    }

    fn stop_all(_s: &AsyncSched) -> bool {
        false
    }

    fn result_value(sched: &mut AsyncSched, id: i64) -> Value {
        let (sv, _notes) = sched
            .take_finished(id)
            .unwrap_or_else(|| panic!("task {id} not finished"));
        crate::genes::from_send(sv)
    }

    #[test]
    fn sleepers_wake_in_deadline_order() {
        let mut sched = AsyncSched::new();
        // spawn order A(300) B(100) C(200): wake order must be B C A
        sched
            .ready
            .push_back(task(1, "a", "gene w() {\n sleep(300)\n return 1\n}\n", "w"));
        sched
            .ready
            .push_back(task(2, "b", "gene w() {\n sleep(100)\n return 2\n}\n", "w"));
        sched
            .ready
            .push_back(task(3, "c", "gene w() {\n sleep(200)\n return 3\n}\n", "w"));
        let t0 = Instant::now();
        run_to_end(&mut sched);
        // virtual clock: jumped to the last deadline (300), ~zero wall time
        assert_eq!(sched.now_ms, 300);
        assert!(
            t0.elapsed().as_millis() < 2_000,
            "the virtual clock leaked into wall time"
        );
        // completion order IS the deadline order (finish pushes FIFO)
        let order: Vec<i64> = sched.finished.iter().map(|t| t.id).collect();
        assert_eq!(order, vec![2, 3, 1]);
        assert_eq!(result_value(&mut sched, 2).display(), "2");
        assert_eq!(result_value(&mut sched, 1).display(), "1");
    }

    #[test]
    fn sleep_ties_wake_in_park_order() {
        let mut sched = AsyncSched::new();
        for (id, ms) in [(1i64, 100i64), (2, 100), (3, 100)] {
            let src = format!("gene w() {{\n sleep({ms})\n return {id}\n}}\n");
            sched.ready.push_back(task(id, "w", &src, "w"));
        }
        run_to_end(&mut sched);
        let order: Vec<i64> = sched.finished.iter().map(|t| t.id).collect();
        assert_eq!(order, vec![1, 2, 3], "FIFO tie-break broken");
    }

    #[test]
    fn chan_recv_parks_then_wakes_fifo() {
        let ch = Arc::new(crate::value::ChannelShared::new());
        let mut sched = AsyncSched::new();
        // two receivers parked on the same channel, one item on it:
        // the FIRST-parked receiver gets it
        let src_b = "gene w() {\n return recv(ch)\n}\n";
        let mut tb = task(1, "b", src_b, "w");
        tb.interp.global.define("ch", Value::Channel(ch.clone()));
        let g = tb.interp.global.clone();
        tb.fiber = match fiber_call_begin(&mut tb.interp, &g, "w", vec![]) {
            Ok(FiberBegin::Fiber(f)) => f,
            _ => panic!("b begin must reach a body"),
        };
        let mut tc = task(2, "c", src_b, "w");
        tc.interp.global.define("ch", Value::Channel(ch.clone()));
        let g = tc.interp.global.clone();
        tc.fiber = match fiber_call_begin(&mut tc.interp, &g, "w", vec![]) {
            Ok(FiberBegin::Fiber(f)) => f,
            _ => panic!("c begin must reach a body"),
        };
        sched.ready.push_back(tb);
        sched.ready.push_back(tc);
        drain_to_stall(&mut sched);
        // both parked on the empty channel; nothing delivered
        assert_eq!(sched.chan_parks.len(), 2, "both receivers must be parked");
        // a send arrives: first-parked (b) wins, c stays parked
        let mut st = ch.state.lock().unwrap_or_else(|e| e.into_inner());
        st.queue.push_back(SendValue::Int(42));
        drop(st);
        sched
            .wake_reqs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(WakeReq::Sent(ch.clone()));
        drain_to_stall(&mut sched);
        assert_eq!(result_value(&mut sched, 1).display(), "42");
        assert!(sched.is_live(2), "c must still be parked");
        // close delivers the closed+empty null to the remaining waiter
        {
            let mut st = ch.state.lock().unwrap_or_else(|e| e.into_inner());
            st.closed = true;
        }
        sched
            .wake_reqs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(WakeReq::Closed(ch.clone()));
        drain_to_stall(&mut sched);
        assert_eq!(result_value(&mut sched, 2).display(), "null");
    }

    #[test]
    fn cancel_unblocks_a_parked_receiver() {
        let ch = Arc::new(crate::value::ChannelShared::new());
        let mut sched = AsyncSched::new();
        let src = "gene w() {\n return recv(ch)\n}\n";
        let mut tb = task(1, "b", src, "w");
        tb.interp.global.define("ch", Value::Channel(ch.clone()));
        let g = tb.interp.global.clone();
        tb.fiber = match fiber_call_begin(&mut tb.interp, &g, "w", vec![]) {
            Ok(FiberBegin::Fiber(f)) => f,
            _ => panic!("begin must reach a body"),
        };
        sched.ready.push_back(tb);
        drain_to_stall(&mut sched);
        assert_eq!(sched.chan_parks.len(), 1);
        // cancel: the parked cell unblocks and fails with the cancelled
        // stress, phase Cancelled (the thread lane's per-slice contract)
        sched.chan_parks[0].0.cancel.store(true, Ordering::Relaxed);
        run_to_end(&mut sched);
        let (sv, _notes) = sched.take_finished(1).expect("task finished");
        match sv {
            SendValue::Stress(k, m) => {
                assert_eq!(k, "cancelled");
                assert_eq!(m, "task cancelled");
            }
            SendValue::Int(i) => panic!("expected a cancelled stress, got int {}", i),
            SendValue::Null => panic!("expected a cancelled stress, got null"),
            SendValue::Str(s) => panic!("expected a cancelled stress, got str {:?}", s),
            SendValue::Bool(b) => panic!("expected a cancelled stress, got bool {}", b),
            SendValue::Float(f) => panic!("expected a cancelled stress, got float {}", f),
            SendValue::Bytes(_)
            | SendValue::List(_)
            | SendValue::Map(_)
            | SendValue::Variant(_, _) => {
                panic!("expected a cancelled stress, got a container value")
            }
        }
    }

    #[test]
    fn parked_forever_stalls_without_stealing_wall_time() {
        let ch = Arc::new(crate::value::ChannelShared::new());
        let mut sched = AsyncSched::new();
        let src = "gene w() {\n return recv(ch)\n}\n";
        let mut tb = task(1, "b", src, "w");
        tb.interp.global.define("ch", Value::Channel(ch.clone()));
        let g = tb.interp.global.clone();
        tb.fiber = match fiber_call_begin(&mut tb.interp, &g, "w", vec![]) {
            Ok(FiberBegin::Fiber(f)) => f,
            _ => panic!("begin must reach a body"),
        };
        sched.ready.push_back(tb);
        let t0 = Instant::now();
        sched.drain(&mut stop_all);
        // the drain STALLS (nothing can wake the receiver) and returns
        // control instead of blocking the host thread
        assert!(t0.elapsed().as_millis() < 100);
        assert!(sched.has_pending());
        assert!(sched.finished.is_empty());
    }

    #[test]
    fn virtual_slices_charge_parked_receivers() {
        // a sleeper advances the virtual clock; the parked receiver is
        // charged 1000 steps per virtual ms, the thread lane's exact rate
        let ch = Arc::new(crate::value::ChannelShared::new());
        let mut sched = AsyncSched::new();
        let src_r = "gene w() {\n return recv(ch)\n}\n";
        let mut tr = task(1, "r", src_r, "w");
        tr.interp.global.define("ch", Value::Channel(ch.clone()));
        let g = tr.interp.global.clone();
        tr.fiber = match fiber_call_begin(&mut tr.interp, &g, "w", vec![]) {
            Ok(FiberBegin::Fiber(f)) => f,
            _ => panic!("begin must reach a body"),
        };
        let pool = Arc::new(std::sync::atomic::AtomicI64::new(1_000_000_000));
        tr.interp.fuel_pool = Some(pool.clone());
        sched.ready.push_back(tr);
        sched
            .ready
            .push_back(task(2, "s", "gene w() {\n sleep(250)\n return 9\n}\n", "w"));
        drain_to_stall(&mut sched);
        assert_eq!(sched.now_ms, 250);
        // the receiver experienced the full 250 virtual ms
        let charged = 1_000_000_000 - pool.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(charged, 250_000, "parked slice charging drifted");
        assert_eq!(result_value(&mut sched, 2).display(), "9");
        assert!(
            sched.is_live(1),
            "receiver still parked after the send-free wake"
        );
    }

    #[test]
    fn select_wakes_leftmost_ready_or_all_closed() {
        let c0 = Arc::new(crate::value::ChannelShared::new());
        let c1 = Arc::new(crate::value::ChannelShared::new());
        let mut sched = AsyncSched::new();
        let src = "gene w() {\n return select(ch0, ch1)\n}\n";
        let mut ts = task(1, "s", src, "w");
        ts.interp.global.define("ch0", Value::Channel(c0.clone()));
        ts.interp.global.define("ch1", Value::Channel(c1.clone()));
        let g = ts.interp.global.clone();
        ts.fiber = match fiber_call_begin(&mut ts.interp, &g, "w", vec![]) {
            Ok(FiberBegin::Fiber(f)) => f,
            _ => panic!("begin must reach a body"),
        };
        sched.ready.push_back(ts);
        drain_to_stall(&mut sched);
        assert!(
            sched.chan_parks.len() == 1 && sched.chan_parks[0].2,
            "select parked"
        );
        // item lands on ch1 only: select must still answer 1 (leftmost
        // READY wins — ch0 is empty, ch1 is ready)
        {
            let mut st = c1.state.lock().unwrap_or_else(|e| e.into_inner());
            st.queue.push_back(SendValue::Int(7));
        }
        sched
            .wake_reqs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(WakeReq::Sent(c1.clone()));
        drain_to_stall(&mut sched);
        assert_eq!(result_value(&mut sched, 1).display(), "1");
        // all closed answers -1 (fresh channels: stage 1 left its item
        // queued on c1 — select never consumes)
        let c2 = Arc::new(crate::value::ChannelShared::new());
        let c3 = Arc::new(crate::value::ChannelShared::new());
        let mut sched2 = AsyncSched::new();
        let mut ts2 = task(2, "s", src, "w");
        ts2.interp.global.define("ch0", Value::Channel(c2.clone()));
        ts2.interp.global.define("ch1", Value::Channel(c3.clone()));
        let g = ts2.interp.global.clone();
        ts2.fiber = match fiber_call_begin(&mut ts2.interp, &g, "w", vec![]) {
            Ok(FiberBegin::Fiber(f)) => f,
            _ => panic!("begin must reach a body"),
        };
        sched2.ready.push_back(ts2);
        drain_to_stall(&mut sched2);
        {
            let mut st = c2.state.lock().unwrap_or_else(|e| e.into_inner());
            st.closed = true;
            let mut st1 = c3.state.lock().unwrap_or_else(|e| e.into_inner());
            st1.closed = true;
        }
        sched2
            .wake_reqs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(WakeReq::Closed(c2.clone()));
        sched2
            .wake_reqs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(WakeReq::Closed(c3.clone()));
        drain_to_stall(&mut sched2);
        assert_eq!(result_value(&mut sched2, 2).display(), "-1");
    }

    #[test]
    fn task_notes_come_back_prefixed() {
        let mut sched = AsyncSched::new();
        sched.ready.push_back(task(
            1,
            "worker-a",
            "gene w() {\n let x = nope\n return 1\n}\n",
            "w",
        ));
        run_to_end(&mut sched);
        let (_sv, notes) = sched.take_finished(1).expect("finished");
        assert!(notes.iter().any(|n| n.message.contains("[task worker-a]")));
    }
}
