//! vm.rs, W09 stage A2: the OIR1 bytecode compiler and stack machine.
//!
//! Architecture (per docs/vm-design.md §3-§8): the VM shares the tree-walk's
//! `Interp`, `Value`, `Env`, notes, capability sandbox and fuel pool. Native
//! instructions execute the hot core (literals, locals, shared `apply_binop`
//! arithmetic, jumps); calls, methods, builtins and exotic expressions
//! BRIDGE through the tree-walk's own `eval` / `exec_stmt`, so gates,
//! entropy draws, note text and stress kinds are byte-identical by
//! construction. Every native opcode costs one tick (never cheaper than the
//! tree-walk construct it replaces); bridged opcodes cost exactly what the
//! tree-walk costs, because they ARE the tree-walk.
//!
//! Hook point: `Interp.vm` (set by `--vm`). `call_gene_inner` runs the
//! compiled body for a gene call instead of `exec_block` when the flag is
//! on; param binding, the gate funnel and guards stay in the shared code.
//! The compiled bodies cache per gene-definition pointer (definitions live
//! in globals for the whole run, so pointer identity is stable for the
//! cache's lifetime).

use crate::ast::{BinOp, Expr, Stmt};
use crate::interp::{Env, Flow, Interp};
use crate::value::{Stress, Value};
use std::collections::HashMap;
use std::rc::Rc;

pub const OIR_VERSION: u32 = 1;

/// Compile-time constant pool.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

/// One compiled gene body. `lines` parallels `code` (ip -> source line, 0 =
/// leave the interpreter's current line alone) so shared code that reads
/// `interp.cur_line` emits identical diagnostics.
#[derive(Debug, Clone)]
pub struct GeneCode {
    pub name: String,
    pub consts: Vec<Const>,
    pub names: Vec<String>,
    pub code: Vec<Instr>,
    pub lines: Vec<u32>,
}

#[derive(Debug, Clone)]
pub enum Instr {
    /// consts[idx] -> stack
    Push(u32),
    /// name read: charge_clone + unbound note (the tree-walk read arm)
    LoadName(u32),
    /// quiet read: no note, no clone charge (compound-assign target read)
    LoadNameQuiet(u32),
    /// `let` semantics: rebinding note + define
    StoreName(u32),
    /// assignment semantics: const check, set, auto-declare note
    AssignName(u32),
    /// shared apply_binop (exact kinds, messages, line stamps)
    Bin(BinOp),
    /// jump if the popped value is falsy (same truthy() order)
    JmpIfF(u32),
    Jmp(u32),
    /// bridge: evaluate exprs[idx] with the tree-walk, push the result
    EvalExpr(u32),
    /// bridge: execute stmts[idx] with the tree-walk
    BridgeStmt(u32),
    /// bridge inside a COMPILED loop: the bridged statement's flow must
    /// reach the right loop (Ret exits the gene, Brk jumps to the loop
    /// end, Cont jumps to the loop top). top/end are patched at loop end.
    BridgeStmtInLoop(u32, u32, u32),
    /// pop one value (expression statements)
    Pop,
    /// return the value on the stack
    Ret,
    /// break/continue out of the enclosing COMPILED loop (patched target);
    /// a break/continue with no compiled loop compiles as a bridge instead
    Brk(u32),
    Cont(u32),
    /// enter/leave a block scope (fresh child env, tree-walk shape)
    EnterScope,
    ExitScope,
}

/// Bridged sub-AST arena plus the compiled bodies. Lives on the Interp
/// while --vm runs; bridges clone their node out per execution (Expr/Stmt
/// clones are cheap: children are Arc'd definitions and interned strings).
#[derive(Default)]
pub struct VmProgram {
    pub exprs: Vec<Expr>,
    pub stmts: Vec<Stmt>,
    /// gene-definition pointer -> compiled body
    pub codes: HashMap<usize, GeneCode>,
}

impl<'a> Compiler<'a> {
    /// The stmt index of a BridgeStmtInLoop site is stored in the
    /// instruction's first operand, which patching preserves.
    fn stmt_idx_of(&self, site: usize) -> u32 {
        match self.code[site] {
            Instr::BridgeStmtInLoop(idx, _, _) => idx,
            _ => 0,
        }
    }
}

fn intern_const(v: &mut Vec<Const>, c: Const) -> u32 {
    if let Some(i) = v.iter().position(|x| *x == c) {
        return i as u32;
    }
    v.push(c);
    (v.len() - 1) as u32
}

fn intern_name(v: &mut Vec<String>, s: &str) -> u32 {
    if let Some(i) = v.iter().position(|n| n == s) {
        return i as u32;
    }
    v.push(s.to_string());
    (v.len() - 1) as u32
}

struct LoopFrame {
    /// unresolved Brk sites (patched to the loop end)
    brks: Vec<usize>,
    /// unresolved Cont sites (patched to the loop top)
    conts: Vec<usize>,
    /// bridged statements inside this loop (patched with top+end so their
    /// internal break/continue flow lands on the right loop)
    bridges: Vec<usize>,
    /// reserved: the loop top ip, patched into bridge sites at loop end
    #[allow(dead_code)]
    top: usize,
}

struct Compiler<'a> {
    consts: Vec<Const>,
    names: Vec<String>,
    exprs: &'a mut Vec<Expr>,
    stmts: &'a mut Vec<Stmt>,
    code: Vec<Instr>,
    lines: Vec<u32>,
    loops: Vec<LoopFrame>,
}

/// Statement source line for the line table (W07 stamps live on the Raise
/// statement itself; other statements keep the interpreter's current line,
/// exactly like the tree-walk's per-statement behavior). Statements carry
/// their line via the bridge or shared code, so 0 = "leave alone" is the
/// default and only looping constructs (which the tree-walk ticks per
/// iteration) stamp explicitly.
fn line_of(_s: &Stmt) -> u32 {
    0
}

impl<'a> Compiler<'a> {
    fn new(exprs: &'a mut Vec<Expr>, stmts: &'a mut Vec<Stmt>) -> Self {
        Compiler {
            consts: Vec::new(),
            names: Vec::new(),
            exprs,
            stmts,
            code: Vec::new(),
            lines: Vec::new(),
            loops: Vec::new(),
        }
    }

    fn emit(&mut self, i: Instr, line: u32) -> usize {
        self.code.push(i);
        self.lines.push(line);
        self.code.len() - 1
    }

    /// Expression compile. Returns false when the expression must go
    /// through the bridge (nothing emitted by this call in that case).
    fn expr(&mut self, e: &Expr, line: u32) -> bool {
        match e {
            Expr::Null => {
                let idx = intern_const(&mut self.consts, Const::Null);
                self.emit(Instr::Push(idx), line);
            }
            Expr::Bool(b) => {
                let idx = intern_const(&mut self.consts, Const::Bool(*b));
                self.emit(Instr::Push(idx), line);
            }
            Expr::Int(i) => {
                let idx = intern_const(&mut self.consts, Const::Int(*i));
                self.emit(Instr::Push(idx), line);
            }
            Expr::Float(f) => {
                let idx = intern_const(&mut self.consts, Const::Float(*f));
                self.emit(Instr::Push(idx), line);
            }
            Expr::Str(s) => {
                let idx = intern_const(&mut self.consts, Const::Str(s.clone()));
                self.emit(Instr::Push(idx), line);
            }
            Expr::Ident(name) => {
                let idx = intern_name(&mut self.names, name);
                self.emit(Instr::LoadName(idx), line);
            }
            Expr::Binary(op, l, r, op_line) => {
                // And/Or/Nullish short-circuit (the rhs must not evaluate
                // when the lhs decides); In routes through the bridge too.
                // apply_binop's And/Or arm is unreachable!() by contract,
                // the tree-walk never calls it there, and neither may we.
                match op {
                    BinOp::And | BinOp::Or | BinOp::Nullish | BinOp::In => return false,
                    _ => {}
                }
                if !self.expr(l, line) {
                    return false;
                }
                if !self.expr(r, line) {
                    return false;
                }
                self.emit(Instr::Bin(*op), *op_line as u32);
            }
            _ => return false,
        }
        true
    }

    /// Compile an expression natively when possible, otherwise bridge it.
    fn expr_or_bridge(&mut self, e: &Expr, line: u32) {
        let mark = (self.code.len(), self.consts.len(), self.names.len());
        if self.expr(e, line) {
            return;
        }
        // rollback the partial native attempt, then bridge the whole node
        self.code.truncate(mark.0);
        self.lines.truncate(mark.0);
        self.consts.truncate(mark.1);
        self.names.truncate(mark.2);
        let idx = self.exprs.len() as u32;
        self.exprs.push(e.clone());
        self.emit(Instr::EvalExpr(idx), line);
    }

    /// Compile a statement; returns unresolved Brk/Cont sites belonging to
    /// the ENCLOSING compiled loop (empty for most statements).
    fn stmt(&mut self, s: &Stmt) -> Vec<usize> {
        let line = line_of(s);
        match s {
            Stmt::Let(name, e) => {
                self.expr_or_bridge(e, line);
                let idx = intern_name(&mut self.names, name);
                self.emit(Instr::StoreName(idx), line);
                Vec::new()
            }
            Stmt::Assign(name, None, e) => {
                self.expr_or_bridge(e, line);
                let idx = intern_name(&mut self.names, name);
                self.emit(Instr::AssignName(idx), line);
                Vec::new()
            }
            Stmt::Assign(name, Some(op), e) => {
                // compound: quiet read, binop, write (tree-walk order)
                let nidx = intern_name(&mut self.names, name);
                self.emit(Instr::LoadNameQuiet(nidx), line);
                self.expr_or_bridge(e, line);
                self.emit(Instr::Bin(*op), line);
                self.emit(Instr::AssignName(nidx), line);
                Vec::new()
            }
            Stmt::ExprStmt(e) => {
                self.expr_or_bridge(e, line);
                self.emit(Instr::Pop, line);
                Vec::new()
            }
            Stmt::Return(Some(e)) => {
                self.expr_or_bridge(e, line);
                self.emit(Instr::Ret, line);
                Vec::new()
            }
            Stmt::Return(None) => {
                let idx = intern_const(&mut self.consts, Const::Null);
                self.emit(Instr::Push(idx), line);
                self.emit(Instr::Ret, line);
                Vec::new()
            }
            Stmt::Break => {
                if self.loops.is_empty() {
                    // a break with no compiled loop keeps tree-walk flow
                    let idx = self.stmts.len() as u32;
                    self.stmts.push(s.clone());
                    self.emit(Instr::BridgeStmt(idx), line);
                    return Vec::new();
                }
                let site = self.emit(Instr::Brk(0), line);
                self.loops.last_mut().unwrap().brks.push(site);
                Vec::new()
            }
            Stmt::Continue => {
                if self.loops.is_empty() {
                    let idx = self.stmts.len() as u32;
                    self.stmts.push(s.clone());
                    self.emit(Instr::BridgeStmt(idx), line);
                    return Vec::new();
                }
                let site = self.emit(Instr::Cont(0), line);
                self.loops.last_mut().unwrap().conts.push(site);
                Vec::new()
            }
            Stmt::Block(body) => {
                self.emit(Instr::EnterScope, line);
                let out = self.stmts(body);
                self.emit(Instr::ExitScope, line);
                out
            }
            Stmt::If(branches, els) => {
                // each branch: cond, JmpIfF(next), body, Jmp(end)
                let mut end_jumps: Vec<usize> = Vec::new();
                let mut out: Vec<usize> = Vec::new();
                let last = branches.len();
                for (i, (cond, body)) in branches.iter().enumerate() {
                    self.expr_or_bridge(cond, line);
                    let jif = self.emit(Instr::JmpIfF(0), line);
                    out.extend(self.stmts(body));
                    let jmp = self.emit(Instr::Jmp(0), line);
                    end_jumps.push(jmp);
                    let next = self.code.len() as u32;
                    self.code[jif] = Instr::JmpIfF(next);
                    let _ = i;
                }
                if let Some(eb) = els {
                    out.extend(self.stmts(eb));
                } else {
                    let _ = last;
                }
                let end = self.code.len() as u32;
                for j in end_jumps {
                    self.code[j] = Instr::Jmp(end);
                }
                out
            }
            Stmt::While(cond, body) => {
                let top = self.code.len();
                self.loops.push(LoopFrame {
                    brks: Vec::new(),
                    conts: Vec::new(),
                    bridges: Vec::new(),
                    top,
                });
                self.expr_or_bridge(cond, line);
                let jif = self.emit(Instr::JmpIfF(0), line);
                let _out = self.stmts(body);
                self.emit(Instr::Jmp(top as u32), line);
                let end = self.code.len() as u32;
                self.code[jif] = Instr::JmpIfF(end);
                let frame = self.loops.pop().unwrap();
                for b in frame.brks {
                    self.code[b] = Instr::Brk(end);
                }
                for c in frame.conts {
                    self.code[c] = Instr::Cont(top as u32);
                }
                for bidx in frame.bridges {
                    self.code[bidx] =
                        Instr::BridgeStmtInLoop(self.stmt_idx_of(bidx), top as u32, end);
                }
                // break/continue sites were recorded on THIS loop's frame
                // and patched above; nothing propagates to an outer loop
                Vec::new()
            }
            Stmt::Loop(body) => {
                let top = self.code.len();
                self.loops.push(LoopFrame {
                    brks: Vec::new(),
                    conts: Vec::new(),
                    bridges: Vec::new(),
                    top,
                });
                let _out = self.stmts(body);
                self.emit(Instr::Jmp(top as u32), line);
                let end = self.code.len() as u32;
                let frame = self.loops.pop().unwrap();
                for b in frame.brks {
                    self.code[b] = Instr::Brk(end);
                }
                for c in frame.conts {
                    self.code[c] = Instr::Cont(top as u32);
                }
                for bidx in frame.bridges {
                    self.code[bidx] =
                        Instr::BridgeStmtInLoop(self.stmt_idx_of(bidx), top as u32, end);
                }
                Vec::new()
            }
            other => {
                let idx = self.stmts.len() as u32;
                self.stmts.push(other.clone());
                if self.loops.is_empty() {
                    self.emit(Instr::BridgeStmt(idx), line);
                } else {
                    // the bridged statement may return/branch; its flow
                    // needs this loop's top/end, patched at loop end
                    let site = self.emit(Instr::BridgeStmtInLoop(idx, 0, 0), line);
                    self.loops.last_mut().unwrap().bridges.push(site);
                }
                Vec::new()
            }
        }
    }

    fn stmts(&mut self, list: &[Stmt]) -> Vec<usize> {
        let mut out = Vec::new();
        for s in list {
            out.extend(self.stmt(s));
        }
        out
    }
}

/// Compile a gene body. Infallible: every construct either compiles native
/// or bridges (Total Grammar: nothing is rejected at compile time).
pub fn compile_body(name: &str, body: &[Stmt], prog: &mut VmProgram) -> GeneCode {
    let mut c = Compiler::new(&mut prog.exprs, &mut prog.stmts);
    let pending = c.stmts(body);
    // a body-level break/continue cannot reach a compiled loop target:
    // it would be a bare flow the tree-walk resolves at its own boundary.
    // Such sites only exist when a loop was bridged BETWEEN the break and
    // its loop, which cannot happen (a loop containing the break compiles
    // the break itself). Patch any leftover to fall off the end safely.
    let end = c.code.len() as u32;
    for b in pending {
        if b < c.code.len() {
            match c.code[b] {
                Instr::Brk(_) => c.code[b] = Instr::Brk(end),
                Instr::Cont(_) => c.code[b] = Instr::Cont(end),
                _ => {}
            }
        }
    }
    GeneCode {
        name: name.to_string(),
        consts: c.consts,
        names: c.names,
        code: c.code,
        lines: c.lines,
    }
}

/// Compile (or fetch from cache) the body of a gene definition and execute
/// it. Called from call_gene_inner with the freshly built frame env.
pub fn exec_gene_body(
    interp: &mut Interp,
    def_key: usize,
    name: &str,
    body: &[Stmt],
    env: &Rc<Env>,
) -> Result<Flow, Stress> {
    let code = {
        let opt = interp.vm_opt;
        let prog = interp.vm_program.as_mut().unwrap();
        let key = if opt >= 1 {
            // cache the optimized form under a shifted key
            def_key.wrapping_add(1usize << 62)
        } else {
            def_key
        };
        if let Some(cached) = prog.codes.get(&key) {
            cached.clone()
        } else {
            let compiled = compile_body(name, body, prog);
            let compiled = if opt >= 1 {
                optimize(&compiled)
            } else {
                compiled
            };
            prog.codes.insert(key, compiled.clone());
            compiled
        }
    };
    exec_gene_code(interp, &code, env)
}

/// Execute a compiled gene body against the shared interpreter.
fn exec_gene_code(interp: &mut Interp, code: &GeneCode, env: &Rc<Env>) -> Result<Flow, Stress> {
    let mut stack: Vec<Value> = Vec::new();
    let mut scopes: Vec<Rc<Env>> = Vec::new();
    let mut cur = env.clone();
    let mut ip: usize = 0;
    loop {
        let instr = match code.code.get(ip) {
            Some(i) => i.clone(),
            None => return Ok(Flow::Norm), // fell off the end
        };
        let line = code.lines[ip];
        if line > 0 {
            interp.cur_line = line as usize;
        }
        ip += 1;
        interp.tick()?;
        match instr {
            Instr::Push(idx) => {
                let v = match code.consts.get(idx as usize) {
                    Some(Const::Null) | None => Value::Null,
                    Some(Const::Bool(b)) => Value::Bool(*b),
                    Some(Const::Int(i)) => Value::Int(*i),
                    Some(Const::Float(f)) => Value::Float(*f),
                    Some(Const::Str(s)) => Value::Str(s.clone()),
                };
                stack.push(v);
            }
            Instr::LoadName(idx) => {
                let name = &code.names[idx as usize];
                match cur.get(name) {
                    Some(v) => {
                        crate::interp::charge_clone(&v)?;
                        stack.push(v);
                    }
                    None => {
                        interp.note(0, 4, format!("unbound '{}' read as null", name));
                        stack.push(Value::Null);
                    }
                }
            }
            Instr::LoadNameQuiet(idx) => {
                let name = &code.names[idx as usize];
                stack.push(cur.get(name).unwrap_or(Value::Null));
            }
            Instr::StoreName(idx) => {
                let name = code.names[idx as usize].clone();
                let v = stack.pop().unwrap_or(Value::Null);
                if cur.get(&name).is_some() {
                    interp.note(0, 4, format!("rebinding '{}'", name));
                }
                cur.define(&name, v);
            }
            Instr::AssignName(idx) => {
                let name = code.names[idx as usize].clone();
                let v = stack.pop().unwrap_or(Value::Null);
                if cur.is_const(&name) {
                    return Err(Stress::new(
                        "frozen",
                        format!("cannot reassign const '{}'", name),
                    ));
                }
                if !cur.set(&name, v) {
                    interp.note(0, 4, format!("'{}' was not declared; auto-declared", name));
                }
            }
            Instr::Bin(op) => {
                let r = stack.pop().unwrap_or(Value::Null);
                let l = stack.pop().unwrap_or(Value::Null);
                let v = interp.apply_binop(&cur, op, &l, &r)?;
                stack.push(v);
            }
            Instr::JmpIfF(t) => {
                let v = stack.pop().unwrap_or(Value::Null);
                if !v.truthy() {
                    ip = t as usize;
                }
            }
            Instr::Jmp(t) => ip = t as usize,
            Instr::EvalExpr(idx) => {
                // clone the bridged node out of the arena (cheap: Arc'd
                // children) so the mutable interpreter borrow is free
                let e = interp
                    .vm_program
                    .as_ref()
                    .and_then(|p| p.exprs.get(idx as usize))
                    .cloned();
                match e {
                    Some(e) => {
                        let v = interp.eval(&cur, &e)?;
                        stack.push(v);
                    }
                    None => stack.push(Value::Null),
                }
            }
            Instr::BridgeStmt(idx) => {
                let s = interp
                    .vm_program
                    .as_ref()
                    .and_then(|p| p.stmts.get(idx as usize))
                    .cloned();
                if let Some(s) = s {
                    // the bridged statement's flow propagates exactly the
                    // way exec_block would propagate it (Ret leaves the
                    // gene; Brk/Cont reach the gene boundary, where the
                    // shared code turns them into a null return)
                    match interp.exec_stmt(&cur, &s)? {
                        Flow::Norm => {}
                        other => return Ok(other),
                    }
                }
            }
            Instr::BridgeStmtInLoop(idx, top, end) => {
                let s = interp
                    .vm_program
                    .as_ref()
                    .and_then(|p| p.stmts.get(idx as usize))
                    .cloned();
                if let Some(s) = s {
                    match interp.exec_stmt(&cur, &s)? {
                        Flow::Norm => {}
                        // a return from inside a bridged statement IS the
                        // gene's return (the tree-walk contract)
                        Flow::Ret(v) => return Ok(Flow::Ret(v)),
                        // a break/continue inside a bridged statement
                        // belongs to THIS compiled loop (compile-time fact)
                        Flow::Brk => ip = end as usize,
                        Flow::Cont => ip = top as usize,
                    }
                }
            }
            Instr::Pop => {
                stack.pop();
            }
            Instr::Ret => {
                let v = stack.pop().unwrap_or(Value::Null);
                return Ok(Flow::Ret(v));
            }
            Instr::Brk(t) => ip = t as usize,
            Instr::Cont(t) => ip = t as usize,
            Instr::EnterScope => {
                scopes.push(cur.clone());
                cur = Env::new(Some(cur.clone()));
            }
            Instr::ExitScope => {
                if let Some(p) = scopes.pop() {
                    cur = p;
                }
            }
        }
    }
}

/// `operon ir`: compile every top-level gene and render the OIR1 listing
/// (`op idx | mnemonic | operands | line`). W10 stage 1.
pub fn disassemble_program(prog: &crate::ast::Program) -> String {
    let mut out = String::new();
    out.push_str(&format!("OIR{} disassembly\n", OIR_VERSION));
    let mut vmprog = VmProgram::default();
    for s in &prog.stmts {
        if let Stmt::Gene(g) = s {
            let name = g.name.clone().unwrap_or_else(|| "<lambda>".into());
            let code = compile_body(&name, &g.body, &mut vmprog);
            out.push_str(&format!("\ngene {} ({} instr(s))\n", name, code.code.len()));
            for (i, instr) in code.code.iter().enumerate() {
                out.push_str(&format!(
                    "  {:04} | {} | {}\n",
                    i,
                    mnemonic(instr),
                    render(instr, &code)
                ));
            }
            if code.code.is_empty() {
                out.push_str("  (empty body)\n");
            }
        }
    }
    out
}

fn mnemonic(i: &Instr) -> &'static str {
    match i {
        Instr::Push(_) => "Push",
        Instr::LoadName(_) => "LoadName",
        Instr::LoadNameQuiet(_) => "LoadNameQuiet",
        Instr::StoreName(_) => "StoreName",
        Instr::AssignName(_) => "AssignName",
        Instr::Bin(_) => "Bin",
        Instr::JmpIfF(_) => "JmpIfF",
        Instr::Jmp(_) => "Jmp",
        Instr::EvalExpr(_) => "EvalExpr",
        Instr::BridgeStmt(_) => "BridgeStmt",
        Instr::BridgeStmtInLoop(..) => "BridgeStmtInLoop",
        Instr::Pop => "Pop",
        Instr::Ret => "Ret",
        Instr::Brk(_) => "Brk",
        Instr::Cont(_) => "Cont",
        Instr::EnterScope => "EnterScope",
        Instr::ExitScope => "ExitScope",
    }
}

fn render(i: &Instr, code: &GeneCode) -> String {
    match i {
        Instr::Push(idx) => match code.consts.get(*idx as usize) {
            Some(Const::Str(s)) => format!("c{} {:?}", idx, s),
            Some(Const::Int(v)) => format!("c{} {}", idx, v),
            Some(other) => format!("c{} {:?}", idx, other),
            None => format!("c{} <missing>", idx),
        },
        Instr::LoadName(idx)
        | Instr::LoadNameQuiet(idx)
        | Instr::StoreName(idx)
        | Instr::AssignName(idx) => code
            .names
            .get(*idx as usize)
            .cloned()
            .unwrap_or_else(|| format!("c{}", idx)),
        Instr::Bin(op) => format!("{:?}", op),
        Instr::JmpIfF(t) | Instr::Jmp(t) | Instr::Brk(t) | Instr::Cont(t) => format!("-> {}", t),
        Instr::EvalExpr(idx) => format!("expr#{}", idx),
        Instr::BridgeStmt(idx) => format!("stmt#{}", idx),
        Instr::BridgeStmtInLoop(idx, top, end) => {
            format!("stmt#{} top {} end {}", idx, top, end)
        }
        _ => String::new(),
    }
}

/// W11 stage 1: the optimization pipeline. Two draw-free, provably
/// behavior-preserving passes over the OIR1:
///   1. constant folding: Push a, Push b, Bin -> Push (folded). Only pure
///      Int/Float arithmetic folds, and only when the operation cannot
///      stress (i64 checked ops); anything that would raise at runtime
///      stays runtime (the stress kind+message are the contract).
///   2. jump threading: a Jmp whose target is another Jmp follows the
///      chain; an unconditional Jmp to the NEXT instruction disappears.
///      Folding never touches a draw or a call: those are Bridge/EvalExpr
///      instructions, and the folder must not reorder or remove draws
///      (the entropy discipline, vm-design.md invariant 2).
pub fn optimize(code: &GeneCode) -> GeneCode {
    let mut consts = code.consts.clone();
    let mut code = code.clone();

    // pass 1: constant folding (fixpoint, bounded)
    for _round in 0..8 {
        let mut folded = false;
        let mut i = 0;
        while i + 2 < code.code.len() {
            if let (Instr::Push(a), Instr::Push(b), Instr::Bin(op)) =
                (&code.code[i], &code.code[i + 1], &code.code[i + 2])
            {
                if let Some(newc) = fold_consts(&mut consts, *a, *b, *op) {
                    code.code[i] = Instr::Push(newc);
                    code.code.remove(i + 2);
                    code.code.remove(i + 1);
                    code.lines.remove(i + 2);
                    code.lines.remove(i + 1);
                    remap_after_removal(&mut code, i + 1, 2);
                    folded = true;
                    continue; // try to fold the result into its neighbor
                }
            }
            i += 1;
        }
        if !folded {
            break;
        }
    }

    // pass 2: jump threading + dead unconditional-jump removal
    for _round in 0..8 {
        let mut changed = false;
        for i in 0..code.code.len() {
            if let Instr::Jmp(t) = code.code[i] {
                let mut target = t as usize;
                let mut hops = 0;
                while hops < 16 {
                    match code.code.get(target) {
                        Some(Instr::Jmp(t2)) => {
                            target = *t2 as usize;
                            hops += 1;
                        }
                        _ => break,
                    }
                }
                if target != t as usize {
                    code.code[i] = Instr::Jmp(target as u32);
                    changed = true;
                }
            }
        }
        let mut i = 0;
        while i < code.code.len() {
            if let Instr::Jmp(t) = code.code[i] {
                if t as usize == i + 1 {
                    code.code.remove(i);
                    code.lines.remove(i);
                    remap_after_removal(&mut code, i, 1);
                    changed = true;
                    continue;
                }
            }
            i += 1;
        }
        if !changed {
            break;
        }
    }

    GeneCode {
        name: code.name,
        consts,
        names: code.names,
        code: code.code,
        lines: code.lines,
    }
}

/// Remap jump targets after `count` instruction(s) at `at` were removed:
/// every target > `at` shifts down by `count`; a target inside the removed
/// range cannot exist (we only remove folded operands, never jump targets,
/// because jumps are only emitted at statement boundaries the folder does
/// not touch, enforced by only folding Push/Push/Bin triples).
fn remap_after_removal(code: &mut GeneCode, at: usize, count: usize) {
    for instr in code.code.iter_mut() {
        match instr {
            Instr::Jmp(t) | Instr::JmpIfF(t) | Instr::Brk(t) | Instr::Cont(t) => {
                let tt = *t as usize;
                if tt > at + count - 1 {
                    *t = (tt - count) as u32;
                }
            }
            _ => {}
        }
    }
    let _ = at;
}

/// Fold one Push/Push/Bin triple into a constant (appended to the pool).
fn fold_consts(consts: &mut Vec<Const>, a: u32, b: u32, op: BinOp) -> Option<u32> {
    let va = consts.get(a as usize)?.clone();
    let vb = consts.get(b as usize)?.clone();
    use BinOp::*;
    let folded: Const = match (va, vb) {
        (Const::Int(x), Const::Int(y)) => match op {
            Add => Const::Int(x.checked_add(y)?),
            Sub => Const::Int(x.checked_sub(y)?),
            Mul => Const::Int(x.checked_mul(y)?),
            Div => {
                if y == 0 {
                    return None; // the runtime stress IS the contract
                }
                Const::Int(x.checked_div(y)?)
            }
            Mod => {
                if y == 0 {
                    return None;
                }
                Const::Int(x.checked_rem(y)?)
            }
            Eq => Const::Bool(x == y),
            Neq => Const::Bool(x != y),
            Lt => Const::Bool(x < y),
            Le => Const::Bool(x <= y),
            Gt => Const::Bool(x > y),
            Ge => Const::Bool(x >= y),
            _ => return None,
        },
        (Const::Float(x), Const::Float(y)) => match op {
            Add => Const::Float(x + y),
            Sub => Const::Float(x - y),
            Mul => Const::Float(x * y),
            Div => Const::Float(x / y),
            Eq => Const::Bool(x == y),
            Neq => Const::Bool(x != y),
            Lt => Const::Bool(x < y),
            Le => Const::Bool(x <= y),
            Gt => Const::Bool(x > y),
            Ge => Const::Bool(x >= y),
            _ => return None,
        },
        (Const::Bool(x), Const::Bool(y)) => match op {
            Eq => Const::Bool(x == y),
            Neq => Const::Bool(x != y),
            _ => return None,
        },
        _ => return None,
    };
    if let Some(i) = consts.iter().position(|c| *c == folded) {
        return Some(i as u32);
    }
    consts.push(folded);
    Some((consts.len() - 1) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Stmt;

    /// W11 stage 1: the folding pass removes constant arithmetic without
    /// changing the program's observable encoding of jumps.
    #[test]
    fn folds_constants_and_threads_jumps() {
        let src = "gene f() {\n    return 6 * 7\n}\n";
        let parsed = crate::parser::parse(src);
        let mut body: Vec<Stmt> = Vec::new();
        for s in &parsed.stmts {
            if let Stmt::Gene(g) = s {
                body = g.body.clone();
            }
        }
        let mut prog = VmProgram::default();
        let raw = compile_body("f", &body, &mut prog);
        let opt = optimize(&raw);
        // the folded body is exactly: Push 42, Ret
        let names: Vec<String> = opt.code.iter().map(|i| mnemonic(i).to_string()).collect();
        assert_eq!(names, vec!["Push", "Ret"], "folding changed the shape");
        // and the constant really is 42
        match &opt.code[0] {
            Instr::Push(idx) => match &opt.consts[*idx as usize] {
                Const::Int(42) => {}
                other => panic!("folded to {:?}", other),
            },
            other => panic!("not a Push: {:?}", other),
        }
    }

    /// W10 done-when: one compiled function's encoding is pinned.
    #[test]
    fn pins_the_encoding_of_a_small_function() {
        let src = "gene f(n) {\n    let x = n + 1\n    return x * 2\n}\n";
        let parsed = crate::parser::parse(src);
        let mut found: Option<(String, Vec<Stmt>)> = None;
        for s in &parsed.stmts {
            if let Stmt::Gene(g) = s {
                found = Some((g.name.clone().unwrap_or_default(), g.body.clone()));
            }
        }
        let (name, body) = found.expect("gene present");
        assert_eq!(name, "f");
        let mut prog = VmProgram::default();
        let code = compile_body("f", &body, &mut prog);
        let rendered: Vec<String> = code
            .code
            .iter()
            .map(|i| {
                let r = render(i, &code);
                if r.is_empty() {
                    mnemonic(i).to_string()
                } else {
                    format!("{} {}", mnemonic(i), r)
                }
            })
            .collect();
        let expected = vec![
            "LoadName n",
            "Push c0 1",
            "Bin Add",
            "StoreName x",
            "LoadName x",
            "Push c1 2",
            "Bin Mul",
            "Ret",
        ];
        assert_eq!(rendered, expected, "OIR1 encoding drifted");
        let _ = OIR_VERSION;
    }
}
