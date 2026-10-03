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
}
