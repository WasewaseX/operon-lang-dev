//! compile.rs — W09 (A1/A2/A3): the bytecode compiler.
//!
//! Compiles gene bodies into `FuncCode` objects for the stack-machine VM in
//! vm.rs. Design rule W09-1 (docs/VM.md): the compiler owns only control
//! flow and operand plumbing; every semantic operation becomes an
//! instruction whose handler delegates to the same funnel the tree-walk
//! uses. Constructs without hot-loop value compile to `Insn::Stmt` /
//! `Insn::Expr` delegation — compilation is total (Total Grammar carries
//! over: a construct the compiler does not specialize still runs).

use crate::ast::*;
use crate::value::Value;
use std::rc::Rc;
use std::sync::Arc;

/// One compiled function (a gene body). `consts` holds literal values,
/// `names` interned identifiers; both are referenced by index.
pub struct FuncCode {
    pub name: String,
    pub insns: Vec<Insn>,
    pub consts: Vec<Value>,
    pub names: Vec<String>,
}

pub enum Insn {
    // operands
    Const(u32),
    Pop,
    Dup,
    /// Ident arm mirror: get + clone charge + unbound note.
    Load(u32),
    /// The `op=` current-value read: get without charge or note.
    LoadPlain(u32),
    Define(u32),
    /// W01: annotated let — check then define (mismatch = unfolded stress).
    DefineAnn(u32, Rc<TypeAnn>),
    Store(u32),
    StoreOp(u32, BinOp),
    // operators
    /// dx-r4 stamp trajectory: cur_line = l (emitted at stamping arm entries).
    Line(u32),
    Bin(BinOp),
    AndJmp(u32),
    OrJmp(u32),
    NullishJmp(u32),
    Un(UnOp),
    Index,
    Member(u32),
    MemberSafe(u32),
    Method(u32, u8),
    MethodSafe(u32, u8),
    MakeList(u16),
    MakeMap(u16),
    // control
    Jmp(u32),
    JmpIfFalse(u32),
    Tick,
    PushScope,
    PopScope,
    Ret,
    RetNull,
    // calls
    CallStart(u32, u32),
    IfDegraded(u8, u32),
    CallFinish(u8, u32),
    CallValue(u8),
    // loops
    IterMake,
    IterNext(u32),
    IterBindName(u32),
    IterEnd,
    LoopPop,
    /// push a runtime loop record: (continue-target pc, exit/LoopPop pc).
    /// Emitted ONCE per loop, before the top; LoopPop pops it.
    LoopEnter(u32, u32),
    // stress
    CatchEnter(Option<u32>, u32, u32),
    CatchLeave,
    CatchTrim(u32),
    RescueTicks,
    RescueBind(Option<u32>),
    RaiseStmt(Option<u32>, u32),
    // delegation (total coverage)
    Stmt(Rc<Stmt>),
    Expr(Rc<Expr>),
    /// W11 (R1): filler left by the optimizer where a folded insn used to
    /// be. Executes as nothing (no tick, no effect); keeps every pc stable
    /// so folding never has to rewrite jump targets.
    Nop,
}

/// The compiled unit for one program: gene bodies keyed by their
/// `Arc<GeneDef>` identity pointer, plus delegation coverage stats. Codes
/// are un-wrapped so the W11 optimizer can rewrite them before the caller
/// pins them behind `Rc` for execution.
pub struct Unit {
    pub funcs: Vec<(usize, FuncCode)>,
    pub delegated: Vec<&'static str>,
}

struct Compiler {
    funcs: Vec<(usize, FuncCode)>,
    seen: std::collections::HashSet<usize>,
    delegated: Vec<&'static str>,
}

impl Compiler {
    fn new() -> Self {
        Compiler {
            funcs: Vec::new(),
            seen: std::collections::HashSet::new(),
            delegated: Vec::new(),
        }
    }

    fn mark_delegated(&mut self, tag: &'static str) {
        if !self.delegated.contains(&tag) {
            self.delegated.push(tag);
        }
    }

    /// Walk the whole program collecting compilable function definitions.
    /// Sequences are deliberately skipped (A4 boundary: worker Interps have
    /// no vm_funcs and fall back to the tree-walk by construction).
    fn collect_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Gene(def) => self.collect_gene(def),
            Stmt::Seq(_) => {
                self.mark_delegated("sequence-def");
            }
            Stmt::Pheno(d) => {
                for m in &d.methods {
                    self.collect_gene(m);
                }
            }
            Stmt::Splice(d) => {
                for (_, v) in &d.variants {
                    self.collect_gene(v);
                }
            }
            Stmt::Tad(_, body) => {
                for s2 in body {
                    self.collect_stmt(s2);
                }
            }
            Stmt::Frame { body, .. } => {
                for s2 in body {
                    self.collect_stmt(s2);
                }
            }
            _ => {
                // walk expressions of otherwise-delegated statements for
                // lambdas and nested defs
                self.walk_stmt_exprs(s);
            }
        }
    }

    fn collect_gene(&mut self, def: &Arc<GeneDef>) {
        let key = Arc::as_ptr(def) as *const u8 as usize;
        if !self.seen.insert(key) {
            return;
        }
        // nested defs inside the body first (inner Arcs must register too)
        for s in &def.body {
            self.collect_stmt(s);
        }
        self.walk_gene_exprs(def);
        let mut fc = FuncCompiler::new(def.name.clone().unwrap_or_else(|| "<lambda>".into()));
        fc.stmts(&def.body);
        for tag in fc.delegated.drain(..) {
            if !self.delegated.contains(&tag) {
                self.delegated.push(tag);
            }
        }
        self.funcs.push((key, fc.finish()));
    }

    fn walk_stmt_exprs(&mut self, s: &Stmt) {
        match s {
            Stmt::Gene(d) | Stmt::Seq(d) => self.collect_gene_or_skip(d, true),
            Stmt::Pheno(d) => {
                for (_, e) in &d.fields {
                    self.walk_expr(e);
                }
                for m in &d.methods {
                    self.collect_gene(m);
                }
            }
            Stmt::Splice(d) => {
                for (_, v) in &d.variants {
                    self.collect_gene(v);
                }
            }
            Stmt::Let(_, e) | Stmt::Return(Some(e)) | Stmt::ExprStmt(e) | Stmt::Yield(Some(e)) => {
                self.walk_expr(e)
            }
            Stmt::LetAnn(_, _, e) | Stmt::Raise(_, e, _) => self.walk_expr(e),
            Stmt::Assign(_, _, e) | Stmt::LetPat(_, e) => self.walk_expr(e),
            Stmt::IndexAssign(t, i, _, e) => {
                self.walk_expr(t);
                self.walk_expr(i);
                self.walk_expr(e);
            }
            Stmt::MemberAssign(t, _, _, e) => {
                self.walk_expr(t);
                self.walk_expr(e);
            }
            Stmt::MultiAssign(ts, vs, _) => {
                for e in ts.iter().chain(vs.iter()) {
                    self.walk_expr(e);
                }
            }
            Stmt::If(branches, els) => {
                for (c, body) in branches {
                    self.walk_expr(c);
                    for s2 in body {
                        self.collect_stmt(s2);
                    }
                }
                if let Some(eb) = els {
                    for s2 in eb {
                        self.collect_stmt(s2);
                    }
                }
            }
            Stmt::While(c, body) | Stmt::ForPat(_, c, body) => {
                self.walk_expr(c);
                for s2 in body {
                    self.collect_stmt(s2);
                }
            }
            Stmt::For(_, it, body) => {
                self.walk_expr(it);
                for s2 in body {
                    self.collect_stmt(s2);
                }
            }
            Stmt::Loop(body) => {
                for s2 in body {
                    self.collect_stmt(s2);
                }
            }
            Stmt::Match(subj, arms) => {
                self.walk_expr(subj);
                for (pat, body) in arms {
                    self.walk_pat_exprs(pat);
                    for s2 in body {
                        self.collect_stmt(s2);
                    }
                }
            }
            Stmt::Stress { body, rescue, .. } => {
                for s2 in body {
                    self.collect_stmt(s2);
                }
                if let Some((_, rb)) = rescue {
                    for s2 in rb {
                        self.collect_stmt(s2);
                    }
                }
            }
            Stmt::Block(body) | Stmt::Tad(_, body) | Stmt::Frame { body, .. } => {
                for s2 in body {
                    self.collect_stmt(s2);
                }
            }
            _ => {}
        }
    }

    fn collect_gene_or_skip(&mut self, def: &Arc<GeneDef>, allow_seq: bool) {
        if !allow_seq && def.seq {
            return;
        }
        self.collect_gene(def);
    }

    fn walk_pat_exprs(&mut self, p: &MatchPat) {
        match p {
            MatchPat::Lit(e) => self.walk_expr(e),
            MatchPat::Multi(es) => {
                for e in es {
                    self.walk_expr(e);
                }
            }
            MatchPat::Guard(inner, cond) => {
                self.walk_pat_exprs(inner);
                self.walk_expr(cond);
            }
            MatchPat::Variant(_, Some(p2)) => self.walk_pat_exprs(p2),
            MatchPat::ListPat { elems, .. } => {
                for e in elems {
                    self.walk_pat_exprs(e);
                }
            }
            MatchPat::Or(alts) => {
                for e in alts {
                    self.walk_pat_exprs(e);
                }
            }
            _ => {}
        }
    }

    fn walk_expr(&mut self, e: &Expr) {
        match e {
            Expr::Lambda(def) => self.collect_gene(def),
            Expr::Unary(_, a) => self.walk_expr(a),
            Expr::Binary(_, a, b, _) => {
                self.walk_expr(a);
                self.walk_expr(b);
            }
            Expr::Call(c, args, _) => {
                self.walk_expr(c);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Index(a, b, _) => {
                self.walk_expr(a);
                self.walk_expr(b);
            }
            Expr::Member(a, _) | Expr::MemberSafe(a, _) => self.walk_expr(a),
            Expr::Method(a, _, args) | Expr::MethodSafe(a, _, args) => {
                self.walk_expr(a);
                for x in args {
                    self.walk_expr(x);
                }
            }
            Expr::List(items) => {
                for i in items {
                    self.walk_expr(i);
                }
            }
            Expr::Map(pairs) => {
                for (k, v) in pairs {
                    self.walk_expr(k);
                    self.walk_expr(v);
                }
            }
            Expr::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(e2) = p {
                        self.walk_expr(e2);
                    }
                }
            }
            Expr::Collect {
                iter, filter, body, ..
            } => {
                self.walk_expr(iter);
                if let Some(f) = filter {
                    self.walk_expr(f);
                }
                self.walk_expr(body);
            }
            Expr::New(_, args) => {
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Ternary(c, a, b) => {
                self.walk_expr(c);
                self.walk_expr(a);
                self.walk_expr(b);
            }
            Expr::Propagate(a, _) => self.walk_expr(a),
            _ => {}
        }
    }

    fn walk_gene_exprs(&mut self, def: &GeneDef) {
        if let Some((c, gbody)) = &def.guard {
            self.walk_expr(c);
            for s in gbody {
                self.collect_stmt(s);
            }
        }
        for (_, d) in &def.params {
            if let Some(e) = d {
                self.walk_expr(e);
            }
        }
    }
}

/// Per-function compilation state.
struct FuncCompiler {
    name: String,
    insns: Vec<Insn>,
    consts: Vec<Value>,
    names: Vec<String>,
    /// static scope depth (PushScope emitted minus PopScope emitted)
    depth: u32,
    /// catch nesting depth at the current emit point
    catch_depth: u32,
    loops: Vec<LoopCtx>,
    /// delegation coverage tags (merged into the Unit by the Compiler)
    delegated: Vec<&'static str>,
}

struct LoopCtx {
    /// pc to jump for continue (loop top)
    top: u32,
    /// static scope depth at loop entry
    depth: u32,
    /// catch depth at loop entry
    catch_depth: u32,
    /// patch sites for the break jumps (all target the LoopPop)
    breaks: Vec<u32>,
}

impl FuncCompiler {
    fn new(name: String) -> Self {
        FuncCompiler {
            name,
            insns: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            depth: 0,
            catch_depth: 0,
            loops: Vec::new(),
            delegated: Vec::new(),
        }
    }

    fn mark_delegated(&mut self, tag: &'static str) {
        if !self.delegated.contains(&tag) {
            self.delegated.push(tag);
        }
    }

    fn finish(self) -> FuncCode {
        FuncCode {
            name: self.name,
            insns: self.insns,
            consts: self.consts,
            names: self.names,
        }
    }

    fn here(&self) -> u32 {
        self.insns.len() as u32
    }

    fn emit(&mut self, i: Insn) -> u32 {
        self.insns.push(i);
        (self.insns.len() - 1) as u32
    }

    fn konst(&mut self, v: Value) -> u32 {
        // dedupe identical constants (cheap linear scan; pools are small)
        for (i, c) in self.consts.iter().enumerate() {
            if c.deep_eq(&v) && std::mem::discriminant(c) == std::mem::discriminant(&v) {
                return i as u32;
            }
        }
        self.consts.push(v);
        (self.consts.len() - 1) as u32
    }

    fn intern(&mut self, n: &str) -> u32 {
        for (i, s) in self.names.iter().enumerate() {
            if s == n {
                return i as u32;
            }
        }
        self.names.push(n.to_string());
        (self.names.len() - 1) as u32
    }

    // ---------- statements ----------

    fn stmts(&mut self, body: &[Stmt]) {
        for s in body {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let(name, e) => {
                self.expr(e);
                let k = self.intern(name);
                self.emit(Insn::Define(k));
            }
            Stmt::LetAnn(name, ann, e) => {
                self.expr(e);
                let k = self.intern(name);
                self.emit(Insn::DefineAnn(k, Rc::new(ann.clone())));
            }
            Stmt::Assign(name, None, e) => {
                self.expr(e);
                let k = self.intern(name);
                self.emit(Insn::Store(k));
            }
            Stmt::Assign(name, Some(op), e) => {
                self.expr(e);
                let k = self.intern(name);
                self.emit(Insn::StoreOp(k, *op));
            }
            Stmt::If(branches, els) => {
                // per-branch child scope; jumps chain like the arm's early return
                let mut jmp_ends = Vec::new();
                let n = branches.len();
                for (bi, (cond, body)) in branches.iter().enumerate() {
                    self.expr(cond);
                    let next = self.emit(Insn::JmpIfFalse(0));
                    self.emit(Insn::PushScope);
                    self.depth += 1;
                    self.stmts(body);
                    self.emit(Insn::PopScope);
                    self.depth -= 1;
                    jmp_ends.push(self.emit(Insn::Jmp(0)));
                    let _ = bi;
                    // patch the false-jump to the next branch's start
                    let t = self.here();
                    self.insns[next as usize] = Insn::JmpIfFalse(t);
                    let _ = n;
                }
                if let Some(eb) = els {
                    self.emit(Insn::PushScope);
                    self.depth += 1;
                    self.stmts(eb);
                    self.emit(Insn::PopScope);
                    self.depth -= 1;
                }
                let end = self.here();
                for j in jmp_ends {
                    self.insns[j as usize] = Insn::Jmp(end);
                }
            }
            Stmt::While(cond, body) => {
                let enter = self.emit(Insn::LoopEnter(0, 0)); // patched below
                let top = self.here();
                self.emit(Insn::Tick);
                self.expr(cond);
                let exit = self.emit(Insn::JmpIfFalse(0));
                self.emit(Insn::PushScope);
                self.depth += 1;
                let lc = LoopCtx {
                    top,
                    depth: self.depth - 1,
                    catch_depth: self.catch_depth,
                    breaks: Vec::new(),
                };
                let lidx = self.loops.len();
                self.loops.push(lc);
                self.stmts(body);
                // break patch sites point at the LoopPop
                self.emit(Insn::PopScope);
                self.depth -= 1;
                self.emit(Insn::Jmp(top));
                let loop_pop = self.emit(Insn::LoopPop);
                let lc = self.loops.remove(lidx);
                self.insns[exit as usize] = Insn::JmpIfFalse(loop_pop);
                self.insns[enter as usize] = Insn::LoopEnter(top, loop_pop);
                for b in lc.breaks {
                    self.insns[b as usize] = Insn::Jmp(loop_pop);
                }
            }
            Stmt::Loop(body) => {
                let enter = self.emit(Insn::LoopEnter(0, 0)); // patched below
                let top = self.here();
                self.emit(Insn::Tick);
                self.emit(Insn::PushScope);
                self.depth += 1;
                let lc = LoopCtx {
                    top,
                    depth: self.depth - 1,
                    catch_depth: self.catch_depth,
                    breaks: Vec::new(),
                };
                let lidx = self.loops.len();
                self.loops.push(lc);
                self.stmts(body);
                self.emit(Insn::PopScope);
                self.depth -= 1;
                self.emit(Insn::Jmp(top));
                let loop_pop = self.emit(Insn::LoopPop);
                let lc = self.loops.remove(lidx);
                self.insns[enter as usize] = Insn::LoopEnter(top, loop_pop);
                for b in lc.breaks {
                    self.insns[b as usize] = Insn::Jmp(loop_pop);
                }
            }
            Stmt::For(name, iter, body) => {
                self.expr(iter);
                self.emit(Insn::IterMake);
                let enter = self.emit(Insn::LoopEnter(0, 0)); // patched below
                let top = self.here();
                let exit = self.emit(Insn::IterNext(0));
                self.emit(Insn::PushScope);
                self.depth += 1;
                let k = self.intern(name);
                self.emit(Insn::IterBindName(k));
                let lc = LoopCtx {
                    top,
                    depth: self.depth - 1,
                    catch_depth: self.catch_depth,
                    breaks: Vec::new(),
                };
                let lidx = self.loops.len();
                self.loops.push(lc);
                self.stmts(body);
                self.emit(Insn::PopScope);
                self.depth -= 1;
                self.emit(Insn::Jmp(top));
                let loop_pop = self.emit(Insn::LoopPop);
                let _ = self.emit(Insn::IterEnd);
                let lc = self.loops.remove(lidx);
                self.insns[exit as usize] = Insn::IterNext(loop_pop);
                self.insns[enter as usize] = Insn::LoopEnter(top, loop_pop);
                for b in lc.breaks {
                    self.insns[b as usize] = Insn::Jmp(loop_pop);
                }
            }
            Stmt::Return(Some(e)) => {
                self.expr(e);
                self.emit(Insn::Ret);
            }
            Stmt::Return(None) => {
                self.emit(Insn::RetNull);
            }
            Stmt::Break => {
                if let Some(li) = self.loops.len().checked_sub(1) {
                    let (depth, cd) = {
                        let l = &self.loops[li];
                        (l.depth, l.catch_depth)
                    };
                    self.emit(Insn::CatchTrim(cd));
                    for _ in 0..(self.depth - depth) {
                        self.emit(Insn::PopScope);
                    }
                    // the trailing Jmp is patched to Jmp(loop_pop) in the
                    // loop epilogue (only the Jmp pc is recorded — the
                    // CatchTrim/PopScope prefix must not be overwritten)
                    self.emit(Insn::Jmp(0));
                    let jpc = self.here() - 1;
                    self.loops[li].breaks.push(jpc);
                } else {
                    // no enclosing compiled loop: delegate (tree-walk flow)
                    let r = Rc::new(s.clone());
                    self.emit(Insn::Stmt(r));
                }
            }
            Stmt::Continue => {
                if let Some(li) = self.loops.len().checked_sub(1) {
                    let (depth, cd, top) = {
                        let l = &self.loops[li];
                        (l.depth, l.catch_depth, l.top)
                    };
                    self.emit(Insn::CatchTrim(cd));
                    for _ in 0..(self.depth - depth) {
                        self.emit(Insn::PopScope);
                    }
                    self.emit(Insn::Jmp(top));
                } else {
                    let r = Rc::new(s.clone());
                    self.emit(Insn::Stmt(r));
                }
            }
            Stmt::ExprStmt(e) => {
                self.expr(e);
                self.emit(Insn::Pop);
            }
            Stmt::Block(body) => {
                self.emit(Insn::PushScope);
                self.depth += 1;
                self.stmts(body);
                self.emit(Insn::PopScope);
                self.depth -= 1;
            }
            Stmt::Stress { kind, body, rescue } => {
                // CatchEnter(start, end= CatchLeave pc, handler) — patched
                let kind_k = kind.as_ref().map(|k| self.intern(k));
                let enter = self.emit(Insn::CatchEnter(None, 0, 0));
                let start = enter + 1;
                self.catch_depth += 1;
                self.stmts(body);
                self.emit(Insn::CatchLeave);
                let leave = self.here() - 1;
                self.catch_depth -= 1;
                // skip over the handler
                let skip = self.emit(Insn::Jmp(0));
                let handler = self.here();
                self.emit(Insn::RescueTicks);
                self.emit(Insn::PushScope);
                self.depth += 1;
                let bind_k = match rescue {
                    Some((Some(b), _)) => Some(self.intern(b)),
                    _ => None,
                };
                self.emit(Insn::RescueBind(bind_k));
                if let Some((_, rb)) = rescue {
                    self.stmts(rb);
                }
                self.emit(Insn::PopScope);
                self.depth -= 1;
                let join = self.here();
                self.insns[skip as usize] = Insn::Jmp(join);
                self.insns[enter as usize] = Insn::CatchEnter(kind_k, leave, handler);
                let _ = start;
            }
            _ => {
                // delegated statement — total coverage
                self.delegate_stmt(s);
            }
        }
    }

    fn delegate_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Use(..) => self.mark_delegated("use"),
            Stmt::Match(..) => self.mark_delegated("match"),
            Stmt::LetPat(..) => self.mark_delegated("destructure-let"),
            Stmt::ForPat(..) => self.mark_delegated("destructure-for"),
            Stmt::MultiAssign(..) => self.mark_delegated("multi-assign"),
            Stmt::IndexAssign(..) => self.mark_delegated("index-assign"),
            Stmt::MemberAssign(..) => self.mark_delegated("member-assign"),
            Stmt::Seq(_) => self.mark_delegated("sequence-def"),
            Stmt::Pheno(_) => self.mark_delegated("phenotype-def"),
            Stmt::Raise(..) => self.mark_delegated("raise"),
            _ => self.mark_delegated("decl-or-effect"),
        }
        let r = Rc::new(s.clone());
        self.emit(Insn::Stmt(r));
    }

    // ---------- expressions ----------

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Null => {
                let k = self.konst(Value::Null);
                self.emit(Insn::Const(k));
            }
            Expr::Bool(b) => {
                let k = self.konst(Value::Bool(*b));
                self.emit(Insn::Const(k));
            }
            Expr::Int(i) => {
                let k = self.konst(Value::Int(*i));
                self.emit(Insn::Const(k));
            }
            Expr::Float(f) => {
                let k = self.konst(Value::Float(*f));
                self.emit(Insn::Const(k));
            }
            Expr::Str(s) => {
                let k = self.konst(Value::Str(s.clone()));
                self.emit(Insn::Const(k));
            }
            Expr::Ident(name) => {
                let k = self.intern(name);
                self.emit(Insn::Load(k));
            }
            Expr::Unary(op, a) => {
                self.expr(a);
                self.emit(Insn::Un(*op));
            }
            Expr::Binary(op, l, r, line) => {
                // short-circuit forms compile to jumps; the LEFT value stays
                // as the result when short-circuiting (arm-exact)
                match op {
                    BinOp::And => {
                        self.expr(l);
                        let j = self.emit(Insn::AndJmp(0));
                        self.expr(r);
                        let end = self.here();
                        self.insns[j as usize] = Insn::AndJmp(end);
                    }
                    BinOp::Or => {
                        self.expr(l);
                        let j = self.emit(Insn::OrJmp(0));
                        self.expr(r);
                        let end = self.here();
                        self.insns[j as usize] = Insn::OrJmp(end);
                    }
                    BinOp::Nullish => {
                        self.expr(l);
                        let j = self.emit(Insn::NullishJmp(0));
                        self.expr(r);
                        let end = self.here();
                        self.insns[j as usize] = Insn::NullishJmp(end);
                    }
                    _ => {
                        self.emit(Insn::Line(*line as u32));
                        self.expr(l);
                        self.expr(r);
                        self.emit(Insn::Bin(*op));
                    }
                }
            }
            Expr::Ternary(c, a, b) => {
                self.expr(c);
                let jf = self.emit(Insn::JmpIfFalse(0));
                self.expr(a);
                let j = self.emit(Insn::Jmp(0));
                let tb = self.here();
                self.insns[jf as usize] = Insn::JmpIfFalse(tb);
                self.expr(b);
                let end = self.here();
                self.insns[j as usize] = Insn::Jmp(end);
            }
            Expr::Index(t, i, line) => {
                self.emit(Insn::Line(*line as u32));
                self.expr(t);
                self.expr(i);
                self.emit(Insn::Index);
            }
            Expr::Member(t, key) => {
                self.expr(t);
                let k = self.intern(key);
                self.emit(Insn::Member(k));
            }
            Expr::MemberSafe(t, key) => {
                self.expr(t);
                let k = self.intern(key);
                self.emit(Insn::MemberSafe(k));
            }
            Expr::Method(t, name, args) => {
                self.expr(t);
                for a in args {
                    self.expr(a);
                }
                let k = self.intern(name);
                self.emit(Insn::Method(k, args.len() as u8));
            }
            Expr::MethodSafe(t, name, args) => {
                self.expr(t);
                for a in args {
                    self.expr(a);
                }
                let k = self.intern(name);
                self.emit(Insn::MethodSafe(k, args.len() as u8));
            }
            Expr::List(items) => {
                for i in items {
                    self.expr(i);
                }
                self.emit(Insn::MakeList(items.len() as u16));
            }
            Expr::Map(pairs) => {
                for (k, v) in pairs {
                    self.expr(k);
                    self.expr(v);
                }
                self.emit(Insn::MakeMap(pairs.len() as u16));
            }
            Expr::Call(callee, args, line) => {
                self.emit(Insn::Line(*line as u32));
                if let Expr::Ident(name) = &**callee {
                    // named path: RISC decision BEFORE args (order parity);
                    // degraded calls skip argument evaluation entirely
                    let k = self.intern(name);
                    self.emit(Insn::CallStart(k, *line as u32));
                    let deg = self.emit(Insn::IfDegraded(args.len() as u8, 0));
                    for a in args {
                        self.expr(a);
                    }
                    let fin = self.emit(Insn::CallFinish(args.len() as u8, k));
                    let after = self.here();
                    self.insns[deg as usize] = Insn::IfDegraded(args.len() as u8, after);
                    let _ = fin;
                } else {
                    self.expr(callee);
                    for a in args {
                        self.expr(a);
                    }
                    self.emit(Insn::CallValue(args.len() as u8));
                }
            }
            _ => {
                // delegated leaf expression (interpolation, comprehension,
                // lambda construction, New, ?!, …)
                let r = Rc::new(e.clone());
                self.emit(Insn::Expr(r));
            }
        }
    }
}

// --------------------------------------------------------------- W11: opt

/// W11 (R1): semantics-preserving optimization pass, run by `--vm-opt`.
///
/// The reliability rule (docs/COMPAT.md): every optimization must preserve
/// semantics, and that is enforced by running opt vs unopt byte-identical
/// over the whole corpus. To make the proof easy, the pass is deliberately
/// small and conservative:
///
///   1. Constant folding — ONLY ops that cannot stress at runtime are
///      folded (checked arithmetic that succeeded, non-zero divisors,
///      scalar comparisons). An op whose runtime result would be a stress
///      is left alone so the VM produces the identical stress at the
///      identical point with the identical message. Float arithmetic is
///      never folded (platform conservatism; same binary evaluates it
///      identically at runtime).
///   2. Fold windows overlapping any control-transfer target are refused;
///      folded-out insns become `Nop`, so every pc stays valid and no
///      jump target ever needs rewriting for folding.
///   3. Jump threading — a jump whose target is itself an unconditional
///      `Jmp` is re-pointed at the final target (operand rewrite only,
///      fixpoint-bounded).
///
/// `Insn::Line` stamps are never folded away: the `cur_line` trajectory is
/// identical with and without the pass, so tracebacks are byte-identical.
pub fn optimize(code: &mut FuncCode) {
    // 1. every pc any control-transfer insn names (over-inclusion is safe:
    //    it only refuses folds).
    let mut targets: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (pc, insn) in code.insns.iter().enumerate() {
        let _ = pc;
        for t in insn_targets(insn) {
            targets.insert(t as usize);
        }
    }
    let frozen = |i: usize, targets: &std::collections::HashSet<usize>| targets.contains(&i);

    // 2. constant folding, adjacency windows.
    let mut i = 0;
    while i < code.insns.len() {
        // binary: [Const a, Const b, Bin op] -> [Nop, Nop, Const folded]
        if i + 2 < code.insns.len() {
            if let (Insn::Const(a), Insn::Const(b), Insn::Bin(op)) =
                (&code.insns[i], &code.insns[i + 1], &code.insns[i + 2])
            {
                let (a, b, op) = (*a, *b, *op);
                if !frozen(i, &targets) && !frozen(i + 1, &targets) && !frozen(i + 2, &targets) {
                    if let Some(v) =
                        fold_bin(op, &code.consts[a as usize], &code.consts[b as usize])
                    {
                        let k = code.consts.len() as u32;
                        code.consts.push(v);
                        code.insns[i] = Insn::Nop;
                        code.insns[i + 1] = Insn::Nop;
                        code.insns[i + 2] = Insn::Const(k);
                        i += 3;
                        continue;
                    }
                }
            }
        }
        // unary: [Const a, Un op] -> [Nop, Const folded]
        if i + 1 < code.insns.len() {
            if let (Insn::Const(a), Insn::Un(op)) = (&code.insns[i], &code.insns[i + 1]) {
                let (a, op) = (*a, *op);
                if !frozen(i, &targets) && !frozen(i + 1, &targets) {
                    if let Some(v) = fold_un(op, &code.consts[a as usize]) {
                        let k = code.consts.len() as u32;
                        code.consts.push(v);
                        code.insns[i] = Insn::Nop;
                        code.insns[i + 1] = Insn::Const(k);
                        i += 2;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }

    // 3. jump threading (operand rewrite only; fixpoint-bounded).
    for _round in 0..8 {
        let mut changed = false;
        for i in 0..code.insns.len() {
            let t = match &code.insns[i] {
                Insn::Jmp(t)
                | Insn::JmpIfFalse(t)
                | Insn::AndJmp(t)
                | Insn::OrJmp(t)
                | Insn::NullishJmp(t) => *t as usize,
                _ => continue,
            };
            // t == insns.len() is a legal "jump to the end" target (the VM
            // treats pc == len as a normal return); there is no insn to
            // thread through, so leave it alone.
            if t >= code.insns.len() {
                continue;
            }
            if let Insn::Jmp(u) = code.insns[t] {
                if u as usize != t {
                    match &mut code.insns[i] {
                        Insn::Jmp(x)
                        | Insn::JmpIfFalse(x)
                        | Insn::AndJmp(x)
                        | Insn::OrJmp(x)
                        | Insn::NullishJmp(x) => *x = u,
                        _ => unreachable!(),
                    }
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Every pc operand a control-transfer insn names (used to keep folds away
/// from jump targets). Over-inclusive by design: CatchTrim's operand is a
/// stack depth, not a pc, but naming it only refuses an occasional fold.
fn insn_targets(insn: &Insn) -> Vec<u32> {
    match insn {
        Insn::Jmp(t)
        | Insn::JmpIfFalse(t)
        | Insn::AndJmp(t)
        | Insn::OrJmp(t)
        | Insn::NullishJmp(t)
        | Insn::IterNext(t)
        | Insn::IfDegraded(_, t)
        | Insn::CatchTrim(t) => vec![*t],
        Insn::LoopEnter(a, b) => vec![*a, *b],
        Insn::CatchEnter(_, a, b) => vec![*a, *b],
        _ => Vec::new(),
    }
}

/// Compile-time fold of a binary op over constants. Mirrors the runtime
/// arms in interp.rs `apply_binop` for the SAFE subset only; `None` means
/// "not provably stress-free" and the VM will evaluate it at runtime
/// exactly as the unoptimized code would.
fn fold_bin(op: BinOp, l: &Value, r: &Value) -> Option<Value> {
    use BinOp::*;
    match (op, l, r) {
        // int arithmetic: fold only when the checked op succeeds; on
        // overflow the runtime produces the identical stress untouched.
        (Add, Value::Int(a), Value::Int(b)) => a.checked_add(*b).map(Value::Int),
        (Sub, Value::Int(a), Value::Int(b)) => a.checked_sub(*b).map(Value::Int),
        (Mul, Value::Int(a), Value::Int(b)) => a.checked_mul(*b).map(Value::Int),
        // floor division / modulo: mirrored arm-exact from interp.rs;
        // refused (None) on every stress-producing input.
        (FloorDiv, Value::Int(a), Value::Int(b)) => {
            if *b == 0 || (*a == i64::MIN && *b == -1) {
                return None;
            }
            let mut q = a / b;
            if (*a < 0) != (*b < 0) && q.checked_mul(*b)? != *a {
                q -= 1;
            }
            Some(Value::Int(q))
        }
        (Mod, Value::Int(a), Value::Int(b)) => {
            if *b == 0 || (*a == i64::MIN && *b == -1) {
                return None;
            }
            let mut q = a / b;
            if (*a < 0) != (*b < 0) && q.checked_mul(*b)? != *a {
                q -= 1;
            }
            let rb = q.checked_mul(*b)?;
            Some(Value::Int(a.checked_sub(rb)?))
        }
        // bitwise: total, stress-free, two's complement
        (BitAnd, Value::Int(a), Value::Int(b)) => Some(Value::Int(a & b)),
        (BitOr, Value::Int(a), Value::Int(b)) => Some(Value::Int(a | b)),
        (BitXor, Value::Int(a), Value::Int(b)) => Some(Value::Int(a ^ b)),
        // scalar equality (deep_eq on scalars is structural ==)
        (Eq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a == b)),
        (Eq, Value::Str(a), Value::Str(b)) => Some(Value::Bool(a == b)),
        (Eq, Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(a == b)),
        (Eq, Value::Null, Value::Null) => Some(Value::Bool(true)),
        (Neq, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a != b)),
        (Neq, Value::Str(a), Value::Str(b)) => Some(Value::Bool(a != b)),
        (Neq, Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(a != b)),
        (Neq, Value::Null, Value::Null) => Some(Value::Bool(false)),
        // ordering on ints and strings (interp `compare` uses a.cmp(b))
        (Lt, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a < b)),
        (Le, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a <= b)),
        (Gt, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a > b)),
        (Ge, Value::Int(a), Value::Int(b)) => Some(Value::Bool(a >= b)),
        (Lt, Value::Str(a), Value::Str(b)) => Some(Value::Bool(a < b)),
        (Le, Value::Str(a), Value::Str(b)) => Some(Value::Bool(a <= b)),
        (Gt, Value::Str(a), Value::Str(b)) => Some(Value::Bool(a > b)),
        (Ge, Value::Str(a), Value::Str(b)) => Some(Value::Bool(a >= b)),
        // everything else (float arithmetic, shifts, Pow, In, mixed
        // types, container equality) evaluates at runtime, unchanged.
        _ => None,
    }
}

/// Compile-time fold of a unary op over a constant (safe subset).
fn fold_un(op: UnOp, v: &Value) -> Option<Value> {
    match (op, v) {
        (UnOp::Neg, Value::Int(a)) => a.checked_neg().map(Value::Int),
        // float negation is an IEEE-exact sign flip on every platform
        (UnOp::Neg, Value::Float(f)) => Some(Value::Float(-*f)),
        (UnOp::Not, Value::Bool(b)) => Some(Value::Bool(!*b)),
        (UnOp::BitNot, Value::Int(a)) => Some(Value::Int(!*a)),
        _ => None,
    }
}

/// Compile a whole program into its unit of gene function codes.
pub fn compile_program(prog: &Program) -> Unit {
    let mut c = Compiler::new();
    for s in &prog.stmts {
        c.collect_stmt(s);
    }
    // proof frames and named frames can carry gene defs too
    for frame in prog
        .proofs
        .iter()
        .chain(prog.named_frames.iter().map(|(_, b)| b))
    {
        for s in frame {
            c.collect_stmt(s);
        }
    }
    Unit {
        funcs: c.funcs,
        delegated: c.delegated,
    }
}
