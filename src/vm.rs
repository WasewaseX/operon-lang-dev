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
    /// W11 superinstruction: pop lhs, push consts[idx] as rhs, apply the
    /// SAME apply_binop. Fuses the measured Push+Bin pair (fib25 histogram:
    /// 96% of Pushes feed a Bin); identical evaluation order and stress
    /// surface, one tick instead of two.
    BinImm(BinOp, u32),
    /// W11 superinstruction: read name (the EXACT LoadName arm: clone-charge
    /// plus the unbound note), then apply_binop against consts[idx]. Fuses
    /// measured LoadName+Push+Bin triple (`n < 2`, `n - 1`, `a = a + 1`).
    LoadBinImm(u32, BinOp, u32),
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
    /// The 4th operand is the scope unwind count for that site: the scopes
    /// opened since the loop top, restored before the jump lands.
    BridgeStmtInLoop(u32, u32, u32, u32),
    /// pop one value (expression statements)
    Pop,
    /// return the value on the stack
    Ret,
    /// W11 superinstruction: read name (exact LoadName arm) and return it.
    /// Fuses the measured LoadName+Ret pair (every `return <name>` tail).
    RetName(u32),
    /// break/continue out of the enclosing COMPILED loop (patched target);
    /// a break/continue with no compiled loop compiles as a bridge instead
    Brk(u32),
    Cont(u32),
    /// enter/leave a block scope (fresh child env, tree-walk shape)
    EnterScope,
    ExitScope,
    /// W09 native calls: pop argc values, run the SHARED named-call tail
    /// (RISC gate + call_named funnel, src/interp.rs named_call_tail), push
    /// the result. Only Expr::Call over a bare identifier compiles to this;
    /// method calls, gene-value calls and every exotic callee stay bridged.
    CallNamed(u32, u32),
    /// compat-matrix fix (rt_p22a, 2026-09-30): a line stamp that executes
    /// as nothing. The tree-walk stamps Expr::Call's line at ARM ENTRY,
    /// before any argument evaluates, and never re-stamps after; the per-
    /// insn line table can only stamp when an insn executes, so a stamp
    /// BEFORE the arg insns needs a real (no-effect) insn. Costs one tick
    /// ("never cheaper than the tree-walk" holds); invisible to output.
    Nop,
}

/// Identity hasher for pointer-keyed cache maps: a pointer is already a
/// well-distributed u64, SipHash's mixing (the std default) is pure per-call
/// overhead on the fib25 path (~243k lookups). Parity-neutral: the cache is
/// invisible to outputs, notes, and fuel.
#[derive(Default)]
pub struct IdentityHash(u64);

impl std::hash::Hasher for IdentityHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            self.0 ^= (*b as u64) << ((i % 8) * 8);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
    fn write_usize(&mut self, n: usize) {
        self.0 = n as u64;
    }
}

/// Bridged sub-AST arena plus the compiled bodies. Lives on the Interp
/// while --vm runs; bridges clone their node out per execution (Expr/Stmt
/// clones are cheap: children are Arc'd definitions and interned strings).
#[derive(Default)]
pub struct VmProgram {
    pub exprs: Vec<Expr>,
    pub stmts: Vec<Stmt>,
    /// gene-definition pointer -> compiled body. Rc so a call hands the
    /// body to the machine with a refcount bump, not a deep clone: fib25's
    /// ~243k calls were cloning the whole code vec per call, which made the
    /// machine SLOWER than the tree-walk (the bug the fib25 gate exists to
    /// catch; measured 0.54x before, same outputs after).
    pub codes: HashMap<usize, std::rc::Rc<GeneCode>, std::hash::BuildHasherDefault<IdentityHash>>,
}

impl<'a> Compiler<'a> {
    /// The stmt index of a BridgeStmtInLoop site is stored in the
    /// instruction's first operand, which patching preserves.
    fn stmt_idx_of(&self, site: usize) -> u32 {
        match self.code[site] {
            Instr::BridgeStmtInLoop(idx, _, _, _) => idx,
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

/// W11: the literal shapes that may ride inside a superinstruction. Scalars
/// only, no operators: a Unary minus must keep its own runtime checked path,
/// and anything with a sub-expression must stay a real instruction sequence.
fn literal_const(e: &Expr) -> Option<Const> {
    match e {
        Expr::Null => Some(Const::Null),
        Expr::Bool(b) => Some(Const::Bool(*b)),
        Expr::Int(i) => Some(Const::Int(*i)),
        Expr::Float(f) => Some(Const::Float(*f)),
        Expr::Str(s) => Some(Const::Str(s.clone())),
        _ => None,
    }
}

/// Materialize a constant exactly the way the Push arm does (shared by the
/// superinstructions so their operand values cannot drift from Push's).
fn const_value(consts: &[Const], idx: u32) -> Value {
    match consts.get(idx as usize) {
        Some(Const::Null) | None => Value::Null,
        Some(Const::Bool(b)) => Value::Bool(*b),
        Some(Const::Int(i)) => Value::Int(*i),
        Some(Const::Float(f)) => Value::Float(*f),
        Some(Const::Str(s)) => Value::Str(s.clone()),
    }
}

struct LoopFrame {
    /// unresolved Brk sites (patched to the loop end; each site already
    /// carries its scope unwinds inline, emitted before the Brk)
    brks: Vec<usize>,
    /// unresolved Cont sites (patched to the loop top)
    conts: Vec<usize>,
    /// bridged statements inside this loop: (site, scope unwinds) patched
    /// with cont/brk targets at loop end so their internal break/continue
    /// flow lands on the right loop with the right envs restored
    bridges: Vec<(usize, usize)>,
    /// the compiler scope_depth at the loop top: every Brk/Cont/bridge
    /// flow unwinds (scope_depth - depth) scopes, the exact child envs the
    /// tree-walk abandons when its exec_block frames return
    depth: usize,
}

struct Compiler<'a> {
    consts: Vec<Const>,
    names: Vec<String>,
    exprs: &'a mut Vec<Expr>,
    stmts: &'a mut Vec<Stmt>,
    code: Vec<Instr>,
    lines: Vec<u32>,
    loops: Vec<LoopFrame>,
    /// scopes currently open (EnterScope minus ExitScope emitted): the
    /// machine restores them with ExitScope; break/continue and bridged
    /// loop flow must unwind exactly this many against the loop's depth
    scope_depth: usize,
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
            scope_depth: 0,
        }
    }

    /// Emit the ExitScope instructions that undo every scope opened since
    /// the innermost compiled loop's top (its per-iteration scope included):
    /// the tree-walk abandons those child envs when the break's Flow
    /// unwinds its exec_block frames; the machine restores `cur` the same
    /// way, so a `let` inside a loop body can never leak across a break.
    fn emit_scope_unwinds(&mut self) {
        let depth = self.loops.last().map(|f| f.depth).unwrap_or(0);
        for _ in depth..self.scope_depth {
            self.emit(Instr::ExitScope, 0);
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
                // W11 superinstruction: `name op literal` (3 instrs -> 1).
                // The literal is ALWAYS the rhs: evaluation order (lhs read
                // first) and non-commutative ops (Sub/Div/concat) rule out
                // any operand reordering, so this is a pure fusion.
                if let (Expr::Ident(name), Some(c)) = (&**l, literal_const(r)) {
                    let nidx = intern_name(&mut self.names, name);
                    let cidx = intern_const(&mut self.consts, c);
                    self.emit(Instr::LoadBinImm(nidx, *op, cidx), *op_line as u32);
                    return true;
                }
                if !self.expr(l, line) {
                    return false;
                }
                // W11 superinstruction: literal rhs (Push+Bin -> BinImm).
                if let Some(c) = literal_const(r) {
                    let cidx = intern_const(&mut self.consts, c);
                    self.emit(Instr::BinImm(*op, cidx), *op_line as u32);
                    return true;
                }
                let mark = self.code.len();
                if !self.expr(r, line) {
                    return false;
                }
                // rhs compiled natively as exactly one Push -> fold it.
                if self.code.len() == mark + 1 {
                    if let Instr::Push(idx) = self.code[mark] {
                        self.code.truncate(mark);
                        self.lines.truncate(mark);
                        self.emit(Instr::BinImm(*op, idx), *op_line as u32);
                        return true;
                    }
                }
                self.emit(Instr::Bin(*op), *op_line as u32);
            }
            Expr::Call(callee, args, call_line) => {
                // W09 native calls: only the bare-identifier callee compiles
                // to CallNamed; every other callee shape bridges whole (the
                // machine evaluates the args natively either way, each arg
                // pushing exactly one value, tree-walk order preserved).
                let name = match &**callee {
                    Expr::Ident(n) => n.clone(),
                    _ => return false,
                };
                let nidx = intern_name(&mut self.names, &name);
                // tree-walk stamp discipline: Expr::Call stamps the call
                // line at ARM ENTRY, before any argument evaluates, and
                // never re-stamps after (its builtin diagnostics carry the
                // LAST argument's line — rt_p22a's strengthen note). The
                // pre-arg Nop carries the stamp; the call insn itself must
                // not re-stamp post-args.
                self.emit(Instr::Nop, *call_line as u32);
                for a in args {
                    self.expr_or_bridge(a, line);
                }
                self.emit(Instr::CallNamed(nidx, args.len() as u32), 0);
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
                // W11 superinstruction: `return <name>` tail (2 instrs -> 1)
                if let Expr::Ident(name) = e {
                    let nidx = intern_name(&mut self.names, name);
                    self.emit(Instr::RetName(nidx), line);
                    return Vec::new();
                }
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
                self.emit_scope_unwinds();
                let site = self.emit(Instr::Brk(0), line);
                // ast-grep-ignore: no-unwrap-in-src
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
                self.emit_scope_unwinds();
                let site = self.emit(Instr::Cont(0), line);
                // ast-grep-ignore: no-unwrap-in-src
                self.loops.last_mut().unwrap().conts.push(site);
                Vec::new()
            }
            Stmt::Block(body) => {
                // the tree-walk's Block arm runs the body in a fresh child
                // scope ("exactly like `for name`"); EnterScope/ExitScope
                // mirror it, and the depth bookkeeping lets a break inside
                // the block unwind it on the way out
                self.emit(Instr::EnterScope, line);
                self.scope_depth += 1;
                let out = self.stmts(body);
                self.emit(Instr::ExitScope, line);
                self.scope_depth -= 1;
                out
            }
            Stmt::If(branches, els) => {
                // each branch: cond, JmpIfF(next), EnterScope, body,
                // ExitScope, Jmp(end). The tree-walk runs every branch body
                // in a FRESH child env (exec_block with a new child) and
                // stops at the first truthy branch; the machine mirrors
                // both, so a `let` inside a branch cannot leak past it.
                let mut end_jumps: Vec<usize> = Vec::new();
                let mut out: Vec<usize> = Vec::new();
                let last = branches.len().saturating_sub(1);
                for (i, (cond, body)) in branches.iter().enumerate() {
                    self.expr_or_bridge(cond, line);
                    let jif = self.emit(Instr::JmpIfF(0), line);
                    self.emit(Instr::EnterScope, line);
                    self.scope_depth += 1;
                    out.extend(self.stmts(body));
                    self.emit(Instr::ExitScope, line);
                    self.scope_depth -= 1;
                    // the LAST branch with no else falls into the end
                    // directly: its Jmp would target exactly the next
                    // instruction (a runtime no-op), so it is not emitted.
                    if !(i == last && els.is_none()) {
                        let jmp = self.emit(Instr::Jmp(0), line);
                        end_jumps.push(jmp);
                    }
                    let next = self.code.len() as u32;
                    self.code[jif] = Instr::JmpIfF(next);
                }
                if let Some(eb) = els {
                    // else body: fresh child scope, exactly like a branch
                    self.emit(Instr::EnterScope, line);
                    self.scope_depth += 1;
                    out.extend(self.stmts(eb));
                    self.emit(Instr::ExitScope, line);
                    self.scope_depth -= 1;
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
                    depth: self.scope_depth,
                });
                self.expr_or_bridge(cond, line);
                let jif = self.emit(Instr::JmpIfF(0), line);
                // per-iteration scope: the tree-walk builds a fresh child
                // env for EVERY iteration and evaluates the cond OUTSIDE it
                // (a `let` in the body is per-iteration; the machine must
                // not let it leak into the next iteration or past the loop)
                self.emit(Instr::EnterScope, line);
                self.scope_depth += 1;
                let _out = self.stmts(body);
                self.emit(Instr::ExitScope, line);
                self.scope_depth -= 1;
                self.emit(Instr::Jmp(top as u32), line);
                let end = self.code.len() as u32;
                self.code[jif] = Instr::JmpIfF(end);
                // ast-grep-ignore: no-unwrap-in-src
                let frame = self.loops.pop().unwrap();
                for b in frame.brks {
                    self.code[b] = Instr::Brk(end);
                }
                for c in frame.conts {
                    self.code[c] = Instr::Cont(top as u32);
                }
                for (bidx, unwinds) in frame.bridges {
                    self.code[bidx] = Instr::BridgeStmtInLoop(
                        self.stmt_idx_of(bidx),
                        top as u32,
                        end,
                        unwinds as u32,
                    );
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
                    depth: self.scope_depth,
                });
                // per-iteration scope: the tree-walk builds a fresh child
                // env for every iteration of a bare loop, exactly like while
                self.emit(Instr::EnterScope, line);
                self.scope_depth += 1;
                let _out = self.stmts(body);
                self.emit(Instr::ExitScope, line);
                self.scope_depth -= 1;
                self.emit(Instr::Jmp(top as u32), line);
                let end = self.code.len() as u32;
                // ast-grep-ignore: no-unwrap-in-src
                let frame = self.loops.pop().unwrap();
                for b in frame.brks {
                    self.code[b] = Instr::Brk(end);
                }
                for c in frame.conts {
                    self.code[c] = Instr::Cont(top as u32);
                }
                for (bidx, unwinds) in frame.bridges {
                    self.code[bidx] = Instr::BridgeStmtInLoop(
                        self.stmt_idx_of(bidx),
                        top as u32,
                        end,
                        unwinds as u32,
                    );
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
                    // needs this loop's cont/brk targets (patched at loop
                    // end) plus the count of scopes opened since the loop
                    // top, so a bridged break/continue restores the envs
                    // the tree-walk abandons on its way out
                    // ast-grep-ignore: no-unwrap-in-src
                    let unwinds = self.scope_depth - self.loops.last().unwrap().depth;
                    let site = self.emit(Instr::BridgeStmtInLoop(idx, 0, 0, unwinds as u32), line);
                    // ast-grep-ignore: no-unwrap-in-src
                    self.loops.last_mut().unwrap().bridges.push((site, unwinds));
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
    let code: std::rc::Rc<GeneCode> = {
        let opt = interp.vm_opt;
        // ast-grep-ignore: no-unwrap-in-src
        let prog = interp.vm_program.as_mut().unwrap();
        let key = if opt >= 1 {
            // cache the optimized form under a shifted key
            def_key.wrapping_add(1usize << 62)
        } else {
            def_key
        };
        match prog.codes.get(&key) {
            Some(cached) => cached.clone(),
            None => {
                let compiled = compile_body(name, body, prog);
                let compiled = if opt >= 1 {
                    optimize(&compiled)
                } else {
                    compiled
                };
                let rc = std::rc::Rc::new(compiled);
                prog.codes.insert(key, rc.clone());
                rc
            }
        }
    };
    exec_gene_code(interp, &code, env)
}

/// Execute a compiled gene body against the shared interpreter.
fn exec_gene_code(interp: &mut Interp, code: &GeneCode, env: &Rc<Env>) -> Result<Flow, Stress> {
    // the operand stack comes from the per-interpreter pool: fib25 taught
    // this lesson (243k fresh Vecs per run), the pool hands each frame a
    // warm stack and takes it back on every exit path
    let mut stack: Vec<Value> = match interp.vm_stack_pool.pop() {
        Some(s) => s,
        None => Vec::with_capacity(16),
    };
    stack.clear();
    let out = exec_gene_code_inner(interp, code, env, &mut stack);
    if stack.capacity() <= 64 {
        interp.vm_stack_pool.push(stack);
    }
    out
}

fn exec_gene_code_inner(
    interp: &mut Interp,
    code: &GeneCode,
    env: &Rc<Env>,
    stack: &mut Vec<Value>,
) -> Result<Flow, Stress> {
    let mut scopes: Vec<Rc<Env>> = Vec::new();
    let mut cur = env.clone();
    let mut ip: usize = 0;
    loop {
        // dispatch borrows the instruction (no per-instruction clone; the
        // machine must beat the tree-walk it replaced, the fib25 gate
        // measures exactly this)
        let instr = match code.code.get(ip) {
            Some(i) => i,
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
                let v = const_value(&code.consts, *idx);
                stack.push(v);
            }
            Instr::LoadName(idx) => {
                let name = &code.names[*idx as usize];
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
                let name = &code.names[*idx as usize];
                stack.push(cur.get(name).unwrap_or(Value::Null));
            }
            Instr::StoreName(idx) => {
                let name = code.names[*idx as usize].clone();
                let v = stack.pop().unwrap_or(Value::Null);
                if cur.get(&name).is_some() {
                    interp.note(0, 4, format!("rebinding '{}'", name));
                }
                cur.define(&name, v);
            }
            Instr::AssignName(idx) => {
                let name = code.names[*idx as usize].clone();
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
                let v = interp.apply_binop(&cur, *op, &l, &r)?;
                stack.push(v);
            }
            Instr::BinImm(op, cidx) => {
                // W11: identical to Bin with the rhs taken from the constant
                // pool (the folded Push); same apply_binop, same order
                // (lhs was evaluated first, by construction of the code).
                let r = const_value(&code.consts, *cidx);
                let l = stack.pop().unwrap_or(Value::Null);
                let v = interp.apply_binop(&cur, *op, &l, &r)?;
                stack.push(v);
            }
            Instr::LoadBinImm(nidx, op, cidx) => {
                // W11: the EXACT LoadName read (clone-charge + unbound
                // note) composed with the EXACT Bin arm. Nothing else may
                // differ: the unbound note text and the charge are output
                // and fuel contract respectively.
                let name = &code.names[*nidx as usize];
                let l = match cur.get(name) {
                    Some(v) => {
                        crate::interp::charge_clone(&v)?;
                        v
                    }
                    None => {
                        interp.note(0, 4, format!("unbound '{}' read as null", name));
                        Value::Null
                    }
                };
                let r = const_value(&code.consts, *cidx);
                let v = interp.apply_binop(&cur, *op, &l, &r)?;
                stack.push(v);
            }
            Instr::JmpIfF(t) => {
                let v = stack.pop().unwrap_or(Value::Null);
                if !v.truthy() {
                    ip = *t as usize;
                }
            }
            Instr::Jmp(t) => ip = *t as usize,
            Instr::EvalExpr(idx) => {
                // clone the bridged node out of the arena (cheap: Arc'd
                // children) so the mutable interpreter borrow is free
                let e = interp
                    .vm_program
                    .as_ref()
                    .and_then(|p| p.exprs.get(*idx as usize))
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
                    .and_then(|p| p.stmts.get(*idx as usize))
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
            Instr::BridgeStmtInLoop(idx, cont_t, brk_t, unwinds) => {
                let s = interp
                    .vm_program
                    .as_ref()
                    .and_then(|p| p.stmts.get(*idx as usize))
                    .cloned();
                if let Some(s) = s {
                    match interp.exec_stmt(&cur, &s)? {
                        Flow::Norm => {}
                        // a return from inside a bridged statement IS the
                        // gene's return (the tree-walk contract)
                        Flow::Ret(v) => return Ok(Flow::Ret(v)),
                        // a break/continue inside a bridged statement
                        // belongs to THIS compiled loop (compile-time fact);
                        // the scopes opened since the loop top are unwound
                        // first, the envs the tree-walk abandons when its
                        // exec_block frames return
                        Flow::Brk => {
                            for _ in 0..*unwinds {
                                if let Some(p) = scopes.pop() {
                                    cur = p;
                                }
                            }
                            ip = *brk_t as usize;
                        }
                        Flow::Cont => {
                            for _ in 0..*unwinds {
                                if let Some(p) = scopes.pop() {
                                    cur = p;
                                }
                            }
                            ip = *cont_t as usize;
                        }
                    }
                }
            }
            Instr::Pop => {
                stack.pop();
            }
            Instr::Nop => {
                // executes as nothing; exists to carry a line stamp
            }
            Instr::Ret => {
                let v = stack.pop().unwrap_or(Value::Null);
                return Ok(Flow::Ret(v));
            }
            Instr::RetName(nidx) => {
                // W11: the EXACT LoadName read composed with Ret.
                let name = &code.names[*nidx as usize];
                let v = match cur.get(name) {
                    Some(v) => {
                        crate::interp::charge_clone(&v)?;
                        v
                    }
                    None => {
                        interp.note(0, 4, format!("unbound '{}' read as null", name));
                        Value::Null
                    }
                };
                return Ok(Flow::Ret(v));
            }
            Instr::Brk(t) => ip = *t as usize,
            Instr::Cont(t) => ip = *t as usize,
            Instr::EnterScope => {
                scopes.push(cur.clone());
                cur = Env::new(Some(cur.clone()));
            }
            Instr::ExitScope => {
                if let Some(p) = scopes.pop() {
                    cur = p;
                }
            }
            Instr::CallNamed(name_idx, argc) => {
                // W09 native calls: pop the args in reverse, then ride the
                // SHARED named-call tail (RISC gate + call_named funnel).
                // The call line was stamped pre-args by the Nop (the
                // tree-walk stamps at arm entry and never re-stamps after
                // the args — this insn carries line 0 on purpose).
                // The name is borrowed, not cloned: fib25 measures 242k
                // calls and the clone was a malloc per call.
                let name = code.names[*name_idx as usize].as_str();
                let n = *argc as usize;
                let base = stack.len() - n;
                let argvs: Vec<Value> = stack.drain(base..).collect();
                let v = interp.named_call_tail_vm(&cur, name, argvs)?;
                stack.push(v);
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
        Instr::BinImm(..) => "BinImm",
        Instr::LoadBinImm(..) => "LoadBinImm",
        Instr::JmpIfF(_) => "JmpIfF",
        Instr::Jmp(_) => "Jmp",
        Instr::EvalExpr(_) => "EvalExpr",
        Instr::BridgeStmt(_) => "BridgeStmt",
        Instr::BridgeStmtInLoop(..) => "BridgeStmtInLoop",
        Instr::Pop => "Pop",
        Instr::Nop => "Nop",
        Instr::Ret => "Ret",
        Instr::RetName(_) => "RetName",
        Instr::Brk(_) => "Brk",
        Instr::Cont(_) => "Cont",
        Instr::EnterScope => "EnterScope",
        Instr::ExitScope => "ExitScope",
        Instr::CallNamed(_, _) => "CallNamed",
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
        | Instr::AssignName(idx)
        | Instr::RetName(idx) => code
            .names
            .get(*idx as usize)
            .cloned()
            .unwrap_or_else(|| format!("c{}", idx)),
        Instr::Bin(op) => format!("{:?}", op),
        Instr::BinImm(op, idx) => format!("{:?} {}", op, render_const(code, *idx)),
        Instr::LoadBinImm(nidx, op, idx) => format!(
            "'{}' {:?} {}",
            code.names.get(*nidx as usize).cloned().unwrap_or_default(),
            op,
            render_const(code, *idx)
        ),
        Instr::JmpIfF(t) | Instr::Jmp(t) | Instr::Brk(t) | Instr::Cont(t) => format!("-> {}", t),
        Instr::EvalExpr(idx) => format!("expr#{}", idx),
        Instr::BridgeStmt(idx) => format!("stmt#{}", idx),
        Instr::BridgeStmtInLoop(idx, cont_t, brk_t, unwinds) => {
            format!(
                "stmt#{} cont {} brk {} unwinds {}",
                idx, cont_t, brk_t, unwinds
            )
        }
        Instr::CallNamed(idx, argc) => {
            format!(
                "'{}' argc {}",
                code.names.get(*idx as usize).cloned().unwrap_or_default(),
                argc
            )
        }
        _ => String::new(),
    }
}

fn render_const(code: &GeneCode, idx: u32) -> String {
    match code.consts.get(idx as usize) {
        Some(Const::Str(s)) => format!("c{} {:?}", idx, s),
        Some(Const::Int(v)) => format!("c{} {}", idx, v),
        Some(other) => format!("c{} {:?}", idx, other),
        None => format!("c{} <missing>", idx),
    }
}

/// W11 stage 1+2: the optimization pipeline. Draw-free, provably
/// behavior-preserving passes over the OIR1:
///   1. constant folding: Push a, Push b, Bin -> Push (folded). Only pure
///      Int/Float arithmetic folds, and only when the operation cannot
///      stress (i64 checked ops); anything that would raise at runtime
///      stays runtime (the stress kind+message are the contract).
///   2. jump threading: a Jmp whose target is another Jmp follows the
///      chain; an unconditional Jmp to the NEXT instruction disappears.
///   3. reachability DCE: instructions that cannot be reached from ip 0
///      (dead code after a Ret, abandoned jump islands) are dropped. They
///      never execute, so no tick, note, draw or output can change.
///      Folding never touches a draw or a call: those are Bridge/EvalExpr
///      instructions, and the folder must not reorder or remove draws
///      (the entropy discipline, vm-design.md invariant 2).
pub fn optimize(code: &GeneCode) -> GeneCode {
    let mut consts = code.consts.clone();
    let mut code = code.clone();

    // pass 1: constant folding (fixpoint, bounded). Both Bin shapes fold:
    // the Push/Push/Bin triple and the W11 fused Push/BinImm pair (the
    // folded form of a literal rhs).
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
        let mut i = 0;
        while i + 1 < code.code.len() {
            if let (Instr::Push(a), Instr::BinImm(op, b)) = (&code.code[i], &code.code[i + 1]) {
                if let Some(newc) = fold_consts(&mut consts, *a, *b, *op) {
                    code.code[i] = Instr::Push(newc);
                    code.code.remove(i + 1);
                    code.lines.remove(i + 1);
                    remap_after_removal(&mut code, i + 1, 1);
                    folded = true;
                    continue;
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

    // pass 3: reachability DCE. Mark from ip 0 following fallthrough and
    // every jump target; keep the marked instructions in order and remap
    // every target through the old->new table. A target may legally be
    // code.len() ("fall off the end"), so the table has n+1 slots.
    {
        let n = code.code.len();
        let mut mask = vec![false; n];
        let mut work = vec![0usize];
        while let Some(ip) = work.pop() {
            if ip >= n || mask[ip] {
                continue;
            }
            mask[ip] = true;
            match &code.code[ip] {
                Instr::Jmp(t) | Instr::Brk(t) | Instr::Cont(t) => work.push(*t as usize),
                Instr::JmpIfF(t) => {
                    work.push(*t as usize);
                    work.push(ip + 1);
                }
                Instr::Ret | Instr::RetName(_) => {}
                _ => work.push(ip + 1),
            }
        }
        if mask.iter().any(|m| !*m) {
            let mut map = vec![0u32; n + 1];
            let mut next = 0u32;
            for i in 0..n {
                if mask[i] {
                    map[i] = next;
                    next += 1;
                }
            }
            map[n] = next; // the fall-off-the-end target
            let mut new_code = Vec::with_capacity(next as usize);
            let mut new_lines = Vec::with_capacity(next as usize);
            for (i, keep) in mask.iter().enumerate() {
                if !keep {
                    continue;
                }
                let mut instr = code.code[i].clone();
                match &mut instr {
                    Instr::Jmp(t) | Instr::JmpIfF(t) | Instr::Brk(t) | Instr::Cont(t) => {
                        *t = map[*t as usize];
                    }
                    _ => {}
                }
                new_code.push(instr);
                new_lines.push(code.lines[i]);
            }
            code.code = new_code;
            code.lines = new_lines;
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
            Div => {
                // compat-matrix finding (rt_p5a_arith, 2026-09-30): the
                // runtime stresses on float division by zero (interp
                // apply_binop: "division by zero"), so a zero divisor must
                // NOT fold — the runtime stress IS the contract. Covers
                // -0.0 as well: IEEE == treats -0.0 == 0.0.
                if y == 0.0 {
                    return None;
                }
                Const::Float(x / y)
            }
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
            // W11: `n + 1` and `x * 2` are the measured LoadName+Push+Bin
            // triples, fused to LoadBinImm (the read arm inside is byte-exact
            // LoadName: clone-charge + unbound note).
            "LoadBinImm 'n' Add c0 1",
            "StoreName x",
            "LoadBinImm 'x' Mul c1 2",
            "Ret",
        ];
        assert_eq!(rendered, expected, "OIR1 encoding drifted");
        let _ = OIR_VERSION;
    }

    /// W11 stage 2: the fib25 gate's measured hot shapes fuse to
    /// superinstructions, one tick each, same apply_binop, same read arm.
    #[test]
    fn fuses_the_fib25_hot_shapes() {
        let src = "gene f(n) {\n    if n < 2 { return n }\n    return n * 2 + 1\n}\n";
        let parsed = crate::parser::parse(src);
        let mut body: Vec<Stmt> = Vec::new();
        for s in &parsed.stmts {
            if let Stmt::Gene(g) = s {
                body = g.body.clone();
            }
        }
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
            // W11: `n < 2` fuses to LoadBinImm; the branch body runs in its
            // own scope (the tree-walk's fresh child env) and the last
            // branch with no else falls into the end (no trailing Jmp);
            // `return n` fuses to RetName; `n * 2` fuses and `+ 1` rides
            // BinImm.
            "LoadBinImm 'n' Lt c0 2",
            "JmpIfF -> 5",
            "EnterScope",
            "RetName n",
            "ExitScope",
            "LoadBinImm 'n' Mul c0 2",
            "BinImm Add c1 1",
            "Ret",
        ];
        assert_eq!(rendered, expected, "superinstruction fusion drifted");
    }

    /// W11 stage 2: reachability DCE drops only code after an unconditional
    /// exit; the jump targets survive remapped and in range.
    #[test]
    fn dce_drops_only_unreachable_code() {
        let src = "gene f() {\n    return 1\n    print(\"dead\")\n}\n";
        let parsed = crate::parser::parse(src);
        let mut body: Vec<Stmt> = Vec::new();
        for s in &parsed.stmts {
            if let Stmt::Gene(g) = s {
                body = g.body.clone();
            }
        }
        let mut prog = VmProgram::default();
        let raw = compile_body("f", &body, &mut prog);
        // the raw body carries the dead call (Nop stamp + CallNamed print +
        // Pop); the pre-arg Nop stamp (rt_p22a fix) shifts the call to [4]
        assert!(
            render(&raw.code[4], &raw).starts_with("'print'"),
            "fixture stale"
        );
        let opt = optimize(&raw);
        let names: Vec<String> = opt.code.iter().map(|i| mnemonic(i).to_string()).collect();
        assert_eq!(
            names,
            vec!["Push", "Ret"],
            "DCE must keep the live prefix and drop the dead tail"
        );
        // every surviving target stays in range (no dangling remap)
        for i in &opt.code {
            if let Instr::Jmp(t) | Instr::JmpIfF(t) | Instr::Brk(t) | Instr::Cont(t) = i {
                assert!((*t as usize) <= opt.code.len(), "DCE remap out of range");
            }
        }
    }
}

#[cfg(test)]
mod stability_tests {
    use super::*;
    use crate::parser::parse;

    /// W10 done-when: the annotated dump is STABLE across runs. The same
    /// source compiles to the same listing, every time, opt on or off, and
    /// the listing carries every opcode mnemonic the machine knows (the
    /// SPEC 8 VM table and this test move together).
    #[test]
    fn dump_is_deterministic_and_complete() {
        let src = "\
gene f(n) {
    let x = n + 1
    return x * 2
}
gene g(xs) {
    let acc = 0
    for v in xs {
        acc = acc + v
    }
    if acc > 10 {
        return acc
    }
    return f(acc)
}
gene main() {
    print(g([1, 2, 3]))
}
";
        let d1 = disassemble_program(&parse(src));
        let d2 = disassemble_program(&parse(src));
        assert_eq!(d1, d2, "same source, same listing, always");
        // every opcode the machine executes appears in the listing of this
        // deliberately mixed program (bridges included: gene values in args;
        // W11 superinstructions included: `acc > 10` and `return acc` fuse)
        for m in [
            "Push",
            "LoadName",
            "StoreName",
            "Bin",
            "LoadBinImm",
            "RetName",
            "JmpIfF",
            "CallNamed",
            "Ret",
            "Pop",
        ] {
            assert!(d1.contains(m), "listing missing {m}:\n{d1}");
        }
        // an unknown mnemonic would mean the SPEC VM table drifted
        for line in d1.lines() {
            if line.contains('|') {
                let m = line.split('|').nth(1).unwrap_or("").trim();
                assert!(!m.is_empty(), "bare listing line: {line}");
            }
        }
    }
}
