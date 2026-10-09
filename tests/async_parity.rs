//! async_parity.rs, W16 (async epic): the two spawn lanes must be
//! indistinguishable. docs/specs/ASYNC.md: "under `.cell io.pool` the
//! spawned task may run as a fiber on the VM loop instead of a thread.
//! Callers cannot tell, determinism and the snapshot membrane are
//! identical by test, not by promise."
//!
//! Every test runs the SAME program twice — `io.pool = fiber` (the W16
//! scheduler, virtual clock) and `io.pool = thread` (the historical OS
//! threads) — and requires identical captured stdout, identical notes and
//! identical task-lifecycle observables. Programs are join-ordered so the
//! thread lane is deterministic; fiber-only determinism (FIFO wake order)
//! gets its own repeat-stability test.

use operon::interp::{Env, Interp};
use operon::vm::VmProgram;
use std::cell::RefCell;
use std::rc::Rc;

struct Run {
    out: Vec<String>,
    notes: Vec<String>,
    main_err: Option<(String, String)>, // kind, message
}

/// Run a program's main() on one lane. `cell_value` is the io.pool value;
/// `fuel` optionally pins the shared run-wide pool (the loop-5 law).
fn run_lane(src: &str, cell_value: &str, fuel: Option<i64>) -> Run {
    let parsed = operon::parser::parse(src);
    let mut interp = Interp::new();
    interp.vm = true;
    interp.vm_program = Some(VmProgram::default());
    interp
        .cell
        .insert("io.pool".to_string(), cell_value.to_string());
    if let Some(steps) = fuel {
        interp.fuel_pool = Some(std::sync::Arc::new(std::sync::atomic::AtomicI64::new(
            steps,
        )));
    }
    let sink = Rc::new(RefCell::new(Vec::new()));
    interp.stdout_sink = Some(sink.clone());
    for s in &parsed.stmts {
        let g = interp.global.clone();
        let _ = interp.exec_stmt(&g, s);
    }
    let g: Rc<Env> = interp.global.clone();
    let main_result = interp.named_call_tail_vm(&g, "main", vec![], None);
    let main_err = match main_result {
        Ok(_) => None,
        Err(s) => Some((s.kind, s.message)),
    };
    let out = sink.borrow().clone();
    Run {
        out,
        notes: interp.notes.iter().map(|n| n.message.clone()).collect(),
        main_err,
    }
}

fn assert_lanes_agree(src: &str, fuel: Option<i64>) -> (Run, Run) {
    let f = run_lane(src, "fiber", fuel);
    let t = run_lane(src, "thread", fuel);
    assert_eq!(f.out, t.out, "stdout diverged between lanes");
    assert_eq!(f.notes, t.notes, "notes diverged between lanes");
    assert_eq!(
        f.main_err.as_ref().map(|e| &e.0),
        t.main_err.as_ref().map(|e| &e.0),
        "main stress kinds diverged"
    );
    (f, t)
}

const JOIN_ORDERED: &str = r#"
gene sleeper_a() {
    sleep(40)
    return 1
}
gene sleeper_b() {
    sleep(10)
    return 2
}
gene relay(src, dst) {
    let x = recv(src)
    send(dst, x + 1)
    return x
}
gene tail(ch) {
    return recv(ch)
}
gene counter(ch) {
    let x = recv(ch)
    return x * 10
}
gene main() {
    let a = spawn(sleeper_a, [])
    let b = spawn(sleeper_b, [])
    let ch1 = channel()
    let ch2 = channel()
    let c = spawn(relay, [ch1, ch2])
    let d = spawn(tail, [ch2])
    send(ch1, 5)
    let ch3 = channel()
    let e = spawn(counter, [ch3])
    send(ch3, 7)
    promote("b={join(b)} a={join(a)} d={join(d)} c={join(c)} e={join(e)}")
    promote("state a={task_state(a)} b={task_state(b)}")
}
"#;

#[test]
fn join_ordered_program_is_byte_identical_across_lanes() {
    let (f, _t) = assert_lanes_agree(JOIN_ORDERED, None);
    // the pinned values: joins return through the same wire on both lanes
    assert_eq!(f.out, vec!["b=2 a=1 d=6 c=5 e=70", "state a=done b=done"]);
    // the virtual clock means the fiber lane must be effectively instant;
    // nothing to assert structurally here — a real wall sleep would show
    // up as a 50ms+ test, the suite runs in microseconds
}

#[test]
fn fiber_lane_join_order_is_instant() {
    // 60 fibers sleeping 60s (the builtin cap) each: the virtual clock
    // finishes them without wall time. The thread lane would need real
    // minutes, so this lane-only test pins the virtual clock instead.
    let src = r#"
gene hog() {
    sleep(60000)
    return 1
}
gene main() {
    let ids = []
    for i in range(0, 60) {
        push(ids, spawn(hog, []))
    }
    let acc = 0
    for id in ids {
        acc += join(id)
    }
    promote("acc={acc}")
}
"#;
    let t0 = std::time::Instant::now();
    let f = run_lane(src, "fiber", None);
    assert!(
        t0.elapsed().as_secs() < 10,
        "virtual clock leaked wall time"
    );
    assert_eq!(f.out, vec!["acc=60"]);
}

#[test]
fn cancel_lands_on_both_lanes() {
    let src = r#"
gene victim() {
    while true {
        let x = 1
    }
    return 0
}
gene main() {
    let a = spawn(victim, [])
    cancel(a)
    let v = join(a)
    promote("state={task_state(a)}")
}
"#;
    let (f, t) = assert_lanes_agree(src, None);
    // W18 contract on both lanes: the cancelled-run task names its phase
    assert_eq!(f.out, vec!["state=cancelled"]);
    assert_eq!(t.out, vec!["state=cancelled"]);
}

#[test]
fn fuel_exhaustion_names_the_same_stress_on_both_lanes() {
    let src = r#"
gene hog() {
    sleep(60000)
    return 1
}
gene main() {
    let a = spawn(hog, [])
    promote("j={join(a)}")
    promote("state={task_state(a)}")
}
"#;
    // 60_000 ms * 1000 steps/ms = 60M steps for one sleep; the pool is
    // smaller, so the worker dies with the run-wide overflow stress
    let (f, t) = assert_lanes_agree(src, Some(1_000_000));
    assert_eq!(f.out, t.out);
    // a task that died by the fuel contract still finished (the W18 rule:
    // only a cancelled-run stress names the Cancelled phase), and the
    // stress crosses the wire as the {kind, message} map on BOTH lanes —
    // the same value, the same text ("run-wide step budget exhausted
    // (sleep)"), byte-identical
    assert_eq!(
        f.out,
        vec![
            "j={kind: \"overflow\", message: \"run-wide step budget exhausted (sleep)\"}",
            "state=done"
        ]
    );
}

#[test]
fn membrane_refuses_weak_payload_on_both_lanes() {
    let src = r#"
gene taker(w) {
    return 1
}
gene main() {
    let m = {k: [1, 2, 3]}
    let w = weak(m)
    let kind = "none"
    stress {
        let t = spawn(taker, [w])
        promote("spawned {t}")
    } rescue (e) {
        kind = e.kind
    }
    promote("kind={kind}")
}
"#;
    // spawn refuses the weak payload with the catchable membrane stress;
    // the rescue observes the SAME kind on both lanes
    let r = run_lane(src, "fiber", None);
    assert_eq!(
        r.out,
        vec!["kind=membrane"],
        "membrane refusal shape (fiber)"
    );
    let t = run_lane(src, "thread", None);
    assert_eq!(
        t.out,
        vec!["kind=membrane"],
        "membrane refusal shape (thread)"
    );
}

#[test]
fn scope_reaps_spawned_tasks_on_both_lanes() {
    let src = r#"
gene quick() {
    sleep(5)
    return 9
}
gene main() {
    let v = 0
    scope {
        let a = spawn(quick, [])
        v = join(a)
    }
    promote("v={v}")
}
"#;
    let (f, _t) = assert_lanes_agree(src, None);
    assert_eq!(f.out, vec!["v=9"]);
}

#[test]
fn fiber_lane_wake_order_is_stable_across_repeats() {
    // FIFO determinism pin (fiber lane only): two sleepers, the earlier
    // deadline finishes first; three repeats must be identical
    let src = r#"
gene s(n, ms) {
    sleep(ms)
    return n
}
gene main() {
    let b = spawn(s, [1, 5])
    let a = spawn(s, [2, 50])
    let c = spawn(s, [3, 20])
    promote("{join(b)}{join(c)}{join(a)}")
}
"#;
    let f1 = run_lane(src, "fiber", None);
    let f2 = run_lane(src, "fiber", None);
    assert_eq!(f1.out, vec!["132"]);
    assert_eq!(f1.out, f2.out, "wake order is not stable");
    assert_eq!(f1.notes, f2.notes);
}

#[test]
fn unbound_reads_inside_fibers_match_worker_notes() {
    // the note stream is output: a fiber task's notes must arrive with the
    // same [task <name>] prefix and the same text as a thread worker's
    let src = r#"
gene sloppy() {
    let x = nope
    return 1
}
gene main() {
    let a = spawn(sloppy, [])
    promote("j={join(a)}")
}
"#;
    let (f, t) = assert_lanes_agree(src, None);
    assert_eq!(f.out, t.out);
    assert!(
        f.notes
            .iter()
            .any(|n| n.starts_with("[task sloppy]") && n.contains("unbound 'nope'")),
        "fiber task notes lost the worker prefix: {:?}",
        f.notes
    );
}

/// Z-120 (#120): a stale `WakeReq::Sent` must never wake a parked waiter
/// whose item was already consumed by a RUNNING fiber's direct recv — that
/// wake's recv_answer popped an EMPTY OPEN channel and delivered null,
/// breaking the pinned invariant (closed+empty is the ONLY null recv can
/// produce). The scheduler now drops a Sent whose queue is empty at
/// processing time; the starved waiter surfaces the deterministic
/// join-timeout note instead. Fiber-lane assertions only: the thread lane
/// hands the same send to the parked consumer via the condvar (documented
/// "different scheduling") and its producer then blocks on the condvar
/// with no further wake source, so the program is inherently
/// scheduling-dependent ACROSS lanes — the invariant under test (no
/// recv-null from an open channel) holds on BOTH.
const STALE_SENT: &str = r#"
gene consumer(ch) {
    let v = recv(ch)
    if v == null { print("BUG: consumer got null on an OPEN channel") }
    else { print("consumer ok:", v) }
    return v
}
gene producer(ch) {
    send(ch, 42)
    let echo = recv(ch)
    print("producer echo:", echo)
    return echo
}
gene main() {
    let ch = channel()
    let c = spawn(consumer, [ch])
    let p = spawn(producer, [ch])
    print("joined consumer:", join(c), "producer:", join(p))
    return 0
}
"#;

#[test]
fn stale_sent_wake_never_delivers_null_from_an_open_channel() {
    let f = run_lane(STALE_SENT, "fiber", None);
    let joined = f.out.join("\n");
    assert!(
        !joined.contains("BUG: consumer got null"),
        "recv delivered null from an OPEN channel (stale WakeReq::Sent wake): {joined}"
    );
    assert!(
        joined.contains("producer: 42"),
        "the producer's running recv must keep its direct-pop (join sees the echoed 42): {joined}"
    );
    assert!(
        f.notes
            .iter()
            .any(|n| n.contains("join timeout") && n.contains("task 1")),
        "the starved waiter must surface the deterministic join-timeout note, not a fake recv null: {:?}",
        f.notes
    );
    assert!(
        f.main_err.is_none(),
        "main must not stress: {:?}",
        f.main_err
    );
}
