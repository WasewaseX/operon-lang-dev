//! interp.rs — the tree-walking evaluator. Total Grammar at runtime:
//! soft failures become notes; only catchable Stress propagates.

use crate::ast::*;
use crate::value::{key_scalar, SeqState, Stress, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::{mpsc, Arc};

pub struct Env {
    pub vars: RefCell<HashMap<String, Value>>,
    pub parent: Option<Rc<Env>>,
}

impl Env {
    pub fn new(parent: Option<Rc<Env>>) -> Rc<Env> {
        Rc::new(Env { vars: RefCell::new(HashMap::new()), parent })
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
}

impl Default for Caps {
    fn default() -> Caps {
        // secure by default: enabled (denying) with zero grants
        Caps { enabled: true, read: Vec::new(), write: Vec::new(), run: Vec::new(), net: Vec::new(), env: Vec::new() }
    }
}

impl Caps {
    pub fn allow_all() -> Caps {
        Caps { enabled: false, ..Default::default() }
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
    fn path_allowed(list: &[String], path: &str) -> bool {
        let resolved_requested = std::fs::canonicalize(path)
            .ok()
            .map(|p: std::path::PathBuf| p.to_string_lossy().replace('\\', "/"));
        for g in list {
            // grant must exist and resolve inside its directory
            if let Ok(rg) = std::fs::canonicalize(g) {
                let rg_str = rg.to_string_lossy().replace('\\', "/");
                if let Some(rp) = &resolved_requested {
                    let rp_str = rp.as_str().replace('\\', "/");
                    if rp_str == rg_str || rp_str.starts_with(&format!("{}/", rg_str)) {
                        return true;
                    }
                }
                // non-existent target (create/write case): its parent chain
                // must resolve inside the grant
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
            // lexical fallback for grants that do not exist on disk
            let np = Self::norm_path(path);
            let ng = Self::norm_path(g);
            if !ng.is_empty() && (np == ng || np.starts_with(&format!("{}/", ng))) {
                return true;
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

pub struct Interp {
    pub notes: Vec<Note>,
    pub cell: HashMap<String, String>,
    pub cell_entry: Option<String>,
    pub base_dir: Option<String>,
    pub silences: Vec<(String, String)>,
    pub fates: HashMap<String, Arc<FateDef>>,
    pub phenos: HashMap<String, Arc<PhenoDef>>,
    pub grn_edges: Vec<RegEdge>,
    pub grn_levels: HashMap<String, f64>,
    pub toggles: Vec<(String, String, bool)>, // (a, b, a_on) — mutual repression pair
    pub repressi_ring: Vec<String>,
    pub repressi_i: usize,
    pub repressi_atomic: Option<Arc<AtomicU64>>,
    pub ires: Vec<String>,
    pub enhanced: Vec<String>,
    pub defined_genes: Vec<String>,
    pub call_counts: HashMap<String, u64>,
    pub call_time: HashMap<String, f64>, // µs inclusive (profiling)
    pub call_time_self: HashMap<String, f64>, // µs exclusive (children subtracted)
    pub call_stack: Vec<(String, f64, f64)>, // (name, start_ns, child_acc µs)
    pub call_clock: u64,
    pub gene_buckets: HashMap<String, HashMap<u64, u64>>, // burst-index bins (20 calls/bin)
    pub modules: HashMap<String, Value>, // path -> module map
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
    pub asserts_run: u64,
    pub rng: u64,
    pub cli_args: Vec<String>,
    pub tasks: HashMap<i64, TaskHandle>,
    pub next_task_id: i64,
    pub seq_tx: Option<mpsc::SyncSender<crate::value::SeqMsg>>,
}

impl Interp {
    pub fn new() -> Interp {
        Interp {
            notes: Vec::new(),
            cell: HashMap::new(),
            cell_entry: None,
            base_dir: None,
            silences: Vec::new(),
            fates: HashMap::new(),
            phenos: HashMap::new(),
            grn_edges: Vec::new(),
            grn_levels: HashMap::new(),
            toggles: Vec::new(),
            repressi_ring: Vec::new(),
            repressi_i: 0,
            repressi_atomic: None,
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
            asserts_run: 0,
            rng: 0x9E3779B97F4A7C15,
            cli_args: Vec::new(),
            tasks: HashMap::new(),
            next_task_id: 1,
            seq_tx: None,
        }
    }

    pub fn note(&mut self, line: usize, rung: u8, msg: impl Into<String>) {
        self.notes.push(Note { line, rung, message: msg.into() });
    }

    fn tick(&mut self) -> Result<(), Stress> {
        self.steps += 1;
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
                        (Value::Map(m), _) => m.borrow().iter().find(|(k, _)| k.deep_eq(&iv)).map(|(_, v)| v.clone()).unwrap_or(Value::Null),
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
                        self.note(0, 4, format!("cannot iterate {}; loop skipped", other.type_name()));
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
                                    self.note(0, 4, format!("pattern evaluation contained: [{}] {}", s.kind, s.message));
                                    Value::Null
                                }
                            };
                            sv.deep_eq(&lv)
                        }
                        MatchPat::Multi(ls) => ls.iter().any(|l| {
                            let lv = match self.eval(env, l) {
                                Ok(v) => v,
                                Err(s) => {
                                    self.note(0, 4, format!("pattern evaluation contained: [{}] {}", s.kind, s.message));
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
                Err(Stress { kind: k, message })
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
                            Some(k) => {
                                k == &stress.kind
                                    || k == "any"
                            }
                        };
                        if !kind_ok {
                            return Err(stress);
                        }
                        match rescue {
                            Some((binding, rbody)) => {
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
                                self.note(0, 4, format!("stress contained: [{}] {}", stress.kind, stress.message));
                                Ok(Flow::Norm)
                            }
                        }
                    }
                }
            }
            Stmt::Gene(def) => {
                let name = def.name.clone().unwrap_or_else(|| "<lambda>".into());
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
                    self.note(0, 1, format!("splice '{}' → variant '{}' active", sp.root, vname));
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
                if let Some(sec) = period {
                    if *sec > 0.0 {
                        let counter = Arc::new(AtomicU64::new(0));
                        let c2 = counter.clone();
                        let ms = (*sec * 1000.0) as u64;
                        std::thread::spawn(move || loop {
                            std::thread::sleep(std::time::Duration::from_millis(ms));
                            c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        });
                        self.repressi_atomic = Some(counter);
                        self.note(0, 1, format!("repressilator oscillating every {} ms", ms));
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

    pub fn stress_map(&self, s: &Stress) -> Value {
        let m = Rc::new(RefCell::new(vec![
            (Value::Str("kind".into()), Value::Str(s.kind.clone())),
            (Value::Str("message".into()), Value::Str(s.message.clone())),
        ]));
        Value::Map(m)
    }

    pub fn as_index(&self, v: &Value, len: usize) -> Result<usize, Stress> {
        match v {
            Value::Int(i) => {
                if *i < 0 {
                    // negative indexing counts from the end; past-the-front is
                    // out of range (never clamped)
                    let j = len as i64 + *i;
                    if j < 0 {
                        Err(Stress::new("missing", format!("index {} out of range", *i)))
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
        let mut b = m.borrow_mut();
        for (k, v) in b.iter_mut() {
            if k.deep_eq(&key) {
                *v = val;
                return;
            }
        }
        b.push((key, val));
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
                                self.note(0, 4, format!("interpolation stress contained: [{}] {}", s.kind, s.message));
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
                let m: crate::value::MapRef = Rc::new(RefCell::new(Vec::new()));
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
                        Value::Int(i) => Ok(Value::Int(-i)),
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
            Expr::Binary(op, l, r) => {
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
                    _ => {}
                }
                let lv = self.eval(env, l)?;
                let rv = self.eval(env, r)?;
                self.apply_binop(env, *op, &lv, &rv)
            }
            Expr::Call(callee, args) => {
                // check silences at call sites (RISC)
                if let Expr::Ident(name) = &**callee {
                    if let Some((from, to)) = self.silences.iter().find(|(f, _)| f == name).cloned() {
                        // acetylated genes are immune
                        let immune = match env.get(name) {
                            Some(Value::Gene(d, _)) => d.acetylate,
                            _ => false,
                        };
                        if !immune {
                            self.note(0, 4, format!("RISC: call to '{}' silenced → '{}'", from, to));
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
            Expr::Index(t, i) => {
                let tv = self.eval(env, t)?;
                let iv = self.eval(env, i)?;
                match (&tv, &iv) {
                    (Value::List(l), _) => {
                        let idx = self.as_index(&iv, l.borrow().len())?;
                        match l.borrow().get(idx) {
                            Some(v) => Ok(v.clone()),
                            None => Err(Stress::new("missing", format!("index {} out of range", idx))),
                        }
                    }
                    (Value::Map(m), _) => match m.borrow().iter().find(|(k, _)| k.deep_eq(&iv)) {
                        Some((_, v)) => Ok(v.clone()),
                        None => Err(Stress::new("missing", "key not found")),
                    },
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
                match &tv {
                    Value::Map(m) => match m.borrow().iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)) {
                        Some((_, v)) => Ok(v.clone()),
                        None => {
                            self.note(0, 4, format!("member '{}' missing on map; null", key));
                            Ok(Value::Null)
                        }
                    },
                    Value::Obj(d, m) => match m.borrow().iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)) {
                        Some((_, v)) => Ok(v.clone()),
                        None => {
                            self.note(0, 4, format!("field '{}' missing on phenotype {}; null", key, d.name));
                            Ok(Value::Null)
                        }
                    },
                    _ => {
                        self.note(0, 4, format!("member '{}' on {} is null", key, tv.type_name()));
                        Ok(Value::Null)
                    }
                }
            }
            Expr::Method(t, name, args) => {
                let tv = self.eval(env, t)?;
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
                        self.note(0, 4, format!("phenotype '{}' not declared; instance is an empty map", name));
                        return Ok(Value::Map(Rc::new(RefCell::new(Vec::new()))));
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
                        self.note(0, 4, format!("fate '{}' not declared; instance inert", name));
                        return Ok(Value::Map(Rc::new(RefCell::new(Vec::new()))));
                    }
                };
                let enter = def.enter.clone().unwrap_or_else(|| {
                    def.states.first().map(|(s, _)| s.clone()).unwrap_or_default()
                });
                let m: crate::value::MapRef = Rc::new(RefCell::new(vec![
                    (Value::Str("#fate".into()), Value::Str(def.name.clone())),
                    (Value::Str("#state".into()), Value::Str(enter)),
                ]));
                Ok(Value::Map(m))
            }
            Expr::Collect { var, iter, filter, body } => {
                let itv = self.eval(env, iter)?;
                let mut source: Vec<Value> = Vec::new();
                let mut seq_state: Option<Rc<RefCell<SeqState>>> = None;
                match itv {
                    Value::List(l) => source = l.borrow().clone(),
                    Value::Str(s) => source = s.chars().map(|c| Value::Str(c.to_string())).collect(),
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

    pub fn apply_binop(&mut self, _env: &Rc<Env>, op: BinOp, l: &Value, r: &Value) -> Result<Value, Stress> {
        use BinOp::*;
        match op {
            Add => match (l, r) {
                (Value::Int(a), Value::Int(b)) => a.checked_add(*b).map(Value::Int).ok_or_else(|| Stress::new("overflow", "int overflow in '+'")),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
                (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
                (Value::List(a), Value::List(b)) => {
                    let mut v = a.borrow().clone();
                    v.extend(b.borrow().iter().cloned());
                    Ok(Value::List(Rc::new(RefCell::new(v))))
                }
                _ => Err(Stress::new(
                    "unfolded",
                    format!("cannot add {} and {}", l.type_name(), r.type_name()),
                )),
            },
            Sub => self.arith(l, r, "+-", |a, b| a.checked_sub(*b).map(Value::Int), |a, b| a - b),
            Mul => self.arith(l, r, "*", |a, b| a.checked_mul(*b).map(Value::Int), |a, b| a * b),
            Pow => {
                // 2**10 → int (checked); anything else promotes to float
                match (l, r) {
                    (Value::Int(a), Value::Int(b)) if *b >= 0 && *b <= u32::MAX as i64 => {
                        a.checked_pow(*b as u32).map(Value::Int).ok_or_else(|| Stress::new("overflow", "int overflow in '**'"))
                    }
                    _ => {
                        let (a, b) = self.as_floats(l, r)?;
                        Ok(Value::Float(a.powf(b)))
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
                if b < 0 || b > 63 {
                    return Err(Stress::new("overflow", format!("shift amount {} out of range", b)));
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
                let (a, b) = self.as_floats(l, r)?;
                if b == 0.0 {
                    return Err(Stress::new("unfolded", "division by zero in '//'"));
                }
                Ok(Value::Int(a.div_euclid(b) as i64))
            }
            Mod => {
                // int % int stays int (Python semantics); sign follows divisor
                if let (Value::Int(a), Value::Int(b)) = (l, r) {
                    if *b == 0 {
                        return Err(Stress::new("unfolded", "modulo by zero"));
                    }
                    let m = a.rem_euclid(b.abs());
                    return Ok(Value::Int(if *b < 0 { -m } else { m }));
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
                        format!("'in' not defined for {} in {}", r.type_name(), l.type_name()),
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
            (Value::Int(a), Value::Int(b)) => fi(a, b).ok_or_else(|| Stress::new("overflow", format!("int overflow in '{}'", opname))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(ff(*a, *b))),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(ff(*a as f64, *b))),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(ff(*a, *b as f64))),
            _ => Err(Stress::new(
                "unfolded",
                format!("cannot apply '{}' to {} and {}", opname, l.type_name(), r.type_name()),
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
            _ => Err(Stress::new(
                "unfolded",
                format!("numeric op needs numbers, found {} and {}", l.type_name(), r.type_name()),
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
    pub fn call_value(&mut self, _env: &Rc<Env>, callee: &Value, args: Vec<Value>) -> Result<Value, Stress> {
        match callee {
            Value::Gene(def, closure) if def.seq => {
                // calling a sequence starts a worker; pulls are lazy
                if args.len() > def.params.len() && !def.params.is_empty() {
                    self.note(0, 4, format!("{} extra argument(s) in call to sequence ignored", args.len() - def.params.len()));
                }
                let snap = crate::genes::snapshot_globals(self);
                let st = crate::genes::seq_start(def.clone(), args, snap, self.caps.clone());
                Ok(Value::Seq(def.clone(), st))
            }
            Value::Gene(def, closure) => self.call_gene(def.clone(), closure.clone(), args),
            other => {
                self.note(0, 4, format!("called a {} (not a gene); result null", other.type_name()));
                Ok(Value::Null)
            }
        }
    }

    pub fn call_named(&mut self, env: &Rc<Env>, name: &str, args: Vec<Value>) -> Result<Value, Stress> {
        // toggle bistability gate: the repressed allele of a toggle pair refuses
        // calls (acetylated genes override repression — open chromatin wins)
        if let Some(&(ref a, ref b, a_on)) = self.toggles.iter().find(|(a, b, _)| a == name || b == name) {
            let this_is_a = a == name;
            let active = (a_on && this_is_a) || (!a_on && !this_is_a);
            let immune = matches!(env.get(name), Some(Value::Gene(d, _)) if d.acetylate);
            if !active && !immune {
                let winner = if a_on { a } else { b };
                self.note(0, 4, format!("toggle repressed: '{}' is the inactive allele ('{}' is on)", name, winner));
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
            self.note(0, 3, format!("wobble: unknown gene '{}' repaired to builtin '{}'", name, k));
            return self.call_builtin(env, k, args);
        }
        if let Some(g) = self.defined_genes.iter().min_by_key(|g| crate::ffi::edit_distance(name, g)).cloned() {
            let d = crate::ffi::edit_distance(name, &g);
            let max = if name.chars().count() <= 4 { 1 } else { 2 };
            if d <= max && d > 0 {
                self.note(0, 3, format!("wobble: unknown gene '{}' repaired to gene '{}'", name, g));
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

    pub fn call_gene(&mut self, def: Arc<GeneDef>, closure: Option<Rc<Env>>, args: Vec<Value>) -> Result<Value, Stress> {
        // recursion depth limit (SPEC §7): runaway self-transcription is
        // contained as catchable overflow stress instead of a hard crash
        self.depth += 1;
        if self.depth > self.depth_limit {
            self.depth -= 1;
            return Err(Stress::new("overflow", format!("recursion depth limit ({}) exceeded", self.depth_limit)));
        }
        let result = self.call_gene_inner(def, closure, args);
        self.depth -= 1;
        result
    }

    fn call_gene_inner(&mut self, def: Arc<GeneDef>, closure: Option<Rc<Env>>, args: Vec<Value>) -> Result<Value, Stress> {
        let name = def.name.clone().unwrap_or_else(|| "<lambda>".into());
        *self.call_counts.entry(name.clone()).or_insert(0) += 1;
        // burst-index binning: 20 calls per bin, per gene (gene-expression
        // burstiness is measured on per-gene time bins, not across genes)
        self.call_clock += 1;
        let bucket = self.call_clock / 20;
        *self.gene_buckets.entry(name.clone()).or_default().entry(bucket).or_insert(0) += 1;
        // @methylate: transcriptionally repressed genes announce their first
        // call (suppressed by .cell `methylate.quiet = true`)
        if def.methylate && !self.methyl_quiet && !self.methyl_noted.contains(&name) {
            self.methyl_noted.insert(name.clone());
            self.note(0, 4, format!("methylated call: '{}' (chromatin repressed)", name));
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
                self.note(0, 4, format!("missing argument '{}' in call to {}; bound null", pname, name));
                fenv.define(pname, Value::Null);
            }
        }
        if args.len() > def.params.len() && !def.params.is_empty() {
            self.note(0, 4, format!("{} extra argument(s) in call to {} ignored", args.len() - def.params.len(), name));
        }
        // inclusive/exclusive timing: exclusive time subtracts children so a
        // caller never inflates itself with its callees' cost
        let start = if self.profiling { Some(crate::ffi::now_ns()) } else { None };
        self.call_stack.push((name.clone(), start.unwrap_or(0.0), 0.0));
        // uORF guard
        if let Some((cond, gbody)) = &def.guard {
            let ok = self.eval(&fenv, cond).map(|v| v.truthy()).unwrap_or(false);
            if !ok {
                self.note(0, 4, format!("guard tripped calling {}", name));
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
                        self.note(0, 4, format!("guard of {} returned null (uORF repression)", name));
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
    pub fn seq_pull(&mut self, st: &Rc<RefCell<SeqState>>) -> Result<Option<Value>, Stress> {
        loop {
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
                    return Ok(Some(crate::genes::from_send(sv)));
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
                    return Ok(None);
                }
                Err(_) => {
                    b.done = true;
                    return Ok(None);
                }
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
    pub fn construct_obj(&mut self, def: &Arc<PhenoDef>, args: Vec<Value>) -> Result<Value, Stress> {
        let m: crate::value::MapRef = Rc::new(RefCell::new(Vec::new()));
        // gather the lineage root-first
        let chain = self.pheno_chain(def);
        for d in chain.iter().rev() {
            for (fname, fexpr) in &d.fields {
                let v = self.eval(&self.global.clone(), fexpr).unwrap_or(Value::Null);
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
    pub fn call_method_gene(&mut self, def: Arc<GeneDef>, self_val: Value, args: Vec<Value>) -> Result<Value, Stress> {
        self.depth += 1;
        if self.depth > self.depth_limit {
            self.depth -= 1;
            return Err(Stress::new("overflow", format!("recursion depth limit ({}) exceeded", self.depth_limit)));
        }
        let r = self.call_method_gene_inner(def, self_val, args);
        self.depth -= 1;
        r
    }

    fn call_method_gene_inner(&mut self, def: Arc<GeneDef>, self_val: Value, args: Vec<Value>) -> Result<Value, Stress> {
        let name = def.name.clone().unwrap_or_else(|| "<method>".into());
        *self.call_counts.entry(name.clone()).or_insert(0) += 1;
        self.call_clock += 1;
        let bucket = self.call_clock / 20;
        *self.gene_buckets.entry(name.clone()).or_default().entry(bucket).or_insert(0) += 1;
        if def.methylate && !self.methyl_quiet && !self.methyl_noted.contains(&name) {
            self.methyl_noted.insert(name.clone());
            self.note(0, 4, format!("methylated call: '{}' (chromatin repressed)", name));
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
                self.note(0, 4, format!("missing argument '{}' in call to {}; bound null", pname, name));
                fenv.define(pname, Value::Null);
            }
        }
        let start = if self.profiling { Some(crate::ffi::now_ns()) } else { None };
        self.call_stack.push((name.clone(), start.unwrap_or(0.0), 0.0));
        if let Some((cond, gbody)) = &def.guard {
            let ok = self.eval(&fenv, cond).map(|v| v.truthy()).unwrap_or(false);
            if !ok {
                self.note(0, 4, format!("guard tripped calling {}", name));
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
    fn call_builtin(&mut self, env: &Rc<Env>, name: &str, args: Vec<Value>) -> Result<Value, Stress> {
        match name {
            "promote" => {
                let parts: Vec<String> = args.iter().map(|v| v.display()).collect();
                println!("{}", parts.join(" "));
                Ok(Value::Null)
            }
            "len" => Ok(Value::Int(match args.first() {
                Some(Value::Str(s)) => s.chars().count() as i64,
                Some(Value::List(l)) => l.borrow().len() as i64,
                Some(Value::Map(m)) => m.borrow().len() as i64,
                _ => {
                    self.note(0, 4, "len() of non-container is 0");
                    0
                }
            })),
            "push" => {
                if let (Some(Value::List(l)), Some(v)) = (args.first(), args.get(1)) {
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
                (Some(Value::Map(m)), Some(k)) => Ok(Value::Bool(m.borrow().iter().any(|(kk, _)| kk.deep_eq(k)))),
                _ => Ok(Value::Bool(false)),
            },
            "del" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(k)) => {
                    let mut b = m.borrow_mut();
                    if let Some(pos) = b.iter().position(|(kk, _)| kk.deep_eq(k)) {
                        b.remove(pos);
                    }
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
                        self.note(0, 4, "range() needs ints; returned []");
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
            "str" => Ok(Value::Str(args.first().map(|v| v.display()).unwrap_or_default())),
            "num" => match args.first() {
                Some(Value::Str(s)) => {
                    let t = s.trim();
                    if let Ok(i) = t.parse::<i64>() {
                        Ok(Value::Int(i))
                    } else if let Ok(f) = t.parse::<f64>() {
                        Ok(Value::Float(f))
                    } else {
                        self.note(0, 4, format!("num('{}') failed; 0", t));
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
                Some(Value::Int(i)) => Ok(Value::Int(i.abs())),
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
                                    if self.compare(v, &b).unwrap_or(std::cmp::Ordering::Equal) == std::cmp::Ordering::Less {
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
                                if self.compare(a, &b).unwrap_or(std::cmp::Ordering::Equal) == std::cmp::Ordering::Less {
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
                                    if self.compare(v, &b).unwrap_or(std::cmp::Ordering::Equal) == std::cmp::Ordering::Greater {
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
                                if self.compare(a, &b).unwrap_or(std::cmp::Ordering::Equal) == std::cmp::Ordering::Greater {
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
                    let msg = args.get(1).map(|v| v.display()).unwrap_or_else(|| "assertion failed".into());
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
                    return Err(Stress::new("overflow", "distance inputs exceed the 10M-cell DP ceiling"));
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
                let s = args.first().map(|v| v.display()).unwrap_or_default().to_uppercase();
                let n = s.chars().filter(|c| "ATGC".contains(*c)).count();
                if n == 0 {
                    return Ok(Value::Float(0.0));
                }
                let gc = s.chars().filter(|c| *c == 'G' || *c == 'C').count();
                Ok(Value::Float(gc as f64 * 100.0 / n as f64))
            }
            "translate" => {
                let s = args.first().map(|v| v.display()).unwrap_or_default().to_uppercase().replace('U', "T");
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
                let s = args.first().map(|v| v.display()).unwrap_or_default().to_uppercase();
                let orfs = crate::genes::find_orfs(&s);
                Ok(Value::List(Rc::new(RefCell::new(
                    orfs.into_iter().map(Value::Str).collect(),
                ))))
            }
            "memory" => {
                let m = Rc::new(RefCell::new(vec![
                    (Value::Str("arena_bytes".into()), Value::Int(unsafe_arena() as i64)),
                    (Value::Str("interns".into()), Value::Int(unsafe_interns() as i64)),
                    (Value::Str("allocs".into()), Value::Int(unsafe_allocs() as i64)),
                ]));
                Ok(Value::Map(m))
            }
            "methyl" => {
                let k = args.first().map(|v| v.display()).unwrap_or_default();
                let d = args.get(1).cloned().unwrap_or(Value::Null);
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
                let complete_bins = (self.call_clock / 20) as u64;
                let mut burst_total = 0.0;
                let mut burst_n = 0usize;
                let mut burst_by_gene: Vec<(Value, Value)> = Vec::new();
                for (g, bins) in &self.gene_buckets {
                    if complete_bins == 0 {
                        burst_by_gene.push((Value::Str(g.clone()), Value::Float(0.0)));
                        continue;
                    }
                    let n = complete_bins as f64;
                    let total: u64 = (0..complete_bins).map(|b| *bins.get(&b).unwrap_or(&0)).sum();
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
                let burst_avg = if burst_n > 0 { burst_total / burst_n as f64 } else { 0.0 };
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
                let m = Rc::new(RefCell::new(vec![
                    (Value::Str("calls".into()), Value::Map(Rc::new(RefCell::new(counts)))),
                    (Value::Str("burst".into()), Value::Float(burst_avg)),
                    (Value::Str("burst_by_gene".into()), Value::Map(Rc::new(RefCell::new(burst_by_gene)))),
                    (Value::Str("mature".into()), Value::Int(mature as i64)),
                    (Value::Str("nascent".into()), Value::Int(nascent as i64)),
                    (Value::Str("maturation".into()), Value::Float(maturation)),
                ]));
                Ok(Value::Map(m))
            }
            "toggle_on" => {
                let name = args.first().map(|v| v.display()).unwrap_or_default();
                for (a, b, a_on) in self.toggles.iter_mut() {
                    if a == &name || b == &name {
                        *a_on = a == &name;
                        return Ok(Value::Bool(true));
                    }
                }
                self.note(0, 4, format!("toggle pair containing '{}' not declared", name));
                Ok(Value::Bool(false))
            }
            "toggle_state" => {
                let mut out = Vec::new();
                for (a, b, a_on) in &self.toggles {
                    out.push((Value::Str(a.clone()), Value::Bool(*a_on)));
                    out.push((Value::Str(b.clone()), Value::Bool(!*a_on)));
                }
                Ok(Value::Map(Rc::new(RefCell::new(out))))
            }
            "repressi_next" => {
                if self.repressi_ring.is_empty() {
                    self.note(0, 4, "no repressilator declared");
                    return Ok(Value::Null);
                }
                self.repressi_i = (self.repressi_i + 1) % self.repressi_ring.len();
                Ok(Value::Int(self.repressi_i as i64))
            }
            "repressi_start" => {
                // start the wall-clock oscillator: one thread tick advances the
                // ring each period (deterministic manual rings need no thread)
                let ms = match args.first() {
                    Some(Value::Int(i)) => *i as u64,
                    Some(Value::Float(f)) => (*f).max(0.0) as u64,
                    _ => 1000,
                };
                if self.repressi_ring.is_empty() {
                    self.note(0, 4, "repressi_start: no repressilator ring declared");
                    return Ok(Value::Bool(false));
                }
                if ms == 0 {
                    self.note(0, 4, "repressi_start period must be > 0 ms");
                    return Ok(Value::Bool(false));
                }
                let counter = Arc::new(AtomicU64::new(0));
                let c2 = counter.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                });
                self.repressi_atomic = Some(counter);
                self.note(0, 1, format!("repressilator oscillating every {} ms", ms));
                Ok(Value::Bool(true))
            }
            "repressi_state" => {
                let n = self.repressi_ring.len();
                let idx = match &self.repressi_atomic {
                    Some(a) => (a.load(std::sync::atomic::Ordering::SeqCst) as usize) % n.max(1),
                    None => self.repressi_i,
                };
                let mut out = Vec::new();
                for (i, name) in self.repressi_ring.iter().enumerate() {
                    out.push((Value::Str(name.clone()), Value::Float(if i == idx { 1.0 } else { 0.0 })));
                }
                Ok(Value::Map(Rc::new(RefCell::new(out))))
            }
            "grn_fire" => {
                let seed = args.first().map(|v| v.display()).unwrap_or_default();
                // STATEFUL network: levels persist across fires (homeostasis)
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
                            _ => parent * e.strength.powi(_wave),
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
                        .map(|(k, v)| Value::List(Rc::new(RefCell::new(vec![k.clone(), v.clone()]))))
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
                crate::genes::join_task(self, id)
            }
            // -------------------------------------------------- math
            "floor" => Ok(Value::Int(match args.first() {
                Some(Value::Int(i)) => *i,
                Some(Value::Float(f)) => f.floor() as i64,
                other => {
                    self.note(0, 4, format!("floor of {:?}; 0", other.map(|v| v.type_name())));
                    0
                }
            })),
            "ceil" => Ok(Value::Int(match args.first() {
                Some(Value::Int(i)) => *i,
                Some(Value::Float(f)) => f.ceil() as i64,
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
                        self.note(0, 4, "pow(x, y) needs two numbers; 0");
                        (0.0, 0.0)
                    }
                };
                Ok(Value::Float(a.powf(b)))
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
                        let r = (x.wrapping_mul(0x2545F4914F6CDD1D)) as u64;
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
                Some(Value::Int(i)) if *i >= 0 && *i <= 0x10FFFF => {
                    char::from_u32(*i as u32).map(|c| c.to_string()).unwrap_or_default()
                }
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
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Ok(Value::Null)
            }
            "argv" => Ok(Value::List(Rc::new(RefCell::new(
                self.cli_args.iter().map(|a| Value::Str(a.clone())).collect(),
            )))),
            // -------------------------------------------------- filesystem (capability-gated)
            "read_file" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.read, "read", &path)?;
                match std::fs::read_to_string(&path) {
                    Ok(s) => Ok(Value::Str(s)),
                    Err(e) => Err(Stress::new("missing", format!("read_file '{}': {}", path, e))),
                }
            }
            "write_file" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                let body = args.get(1).map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.write, "write", &path)?;
                match std::fs::write(&path, body) {
                    Ok(()) => Ok(Value::Bool(true)),
                    Err(e) => Err(Stress::new("missing", format!("write_file '{}': {}", path, e))),
                }
            }
            "append_file" => {
                let path = args.first().map(|v| v.display()).unwrap_or_default();
                let body = args.get(1).map(|v| v.display()).unwrap_or_default();
                self.caps.check(&self.caps.write, "write", &path)?;
                match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                    Ok(mut f) => {
                        use std::io::Write;
                        match f.write_all(body.as_bytes()) {
                            Ok(()) => Ok(Value::Bool(true)),
                            Err(e) => Err(Stress::new("missing", format!("append_file '{}': {}", path, e))),
                        }
                    }
                    Err(e) => Err(Stress::new("missing", format!("append_file '{}': {}", path, e))),
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
                    Err(e) => Err(Stress::new("missing", format!("read_dir '{}': {}", path, e))),
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
                let out = std::process::Command::new(&prog).args(&prog_args).output();
                match out {
                    Ok(o) => {
                        let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                        let m = Rc::new(RefCell::new(vec![
                            (Value::Str("code".into()), Value::Int(o.status.code().unwrap_or(-1) as i64)),
                            (Value::Str("stdout".into()), Value::Str(stdout)),
                            (Value::Str("stderr".into()), Value::Str(stderr)),
                            (Value::Str("ok".into()), Value::Bool(o.status.success())),
                        ]));
                        Ok(Value::Map(m))
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
                let path = args.get(2).map(|v| v.display()).unwrap_or_else(|| "/".into());
                let target = format!("{}:{}", host, port);
                self.caps.check(&self.caps.net, "net", &target)?;
                match http_get(&host, port, &path) {
                    Ok(body) => Ok(Value::Str(body)),
                    Err(e) => Err(Stress::new("missing", format!("http_get '{}': {}", target, e))),
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
                    Err(e) => Err(Stress::new("missing", format!("serve port {}: {}", port, e))),
                }
            }
            "recv_request" => {
                // poll the shared request queue: {conn, method, path, body} or null
                match crate::interp::recv_request() {
                    Some((conn, method, path, body)) => {
                        let m = Rc::new(RefCell::new(vec![
                            (Value::Str("conn".into()), Value::Int(conn as i64)),
                            (Value::Str("method".into()), Value::Str(method)),
                            (Value::Str("path".into()), Value::Str(path)),
                            (Value::Str("body".into()), Value::Str(body)),
                        ]));
                        Ok(Value::Map(m))
                    }
                    None => Ok(Value::Null),
                }
            }
            "send_response" => {
                let conn = match args.first() {
                    Some(Value::Int(i)) => *i as u64,
                    _ => 0,
                };
                let status = args.get(1).map(|v| v.display()).unwrap_or_else(|| "200".into());
                let ctype = args.get(2).map(|v| v.display()).unwrap_or_else(|| "text/html".into());
                let body = args.get(3).map(|v| v.display()).unwrap_or_default();
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
                self.note(0, 4, format!("unknown builtin '{}'; null", name));
                Ok(Value::Null)
            }
        }
    }

    // ------------------------------------------------------- methods
    fn call_method(&mut self, env: &Rc<Env>, recv: Value, name: &str, args: Vec<Value>) -> Result<Value, Stress> {
        // fate instances intercept first
        if let Value::Map(m) = &recv {
            let is_fate = m.borrow().iter().any(|(k, _)| matches!(k, Value::Str(s) if s == "#fate"));
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
                                format!("fate {}: '{}' → '{}' crosses a valley; state held", fate_name, cur, target),
                            );
                            Some(Ok(Value::Bool(false)))
                        }
                    }
                    "state" => Some(Ok(
                        m.borrow()
                            .iter()
                            .find(|(k, _)| matches!(k, Value::Str(s) if s == "#state"))
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null),
                    )),
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
                "lower" => Ok(Value::Str(s.to_lowercase())),
                "trim" => Ok(Value::Str(s.trim().to_string())),
                "split" => {
                    let sep = args.first().map(|v| v.display()).unwrap_or_else(|| " ".into());
                    Ok(Value::List(Rc::new(RefCell::new(
                        s.split(sep.as_str()).map(|p| Value::Str(p.to_string())).collect(),
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
                        return Err(Stress::new("overflow", "repeat exceeds the 512 MiB string ceiling"));
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
                    let a = a.clamp(0, len) as usize;
                    let b = b.clamp(0, len) as usize;
                    Ok(Value::Str(s.chars().skip(a).take(b.saturating_sub(a)).collect()))
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
                    for v in l.borrow().iter() {
                        out.push(self.call_value(env, &f, vec![v.clone()])?);
                    }
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                "filter" => {
                    let f = args.first().cloned().unwrap_or(Value::Null);
                    let mut out = Vec::new();
                    for v in l.borrow().iter() {
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
                    for v in l.borrow().iter() {
                        acc = self.call_value(env, &f, vec![acc, v.clone()])?;
                    }
                    Ok(acc)
                }
                "each" => {
                    let f = args.first().cloned().unwrap_or(Value::Null);
                    for v in l.borrow().iter() {
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
                        l.borrow().iter().position(|v| v.deep_eq(&t)).map(|i| i as i64).unwrap_or(-1),
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
                    let a = a.clamp(0, len) as usize;
                    let b = b.clamp(0, len) as usize;
                    let out: Vec<Value> = l.borrow()[a..b.max(a)].to_vec();
                    Ok(Value::List(Rc::new(RefCell::new(out))))
                }
                "join" => {
                    let sep = args.first().map(|v| v.display()).unwrap_or_default();
                    let parts: Vec<String> = l.borrow().iter().map(|v| v.display()).collect();
                    Ok(Value::Str(parts.join(&sep)))
                }
                "len" => Ok(Value::Int(l.borrow().len() as i64)),
                "push" => {
                    if let Some(v) = args.first() {
                        l.borrow_mut().push(v.clone());
                    }
                    Ok(Value::List(l.clone()))
                }
                "pop" => Ok(l.borrow_mut().pop().unwrap_or(Value::Null)),
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
                        .map(|(k, v)| Value::List(Rc::new(RefCell::new(vec![k.clone(), v.clone()]))))
                        .collect(),
                )))),
                "has" => {
                    let t = args.first().cloned().unwrap_or(Value::Null);
                    Ok(Value::Bool(m.borrow().iter().any(|(k, _)| k.deep_eq(&t))))
                }
                "del" => {
                    let t = args.first().cloned().unwrap_or(Value::Null);
                    let mut b = m.borrow_mut();
                    if let Some(pos) = b.iter().position(|(k, _)| k.deep_eq(&t)) {
                        b.remove(pos);
                    }
                    Ok(Value::Null)
                }
                "len" => Ok(Value::Int(m.borrow().len() as i64)),
                _ => match m.borrow().iter().find(|(k, _)| matches!(k, Value::Str(s) if s == name)) {
                    Some((_, Value::Gene(_, _))) => {
                        let f = m.borrow().iter().find(|(k, _)| matches!(k, Value::Str(s) if s == name)).map(|(_, v)| v.clone()).unwrap();
                        self.call_value(env, &f, args)
                    }
                    _ => {
                        self.note(0, 4, format!("unknown map method '{}'; null", name));
                        Ok(Value::Null)
                    }
                },
            },
            Value::Seq(_d, st) => match name {
                "next" => {
                    match self.seq_pull(&st)? {
                        Some(v) => Ok(v),
                        None => Ok(Value::Null),
                    }
                }
                "collect" => {
                    let mut out = Vec::new();
                    loop {
                        match self.seq_pull(&st)? {
                            Some(v) => out.push(v),
                            None => break,
                        }
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
                match fields.borrow().iter().find(|(k, _)| matches!(k, Value::Str(s) if s == name)) {
                    Some((_, v)) => self.call_value(env, v, args),
                    None => {
                        self.note(0, 4, format!("phenotype {} has no method '{}'; null", def.name, name));
                        Ok(Value::Null)
                    }
                }
            }
            other => {
                self.note(0, 4, format!("{} has no method '{}'; null", other.type_name(), name));
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
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let conn = next2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // read the request head (and body if declared)
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            let mut head_end = None;
            loop {
                match stream.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => {
                        buf.push(byte[0]);
                        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
                            head_end = Some(buf.len());
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
            let mut body = String::new();
            let mut method = "GET".into();
            let mut path = "/".into();
            if let Some(l0) = head.lines().next() {
                let mut it = l0.split_whitespace();
                method = it.next().unwrap_or("GET").to_string();
                path = it.next().unwrap_or("/").to_string();
            }
            let clen = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().to_string()))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            if clen > 0 && clen <= 8 * 1024 * 1024 {
                let mut rest = vec![0u8; clen];
                if stream.read_exact(&mut rest).is_ok() {
                    body = String::from_utf8_lossy(&rest).to_string();
                }
            }
            let _ = head_end;
            conns2.lock().ok().map(|mut c| c.insert(conn, stream));
            let _ = tx.send((conn, method, path, body));
        }
    });
    *guard = Some(ServerState { rx, conns });
    Ok(())
}

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
    let stream = st.conns.lock().map_err(|_| "conn lock poisoned")?.remove(&conn);
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
    let mut stream = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    let req = format!("GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: operon\r\n\r\n", path, host);
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    // split headers from body
    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        Ok(String::from_utf8_lossy(&buf[pos + 4..]).to_string())
    } else {
        Ok(String::from_utf8_lossy(&buf).to_string())
    }
}

// ------------------------------------------------------------------ json
pub fn json_stringify(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => crate::value::format_float(*f),
        Value::Str(s) => json_quote(s),
        Value::List(l) => {
            let parts: Vec<String> = l.borrow().iter().map(json_stringify).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Map(m) => {
            let parts: Vec<String> = m
                .borrow()
                .iter()
                .map(|(k, v)| format!("{}:{}", json_quote(&k.display()), json_stringify(v)))
                .collect();
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
                        return Ok(Value::Map(Rc::new(RefCell::new(out))));
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
                    Ok(Value::Map(Rc::new(RefCell::new(out))))
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
                                                let c = *p.b.get(p.i + 1 + off + k).ok_or("bad unicode escape")?;
                                                let d = c.to_digit(16).ok_or("bad unicode escape")?;
                                                v = v * 16 + d;
                                            }
                                            Ok(v)
                                        };
                                        let cp = hex(self, 0)?;
                                        self.i += 4;
                                        if (0xD800..0xDC00).contains(&cp) {
                                            // high surrogate: expect a low surrogate next
                                            if self.b.get(self.i + 1) == Some(&'\\') && self.b.get(self.i + 2) == Some(&'u') {
                                                let lo = hex(self, 2)?;
                                                if (0xDC00..0xE000).contains(&lo) {
                                                    self.i += 6;
                                                    let combined = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                                    s.push(char::from_u32(combined).unwrap_or('\u{FFFD}'));
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
                    if let Ok(i) = txt.parse::<i64>() {
                        Ok(Value::Int(i))
                    } else {
                        txt.parse::<f64>().map(Value::Float).map_err(|e| e.to_string())
                    }
                }
            }
        }
        fn expect_word(&mut self, w: &str) -> Result<(), String> {
            let chars: Vec<char> = w.chars().collect();
            if self.b.len() >= self.i + chars.len() && self.b[self.i..self.i + chars.len()] == chars[..] {
                self.i += chars.len();
                Ok(())
            } else {
                Err(format!("expected '{}'", w))
            }
        }
    }
    let mut p = P { b: src.chars().collect(), i: 0, depth: 0 };
    let v = p.value()?;
    p.ws();
    if p.i < p.b.len() {
        return Err(format!("trailing garbage at position {}", p.i));
    }
    Ok(v)
}

pub fn codon_table_char(codon: &str) -> char {
    match codon {
        "TTT" | "TTC" => 'F', "TTA" | "TTG" | "CTT" | "CTC" | "CTA" | "CTG" => 'L',
        "ATT" | "ATC" | "ATA" => 'I', "ATG" => 'M', "GTT" | "GTC" | "GTA" | "GTG" => 'V',
        "TCT" | "TCC" | "TCA" | "TCG" | "AGT" | "AGC" => 'S',
        "CCT" | "CCC" | "CCA" | "CCG" => 'P', "ACT" | "ACC" | "ACA" | "ACG" => 'T',
        "GCT" | "GCC" | "GCA" | "GCG" => 'A', "TAT" | "TAC" => 'Y', "TAA" | "TAG" | "TGA" => '*',
        "CAT" | "CAC" => 'H', "CAA" | "CAG" => 'Q', "AAT" | "AAC" => 'N', "AAA" | "AAG" => 'K',
        "GAT" | "GAC" => 'D', "GAA" | "GAG" => 'E', "TGT" | "TGC" => 'C', "TGG" => 'W',
        "CGT" | "CGC" | "CGA" | "CGG" | "AGA" | "AGG" => 'R', "GGT" | "GGC" | "GGA" | "GGG" => 'G',
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
    "promote", "len", "push", "pop", "insert", "remove", "keys", "values", "has", "del",
    "range", "str", "num", "type", "abs", "min", "max", "sum", "clock", "exit", "assert",
    "codon", "distance", "similar", "transcribe", "reverse_complement", "gc_content",
    "translate", "find_orf", "memory", "methyl", "fingerprint", "toggle_on", "toggle_state",
    "repressi_next", "repressi_state", "repressi_start", "grn_fire", "grn_state", "spawn", "join",
    "floor", "ceil", "sqrt", "pow", "random", "randomize", "chr", "ord", "now", "sleep",
    "argv", "read_file", "write_file", "append_file", "exists", "file_size", "read_dir", "items", "run",
    "http_get", "serve", "recv_request", "send_response", "json_parse", "json_str", "env",
    "call",
];

// direct C kernel accessors for the memory() builtin
pub fn unsafe_arena() -> usize {
    unsafe { crate::ffi::rt_arena_used() }
}
pub fn unsafe_interns() -> u32 {
    unsafe { crate::ffi::rt_intern_count() }
}
pub fn unsafe_allocs() -> u64 {
    unsafe { crate::ffi::rt_alloc_count() }
}
