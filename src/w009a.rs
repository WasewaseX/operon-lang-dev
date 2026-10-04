//! W009-A SCRATCH instrumentation — investigation evidence only (builder-A).
//! NOT part of the language contract. Completely inert unless the env vars
//! OPERON_W009A_ABLATE / OPERON_W009A_COUNTS are set; every call site costs
//! one AtomicBool load (sub-ns) when the harness is off.
//!
//! Ablation flags (comma list in OPERON_W009A_ABLATE):
//!   tick   — skip the per-instruction fuel tick() in the VM dispatch loop
//!   tb     — lazy traceback frame (no happy-path String clone in call_gene)
//!   promo  — promoter_veto early-return BEFORE the per-call name clone
//!   bk     — skip bump_call_bookkeeping entirely (counters/buckets/clock)
//!   decay  — skip only the m6a/grn decay tickers inside bookkeeping
//!   gates  — skip the whole regulatory gate block in call_gene_inner
//!   pool   — recycle frame-Env HashMap/HashSet capacity via a free-list
//!   all    — every flag above
//! Counters (OPERON_W009A_COUNTS=1) print to stderr when finish() runs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

pub static A_TICK: AtomicBool = AtomicBool::new(false);
pub static A_TB: AtomicBool = AtomicBool::new(false);
pub static A_PROMO: AtomicBool = AtomicBool::new(false);
pub static A_BK: AtomicBool = AtomicBool::new(false);
pub static A_DECAY: AtomicBool = AtomicBool::new(false);
pub static A_GATES: AtomicBool = AtomicBool::new(false);
pub static A_POOL: AtomicBool = AtomicBool::new(false);
pub static COUNTS_ON: AtomicBool = AtomicBool::new(false);

pub static C_INSTRS: AtomicU64 = AtomicU64::new(0);
pub static C_TICKS: AtomicU64 = AtomicU64::new(0);
pub static C_ENVNEW: AtomicU64 = AtomicU64::new(0);
pub static C_BK: AtomicU64 = AtomicU64::new(0);
pub static C_NAMES: AtomicU64 = AtomicU64::new(0);
pub static C_TB: AtomicU64 = AtomicU64::new(0);
pub static C_PROMO: AtomicU64 = AtomicU64::new(0);
pub static C_MONO: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static POOL_VARS: RefCell<Vec<HashMap<String, crate::value::Value>>,
    > = const { RefCell::new(Vec::new()) };
    static POOL_CONSTS: RefCell<Vec<std::collections::HashSet<String>>> =
        const { RefCell::new(Vec::new()) };
}

pub fn env_parts() -> (
    HashMap<String, crate::value::Value>,
    std::collections::HashSet<String>,
) {
    let vars = POOL_VARS.with(|p| p.borrow_mut().pop()).unwrap_or_default();
    let consts = POOL_CONSTS
        .with(|p| p.borrow_mut().pop())
        .unwrap_or_default();
    (vars, consts)
}

pub fn recycle(
    mut vars: HashMap<String, crate::value::Value>,
    mut consts: std::collections::HashSet<String>,
) {
    // clear PRESERVES capacity — the pooling win — while dropping the dead
    // frame's bindings; without this, block-scope envs pop stale maps and
    // satisfy name reads locally instead of walking the parent chain
    // (the fib(25)=1 failure, fixed in-scope).
    vars.clear();
    consts.clear();
    POOL_VARS.with(|p| {
        let mut q = p.borrow_mut();
        if q.len() < 4096 {
            q.push(vars);
        }
    });
    POOL_CONSTS.with(|p| {
        let mut q = p.borrow_mut();
        if q.len() < 4096 {
            q.push(consts);
        }
    });
}

fn print_counters() {
    eprintln!(
        "w009a counters: instrs={} ticks={} env_new={} bookkeep={} name_clones={} tb_clones={} promo_clones={} mono_hits={} slot_frames={}",
        C_INSTRS.load(Relaxed),
        C_TICKS.load(Relaxed),
        C_ENVNEW.load(Relaxed),
        C_BK.load(Relaxed),
        C_NAMES.load(Relaxed),
        C_TB.load(Relaxed),
        C_PROMO.load(Relaxed),
        C_MONO.load(Relaxed),
        crate::vm::SLOT_FRAMES.load(Relaxed),
    );
}

pub struct Guard;
impl Guard {
    pub fn install() {
        init_from_env();
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if COUNTS_ON.load(Relaxed) {
            print_counters();
        }
    }
}

pub fn init_from_env() {
    let spec = std::env::var("OPERON_W009A_ABLATE").unwrap_or_default();
    let set = |s: &str| spec.split(',').any(|f| f.trim() == s);
    let any = !spec.is_empty();
    A_TICK.store(any && (set("tick") || set("all")), Relaxed);
    A_TB.store(any && (set("tb") || set("all")), Relaxed);
    A_PROMO.store(any && (set("promo") || set("all")), Relaxed);
    A_BK.store(any && (set("bk") || set("all")), Relaxed);
    A_DECAY.store(any && (set("decay") || set("all")), Relaxed);
    A_GATES.store(any && (set("gates") || set("all")), Relaxed);
    A_POOL.store(any && (set("pool") || set("all")), Relaxed);
    let counts = std::env::var("OPERON_W009A_COUNTS").as_deref() == Ok("1");
    COUNTS_ON.store(counts, Relaxed);
    if any || counts {
        eprintln!("w009a harness: ablate=[{}] counts={}", spec, counts);
    }
    envbench();
}

/// W011-s3 measurement (evidence-only, W009-A instrument module): time the
/// EXACT per-call frame-construction shape of the stage-2 slot path —
/// `Env::new(parent)` + N `define_param` inserts + the slot Vec + drops —
/// so the stage-3 elimination ceiling is a measured number, not an estimate.
/// Inert unless OPERON_W009A_ENVBENCH=<iters> is set.
pub fn envbench() {
    let iters: u64 = match std::env::var("OPERON_W009A_ENVBENCH") {
        Ok(s) => s.parse().unwrap_or(0),
        Err(_) => 0,
    };
    if iters == 0 {
        return;
    }
    // realistic parent: a global env with a handful of bindings (fib's
    // global carries the gene binding + shadow names)
    let parent = crate::interp::Env::new(None);
    for i in 0..16 {
        parent.define_param(&format!("g{}", i), crate::value::Value::Int(i as i64));
    }
    // warm the allocator + branch predictors (both shapes)
    for _ in 0..10_000 {
        frame_shape(&parent, 1);
        frame_shape(&parent, 2);
    }
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        frame_shape(&parent, 1);
    }
    let d1 = t0.elapsed().as_nanos() / iters as u128;
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        frame_shape(&parent, 2);
    }
    let d2 = t0.elapsed().as_nanos() / iters as u128;
    eprintln!("w009a envbench: 1-param frame {d1} ns/op, 2-param frame {d2} ns/op (iters={iters})");
}

/// The exact stage-2 per-call frame shape for a params-long frame: fresh
/// Env (write-through copy), param inserts, slot Vec, then both dropped
/// (the real path drops the frame env at call end; the slot Vec drops with
/// the frame).
#[inline(never)]
fn frame_shape(parent: &std::rc::Rc<crate::interp::Env>, params: usize) {
    let fenv = crate::interp::Env::new(Some(parent.clone()));
    let mut slots = vec![crate::value::Value::Null; params];
    for (i, s) in slots.iter_mut().enumerate() {
        let v = crate::value::Value::Int(i as i64);
        *s = v.clone();
        fenv.define_param("n", v);
    }
    drop(slots);
    drop(fenv);
}
