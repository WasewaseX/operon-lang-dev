//! interp.rs — the tree-walking evaluator. Total Grammar at runtime:
//! soft failures become notes; only catchable Stress propagates.

use crate::ast::*;
use crate::value::{format_float, key_scalar, Stress, Value};
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
    pub done: bool,
}

pub struct Interp {
    pub notes: Vec<Note>,
    pub cell: HashMap<String, String>,
    pub silences: Vec<(String, String)>,
    pub fates: HashMap<String, Arc<FateDef>>,
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
    pub modules: HashMap<String, Value>, // path -> module map
    pub loading: Vec<String>,
    pub profiling: bool,
    pub proof_mode: bool,
    pub spliced_unspliced: (usize, usize),
    pub global: Rc<Env>,
    pub steps: u64,
    pub step_budget: u64,
    pub cli_args: Vec<String>,
    pub tasks: HashMap<i64, TaskHandle>,
    pub next_task_id: i64,
}

pub enum Builtin {
    Done(Result<Value, Stress>),
    NotBuiltin,
}

impl Interp {
    pub fn new() -> Interp {
        Interp {
            notes: Vec::new(),
            cell: HashMap::new(),
            silences: Vec::new(),
            fates: HashMap::new(),
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
            modules: HashMap::new(),
            loading: Vec::new(),
            profiling: false,
            proof_mode: false,
            spliced_unspliced: (0, 0),
            global: Env::new(None),
            steps: 0,
            step_budget: 200_000_000,
            cli_args: Vec::new(),
            tasks: HashMap::new(),
            next_task_id: 1,
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
                    let cur = match &tv {
                        Value::List(l) => {
                            let idx = self.as_index(&iv, l.borrow().len())?;
                            l.borrow().get(idx).cloned().unwrap_or(Value::Null)
                        }
                        Value::Map(m) => m.borrow().iter().find(|(k, _)| k.deep_eq(&iv)).map(|(_, v)| v.clone()).unwrap_or(Value::Null),
                        _ => Value::Null,
                    };
                    val = self.apply_binop(env, *binop, &cur, &val)?;
                }
                match (&tv, &iv) {
                    (Value::List(l), _) => {
                        let idx = self.as_index(&iv, l.borrow().len())?;
                        if idx < l.borrow().len() {
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
                            let lv = self.eval(env, l).unwrap_or(Value::Null);
                            sv.deep_eq(&lv)
                        }
                        MatchPat::Multi(ls) => ls.iter().any(|l| {
                            let lv = self.eval(env, l).unwrap_or(Value::Null);
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
                        env.define(&name, modv);
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
                                self.exec_block(&child, rbody)?;
                                Ok(Flow::Norm)
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
        }
    }

    pub fn stress_map(&self, s: &Stress) -> Value {
        let m = Rc::new(RefCell::new(vec![
            (Value::Str("kind".into()), Value::Str(s.kind.clone())),
            (Value::Str("message".into()), Value::Str(s.message.clone())),
        ]));
        Value::Map(m)
    }

    /// Contain a stress: uncaught at statement level → note + continue.
    fn contain(&mut self, _env: &Rc<Env>, s: Stress) -> Result<Value, Stress> {
        self.note(0, 4, format!("stress contained: [{}] {}", s.kind, s.message));
        Ok(Value::Null)
    }

    pub fn as_index(&self, v: &Value, len: usize) -> Result<usize, Stress> {
        match v {
            Value::Int(i) => {
                if *i < 0 {
                    Ok((len as i64 + *i).max(0) as usize)
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
                        InterpPart::Expr(e) => {
                            let v = self.eval(env, e)?;
                            out.push_str(&v.display());
                        }
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
                let items: Vec<Value> = match itv {
                    Value::List(l) => l.borrow().clone(),
                    Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    Value::Map(m) => m.borrow().iter().map(|(k, _)| k.clone()).collect(),
                    _ => Vec::new(),
                };
                let mut out = Vec::new();
                for item in items {
                    self.tick()?;
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
    pub fn call_value(&mut self, env: &Rc<Env>, callee: &Value, args: Vec<Value>) -> Result<Value, Stress> {
        match callee {
            Value::Gene(def, closure) => self.call_gene(def.clone(), closure.clone(), args),
            Value::Native(name) => {
                let n = *name;
                self.call_builtin(env, n, args)
            }
            other => {
                self.note(0, 4, format!("called a {} (not a gene); result null", other.type_name()));
                Ok(Value::Null)
            }
        }
    }

    pub fn call_named(&mut self, env: &Rc<Env>, name: &str, args: Vec<Value>) -> Result<Value, Stress> {
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
        let name = def.name.clone().unwrap_or_else(|| "<lambda>".into());
        *self.call_counts.entry(name.clone()).or_insert(0) += 1;
        let base = self.call_counts.get(&name).cloned().unwrap_or(0);
        if self.profiling {
            *self.call_time.entry(name.clone()).or_insert(0.0) += 0.0; // replaced by timed wrapper in profile mode
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
        // entry bookkeeping for telemetry
        let start = if self.profiling { Some(crate::ffi::now_ns()) } else { None };
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
        if let Some(t0) = start {
            let dt = (crate::ffi::now_ns() - t0) / 1000.0;
            *self.call_time.entry(name.clone()).or_insert(0.0) += dt;
        }
        let _ = base;
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
            "type" => Ok(Value::Str(
                args.first().map(|v| v.type_name().to_string()).unwrap_or_default(),
            )),
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
                // Fano factor over per-gene call counts
                let n = self.call_counts.len();
                let mean = if n > 0 {
                    self.call_counts.values().sum::<u64>() as f64 / n as f64
                } else {
                    0.0
                };
                let var = if n > 0 {
                    self.call_counts.values().map(|c| (*c as f64 - mean).powi(2)).sum::<f64>() / n as f64
                } else {
                    0.0
                };
                let fano = if mean > 0.0 { var / mean } else { 0.0 };
                let total_defined = self.defined_genes.len();
                let spliced = self.call_counts.len().min(total_defined);
                let unspliced = total_defined.saturating_sub(spliced);
                let velocity = if total_defined > 0 {
                    unspliced as f64 / total_defined as f64
                } else {
                    0.0
                };
                let m = Rc::new(RefCell::new(vec![
                    (Value::Str("calls".into()), Value::Map(Rc::new(RefCell::new(counts)))),
                    (Value::Str("fano".into()), Value::Float(fano)),
                    (Value::Str("spliced".into()), Value::Int(spliced as i64)),
                    (Value::Str("unspliced".into()), Value::Int(unspliced as i64)),
                    (Value::Str("velocity".into()), Value::Float(velocity)),
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
                self.grn_levels.clear();
                for e in &self.grn_edges {
                    self.grn_levels.entry(e.from.clone()).or_insert(0.0);
                    self.grn_levels.entry(e.to.clone()).or_insert(0.0);
                }
                *self.grn_levels.entry(seed.clone()).or_insert(0.0) = 1.0;
                // propagate waves (activates: max-propagate × strength^wave; inhibits: subtract)
                for _wave in 0..10 {
                    let mut changed = false;
                    let snapshot = self.grn_levels.clone();
                    for e in &self.grn_edges {
                        let parent = *snapshot.get(&e.from).unwrap_or(&0.0);
                        if parent <= 0.0 {
                            continue;
                        }
                        let influence = parent * e.strength.powi(_wave as i32 + 1);
                        let cur = *self.grn_levels.get(&e.to).unwrap_or(&0.0);
                        let next = if e.inhibit {
                            (cur - influence).max(0.0)
                        } else {
                            cur.max(influence)
                        };
                        if (next - cur).abs() > 1e-12 {
                            self.grn_levels.insert(e.to.clone(), next);
                            changed = true;
                        }
                    }
                    if !changed {
                        break;
                    }
                }
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
                    if let Value::Gene(_, _) | Value::Native(_) = &cmp {
                        // insertion sort with user comparator (a before b)
                        for i in 1..v.len() {
                            let mut j = i;
                            while j > 0 {
                                let a = v[j - 1].clone();
                                let b = v[j].clone();
                                let before = self.call_value(env, &cmp, vec![a, b])?.truthy();
                                if before {
                                    v.swap(j - 1, j);
                                    j -= 1;
                                } else {
                                    break;
                                }
                            }
                        }
                    } else {
                        // default: numbers and strings ascending
                        v.sort_by(|a, b| match (a, b) {
                            (Value::Int(x), Value::Int(y)) => x.cmp(y),
                            (Value::Str(x), Value::Str(y)) => x.cmp(y),
                            _ => {
                                let (x, y) = self.as_floats(a, b).unwrap_or((0.0, 0.0));
                                x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
                            }
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
                    Some((_, Value::Gene(_, _)) | (_, Value::Native(_))) => {
                        let f = m.borrow().iter().find(|(k, _)| matches!(k, Value::Str(s) if s == name)).map(|(_, v)| v.clone()).unwrap();
                        self.call_value(env, &f, args)
                    }
                    _ => {
                        self.note(0, 4, format!("unknown map method '{}'; null", name));
                        Ok(Value::Null)
                    }
                },
            },
            other => {
                self.note(0, 4, format!("{} has no method '{}'; null", other.type_name(), name));
                Ok(Value::Null)
            }
        }
    }
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
    "repressi_next", "repressi_state", "grn_fire", "grn_state", "spawn", "join",
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
