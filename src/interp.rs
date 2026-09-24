//! interp.rs — the tree-walking evaluator. Total Grammar at runtime:
//! soft failures become notes; only catchable Stress propagates.

use crate::ast::*;
use crate::value::{key_scalar, SeqState, Stress, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{mpsc, Arc};

pub struct Env {
    pub vars: RefCell<HashMap<String, Value>>,
    pub parent: Option<Rc<Env>>,
}

impl Env {
    pub fn new(parent: Option<Rc<Env>>) -> Rc<Env> {
        Rc::new(Env {
            vars: RefCell::new(HashMap::new()),
            parent,
        })
    }
    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.vars.borrow().get(name) {
            return Some(v.clone());
        }
        let mut node = self.parent.clone();
        while let Some(env) = node {
            if let Some(v) = env.vars.borrow().get(name) {
                return Some(v.clone());
            }
            node = env.parent.clone();
        }
        None
    }
    /// Assign: rebind where found, else define in this (current) scope.
    pub fn set(&self, name: &str, val: Value) -> bool {
        if self.vars.borrow().contains_key(name) {
            self.vars.borrow_mut().insert(name.to_string(), val);
            return true;
        }
        let mut node = self.parent.clone();
        while let Some(env) = node {
            if env.vars.borrow().contains_key(name) {
                env.vars.borrow_mut().insert(name.to_string(), val);
                return true;
            }
            node = env.parent.clone();
        }
        self.vars.borrow_mut().insert(name.to_string(), val);
        false
    }
    pub fn define(&self, name: &str, val: Value) {
        self.vars.borrow_mut().insert(name.to_string(), val);
    }
}

#[derive(Clone)]
pub enum Flow {
    Norm,
    Ret(Value),
    Brk,
    Cont,
}

pub struct TaskHandle {
    pub rx: mpsc::Receiver<(crate::genes::SendValue, Vec<Note>)>,
}

/// Capability grants (default-deny for I/O, processes, sockets, env).
/// "Safer than Rust by default": an Operon program can touch nothing unless
/// the host explicitly grants it. Violations raise catchable `interference`
/// stress (RNA interference — the cell's antiviral silencing response).
#[derive(Clone)]
pub struct Caps {
    pub enabled: bool,
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub run: Vec<String>,
    pub net: Vec<String>,
    pub env: Vec<String>,
    /// sec-r2 (audit C-11): exit() kills the whole host process — in test
    /// runners, the REPL, the LSP, or any embedded host that is fatal. So
    /// it is a capability like any other, default-deny.
    pub exit_allowed: bool,
}

impl Default for Caps {
    fn default() -> Caps {
        // secure by default: enabled (denying) with zero grants
        Caps {
            enabled: true,
            read: Vec::new(),
            write: Vec::new(),
            run: Vec::new(),
            net: Vec::new(),
            env: Vec::new(),
            exit_allowed: false,
        }
    }
}

impl Caps {
    pub fn allow_all() -> Caps {
        Caps {
            enabled: false,
            exit_allowed: true,
            ..Default::default()
        }
    }
    pub fn denied(kind: &str, what: &str) -> Stress {
        Stress::new(
            "interference",
            format!("{} denied — no capability grant covers '{}' (grant with --allow-{} or --allow-all)", kind, what, kind),
        )
    }
    /// Lexical path normalization (no filesystem access — pure string math).
    fn norm_path(p: &str) -> String {
        let mut out: Vec<&str> = Vec::new();
        for seg in p.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                s => out.push(s),
            }
        }
        out.join("/")
    }
    /// A grant that normalizes to the empty string would match everything —
    /// reject it instead (granting "." from /, or "/", must not mean the
    /// whole filesystem).
    pub fn validate_grant(kind: &str, g: &str) -> Result<(), Stress> {
        let ng = Self::norm_path(g);
        if ng.is_empty() {
            return Err(Stress::new(
                "interference",
                format!("capability grant '{}' for {} is empty after normalization; grant a concrete directory or file", g, kind),
            ));
        }
        Ok(())
    }
    pub fn add_grant(&mut self, kind: &str, g: &str) -> Result<(), Stress> {
        Self::validate_grant(kind, g)?;
        let list = match kind {
            "read" => &mut self.read,
            "write" => &mut self.write,
            "run" => &mut self.run,
            "net" => &mut self.net,
            _ => &mut self.env,
        };
        list.push(g.to_string());
        Ok(())
    }
    /// Resolve what the path ACTUALLY is on disk (symlinks included) and
    /// compare against the resolved grant. Falls back to lexical comparison
    /// when the target does not exist (e.g. a file about to be created).
    ///
    /// STRICT-RULE (sandbox): if the path EXISTS, its fully-resolved form
    /// must be inside a grant — no parent-chain probing. A symlink placed
    /// inside a granted directory that points outside the sandbox resolves
    /// to its target and is rejected even though the link's parent is
    /// granted (Critic-X B1).
    fn path_allowed(list: &[String], path: &str) -> bool {
        let resolved_requested = std::fs::canonicalize(path)
            .ok()
            .map(|p: std::path::PathBuf| p.to_string_lossy().replace('\\', "/"));
        if let Some(rp) = &resolved_requested {
            // existing target: strict resolved-prefix match only
            let rp_str = rp.as_str().replace('\\', "/");
            for g in list {
                if let Ok(rg) = std::fs::canonicalize(g) {
                    let rg_str = rg.to_string_lossy().replace('\\', "/");
                    if rp_str == rg_str || rp_str.starts_with(&format!("{}/", rg_str)) {
                        return true;
                    }
                }
            }
            return false;
        }
        // non-existent target (create/write case): its parent chain
        // must resolve inside the grant
        for g in list {
            match std::fs::canonicalize(g) {
                Ok(rg) => {
                    let rg_str = rg.to_string_lossy().replace('\\', "/");
                    let mut probe = std::path::PathBuf::from(path);
                    while probe.pop() {
                        if let Ok(pp) = std::fs::canonicalize(&probe) {
                            let pp_str = pp.to_string_lossy().replace('\\', "/");
                            if pp_str == rg_str || pp_str.starts_with(&format!("{}/", rg_str)) {
                                return true;
                            }
                            break; // nearest existing ancestor checked
                        }
                    }
                }
                Err(_) => {
                    // lexical fallback ONLY for grants that do not exist on
                    // disk. A grant that exists but failed to cover an
                    // existing canonicalized target must never be re-admitted
                    // lexically — that is the symlink-escape hole (S4 NEW-1).
                    let np = Self::norm_path(path);
                    let ng = Self::norm_path(g);
                    if !ng.is_empty() && (np == ng || np.starts_with(&format!("{}/", ng))) {
                        return true;
                    }
                }
            }
        }
        false
    }
    pub fn check(&self, list: &[String], kind: &str, what: &str) -> Result<(), Stress> {
        if !self.enabled || list.iter().any(|g| g == "*") {
            return Ok(());
        }
        let ok = if kind == "read" || kind == "write" {
            Self::path_allowed(list, what)
        } else {
            list.iter().any(|g| g == what)
        };
        if ok {
            Ok(())
        } else {
            Err(Self::denied(kind, what))
        }
    }
}

/// T2e: super-enhancer activation boost — an `enhance`d gene lowers its
/// GRN activating thresholds by this much (0.25), so under the T2a call
/// gate an enhanced gene demonstrably fires where an unenhanced one would
/// stay gated. Only meaningful on edges that carry an explicit threshold.
pub const ENHANCE_DELTA: f64 = 0.25;

pub struct Interp {
    pub notes: Vec<Note>,
    /// sec-r3: notes suppressed past the cap (surfaced once).
    pub notes_dropped: u64,
    pub cell: HashMap<String, String>,
    pub cell_entry: Option<String>,
    pub base_dir: Option<String>,
    /// A13 (dx-r2): line of the call expression currently executing —
    /// builtin diagnostics stamp this instead of 0.
    pub cur_line: usize,
    /// A13 (dx-r2): source file name for diagnostic rendering.
    pub file: String,
    /// dx-r3 (re-audit / A14 leftover): when set, promote() writes here
    /// instead of process stdout — the test runner captures per-file
    /// program output and shows it only on failure (clean reports).
    pub stdout_sink: Option<Rc<RefCell<Vec<String>>>>,
    pub silences: Vec<(String, String)>,
    pub fates: HashMap<String, Arc<FateDef>>,
    pub phenos: HashMap<String, Arc<PhenoDef>>,
    pub grn_edges: Vec<RegEdge>,
    pub grn_levels: HashMap<String, f64>,
    pub toggles: Vec<(String, String, bool)>, // (a, b, a_on) — mutual repression pair
    pub repressi_ring: Vec<String>,
    pub repressi_i: usize,
    /// T2d: manual-mode tick counter — drives the level oscillation formula.
    pub repressi_tick: u64,
    pub repressi_atomic: Option<Arc<AtomicU64>>,
    /// sec-r3: cancellable timer flag — repressi_start/repressilator share
    /// ONE timer thread; the previous one is flagged off before a new spawn
    /// (no permanent thread-budget drain).
    pub repressi_timer_stop: Option<Arc<AtomicBool>>,
    /// sec-r3: memoized ring state — (tick, levels). repressi_state()
    /// integrates incrementally from here instead of re-folding O(tick)
    /// from init on every read (the re-audit's CPU-burn finding), with the
    /// per-tick work charged as fuel.
    pub repressi_cache: std::cell::RefCell<(u64, Vec<f64>)>,
    pub ires: Vec<String>,
    pub enhanced: Vec<String>,
    pub defined_genes: Vec<String>,
    pub call_counts: HashMap<String, u64>,
    pub call_time: HashMap<String, f64>, // µs inclusive (profiling)
    pub call_time_self: HashMap<String, f64>, // µs exclusive (children subtracted)
    pub call_stack: Vec<(String, f64, f64)>, // (name, start_ns, child_acc µs)
    pub call_clock: u64,
    pub gene_buckets: HashMap<String, HashMap<u64, u64>>, // burst-index bins (20 calls/bin)
    pub modules: HashMap<String, Value>,                  // path -> module map
    pub loading: Vec<String>,
    pub profiling: bool,
    pub proof_mode: bool,
    pub global: Rc<Env>,
    pub steps: u64,
    pub step_budget: u64,
    pub depth: u32,
    pub depth_limit: u32,
    pub caps: Caps,
    pub methyl_quiet: bool,
    pub methyl_noted: std::collections::HashSet<String>,
    /// T2b graded methylation: per-gene silencing level (@methylate defs +1,
    /// @acetylate defs −1). Calls are blocked at `methyl_threshold` (default 3).
    pub methyl_levels: HashMap<String, u32>,
    pub methyl_threshold: u32,
    pub asserts_run: u64,
    pub rng: u64,
    pub cli_args: Vec<String>,
    pub tasks: HashMap<i64, TaskHandle>,
    pub next_task_id: i64,
    pub seq_tx: Option<mpsc::SyncSender<crate::value::SeqMsg>>,
    /// Process-wide step ceiling shared with every spawned worker: when
    /// present, the run's TOTAL fuel (host + all threads) drains this pool.
    pub fuel_pool: Option<Arc<std::sync::atomic::AtomicI64>>,
}

/// `Interp` is never `Default::default()`d with semantics on purpose:
/// `new()` carries the deterministic RNG seed (0x9E3779B97F4A7C15) and the
/// 5M step ceiling — both are contract (differential parity, §9 fuel).
impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    pub fn new() -> Interp {
        Interp {
            notes: Vec::new(),
            notes_dropped: 0,
            cell: HashMap::new(),
            cell_entry: None,
            base_dir: None,
            cur_line: 0,
            file: "<repl>".to_string(),
            stdout_sink: None,
            silences: Vec::new(),
            fates: HashMap::new(),
            phenos: HashMap::new(),
            grn_edges: Vec::new(),
            grn_levels: HashMap::new(),
            toggles: Vec::new(),
            repressi_ring: Vec::new(),
            repressi_i: 0,
            repressi_tick: 0,
            repressi_atomic: None,
            repressi_timer_stop: None,
            repressi_cache: std::cell::RefCell::new((0, Vec::new())),
            ires: Vec::new(),
            enhanced: Vec::new(),
            defined_genes: Vec::new(),
            call_counts: HashMap::new(),
            call_time: HashMap::new(),
            call_time_self: HashMap::new(),
            call_stack: Vec::new(),
            call_clock: 0,
            gene_buckets: HashMap::new(),
            modules: HashMap::new(),
            loading: Vec::new(),
            profiling: false,
            proof_mode: false,
            global: Env::new(None),
            steps: 0,
            step_budget: 200_000_000,
            depth: 0,
            depth_limit: 10_000,
            caps: Caps::default(),
            methyl_quiet: false,
            methyl_noted: std::collections::HashSet::new(),
            methyl_levels: HashMap::new(),
            methyl_threshold: 3,
            asserts_run: 0,
            rng: 0x9E3779B97F4A7C15,
            cli_args: Vec::new(),
            tasks: HashMap::new(),
            next_task_id: 1,
            seq_tx: None,
            fuel_pool: None,
        }
    }

    pub fn note(&mut self, line: usize, rung: u8, msg: impl Into<String>) {
        // sec-r3 (re-audit #4): unbounded note growth bypassed the 2 GiB
        // allocation contract (1.8 GB of stderr observed from a methylate
        // loop). Past the cap, further notes are counted, not stored.
        const NOTE_CAP: usize = 10_000;
        if self.notes.len() >= NOTE_CAP {
            self.notes_dropped += 1;
            if self.notes_dropped == 1 {
                self.notes.push(Note {
                    line,
                    rung: 4,
                    message: "note cap (10000) reached — further notes suppressed".into(),
                });
            }
            return;
        }
        self.notes.push(Note {
            line,
            rung,
            message: msg.into(),
        });
    }

    fn str_repeat(&self, s: &str, n: i64) -> Result<Value, Stress> {
        if n < 0 {
            return Err(Stress::new("unfolded", "repeat count must be non-negative"));
        }
        let n = n as u64;
        // allocation ceiling: giant repeats abort the process outside the
        // stress model — cap as catchable overflow
        if n.saturating_mul(s.len() as u64) > 512 * 1024 * 1024 {
            return Err(Stress::new(
                "overflow",
                "repeat exceeds the 512 MiB string ceiling",
            ));
        }
        mem_charge(n.saturating_mul(s.len() as u64))?;
        Ok(Value::Str(s.repeat(n as usize)))
    }

    fn tick(&mut self) -> Result<(), Stress> {
        self.steps += 1;
        // shared pool: every 65_536 steps, drain a chunk from the run-wide
        // pool so host + workers share one "steps per run" ceiling
        if self.steps.is_multiple_of(65_536) {
            if let Some(pool) = &self.fuel_pool {
                let left = pool.fetch_sub(65_536, std::sync::atomic::Ordering::Relaxed);
                if left <= 65_536 {
                    return Err(Stress::new(
                        "overflow",
                        "run-wide step budget exhausted (shared pool)",
                    ));
                }
            }
        }
        if self.steps > self.step_budget {
            Err(Stress::new("overflow", "step budget exhausted"))
        } else {
            Ok(())
        }
    }

    // ------------------------------------------------------- statements
    pub fn exec_block(&mut self, env: &Rc<Env>, stmts: &[Stmt]) -> Result<Flow, Stress> {
        for s in stmts {
            match self.exec_stmt(env, s)? {
                Flow::Norm => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Norm)
    }

    pub fn exec_stmt(&mut self, env: &Rc<Env>, stmt: &Stmt) -> Result<Flow, Stress> {
        self.tick()?;
        match stmt {
            Stmt::Block(body) => {
                let child = Env::new(Some(env.clone()));
                self.exec_block(&child, body)
            }
            Stmt::Let(name, e) => {
                let v = self.eval(env, e)?;
                if env.get(name).is_some() {
                    self.note(0, 4, format!("rebinding '{}'", name));
                }
                env.define(name, v);
                Ok(Flow::Norm)
            }
            Stmt::Assign(name, op, e) => {
                let val = self.eval(env, e)?;
                match op {
                    None => {
                        if !env.set(name, val) {
                            self.note(0, 4, format!("'{}' was not declared; auto-declared", name));
                        }
                    }
                    Some(binop) => {
                        let cur = env.get(name).unwrap_or(Value::Null);
                        let newv = self.apply_binop(env, *binop, &cur, &val)?;
                        if !env.set(name, newv) {
                            self.note(0, 4, format!("'{}' was not declared; auto-declared", name));
                        }
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::IndexAssign(t, i, op, e) => {
                let tv = self.eval(env, t)?;
                let iv = self.eval(env, i)?;
                let mut val = self.eval(env, e)?;
                if let Some(binop) = op {
                    let cur = match (&tv, &iv) {
                        (Value::List(l), Value::Int(idx)) => {
                            let n = l.borrow().len() as i64;
                            let j = if *idx < 0 { n + *idx } else { *idx };
                            if j >= 0 && j < n {
                                l.borrow().get(j as usize).cloned().unwrap_or(Value::Null)
                            } else {
                                Value::Null
                            }
                        }
                        (Value::List(l), _) => {
                            let idx = self.as_index(&iv, l.borrow().len()).unwrap_or(usize::MAX);
                            if idx != usize::MAX && idx < l.borrow().len() {
                                l.borrow().get(idx).cloned().unwrap_or(Value::Null)
                            } else {
                                Value::Null
                            }
                        }
                        (Value::Map(m), _) => m
                            .borrow()
                            .iter()
                            .find(|(k, _)| k.deep_eq(&iv))
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null),
                        _ => Value::Null,
                    };
                    val = self.apply_binop(env, *binop, &cur, &val)?;
                }
                match (&tv, &iv) {
                    (Value::List(l), Value::Int(idx)) => {
                        let n = l.borrow().len() as i64;
                        let j = if *idx < 0 { n + *idx } else { *idx };
                        if j >= 0 && j < n {
                            l.borrow_mut()[j as usize] = val;
                        } else {
                            // out-of-range writes append (Total Grammar: degrade,
                            // never reject) — noted so the shape change is visible
                            l.borrow_mut().push(val);
                            self.note(0, 4, "index out of range; value appended");
                        }
                    }
                    (Value::List(l), _) => {
                        let idx = self.as_index(&iv, l.borrow().len()).unwrap_or(usize::MAX);
                        if idx != usize::MAX && idx < l.borrow().len() {
                            l.borrow_mut()[idx] = val;
                        } else {
                            l.borrow_mut().push(val);
                            self.note(0, 4, "index out of range; value appended");
                        }
                    }
                    (Value::Map(m), _) => {
                        self.map_insert(m, iv, val);
                    }
                    _ => {
                        self.note(0, 4, "index assignment on non-container ignored");
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::MemberAssign(t, key, op, e) => {
                let tv = self.eval(env, t)?;
                let mut val = self.eval(env, e)?;
                if let Some(binop) = op {
                    let cur = match &tv {
                        Value::Map(m) => m
                            .borrow()
                            .iter()
                            .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null),
                        _ => Value::Null,
                    };
                    val = self.apply_binop(env, *binop, &cur, &val)?;
                }
                match tv {
                    Value::Map(m) => {
                        self.map_insert(&m, Value::Str(key.clone()), val);
                    }
                    Value::Obj(_d, m) => {
                        self.map_insert(&m, Value::Str(key.clone()), val);
                    }
                    _ => self.note(0, 4, "member assignment on non-map ignored"),
                }
                Ok(Flow::Norm)
            }
            Stmt::LetPat(pat, e) => {
                // L1a: destructuring definition. Soft-miss: a pattern never
                // hard-fails — wrong shape or missing piece binds Null + note.
                let v = self.eval(env, e)?;
                self.bind_pattern(env, pat, v);
                Ok(Flow::Norm)
            }
            Stmt::ForPat(pat, iter, body) => {
                // L1a: destructuring loop — each item binds the pattern in a
                // fresh child scope, exactly like `for name`.
                let itv = self.eval(env, iter)?;
                if let Value::Seq(_def, st) = itv {
                    loop {
                        self.tick()?;
                        let v = self.seq_pull(&st)?;
                        match v {
                            Some(item) => {
                                let child = Env::new(Some(env.clone()));
                                self.bind_pattern(&child, pat, item);
                                match self.exec_block(&child, body)? {
                                    Flow::Brk => break,
                                    Flow::Ret(v) => return Ok(Flow::Ret(v)),
                                    _ => {}
                                }
                            }
                            None => break,
                        }
                    }
                    return Ok(Flow::Norm);
                }
                let items: Vec<Value> = match itv {
                    Value::List(l) => l.borrow().clone(),
                    Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    Value::Map(m) => m.borrow().iter().map(|(k, _)| k.clone()).collect(),
                    other => {
                        self.note(
                            0,
                            4,
                            format!("cannot iterate {}; loop skipped", other.type_name()),
                        );
                        Vec::new()
                    }
                };
                for item in items {
                    self.tick()?;
                    let child = Env::new(Some(env.clone()));
                    self.bind_pattern(&child, pat, item);
                    match self.exec_block(&child, body)? {
                        Flow::Brk => break,
                        Flow::Ret(v) => return Ok(Flow::Ret(v)),
                        _ => {}
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::MultiAssign(targets, values, define) => {
                // L1a: swap / multiple assignment. All right-hand values are
                // evaluated (left to right) BEFORE any target is written —
                // `a, b = b, a` swaps, never smears.
                let mut vals = Vec::with_capacity(values.len());
                for v in values {
                    vals.push(self.eval(env, v)?);
                }
                if vals.len() < targets.len() {
                    self.note(
                        0,
                        4,
                        "multi-assign: fewer values than targets; the rest bind null",
                    );
                } else if vals.len() > targets.len() {
                    self.note(0, 4, "multi-assign: extra values dropped");
                }
                if *define && vals.len() != targets.len() {
                    self.note(
                        0,
                        4,
                        "multi 'let': fewer values than names; the rest bind null",
                    );
                }
                for (i, t) in targets.iter().enumerate() {
                    let val = vals.get(i).cloned().unwrap_or(Value::Null);
                    if *define {
                        match t {
                            Expr::Ident(name) => {
                                if env.get(name).is_some() {
                                    self.note(0, 4, format!("rebinding '{}'", name));
                                }
                                env.define(name, val);
                            }
                            _ => {
                                self.note(0, 4, "multi 'let' target must be a name; dropped");
                            }
                        }
                    } else {
                        self.assign_to_target(env, t, val)?;
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::If(branches, els) => {
                for (cond, body) in branches {
                    let c = self.eval(env, cond)?;
                    if c.truthy() {
                        let child = Env::new(Some(env.clone()));
                        return self.exec_block(&child, body);
                    }
                }
                if let Some(eb) = els {
                    let child = Env::new(Some(env.clone()));
                    return self.exec_block(&child, eb);
                }
                Ok(Flow::Norm)
            }
            Stmt::While(cond, body) => {
                loop {
                    self.tick()?;
                    let c = self.eval(env, cond)?;
                    if !c.truthy() {
                        break;
                    }
                    let child = Env::new(Some(env.clone()));
                    match self.exec_block(&child, body)? {
                        Flow::Brk => break,
                        Flow::Ret(v) => return Ok(Flow::Ret(v)),
                        _ => {}
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::Loop(body) => {
                loop {
                    self.tick()?;
                    let child = Env::new(Some(env.clone()));
                    match self.exec_block(&child, body)? {
                        Flow::Brk => break,
                        Flow::Ret(v) => return Ok(Flow::Ret(v)),
                        _ => {}
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::For(name, iter, body) => {
                let itv = self.eval(env, iter)?;
                if let Value::Seq(_def, st) = itv {
                    // lazy pull iteration over a sequence
                    loop {
                        self.tick()?;
                        let v = self.seq_pull(&st)?;
                        match v {
                            Some(item) => {
                                let child = Env::new(Some(env.clone()));
                                child.define(name, item);
                                match self.exec_block(&child, body)? {
                                    Flow::Brk => break,
                                    Flow::Ret(v) => return Ok(Flow::Ret(v)),
                                    _ => {}
                                }
                            }
                            None => break,
                        }
                    }
                    return Ok(Flow::Norm);
                }
                let items: Vec<Value> = match itv {
                    Value::List(l) => l.borrow().clone(),
                    Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    Value::Map(m) => m.borrow().iter().map(|(k, _)| k.clone()).collect(),
                    other => {
                        self.note(
                            0,
                            4,
                            format!("cannot iterate {}; loop skipped", other.type_name()),
                        );
                        Vec::new()
                    }
                };
                for item in items {
                    self.tick()?;
                    let child = Env::new(Some(env.clone()));
                    child.define(name, item);
                    match self.exec_block(&child, body)? {
                        Flow::Brk => break,
                        Flow::Ret(v) => return Ok(Flow::Ret(v)),
                        _ => {}
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(env, e)?,
                    None => Value::Null,
                };
                Ok(Flow::Ret(v))
            }
            Stmt::Break => Ok(Flow::Brk),
            Stmt::Continue => Ok(Flow::Cont),
            Stmt::ExprStmt(e) => {
                // Stress propagates to the nearest stress frame or the top-level
                // containment loop (proof frames record assert failures).
                self.eval(env, e)?;
                Ok(Flow::Norm)
            }
            Stmt::Match(subject, cases) => {
                let sv = self.eval(env, subject)?;
                for (pat, body) in cases {
                    let hit = match pat {
                        MatchPat::Wild => true,
                        MatchPat::Lit(l) => {
                            let lv = match self.eval(env, l) {
                                Ok(v) => v,
                                Err(s) => {
                                    self.note(
                                        0,
                                        4,
                                        format!(
                                            "pattern evaluation contained: [{}] {}",
                                            s.kind, s.message
                                        ),
                                    );
                                    Value::Null
                                }
                            };
                            sv.deep_eq(&lv)
                        }
                        MatchPat::Multi(ls) => ls.iter().any(|l| {
                            let lv = match self.eval(env, l) {
                                Ok(v) => v,
                                Err(s) => {
                                    self.note(
                                        0,
                                        4,
                                        format!(
                                            "pattern evaluation contained: [{}] {}",
                                            s.kind, s.message
                                        ),
                                    );
                                    Value::Null
                                }
                            };
                            sv.deep_eq(&lv)
                        }),
                        MatchPat::Bind(n) => {
                            let child = Env::new(Some(env.clone()));
                            child.define(n, sv.clone());
                            return self.exec_block(&child, body);
                        }
                    };
                    if hit {
                        let child = Env::new(Some(env.clone()));
                        return self.exec_block(&child, body);
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::Use(path, alias) => {
                match crate::genes::load_module(self, path) {
                    Ok(modv) => {
                        let name = alias.clone().unwrap_or_else(|| {
                            std::path::Path::new(path)
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| "mod".into())
                        });
                        env.define(&name, modv.clone());
                        // flat-bind ALL exports beside the alias map: the whole
                        // module repertoire (genes AND data like config lets)
                        // stays addressable by name — workers and call()
                        // resolve them without a prefix, and worker snapshots
                        // carry module data across the membrane
                        if let Value::Map(m) = &modv {
                            let flat: Vec<(String, Value)> = m
                                .borrow()
                                .iter()
                                .filter_map(|(k, v)| match k {
                                    Value::Str(s) => Some((s.clone(), v.clone())),
                                    _ => None,
                                })
                                .collect();
                            for (kname, v) in flat {
                                env.define(&kname, v);
                            }
                        }
                    }
                    Err(msg) => {
                        self.note(0, 4, format!("use '{}' failed: {}", path, msg));
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::Raise(kind, msg) => {
                let mv = self.eval(env, msg)?;
                let message = mv.display();
                let k = kind.clone().unwrap_or_else(|| "unfolded".into());
                Err(Stress {
                    kind: k,
                    message,
                    line: 0,
                })
            }
            Stmt::Stress { kind, body, rescue } => {
                let result = self.exec_block(env, body);
                match result {
                    // return/break/continue inside the guarded body propagate
                    // to the enclosing gene/loop — only STRESS is intercepted
                    Ok(flow @ (Flow::Ret(_) | Flow::Brk | Flow::Cont)) => Ok(flow),
                    Ok(_) => Ok(Flow::Norm),
                    Err(stress) => {
                        let kind_ok = match kind {
                            None => true,
                            Some(k) => k == &stress.kind || k == "any",
                        };
                        if !kind_ok {
                            return Err(stress);
                        }
                        match rescue {
                            Some((binding, rbody)) => {
                                // rescue entry is real work: charge it. A
                                // `rescue { return f() }` retry-spin otherwise
                                // dodges the fuel counter entirely (wave-3
                                // Critic-X hang #1).
                                for _ in 0..64 {
                                    self.tick()?;
                                }
                                let child = Env::new(Some(env.clone()));
                                if let Some(b) = binding {
                                    let m = self.stress_map(&stress);
                                    child.define(b, m);
                                }
                                match self.exec_block(&child, rbody)? {
                                    Flow::Norm => Ok(Flow::Norm),
                                    other => Ok(other),
                                }
                            }
                            None => {
                                self.note(
                                    0,
                                    4,
                                    format!(
                                        "stress contained: [{}] {}",
                                        stress.kind, stress.message
                                    ),
                                );
                                Ok(Flow::Norm)
                            }
                        }
                    }
                }
            }
            Stmt::Gene(def) => {
                let name = def.name.clone().unwrap_or_else(|| "<lambda>".into());
                // T2b graded methylation (D-005): every @methylate-marked
                // definition deepens the silencing level; an @acetylate
                // definition relaxes it (histone marks compete on chromatin).
                if def.methylate {
                    *self.methyl_levels.entry(name.clone()).or_insert(0) += 1;
                } else if def.acetylate {
                    let lvl = self.methyl_levels.entry(name.clone()).or_insert(0);
                    *lvl = lvl.saturating_sub(1);
                }
                // @m6a-stabilized transcripts win dispatch among same-name
                // candidates: a redefinition cannot overwrite an m6a-marked
                // binding unless it carries the mark itself.
                if !def.m6a {
                    if let Some(Value::Gene(old, _)) = env.get(&name) {
                        if old.m6a {
                            self.note(0, 4, format!("'{}' is @m6a-stabilized; redefinition ignored (mark the new copy to replace it)", name));
                            return Ok(Flow::Norm);
                        }
                    }
                }
                if !self.defined_genes.contains(&name) {
                    self.defined_genes.push(name.clone());
                }
                env.define(&name, Value::Gene(def.clone(), Some(env.clone())));
                Ok(Flow::Norm)
            }
            Stmt::Splice(sp) => {
                let chosen = crate::genes::choose_variant(self, sp);
                if let Some((vname, def)) = chosen {
                    self.note(
                        0,
                        1,
                        format!("splice '{}' → variant '{}' active", sp.root, vname),
                    );
                    if !self.defined_genes.contains(&sp.root) {
                        self.defined_genes.push(sp.root.clone());
                    }
                    env.define(&sp.root, Value::Gene(def, Some(env.clone())));
                }
                Ok(Flow::Norm)
            }
            Stmt::Silence(from, to) => {
                if let Some(t) = to {
                    self.silences.push((from.clone(), t.clone()));
                    self.note(0, 1, format!("RISC loaded: '{}' silenced → '{}'", from, t));
                }
                Ok(Flow::Norm)
            }
            Stmt::Enhance(names) => {
                for n in names {
                    if !self.enhanced.contains(n) {
                        self.enhanced.push(n.clone());
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::Ires(name) => {
                if !self.ires.contains(name) {
                    self.ires.push(name.clone());
                }
                Ok(Flow::Norm)
            }
            Stmt::Fate(def) => {
                self.fates.insert(def.name.clone(), def.clone());
                Ok(Flow::Norm)
            }
            Stmt::Regulate(edges) => {
                self.grn_edges.extend(edges.clone());
                Ok(Flow::Norm)
            }
            Stmt::Toggle(a, b) => {
                self.toggles.push((a.clone(), b.clone(), true));
                Ok(Flow::Norm)
            }
            Stmt::Repressilator(ring, period) => {
                self.repressi_ring = ring.clone();
                self.repressi_i = 0;
                self.repressi_tick = 0;
                if let Some(sec) = period {
                    if *sec > 0.0 {
                        let counter = Arc::new(AtomicU64::new(0));
                        let c2 = counter.clone();
                        let ms = (*sec * 1000.0).clamp(1.0, 60_000.0) as u64;
                        // sec-r3: one SHARED cancellable timer per interp — the
                        // previous timer is flagged off before a new spawn, so
                        // repeated repressilator declarations cannot drain the
                        // thread budget with orphaned sleeper threads.
                        if let Some(flag) = self.repressi_timer_stop.take() {
                            flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        let stop = Arc::new(AtomicBool::new(false));
                        let stop2 = stop.clone();
                        // timer goes through the thread budget (S4 NEW-5):
                        // capped, guarded, and failure is a note — never a
                        // bare uncapped spawn
                        match crate::genes::spawn_worker(move || {
                            while !stop2.load(std::sync::atomic::Ordering::SeqCst) {
                                std::thread::sleep(std::time::Duration::from_millis(ms));
                                c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            }
                        }) {
                            Ok(()) => {
                                self.repressi_atomic = Some(counter);
                                self.repressi_timer_stop = Some(stop);
                                self.note(
                                    0,
                                    1,
                                    format!("repressilator oscillating every {} ms", ms),
                                );
                            }
                            Err(_) => {
                                self.note(
                                    0,
                                    4,
                                    "repressilator timer skipped: thread budget exhausted",
                                );
                            }
                        }
                    }
                }
                Ok(Flow::Norm)
            }
            Stmt::Frame { is_proof, .. } => {
                let _ = is_proof; // proof frames are collected by the loader
                Ok(Flow::Norm)
            }
            Stmt::Edit(target, reps) => {
                let _ = (target, reps); // in-file edit blocks are metadata (load-time .rna handles patches)
                Ok(Flow::Norm)
            }
            Stmt::AnchorExport(_) | Stmt::AnchorImport(_) => Ok(Flow::Norm),
            Stmt::Tad(_, body) => {
                // TAD insulation controls EXPORTS (module boundary), not lexical
                // scope: members bind into the enclosing environment.
                self.exec_block(env, body)
            }
            Stmt::Pheno(def) => {
                self.phenos.insert(def.name.clone(), def.clone());
                Ok(Flow::Norm)
            }
            Stmt::Seq(def) => {
                let name = def.name.clone().unwrap_or_else(|| "<lambda>".into());
                if !self.defined_genes.contains(&name) {
                    self.defined_genes.push(name.clone());
                }
                env.define(&name, Value::Gene(def.clone(), Some(env.clone())));
                Ok(Flow::Norm)
            }
            Stmt::Yield(e) => {
                let v = match e {
                    Some(e) => self.eval(env, e)?,
                    None => Value::Null,
                };
                if let Some(tx) = &self.seq_tx {
                    // rendezvous: send blocks until the consumer pulls — laziness
                    let _ = tx.send(crate::value::SeqMsg::Yield(crate::genes::to_send(&v)));
                    Ok(Flow::Norm)
                } else {
                    self.note(0, 4, "yield outside a sequence; treated as return");
                    Ok(Flow::Ret(v))
                }
            }
        }
    }

    /// L1a: shared member-read logic for `Expr::Member` and `Expr::MemberSafe`
    /// (the safe form null-checks the receiver before calling this).
    fn member_value(&mut self, tv: Value, key: &str) -> Result<Value, Stress> {
        match &tv {
            Value::Map(m) => match m
                .borrow()
                .iter()
                .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
            {
                Some((_, v)) => Ok(v.clone()),
                None => {
                    self.note(0, 4, format!("member '{}' missing on map; null", key));
                    Ok(Value::Null)
                }
            },
            Value::Obj(d, m) => match m
                .borrow()
                .iter()
                .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
            {
                Some((_, v)) => Ok(v.clone()),
                None => {
                    self.note(
                        0,
                        4,
                        format!("field '{}' missing on phenotype {}; null", key, d.name),
                    );
                    Ok(Value::Null)
                }
            },
            _ => {
                self.note(
                    0,
                    4,
                    format!("member '{}' on {} is null", key, tv.type_name()),
                );
                Ok(Value::Null)
            }
        }
    }

    /// L1a: bind a destructuring pattern. Total Grammar soft-miss: a wrong
    /// container shape binds all Nulls with one note; a missing element/key
    /// binds Null for that piece only. Every binding is a fresh `let`
    /// (rebinding notes if the name already exists in this scope).
    fn bind_pattern(&mut self, env: &Rc<Env>, pat: &Pat, v: Value) {
        match pat {
            Pat::Bind(name) => {
                if env.get(name).is_some() {
                    self.note(0, 4, format!("rebinding '{}'", name));
                }
                env.define(name, v);
            }
            Pat::List { elems, rest } => match &v {
                Value::List(l) => {
                    let n = l.borrow().len();
                    for (i, ep) in elems.iter().enumerate() {
                        let item = if i < n {
                            l.borrow()[i].clone()
                        } else {
                            self.note(0, 4, format!("destructure: index {} missing; null", i));
                            Value::Null
                        };
                        self.bind_pattern(env, ep, item);
                    }
                    match rest {
                        Some(rname) => {
                            let tail: Vec<Value> = if elems.len() < n {
                                l.borrow()[elems.len()..].to_vec()
                            } else {
                                Vec::new()
                            };
                            if env.get(rname).is_some() {
                                self.note(0, 4, format!("rebinding '{}'", rname));
                            }
                            env.define(rname, Value::List(Rc::new(RefCell::new(tail))));
                        }
                        None => {
                            if n > elems.len() {
                                self.note(
                                    0,
                                    4,
                                    format!(
                                        "destructure: {} extra element(s) dropped",
                                        n - elems.len()
                                    ),
                                );
                            }
                        }
                    }
                }
                Value::Null => {
                    self.note(0, 4, "destructure of null binds nulls");
                    for ep in elems {
                        self.bind_pattern(env, ep, Value::Null);
                    }
                    if let Some(rname) = rest {
                        if env.get(rname).is_some() {
                            self.note(0, 4, format!("rebinding '{}'", rname));
                        }
                        env.define(rname, Value::List(Rc::new(RefCell::new(Vec::new()))));
                    }
                }
                Value::Str(sv) => {
                    // list patterns destructure strings by char (like for-in)
                    let chars: Vec<Value> = sv.chars().map(|c| Value::Str(c.to_string())).collect();
                    let n = chars.len();
                    for (i, ep) in elems.iter().enumerate() {
                        let item = if i < n {
                            chars[i].clone()
                        } else {
                            self.note(0, 4, format!("destructure: index {} missing; null", i));
                            Value::Null
                        };
                        self.bind_pattern(env, ep, item);
                    }
                    match rest {
                        Some(rname) => {
                            let tail: Vec<Value> = if elems.len() < n {
                                chars[elems.len()..].to_vec()
                            } else {
                                Vec::new()
                            };
                            if env.get(rname).is_some() {
                                self.note(0, 4, format!("rebinding '{}'", rname));
                            }
                            env.define(rname, Value::List(Rc::new(RefCell::new(tail))));
                        }
                        None => {
                            if n > elems.len() {
                                self.note(
                                    0,
                                    4,
                                    format!(
                                        "destructure: {} extra element(s) dropped",
                                        n - elems.len()
                                    ),
                                );
                            }
                        }
                    }
                }
                other => {
                    self.note(
                        0,
                        4,
                        format!(
                            "cannot destructure {}; pattern binds nulls",
                            other.type_name()
                        ),
                    );
                    for ep in elems {
                        self.bind_pattern(env, ep, Value::Null);
                    }
                    if let Some(rname) = rest {
                        if env.get(rname).is_some() {
                            self.note(0, 4, format!("rebinding '{}'", rname));
                        }
                        env.define(rname, Value::List(Rc::new(RefCell::new(Vec::new()))));
                    }
                }
            },
            Pat::Map { keys } => match &v {
                Value::Map(m) => {
                    for k in keys {
                        let item = m
                            .borrow()
                            .iter()
                            .find(|(mk, _)| matches!(mk, Value::Str(s) if s == k))
                            .map(|(_, mv)| mv.clone());
                        match item {
                            Some(mv) => {
                                if env.get(k).is_some() {
                                    self.note(0, 4, format!("rebinding '{}'", k));
                                }
                                env.define(k, mv);
                            }
                            None => {
                                self.note(0, 4, format!("destructure: key '{}' missing; null", k));
                                if env.get(k).is_some() {
                                    self.note(0, 4, format!("rebinding '{}'", k));
                                }
                                env.define(k, Value::Null);
                            }
                        }
                    }
                }
                other => {
                    self.note(
                        0,
                        4,
                        format!(
                            "cannot destructure {}; pattern binds nulls",
                            other.type_name()
                        ),
                    );
                    for k in keys {
                        if env.get(k).is_some() {
                            self.note(0, 4, format!("rebinding '{}'", k));
                        }
                        env.define(k, Value::Null);
                    }
                }
            },
        }
    }

    /// L1a: single-target writer for multi-assignment (names, index targets,
    /// member targets). Mirrors IndexAssign/MemberAssign write semantics.
    fn assign_to_target(&mut self, env: &Rc<Env>, t: &Expr, val: Value) -> Result<(), Stress> {
        match t {
            Expr::Ident(name) => {
                if !env.set(name, val) {
                    self.note(0, 4, format!("'{}' was not declared; auto-declared", name));
                }
                Ok(())
            }
            Expr::Index(ct, it, idx_line) => {
                self.cur_line = *idx_line;
                let tv = self.eval(env, ct)?;
                let iv = self.eval(env, it)?;
                match (&tv, &iv) {
                    (Value::List(l), Value::Int(idx)) => {
                        let n = l.borrow().len() as i64;
                        let j = if *idx < 0 { n + *idx } else { *idx };
                        if j >= 0 && j < n {
                            l.borrow_mut()[j as usize] = val;
                        } else {
                            l.borrow_mut().push(val);
                            self.note(0, 4, "index out of range; value appended");
                        }
                        Ok(())
                    }
                    (Value::List(l), _) => {
                        let idx = self.as_index(&iv, l.borrow().len()).unwrap_or(usize::MAX);
                        if idx != usize::MAX && idx < l.borrow().len() {
                            l.borrow_mut()[idx] = val;
                        } else {
                            l.borrow_mut().push(val);
                            self.note(0, 4, "index out of range; value appended");
                        }
                        Ok(())
                    }
                    (Value::Map(m), _) => {
                        self.map_insert(m, iv, val);
                        Ok(())
                    }
                    _ => {
                        self.note(0, 4, "index assignment on non-container ignored");
                        Ok(())
                    }
                }
            }
            Expr::Member(ct, key) => {
                let tv = self.eval(env, ct)?;
                match tv {
                    Value::Map(m) => {
                        self.map_insert(&m, Value::Str(key.clone()), val);
                        Ok(())
                    }
                    Value::Obj(_d, m) => {
                        self.map_insert(&m, Value::Str(key.clone()), val);
                        Ok(())
                    }
                    _ => {
                        self.note(0, 4, "member assignment on non-map ignored");
                        Ok(())
                    }
                }
            }
            _ => {
                self.note(0, 4, "multi-assign target invalid; value dropped");
                Ok(())
            }
        }
    }

    pub fn stress_map(&self, s: &Stress) -> Value {
        Value::Map(Rc::new(RefCell::new(crate::value::MapStore::from_vec(
            vec![
                (Value::Str("kind".into()), Value::Str(s.kind.clone())),
                (Value::Str("message".into()), Value::Str(s.message.clone())),
            ],
        ))))
    }

    pub fn as_index(&self, v: &Value, len: usize) -> Result<usize, Stress> {
        match v {
            Value::Int(i) => {
                if *i < 0 {
                    // negative indexing counts from the end; past-the-front is
                    // out of range (never clamped)
                    let j = len as i64 + *i;
                    if j < 0 {
                        Err(Stress::at(
                            self.cur_line,
                            "missing",
                            format!("index {} out of range", *i),
                        ))
                    } else {
                        Ok(j as usize)
                    }
                } else {
                    Ok(*i as usize)
                }
            }
            other => Err(Stress::new(
                "missing",
                format!("index must be int, found {}", other.type_name()),
            )),
        }
    }

    pub fn map_insert(&self, m: &crate::value::MapRef, key: Value, val: Value) {
        // dx-r3: memoized upsert (was a linear deep_eq scan)
        m.borrow_mut().insert(key, val);
    }

    // ------------------------------------------------------- expressions
    pub fn eval(&mut self, env: &Rc<Env>, e: &Expr) -> Result<Value, Stress> {
        self.tick()?;
        match e {
            Expr::Null => Ok(Value::Null),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Int(i) => Ok(Value::Int(*i)),
            Expr::Float(f) => Ok(Value::Float(*f)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Interp(parts) => {
                let mut out = String::new();
                for p in parts {
                    match p {
                        InterpPart::Lit(s) => out.push_str(s),
                        InterpPart::Expr(e) => match self.eval(env, e) {
                            Ok(v) => out.push_str(&v.display()),
                            Err(s) => {
                                // a stressed interpolation degrades to "null" —
                                // the surrounding statement still produces output
                                self.note(
                                    0,
                                    4,
                                    format!(
                                        "interpolation stress contained: [{}] {}",
                                        s.kind, s.message
                                    ),
                                );
                                out.push_str("null");
                            }
                        },
                    }
                }
                Ok(Value::Str(out))
            }
            Expr::List(items) => {
                let mut vs = Vec::with_capacity(items.len());
                for it in items {
                    vs.push(self.eval(env, it)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(vs))))
            }
            Expr::Map(pairs) => {
                let m: crate::value::MapRef =
                    Rc::new(RefCell::new(crate::value::MapStore::default()));
                for (k, v) in pairs {
                    let kv = self.eval(env, k)?;
                    let vv = self.eval(env, v)?;
                    let key = match key_scalar(&kv) {
                        Some(k) => k,
                        None => {
                            self.note(0, 4, "map key must be scalar; key stringified");
                            Value::Str(kv.display())
                        }
                    };
                    self.map_insert(&m, key, vv);
                }
                Ok(Value::Map(m))
            }
            Expr::Ident(name) => match env.get(name) {
                Some(v) => Ok(v),
                None => {
                    self.note(0, 4, format!("unbound '{}' read as null", name));
                    Ok(Value::Null)
                }
            },
            Expr::Unary(op, e) => {
                let v = self.eval(env, e)?;
                match op {
                    UnOp::Neg => match v {
                        Value::Int(i) => i.checked_neg().map(Value::Int).ok_or_else(|| {
                            Stress::new("overflow", "int overflow in negation (i64::MIN)")
                        }),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        other => Err(Stress::new(
                            "unfolded",
                            format!("cannot negate {}", other.type_name()),
                        )),
                    },
                    UnOp::Not => Ok(Value::Bool(!v.truthy())),
                    UnOp::BitNot => match v {
                        Value::Int(i) => Ok(Value::Int(!i)),
                        Value::Bool(b) => Ok(Value::Int(!(b as i64))),
                        other => Err(Stress::new(
                            "unfolded",
                            format!("cannot bit-invert {}", other.type_name()),
                        )),
                    },
                }
            }
            Expr::Binary(op, l, r, bin_line) => {
                // dx-r4: hard type errors from this expression locate here
                self.cur_line = *bin_line;
                // short-circuit
                match op {
                    BinOp::And => {
                        let lv = self.eval(env, l)?;
                        if !lv.truthy() {
                            return Ok(lv);
                        }
                        return self.eval(env, r);
                    }
                    BinOp::Or => {
                        let lv = self.eval(env, l)?;
                        if lv.truthy() {
                            return Ok(lv);
                        }
                        return self.eval(env, r);
                    }
                    BinOp::Nullish => {
                        // L1a: coalesce Null only — falsy-but-non-null values
                        // (0, "", []) pass through unchanged.
                        let lv = self.eval(env, l)?;
                        if matches!(lv, Value::Null) {
                            return self.eval(env, r);
                        }
                        return Ok(lv);
                    }
                    _ => {}
                }
                let lv = self.eval(env, l)?;
                let rv = self.eval(env, r)?;
                self.apply_binop(env, *op, &lv, &rv)
            }
            Expr::Call(callee, args, call_line) => {
                // A13 (dx-r2): builtin diagnostics carry the call site
                self.cur_line = *call_line;
                // check silences at call sites (RISC)
                if let Expr::Ident(name) = &**callee {
                    if let Some((from, to)) = self.silences.iter().find(|(f, _)| f == name).cloned()
                    {
                        // acetylated genes are immune
                        let immune = match env.get(name) {
                            Some(Value::Gene(d, _)) => d.acetylate,
                            _ => false,
                        };
                        if !immune {
                            self.note(
                                0,
                                4,
                                format!("RISC: call to '{}' silenced → '{}'", from, to),
                            );
                            let target = env.get(&to).unwrap_or(Value::Null);
                            let mut argvs = Vec::new();
                            for a in args {
                                argvs.push(self.eval(env, a)?);
                            }
                            return self.call_value(env, &target, argvs);
                        }
                    }
                    // named call: user genes, builtins, wobble repair, phantoms
                    let mut argvs = Vec::with_capacity(args.len());
                    for a in args {
                        argvs.push(self.eval(env, a)?);
                    }
                    return self.call_named(env, name, argvs);
                }
                let cv = self.eval(env, callee)?;
                let mut argvs = Vec::with_capacity(args.len());
                for a in args {
                    argvs.push(self.eval(env, a)?);
                }
                self.call_value(env, &cv, argvs)
            }
            Expr::Index(t, i, idx_line) => {
                // dx-r4: index errors locate at the bracket
                self.cur_line = *idx_line;
                let tv = self.eval(env, t)?;
                let iv = self.eval(env, i)?;
                match (&tv, &iv) {
                    (Value::List(l), _) => {
                        let idx = self.as_index(&iv, l.borrow().len())?;
                        match l.borrow().get(idx) {
                            Some(v) => Ok(v.clone()),
                            None => Err(Stress::at(
                                self.cur_line,
                                "missing",
                                format!("index {} out of range", idx),
                            )),
                        }
                    }
                    (Value::Map(m), _) => {
                        let pos = m.borrow().position(&iv);
                        match pos {
                            Some(i) => Ok(m.borrow().get(i).unwrap().1.clone()),
                            None => Err(Stress::new("missing", "key not found")),
                        }
                    }
                    (Value::Str(s), _) => {
                        let idx = self.as_index(&iv, s.chars().count())?;
                        match s.chars().nth(idx) {
                            Some(c) => Ok(Value::Str(c.to_string())),
                            None => Err(Stress::new("missing", "char index out of range")),
                        }
                    }
                    _ => Err(Stress::new(
                        "unfolded",
                        format!("cannot index {}", tv.type_name()),
                    )),
                }
            }
            Expr::Member(t, key) => {
                let tv = self.eval(env, t)?;
                self.member_value(tv, key)
            }
            Expr::MemberSafe(t, key) => {
                // L1a: `a?.k` — a Null receiver is Null, silently; anything
                // else behaves exactly like `.` (missing keys still note).
                let tv = self.eval(env, t)?;
                if matches!(tv, Value::Null) {
                    return Ok(Value::Null);
                }
                self.member_value(tv, key)
            }
            Expr::Method(t, name, args) => {
                let tv = self.eval(env, t)?;
                let mut argvs = Vec::with_capacity(args.len());
                for a in args {
                    argvs.push(self.eval(env, a)?);
                }
                self.call_method(env, tv, name, argvs)
            }
            Expr::MethodSafe(t, name, args) => {
                // L1a: `a?.k(args)` — same contract as `?.` members.
                let tv = self.eval(env, t)?;
                if matches!(tv, Value::Null) {
                    return Ok(Value::Null);
                }
                let mut argvs = Vec::with_capacity(args.len());
                for a in args {
                    argvs.push(self.eval(env, a)?);
                }
                self.call_method(env, tv, name, argvs)
            }
            Expr::Lambda(def) => Ok(Value::Gene(def.clone(), Some(env.clone()))),
            Expr::Ternary(c, a, b) => {
                let cv = self.eval(env, c)?;
                if cv.truthy() {
                    self.eval(env, a)
                } else {
                    self.eval(env, b)
                }
            }
            Expr::New(name, args) => {
                let def = match self.phenos.get(name) {
                    Some(d) => d.clone(),
                    None => {
                        self.note(
                            0,
                            4,
                            format!(
                                "phenotype '{}' not declared; instance is an empty map",
                                name
                            ),
                        );
                        return Ok(Value::Map(Rc::new(RefCell::new(
                            crate::value::MapStore::default(),
                        ))));
                    }
                };
                let mut argvs = Vec::with_capacity(args.len());
                for a in args {
                    argvs.push(self.eval(env, a)?);
                }
                self.construct_obj(&def, argvs)
            }
            Expr::FateNew(name) => {
                let def = match self.fates.get(name) {
                    Some(d) => d.clone(),
                    None => {
                        self.note(
                            0,
                            4,
                            format!("fate '{}' not declared; instance inert", name),
                        );
                        return Ok(Value::Map(Rc::new(RefCell::new(
                            crate::value::MapStore::default(),
                        ))));
                    }
                };
                let enter = def.enter.clone().unwrap_or_else(|| {
                    def.states
                        .first()
                        .map(|(s, _)| s.clone())
                        .unwrap_or_default()
                });
                Ok(Value::Map(Rc::new(RefCell::new(
                    crate::value::MapStore::from_vec(vec![
                        (Value::Str("#fate".into()), Value::Str(def.name.clone())),
                        (Value::Str("#state".into()), Value::Str(enter)),
                    ]),
                ))))
            }
            Expr::Collect {
                var,
                iter,
                filter,
                body,
            } => {
                let itv = self.eval(env, iter)?;
                let mut source: Vec<Value> = Vec::new();
                let mut seq_state: Option<Rc<RefCell<SeqState>>> = None;
                match itv {
                    Value::List(l) => source = l.borrow().clone(),
                    Value::Str(s) => {
                        source = s.chars().map(|c| Value::Str(c.to_string())).collect()
                    }
                    Value::Map(m) => source = m.borrow().iter().map(|(k, _)| k.clone()).collect(),
                    Value::Seq(_d, st) => seq_state = Some(st),
                    _ => {}
                }
                let mut out = Vec::new();
                let mut idx = 0usize;
                loop {
                    self.tick()?;
                    let item = if let Some(st) = &seq_state {
                        self.seq_pull(st)?
                    } else {
                        if idx >= source.len() {
                            None
                        } else {
                            let v = source[idx].clone();
                            idx += 1;
                            Some(v)
                        }
                    };
                    let item = match item {
                        Some(v) => v,
                        None => break,
                    };
                    let child = Env::new(Some(env.clone()));
                    child.define(var, item);
                    if let Some(f) = filter {
                        let keep = self.eval(&child, f)?;
                        if !keep.truthy() {
                            continue;
                        }
                    }
                    out.push(self.eval(&child, body)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
        }
    }

    pub fn apply_binop(
        &mut self,
        _env: &Rc<Env>,
        op: BinOp,
        l: &Value,
        r: &Value,
    ) -> Result<Value, Stress> {
        use BinOp::*;
        match op {
            // L1a: `??` short-circuits in eval() before apply_binop; this arm
            // exists for totality (value-level coalescing, both sides ready).
            Nullish => Ok(if matches!(l, Value::Null) {
                r.clone()
            } else {
                l.clone()
            }),
            Add => match (l, r) {
                (Value::Int(a), Value::Int(b)) => a
                    .checked_add(*b)
                    .map(Value::Int)
                    .ok_or_else(|| Stress::new("overflow", "int overflow in '+'")),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
                (Value::Str(a), Value::Str(b)) => {
                    // per-op ceiling + aggregate allocation ceiling
                    if a.len().saturating_add(b.len()) > 512 * 1024 * 1024 {
                        return Err(Stress::new(
                            "overflow",
                            "string concat exceeds the 512 MiB ceiling",
                        ));
                    }
                    mem_charge(a.len() as u64 + b.len() as u64)?;
                    Ok(Value::Str(format!("{}{}", a, b)))
                }
                (Value::List(a), Value::List(b)) => {
                    if a.borrow().len().saturating_add(b.borrow().len()) > 64 * 1024 * 1024 {
                        return Err(Stress::new(
                            "overflow",
                            "list concat exceeds the 64M-element ceiling",
                        ));
                    }
                    let mut v = a.borrow().clone();
                    v.extend(b.borrow().iter().cloned());
                    Ok(Value::List(Rc::new(RefCell::new(v))))
                }
                _ => Err(Stress::at(
                    self.cur_line,
                    "unfolded",
                    format!("cannot add {} and {}", l.type_name(), r.type_name()),
                )),
            },
            Sub => self.arith(
                l,
                r,
                "+-",
                |a, b| a.checked_sub(*b).map(Value::Int),
                |a, b| a - b,
            ),
            Mul => {
                // string repetition (Python parity): "ab" * 3 / 3 * "ab"
                if let (Value::Str(s), Value::Int(n)) = (l, r) {
                    return self.str_repeat(s, *n);
                }
                if let (Value::Int(n), Value::Str(s)) = (l, r) {
                    return self.str_repeat(s, *n);
                }
                self.arith(
                    l,
                    r,
                    "*",
                    |a, b| a.checked_mul(*b).map(Value::Int),
                    |a, b| a * b,
                )
            }
            Pow => {
                // 2**10 → int (checked); anything else promotes to float
                match (l, r) {
                    (Value::Int(a), Value::Int(b)) if *b >= 0 && *b <= u32::MAX as i64 => a
                        .checked_pow(*b as u32)
                        .map(Value::Int)
                        .ok_or_else(|| Stress::new("overflow", "int overflow in '**'")),
                    _ => {
                        let (a, b) = self.as_floats(l, r)?;
                        let out = a.powf(b);
                        if out.is_infinite() && a.is_finite() && b.is_finite() && b > 0.0 {
                            return Err(Stress::new(
                                "overflow",
                                "float '**' overflowed to infinity",
                            ));
                        }
                        Ok(Value::Float(out))
                    }
                }
            }
            BitAnd | BitOr | BitXor => {
                let (a, b) = self.as_ints(l, r)?;
                Ok(Value::Int(match op {
                    BinOp::BitAnd => a & b,
                    BinOp::BitOr => a | b,
                    _ => a ^ b,
                }))
            }
            Shl | Shr => {
                let (a, b) = self.as_ints(l, r)?;
                if !(0..=63).contains(&b) {
                    return Err(Stress::new(
                        "overflow",
                        format!("shift amount {} out of range", b),
                    ));
                }
                Ok(Value::Int(if op == BinOp::Shl {
                    a.checked_shl(b as u32).unwrap_or(0)
                } else {
                    a >> b // arithmetic (sign-extending), like Python
                }))
            }
            Div => {
                let (a, b) = self.as_floats(l, r)?;
                if b == 0.0 {
                    return Err(Stress::new("unfolded", "division by zero"));
                }
                Ok(Value::Float(a / b))
            }
            FloorDiv => {
                // floor division (Python-parity), ALWAYS int, overflow-contained
                if let (Value::Int(a), Value::Int(b)) = (l, r) {
                    if *b == 0 {
                        return Err(Stress::new("unfolded", "division by zero in '//'"));
                    }
                    if *a == i64::MIN && *b == -1 {
                        return Err(Stress::new("overflow", "int overflow in '//'"));
                    }
                    let mut q = a / b;
                    if (*a < 0) != (*b < 0) && q * b != *a {
                        q -= 1; // Rust / truncates toward zero; floor rounds down
                    }
                    return Ok(Value::Int(q));
                }
                let (a, b) = self.as_floats(l, r)?;
                if b == 0.0 {
                    return Err(Stress::new("unfolded", "division by zero in '//'"));
                }
                let q = (a / b).floor();
                if !q.is_finite() || q >= 9.223372036854776e18 || q <= -9.223372036854776e18 {
                    return Err(Stress::new("overflow", "int overflow in '//'"));
                }
                Ok(Value::Int(q as i64))
            }
            Mod => {
                // int % int stays int (Python floored semantics); sign follows
                // divisor: r = a - b * floor(a / b) — 7 % -3 == -2, -7 % -3 == -1
                if let (Value::Int(a), Value::Int(b)) = (l, r) {
                    if *b == 0 {
                        return Err(Stress::new("unfolded", "modulo by zero"));
                    }
                    if *a == i64::MIN && *b == -1 {
                        return Err(Stress::new("overflow", "int overflow in '%'"));
                    }
                    let mut q = a / b; // Rust / truncates toward zero
                    if (*a < 0) != (*b < 0) && q * b != *a {
                        q -= 1; // floor rounds down
                    }
                    let rb = q
                        .checked_mul(*b)
                        .ok_or_else(|| Stress::new("overflow", "int overflow in '%'"))?;
                    let m = (*a)
                        .checked_sub(rb)
                        .ok_or_else(|| Stress::new("overflow", "int overflow in '%'"))?;
                    return Ok(Value::Int(m));
                }
                let (a, b) = self.as_floats(l, r)?;
                if b == 0.0 {
                    return Err(Stress::new("unfolded", "modulo by zero"));
                }
                Ok(Value::Float(a.rem_euclid(b.abs()) * b.signum()))
            }
            Eq => Ok(Value::Bool(l.deep_eq(r))),
            Neq => Ok(Value::Bool(!l.deep_eq(r))),
            Lt | Le | Gt | Ge => {
                // IEEE/Python parity: any comparison with NaN is false
                // (Eq/Neq already behave correctly via deep_eq)
                let nan_involved = matches!(l, Value::Float(f) if f.is_nan())
                    || matches!(r, Value::Float(f) if f.is_nan());
                if nan_involved {
                    return Ok(Value::Bool(false));
                }
                let ord = self.compare(l, r)?;
                Ok(Value::Bool(match op {
                    Lt => ord == std::cmp::Ordering::Less,
                    Le => ord != std::cmp::Ordering::Greater,
                    Gt => ord == std::cmp::Ordering::Greater,
                    Ge => ord != std::cmp::Ordering::Less,
                    _ => unreachable!(),
                }))
            }
            In => Ok(Value::Bool(match (l, r) {
                (needle, Value::List(list)) => list.borrow().iter().any(|v| v.deep_eq(needle)),
                (Value::Str(n), Value::Str(h)) => h.contains(n.as_str()),
                (needle, Value::Map(m)) => m.borrow().iter().any(|(k, _)| k.deep_eq(needle)),
                _ => {
                    return Err(Stress::new(
                        "unfolded",
                        format!(
                            "'in' not defined for {} in {}",
                            r.type_name(),
                            l.type_name()
                        ),
                    ))
                }
            })),
            And | Or => unreachable!("short-circuited earlier"),
        }
    }

    fn arith(
        &mut self,
        l: &Value,
        r: &Value,
        opname: &str,
        fi: fn(&i64, &i64) -> Option<Value>,
        ff: fn(f64, f64) -> f64,
    ) -> Result<Value, Stress> {
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => fi(a, b)
                .ok_or_else(|| Stress::new("overflow", format!("int overflow in '{}'", opname))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(ff(*a, *b))),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(ff(*a as f64, *b))),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(ff(*a, *b as f64))),
            _ => Err(Stress::at(
                self.cur_line,
                "unfolded",
                format!(
                    "cannot apply '{}' to {} and {}",
                    opname,
                    l.type_name(),
                    r.type_name()
                ),
            )),
        }
    }

    fn as_ints(&self, l: &Value, r: &Value) -> Result<(i64, i64), Stress> {
        let conv = |v: &Value| -> Result<i64, Stress> {
            match v {
                Value::Int(i) => Ok(*i),
                Value::Bool(b) => Ok(*b as i64),
                other => Err(Stress::new(
                    "unfolded",
                    format!("bitwise op needs ints, found {}", other.type_name()),
                )),
            }
        };
        Ok((conv(l)?, conv(r)?))
    }

    fn as_floats(&self, l: &Value, r: &Value) -> Result<(f64, f64), Stress> {
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => Ok((*a as f64, *b as f64)),
            (Value::Float(a), Value::Float(b)) => Ok((*a, *b)),
            (Value::Int(a), Value::Float(b)) => Ok((*a as f64, *b)),
            (Value::Float(a), Value::Int(b)) => Ok((*a, *b as f64)),
            _ => Err(Stress::at(
                self.cur_line,
                "unfolded",
                format!(
                    "numeric op needs numbers, found {} and {}",
                    l.type_name(),
                    r.type_name()
                ),
            )),
        }
    }

    fn compare(&self, l: &Value, r: &Value) -> Result<std::cmp::Ordering, Stress> {
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => Ok(a.cmp(b)),
            (Value::Str(a), Value::Str(b)) => Ok(a.cmp(b)),
            _ => {
                let (a, b) = self.as_floats(l, r)?;
                Ok(a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal))
            }
        }
    }

    // ------------------------------------------------------- calls
    pub fn call_value(
        &mut self,
        _env: &Rc<Env>,
        callee: &Value,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        match callee {
            Value::Gene(def, closure) if def.seq => {
                // reg-r3 (re-audit): the sequence's own gates ALL apply at
                // creation, in the same order as call_gene_inner — GRN veto,
                // methylation, toggle. reg-r1 gated only the toggle; a
                // level-3-methylated sequence still transcribed, breaking
                // "silenced means silenced" for the sibling feature.
                let seq_name = def.name.clone().unwrap_or_else(|| "<seq>".into());
                let dl = def.line;
                // GRN gate first (same funnel contract as gene calls)
                if let Some(reason) = self.grn_veto(&seq_name) {
                    self.note(
                        dl,
                        4,
                        format!(
                            "grn gate: sequence '{}' call suppressed ({})",
                            seq_name, reason
                        ),
                    );
                    return Ok(Value::Null);
                }
                // methylation gate (graded; @acetylate exempt)
                if !def.acetylate {
                    let lvl = *self.methyl_levels.get(&seq_name).unwrap_or(&0);
                    if lvl >= self.methyl_threshold {
                        self.note(
                            dl,
                            4,
                            format!(
                                "methylation silences: sequence '{}' (level {} >= threshold {}) — call returns null",
                                seq_name, lvl, self.methyl_threshold
                            ),
                        );
                        return Ok(Value::Null);
                    }
                }
                // reg-r1: the sequence's own toggle gate applies at creation —
                // the lazy body runs in a worker whose yield-loop cannot pass
                // through call_named, so a repressed allele must not start
                if let Some(&(ref a, ref b, a_on)) = self
                    .toggles
                    .iter()
                    .find(|(a, b, _)| a == &seq_name || b == &seq_name)
                {
                    let this_is_a = a == &seq_name;
                    let active = (a_on && this_is_a) || (!a_on && !this_is_a);
                    let immune = def.acetylate;
                    if !active && !immune {
                        let winner = if a_on { a } else { b };
                        self.note(
                            0,
                            4,
                            format!(
                                "toggle repressed: sequence '{}' is the inactive allele ('{}' is on)",
                                seq_name, winner
                            ),
                        );
                        return Ok(Value::Null);
                    }
                }
                // calling a sequence starts a worker; pulls are lazy
                if args.len() > def.params.len() && !def.params.is_empty() {
                    self.note(
                        0,
                        4,
                        format!(
                            "{} extra argument(s) in call to sequence ignored",
                            args.len() - def.params.len()
                        ),
                    );
                }
                let snap = crate::genes::snapshot_globals(self);
                let st = crate::genes::seq_start(
                    def.clone(),
                    args,
                    snap,
                    crate::genes::snapshot_regulation(self),
                    self.caps.clone(),
                    self.fuel_pool.clone(),
                )?;
                Ok(Value::Seq(def.clone(), st))
            }
            Value::Gene(def, closure) => {
                // reg-r4 (re-audit B-5): value-bound (higher-order) gene
                // calls pass the toggle gate too — "the pair gates calls"
                // is unqualified (SPEC §11); the seq branch already gated,
                // plain genes did not.
                if let Some(gname) = &def.name {
                    if let Some(&(ref a, ref b, a_on)) = self
                        .toggles
                        .iter()
                        .find(|(a, b, _)| a == gname || b == gname)
                    {
                        let this_is_a = a == gname;
                        let active = (a_on && this_is_a) || (!a_on && !this_is_a);
                        if !active && !def.acetylate {
                            let winner = if a_on { a } else { b };
                            self.note(
                                def.line,
                                4,
                                format!(
                                    "toggle repressed: '{}' is the inactive allele ('{}' is on)",
                                    gname, winner
                                ),
                            );
                            return Ok(Value::Null);
                        }
                    }
                }
                self.call_gene(def.clone(), closure.clone(), args)
            }
            other => {
                self.note(
                    0,
                    4,
                    format!("called a {} (not a gene); result null", other.type_name()),
                );
                Ok(Value::Null)
            }
        }
    }

    pub fn call_named(
        &mut self,
        env: &Rc<Env>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        // toggle bistability gate: the repressed allele of a toggle pair refuses
        // calls (acetylated genes override repression — open chromatin wins)
        if let Some(&(ref a, ref b, a_on)) =
            self.toggles.iter().find(|(a, b, _)| a == name || b == name)
        {
            let this_is_a = a == name;
            let active = (a_on && this_is_a) || (!a_on && !this_is_a);
            let immune = matches!(env.get(name), Some(Value::Gene(d, _)) if d.acetylate);
            if !active && !immune {
                let winner = if a_on { a } else { b };
                self.note(
                    0,
                    4,
                    format!(
                        "toggle repressed: '{}' is the inactive allele ('{}' is on)",
                        name, winner
                    ),
                );
                return Ok(Value::Null);
            }
        }
        // canonical builtin synonyms (print/echo/say/show → promote)
        if let Some((_, canon)) = BUILTIN_SYNONYMS.iter().find(|(s, _)| *s == name) {
            return self.call_builtin(env, canon, args);
        }
        // user gene
        if let Some(v) = env.get(name) {
            return self.call_value(env, &v, args);
        }
        // builtin
        if BUILTIN_NAMES.contains(&name) {
            return self.call_builtin(env, name, args);
        }
        // wobble: nearest callable
        let mut best: Option<(&str, i32)> = None;
        for k in BUILTIN_NAMES {
            let d = crate::ffi::edit_distance(name, k);
            let max = if name.chars().count() <= 4 { 1 } else { 2 };
            if d <= max {
                match best {
                    Some((_, bd)) if d >= bd => {}
                    _ => best = Some((k, d)),
                }
            }
        }
        if let Some((k, _)) = best {
            self.note(
                0,
                3,
                format!(
                    "wobble: unknown gene '{}' repaired to builtin '{}'",
                    name, k
                ),
            );
            return self.call_builtin(env, k, args);
        }
        if let Some(g) = self
            .defined_genes
            .iter()
            .min_by_key(|g| crate::ffi::edit_distance(name, g))
            .cloned()
        {
            let d = crate::ffi::edit_distance(name, &g);
            let max = if name.chars().count() <= 4 { 1 } else { 2 };
            if d <= max && d > 0 {
                self.note(
                    0,
                    3,
                    format!("wobble: unknown gene '{}' repaired to gene '{}'", name, g),
                );
                let v = env.get(&g).unwrap_or(Value::Null);
                return self.call_value(env, &v, args);
            }
        }
        // fate constructor: Name() creates a fate-landscape instance
        if self.fates.contains_key(name) {
            let inst = self.eval(env, &Expr::FateNew(name.to_string()))?;
            return Ok(inst);
        }
        self.note(0, 4, format!("phantom call to '{}'; result null", name));
        Ok(Value::Null)
    }

    pub fn call_gene(
        &mut self,
        def: Arc<GeneDef>,
        closure: Option<Rc<Env>>,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        // recursion depth limit (SPEC §7): runaway self-transcription is
        // contained as catchable overflow stress instead of a hard crash
        self.depth += 1;
        if self.depth > self.depth_limit {
            self.depth -= 1;
            return Err(Stress::new(
                "overflow",
                format!("recursion depth limit ({}) exceeded", self.depth_limit),
            ));
        }
        let result = self.call_gene_inner(def, closure, args);
        self.depth -= 1;
        result
    }

    /// GRN call gate (T2a / SPEC §11): does the regulatory network veto a
    /// call to `name`? Returns the veto reason when the call must be
    /// suppressed. Contract:
    ///   - activating edge with explicit `threshold t`: vetoes while
    ///     `level(source) < t` (all incoming activating edges must pass);
    ///   - inhibiting edge with explicit `threshold t`: vetoes while
    ///     `level(inhibitor) >= t`;
    ///   - edges without a threshold stay declarative (level dynamics only),
    ///   - and a threshold of 0 never blocks — so declaring a network with
    ///     no explicit thresholds changes zero call behavior (back-compat).
    ///
    /// T2e: an `enhance`d gene lowers its activating thresholds by
    /// ENHANCE_DELTA — a super-enhancer fires where an unenhanced gene
    /// stays gated.
    fn grn_veto(&mut self, name: &str) -> Option<String> {
        if self.grn_edges.is_empty() {
            return None;
        }
        let boosted = self.enhanced.iter().any(|g| g == name);
        // A11 (reg-r2): a repressilator node doubles as a GRN regulator.
        // If an edge's source is a ring node with no explicit grn_fire
        // level, the gate reads the node's normalized oscillation level
        // (raw/α, clamped 0..1) at the current tick — so the emergent
        // oscillator genuinely drives downstream genes.
        let ring_len = self.repressi_ring.len();
        let tick: u64 = match &self.repressi_atomic {
            Some(a) => a.load(std::sync::atomic::Ordering::SeqCst),
            None => self.repressi_tick,
        };
        let mut veto: Option<String> = None;
        // sec-r3: clone the matching edges — the ring-overlay path mutates
        // self (fuel-charged cache), which cannot borrow grn_edges at the
        // same time. Edge counts are tiny; a clone per veto check is noise.
        let edges: Vec<RegEdge> = self
            .grn_edges
            .iter()
            .filter(|e| e.to == name)
            .cloned()
            .collect();
        for e in &edges {
            if veto.is_some() {
                break;
            }
            let lvl = match self.grn_levels.get(&e.from) {
                Some(v) => *v,
                None => {
                    if ring_len > 0 {
                        if let Some(idx) = self.repressi_ring.iter().position(|r| r == &e.from) {
                            self.ring_gate_level(ring_len, tick, idx).unwrap_or(0.0)
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    }
                }
            };
            if e.inhibit {
                if let Some(t) = e.threshold {
                    if lvl >= t {
                        veto = Some(format!(
                            "inhibitor '{}' level {} >= threshold {}",
                            e.from, lvl, t
                        ));
                    }
                }
            } else {
                let t = e.threshold.unwrap_or(0.0);
                let t = if boosted {
                    (t - ENHANCE_DELTA).max(0.0)
                } else {
                    t
                };
                if lvl < t {
                    veto = Some(format!(
                        "regulator '{}' level {} < threshold {}",
                        e.from, lvl, t
                    ));
                }
            }
        }
        veto
    }

    /// sec-r3: cached, fuel-charged ring levels. Integrates incrementally
    /// from the memoized state — bit-identical values to folding from init
    /// (identical per-tick operation sequence via `repressilator_step`),
    /// but O(new ticks) per read instead of O(total ticks), with the
    /// integrated work charged as fuel (re-audit finding #2: 1000 reads at
    /// tick 200k cost 80 s of uncharged CPU; now it drains the step budget
    /// like any other work).
    fn ring_advance(&mut self, n: usize, tick: u64) -> Result<Vec<f64>, Stress> {
        const FUEL_PER_TICK: u64 = 20; // one step per Euler substep
        let (cached_tick, cached_levels) = {
            let c = self.repressi_cache.borrow();
            let (t, v) = &*c;
            if v.len() == n && *t <= tick {
                (*t, v.clone())
            } else {
                // ring resized or clock went backwards: rebuild from init
                (u64::MAX, Vec::new())
            }
        };
        let levels = if cached_tick == u64::MAX {
            self.steps = self
                .steps
                .saturating_add(tick.saturating_mul(FUEL_PER_TICK));
            if self.steps > self.step_budget {
                return Err(Stress::new(
                    "overflow",
                    "step budget exhausted (repressilator fold)",
                ));
            }
            repressilator_levels(n, tick)
        } else if cached_tick == tick {
            cached_levels
        } else {
            let new_ticks = tick - cached_tick;
            self.steps = self
                .steps
                .saturating_add(new_ticks.saturating_mul(FUEL_PER_TICK));
            if self.steps > self.step_budget {
                return Err(Stress::new(
                    "overflow",
                    "step budget exhausted (repressilator fold)",
                ));
            }
            let mut lv = cached_levels;
            for _ in 0..new_ticks {
                lv = repressilator_step(lv);
            }
            lv
        };
        if let Some(pool) = &self.fuel_pool {
            // ring reads pool with the run-wide budget like sleep does
            let left = pool.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            if left <= 1 {
                return Err(Stress::new(
                    "overflow",
                    "run-wide step budget exhausted (repressilator)",
                ));
            }
        }
        *self.repressi_cache.borrow_mut() = (tick, levels.clone());
        Ok(levels)
    }

    /// Normalized ring level for a GRN gate (cached, fuel-charged path).
    fn ring_gate_level(&mut self, n: usize, tick: u64, node_index: usize) -> Result<f64, Stress> {
        const ALPHA: f64 = 10.0;
        let levels = self.ring_advance(n, tick)?;
        let raw = levels[node_index.min(n.saturating_sub(1))];
        let norm = raw / ALPHA;
        Ok(if norm > 1.0 { 1.0 } else { norm })
    }

    fn call_gene_inner(
        &mut self,
        def: Arc<GeneDef>,
        closure: Option<Rc<Env>>,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        let name = def.name.clone().unwrap_or_else(|| "<lambda>".into());
        // A13 (dx-r2): gate notes point at the gene's definition line
        let dl = def.line;
        // GRN gate first: a suppressed call is not expression — it must not
        // reach the call counters, the burst bins, or the gene body.
        if let Some(reason) = self.grn_veto(&name) {
            self.note(
                dl,
                4,
                format!("grn gate: '{}' call suppressed ({})", name, reason),
            );
            return Ok(Value::Null);
        }
        // T2b methylation gate: level >= threshold blocks transcription;
        // @acetylate genes are exempt (open chromatin wins — D-005).
        if !def.acetylate {
            let lvl = *self.methyl_levels.get(&name).unwrap_or(&0);
            if lvl >= self.methyl_threshold {
                self.note(
                    dl,
                    4,
                    format!(
                        "methylation silences: '{}' (level {} >= threshold {}) — call returns null",
                        name, lvl, self.methyl_threshold
                    ),
                );
                return Ok(Value::Null);
            }
        }
        *self.call_counts.entry(name.clone()).or_insert(0) += 1;
        // burst-index binning: 20 calls per bin, per gene (gene-expression
        // burstiness is measured on per-gene time bins, not across genes)
        self.call_clock += 1;
        let bucket = self.call_clock / 20;
        *self
            .gene_buckets
            .entry(name.clone())
            .or_default()
            .entry(bucket)
            .or_insert(0) += 1;
        // @methylate: transcriptionally repressed genes announce their first
        // call (suppressed by .cell `methylate.quiet = true`)
        if def.methylate && !self.methyl_quiet && !self.methyl_noted.contains(&name) {
            self.methyl_noted.insert(name.clone());
            self.note(
                dl,
                4,
                format!("methylated call: '{}' (chromatin repressed)", name),
            );
        }
        let fenv = match &closure {
            Some(e) => Env::new(Some(e.clone())),
            None => Env::new(Some(self.global.clone())),
        };
        // bind params
        for (i, (pname, default)) in def.params.iter().enumerate() {
            if pname.is_empty() || pname == "?" {
                continue;
            }
            if let Some(a) = args.get(i) {
                fenv.define(pname, a.clone());
            } else if let Some(d) = default {
                let dv = self.eval(&fenv, d).unwrap_or(Value::Null);
                fenv.define(pname, dv);
            } else {
                self.note(
                    dl,
                    4,
                    format!(
                        "missing argument '{}' in call to {}; bound null",
                        pname, name
                    ),
                );
                fenv.define(pname, Value::Null);
            }
        }
        if args.len() > def.params.len() && !def.params.is_empty() {
            self.note(
                dl,
                4,
                format!(
                    "{} extra argument(s) in call to {} ignored",
                    args.len() - def.params.len(),
                    name
                ),
            );
        }
        // inclusive/exclusive timing: exclusive time subtracts children so a
        // caller never inflates itself with its callees' cost
        let start = if self.profiling {
            Some(crate::ffi::now_ns())
        } else {
            None
        };
        self.call_stack
            .push((name.clone(), start.unwrap_or(0.0), 0.0));
        // uORF guard
        if let Some((cond, gbody)) = &def.guard {
            let ok = self.eval(&fenv, cond).map(|v| v.truthy()).unwrap_or(false);
            if !ok {
                self.note(dl, 4, format!("guard tripped calling {}", name));
                let mut flowed = Flow::Norm;
                for s in gbody {
                    match self.exec_stmt(&fenv, s)? {
                        Flow::Norm => {}
                        other => {
                            flowed = other;
                            break;
                        }
                    }
                }
                self.close_timing(&name);
                return Ok(match flowed {
                    Flow::Ret(v) => v,
                    _ => {
                        self.note(
                            dl,
                            4,
                            format!("guard of {} returned null (uORF repression)", name),
                        );
                        Value::Null
                    }
                });
            }
        }
        let result = self.exec_block(&fenv, &def.body);
        self.close_timing(&name);
        match result? {
            Flow::Ret(v) => Ok(v),
            _ => Ok(Value::Null),
        }
    }

    /// dx-r1 (audit W5): clear all profiler accounting so a second execution
    /// (profile re-run) starts from zero without inheriting load-time counts.
    pub fn reset_profile(&mut self) {
        self.call_counts.clear();
        self.call_time.clear();
        self.call_time_self.clear();
        self.call_stack.clear();
        self.call_clock = 0;
    }

    /// Close the timing frame for a gene call: accumulate exclusive (self)
    /// time and hand the inclusive time to the parent's child budget.
    fn close_timing(&mut self, name: &str) {
        if let Some((n, t0, child_acc)) = self.call_stack.pop() {
            if self.profiling {
                let incl = (crate::ffi::now_ns() - t0) / 1000.0;
                let self_us = (incl - child_acc).max(0.0);
                *self.call_time.entry(n.clone()).or_insert(0.0) += incl;
                *self.call_time_self.entry(n.clone()).or_insert(0.0) += self_us;
                if let Some(parent) = self.call_stack.last_mut() {
                    parent.2 += incl; // my inclusive time is my parent's child time
                }
            }
            let _ = (n, child_acc);
        }
        let _ = name;
    }

    /// Pull the next value from a sequence; Ok(None) means exhaustion.
    /// (Straight-line: every branch below terminates the pull — there is no retry.)
    pub fn seq_pull(&mut self, st: &Rc<RefCell<SeqState>>) -> Result<Option<Value>, Stress> {
        let msg = {
            let mut b = st.borrow_mut();
            if b.done {
                return Ok(None);
            }
            if let Some(s) = b.stress.take() {
                b.done = true;
                return Err(Stress::new(&s.0, s.1));
            }
            b.rx.take()
        };
        let rx = match msg {
            Some(rx) => rx,
            None => {
                // another pull already consumed the receiver; done
                let mut b = st.borrow_mut();
                b.done = true;
                return Ok(None);
            }
        };
        let m = rx.recv();
        let mut b = st.borrow_mut();
        match m {
            Ok(crate::value::SeqMsg::Yield(sv)) => {
                b.rx = Some(rx); // put the receiver back for the next pull
                drop(b);
                Ok(Some(crate::genes::from_send(sv)))
            }
            Ok(crate::value::SeqMsg::Done(notes, stress)) => {
                b.done = true;
                for n in notes {
                    let mut n2 = n;
                    n2.message = format!("[seq] {}", n2.message);
                    self.notes.push(n2);
                }
                if let Some((k, m)) = stress {
                    b.stress = Some((k, m));
                    drop(b);
                    return Err(self.seq_stress(st));
                }
                Ok(None)
            }
            Err(_) => {
                b.done = true;
                Ok(None)
            }
        }
    }

    fn seq_stress(&mut self, st: &Rc<RefCell<SeqState>>) -> Stress {
        let s = st.borrow().stress.clone();
        match s {
            Some((k, m)) => Stress::new(&k, m),
            None => Stress::new("unfolded", "sequence stress"),
        }
    }

    /// Build a phenotype instance: parent fields first (differentiation
    /// lineage), then own overrides, then the init method if declared.
    pub fn construct_obj(
        &mut self,
        def: &Arc<PhenoDef>,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        let m: crate::value::MapRef = Rc::new(RefCell::new(crate::value::MapStore::default()));
        // gather the lineage root-first
        let chain = self.pheno_chain(def);
        for d in chain.iter().rev() {
            for (fname, fexpr) in &d.fields {
                let v = self
                    .eval(&self.global.clone(), fexpr)
                    .unwrap_or(Value::Null);
                self.map_insert(&m, Value::Str(fname.clone()), v);
            }
        }
        let obj = Value::Obj(def.clone(), m.clone());
        // constructor: own init, else nearest ancestor's
        for d in chain.iter().rev() {
            if let Some(init) = d.methods.iter().find(|g| g.name.as_deref() == Some("init")) {
                self.call_method_gene(init.clone(), obj.clone(), args)?;
                break;
            }
        }
        Ok(Value::Obj(def.clone(), m))
    }

    /// Inheritance lineage (child → root), capped for cycle safety.
    fn pheno_chain(&self, def: &Arc<PhenoDef>) -> Vec<Arc<PhenoDef>> {
        let mut chain: Vec<Arc<PhenoDef>> = Vec::new();
        let mut cur = Some(def.clone());
        let mut hops = 0;
        while let Some(d) = cur {
            chain.push(d.clone());
            hops += 1;
            if hops > 32 {
                break;
            }
            cur = d.parent.as_ref().and_then(|p| self.phenos.get(p).cloned());
        }
        chain
    }

    /// Call a phenotype method: binds `self` plus params, supports guard, and
    /// records telemetry under `Name.method`.
    pub fn call_method_gene(
        &mut self,
        def: Arc<GeneDef>,
        self_val: Value,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        self.depth += 1;
        if self.depth > self.depth_limit {
            self.depth -= 1;
            return Err(Stress::new(
                "overflow",
                format!("recursion depth limit ({}) exceeded", self.depth_limit),
            ));
        }
        let r = self.call_method_gene_inner(def, self_val, args);
        self.depth -= 1;
        r
    }

    fn call_method_gene_inner(
        &mut self,
        def: Arc<GeneDef>,
        self_val: Value,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        let name = def.name.clone().unwrap_or_else(|| "<method>".into());
        // A13 (dx-r2): gate notes point at the method's definition line
        let dl = def.line;
        // T2b methylation gate for phenotype methods (same contract).
        if !def.acetylate {
            let lvl = *self.methyl_levels.get(&name).unwrap_or(&0);
            if lvl >= self.methyl_threshold {
                self.note(
                    dl,
                    4,
                    format!(
                        "methylation silences: '{}' (level {} >= threshold {}) — call returns null",
                        name, lvl, self.methyl_threshold
                    ),
                );
                return Ok(Value::Null);
            }
        }
        *self.call_counts.entry(name.clone()).or_insert(0) += 1;
        self.call_clock += 1;
        let bucket = self.call_clock / 20;
        *self
            .gene_buckets
            .entry(name.clone())
            .or_default()
            .entry(bucket)
            .or_insert(0) += 1;
        if def.methylate && !self.methyl_quiet && !self.methyl_noted.contains(&name) {
            self.methyl_noted.insert(name.clone());
            self.note(
                dl,
                4,
                format!("methylated call: '{}' (chromatin repressed)", name),
            );
        }
        let fenv = Env::new(Some(self.global.clone()));
        fenv.define("self", self_val);
        for (i, (pname, default)) in def.params.iter().enumerate() {
            if pname.is_empty() || pname == "?" || pname == "self" {
                continue;
            }
            if let Some(a) = args.get(i) {
                fenv.define(pname, a.clone());
            } else if let Some(d) = default {
                let dv = self.eval(&fenv, d).unwrap_or(Value::Null);
                fenv.define(pname, dv);
            } else {
                self.note(
                    dl,
                    4,
                    format!(
                        "missing argument '{}' in call to {}; bound null",
                        pname, name
                    ),
                );
                fenv.define(pname, Value::Null);
            }
        }
        let start = if self.profiling {
            Some(crate::ffi::now_ns())
        } else {
            None
        };
        self.call_stack
            .push((name.clone(), start.unwrap_or(0.0), 0.0));
        if let Some((cond, gbody)) = &def.guard {
            let ok = self.eval(&fenv, cond).map(|v| v.truthy()).unwrap_or(false);
            if !ok {
                self.note(dl, 4, format!("guard tripped calling {}", name));
                let mut flowed = Flow::Norm;
                for s in gbody {
                    match self.exec_stmt(&fenv, s)? {
                        Flow::Norm => {}
                        other => {
                            flowed = other;
                            break;
                        }
                    }
                }
                self.close_timing(&name);
                return Ok(match flowed {
                    Flow::Ret(v) => v,
                    _ => Value::Null,
                });
            }
        }
        let result = self.exec_block(&fenv, &def.body);
        self.close_timing(&name);
        match result? {
            Flow::Ret(v) => Ok(v),
            _ => Ok(Value::Null),
        }
    }

    // ------------------------------------------------------- builtins
    fn call_builtin(
        &mut self,
        env: &Rc<Env>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        match name {
            "promote" => {
                let parts: Vec<String> = args.iter().map(|v| v.display()).collect();
                let line = parts.join(" ");
                if let Some(sink) = self.stdout_sink.clone() {
                    // dx-r3: captured by the host (test runner) — nothing
                    // reaches the report until the host prints it
                    sink.borrow_mut().push(line);
                } else {
                    println!("{}", line);
                }
                Ok(Value::Null)
            }
            "len" => Ok(Value::Int(match args.first() {
                Some(Value::Str(s)) => s.chars().count() as i64,
                Some(Value::List(l)) => l.borrow().len() as i64,
                Some(Value::Map(m)) => m.borrow().len() as i64,
                _ => {
                    self.note(self.cur_line, 4, "len() of non-container is 0");
                    0
                }
            })),
            "push" => {
                if let (Some(Value::List(l)), Some(v)) = (args.first(), args.get(1)) {
                    // sec-r1 (audit C-3): the builtin form MUST be memory-
                    // charged exactly like the method form — an uncharged
                    // growth path is an allocator-abort DoS (rc=134, outside
                    // the catchable-stress contract)
                    let bytes = match v {
                        Value::Str(x) => x.len() as u64 + 24,
                        Value::List(x) => 8 * x.borrow().len() as u64 + 48,
                        _ => 16,
                    };
                    mem_charge(bytes)?;
                    l.borrow_mut().push(v.clone());
                    Ok(Value::List(l.clone()))
                } else {
                    Err(Stress::new("unfolded", "push(list, value) needs a list"))
                }
            }
            "pop" => match args.first() {
                Some(Value::List(l)) => Ok(l.borrow_mut().pop().unwrap_or(Value::Null)),
                _ => Err(Stress::new("unfolded", "pop(list) needs a list")),
            },
            "insert" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::List(l)), Some(i), Some(v)) => {
                    let idx = self.as_index(i, l.borrow().len())?;
                    let idx = idx.min(l.borrow().len());
                    l.borrow_mut().insert(idx, v.clone());
                    Ok(Value::List(l.clone()))
                }
                _ => Err(Stress::new("unfolded", "insert(list, i, v)")),
            },
            "remove" => match (args.first(), args.get(1)) {
                (Some(Value::List(l)), Some(i)) => {
                    let idx = self.as_index(i, l.borrow().len())?;
                    if idx < l.borrow().len() {
                        Ok(l.borrow_mut().remove(idx))
                    } else {
                        Err(Stress::new("missing", "remove index out of range"))
                    }
                }
                _ => Err(Stress::new("unfolded", "remove(list, i)")),
            },
            "keys" => match args.first() {
                Some(Value::Map(m)) => Ok(Value::List(Rc::new(RefCell::new(
                    m.borrow().iter().map(|(k, _)| k.clone()).collect(),
                )))),
                _ => Ok(Value::List(Rc::new(RefCell::new(vec![])))),
            },
            "values" => match args.first() {
                Some(Value::Map(m)) => Ok(Value::List(Rc::new(RefCell::new(
                    m.borrow().iter().map(|(_, v)| v.clone()).collect(),
                )))),
                _ => Ok(Value::List(Rc::new(RefCell::new(vec![])))),
            },
            "has" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(k)) => Ok(Value::Bool(m.borrow().position(k).is_some())),
                _ => Ok(Value::Bool(false)),
            },
            "del" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(k)) => {
                    m.borrow_mut().del(k);
                    Ok(Value::Null)
                }
                _ => Ok(Value::Null),
            },
            "range" => {
                let (a, b, step) = match (args.first(), args.get(1), args.get(2)) {
                    (Some(Value::Int(x)), None, _) => (0, *x, 1),
                    (Some(Value::Int(x)), Some(Value::Int(y)), None) => (*x, *y, 1),
                    (Some(Value::Int(x)), Some(Value::Int(y)), Some(Value::Int(z))) => (*x, *y, *z),
                    _ => {
                        self.note(self.cur_line, 4, "range() needs ints; returned []");
                        (0, 0, 1)
                    }
                };
                if step == 0 {
                    return Err(Stress::new("unfolded", "range step cannot be 0"));
                }
                let mut out = Vec::new();
                let mut i = a;
                while (step > 0 && i < b) || (step < 0 && i > b) {
                    out.push(Value::Int(i));
                    i += step;
                    if out.len() > 10_000_000 {
                        return Err(Stress::new("overflow", "range too large"));
                    }
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "str" => Ok(Value::Str(
                args.first().map(|v| v.display()).unwrap_or_default(),
            )),
            "num" => match args.first() {
                Some(Value::Str(s)) => {
                    let t = s.trim();
                    if let Ok(i) = t.parse::<i64>() {
                        Ok(Value::Int(i))
                    } else if let Ok(f) = t.parse::<f64>() {
                        Ok(Value::Float(f))
                    } else {
                        self.note(self.cur_line, 4, format!("num('{}') failed; 0", t));
                        Ok(Value::Int(0))
                    }
                }
                Some(Value::Int(i)) => Ok(Value::Int(*i)),
                Some(Value::Float(f)) => Ok(Value::Float(*f)),
                _ => Ok(Value::Int(0)),
            },
            "type" => Ok(Value::Str(match args.first() {
                Some(Value::Obj(d, _)) => d.name.clone(), // like Python: instance's class name
                Some(v) => v.type_name().to_string(),
                None => "null".to_string(),
            })),
            "abs" => match args.first() {
                Some(Value::Int(i)) => i
                    .checked_abs()
                    .map(Value::Int)
                    .ok_or_else(|| Stress::new("overflow", "int overflow in abs(i64::MIN)")),
                Some(Value::Float(f)) => Ok(Value::Float(f.abs())),
                _ => Ok(Value::Int(0)),
            },
            "min" => {
                let mut best: Option<Value> = None;
                for a in &args {
                    if let Value::List(l) = a {
                        for v in l.borrow().iter() {
                            best = Some(match best {
                                None => v.clone(),
                                Some(b) => {
                                    if self.compare(v, &b).unwrap_or(std::cmp::Ordering::Equal)
                                        == std::cmp::Ordering::Less
                                    {
                                        v.clone()
                                    } else {
                                        b
                                    }
                                }
                            });
                        }
                    } else {
                        best = Some(match best {
                            None => a.clone(),
                            Some(b) => {
                                if self.compare(a, &b).unwrap_or(std::cmp::Ordering::Equal)
                                    == std::cmp::Ordering::Less
                                {
                                    a.clone()
                                } else {
                                    b
                                }
                            }
                        });
                    }
                }
                Ok(best.unwrap_or(Value::Null))
            }
            "max" => {
                let mut best: Option<Value> = None;
                for a in &args {
                    if let Value::List(l) = a {
                        for v in l.borrow().iter() {
                            best = Some(match best {
                                None => v.clone(),
                                Some(b) => {
                                    if self.compare(v, &b).unwrap_or(std::cmp::Ordering::Equal)
                                        == std::cmp::Ordering::Greater
                                    {
                                        v.clone()
                                    } else {
                                        b
                                    }
                                }
                            });
                        }
                    } else {
                        best = Some(match best {
                            None => a.clone(),
                            Some(b) => {
                                if self.compare(a, &b).unwrap_or(std::cmp::Ordering::Equal)
                                    == std::cmp::Ordering::Greater
                                {
                                    a.clone()
                                } else {
                                    b
                                }
                            }
                        });
                    }
                }
                Ok(best.unwrap_or(Value::Null))
            }
            // ---- L1a: iteration + numeric builtins (oracle mirrors each) ----
            "enumerate" => match args.first() {
                Some(Value::List(l)) => {
                    let out: Vec<Value> = l
                        .borrow()
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            Value::List(Rc::new(RefCell::new(vec![
                                Value::Int(i as i64),
                                v.clone(),
                            ])))
                        })
                        .collect();
                    mem_charge(8 * out.len() as u64 + 48)?;
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                Some(Value::Str(s)) => {
                    let out: Vec<Value> = s
                        .chars()
                        .enumerate()
                        .map(|(i, c)| {
                            Value::List(Rc::new(RefCell::new(vec![
                                Value::Int(i as i64),
                                Value::Str(c.to_string()),
                            ])))
                        })
                        .collect();
                    mem_charge(8 * out.len() as u64 + 48)?;
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "enumerate of a {}; null",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Null)
                }
            },
            "zip" => {
                let a = match args.first() {
                    Some(Value::List(l)) => l.borrow().clone(),
                    _ => {
                        self.note(self.cur_line, 4, "zip needs two lists; null");
                        return Ok(Value::Null);
                    }
                };
                let b = match args.get(1) {
                    Some(Value::List(l)) => l.borrow().clone(),
                    _ => {
                        self.note(self.cur_line, 4, "zip needs two lists; null");
                        return Ok(Value::Null);
                    }
                };
                let n = a.len().min(b.len());
                let out: Vec<Value> = (0..n)
                    .map(|i| Value::List(Rc::new(RefCell::new(vec![a[i].clone(), b[i].clone()]))))
                    .collect();
                mem_charge(8 * out.len() as u64 + 48)?;
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "sorted" => match args.first() {
                Some(Value::List(l)) => {
                    let mut v = l.borrow().clone();
                    match args.get(1) {
                        Some(Value::Gene(_, _)) => {
                            let cmp = args.get(1).unwrap().clone();
                            // insertion sort with user comparator — the exact
                            // `.sort()` contract, non-mutating output
                            for i in 1..v.len() {
                                let mut j = i;
                                while j > 0 {
                                    let a = v[j - 1].clone();
                                    let b = v[j].clone();
                                    let before = self.call_value(env, &cmp, vec![a, b])?.truthy();
                                    if !before {
                                        v.swap(j - 1, j);
                                        j -= 1;
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {
                            let key = |v: &Value| -> (u8, f64, String) {
                                match v {
                                    Value::Bool(b) => (0, *b as i64 as f64, String::new()),
                                    Value::Int(i) => (0, *i as f64, String::new()),
                                    Value::Float(f) => (0, *f, String::new()),
                                    Value::Str(s) => (1, 0.0, s.clone()),
                                    other => (2, 0.0, other.repr()),
                                }
                            };
                            v.sort_by(|a, b| {
                                let (ka, kb) = (key(a), key(b));
                                ka.0.cmp(&kb.0)
                                    .then(
                                        ka.1.partial_cmp(&kb.1)
                                            .unwrap_or(std::cmp::Ordering::Equal),
                                    )
                                    .then_with(|| ka.2.cmp(&kb.2))
                            });
                        }
                    }
                    mem_charge(8 * v.len() as u64 + 48)?;
                    Ok(Value::List(Rc::new(RefCell::new(v))))
                }
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "sorted of a {}; null",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Null)
                }
            },
            "reversed" => match args.first() {
                Some(Value::List(l)) => {
                    let mut v = l.borrow().clone();
                    v.reverse();
                    Ok(Value::List(Rc::new(RefCell::new(v))))
                }
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "reversed of a {}; null",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Null)
                }
            },
            "any" => match args.first() {
                Some(Value::List(l)) => Ok(Value::Bool(l.borrow().iter().any(|v| v.truthy()))),
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "any of a {}; false",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Bool(false))
                }
            },
            "all" => match args.first() {
                Some(Value::List(l)) => Ok(Value::Bool(l.borrow().iter().all(|v| v.truthy()))),
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "all of a {}; false",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Bool(false))
                }
            },
            "first" => match args.first() {
                Some(Value::List(l)) => {
                    if l.borrow().is_empty() {
                        self.note(self.cur_line, 4, "first of an empty list; null");
                        Ok(Value::Null)
                    } else {
                        Ok(l.borrow()[0].clone())
                    }
                }
                Some(Value::Str(s)) => match s.chars().next() {
                    Some(c) => Ok(Value::Str(c.to_string())),
                    None => {
                        self.note(self.cur_line, 4, "first of an empty string; null");
                        Ok(Value::Null)
                    }
                },
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "first of a {}; null",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Null)
                }
            },
            "last" => match args.first() {
                Some(Value::List(l)) => {
                    if l.borrow().is_empty() {
                        self.note(self.cur_line, 4, "last of an empty list; null");
                        Ok(Value::Null)
                    } else {
                        Ok(l.borrow()[l.borrow().len() - 1].clone())
                    }
                }
                Some(Value::Str(s)) => match s.chars().last() {
                    Some(c) => Ok(Value::Str(c.to_string())),
                    None => {
                        self.note(self.cur_line, 4, "last of an empty string; null");
                        Ok(Value::Null)
                    }
                },
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "last of a {}; null",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Null)
                }
            },
            "take" => {
                let n = match args.get(1) {
                    Some(Value::Int(i)) if *i >= 0 => *i as usize,
                    Some(Value::Int(_)) => 0,
                    _ => {
                        self.note(self.cur_line, 4, "take needs a count; null");
                        return Ok(Value::Null);
                    }
                };
                match args.first() {
                    Some(Value::List(l)) => Ok(Value::List(Rc::new(RefCell::new(
                        l.borrow().iter().take(n).cloned().collect(),
                    )))),
                    Some(Value::Str(s)) => Ok(Value::Str(s.chars().take(n).collect())),
                    other => {
                        self.note(
                            self.cur_line,
                            4,
                            format!(
                                "take of a {}; null",
                                other.map(|v| v.type_name()).unwrap_or("null")
                            ),
                        );
                        Ok(Value::Null)
                    }
                }
            }
            "drop" => {
                let n = match args.get(1) {
                    Some(Value::Int(i)) if *i >= 0 => *i as usize,
                    Some(Value::Int(_)) => 0,
                    _ => {
                        self.note(self.cur_line, 4, "drop needs a count; null");
                        return Ok(Value::Null);
                    }
                };
                match args.first() {
                    Some(Value::List(l)) => Ok(Value::List(Rc::new(RefCell::new(
                        l.borrow().iter().skip(n).cloned().collect(),
                    )))),
                    Some(Value::Str(s)) => Ok(Value::Str(s.chars().skip(n).collect())),
                    other => {
                        self.note(
                            self.cur_line,
                            4,
                            format!(
                                "drop of a {}; null",
                                other.map(|v| v.type_name()).unwrap_or("null")
                            ),
                        );
                        Ok(Value::Null)
                    }
                }
            }
            "unique" => match args.first() {
                Some(Value::List(l)) => {
                    let mut out: Vec<Value> = Vec::new();
                    for v in l.borrow().iter() {
                        if !out.iter().any(|u| u.deep_eq(v)) {
                            out.push(v.clone());
                        }
                    }
                    mem_charge(8 * out.len() as u64 + 48)?;
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "unique of a {}; null",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Null)
                }
            },
            "flatten" => match args.first() {
                Some(Value::List(l)) => {
                    let mut out: Vec<Value> = Vec::new();
                    for v in l.borrow().iter() {
                        if let Value::List(inner) = v {
                            out.extend(inner.borrow().iter().cloned());
                        } else {
                            out.push(v.clone());
                        }
                    }
                    mem_charge(8 * out.len() as u64 + 48)?;
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "flatten of a {}; null",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    Ok(Value::Null)
                }
            },
            "chunk" => {
                let n = match args.get(1) {
                    Some(Value::Int(i)) if *i > 0 => *i as usize,
                    Some(Value::Int(_)) => {
                        self.note(self.cur_line, 4, "chunk size must be positive; empty");
                        return Ok(Value::List(Rc::new(RefCell::new(Vec::new()))));
                    }
                    _ => {
                        self.note(self.cur_line, 4, "chunk needs a size; null");
                        return Ok(Value::Null);
                    }
                };
                match args.first() {
                    Some(Value::List(l)) => {
                        let out: Vec<Value> = l
                            .borrow()
                            .chunks(n)
                            .map(|c| Value::List(Rc::new(RefCell::new(c.to_vec()))))
                            .collect();
                        mem_charge(8 * out.len() as u64 + 48)?;
                        Ok(Value::List(Rc::new(RefCell::new(out))))
                    }
                    other => {
                        self.note(
                            self.cur_line,
                            4,
                            format!(
                                "chunk of a {}; null",
                                other.map(|v| v.type_name()).unwrap_or("null")
                            ),
                        );
                        Ok(Value::Null)
                    }
                }
            }
            "round" => {
                let d = match args.get(1) {
                    None => 0u32,
                    Some(Value::Int(n)) if *n >= 0 => *n as u32,
                    Some(Value::Int(_)) => {
                        self.note(self.cur_line, 4, "negative digit count; treated as 0");
                        0u32
                    }
                    Some(_) => {
                        self.note(self.cur_line, 4, "digit count must be an int; treated as 0");
                        0u32
                    }
                };
                match args.first() {
                    Some(Value::Int(i)) => Ok(Value::Int(*i)),
                    Some(Value::Float(f)) => {
                        // half-away-from-zero at the d-th decimal, computed
                        // identically in the oracle (same f64 formula —
                        // bit-identical, like the repressilator mirror)
                        if !f.is_finite() {
                            return Ok(Value::Float(*f));
                        }
                        let scale = 10f64.powi(d as i32);
                        let r = ((f.abs() * scale) + 0.5).floor() / scale * f.signum();
                        if d == 0 {
                            if !r.is_finite()
                                || r >= 9.223372036854776e18
                                || r <= -9.223372036854776e18
                            {
                                return Err(Stress::new(
                                    "overflow",
                                    "float too large for round to int",
                                ));
                            }
                            Ok(Value::Int(r as i64))
                        } else {
                            Ok(Value::Float(r))
                        }
                    }
                    other => {
                        self.note(
                            self.cur_line,
                            4,
                            format!(
                                "round of a {}; 0",
                                other.map(|v| v.type_name()).unwrap_or("null")
                            ),
                        );
                        Ok(Value::Int(0))
                    }
                }
            }
            "clamp" => {
                let num =
                    |v: Option<&Value>| matches!(v, Some(Value::Int(_)) | Some(Value::Float(_)));
                let a = args.first();
                let lo = args.get(1);
                let hi = args.get(2);
                if !(num(a) && num(lo) && num(hi)) {
                    self.note(self.cur_line, 4, "clamp needs three numbers; null");
                    return Ok(Value::Null);
                }
                let v = a.unwrap().clone();
                let lo = lo.unwrap().clone();
                let hi = hi.unwrap().clone();
                if self.compare(&v, &lo).unwrap_or(std::cmp::Ordering::Equal)
                    == std::cmp::Ordering::Less
                {
                    Ok(lo)
                } else if self.compare(&v, &hi).unwrap_or(std::cmp::Ordering::Equal)
                    == std::cmp::Ordering::Greater
                {
                    Ok(hi)
                } else {
                    Ok(v)
                }
            }
            "divmod" => {
                // parity by construction: reuses the `//` and `%` operators
                let a = args.first().cloned().unwrap_or(Value::Null);
                let b = args.get(1).cloned().unwrap_or(Value::Null);
                let q = self.apply_binop(env, BinOp::FloorDiv, &a, &b)?;
                let r = self.apply_binop(env, BinOp::Mod, &a, &b)?;
                Ok(Value::List(Rc::new(RefCell::new(vec![q, r]))))
            }
            "sum" => {
                let mut acc = Value::Int(0);
                if let Some(Value::List(l)) = args.first() {
                    for v in l.borrow().iter() {
                        acc = self.apply_binop(env, BinOp::Add, &acc, v)?;
                    }
                } else {
                    for v in &args {
                        acc = self.apply_binop(env, BinOp::Add, &acc, v)?;
                    }
                }
                Ok(acc)
            }
            "clock" => Ok(Value::Float(crate::ffi::now_ns() / 1e9)),
            "exit" => {
                // sec-r2 (audit C-11): process exit is a capability, not a
                // builtin right. An ungranted exit kills the test runner,
                // the REPL, the LSP — the host — so default-deny applies.
                // sec-r3: decoupled from `enabled` — an embedder that turns
                // the sandbox off wholesale must still grant exit explicitly
                // (only Caps::allow_all() implies it).
                if !self.caps.exit_allowed {
                    self.note(self.cur_line,
                        4,
                        "exit denied: capability 'exit' not granted (grant with --allow-exit or .cell allow.exit = true)",
                    );
                    return Err(Stress::new(
                        "interference",
                        "exit denied — capability 'exit' not granted (grant with --allow-exit or .cell allow.exit = true)",
                    ));
                }
                let code = match args.first() {
                    Some(Value::Int(i)) => *i as i32,
                    _ => 0,
                };
                std::process::exit(code);
            }
            "assert" => {
                let ok = args.first().map(|v| v.truthy()).unwrap_or(false);
                self.asserts_run += 1;
                if !ok {
                    let msg = args
                        .get(1)
                        .map(|v| v.display())
                        .unwrap_or_else(|| "assertion failed".into());
                    return Err(Stress::new("burned", msg));
                }
                Ok(Value::Bool(true))
            }
            "codon" => {
                let s = args.first().map(|v| v.display()).unwrap_or_default();
                Ok(Value::Int(crate::ffi::codon_score(&s) as i64))
            }
            "distance" => {
                let a = args.first().map(|v| v.display()).unwrap_or_default();
                let b = args.get(1).map(|v| v.display()).unwrap_or_default();
                // the DP fallback kernel is O(la*lb): refuse pathological
                // inputs instead of freezing outside the step budget
                if a.len().saturating_mul(b.len()) > 10_000_000 {
                    return Err(Stress::new(
                        "overflow",
                        "distance inputs exceed the 10M-cell DP ceiling",
                    ));
                }
                Ok(Value::Int(crate::ffi::edit_distance(&a, &b) as i64))
            }
            "similar" => {
                let a = args.first().map(|v| v.display()).unwrap_or_default();
                let b = args.get(1).map(|v| v.display()).unwrap_or_default();
                let maxd = match args.get(2) {
                    Some(Value::Int(i)) => *i as i32,
                    _ => 2,
                };
                // sec-r1 (audit C-1): same DP ceiling as distance() — the
                // kernel hop is fuel-blind, so the budget must be checked
                // before the call, not by the caller
                if a.len().saturating_mul(b.len()) > crate::ffi::DP_CELL_BUDGET {
                    return Err(Stress::new(
                        "overflow",
                        "similar inputs exceed the 10M-cell DP ceiling",
                    ));
                }
                // byte-length difference lower-bounds the distance: if the
                // strings differ in length by more than maxd, the kernel
                // cannot possibly return a value <= maxd
                let diff = (a.len() as i64 - b.len() as i64).abs();
                if diff > maxd as i64 {
                    return Ok(Value::Bool(false));
                }
                Ok(Value::Bool(crate::ffi::edit_distance(&a, &b) <= maxd))
            }
            "transcribe" => {
                let s = args.first().map(|v| v.display()).unwrap_or_default();
                let out: String = s
                    .chars()
                    .map(|c| match c {
                        'T' | 't' => 'U',
                        'A' | 'a' => 'A',
                        'G' | 'g' => 'G',
                        'C' | 'c' => 'C',
                        other => other,
                    })
                    .collect();
                Ok(Value::Str(out.to_uppercase()))
            }
            "reverse_complement" => {
                let s = args.first().map(|v| v.display()).unwrap_or_default();
                let mut out: String = s
                    .chars()
                    .rev()
                    .map(|c| match c {
                        'A' | 'a' => 'T',
                        'T' | 't' => 'A',
                        'G' | 'g' => 'C',
                        'C' | 'c' => 'G',
                        other => other,
                    })
                    .collect();
                out.make_ascii_uppercase();
                Ok(Value::Str(out))
            }
            "gc_content" => {
                let s = args
                    .first()
                    .map(|v| v.display())
                    .unwrap_or_default()
                    .to_uppercase();
                let n = s.chars().filter(|c| "ATGC".contains(*c)).count();
                if n == 0 {
                    return Ok(Value::Float(0.0));
                }
                let gc = s.chars().filter(|c| *c == 'G' || *c == 'C').count();
                Ok(Value::Float(gc as f64 * 100.0 / n as f64))
            }
            "translate" => {
                let s = args
                    .first()
                    .map(|v| v.display())
                    .unwrap_or_default()
                    .to_uppercase()
                    .replace('U', "T");
                let mut protein = String::new();
                let chars: Vec<char> = s.chars().collect();
                let mut i = 0;
                while i + 2 < chars.len() + 1 && i + 3 <= chars.len() {
                    let codon: String = chars[i..i + 3].iter().collect();
                    // after U→T normalization, stops are TAA/TAG/TGA
                    if codon == "TAA" || codon == "TAG" || codon == "TGA" {
                        break;
                    }
                    protein.push(codon_table_char(&codon));
                    i += 3;
                }
                Ok(Value::Str(protein))
            }
            "find_orf" => {
                let s = args
                    .first()
                    .map(|v| v.display())
                    .unwrap_or_default()
                    .to_uppercase();
                let orfs = crate::genes::find_orfs(&s);
                Ok(Value::List(Rc::new(RefCell::new(
                    orfs.into_iter().map(Value::Str).collect(),
                ))))
            }
            "memory" => Ok(Value::Map(Rc::new(RefCell::new(
                crate::value::MapStore::from_vec(vec![
                    (
                        Value::Str("arena_bytes".into()),
                        Value::Int(unsafe_arena() as i64),
                    ),
                    (
                        Value::Str("interns".into()),
                        Value::Int(unsafe_interns() as i64),
                    ),
                    (
                        Value::Str("allocs".into()),
                        Value::Int(unsafe_allocs() as i64),
                    ),
                ]),
            )))),
            "methyl" => {
                let k = args.first().map(|v| v.display()).unwrap_or_default();
                let d = args.get(1).cloned().unwrap_or(Value::Null);
                // sec-r3 (re-audit #15): capability grants are the host's
                // business, not the sandboxed program's — redact them
                if k.starts_with("allow.") {
                    return Ok(d);
                }
                Ok(match self.cell.get(&k) {
                    Some(v) => {
                        if v == "true" {
                            Value::Bool(true)
                        } else if v == "false" {
                            Value::Bool(false)
                        } else if let Ok(i) = v.parse::<i64>() {
                            Value::Int(i)
                        } else if let Ok(f) = v.parse::<f64>() {
                            Value::Float(f)
                        } else {
                            Value::Str(v.clone())
                        }
                    }
                    None => d,
                })
            }
            "methylate" => {
                // A12 (reg-r2): runtime methylation — the SAME graded
                // semantics as the @methylate definition attribute (D-005):
                // level += 1, gate applies at the NEXT call. Returns the
                // gene's new level as an Int.
                let k = args.first().map(|v| v.display()).unwrap_or_default();
                // sec-r3 (re-audit #4): the map + its note are allocations —
                // charge them against the 2 GiB ceiling like every other
                // growth; ungranted note spam honors methylate.quiet
                mem_charge((k.len() + 64) as u64)?;
                let (lvl, silenced) = {
                    let lvl = self.methyl_levels.entry(k.clone()).or_insert(0);
                    *lvl += 1;
                    (*lvl, *lvl >= self.methyl_threshold)
                };
                if !self.methyl_quiet {
                    self.note(
                        self.cur_line,
                        2,
                        format!(
                            "methylation deepened: '{}' (level {}) — silenced at the next call if {}",
                            k, lvl, silenced
                        ),
                    );
                }
                Ok(Value::Int(lvl as i64))
            }
            "demethylate" => {
                // A12: the @acetylate direction — saturating relaxation.
                let k = args.first().map(|v| v.display()).unwrap_or_default();
                mem_charge((k.len() + 64) as u64)?;
                let (lvl, silenced) = {
                    let lvl = self.methyl_levels.entry(k.clone()).or_insert(0);
                    *lvl = lvl.saturating_sub(1);
                    (*lvl, *lvl >= self.methyl_threshold)
                };
                if !self.methyl_quiet {
                    self.note(
                        self.cur_line,
                        2,
                        format!(
                            "methylation relaxed: '{}' (level {}) — silenced at the next call if {}",
                            k, lvl, silenced
                        ),
                    );
                }
                Ok(Value::Int(lvl as i64))
            }
            "grn_set" => {
                // A12: write a GRN node's level directly (0..1, clamped).
                // This is how host programs steer gate state at runtime
                // without re-firing the whole network.
                let k = args.first().map(|v| v.display()).unwrap_or_default();
                let v = match args.get(1) {
                    Some(Value::Int(i)) => *i as f64,
                    Some(Value::Float(f)) => *f,
                    _ => 0.0,
                };
                let v = v.clamp(0.0, 1.0);
                mem_charge((k.len() + 64) as u64)?;
                self.grn_levels.insert(k.clone(), v);
                if !self.methyl_quiet {
                    self.note(self.cur_line, 2, format!("grn level set: '{}' = {}", k, v));
                }
                Ok(Value::Float(v))
            }
            "grn_get" => {
                // A12: read a GRN node's level (the write-side counterpart
                // of grn_set; pairs with grn_state() for the whole map).
                let k = args.first().map(|v| v.display()).unwrap_or_default();
                Ok(Value::Float(*self.grn_levels.get(&k).unwrap_or(&0.0)))
            }
            "fingerprint" => {
                let counts: Vec<(Value, Value)> = self
                    .call_counts
                    .iter()
                    .map(|(k, v)| (Value::Str(k.clone()), Value::Int(*v as i64)))
                    .collect();
                // burst index (variance/mean of per-gene call counts over
                // COMPLETE 20-call bins): constitutive genes → 0; the trailing
                // partial bin is excluded so a perfectly periodic pattern
                // scores 0, as the biology demands
                let complete_bins = self.call_clock / 20;
                let mut burst_total = 0.0;
                let mut burst_n = 0usize;
                let mut burst_by_gene: Vec<(Value, Value)> = Vec::new();
                for (g, bins) in &self.gene_buckets {
                    if complete_bins == 0 {
                        burst_by_gene.push((Value::Str(g.clone()), Value::Float(0.0)));
                        continue;
                    }
                    let n = complete_bins as f64;
                    let total: u64 = (0..complete_bins)
                        .map(|b| *bins.get(&b).unwrap_or(&0))
                        .sum();
                    let mean = total as f64 / n;
                    let var = (0..complete_bins)
                        .map(|b| {
                            let c = *bins.get(&b).unwrap_or(&0) as f64;
                            (c - mean).powi(2)
                        })
                        .sum::<f64>()
                        / n;
                    let burst = if mean > 0.0 { var / mean } else { 0.0 };
                    burst_by_gene.push((Value::Str(g.clone()), Value::Float(burst)));
                    burst_total += burst;
                    burst_n += 1;
                }
                burst_by_gene.sort_by(|a, b| match (&a.0, &b.0) {
                    (Value::Str(x), Value::Str(y)) => x.cmp(y),
                    _ => std::cmp::Ordering::Equal,
                });
                let burst_avg = if burst_n > 0 {
                    burst_total / burst_n as f64
                } else {
                    0.0
                };
                // transcript maturation: mature = genes translated at least
                // once; nascent = defined but never called
                let total_defined = self.defined_genes.len();
                let mature = self.call_counts.len().min(total_defined);
                let nascent = total_defined.saturating_sub(mature);
                let maturation = if total_defined > 0 {
                    mature as f64 / total_defined as f64
                } else {
                    0.0
                };
                Ok(Value::Map(Rc::new(RefCell::new(
                    crate::value::MapStore::from_vec(vec![
                        (
                            Value::Str("calls".into()),
                            Value::Map(Rc::new(RefCell::new(crate::value::MapStore::from_vec(
                                counts,
                            )))),
                        ),
                        (Value::Str("burst".into()), Value::Float(burst_avg)),
                        (
                            Value::Str("burst_by_gene".into()),
                            Value::Map(Rc::new(RefCell::new(crate::value::MapStore::from_vec(
                                burst_by_gene,
                            )))),
                        ),
                        (Value::Str("mature".into()), Value::Int(mature as i64)),
                        (Value::Str("nascent".into()), Value::Int(nascent as i64)),
                        (Value::Str("maturation".into()), Value::Float(maturation)),
                    ]),
                ))))
            }
            "toggle_on" => {
                let name = args.first().map(|v| v.display()).unwrap_or_default();
                for (a, b, a_on) in self.toggles.iter_mut() {
                    if a == &name || b == &name {
                        *a_on = a == &name;
                        return Ok(Value::Bool(true));
                    }
                }
                self.note(
                    self.cur_line,
                    4,
                    format!("toggle pair containing '{}' not declared", name),
                );
                Ok(Value::Bool(false))
            }
            "toggle_state" => {
                let mut out = Vec::new();
                for (a, b, a_on) in &self.toggles {
                    out.push((Value::Str(a.clone()), Value::Bool(*a_on)));
                    out.push((Value::Str(b.clone()), Value::Bool(!*a_on)));
                }
                Ok(Value::Map(Rc::new(RefCell::new(
                    crate::value::MapStore::from_vec(out),
                ))))
            }
            "repressi_next" => {
                if self.repressi_ring.is_empty() {
                    self.note(self.cur_line, 4, "no repressilator declared");
                    return Ok(Value::Null);
                }
                self.repressi_i = (self.repressi_i + 1) % self.repressi_ring.len();
                self.repressi_tick += 1;
                Ok(Value::Int(self.repressi_i as i64))
            }
            "repressi_start" => {
                // start the wall-clock oscillator: one thread tick advances the
                // ring each period (deterministic manual rings need no thread)
                let ms = match args.first() {
                    // sec-r3: negative Int must not wrap to a near-infinite sleep
                    Some(Value::Int(i)) => (*i).max(0) as u64,
                    Some(Value::Float(f)) => (*f).max(0.0) as u64,
                    _ => 1000,
                };
                if self.repressi_ring.is_empty() {
                    self.note(
                        self.cur_line,
                        4,
                        "repressi_start: no repressilator ring declared",
                    );
                    return Ok(Value::Bool(false));
                }
                if ms == 0 {
                    self.note(self.cur_line, 4, "repressi_start period must be > 0 ms");
                    return Ok(Value::Bool(false));
                }
                let counter = Arc::new(AtomicU64::new(0));
                let c2 = counter.clone();
                // sec-r3: shared cancellable timer (see Stmt::Repressilator)
                if let Some(flag) = self.repressi_timer_stop.take() {
                    flag.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                let stop = Arc::new(AtomicBool::new(false));
                let stop2 = stop.clone();
                match crate::genes::spawn_worker(move || {
                    while !stop2.load(std::sync::atomic::Ordering::SeqCst) {
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                        c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }) {
                    Ok(()) => {
                        self.repressi_timer_stop = Some(stop);
                    }
                    Err(_) => {
                        self.note(
                            self.cur_line,
                            4,
                            "repressilator timer skipped: thread budget exhausted",
                        );
                        return Ok(Value::Bool(false));
                    }
                }
                self.repressi_atomic = Some(counter);
                self.note(
                    self.cur_line,
                    1,
                    format!("repressilator oscillating every {} ms", ms),
                );
                Ok(Value::Bool(true))
            }
            "repressi_state" => {
                // A11 (reg-r2, honesty): emergent mutual repression. The old
                // "0.5^tj" formula was a hardcoded drive schedule — no gene
                // repressed anything. The ring now integrates the discrete
                // Elowitz–Leibler repressilator (Elowitz & Leibler 2000):
                //   dA/dt = α / (1 + R^h) − γA   (R = the repressor's level)
                // Euler-integrated, 20 substeps of dt=0.05 per ring tick,
                // α=10, γ=1, h=4, init [5, 0, 0]. Each node represses its
                // clockwise neighbor; the oscillation is EMERGENT from that
                // loop, not scheduled. State is a pure function of the tick
                // count — both manual (repressi_next) and wall-clock
                // (repressi_start) modes fold the identical arithmetic, so
                // they can never diverge, and the Python oracle mirrors it
                // op-for-op (bit-identical IEEE-754 results).
                let n = self.repressi_ring.len();
                if n == 0 {
                    return Ok(Value::Map(Rc::new(RefCell::new(
                        crate::value::MapStore::default(),
                    ))));
                }
                let tick: u64 = match &self.repressi_atomic {
                    Some(a) => a.load(std::sync::atomic::Ordering::SeqCst),
                    None => self.repressi_tick,
                };
                // sec-r3: cached + fuel-charged (was O(tick) per read, uncharged)
                let levels = self.ring_advance(n, tick)?;
                let mut out = Vec::new();
                for (name, lvl) in self.repressi_ring.iter().zip(levels.iter()) {
                    out.push((Value::Str(name.clone()), Value::Float(*lvl)));
                }
                Ok(Value::Map(Rc::new(RefCell::new(
                    crate::value::MapStore::from_vec(out),
                ))))
            }
            "grn_fire" => {
                let seed = args.first().map(|v| v.display()).unwrap_or_default();
                // A10 (reg-r2): GRN decay — opt-in dilution. Real regulation decays:
                // levels bleed off unless refreshed. Before every pulse, existing
                // levels decay by the configured fraction. Decay comes from
                // the pulse itself — grn_fire(seed, f) — or falls back to
                // .cell `[grn] decay = f`. Default (unset / 0) is
                // byte-identical to the pre-A10 behavior: the multiply loop
                // is skipped entirely, not multiplied by 1.
                let decay: f64 = match args.get(1) {
                    Some(Value::Int(i)) => (*i as f64).clamp(0.0, 1.0),
                    Some(Value::Float(f)) => f.clamp(0.0, 1.0),
                    _ => self
                        .cell
                        .get("grn.decay")
                        .and_then(|v| v.parse::<f64>().ok())
                        .map(|d| d.clamp(0.0, 1.0))
                        .unwrap_or(0.0),
                };
                if decay > 0.0 && !self.grn_levels.is_empty() {
                    let retention = 1.0 - decay;
                    for lvl in self.grn_levels.values_mut() {
                        *lvl *= retention;
                        if *lvl < f64::EPSILON {
                            *lvl = 0.0;
                        }
                    }
                }
                // STATEFUL network: levels persist across fires (a latch by default — decay makes the dilution explicit)
                for e in &self.grn_edges {
                    self.grn_levels.entry(e.from.clone()).or_insert(0.0);
                    self.grn_levels.entry(e.to.clone()).or_insert(0.0);
                }
                let cur = *self.grn_levels.get(&seed).unwrap_or(&0.0);
                self.grn_levels.insert(seed.clone(), (cur + 1.0).min(1.0));
                // Phase 1 — activation: child = max(child, parent × strength^wave)
                // where wave counts propagation hops (multi-hop attenuation).
                // Phase 2 — inhibition: each repressor applies its influence
                // ONCE per fire (a repressor's concentration sets the output
                // level; it does not compound over propagation waves).
                let mut activated: HashMap<String, f64> = self.grn_levels.clone();
                for _wave in 1..=10 {
                    let mut changed = false;
                    let snapshot = activated.clone();
                    for e in &self.grn_edges {
                        if e.inhibit {
                            continue; // inhibitors propagate in phase 2
                        }
                        let parent = *snapshot.get(&e.from).unwrap_or(&0.0);
                        if parent <= 0.0 {
                            continue;
                        }
                        let influence = match e.threshold {
                            Some(t) if t > 0.0 => {
                                let p2 = parent * parent;
                                let t2 = t * t;
                                e.strength * (p2 / (p2 + t2))
                            }
                            // sec-r3/reg-r3: repeated multiplication, NOT
                            // powi — powi's algorithm is platform-chosen,
                            // while the oracle must mirror this op-for-op
                            // (bit-identical IEEE-754 parity contract)
                            _ => {
                                let mut s = e.strength;
                                let mut k = _wave;
                                while k > 1 {
                                    s *= e.strength;
                                    k -= 1;
                                }
                                parent * s
                            }
                        };
                        let cur = *activated.get(&e.to).unwrap_or(&0.0);
                        let next = cur.max(influence);
                        if (next - cur).abs() > 1e-12 {
                            activated.insert(e.to.clone(), next);
                            changed = true;
                        }
                    }
                    if !changed {
                        break;
                    }
                }
                // phase 2: inhibition subtracts once, based on the source's
                // post-activation level
                let mut inhibited: HashMap<String, f64> = activated.clone();
                for e in &self.grn_edges {
                    if !e.inhibit {
                        continue;
                    }
                    let parent = *activated.get(&e.from).unwrap_or(&0.0);
                    if parent <= 0.0 {
                        continue;
                    }
                    let influence = match e.threshold {
                        Some(t) if t > 0.0 => {
                            let p2 = parent * parent;
                            let t2 = t * t;
                            e.strength * (p2 / (p2 + t2))
                        }
                        _ => parent * e.strength,
                    };
                    let cur = *inhibited.get(&e.to).unwrap_or(&0.0);
                    let next = (cur - influence).max(0.0);
                    inhibited.insert(e.to.clone(), next);
                }
                self.grn_levels = inhibited;
                Ok(Value::Map(Rc::new(RefCell::new(
                    self.grn_levels
                        .iter()
                        .map(|(k, v)| (Value::Str(k.clone()), Value::Float(*v)))
                        .collect(),
                ))))
            }
            "grn_state" => Ok(Value::Map(Rc::new(RefCell::new(
                self.grn_levels
                    .iter()
                    .map(|(k, v)| (Value::Str(k.clone()), Value::Float(*v)))
                    .collect(),
            )))),
            "items" => match args.first() {
                Some(Value::Map(m)) => Ok(Value::List(Rc::new(RefCell::new(
                    m.borrow()
                        .iter()
                        .map(|(k, v)| {
                            Value::List(Rc::new(RefCell::new(vec![k.clone(), v.clone()])))
                        })
                        .collect(),
                )))),
                _ => Ok(Value::List(Rc::new(RefCell::new(vec![])))),
            },
            "spawn" => {
                let callee = args.first().cloned().unwrap_or(Value::Null);
                let task_args = match args.get(1) {
                    Some(Value::List(l)) => l.borrow().clone(),
                    _ => Vec::new(),
                };
                crate::genes::spawn_task(self, callee, task_args)
            }
            "join" => {
                let id = match args.first() {
                    Some(Value::Int(i)) => *i,
                    _ => -1,
                };
                let timeout = match args.get(1) {
                    Some(Value::Int(i)) if *i > 0 => Some(*i as u64),
                    Some(Value::Float(f)) if *f > 0.0 => Some(*f as u64),
                    _ => None,
                };
                crate::genes::join_task(self, id, timeout)
            }
            // -------------------------------------------------- math
            "floor" => Ok(Value::Int(match args.first() {
                Some(Value::Int(i)) => *i,
                Some(Value::Float(f)) => {
                    // a float outside i64 range has no faithful int form —
                    // catchable overflow (the oracle agrees, i64 is the contract)
                    if !f.is_finite() || *f >= 9.223372036854776e18 || *f <= -9.223372036854776e18 {
                        return Err(Stress::new(
                            "overflow",
                            "float too large for floor/ceil to int",
                        ));
                    }
                    f.floor() as i64
                }
                other => {
                    self.note(
                        self.cur_line,
                        4,
                        format!(
                            "floor of a {} value; 0",
                            other.map(|v| v.type_name()).unwrap_or("null")
                        ),
                    );
                    0
                }
            })),
            "ceil" => Ok(Value::Int(match args.first() {
                Some(Value::Int(i)) => *i,
                Some(Value::Float(f)) => {
                    if !f.is_finite() || *f >= 9.223372036854776e18 || *f <= -9.223372036854776e18 {
                        return Err(Stress::new(
                            "overflow",
                            "float too large for floor/ceil to int",
                        ));
                    }
                    f.ceil() as i64
                }
                _ => 0,
            })),
            "sqrt" => {
                let (a, _) = match args.first() {
                    Some(v) => self.as_floats(v, &Value::Int(1))?,
                    None => (0.0, 1.0),
                };
                if a < 0.0 {
                    return Err(Stress::new("unfolded", "sqrt of negative number"));
                }
                Ok(Value::Float(a.sqrt()))
            }
            "pow" => {
                let (a, b) = match (args.first(), args.get(1)) {
                    (Some(x), Some(y)) => self.as_floats(x, y)?,
                    _ => {
                        self.note(self.cur_line, 4, "pow(x, y) needs two numbers; 0");
                        (0.0, 0.0)
                    }
                };
                Ok(Value::Float(a.powf(b)))
            }
            // ------------------------------------------------ regex (ReDoS-safe: 2M step cap)
            "re_match" => {
                // re_match(pattern, s) — full match test
                let pat = args.first().map(|v| v.display()).unwrap_or_default();
                let s = args.get(1).map(|v| v.display()).unwrap_or_default();
                let engine = re_compile(&pat)?;
                let chars: Vec<char> = s.chars().collect();
                let (ok, ovf, _, _) = engine.run_at(&chars, 0);
                if ovf {
                    return Err(Stress::new(
                        "overflow",
                        "regex backtracking exceeded 2M steps",
                    ));
                }
                Ok(Value::Bool(ok))
            }
            "re_find" => {
                // re_find(pattern, s, start?) — leftmost match as a map
                let pat = args.first().map(|v| v.display()).unwrap_or_default();
                let s = args.get(1).map(|v| v.display()).unwrap_or_default();
                let start = match args.get(2) {
                    Some(Value::Int(i)) => (*i).max(0) as usize,
                    _ => 0,
                };
                let engine = re_compile(&pat)?;
                let chars: Vec<char> = s.chars().collect();
                match engine.search(&chars, start.min(chars.len())) {
                    None => Ok(Value::Null),
                    Some((st, en, caps, ovf)) => {
                        if ovf {
                            return Err(Stress::new(
                                "overflow",
                                "regex backtracking exceeded 2M steps",
                            ));
                        }
                        let text: String = chars[st..en].iter().collect();
                        let groups: Vec<Value> = caps
                            .iter()
                            .map(|g| match g {
                                Some((a, b)) => Value::Str(chars[*a..*b].iter().collect()),
                                None => Value::Null,
                            })
                            .collect();
                        Ok(Value::Map(Rc::new(RefCell::new(
                            crate::value::MapStore::from_vec(vec![
                                (Value::Str("text".into()), Value::Str(text)),
                                (Value::Str("start".into()), Value::Int(st as i64)),
                                (Value::Str("end".into()), Value::Int(en as i64)),
                                (
                                    Value::Str("groups".into()),
                                    Value::List(Rc::new(RefCell::new(groups))),
                                ),
                            ]),
                        ))))
                    }
                }
            }
            "re_groups" => {
                let pat = args.first().map(|v| v.display()).unwrap_or_default();
                let s = args.get(1).map(|v| v.display()).unwrap_or_default();
                let engine = re_compile(&pat)?;
                let chars: Vec<char> = s.chars().collect();
                match engine.search(&chars, 0) {
                    None => Ok(Value::Null),
                    Some((_, _, caps, ovf)) => {
                        if ovf {
                            return Err(Stress::new(
                                "overflow",
                                "regex backtracking exceeded 2M steps",
                            ));
                        }
                        let groups: Vec<Value> = caps
                            .iter()
                            .map(|g| match g {
                                Some((a, b)) => Value::Str(chars[*a..*b].iter().collect()),
                                None => Value::Null,
                            })
                            .collect();
                        Ok(Value::List(Rc::new(RefCell::new(groups))))
                    }
                }
            }
            // ------------------------------------------------ date / time (UTC civil calendar)
            "unix_time" => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                Ok(Value::Int(now))
            }
            "date_parts" => {
                let ts = match args.first() {
                    Some(Value::Int(i)) => *i,
                    Some(Value::Float(f)) => *f as i64,
                    _ => {
                        return Err(Stress::new(
                            "unfolded",
                            "date_parts(ts) needs a unix timestamp",
                        ));
                    }
                };
                let days = ts.div_euclid(86_400);
                let secs = ts.rem_euclid(86_400);
                let (y, m, d) = civil_from_days(days);
                let wday = (days + 4).rem_euclid(7); // 1970-01-01 = Thursday(4), Sunday=0
                Ok(Value::Map(Rc::new(RefCell::new(
                    crate::value::MapStore::from_vec(vec![
                        (Value::Str("year".into()), Value::Int(y)),
                        (Value::Str("month".into()), Value::Int(m as i64)),
                        (Value::Str("day".into()), Value::Int(d as i64)),
                        (Value::Str("hour".into()), Value::Int(secs / 3600)),
                        (Value::Str("min".into()), Value::Int((secs % 3600) / 60)),
                        (Value::Str("sec".into()), Value::Int(secs % 60)),
                        (Value::Str("wday".into()), Value::Int(wday)),
                    ]),
                ))))
            }
            "date_fmt" => {
                let ts = match args.first() {
                    Some(Value::Int(i)) => *i,
                    Some(Value::Float(f)) => *f as i64,
                    _ => {
                        return Err(Stress::new(
                            "unfolded",
                            "date_fmt(ts, fmt) needs a unix timestamp",
                        ));
                    }
                };
                let fmt = args
                    .get(1)
                    .map(|v| v.display())
                    .unwrap_or_else(|| "%Y-%m-%d %H:%M:%S".into());
                let days = ts.div_euclid(86_400);
                let secs = ts.rem_euclid(86_400);
                let (y, m, d) = civil_from_days(days);
                let hh = secs / 3600;
                let mi = (secs % 3600) / 60;
                let ss = secs % 60;
                let mut out = String::new();
                let mut it = fmt.chars().peekable();
                while let Some(c) = it.next() {
                    if c == '%' {
                        match it.next() {
                            Some('Y') => out.push_str(&format!("{:04}", y)),
                            Some('m') => out.push_str(&format!("{:02}", m)),
                            Some('d') => out.push_str(&format!("{:02}", d)),
                            Some('H') => out.push_str(&format!("{:02}", hh)),
                            Some('M') => out.push_str(&format!("{:02}", mi)),
                            Some('S') => out.push_str(&format!("{:02}", ss)),
                            Some('%') => out.push('%'),
                            Some(other) => {
                                out.push('%');
                                out.push(other);
                            }
                            None => out.push('%'),
                        }
                    } else {
                        out.push(c);
                    }
                }
                Ok(Value::Str(out))
            }
            "random" => {
                // xorshift64* — identical state machine in both implementations
                let mut x = self.rng;
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                self.rng = x;
                match args.first() {
                    Some(Value::Int(n)) if *n > 0 => {
                        let r = x.wrapping_mul(0x2545F4914F6CDD1D);
                        Ok(Value::Int((r % (*n as u64)) as i64))
                    }
                    _ => {
                        let f = ((x >> 11) as f64) / 9_007_199_254_740_992.0; // 2^53
                        Ok(Value::Float(f))
                    }
                }
            }
            "randomize" => {
                let s = match args.first() {
                    Some(Value::Int(i)) => *i as u64,
                    Some(Value::Float(f)) => (*f as i64) as u64,
                    _ => 0x9E3779B97F4A7C15,
                };
                self.rng = if s == 0 { 0x9E3779B97F4A7C15 } else { s };
                Ok(Value::Null)
            }
            "chr" => Ok(Value::Str(match args.first() {
                Some(Value::Int(i)) if *i >= 0 && *i <= 0x10FFFF => char::from_u32(*i as u32)
                    .map(|c| c.to_string())
                    .unwrap_or_default(),
                _ => String::new(),
            })),
            "ord" => Ok(Value::Int(match args.first() {
                Some(Value::Str(s)) => s.chars().next().map(|c| c as i64).unwrap_or(0),
                _ => 0,
            })),
            "now" => Ok(Value::Float(crate::ffi::now_ns() / 1e9)),
            "sleep" => {
                let ms = match args.first() {
                    Some(Value::Int(i)) => (*i).max(0) as u64,
                    Some(Value::Float(f)) => (*f).max(0.0) as u64,
                    _ => 0,
                };
                // wall-clock ceiling: sleep escapes the step budget, so cap it
                let ms = ms.min(60_000);
                // fuel charge: a sleeping worker drains the shared pool in
                // proportion to its wall time — sleep-loops cannot run forever
                let charge = ms.saturating_mul(1000);
                self.steps = self.steps.saturating_add(charge);
                if let Some(pool) = &self.fuel_pool {
                    let left = pool.fetch_sub(charge as i64, std::sync::atomic::Ordering::Relaxed);
                    if left <= charge as i64 {
                        return Err(Stress::new(
                            "overflow",
                            "run-wide step budget exhausted (sleep)",
                        ));
                    }
                }
                if self.steps > self.step_budget {
                    return Err(Stress::new("overflow", "step budget exhausted (sleep)"));
                }
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Ok(Value::Null)
            }
            "argv" => Ok(Value::List(Rc::new(RefCell::new(
                self.cli_args
                    .iter()
                    .map(|a| Value::Str(a.clone()))
                    .collect(),
            )))),
            // -------------------------------------------------- filesystem (capability-gated)
            // open the CANONICALIZED path: what we checked is what we touch
            // (closes the check/open race on symlink flips)
            "read_file" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.read, "read", &path)?;
                let opened = std::fs::canonicalize(&path)
                    .unwrap_or_else(|_| std::path::PathBuf::from(&path));
                match std::fs::read_to_string(&opened) {
                    Ok(s) => Ok(Value::Str(s)),
                    Err(e) => Err(Stress::new(
                        "missing",
                        format!("read_file '{}': {}", path, e),
                    )),
                }
            }
            "write_file" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                let body = args.get(1).map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.write, "write", &path)?;
                let opened = std::fs::canonicalize(&path)
                    .unwrap_or_else(|_| std::path::PathBuf::from(&path));
                // hardlink defense: refuse to overwrite shared inodes
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if let Ok(m) = std::fs::metadata(&opened) {
                        if m.nlink() > 1 {
                            return Err(Stress::new(
                                "interference",
                                format!(
                                    "write_file '{}': refused — path is a hardlink ({} links)",
                                    path,
                                    m.nlink()
                                ),
                            ));
                        }
                    }
                }
                match std::fs::write(&opened, body) {
                    Ok(()) => Ok(Value::Bool(true)),
                    Err(e) => Err(Stress::new(
                        "missing",
                        format!("write_file '{}': {}", path, e),
                    )),
                }
            }
            "append_file" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                let body = args.get(1).map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.write, "write", &path)?;
                let opened = std::fs::canonicalize(&path)
                    .unwrap_or_else(|_| std::path::PathBuf::from(&path));
                // hardlink defense (parity with write_file — S4 NEW-3)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if let Ok(m) = std::fs::metadata(&opened) {
                        if m.nlink() > 1 {
                            return Err(Stress::new(
                                "interference",
                                format!(
                                    "append_file '{}': refused — path is a hardlink ({} links)",
                                    path,
                                    m.nlink()
                                ),
                            ));
                        }
                    }
                }
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&opened)
                {
                    Ok(mut f) => {
                        use std::io::Write;
                        match f.write_all(body.as_bytes()) {
                            Ok(()) => Ok(Value::Bool(true)),
                            Err(e) => Err(Stress::new(
                                "missing",
                                format!("append_file '{}': {}", path, e),
                            )),
                        }
                    }
                    Err(e) => Err(Stress::new(
                        "missing",
                        format!("append_file '{}': {}", path, e),
                    )),
                }
            }
            "exists" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.read, "read", &path)?;
                Ok(Value::Bool(std::path::Path::new(&path).exists()))
            }
            "file_size" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.read, "read", &path)?;
                match std::fs::metadata(&path) {
                    Ok(m) => Ok(Value::Int(m.len() as i64)),
                    Err(_) => Ok(Value::Int(-1)),
                }
            }
            "read_dir" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.read, "read", &path)?;
                match std::fs::read_dir(&path) {
                    Ok(rd) => {
                        let mut names: Vec<Value> = Vec::new();
                        for e in rd.filter_map(|e| e.ok()) {
                            names.push(Value::Str(e.file_name().to_string_lossy().to_string()));
                        }
                        names.sort_by(|a, b| match (a, b) {
                            (Value::Str(x), Value::Str(y)) => x.cmp(y),
                            _ => std::cmp::Ordering::Equal,
                        });
                        Ok(Value::List(Rc::new(RefCell::new(names))))
                    }
                    Err(e) => Err(Stress::new(
                        "missing",
                        format!("read_dir '{}': {}", path, e),
                    )),
                }
            }
            // -------------------------------------------------- process / net (capability-gated)
            "run" => {
                let prog = args.first().map(|v| v.display()).unwrap_or_default();
                let prog_args: Vec<String> = match args.get(1) {
                    Some(Value::List(l)) => l.borrow().iter().map(|v| v.display()).collect(),
                    _ => Vec::new(),
                };
                self.caps.check(&self.caps.run, "run", &prog)?;
                // safe-base environment: children get the OS essentials plus
                // explicitly env-granted variables — never the whole parent
                // environment (secrets like CI tokens cannot leak to effects)
                let mut cmd = std::process::Command::new(&prog);
                cmd.args(&prog_args).env_clear();
                for (k, v) in std::env::vars_os() {
                    let key = k.to_string_lossy().to_string();
                    let essential = matches!(
                        key.as_str(),
                        "PATH"
                            | "HOME"
                            | "LANG"
                            | "TMPDIR"
                            | "USER"
                            | "SystemRoot"
                            | "SystemDrive"
                            | "COMSPEC"
                            | "PATHEXT"
                            | "WINDIR"
                            | "TEMP"
                            | "TMP"
                            | "APPDATA"
                            | "LOCALAPPDATA"
                            | "PROGRAMFILES"
                            | "PROGRAMDATA"
                            | "USERPROFILE"
                    );
                    if essential || self.caps.env.iter().any(|e| e == &key) {
                        cmd.env(k, v);
                    }
                }
                // sec-r2 (audit A14): a child that never exits used to freeze
                // the interpreter forever (run("sleep", ["10000"]) was a
                // whole-program DoS). Children now run under a wall-clock
                // timeout: .cell `run.timeout_ms`, clamped 1..300_000,
                // default 10_000. A timed-out child is killed and reported.
                // sec-r3 (re-audit #1/#5): two more containment holes closed —
                //   (a) the post-exit pipe drain moved OFF the wait path:
                //       reader threads drain stdout/stderr concurrently, so a
                //       grandchild inheriting the pipe write-end cannot extend
                //       the freeze past the timeout (bounded by DRAIN_WAIT);
                //   (b) the child's WALL TIME is charged as fuel exactly like
                //       sleep (1000 steps/ms) — `while true { run("sleep",…) }`
                //       used to buy unbounded wall time at ~40 fuel steps per
                //       8 s; now it drains the shared pool like sleep does.
                let timeout_ms = self
                    .cell
                    .get("run.timeout_ms")
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(10_000)
                    .clamp(1, 300_000);
                let started = std::time::Instant::now();
                let run_result = cmd
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .map(|mut child| {
                        // sec-r3: concurrent drainers on channels — the run
                        // collects with a BOUNDED receive, so a grandchild
                        // inheriting the pipe write-end cannot extend the
                        // freeze past the timeout (the reader thread itself
                        // dies whenever the pipe finally closes, harmless).
                        use std::sync::mpsc;
                        let (tx_so, rx_so) = mpsc::channel::<Vec<u8>>();
                        let (tx_se, rx_se) = mpsc::channel::<Vec<u8>>();
                        if let Some(mut p) = child.stdout.take() {
                            std::thread::spawn(move || {
                                let mut v = Vec::new();
                                use std::io::Read;
                                let _ = p.read_to_end(&mut v);
                                let _ = tx_so.send(v);
                            });
                        }
                        if let Some(mut p) = child.stderr.take() {
                            std::thread::spawn(move || {
                                let mut v = Vec::new();
                                use std::io::Read;
                                let _ = p.read_to_end(&mut v);
                                let _ = tx_se.send(v);
                            });
                        }
                        (child, rx_so, rx_se)
                    })
                    .and_then(|(mut child, rx_so, rx_se)| {
                        let start = std::time::Instant::now();
                        let mut timed_out = false;
                        let status = loop {
                            match child.try_wait() {
                                Ok(Some(st)) => break st,
                                Ok(None) => {
                                    if start.elapsed().as_millis() as u64 >= timeout_ms {
                                        let _ = child.kill();
                                        let _ = child.wait();
                                        timed_out = true;
                                        break std::process::ExitStatus::default();
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(5));
                                }
                                Err(e) => return Err(e),
                            }
                        };
                        // bounded drain collection: whatever arrived within
                        // the grace window is the output; the rest is dropped
                        const DRAIN_WAIT: std::time::Duration =
                            std::time::Duration::from_millis(250);
                        let so_raw = rx_so.recv_timeout(DRAIN_WAIT).unwrap_or_default();
                        let se_raw = rx_se.recv_timeout(DRAIN_WAIT).unwrap_or_default();
                        Ok((status, so_raw, se_raw, timed_out))
                    });
                match run_result {
                    Ok((status, so_raw, se_raw, timed_out)) => {
                        // sec-r3 (re-audit #5): the child's wall time is fuel,
                        // exactly like sleep (1000 steps/ms) — shelling a
                        // sleep out to a child no longer bypasses the charge.
                        let wall_ms = started.elapsed().as_millis() as u64;
                        let charge = wall_ms.saturating_mul(1000);
                        self.steps = self.steps.saturating_add(charge);
                        if let Some(pool) = &self.fuel_pool {
                            let left =
                                pool.fetch_sub(charge as i64, std::sync::atomic::Ordering::Relaxed);
                            if left <= charge as i64 {
                                return Err(Stress::new(
                                    "overflow",
                                    "run-wide step budget exhausted (run child wall time)",
                                ));
                            }
                        }
                        if self.steps > self.step_budget {
                            return Err(Stress::new(
                                "overflow",
                                "step budget exhausted (run child wall time)",
                            ));
                        }
                        if timed_out {
                            self.note(self.cur_line,
                                4,
                                format!(
                                    "run '{}': killed after {} ms (timeout; set .cell run.timeout_ms)",
                                    prog, timeout_ms
                                ),
                            );
                        }
                        let stdout = String::from_utf8_lossy(&so_raw).to_string();
                        let stderr = String::from_utf8_lossy(&se_raw).to_string();
                        Ok(Value::Map(Rc::new(RefCell::new(
                            crate::value::MapStore::from_vec(vec![
                                (
                                    Value::Str("code".into()),
                                    Value::Int(if timed_out {
                                        -1
                                    } else {
                                        status.code().unwrap_or(-1) as i64
                                    }),
                                ),
                                (Value::Str("stdout".into()), Value::Str(stdout)),
                                (Value::Str("stderr".into()), Value::Str(stderr)),
                                (
                                    Value::Str("ok".into()),
                                    Value::Bool(!timed_out && status.success()),
                                ),
                            ]),
                        ))))
                    }
                    Err(e) => Err(Stress::new("missing", format!("run '{}': {}", prog, e))),
                }
            }
            "http_get" => {
                let host = args.first().map(|v| v.display()).unwrap_or_default();
                let port = match args.get(1) {
                    Some(Value::Int(p)) => *p as u16,
                    _ => 80,
                };
                let path = args
                    .get(2)
                    .map(|v| v.display())
                    .unwrap_or_else(|| "/".into());
                let target = format!("{}:{}", host, port);
                self.caps.check(&self.caps.net, "net", &target)?;
                match http_get(&host, port, &path) {
                    Ok(body) => Ok(Value::Str(body)),
                    Err(e) => Err(Stress::new(
                        "missing",
                        format!("http_get '{}': {}", target, e),
                    )),
                }
            }
            "serve" => {
                let port = match args.first() {
                    Some(Value::Int(p)) => *p as u16,
                    _ => 8080,
                };
                let target = format!("127.0.0.1:{}", port);
                self.caps.check(&self.caps.net, "net", &target)?;
                match crate::interp::serve_start(port) {
                    Ok(()) => Ok(Value::Bool(true)),
                    Err(e) => Err(Stress::new(
                        "missing",
                        format!("serve port {}: {}", port, e),
                    )),
                }
            }
            "recv_request" => {
                // poll the shared request queue: {conn, method, path, body} or null
                match crate::interp::recv_request() {
                    Some((conn, method, path, body)) => Ok(Value::Map(Rc::new(RefCell::new(
                        crate::value::MapStore::from_vec(vec![
                            (Value::Str("conn".into()), Value::Int(conn as i64)),
                            (Value::Str("method".into()), Value::Str(method)),
                            (Value::Str("path".into()), Value::Str(path)),
                            (Value::Str("body".into()), Value::Str(body)),
                        ]),
                    )))),
                    None => Ok(Value::Null),
                }
            }
            "send_response" => {
                let conn = match args.first() {
                    Some(Value::Int(i)) => *i as u64,
                    _ => 0,
                };
                let status = args
                    .get(1)
                    .map(|v| v.display())
                    .unwrap_or_else(|| "200".into());
                let ctype = args
                    .get(2)
                    .map(|v| v.display())
                    .unwrap_or_else(|| "text/html".into());
                let body = args.get(3).map(|v| v.display()).unwrap_or_default();
                // sec-r3 (re-audit #6): response splitting — the server-side
                // twin of the http_get CRLF guard. status/ctype are formatted
                // raw into the response head, so CR/LF/control bytes in them
                // inject headers (proxy/cache poisoning when fronted).
                let header_safe = |s: &str| !s.chars().any(|c| (c as u32) < 0x20 || c == '\x7f');
                if !header_safe(&status) || !header_safe(&ctype) {
                    return Err(Stress::new(
                        "interference",
                        "send_response: status/content-type contain control characters (header injection refused)",
                    ));
                }
                match crate::interp::send_response(conn, &status, &ctype, &body) {
                    Ok(()) => Ok(Value::Bool(true)),
                    Err(e) => Err(Stress::new("missing", format!("send_response: {}", e))),
                }
            }
            // -------------------------------------------------- json
            "json_parse" => {
                let s = args.first().map(|v| v.display()).unwrap_or_default();
                match json_parse(&s) {
                    Ok(v) => Ok(v),
                    Err(e) => Err(Stress::new("unfolded", format!("json_parse: {}", e))),
                }
            }
            "json_str" => {
                let v = args.first().cloned().unwrap_or(Value::Null);
                Ok(Value::Str(json_stringify(&v)))
            }
            // -------------------------------------------------- env (capability-gated)
            "env" => {
                let name = args.first().map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.env, "env", &name)?;
                Ok(match std::env::var(&name) {
                    Ok(v) => Value::Str(v),
                    Err(_) => Value::Null,
                })
            }
            "call" => {
                // dynamic dispatch: call(name_or_gene, args_list) — the
                // ribosome translating a named transcript on demand
                let target = args.first().cloned().unwrap_or(Value::Null);
                let call_args: Vec<Value> = match args.get(1) {
                    Some(Value::List(l)) => l.borrow().clone(),
                    _ => Vec::new(),
                };
                match &target {
                    Value::Str(name) => {
                        if BUILTIN_SYNONYMS.iter().any(|(s, _)| s == name)
                            || BUILTIN_NAMES.contains(&name.as_str())
                        {
                            return self.call_builtin(env, name, call_args);
                        }
                        let v = env.get(name).unwrap_or(Value::Null);
                        self.call_value(env, &v, call_args)
                    }
                    other => self.call_value(env, other, call_args),
                }
            }
            _ => {
                self.note(
                    self.cur_line,
                    4,
                    format!("unknown builtin '{}'; null", name),
                );
                Ok(Value::Null)
            }
        }
    }

    // ------------------------------------------------------- methods
    fn call_method(
        &mut self,
        env: &Rc<Env>,
        recv: Value,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, Stress> {
        // fate instances intercept first
        if let Value::Map(m) = &recv {
            let is_fate = m
                .borrow()
                .iter()
                .any(|(k, _)| matches!(k, Value::Str(s) if s == "#fate"));
            if is_fate {
                let handled: Option<Result<Value, Stress>> = match name {
                    "shift" => {
                        let target = args.first().map(|v| v.display()).unwrap_or_default();
                        let fate_name = m
                            .borrow()
                            .iter()
                            .find(|(k, _)| matches!(k, Value::Str(s) if s == "#fate"))
                            .map(|(_, v)| v.display())
                            .unwrap_or_default();
                        let cur = m
                            .borrow()
                            .iter()
                            .find(|(k, _)| matches!(k, Value::Str(s) if s == "#state"))
                            .map(|(_, v)| v.display())
                            .unwrap_or_default();
                        let allowed = self
                            .fates
                            .get(&fate_name)
                            .and_then(|d| d.states.iter().find(|(s, _)| s == &cur))
                            .map(|(_, tg)| tg.iter().any(|t| t == &target))
                            .unwrap_or(false);
                        if allowed {
                            self.map_insert(m, Value::Str("#state".into()), Value::Str(target));
                            Some(Ok(Value::Bool(true)))
                        } else {
                            self.note(
                                0,
                                4,
                                format!(
                                    "fate {}: '{}' → '{}' crosses a valley; state held",
                                    fate_name, cur, target
                                ),
                            );
                            Some(Ok(Value::Bool(false)))
                        }
                    }
                    "state" => Some(Ok(m
                        .borrow()
                        .iter()
                        .find(|(k, _)| matches!(k, Value::Str(s) if s == "#state"))
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Null))),
                    "can" => {
                        let target = args.first().map(|v| v.display()).unwrap_or_default();
                        let fate_name = m
                            .borrow()
                            .iter()
                            .find(|(k, _)| matches!(k, Value::Str(s) if s == "#fate"))
                            .map(|(_, v)| v.display())
                            .unwrap_or_default();
                        let cur = m
                            .borrow()
                            .iter()
                            .find(|(k, _)| matches!(k, Value::Str(s) if s == "#state"))
                            .map(|(_, v)| v.display())
                            .unwrap_or_default();
                        Some(Ok(Value::Bool(
                            self.fates
                                .get(&fate_name)
                                .and_then(|d| d.states.iter().find(|(s, _)| s == &cur))
                                .map(|(_, tg)| tg.iter().any(|t| t == &target))
                                .unwrap_or(false),
                        )))
                    }
                    _ => None,
                };
                if let Some(r) = handled {
                    return r;
                }
            }
        }
        match recv {
            Value::Str(s) => match name {
                "upper" => Ok(Value::Str(s.to_uppercase())),
                "at" => {
                    // L1a: safe char access with an optional default.
                    let chars: Vec<char> = s.chars().collect();
                    match args.first() {
                        Some(Value::Int(i)) => {
                            let j = if *i < 0 { chars.len() as i64 + *i } else { *i };
                            if j >= 0 && (j as usize) < chars.len() {
                                Ok(Value::Str(chars[j as usize].to_string()))
                            } else {
                                match args.get(1) {
                                    Some(d) => Ok(d.clone()),
                                    None => {
                                        self.note(
                                            self.cur_line,
                                            4,
                                            "char index out of range; null",
                                        );
                                        Ok(Value::Null)
                                    }
                                }
                            }
                        }
                        _ => {
                            self.note(self.cur_line, 4, "at needs an int index; null");
                            Ok(Value::Null)
                        }
                    }
                }
                "lower" => Ok(Value::Str(s.to_lowercase())),
                "trim" => Ok(Value::Str(s.trim().to_string())),
                "split" => {
                    let sep = args
                        .first()
                        .map(|v| v.display())
                        .unwrap_or_else(|| " ".into());
                    Ok(Value::List(Rc::new(RefCell::new(
                        s.split(sep.as_str())
                            .map(|p| Value::Str(p.to_string()))
                            .collect(),
                    ))))
                }
                "join" => {
                    let sep = s.clone();
                    if let Some(Value::List(l)) = args.first() {
                        let parts: Vec<String> = l.borrow().iter().map(|v| v.display()).collect();
                        Ok(Value::Str(parts.join(&sep)))
                    } else {
                        Ok(Value::Str(s))
                    }
                }
                "replace" => {
                    let a = args.first().map(|v| v.display()).unwrap_or_default();
                    let b = args.get(1).map(|v| v.display()).unwrap_or_default();
                    Ok(Value::Str(s.replace(&a, &b)))
                }
                "contains" => {
                    let a = args.first().map(|v| v.display()).unwrap_or_default();
                    Ok(Value::Bool(s.contains(&a)))
                }
                "starts" => {
                    let a = args.first().map(|v| v.display()).unwrap_or_default();
                    Ok(Value::Bool(s.starts_with(&a)))
                }
                "ends" => {
                    let a = args.first().map(|v| v.display()).unwrap_or_default();
                    Ok(Value::Bool(s.ends_with(&a)))
                }
                "repeat" => {
                    let n = match args.first() {
                        Some(Value::Int(i)) => (*i).max(0) as usize,
                        _ => 0,
                    };
                    // allocation ceiling: giant repeats abort the process
                    // outside the stress model — cap as catchable overflow
                    if n.saturating_mul(s.len()) > 512 * 1024 * 1024 {
                        return Err(Stress::new(
                            "overflow",
                            "repeat exceeds the 512 MiB string ceiling",
                        ));
                    }
                    Ok(Value::Str(s.repeat(n)))
                }
                "slice" => {
                    let a = match args.first() {
                        Some(Value::Int(i)) => *i,
                        _ => 0,
                    };
                    let b = match args.get(1) {
                        Some(Value::Int(i)) => *i,
                        _ => s.len() as i64,
                    };
                    let len = s.chars().count() as i64;
                    // Python-style: negative indexes count from the end
                    let norm = |x: i64| -> i64 {
                        if x < 0 {
                            (len + x).max(0)
                        } else {
                            x.min(len)
                        }
                    };
                    let a = norm(a) as usize;
                    let b = norm(b) as usize;
                    Ok(Value::Str(
                        s.chars().skip(a).take(b.saturating_sub(a)).collect(),
                    ))
                }
                "len" => Ok(Value::Int(s.chars().count() as i64)),
                _ => {
                    self.note(0, 4, format!("unknown str method '{}'; null", name));
                    Ok(Value::Null)
                }
            },
            Value::List(l) => match name {
                "map" => {
                    let f = args.first().cloned().unwrap_or(Value::Null);
                    let mut out = Vec::new();
                    // clone-first: user callbacks may mutate the source list
                    // (a borrow() held across the callback would abort)
                    let src: Vec<Value> = l.borrow().clone();
                    for v in src.iter() {
                        out.push(self.call_value(env, &f, vec![v.clone()])?);
                    }
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                "filter" => {
                    let f = args.first().cloned().unwrap_or(Value::Null);
                    let mut out = Vec::new();
                    let src: Vec<Value> = l.borrow().clone();
                    for v in src.iter() {
                        if self.call_value(env, &f, vec![v.clone()])?.truthy() {
                            out.push(v.clone());
                        }
                    }
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                "reduce" => {
                    let f = args.first().cloned().unwrap_or(Value::Null);
                    let init = args.get(1).cloned().unwrap_or(Value::Int(0));
                    let mut acc = init;
                    let src: Vec<Value> = l.borrow().clone();
                    for v in src.iter() {
                        acc = self.call_value(env, &f, vec![acc, v.clone()])?;
                    }
                    Ok(acc)
                }
                "each" => {
                    let f = args.first().cloned().unwrap_or(Value::Null);
                    let src: Vec<Value> = l.borrow().clone();
                    for v in src.iter() {
                        self.call_value(env, &f, vec![v.clone()])?;
                    }
                    Ok(Value::Null)
                }
                "sort" => {
                    let cmp = args.first().cloned().unwrap_or(Value::Null);
                    let mut v = l.borrow().clone();
                    if let Value::Gene(_, _) = &cmp {
                        // insertion sort with user comparator: cmp(a, b) true
                        // when a belongs BEFORE b (SPEC §sorted-order)
                        for i in 1..v.len() {
                            let mut j = i;
                            while j > 0 {
                                let a = v[j - 1].clone();
                                let b = v[j].clone();
                                let before = self.call_value(env, &cmp, vec![a, b])?.truthy();
                                if !before {
                                    v.swap(j - 1, j);
                                    j -= 1;
                                } else {
                                    break;
                                }
                            }
                        }
                    } else {
                        // default: numbers and strings ascending — mixed-type
                        // ordering matches the oracle (numbers first, strings
                        // after), stability preserved
                        let key = |v: &Value| -> (u8, f64, String) {
                            match v {
                                Value::Bool(b) => (0, *b as i64 as f64, String::new()),
                                Value::Int(i) => (0, *i as f64, String::new()),
                                Value::Float(f) => (0, *f, String::new()),
                                Value::Str(s) => (1, 0.0, s.clone()),
                                other => (2, 0.0, other.repr()),
                            }
                        };
                        v.sort_by(|a, b| {
                            let (ka, kb) = (key(a), key(b));
                            ka.0.cmp(&kb.0)
                                .then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
                                .then_with(|| ka.2.cmp(&kb.2))
                        });
                    }
                    // immutable-method contract: returns a NEW sorted list
                    // (mirrors reverse/slice/map — the suite is the contract)
                    Ok(Value::List(Rc::new(RefCell::new(v))))
                }
                "reverse" => {
                    let mut v = l.borrow().clone();
                    v.reverse();
                    Ok(Value::List(Rc::new(RefCell::new(v))))
                }
                "contains" => {
                    let t = args.first().cloned().unwrap_or(Value::Null);
                    Ok(Value::Bool(l.borrow().iter().any(|v| v.deep_eq(&t))))
                }
                "index_of" => {
                    let t = args.first().cloned().unwrap_or(Value::Null);
                    Ok(Value::Int(
                        l.borrow()
                            .iter()
                            .position(|v| v.deep_eq(&t))
                            .map(|i| i as i64)
                            .unwrap_or(-1),
                    ))
                }
                "slice" => {
                    let a = match args.first() {
                        Some(Value::Int(i)) => *i,
                        _ => 0,
                    };
                    let b = match args.get(1) {
                        Some(Value::Int(i)) => *i,
                        _ => l.borrow().len() as i64,
                    };
                    let len = l.borrow().len() as i64;
                    // Python-style: negative indexes count from the end
                    let norm = |x: i64| -> i64 {
                        if x < 0 {
                            (len + x).max(0)
                        } else {
                            x.min(len)
                        }
                    };
                    let a = norm(a) as usize;
                    let b = norm(b) as usize;
                    let out: Vec<Value> = l.borrow()[a..b.max(a)].to_vec();
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                "join" => {
                    let sep = args.first().map(|v| v.display()).unwrap_or_default();
                    let parts: Vec<String> = l.borrow().iter().map(|v| v.display()).collect();
                    // sec-r1 (audit C-10): join allocates the whole result in
                    // one hop — apply the same per-op ceiling + aggregate
                    // charge as concat/repeat, or a 64M-element list mints a
                    // ~1 GB string invisible to every ceiling
                    let total: u64 = parts.iter().map(|p| p.len() as u64).sum();
                    let seps = if parts.is_empty() {
                        0u64
                    } else {
                        sep.len() as u64 * (parts.len() as u64 - 1)
                    };
                    if total.saturating_add(seps) > 512 * 1024 * 1024 {
                        return Err(Stress::new(
                            "overflow",
                            "join result exceeds the 512 MiB ceiling",
                        ));
                    }
                    mem_charge(total.saturating_add(seps))?;
                    Ok(Value::Str(parts.join(&sep)))
                }
                "len" => Ok(Value::Int(l.borrow().len() as i64)),
                "push" => {
                    if let Some(v) = args.first() {
                        // aggregate allocation ceiling (S4 NEW-2)
                        let bytes = match v {
                            Value::Str(x) => x.len() as u64 + 24,
                            Value::List(x) => 8 * x.borrow().len() as u64 + 48,
                            _ => 16,
                        };
                        mem_charge(bytes)?;
                        l.borrow_mut().push(v.clone());
                    }
                    Ok(Value::List(l.clone()))
                }
                "pop" => Ok(l.borrow_mut().pop().unwrap_or(Value::Null)),
                "get" => {
                    // L1a: safe index read with an optional default.
                    let n = l.borrow().len();
                    match args.first() {
                        Some(Value::Int(i)) => {
                            let j = if *i < 0 { n as i64 + *i } else { *i };
                            if j >= 0 && (j as usize) < n {
                                Ok(l.borrow()[j as usize].clone())
                            } else {
                                match args.get(1) {
                                    Some(d) => Ok(d.clone()),
                                    None => {
                                        self.note(self.cur_line, 4, "index out of range; null");
                                        Ok(Value::Null)
                                    }
                                }
                            }
                        }
                        _ => {
                            self.note(self.cur_line, 4, "list get needs an int index; null");
                            Ok(Value::Null)
                        }
                    }
                }
                _ => {
                    self.note(0, 4, format!("unknown list method '{}'; null", name));
                    Ok(Value::Null)
                }
            },
            Value::Map(m) => match name {
                "keys" => Ok(Value::List(Rc::new(RefCell::new(
                    m.borrow().iter().map(|(k, _)| k.clone()).collect(),
                )))),
                "values" => Ok(Value::List(Rc::new(RefCell::new(
                    m.borrow().iter().map(|(_, v)| v.clone()).collect(),
                )))),
                "items" => Ok(Value::List(Rc::new(RefCell::new(
                    m.borrow()
                        .iter()
                        .map(|(k, v)| {
                            Value::List(Rc::new(RefCell::new(vec![k.clone(), v.clone()])))
                        })
                        .collect(),
                )))),
                "has" => {
                    let t = args.first().cloned().unwrap_or(Value::Null);
                    Ok(Value::Bool(m.borrow().position(&t).is_some()))
                }
                "get" => {
                    // L1a: safe key access with an optional default. Bare
                    // missing key keeps the member-access note; with a
                    // default it is returned silently (mainstream idiom).
                    let t = args.first().cloned().unwrap_or(Value::Null);
                    match m.borrow().position(&t) {
                        Some(i) => Ok(m
                            .borrow()
                            .iter()
                            .nth(i)
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null)),
                        None => match args.get(1) {
                            Some(d) => Ok(d.clone()),
                            None => {
                                self.note(
                                    self.cur_line,
                                    4,
                                    format!("key '{}' missing; null", t.display()),
                                );
                                Ok(Value::Null)
                            }
                        },
                    }
                }
                "del" => {
                    let t = args.first().cloned().unwrap_or(Value::Null);
                    m.borrow_mut().del(&t);
                    Ok(Value::Null)
                }
                "len" => Ok(Value::Int(m.borrow().len() as i64)),
                _ => match m
                    .borrow()
                    .iter()
                    .find(|(k, _)| matches!(k, Value::Str(s) if s == name))
                {
                    Some((_, Value::Gene(_, _))) => {
                        let f = m
                            .borrow()
                            .iter()
                            .find(|(k, _)| matches!(k, Value::Str(s) if s == name))
                            .map(|(_, v)| v.clone())
                            .unwrap();
                        self.call_value(env, &f, args)
                    }
                    _ => {
                        self.note(0, 4, format!("unknown map method '{}'; null", name));
                        Ok(Value::Null)
                    }
                },
            },
            Value::Seq(_d, st) => match name {
                "next" => match self.seq_pull(&st)? {
                    Some(v) => Ok(v),
                    None => Ok(Value::Null),
                },
                "collect" => {
                    let mut out = Vec::new();
                    while let Some(v) = self.seq_pull(&st)? {
                        out.push(v);
                    }
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                _ => {
                    self.note(0, 4, format!("unknown sequence method '{}'; null", name));
                    Ok(Value::Null)
                }
            },
            Value::Obj(def, fields) => {
                // phenotype method dispatch: own methods first, then the
                // differentiation lineage (parent chain)
                let chain = self.pheno_chain(&def);
                for d in chain.iter() {
                    if let Some(g) = d.methods.iter().find(|g| g.name.as_deref() == Some(name)) {
                        let obj = Value::Obj(def.clone(), fields.clone());
                        return self.call_method_gene(g.clone(), obj, args);
                    }
                }
                // fall back to field access as a zero-arg call
                match fields
                    .borrow()
                    .iter()
                    .find(|(k, _)| matches!(k, Value::Str(s) if s == name))
                {
                    Some((_, v)) => self.call_value(env, v, args),
                    None => {
                        self.note(
                            0,
                            4,
                            format!("phenotype {} has no method '{}'; null", def.name, name),
                        );
                        Ok(Value::Null)
                    }
                }
            }
            other => {
                self.note(
                    0,
                    4,
                    format!("{} has no method '{}'; null", other.type_name(), name),
                );
                Ok(Value::Null)
            }
        }
    }
}

// ------------------------------------------------------------------ server
// A tiny built-in HTTP server (capability-gated): one listener thread feeds a
// shared request queue; the Operon program polls recv_request / answers with
// send_response. Global state because the toolchain binary hosts one process.
use std::collections::HashMap as StdHashMap;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;

struct ServerState {
    rx: mpsc::Receiver<(u64, String, String, String)>,
    conns: Arc<Mutex<StdHashMap<u64, TcpStream>>>,
}

static SERVER: Mutex<Option<ServerState>> = Mutex::new(None);

pub fn serve_start(port: u16) -> Result<(), String> {
    let mut guard = SERVER.lock().map_err(|_| "server lock poisoned")?;
    if guard.is_some() {
        return Ok(()); // already running
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel::<(u64, String, String, String)>();
    let conns: Arc<Mutex<StdHashMap<u64, TcpStream>>> = Arc::new(Mutex::new(StdHashMap::new()));
    let conns2 = conns.clone();
    let next2 = Arc::new(AtomicU64::new(1));
    const MAX_CONNECTIONS: usize = 256;
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            // a half-open client must not stall the accept loop forever
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
            // connection-table cap: floods are dropped, not queued forever
            if conns2
                .lock()
                .map(|c| c.len() >= MAX_CONNECTIONS)
                .unwrap_or(true)
            {
                continue;
            }
            let conn = next2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // per-connection reader: a trickling client holds only its own
            // thread (with its 10s timeout), never the accept loop (S4 fix 8)
            let conns3 = conns2.clone();
            let tx3 = tx2.clone();
            let read_res = crate::genes::spawn_worker(move || {
                let (head, body) = read_request_head(&mut stream);
                let mut method = "GET".to_string();
                let mut path = "/".to_string();
                if let Some(l0) = head.lines().next() {
                    let mut it = l0.split_whitespace();
                    method = it.next().unwrap_or("GET").to_string();
                    path = it.next().unwrap_or("/").to_string();
                }
                conns3.lock().ok().map(|mut c| c.insert(conn, stream));
                let _ = tx3.send((conn, method, path, body));
            });
            if read_res.is_err() {
                continue; // thread budget exhausted: drop the connection
            }
        }
    });
    *guard = Some(ServerState { rx, conns });
    Ok(())
}

/// Read one request head (+ declared body) from a stream.
/// Returns (head, method-path-body-triple as raw strings pre-parsed here).
fn read_request_head(stream: &mut std::net::TcpStream) -> (String, String) {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                buf.push(byte[0]);
                if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
                    break;
                }
                if buf.len() > 64 * 1024 {
                    break; // oversized head: caller sees no head terminator
                }
            }
            Err(_) => break,
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let clen = head
        .to_ascii_lowercase()
        .lines()
        .find_map(|l| {
            l.strip_prefix("content-length:")
                .map(|v| v.trim().to_string())
        })
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = String::new();
    if clen > 0 && clen <= 8 * 1024 * 1024 {
        let mut rest = vec![0u8; clen];
        if stream.read_exact(&mut rest).is_ok() {
            body = String::from_utf8_lossy(&rest).to_string();
        }
    }
    (head, body)
}

#[allow(dead_code)]
pub fn recv_request() -> Option<(u64, String, String, String)> {
    let guard = SERVER.lock().ok()?;
    let st = guard.as_ref()?;
    match st.rx.recv_timeout(std::time::Duration::from_millis(10)) {
        Ok(r) => Some(r),
        Err(mpsc::RecvTimeoutError::Timeout) => None,
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    }
}

pub fn send_response(conn: u64, status: &str, ctype: &str, body: &str) -> Result<(), String> {
    let guard = SERVER.lock().map_err(|_| "server lock poisoned")?;
    let st = guard.as_ref().ok_or("server not running")?;
    let stream = st
        .conns
        .lock()
        .map_err(|_| "conn lock poisoned")?
        .remove(&conn);
    let mut stream = stream.ok_or("unknown connection id")?;
    let reason = match status {
        "200" => "OK",
        "201" => "Created",
        "204" => "No Content",
        "301" => "Moved Permanently",
        "302" => "Found",
        "400" => "Bad Request",
        "403" => "Forbidden",
        "404" => "Not Found",
        "405" => "Method Not Allowed",
        "413" => "Payload Too Large",
        "431" => "Request Header Fields Too Large",
        "500" => "Internal Server Error",
        "503" => "Service Unavailable",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, reason, ctype, body.len()
    );
    use std::io::Write;
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(body.as_bytes()))
        .and_then(|_| stream.flush())
        .map_err(|e| e.to_string())
}

fn http_get(host: &str, port: u16, path: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    // sec-r1 (audit C-12): CR/LF in host/path injects headers into the
    // request line (request smuggling); reject before any socket exists
    if host.contains('\r') || host.contains('\n') || path.contains('\r') || path.contains('\n') {
        return Err("http_get: host and path must not contain CR or LF".to_string());
    }
    // bounded connect: a black-holed address previously stalled one call for
    // the OS TCP timeout (fuel-blind, minutes)
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| "http_get: host resolved to no address".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let req = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: operon\r\n\r\n",
        path, host
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    // bounded read: an endless peer cannot balloon memory without limit
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16384];
    const MAX_RESPONSE: usize = 64 * 1024 * 1024;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_RESPONSE {
                    return Err("http_get response exceeds the 64 MiB ceiling".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    // split headers from body
    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        Ok(String::from_utf8_lossy(&buf[pos + 4..]).to_string())
    } else {
        Ok(String::from_utf8_lossy(&buf).to_string())
    }
}

// ------------------------------------------------------------------ json
pub fn json_stringify(v: &Value) -> String {
    // cycle-safe: a container containing itself serializes the repeated
    // branch as null (JSON has no cycle marker; CPython's json.dumps errors,
    // we contain instead of crash)
    let mut seen: Vec<usize> = Vec::new();
    json_stringify_g(v, &mut seen, 0)
}

fn json_stringify_g(v: &Value, seen: &mut Vec<usize>, depth: u32) -> String {
    if depth > 512 {
        return "null".into();
    }
    match v {
        Value::Null => "null".into(),
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => crate::value::format_float(*f),
        Value::Str(s) => json_quote(s),
        Value::List(l) => {
            let id = std::rc::Rc::as_ptr(l) as *const u8 as usize;
            if seen.contains(&id) {
                return "null".into();
            }
            seen.push(id);
            let parts: Vec<String> = l
                .borrow()
                .iter()
                .map(|x| json_stringify_g(x, seen, depth + 1))
                .collect();
            seen.pop();
            format!("[{}]", parts.join(","))
        }
        Value::Map(m) => {
            let id = std::rc::Rc::as_ptr(m) as *const u8 as usize;
            if seen.contains(&id) {
                return "null".into();
            }
            seen.push(id);
            let parts: Vec<String> = m
                .borrow()
                .iter()
                .map(|(k, x)| {
                    format!(
                        "{}:{}",
                        json_quote(&k.display()),
                        json_stringify_g(x, seen, depth + 1)
                    )
                })
                .collect();
            seen.pop();
            format!("{{{}}}", parts.join(","))
        }
        other => json_quote(&other.display()),
    }
}

fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other if (other as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", other as u32)),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

// =================================================================== regex
// Zero-dependency regex engine (wave-3 Critic-L gap #2). Explicit-stack
// backtracking matcher with a HARD STEP CAP: catastrophic backtracking
// raises a catchable `overflow` stress instead of hanging (ReDoS-proof).
//
// Supported syntax (Python-compatible subset):
//   literals  .  [a-z0-9^]  \d \w \s \D \W \S  escapes
//   * + ? {m} {m,} {m,n}   |   () groups (and (?:...))   ^ $

#[derive(Debug, Clone)]
enum ReAst {
    Lit(char),
    Dot,
    Cls(Vec<(char, char)>, bool),
    Seq(Vec<ReAst>),
    Alt(Vec<ReAst>),
    Rep(Box<ReAst>, usize, usize),
    Group(usize, Box<ReAst>),
    Bol,
    Eol,
}

const RE_STEP_CAP: u64 = 2_000_000;

/// sec-r1 (audit C-2): the regex PARSER recurses per group nesting — the
/// matcher has a step cap, but a pattern like `"(" * 2_000_000` blew the
/// native stack before a single match step ran (uncatchable abort, rc=134).
/// Cap nesting at 1024 (10× anything a real pattern needs).
const RE_DEPTH_CAP: usize = 1024;

struct ReParser {
    chars: Vec<char>,
    pos: usize,
    ngroups: usize,
    depth: usize,
}

impl ReParser {
    fn parse(&mut self) -> Result<ReAst, String> {
        let a = self.alternation()?;
        if self.pos < self.chars.len() {
            return Err(format!(
                "unexpected '{}' at {}",
                self.chars[self.pos], self.pos
            ));
        }
        Ok(a)
    }
    fn alternation(&mut self) -> Result<ReAst, String> {
        let mut branches = vec![self.concat()?];
        while self.pos < self.chars.len() && self.chars[self.pos] == '|' {
            self.pos += 1;
            branches.push(self.concat()?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().unwrap())
        } else {
            Ok(ReAst::Alt(branches))
        }
    }
    fn concat(&mut self) -> Result<ReAst, String> {
        let mut items = Vec::new();
        while self.pos < self.chars.len()
            && self.chars[self.pos] != '|'
            && self.chars[self.pos] != ')'
        {
            items.push(self.repeat()?);
        }
        Ok(match items.len() {
            0 => ReAst::Seq(Vec::new()),
            1 => items.pop().unwrap(),
            _ => ReAst::Seq(items),
        })
    }
    fn repeat(&mut self) -> Result<ReAst, String> {
        let atom = self.atom()?;
        if self.pos >= self.chars.len() {
            return Ok(atom);
        }
        let (min, max) = match self.chars[self.pos] {
            '*' => {
                self.pos += 1;
                (0, usize::MAX)
            }
            '+' => {
                self.pos += 1;
                (1, usize::MAX)
            }
            '?' => {
                self.pos += 1;
                (0, 1)
            }
            '{' => {
                let save = self.pos;
                self.pos += 1;
                let mut m = String::new();
                while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                    m.push(self.chars[self.pos]);
                    self.pos += 1;
                }
                if m.is_empty() {
                    self.pos = save;
                    return Ok(atom);
                }
                let min: usize = m.parse().map_err(|_| "bad repetition".to_string())?;
                let mut max = min;
                if self.pos < self.chars.len() && self.chars[self.pos] == ',' {
                    self.pos += 1;
                    let mut n = String::new();
                    while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                        n.push(self.chars[self.pos]);
                        self.pos += 1;
                    }
                    max = if n.is_empty() {
                        usize::MAX
                    } else {
                        n.parse().map_err(|_| "bad repetition".to_string())?
                    };
                }
                if self.pos >= self.chars.len() || self.chars[self.pos] != '}' {
                    self.pos = save;
                    return Ok(atom);
                }
                self.pos += 1;
                if min > max || max > 100_000 {
                    return Err("bad repetition range".into());
                }
                (min, max)
            }
            _ => return Ok(atom),
        };
        Ok(ReAst::Rep(Box::new(atom), min, max))
    }
    fn atom(&mut self) -> Result<ReAst, String> {
        if self.pos >= self.chars.len() {
            return Err("unexpected end of pattern".into());
        }
        let c = self.chars[self.pos];
        match c {
            '(' => {
                // sec-r1 (audit C-2): parser recursion depth = group nesting
                self.depth += 1;
                if self.depth > RE_DEPTH_CAP {
                    return Err(format!("regex group nesting exceeds {}", RE_DEPTH_CAP));
                }
                self.pos += 1;
                let capture = if self.chars[self.pos..].starts_with(&['?', ':']) {
                    self.pos += 2;
                    false
                } else {
                    self.ngroups += 1;
                    true
                };
                let inner = self.alternation()?;
                self.depth -= 1;
                if self.pos >= self.chars.len() || self.chars[self.pos] != ')' {
                    return Err("unclosed group".into());
                }
                self.pos += 1;
                Ok(if capture {
                    ReAst::Group(self.ngroups, Box::new(inner))
                } else {
                    inner
                })
            }
            '[' => {
                self.pos += 1;
                let mut neg = false;
                if self.pos < self.chars.len() && self.chars[self.pos] == '^' {
                    neg = true;
                    self.pos += 1;
                }
                let mut items: Vec<(char, char)> = Vec::new();
                let mut first = true;
                while self.pos < self.chars.len() && (self.chars[self.pos] != ']' || first) {
                    first = false;
                    let mut lo = self.chars[self.pos];
                    if lo == '\\' && self.pos + 1 < self.chars.len() {
                        self.pos += 1;
                        let e = self.chars[self.pos];
                        match e {
                            'n' => {
                                lo = '\n';
                                self.pos += 1;
                            }
                            't' => {
                                lo = '\t';
                                self.pos += 1;
                            }
                            'r' => {
                                lo = '\r';
                                self.pos += 1;
                            }
                            'd' | 'w' | 's' | 'D' | 'W' | 'S' => {
                                let (ranges, _) = re_class_shorthand(e);
                                items.extend(ranges);
                                self.pos += 1;
                                continue;
                            }
                            other => {
                                lo = other;
                                self.pos += 1;
                            }
                        }
                    } else {
                        self.pos += 1;
                    }
                    if self.pos + 1 < self.chars.len()
                        && self.chars[self.pos] == '-'
                        && self.chars[self.pos + 1] != ']'
                    {
                        self.pos += 1;
                        let hi = self.chars[self.pos];
                        self.pos += 1;
                        items.push((lo, hi));
                    } else {
                        items.push((lo, lo));
                    }
                }
                if self.pos >= self.chars.len() {
                    return Err("unclosed class".into());
                }
                self.pos += 1;
                Ok(ReAst::Cls(items, neg))
            }
            '.' => {
                self.pos += 1;
                Ok(ReAst::Dot)
            }
            '^' => {
                self.pos += 1;
                Ok(ReAst::Bol)
            }
            '$' => {
                self.pos += 1;
                Ok(ReAst::Eol)
            }
            '\\' => {
                self.pos += 1;
                if self.pos >= self.chars.len() {
                    return Err("trailing backslash".into());
                }
                let e = self.chars[self.pos];
                self.pos += 1;
                if matches!(e, 'd' | 'w' | 's' | 'D' | 'W' | 'S') {
                    let (ranges, _) = re_class_shorthand(e);
                    let neg = matches!(e, 'D' | 'W' | 'S');
                    Ok(ReAst::Cls(ranges, neg))
                } else {
                    Ok(ReAst::Lit(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        other => other,
                    }))
                }
            }
            '*' | '+' | '?' => Err(format!("nothing to repeat at {}", self.pos)),
            other => {
                self.pos += 1;
                Ok(ReAst::Lit(other))
            }
        }
    }
}

fn re_class_shorthand(e: char) -> (Vec<(char, char)>, usize) {
    match e {
        'd' | 'D' => (vec![('0', '9')], 1),
        'w' | 'W' => (vec![('a', 'z'), ('A', 'Z'), ('0', '9'), ('_', '_')], 1),
        's' | 'S' => (
            vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')],
            1,
        ),
        _ => (Vec::new(), 0),
    }
}

/// Pending-work item: either an AST node or a group-close bookkeeping marker.
#[derive(Clone, Copy)]
enum ReItem<'a> {
    Node(&'a ReAst),
    CapOpen(usize, usize), // (group number, start position)
}

/// Capture list: one slot per group; `None` = group unmatched.
type ReCaps = Vec<Option<(usize, usize)>>;

struct ReMatcher<'a> {
    subject: &'a [char],
    steps: u64,
    last_end: usize,
    overflow: bool,
}

impl<'a> ReMatcher<'a> {
    fn m_all(&mut self, stack: &mut Vec<ReItem<'a>>, pos: usize, caps: &mut ReCaps) -> bool {
        match stack.pop() {
            None => {
                // full parse accepted: greedy ordering makes this the
                // longest/rightmost end for this start position
                self.last_end = pos;
                true
            }
            Some(ReItem::CapOpen(n, start)) => {
                caps[n - 1] = Some((start, pos));
                self.m_all(stack, pos, caps)
            }
            Some(ReItem::Node(node)) => self.m(node, stack, pos, caps),
        }
    }

    fn m(
        &mut self,
        node: &'a ReAst,
        stack: &mut Vec<ReItem<'a>>,
        pos: usize,
        caps: &mut ReCaps,
    ) -> bool {
        self.steps += 1;
        if self.steps > RE_STEP_CAP {
            self.overflow = true;
            return false;
        }
        match node {
            ReAst::Lit(c) => {
                if pos < self.subject.len() && self.subject[pos] == *c {
                    self.m_all(stack, pos + 1, caps)
                } else {
                    false
                }
            }
            ReAst::Dot => {
                if pos < self.subject.len() && self.subject[pos] != '\n' {
                    self.m_all(stack, pos + 1, caps)
                } else {
                    false
                }
            }
            ReAst::Cls(items, neg) => {
                if pos >= self.subject.len() {
                    return false;
                }
                let c = self.subject[pos];
                let inside = items.iter().any(|(lo, hi)| c >= *lo && c <= *hi);
                if inside != *neg {
                    self.m_all(stack, pos + 1, caps)
                } else {
                    false
                }
            }
            ReAst::Bol => {
                if pos == 0 {
                    self.m_all(stack, pos, caps)
                } else {
                    false
                }
            }
            ReAst::Eol => {
                if pos == self.subject.len() {
                    self.m_all(stack, pos, caps)
                } else {
                    false
                }
            }
            ReAst::Seq(v) => {
                let saved: Vec<ReItem> = stack.clone();
                for x in v.iter().rev() {
                    stack.push(ReItem::Node(x));
                }
                if self.m_all(stack, pos, caps) {
                    true
                } else {
                    *stack = saved.clone();
                    false
                }
            }
            ReAst::Alt(v) => {
                let saved: Vec<ReItem> = stack.clone();
                for b in v {
                    let snap = caps.clone();
                    stack.push(ReItem::Node(b));
                    if self.m_all(stack, pos, caps) {
                        return true;
                    }
                    *stack = saved.clone();
                    *caps = snap;
                }
                false
            }
            ReAst::Group(n, inner) => {
                let saved: Vec<ReItem> = stack.clone();
                stack.push(ReItem::CapOpen(*n, pos));
                stack.push(ReItem::Node(inner));
                if self.m_all(stack, pos, caps) {
                    true
                } else {
                    *stack = saved.clone();
                    false
                }
            }
            ReAst::Rep(inner, min, max) => {
                // Greedy repetition. Each iteration runs as an isolated
                // sub-match (groups inside repeats take the last-iteration
                // capture, Python-style); ends are recorded and the
                // continuation is tried from the longest end backwards.
                let mut ends: Vec<(usize, ReCaps)> = vec![(pos, caps.clone())];
                let mut cur = pos;
                loop {
                    if ends.len() > *max {
                        break;
                    }
                    let mut iter_caps = caps.clone();
                    match self.sub_match(inner, cur, &mut iter_caps) {
                        Some(e) => {
                            let zero = e == cur;
                            ends.push((e, iter_caps));
                            cur = e;
                            if zero {
                                break; // zero-width iteration: stop expanding
                            }
                        }
                        None => break,
                    }
                }
                // need at least `min` completed iterations (ends[k] = k iters)
                while ends.len() > *min && !ends.is_empty() {
                    let (e, snap) = ends.last().unwrap().clone();
                    let save = caps.clone();
                    *caps = snap;
                    let saved: Vec<ReItem> = stack.clone();
                    if self.m_all(stack, e, caps) {
                        return true;
                    }
                    *stack = saved.clone();
                    *caps = save;
                    ends.pop();
                }
                false
            }
        }
    }

    /// Isolated sub-match of one repetition iteration: returns the end
    /// position of the (greedy-first) match of `node` starting at `pos`.
    fn sub_match(&mut self, node: &'a ReAst, pos: usize, caps: &mut ReCaps) -> Option<usize> {
        let saved_end = self.last_end;
        let mut stack: Vec<ReItem> = vec![ReItem::Node(node)];
        if self.m_all(&mut stack, pos, caps) {
            Some(self.last_end)
        } else {
            self.last_end = saved_end;
            None
        }
    }
}

pub struct ReEngine {
    ast: ReAst,
    pub ngroups: usize,
}

impl ReEngine {
    pub fn new(pattern: &str) -> Result<ReEngine, String> {
        let mut rp = ReParser {
            chars: pattern.chars().collect(),
            pos: 0,
            ngroups: 0,
            depth: 0,
        };
        let ast = rp.parse()?;
        Ok(ReEngine {
            ast,
            ngroups: rp.ngroups,
        })
    }

    /// Full-match test at `start`; returns (matched, overflow, end, caps).
    fn run_at(&self, s: &[char], start: usize) -> (bool, bool, usize, ReCaps) {
        let mut mch = ReMatcher {
            subject: s,
            steps: 0,
            last_end: start,
            overflow: false,
        };
        let mut caps: ReCaps = vec![None; self.ngroups];
        let mut stack: Vec<ReItem> = vec![ReItem::Node(&self.ast)];
        let ok = mch.m_all(&mut stack, start, &mut caps);
        (ok, mch.overflow, mch.last_end, caps)
    }

    /// Leftmost match from `from`: (start, end, caps, overflow).
    pub fn search(&self, s: &[char], from: usize) -> Option<(usize, usize, ReCaps, bool)> {
        for start in from..=s.len() {
            let (ok, ovf, end, caps) = self.run_at(s, start);
            if ovf {
                return Some((start, end, caps, true));
            }
            if ok {
                return Some((start, end, caps, false));
            }
        }
        None
    }
}

pub fn re_compile(pattern: &str) -> Result<ReEngine, Stress> {
    ReEngine::new(pattern).map_err(|e| Stress::new("unfolded", format!("regex: {}", e)))
}

/// Howard-style civil-from-days (UTC): days since 1970-01-01 → (y, m, d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

pub fn json_parse(src: &str) -> Result<Value, String> {
    struct P {
        b: Vec<char>,
        i: usize,
        depth: usize,
    }
    const MAX_DEPTH: usize = 512;
    impl P {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn peek(&mut self) -> Option<char> {
            self.ws();
            self.b.get(self.i).copied()
        }
        fn value(&mut self) -> Result<Value, String> {
            self.depth += 1;
            if self.depth > MAX_DEPTH {
                return Err(format!("json nested deeper than {} levels", MAX_DEPTH));
            }
            let r = self.value_inner();
            self.depth -= 1;
            r
        }
        fn value_inner(&mut self) -> Result<Value, String> {
            match self.peek().ok_or("unexpected end of json")? {
                '{' => {
                    self.i += 1;
                    let mut out: Vec<(Value, Value)> = Vec::new();
                    if self.peek() == Some('}') {
                        self.i += 1;
                        return Ok(Value::Map(Rc::new(RefCell::new(
                            crate::value::MapStore::from_vec(out),
                        ))));
                    }
                    loop {
                        let k = self.value()?;
                        if self.peek() != Some(':') {
                            return Err("expected ':'".into());
                        }
                        self.i += 1;
                        let v = self.value()?;
                        out.push((Value::Str(k.display()), v));
                        match self.peek() {
                            Some(',') => {
                                self.i += 1;
                            }
                            Some('}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err("expected ',' or '}'".into()),
                        }
                    }
                    Ok(Value::Map(Rc::new(RefCell::new(
                        crate::value::MapStore::from_vec(out),
                    ))))
                }
                '[' => {
                    self.i += 1;
                    let mut out: Vec<Value> = Vec::new();
                    if self.peek() == Some(']') {
                        self.i += 1;
                        return Ok(Value::List(Rc::new(RefCell::new(out))));
                    }
                    loop {
                        out.push(self.value()?);
                        match self.peek() {
                            Some(',') => {
                                self.i += 1;
                            }
                            Some(']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err("expected ',' or ']'".into()),
                        }
                    }
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                '"' => {
                    self.i += 1;
                    let mut s = String::new();
                    loop {
                        match self.b.get(self.i) {
                            None => return Err("unterminated string".into()),
                            Some('"') => {
                                self.i += 1;
                                break;
                            }
                            Some('\\') => {
                                self.i += 1;
                                match self.b.get(self.i) {
                                    Some('n') => s.push('\n'),
                                    Some('t') => s.push('\t'),
                                    Some('r') => s.push('\r'),
                                    Some('"') => s.push('"'),
                                    Some('\\') => s.push('\\'),
                                    Some('/') => s.push('/'),
                                    Some('u') => {
                                        // \uXXXX with surrogate-pair decoding
                                        let hex = |p: &P, off: usize| -> Result<u32, String> {
                                            let mut v = 0u32;
                                            for k in 0..4 {
                                                let c =
                                                    *p.b.get(p.i + 1 + off + k)
                                                        .ok_or("bad unicode escape")?;
                                                let d =
                                                    c.to_digit(16).ok_or("bad unicode escape")?;
                                                v = v * 16 + d;
                                            }
                                            Ok(v)
                                        };
                                        let cp = hex(self, 0)?;
                                        self.i += 4;
                                        if (0xD800..0xDC00).contains(&cp) {
                                            // high surrogate: expect a low surrogate next
                                            if self.b.get(self.i + 1) == Some(&'\\')
                                                && self.b.get(self.i + 2) == Some(&'u')
                                            {
                                                let lo = hex(self, 2)?;
                                                if (0xDC00..0xE000).contains(&lo) {
                                                    self.i += 6;
                                                    let combined = 0x10000
                                                        + ((cp - 0xD800) << 10)
                                                        + (lo - 0xDC00);
                                                    s.push(
                                                        char::from_u32(combined)
                                                            .unwrap_or('\u{FFFD}'),
                                                    );
                                                } else {
                                                    s.push('\u{FFFD}');
                                                }
                                            } else {
                                                s.push('\u{FFFD}');
                                            }
                                        } else if (0xDC00..0xE000).contains(&cp) {
                                            s.push('\u{FFFD}'); // orphan low surrogate
                                        } else {
                                            s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                                        }
                                    }
                                    _ => return Err("bad escape".into()),
                                }
                                self.i += 1;
                            }
                            Some(&c) => {
                                s.push(c); // char-wise: real UTF-8, not bytes-as-Latin-1
                                self.i += 1;
                            }
                        }
                    }
                    Ok(Value::Str(s))
                }
                't' => {
                    self.expect_word("true")?;
                    Ok(Value::Bool(true))
                }
                'f' => {
                    self.expect_word("false")?;
                    Ok(Value::Bool(false))
                }
                'n' => {
                    self.expect_word("null")?;
                    Ok(Value::Null)
                }
                _ => {
                    let start = self.i;
                    while self.i < self.b.len()
                        && matches!(self.b[self.i], '-' | '+' | '.' | 'e' | 'E' | '0'..='9')
                    {
                        self.i += 1;
                    }
                    let txt: String = self.b[start..self.i].iter().collect();
                    if txt.is_empty() {
                        return Err(format!("unexpected character at {}", start));
                    }
                    if txt.starts_with('+') {
                        // JSON forbids a leading plus on numbers
                        return Err(format!("json numbers cannot start with '+' at {}", start));
                    }
                    if let Ok(i) = txt.parse::<i64>() {
                        Ok(Value::Int(i))
                    } else {
                        txt.parse::<f64>()
                            .map(Value::Float)
                            .map_err(|e| e.to_string())
                    }
                }
            }
        }
        fn expect_word(&mut self, w: &str) -> Result<(), String> {
            let chars: Vec<char> = w.chars().collect();
            if self.b.len() >= self.i + chars.len()
                && self.b[self.i..self.i + chars.len()] == chars[..]
            {
                self.i += chars.len();
                Ok(())
            } else {
                Err(format!("expected '{}'", w))
            }
        }
    }
    let mut p = P {
        b: src.chars().collect(),
        i: 0,
        depth: 0,
    };
    let v = p.value()?;
    p.ws();
    if p.i < p.b.len() {
        return Err(format!("trailing garbage at position {}", p.i));
    }
    Ok(v)
}

pub fn codon_table_char(codon: &str) -> char {
    match codon {
        "TTT" | "TTC" => 'F',
        "TTA" | "TTG" | "CTT" | "CTC" | "CTA" | "CTG" => 'L',
        "ATT" | "ATC" | "ATA" => 'I',
        "ATG" => 'M',
        "GTT" | "GTC" | "GTA" | "GTG" => 'V',
        "TCT" | "TCC" | "TCA" | "TCG" | "AGT" | "AGC" => 'S',
        "CCT" | "CCC" | "CCA" | "CCG" => 'P',
        "ACT" | "ACC" | "ACA" | "ACG" => 'T',
        "GCT" | "GCC" | "GCA" | "GCG" => 'A',
        "TAT" | "TAC" => 'Y',
        "TAA" | "TAG" | "TGA" => '*',
        "CAT" | "CAC" => 'H',
        "CAA" | "CAG" => 'Q',
        "AAT" | "AAC" => 'N',
        "AAA" | "AAG" => 'K',
        "GAT" | "GAC" => 'D',
        "GAA" | "GAG" => 'E',
        "TGT" | "TGC" => 'C',
        "TGG" => 'W',
        "CGT" | "CGC" | "CGA" | "CGG" | "AGA" | "AGG" => 'R',
        "GGT" | "GGC" | "GGA" | "GGG" => 'G',
        _ => 'X',
    }
}

pub const BUILTIN_SYNONYMS: &[(&str, &str)] = &[
    ("print", "promote"),
    ("echo", "promote"),
    ("say", "promote"),
    ("show", "promote"),
];

pub const BUILTIN_NAMES: &[&str] = &[
    "promote",
    "len",
    "push",
    "pop",
    "insert",
    "remove",
    "keys",
    "values",
    "has",
    "del",
    "range",
    "str",
    "num",
    "type",
    "abs",
    "min",
    "max",
    "sum",
    "clock",
    "exit",
    "assert",
    "codon",
    "distance",
    "similar",
    "transcribe",
    "reverse_complement",
    "gc_content",
    "translate",
    "find_orf",
    "memory",
    "methyl",
    "methylate",
    "demethylate",
    "grn_set",
    "grn_get",
    "fingerprint",
    "toggle_on",
    "toggle_state",
    "repressi_next",
    "repressi_state",
    "repressi_start",
    "grn_fire",
    "grn_state",
    "spawn",
    "join",
    "floor",
    "ceil",
    "sqrt",
    "pow",
    "random",
    "randomize",
    "chr",
    "ord",
    "now",
    "sleep",
    "argv",
    "read_file",
    "write_file",
    "append_file",
    "exists",
    "file_size",
    "read_dir",
    "items",
    "run",
    "http_get",
    "serve",
    "recv_request",
    "send_response",
    "json_parse",
    "json_str",
    "env",
    "re_match",
    "re_find",
    "re_groups",
    "unix_time",
    "date_parts",
    "date_fmt",
    "call",
    // L1a: iteration + numeric builtins
    "enumerate",
    "zip",
    "sorted",
    "reversed",
    "any",
    "all",
    "first",
    "last",
    "take",
    "drop",
    "unique",
    "flatten",
    "chunk",
    "round",
    "clamp",
    "divmod",
];

// ---------------------------------------------------------------- memory
// Aggregate allocation counter (monotonic per run). Per-op ceilings cap
// single operations; this caps the SUM so `push(loop)` cannot walk RSS into
// the allocator's abort. Charged on growth events: push / string concat /
// string repeat. Ceiling 2 GiB (a run that allocates-and-keeps 2 GiB is a
// runaway by contract).
static ALLOC_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub const ALLOC_CEILING: u64 = 2 * 1024 * 1024 * 1024;

pub fn mem_charge(bytes: u64) -> Result<(), Stress> {
    let prev = ALLOC_BYTES.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
    if prev.saturating_add(bytes) > ALLOC_CEILING {
        return Err(Stress::new(
            "overflow",
            "aggregate allocation ceiling (2 GiB) exhausted for this run",
        ));
    }
    Ok(())
}

// symbol-table accessors for the memory() builtin (sec-r2: Rust-owned
// table — the C kernel these used to reach into is deleted, audit A15)
pub fn unsafe_arena() -> usize {
    crate::ffi::table_bytes()
}
pub fn unsafe_interns() -> u32 {
    crate::ffi::intern_count()
}
pub fn unsafe_allocs() -> u64 {
    crate::ffi::table_allocs()
}

// ---------------------------------------------------------------- repressilator

/// A11 (reg-r2): the ring's state as a pure function of the tick count.
///
/// Discrete Elowitz–Leibler repressilator (Elowitz & Leibler, Nature 2000):
/// every substep, each node integrates
///     dA/dt = α / (1 + R^h) − γA        R = level of A's repressor
/// with α=10, γ=1, h=4, dt=0.05, 20 substeps per ring tick, init [5,0,...].
/// Node j's repressor is node (j+n−1) mod n (ring `a -> b -> c` means a
/// represses b). Oscillation emerges from the loop itself — nothing is
/// scheduled.
///
/// Determinism contract (differential oracle parity):
///   * only +, −, ×, ÷ and an unrolled 4th power — no powi/pow/libm calls;
///   * identical op order in the Python mirror (bootstrap/oracle.py);
///   * stateless fold from init — manual and wall-clock modes cannot drift.
pub fn repressilator_levels(n: usize, tick: u64) -> Vec<f64> {
    const ALPHA: f64 = 10.0;
    const GAMMA: f64 = 1.0;
    const DT: f64 = 0.05;
    const SUBSTEPS: usize = 20;
    if n == 0 {
        return Vec::new();
    }
    let mut lv = vec![0.0f64; n];
    lv[0] = 5.0;
    for _ in 0..tick {
        for _ in 0..SUBSTEPS {
            let snap = lv.clone();
            for (j, item) in lv.iter_mut().enumerate() {
                let rep = snap[(j + n - 1) % n];
                // unrolled rep^4 — MUST stay op-identical to the oracle
                let rep4 = rep * rep * rep * rep;
                let d = ALPHA / (1.0 + rep4) - GAMMA * snap[j];
                let v = snap[j] + DT * d;
                *item = if v > 0.0 { v } else { 0.0 };
            }
        }
    }
    lv
}

/// One ring tick of the ODE (20 Euler substeps) — shared by the pure
/// from-init fold and the interpreter's incremental cache (sec-r3), so both
/// paths execute the identical operation sequence (bit-identical parity).
pub fn repressilator_step(mut lv: Vec<f64>) -> Vec<f64> {
    const ALPHA: f64 = 10.0;
    const GAMMA: f64 = 1.0;
    const DT: f64 = 0.05;
    const SUBSTEPS: usize = 20;
    let n = lv.len();
    if n == 0 {
        return lv;
    }
    for _ in 0..SUBSTEPS {
        let snap = lv.clone();
        for (j, item) in lv.iter_mut().enumerate() {
            let rep = snap[(j + n - 1) % n];
            // unrolled rep^4 — MUST stay op-identical to the oracle
            let rep4 = rep * rep * rep * rep;
            let d = ALPHA / (1.0 + rep4) - GAMMA * snap[j];
            let v = snap[j] + DT * d;
            *item = if v > 0.0 { v } else { 0.0 };
        }
    }
    lv
}

/// Normalized (0..1) ring level a GRN gate reads for a ring node: the raw
/// ODE level divided by the production scale α, clamped. (A11: gates can
/// read ring levels.) Pure form — used by tests; the interpreter routes
/// through the fuel-charged cache (`Interp::ring_gate_level`).
pub fn repressilator_gate_level(n: usize, tick: u64, node_index: usize) -> f64 {
    const ALPHA: f64 = 10.0;
    let raw = repressilator_levels(n, tick)[node_index.min(n.saturating_sub(1))];
    let norm = raw / ALPHA;
    if norm > 1.0 {
        1.0
    } else {
        norm
    }
}
