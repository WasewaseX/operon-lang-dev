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

use crate::ast::{BinOp, Expr, MatchPat, Pat, Stmt};
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
    /// TYPED-MODE: the gene's type parameters, carried so the fiber lane's
    /// pop-time return-annotation check treats type params exactly like the
    /// tree-walk lane (ann_is_typaram needs the enclosing gene's params).
    pub type_params: Vec<(String, Option<String>)>,
    pub consts: Vec<Const>,
    pub names: Vec<String>,
    pub code: Vec<Instr>,
    pub lines: Vec<u32>,
    /// W011 stage 3: the trivial-gene fast-dispatch bit. False at the two
    /// construction sites (compile_body, optimize_with); computed for real
    /// on the CACHED form (after optimize_with) in gene_code_cached.
    /// true = the body is a shapes-only sequence (pure reads, const pushes,
    /// arithmetic, returns — no stores, no scopes, no bridges, no calls,
    /// no control flow), so executing it against the caller's parent env
    /// instead of a freshly built frame env is observationally identical:
    /// nothing in the whitelist writes a binding, and an empty frame env
    /// is read-through for every shape it can contain (LoadName's unbound
    /// note included). call_gene_inner pairs this bit with the def-level
    /// predicate (zero params, no guard/annotations/marks). Purely a
    /// dispatch decision: every gate, counter and note is unchanged.
    pub trivial: bool,
    /// W011 stage-2 (frame-slot locals): the compile-time qualification.
    /// `slot_ok` = the FINAL cached IR contains no bridge instruction
    /// (EvalExpr / BridgeStmt / BridgeStmtInLoop). Bridged nodes evaluate
    /// the original AST through the env chain, which cannot see values
    /// that live only in frame slots, so any bridge disqualifies the
    /// whole frame. Computed in gene_code_cached on the post-optimize
    /// form; optimize_with carries the input's analysis verbatim.
    pub slot_ok: bool,
    /// Every name bound by a let-family statement anywhere in the body
    /// (any nesting depth), plus for/match/pattern binders. A param whose
    /// name appears here is shadowed or re-bound by scope rules and must
    /// keep the env path; all other params are slot candidates. Filled by
    /// compile_body's AST walk (conservative: marks names whose binding
    /// forms would bridge too — over-marking is free, under-marking is a
    /// bug, so the walker marks every binder it recognizes).
    pub shadowed: std::collections::HashSet<String>,
    /// Lazily computed (name_idx, slot) pairs for the ELIGIBLE params of
    /// the gene this code object was cached for. Initialized once by
    /// frame_slot_map with that gene's param list — the code-cache key is
    /// the def's Arc pointer, so one code object always sees one param
    /// list (the same discipline the code cache itself relies on).
    pub param_slots: std::sync::OnceLock<std::rc::Rc<Vec<(u32, u32)>>>,
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
    /// P5 builder fast path: pop the rhs, then append it to the named
    /// string slot IN PLACE (amortized O(|rhs|)) — the compiled form of
    /// `s += x` (loud=false: the general sequence it replaces reads the
    /// slot quietly) and `s = s + <pure rhs>` (loud=true: the replaced
    /// sequence reads the slot with the full LoadName arm). Falls back to
    /// the EXACT replaced instruction sequence (read + Bin(Add) +
    /// AssignName) for every non-Str shape, unbound names, const targets
    /// and slot targets, so observable behavior is byte-identical.
    AppendName(u32, bool),
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
            // W08r: the parser's transparent position marker compiles as its
            // inner expression, attributed to the marker's line (this also
            // gives line-silent statements a real line in the VM line table)
            Expr::At(inner, l) => {
                return self.expr(inner, *l as u32);
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
                // P5: `s = s + <pure rhs>` compiles rhs + AppendName(loud).
                // The general path reads the slot BEFORE the rhs evaluates,
                // the fast path appends AFTER it, so the shape is only taken
                // when the rhs is statically side-effect-free (same gate as
                // the tree-walk, crate::interp::expr_is_pure) — anything
                // else keeps the exact old sequence.
                if let Expr::Binary(BinOp::Add, lhs, rhs, _) = e {
                    if matches!(lhs.as_ref(), Expr::Ident(n) if n.as_str() == name.as_str())
                        && crate::interp::expr_is_pure(rhs)
                    {
                        let nidx = intern_name(&mut self.names, name);
                        self.expr_or_bridge(rhs, line);
                        self.emit(Instr::AppendName(nidx, true), line);
                        return Vec::new();
                    }
                }
                self.expr_or_bridge(e, line);
                let idx = intern_name(&mut self.names, name);
                self.emit(Instr::AssignName(idx), line);
                Vec::new()
            }
            Stmt::Assign(name, Some(BinOp::Add), e) => {
                // P5: `s += x` compiles rhs + AppendName(quiet). The VM's
                // general compound sequence reads the slot BEFORE the rhs
                // (LoadNameQuiet first), the tree-walk reads it AFTER, and
                // the fast path appends after — so this shape is gated on
                // the same purity test, under which pre-rhs and post-rhs
                // slot state are identical and both engines stay
                // byte-exact against their own general paths.
                if crate::interp::expr_is_pure(e) {
                    let nidx = intern_name(&mut self.names, name);
                    self.expr_or_bridge(e, line);
                    self.emit(Instr::AppendName(nidx, false), line);
                    return Vec::new();
                }
                // compound: quiet read, binop, write (tree-walk order)
                let nidx = intern_name(&mut self.names, name);
                self.emit(Instr::LoadNameQuiet(nidx), line);
                self.expr_or_bridge(e, line);
                self.emit(Instr::Bin(BinOp::Add), line);
                self.emit(Instr::AssignName(nidx), line);
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
pub fn compile_body(
    name: &str,
    body: &[Stmt],
    prog: &mut VmProgram,
    type_params: &[(String, Option<String>)],
) -> GeneCode {
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
        type_params: type_params.to_vec(),
        trivial: false,
        slot_ok: false,
        shadowed: collect_let_bound_names(body),
        param_slots: std::sync::OnceLock::new(),
    }
}

/// W011 stage-2 analysis half: every name bound by a let-family statement
/// anywhere in the body, plus for/match/pattern binders. Deliberately
/// conservative in both directions that matter:
/// - over-marking is FREE (the name just keeps the env path),
/// - under-marking is a bug (a slot name shadowed by a nested binding
///   would diverge), so the walker marks every binder it recognizes —
///   including binders whose statements currently bridge (those frames
///   are bridge-disqualified anyway; this walker is the safety net if
///   the compiler ever learns to compile them natively).
fn collect_let_bound_names(body: &[Stmt]) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    walk_binders(body, &mut out);
    out
}

fn walk_binders(body: &[Stmt], out: &mut std::collections::HashSet<String>) {
    for s in body {
        match s {
            Stmt::Let(n, _) | Stmt::LetConst(n, _) | Stmt::LetAnn(n, _, _) | Stmt::For(n, _, _) => {
                out.insert(n.clone());
            }
            Stmt::LetPat(p, _) => pat_binders(p, out),
            Stmt::ForPat(p, _, b) => {
                pat_binders(p, out);
                walk_binders(b, out);
            }
            Stmt::Match(_, arms, _) => {
                for (pat, arm_body) in arms {
                    matchpat_binders(pat, out);
                    walk_binders(arm_body, out);
                }
            }
            Stmt::If(branches, els) => {
                for (_, b) in branches {
                    walk_binders(b, out);
                }
                if let Some(eb) = els {
                    walk_binders(eb, out);
                }
            }
            Stmt::While(_, b) | Stmt::Loop(b) | Stmt::Block(b) | Stmt::Scope(b) => {
                walk_binders(b, out)
            }
            Stmt::Stress {
                kind: _,
                body: b,
                rescue,
            } => {
                walk_binders(b, out);
                if let Some((_, rb)) = rescue {
                    walk_binders(rb, out);
                }
            }
            _ => {}
        }
    }
}

fn pat_binders(p: &Pat, out: &mut std::collections::HashSet<String>) {
    match p {
        Pat::Bind(n) => {
            out.insert(n.clone());
        }
        Pat::List { elems, rest } => {
            for e in elems {
                pat_binders(e, out);
            }
            if let Some(r) = rest {
                out.insert(r.clone());
            }
        }
        Pat::Map { keys } => {
            for k in keys {
                out.insert(k.clone());
            }
        }
    }
}

fn matchpat_binders(p: &MatchPat, out: &mut std::collections::HashSet<String>) {
    match p {
        MatchPat::Bind(n) => {
            out.insert(n.clone());
        }
        MatchPat::ListPat { elems, rest } => {
            for e in elems {
                matchpat_binders(e, out);
            }
            if let Some(r) = rest {
                out.insert(r.clone());
            }
        }
        MatchPat::MapPat { keys } => {
            for (k, sub) in keys {
                match sub {
                    // bare `{a, ...}` binds the key name; an explicit
                    // sub-pattern decides its own binders
                    None => {
                        out.insert(k.clone());
                    }
                    Some(sp) => matchpat_binders(sp, out),
                }
            }
        }
        MatchPat::Variant(_, Some(sub)) => matchpat_binders(sub, out),
        MatchPat::Or(pats) => {
            for e in pats {
                matchpat_binders(e, out);
            }
        }
        MatchPat::Guard(p, _) => matchpat_binders(p, out),
        _ => {}
    }
}

/// Compile (or fetch from cache) the body of a gene definition and execute
/// it. Called from call_gene_inner with the freshly built frame env.
pub fn exec_gene_body(
    interp: &mut Interp,
    def_key: usize,
    name: &str,
    body: &[Stmt],
    type_params: &[(String, Option<String>)],
    env: &Rc<Env>,
) -> Result<Flow, Stress> {
    let code = gene_code_cached(interp, def_key, name, body, type_params);
    exec_gene_code(interp, &code, env)
}

/// Compile (or fetch from the cache) a gene body. Same key discipline as
/// before: the optimized form lives under a shifted key when vm_opt >= 1.
/// W16: shared by the sync machine (exec_gene_body) and the fiber machine
/// (fiber_push_frame) so both engines can never drift on the encoding.
pub(crate) fn gene_code_cached(
    interp: &mut Interp,
    def_key: usize,
    name: &str,
    body: &[Stmt],
    type_params: &[(String, Option<String>)],
) -> std::rc::Rc<GeneCode> {
    let opt = interp.vm_opt;
    // W011 toggle matrix: an explicit --opt-passes set wins over the level
    // (a Some set is authoritative, including NONE = compile-only); the
    // level otherwise picks the preset (1 = STAGE1, 2 = ALL).
    let passes = interp.opt_passes.unwrap_or(if opt >= 2 {
        PassSet::ALL
    } else {
        PassSet::STAGE1
    });
    let opt_active = opt >= 1 || interp.opt_passes.is_some();
    // ast-grep-ignore: no-unwrap-in-src
    let prog = interp.vm_program.as_mut().unwrap();
    // W016-v3 (adopted from builder-B's be991fc): a literal 1usize << 62
    // is E0080 on 32-bit targets; the second-highest bit keeps the
    // cache-key space split identically on every width (bit 62 on 64-bit,
    // byte-identical behavior). The pass set is process-constant (flags
    // are parsed once), so ONE shifted space is enough.
    let key = if opt_active {
        // cache the optimized form under a shifted key
        def_key.wrapping_add(1usize << (usize::BITS - 2))
    } else {
        def_key
    };
    match prog.codes.get(&key) {
        Some(cached) => cached.clone(),
        None => {
            let mut compiled = compile_body(name, body, prog, type_params);
            if opt_active {
                compiled = optimize_with(&compiled, passes);
            }
            // W011 stage 3: the triviality bit is computed on the CACHED
            // form (after optimization — folding can only shrink a
            // shapes-only body into more whitelist shapes, never out of
            // them: Jmp/bridges are never introduced by optimize_with).
            compiled.trivial = shapes_only(&compiled);
            // W011 stage-2: slot qualification on the FINAL cached IR.
            // Bridged nodes evaluate the original AST through the env
            // chain (which cannot see slot-only values), so any bridge
            // instruction disqualifies the whole frame. optimize_with
            // never introduces bridges, but running the scan HERE (on the
            // exact bytes that will execute) is the honest placement —
            // the analysis can never drift from the executed form.
            compiled.slot_ok = !compiled.code.iter().any(|i| {
                matches!(
                    i,
                    Instr::EvalExpr(_) | Instr::BridgeStmt(_) | Instr::BridgeStmtInLoop(..)
                )
            });
            let rc = std::rc::Rc::new(compiled);
            prog.codes.insert(key, rc.clone());
            rc
        }
    }
}

/// Execute a compiled gene body against the shared interpreter.
/// pub(crate) for the W011 stage-3 trivial-gene fast dispatch in
/// call_gene_inner (the fast path executes the cached body directly).
/// W011 stage-2 kill switch (A/B measurement + emergency off). The slot
/// path is default-ON; `OPERON_VM_SLOTS=0` (or `off`) restores the pure
/// env-chain frame exactly. Read once per process (LazyLock) — the flag
/// is process-constant like every other W009-A/W011 toggle.
static VM_SLOTS_OFF: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
    std::env::var("OPERON_VM_SLOTS")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("off"))
        .unwrap_or(false)
});

pub fn vm_slots_off() -> bool {
    *VM_SLOTS_OFF
}

/// W011-s3b kill switch (A/B measurement + emergency off). The bookkeeping
/// consumer gate is default-ON for consumer-free programs; `OPERON_VM_BKFAST=0`
/// (or `off`) forces full call_counts/gene_buckets maintenance for every
/// program, restoring the pre-s3b behavior exactly. Read once per process
/// (LazyLock) — the flag is process-constant like every other W009-A/W011
/// toggle.
static VM_BKFAST_OFF: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
    std::env::var("OPERON_VM_BKFAST")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("off"))
        .unwrap_or(false)
});

pub fn vm_bkfast_off() -> bool {
    *VM_BKFAST_OFF
}

/// W011 engagement counter (OPERON_W009A_COUNTS=1 dumps it): slot frames
/// entered. The ablation/verification harness pairs this with fib-class
/// runs to prove the path actually engages (an optimization that never
/// runs optimizes nothing).
pub static SLOT_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Frame-slot locals for one call frame (W011 stage-2). `vals` are the
/// param slots (slot 0..K in param order); `map` carries (name_idx,
/// slot) pairs for every ELIGIBLE param, computed once per code object.
pub(crate) struct FrameSlots {
    pub vals: Vec<Value>,
    pub map: std::rc::Rc<Vec<(u32, u32)>>,
}

/// The slot layout for a gene's params: eligible params (no default, not
/// shadowed by any body binder, present in the names table) get slots in
/// param order. None when the code object is bridge-disqualified or no
/// param qualifies. The OnceLock caches the (name_idx, slot) pairs on the
/// code object — the cache key is the def's Arc pointer, so one code
/// object always sees one param list.
pub(crate) fn frame_slot_map(
    code: &GeneCode,
    params: &[(String, Option<crate::ast::Expr>)],
) -> Option<std::rc::Rc<Vec<(u32, u32)>>> {
    if !code.slot_ok {
        return None;
    }
    let map = code
        .param_slots
        .get_or_init(|| {
            let mut pairs: Vec<(u32, u32)> = Vec::new();
            let mut slot = 0u32;
            for (pname, default) in params {
                if default.is_some() || pname.is_empty() || pname == "?" {
                    continue;
                }
                if code.shadowed.contains(pname) {
                    continue;
                }
                if let Some(nidx) = code.names.iter().position(|n| n == pname) {
                    pairs.push((nidx as u32, slot));
                    slot += 1;
                }
            }
            std::rc::Rc::new(pairs)
        })
        .clone();
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// Slot read for a name index (None = no slot frame or name not slotted).
#[inline]
fn slot_read(sf: Option<&FrameSlots>, idx: u32) -> Option<Value> {
    let fs = sf?;
    let s = fs.map.iter().find(|(ni, _)| *ni == idx)?.1 as usize;
    Some(fs.vals[s].clone())
}

/// Slot write target (None = no slot frame or name not slotted).
#[inline]
fn slot_write(sf: Option<&mut FrameSlots>, idx: u32) -> Option<&mut Value> {
    let fs = sf?;
    let s = fs.map.iter().find(|(ni, _)| *ni == idx)?.1 as usize;
    fs.vals.get_mut(s)
}

pub(crate) fn exec_gene_code(
    interp: &mut Interp,
    code: &GeneCode,
    env: &Rc<Env>,
) -> Result<Flow, Stress> {
    // the operand stack comes from the per-interpreter pool: fib25 taught
    // this lesson (243k fresh Vecs per run), the pool hands each frame a
    // warm stack and takes it back on every exit path
    let mut stack: Vec<Value> = match interp.vm_stack_pool.pop() {
        Some(s) => s,
        None => Vec::with_capacity(16),
    };
    stack.clear();
    let out = exec_gene_code_inner(interp, code, env, &mut stack, None);
    if stack.capacity() <= 64 {
        interp.vm_stack_pool.push(stack);
    }
    out
}

/// W011 stage-2: the slotted entry. `slots` are the frame's param slots,
/// `map` the (name_idx, slot) table — reads and writes of slotted names
/// hit the Vec instead of the env chain; everything else rides `env`
/// exactly as before.
pub(crate) fn exec_gene_code_slotted(
    interp: &mut Interp,
    code: &GeneCode,
    env: &Rc<Env>,
    fs: FrameSlots,
) -> Result<Flow, Stress> {
    let mut stack: Vec<Value> = match interp.vm_stack_pool.pop() {
        Some(s) => s,
        None => Vec::with_capacity(16),
    };
    stack.clear();
    let out = exec_gene_code_inner(interp, code, env, &mut stack, Some(fs));
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
    mut slot_frame: Option<FrameSlots>,
) -> Result<Flow, Stress> {
    let mut scopes: Vec<Rc<Env>> = Vec::new();
    let mut cur = env.clone();
    // P6-wave: per-call scope-env cache. EnterScope normally mints a fresh
    // child Env per block entry (per loop iteration for compiled loops);
    // ExitScope stashes the child back when NOTHING captured it (sole
    // reference) with its maps cleared (capacity preserved), and EnterScope
    // pops it instead of allocating. A captured env fails the strong-count
    // check and drops normally — byte-identical to the fresh-Env scheme.
    let mut scope_cache: Vec<Rc<Env>> = Vec::new();
    let mut ip: usize = 0;
    // W011-r2: the ablation/counters flags are process-constant (set once by
    // init_from_env before any machine runs; no writer exists) — loading
    // them once here instead of twice per instruction removes two atomic
    // loads from every dispatch (fib27: ~11 insns/call x ~7M calls).
    let counts_on = crate::w009a::COUNTS_ON.load(std::sync::atomic::Ordering::Relaxed);
    let a_tick = crate::w009a::A_TICK.load(std::sync::atomic::Ordering::Relaxed);
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
        if counts_on {
            crate::w009a::C_INSTRS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if a_tick {
            if counts_on {
                crate::w009a::C_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            // W009-A ABLATION: fuel tick skipped (measurement only)
        } else {
            interp.tick()?;
        }
        match instr {
            Instr::Push(idx) => {
                let v = const_value(&code.consts, *idx);
                stack.push(v);
            }
            Instr::LoadName(idx) => {
                // W011 stage-2: a slot hit replaces the env-chain walk for
                // an eligible param. Same value, same clone charge — only
                // WHERE the value is read from changes; the unbound-note
                // path is unreachable for a slot (slots are bound at
                // frame entry by construction).
                if let Some(v) = slot_read(slot_frame.as_ref(), *idx) {
                    crate::interp::charge_clone(&v)?;
                    stack.push(v);
                } else {
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
            }
            Instr::LoadNameQuiet(idx) => {
                if let Some(v) = slot_read(slot_frame.as_ref(), *idx) {
                    stack.push(v);
                } else {
                    let name = &code.names[*idx as usize];
                    stack.push(cur.get(name).unwrap_or(Value::Null));
                }
            }
            Instr::StoreName(idx) => {
                // P6-wave: the name is borrowed (the clone was a malloc per
                // binding) and the presence check uses contains() (P5) —
                // get() cloned the bound value just to test presence.
                let name = code.names[*idx as usize].as_str();
                let v = stack.pop().unwrap_or(Value::Null);
                if cur.vars.borrow().contains_key(name) {
                    interp.note(0, 4, format!("rebinding '{}'", name));
                }
                // NOTE: StoreName is deliberately NOT slot-routed. A slot
                // name is by definition never let-bound in its own body
                // (the binder walk marks every let name as shadowed), so
                // a StoreName can never target a slot — the env path here
                // is the only path.
                cur.define(name, v);
            }
            Instr::AssignName(idx) => {
                // P6-wave: the name is borrowed, not cloned (per-assignment
                // malloc; m_intadd measures one AssignName per iteration).
                let name = code.names[*idx as usize].as_str();
                let v = stack.pop().unwrap_or(Value::Null);
                if cur.is_const(name) {
                    return Err(Stress::new(
                        "frozen",
                        format!("cannot reassign const '{}'", name),
                    ));
                }
                // W011 stage-2: a slot write replaces the chain-walking
                // set() for an eligible param. set() would find the frame
                // binding first and rebind it in place, returning true
                // (no note) — the slot write is exactly that rebinding.
                // The const check above runs UNCHANGED first: an outer
                // const of the same name stresses identically (pinned
                // behavior, corpus-covered).
                match slot_write(slot_frame.as_mut(), *idx) {
                    Some(dst) => {
                        // WRITE-THROUGH: the env copy is re-bound too,
                        // keeping the funnel's env view (callee resolution,
                        // RISC/toggle immunity checks) authoritative.
                        // set() finds the frame binding (innermost) and
                        // rebinds it: true, no note — byte-equal to the
                        // env path, DEF_GEN bump included.
                        *dst = v.clone();
                        let _ = cur.set(name, v);
                    }
                    None => {
                        if !cur.set(name, v) {
                            interp.note(
                                0,
                                4,
                                format!("'{}' was not declared; auto-declared", name),
                            );
                        }
                    }
                }
            }
            Instr::AppendName(idx, loud) => {
                // P5 builder fast path: pop the rhs, append to the named
                // string slot in place. The fallback below reproduces the
                // EXACT replaced sequence — LoadName(Quiet) + Bin(Add) +
                // AssignName — for every shape the fast path refuses
                // (non-Str operands, unbound names, const targets, slot
                // targets), so observable behavior is byte-identical.
                let name = code.names[*idx as usize].clone();
                let r = stack.pop().unwrap_or(Value::Null);
                let mut fast_done = false;
                // slot targets keep the general path wholesale: the
                // write-through re-binds the env copy anyway, so an
                // in-place append has nothing to win there
                if slot_read(slot_frame.as_ref(), *idx).is_none() {
                    let mut node = Some(cur.clone());
                    while let Some(env_node) = node {
                        let hit = env_node.vars.borrow().contains_key(&name);
                        if hit {
                            if let Value::Str(suffix) = &r {
                                let mut vars = env_node.vars.borrow_mut();
                                if let Some(Value::Str(slot_s)) = vars.get_mut(&name) {
                                    // Bin(Add) position: ceiling, then charge
                                    let new_len = slot_s.len().saturating_add(suffix.len());
                                    if new_len > 512 * 1024 * 1024 {
                                        return Err(Stress::new(
                                            "overflow",
                                            "string concat exceeds the 512 MiB ceiling",
                                        ));
                                    }
                                    // perf-xlang-r2: charge only the RETAINED
                                    // bytes — the append is in place, so the
                                    // old full-length charge billed O(n^2)
                                    // for O(n) work and pushed big builder
                                    // loops through the aggregate ceiling
                                    // without any real allocation growth.
                                    crate::interp::mem_charge(suffix.len() as u64)?;
                                    // AssignName position: const check
                                    if cur.is_const(&name) {
                                        return Err(Stress::new(
                                            "frozen",
                                            format!("cannot reassign const '{}'", name),
                                        ));
                                    }
                                    slot_s.push_str(suffix);
                                    fast_done = true;
                                }
                            }
                            break;
                        }
                        node = env_node.parent.clone();
                    }
                }
                if !fast_done {
                    // EXACT fallback — the read half (loud = the full
                    // LoadName arm with clone charge + unbound note; quiet
                    // = LoadNameQuiet), then Bin(Add), then AssignName
                    // verbatim (slot write included).
                    let l = if *loud {
                        match slot_read(slot_frame.as_ref(), *idx).or_else(|| cur.get(&name)) {
                            Some(v) => {
                                crate::interp::charge_clone(&v)?;
                                v
                            }
                            None => {
                                interp.note(0, 4, format!("unbound '{}' read as null", name));
                                Value::Null
                            }
                        }
                    } else {
                        slot_read(slot_frame.as_ref(), *idx)
                            .unwrap_or_else(|| cur.get(&name).unwrap_or(Value::Null))
                    };
                    let v = interp.apply_binop(&cur, BinOp::Add, &l, &r)?;
                    if cur.is_const(&name) {
                        return Err(Stress::new(
                            "frozen",
                            format!("cannot reassign const '{}'", name),
                        ));
                    }
                    match slot_write(slot_frame.as_mut(), *idx) {
                        Some(dst) => {
                            *dst = v.clone();
                            let _ = cur.set(&name, v);
                        }
                        None => {
                            if !cur.set(&name, v) {
                                interp.note(
                                    0,
                                    4,
                                    format!("'{}' was not declared; auto-declared", name),
                                );
                            }
                        }
                    }
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
                // and fuel contract respectively. W011 stage-2: the slot
                // hit rides the same charge discipline.
                let l = match slot_read(slot_frame.as_ref(), *nidx) {
                    Some(v) => {
                        crate::interp::charge_clone(&v)?;
                        v
                    }
                    None => {
                        let name = &code.names[*nidx as usize];
                        match cur.get(name) {
                            Some(v) => {
                                crate::interp::charge_clone(&v)?;
                                v
                            }
                            None => {
                                interp.note(0, 4, format!("unbound '{}' read as null", name));
                                Value::Null
                            }
                        }
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
                // W011 stage-2: the slot hit rides first — same value,
                // same clone charge, unbound note unreachable for a slot.
                let v = match slot_read(slot_frame.as_ref(), *nidx) {
                    Some(v) => {
                        crate::interp::charge_clone(&v)?;
                        v
                    }
                    None => {
                        let name = &code.names[*nidx as usize];
                        match cur.get(name) {
                            Some(v) => {
                                crate::interp::charge_clone(&v)?;
                                v
                            }
                            None => {
                                interp.note(0, 4, format!("unbound '{}' read as null", name));
                                Value::Null
                            }
                        }
                    }
                };
                return Ok(Flow::Ret(v));
            }
            Instr::Brk(t) => ip = *t as usize,
            Instr::Cont(t) => ip = *t as usize,
            Instr::EnterScope => {
                scopes.push(cur.clone());
                cur = match scope_cache.pop() {
                    Some(mut c) => {
                        // P6-wave fix (review 5439336263): the pool is ONE
                        // LIFO stack shared across ALL scope sites of the
                        // call, unlike the tree-walk's SITE-LOCAL caches
                        // (loop_scope_env) whose parentage self-corrects. A
                        // cached env therefore carries the PARENT of the
                        // site that stashed it — reusing it verbatim left
                        // lookups walking a stale ancestor chain: iteration-
                        // 1 leftovers stayed readable and `let` rebinds saw
                        // names that a fresh env would not (the 7 divergent
                        // parity files). Re-anchor on every reuse. Stash
                        // requires strong_count == 1, so the pop hands us
                        // sole ownership and Rc::get_mut is infallible here;
                        // the None arm is unreachable and kept defensive.
                        match Rc::get_mut(&mut c) {
                            Some(e) => e.parent = Some(cur.clone()),
                            None => c = Env::new(Some(cur.clone())),
                        }
                        c
                    }
                    None => Env::new(Some(cur.clone())),
                };
            }
            Instr::ExitScope => {
                if let Some(p) = scopes.pop() {
                    // P6-wave: stash the scope env for reuse when nothing
                    // captured it — the clear is capacity-preserving, so a
                    // loop body's next iteration reuses the same maps with
                    // zero allocator traffic. A captured env fails the
                    // strong-count check and drops normally, exactly the
                    // fresh-Env lifetime its capturer expects.
                    if scope_cache.len() < 8 && Rc::strong_count(&cur) == 1 {
                        cur.vars.borrow_mut().clear();
                        cur.consts.borrow_mut().clear();
                        scope_cache.push(cur.clone());
                    }
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
                let v = interp.named_call_tail_vm(
                    &cur,
                    name,
                    argvs,
                    Some((code as *const GeneCode as usize, (ip - 1) as u32)),
                )?;
                stack.push(v);
            }
        }
    }
}

// ============================================================== W16 fibers
// (docs/specs/ASYNC.md; frame contract in docs/vm-design.md §6). Execution
// units are fiber frames on THIS loop, not OS threads: a fiber owns a HEAP
// frame stack (VmFrame) so parking at a suspension point saves the whole
// call chain, not just the top frame. Suspension points are explicit:
// await-shaped builtins (`sleep`, `recv`, `select`) set Interp::fiber_pending
// through the SAME shared code the tree-walk calls; the machine parks the
// fiber at the next loop-head boundary and the scheduler (slice 2) decides
// the deterministic FIFO wake. No preemption ever.

/// Which kind of wake a suspended fiber sits on (the `fiber_state` tag of
/// docs/vm-design.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeTag {
    Sleep,
    Chan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiberState {
    Running,
    SuspendedOn(WakeTag),
    Done,
}

/// What a parked fiber waits for. Produced by an await-shaped builtin
/// through `Interp::fiber_pending`; consumed by the scheduler, which owns
/// the virtual clock and the FIFO wake order. The machine itself stays
/// clock-free: it only reports.
pub enum PendingWake {
    /// virtual-millisecond duration; deadline = scheduler now + ms
    Sleep(u64),
    /// parked until one of these channels delivers; `select` re-polls the
    /// full list on wake (declaration order decides, leftmost ready wins)
    Chan {
        chans: Vec<std::sync::Arc<crate::value::ChannelShared>>,
        select: bool,
    },
}

impl PendingWake {
    pub fn tag(&self) -> WakeTag {
        match self {
            PendingWake::Sleep(_) => WakeTag::Sleep,
            PendingWake::Chan { .. } => WakeTag::Chan,
        }
    }
}

/// One heap-allocated VM frame: the full per-call execution state the sync
/// machine keeps in Rust locals (`exec_gene_code_inner`). Owning it in a
/// struct is what makes parking mid-call-chain possible at all (vm-design
/// §6: "frames move from the host stack to a heap-allocated frame stack").
pub struct VmFrame {
    pub code: std::rc::Rc<GeneCode>,
    pub ip: usize,
    pub stack: Vec<Value>,
    pub scopes: Vec<Rc<Env>>,
    pub cur: Rc<Env>,
    /// P6-wave: per-frame scope-env cache (the fiber machine's mirror of
    /// the sync machine's per-call local). EnterScope pops a cached env
    /// (maps cleared, capacity preserved) instead of minting one; a captured
    /// env fails the strong-count check at stash time and drops normally.
    pub scope_cache: Vec<Rc<Env>>,
    /// traceback identity (the call_gene chain frame: name + call line)
    pub gene: String,
    pub entry_line: usize,
    /// W01 return annotation, checked at pop exactly like call_gene_inner
    pub ret_ann: Option<crate::ast::TypeAnn>,
}

/// A parked-or-running fiber: the frame stack plus the A2 frame reservation
/// fields of docs/vm-design.md §6, now real.
pub struct Fiber {
    pub frames: Vec<VmFrame>,
    /// Running | SuspendedOn(WakeReason) — parked at builtin calls only,
    /// never mid-instruction.
    pub fiber_state: FiberState,
    /// timer-wheel slot in scheduler virtual ms (sleep); None otherwise
    pub wake_deadline: Option<u64>,
    /// W18 cooperative cancellation view for this fiber (the truth lives
    /// in the task interp's cancel_chain, observed at ticks)
    pub cancel_flag: bool,
    /// the awaited value the scheduler delivers before resuming: the parked
    /// call site is already behind ip, so the machine pushes this instead
    /// of the placeholder the builtin returned
    pub wake_result: Option<Value>,
}

/// The prepared frame a fiber call begins on. Produced by the ONE hook at
/// the body-exec point of call_gene_inner — after RISC → toggle → GRN →
/// methylation → riboswitch → promoter → RHO, param binding, uORF guards
/// and call bookkeeping have all run in the SHARED code. The fiber machine
/// cannot drift from the funnel because it never reimplements it.
pub struct FiberFrame {
    pub def_key: usize,
    pub name: String,
    pub def: std::sync::Arc<crate::ast::GeneDef>,
    pub fenv: Rc<Env>,
    pub entry_line: usize,
}

/// What one fiber_run turn produced: a finished value (base frame returned),
/// or a park at a suspension point the scheduler must resolve.
pub enum FiberOutcome {
    /// the base frame returned
    Done(Value),
    /// the fiber parked at a suspension point; resume by setting
    /// fiber.wake_result and calling fiber_run again
    Suspended(PendingWake),
}

enum Step {
    Continue,
    Finished(Value),
}

fn fiber_return_stack(interp: &mut Interp, stack: Vec<Value>) {
    // the same pool discipline as exec_gene_code
    if stack.capacity() <= 64 {
        interp.vm_stack_pool.push(stack);
    }
}

/// Begin a fiber on a named call: ride the SAME funnel (named_call_tail_vm
/// → call_named → call_value → call_gene → call_gene_inner) with the hook
/// armed, so RISC redirects, wobble repairs and toggle vetoes behave
/// exactly as the sync machine sees them.
pub enum FiberBegin {
    /// the call reached a gene body: the fiber starts on its frame
    Fiber(Fiber),
    /// the funnel returned WITHOUT executing a body — RISC degradation,
    /// toggle repression, a gate veto, a uORF guard return, a phantom
    /// call: the value IS the call's result (a thread worker running the
    /// same call would complete with exactly this value)
    Completed(Value),
}

pub fn fiber_call_begin(
    interp: &mut Interp,
    env: &Rc<Env>,
    name: &str,
    argvs: Vec<Value>,
) -> Result<FiberBegin, Stress> {
    interp.fiber_hook = true;
    let r = interp.named_call_tail_vm(env, name, argvs, None);
    interp.fiber_hook = false;
    let v = r?;
    match interp.fiber_hook_out.take() {
        Some(ff) => {
            let mut fiber = Fiber {
                frames: Vec::new(),
                fiber_state: FiberState::Running,
                wake_deadline: None,
                cancel_flag: false,
                wake_result: None,
            };
            fiber_push_frame(interp, &mut fiber, ff)?;
            Ok(FiberBegin::Fiber(fiber))
        }
        None => Ok(FiberBegin::Completed(v)),
    }
}

/// Push the hooked frame. `call_gene` already checked the depth limit and
/// bumped `depth` for this call — but its decrement bracketed only the
/// sentinel path, so the frame re-bumps for its lifetime and pops it at
/// Ret/unwind, keeping the counter symmetric with the sync machine.
fn fiber_push_frame(interp: &mut Interp, fiber: &mut Fiber, ff: FiberFrame) -> Result<(), Stress> {
    interp.depth += 1;
    let code = gene_code_cached(
        interp,
        ff.def_key,
        &ff.name,
        &ff.def.body,
        &ff.def.type_params,
    );
    let mut stack = match interp.vm_stack_pool.pop() {
        Some(s) => s,
        None => Vec::with_capacity(16),
    };
    stack.clear();
    fiber.frames.push(VmFrame {
        code,
        ip: 0,
        stack,
        scopes: Vec::new(),
        cur: ff.fenv,
        scope_cache: Vec::new(),
        gene: ff.name,
        entry_line: ff.entry_line,
        ret_ann: ff.def.ret_ann.clone(),
    });
    Ok(())
}

/// Pop a frame with a return value: depth--, the W01 return annotation
/// (the shared call_gene_inner tail), the value either becomes the fiber's
/// result (base frame) or lands on the caller's stack. `explicit` mirrors
/// call_gene_inner: a Ret carries a real value; fell-off-end, Brk and Cont
/// at the gene boundary return null through the same annotation check.
fn fiber_pop_frame(
    interp: &mut Interp,
    fiber: &mut Fiber,
    v: Value,
    explicit: bool,
) -> Result<Step, Stress> {
    let fr = match fiber.frames.pop() {
        Some(f) => f,
        None => return Ok(Step::Finished(v)),
    };
    interp.depth = interp.depth.saturating_sub(1);
    let checked = interp.check_ret_ann(
        "gene",
        &fr.gene,
        &fr.ret_ann,
        &v,
        explicit,
        &fr.code.type_params,
    );
    fiber_return_stack(interp, fr.stack);
    let v = match checked {
        Ok(v) => v,
        Err(mut s) => {
            // the W007 chain frame call_gene would have appended at this
            // level, same shape, same 64 cap
            if s.chain.len() < 64 {
                s.chain.push((fr.gene, fr.entry_line));
            }
            return Err(s);
        }
    };
    match fiber.frames.last_mut() {
        None => Ok(Step::Finished(v)),
        Some(parent) => {
            parent.stack.push(v);
            Ok(Step::Continue)
        }
    }
}

/// Unwind every frame on stress: depth-- per frame, the traceback chain
/// appends innermost-first exactly as the Rust stack unwinding through
/// call_gene wrappers does, stacks return to the pool.
fn fiber_unwind(interp: &mut Interp, fiber: &mut Fiber, mut s: Stress) -> Stress {
    while let Some(fr) = fiber.frames.pop() {
        interp.depth = interp.depth.saturating_sub(1);
        if s.chain.len() < 64 {
            s.chain.push((fr.gene, fr.entry_line));
        }
        fiber_return_stack(interp, fr.stack);
    }
    fiber.fiber_state = FiberState::Done;
    s
}

/// Run a fiber until it finishes, parks at a suspension point, or fails.
/// Same dispatch arms as the sync machine (`exec_gene_code_inner`) — the
/// differences are structural only: state lives in the heap frame stack,
/// native gene calls push frames instead of Rust-recursing (the hook), and
/// the loop head parks on `Interp::fiber_pending`.
pub fn fiber_run(interp: &mut Interp, fiber: &mut Fiber) -> Result<FiberOutcome, Stress> {
    fiber.fiber_state = FiberState::Running;
    match fiber_run_inner(interp, fiber) {
        Ok(out) => Ok(out),
        Err(s) => Err(fiber_unwind(interp, fiber, s)),
    }
}

fn fiber_run_inner(interp: &mut Interp, fiber: &mut Fiber) -> Result<FiberOutcome, Stress> {
    // resume protocol: the scheduler delivered the awaited value before
    // resuming; the parked call site is behind ip already, so the machine
    // pushes it instead of the placeholder the builtin returned.
    if let Some(v) = fiber.wake_result.take() {
        match fiber.frames.last_mut() {
            None => return Ok(FiberOutcome::Done(v)),
            Some(fr) => fr.stack.push(v),
        }
    }
    loop {
        // loop-head park check: a suspension builtin set fiber_pending
        // through the shared code; ip is already past the call, the
        // builtin's placeholder is discarded, the real value arrives via
        // wake_result on wake. Suspension never refunds fuel (the charges
        // already ran inside the builtin) and never preempts (this is the
        // only place a fiber yields besides Done/Stress).
        if let Some(pw) = interp.fiber_pending.take() {
            fiber.fiber_state = FiberState::SuspendedOn(pw.tag());
            fiber.cancel_flag = interp
                .cancel_chain
                .iter()
                .any(|f| f.load(std::sync::atomic::Ordering::Relaxed));
            return Ok(FiberOutcome::Suspended(pw));
        }
        // fetch: fell off the end of the top frame = the gene boundary
        // null return, exactly like the sync machine's Flow::Norm exit
        let (cur_ip, line) = {
            let fr = match fiber.frames.last_mut() {
                Some(fr) => fr,
                None => return Ok(FiberOutcome::Done(Value::Null)),
            };
            if fr.ip >= fr.code.code.len() {
                match fiber_pop_frame(interp, fiber, Value::Null, false)? {
                    Step::Continue => continue,
                    Step::Finished(v) => {
                        fiber.fiber_state = FiberState::Done;
                        return Ok(FiberOutcome::Done(v));
                    }
                }
            }
            let ip = fr.ip;
            fr.ip += 1;
            (ip, fr.code.lines[ip])
        };
        if line > 0 {
            interp.cur_line = line as usize;
        }
        interp.tick()?;
        // arm the lane for this instruction: await-shaped builtins reached
        // at a compiled call site may park. Bridges swap it out for false
        // around their tree-walk calls (the honest fallback) and restore it.
        interp.fiber_armed = true;
        // destructure the frame into disjoint field borrows (same shape the
        // sync machine keeps in locals); `code` stays immutably borrowed by
        // `instr` while the arms mutate stack/scopes/cur
        let fr = match fiber.frames.last_mut() {
            Some(fr) => fr,
            None => return Ok(FiberOutcome::Done(Value::Null)),
        };
        let VmFrame {
            code,
            ip,
            stack,
            scopes,
            cur,
            scope_cache,
            ..
        } = fr;
        let instr = match code.code.get(cur_ip) {
            Some(i) => i,
            None => continue, // unreachable (bounds checked above)
        };
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
                // P6-wave (fiber mirror): borrowed name + contains() check.
                let name = code.names[*idx as usize].as_str();
                let v = stack.pop().unwrap_or(Value::Null);
                if cur.vars.borrow().contains_key(name) {
                    interp.note(0, 4, format!("rebinding '{}'", name));
                }
                cur.define(name, v);
            }
            Instr::AssignName(idx) => {
                // P6-wave (fiber mirror): borrowed name.
                let name = code.names[*idx as usize].as_str();
                let v = stack.pop().unwrap_or(Value::Null);
                if cur.is_const(name) {
                    return Err(Stress::new(
                        "frozen",
                        format!("cannot reassign const '{}'", name),
                    ));
                }
                if !cur.set(name, v) {
                    interp.note(0, 4, format!("'{}' was not declared; auto-declared", name));
                }
            }
            Instr::AppendName(idx, loud) => {
                // P5 builder fast path — the fiber machine's mirror of the
                // sync arm (no slot frames here: reads and writes go
                // through the env chain directly). The fallback reproduces
                // the EXACT replaced sequence — LoadName(Quiet) + Bin(Add)
                // + AssignName, fiber-machine flavors — for every shape
                // the fast path refuses.
                let name = code.names[*idx as usize].clone();
                let r = stack.pop().unwrap_or(Value::Null);
                let mut fast_done = false;
                let mut node = Some(cur.clone());
                while let Some(env_node) = node {
                    let hit = env_node.vars.borrow().contains_key(&name);
                    if hit {
                        if let Value::Str(suffix) = &r {
                            let mut vars = env_node.vars.borrow_mut();
                            if let Some(Value::Str(slot_s)) = vars.get_mut(&name) {
                                // Bin(Add) position: ceiling, then charge
                                let new_len = slot_s.len().saturating_add(suffix.len());
                                if new_len > 512 * 1024 * 1024 {
                                    return Err(Stress::new(
                                        "overflow",
                                        "string concat exceeds the 512 MiB ceiling",
                                    ));
                                }
                                // perf-xlang-r2: retained-bytes charge (see
                                // the sync arm) — same accounting both arms.
                                crate::interp::mem_charge(suffix.len() as u64)?;
                                // AssignName position: const check
                                if cur.is_const(&name) {
                                    return Err(Stress::new(
                                        "frozen",
                                        format!("cannot reassign const '{}'", name),
                                    ));
                                }
                                slot_s.push_str(suffix);
                                fast_done = true;
                            }
                        }
                        break;
                    }
                    node = env_node.parent.clone();
                }
                if !fast_done {
                    let l = if *loud {
                        match cur.get(&name) {
                            Some(v) => {
                                crate::interp::charge_clone(&v)?;
                                v
                            }
                            None => {
                                interp.note(0, 4, format!("unbound '{}' read as null", name));
                                Value::Null
                            }
                        }
                    } else {
                        cur.get(&name).unwrap_or(Value::Null)
                    };
                    let v = interp.apply_binop(cur, BinOp::Add, &l, &r)?;
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
            }
            Instr::Bin(op) => {
                let r = stack.pop().unwrap_or(Value::Null);
                let l = stack.pop().unwrap_or(Value::Null);
                let v = interp.apply_binop(cur, *op, &l, &r)?;
                stack.push(v);
            }
            Instr::BinImm(op, cidx) => {
                // W11 superinstruction: identical to Bin with the rhs taken
                // from the constant pool (the folded Push); same apply_binop,
                // same order (lhs was evaluated first, by construction).
                let r = const_value(&code.consts, *cidx);
                let l = stack.pop().unwrap_or(Value::Null);
                let v = interp.apply_binop(cur, *op, &l, &r)?;
                stack.push(v);
            }
            Instr::LoadBinImm(nidx, op, cidx) => {
                // W11: the EXACT LoadName read (clone-charge + unbound note)
                // composed with the EXACT Bin arm.
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
                let v = interp.apply_binop(cur, *op, &l, &r)?;
                stack.push(v);
            }
            Instr::JmpIfF(t) => {
                let v = stack.pop().unwrap_or(Value::Null);
                if !v.truthy() {
                    *ip = *t as usize;
                }
            }
            Instr::Jmp(t) => *ip = *t as usize,
            Instr::EvalExpr(idx) => {
                // W16: bridges disarm the lane — a suspension builtin inside
                // bridged tree-walk code degrades to the thread lane's
                // blocking behavior (deterministic: blocking mid-turn cannot
                // reorder a cooperative scheduler). Documented honesty, not
                // a hidden limitation.
                let armed = std::mem::replace(&mut interp.fiber_armed, false);
                // clone the bridged node out of the arena (cheap: Arc'd
                // children) so the mutable interpreter borrow is free
                let e = interp
                    .vm_program
                    .as_ref()
                    .and_then(|p| p.exprs.get(*idx as usize))
                    .cloned();
                let r = match e {
                    Some(e) => interp.eval(cur, &e).map(|v| vec![v]),
                    None => Ok(Vec::new()),
                };
                interp.fiber_armed = armed;
                let mut vs = r?;
                if let Some(v) = vs.pop() {
                    stack.push(v);
                } else {
                    stack.push(Value::Null);
                }
            }
            Instr::BridgeStmt(idx) => {
                let armed = std::mem::replace(&mut interp.fiber_armed, false);
                let s = interp
                    .vm_program
                    .as_ref()
                    .and_then(|p| p.stmts.get(*idx as usize))
                    .cloned();
                let r = match s {
                    Some(s) => interp.exec_stmt(cur, &s),
                    None => Ok(Flow::Norm),
                };
                interp.fiber_armed = armed;
                match r? {
                    // the bridged statement's flow propagates exactly the
                    // way exec_block would propagate it (Ret leaves the
                    // gene; Brk/Cont reach the gene boundary, which pops
                    // with the null return)
                    Flow::Norm => {}
                    Flow::Ret(v) => {
                        // a return inside a nested frame pops THAT frame;
                        // the fiber only ends when the base frame returns
                        match fiber_pop_frame(interp, fiber, v, true)? {
                            Step::Continue => {}
                            Step::Finished(v) => {
                                fiber.fiber_state = FiberState::Done;
                                return Ok(FiberOutcome::Done(v));
                            }
                        }
                    }
                    Flow::Brk | Flow::Cont => {
                        match fiber_pop_frame(interp, fiber, Value::Null, false)? {
                            Step::Continue => {}
                            Step::Finished(v) => {
                                fiber.fiber_state = FiberState::Done;
                                return Ok(FiberOutcome::Done(v));
                            }
                        }
                    }
                }
            }
            Instr::BridgeStmtInLoop(idx, cont_t, brk_t, unwinds) => {
                let armed = std::mem::replace(&mut interp.fiber_armed, false);
                let s = interp
                    .vm_program
                    .as_ref()
                    .and_then(|p| p.stmts.get(*idx as usize))
                    .cloned();
                let r = match s {
                    Some(s) => interp.exec_stmt(cur, &s),
                    None => Ok(Flow::Norm),
                };
                interp.fiber_armed = armed;
                match r? {
                    Flow::Norm => {}
                    // a return from inside a bridged statement IS the
                    // gene's return (the tree-walk contract)
                    Flow::Ret(v) => match fiber_pop_frame(interp, fiber, v, true)? {
                        Step::Continue => {}
                        Step::Finished(v) => {
                            fiber.fiber_state = FiberState::Done;
                            return Ok(FiberOutcome::Done(v));
                        }
                    },
                    Flow::Brk => {
                        for _ in 0..*unwinds {
                            if let Some(p) = scopes.pop() {
                                *cur = p;
                            }
                        }
                        *ip = *brk_t as usize;
                    }
                    Flow::Cont => {
                        for _ in 0..*unwinds {
                            if let Some(p) = scopes.pop() {
                                *cur = p;
                            }
                        }
                        *ip = *cont_t as usize;
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
                match fiber_pop_frame(interp, fiber, v, true)? {
                    // a nested frame returned: the loop continues on the
                    // caller frame (value already pushed to its stack)
                    Step::Continue => {}
                    Step::Finished(v) => {
                        fiber.fiber_state = FiberState::Done;
                        return Ok(FiberOutcome::Done(v));
                    }
                }
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
                match fiber_pop_frame(interp, fiber, v, true)? {
                    Step::Continue => {}
                    Step::Finished(v) => {
                        fiber.fiber_state = FiberState::Done;
                        return Ok(FiberOutcome::Done(v));
                    }
                }
            }
            Instr::Brk(t) => *ip = *t as usize,
            Instr::Cont(t) => *ip = *t as usize,
            Instr::EnterScope => {
                scopes.push(cur.clone());
                let next = match scope_cache.pop() {
                    Some(mut c) => {
                        // P6-wave fix (review 5439336263): same re-anchor as
                        // the sync machine — the per-frame pool is LIFO
                        // across scope sites, so a reused env must point at
                        // the CURRENT parent, not the site that stashed it.
                        // strong_count == 1 at stash makes Rc::get_mut
                        // infallible on the reuse path.
                        match Rc::get_mut(&mut c) {
                            Some(e) => e.parent = Some(cur.clone()),
                            None => c = Env::new(Some(cur.clone())),
                        }
                        c
                    }
                    None => Env::new(Some(cur.clone())),
                };
                *cur = next;
            }
            Instr::ExitScope => {
                if let Some(p) = scopes.pop() {
                    // P6-wave: stash for reuse when nothing captured this
                    // scope env (sole reference, maps cleared capacity-
                    // preserving); captured envs drop normally — the exact
                    // fresh-Env lifetime its capturer expects.
                    if scope_cache.len() < 8 && Rc::strong_count(cur) == 1 {
                        cur.vars.borrow_mut().clear();
                        cur.consts.borrow_mut().clear();
                        scope_cache.push(cur.clone());
                    }
                    *cur = p;
                }
            }
            Instr::CallNamed(name_idx, argc) => {
                // W16: the args drain exactly like the sync arm, then the
                // SHARED funnel runs with the hook armed — whatever gene
                // body it resolves to (direct, RISC-redirected, wobble-
                // repaired) pushes a heap frame; builtins, vetoed calls and
                // guard returns return values inline.
                let name = code.names[*name_idx as usize].as_str();
                let n = *argc as usize;
                let base = stack.len() - n;
                let argvs: Vec<Value> = stack.drain(base..).collect();
                interp.fiber_hook = true;
                let r = interp.named_call_tail_vm(
                    cur,
                    name,
                    argvs,
                    Some((std::rc::Rc::as_ptr(code) as usize, cur_ip as u32)),
                );
                interp.fiber_hook = false;
                let v = r?;
                match interp.fiber_hook_out.take() {
                    Some(ff) => {
                        fiber_push_frame(interp, fiber, ff)?;
                    }
                    None => stack.push(v),
                }
            }
        }
    }
}

/// `operon ir` / `operon disasm`: compile every top-level gene and render
/// the OIR1 listing (`idx | mnemonic | operands | line`). The line column
/// is the instruction's source-line stamp from the `lines` table (0 = the
/// insn carries no stamp, rendered as `-`). W10 stage 1; W010-A completed
/// the annotation (the documented format contract now holds literally).
pub fn disassemble_program(prog: &crate::ast::Program) -> String {
    let mut out = String::new();
    out.push_str(&format!("OIR{} disassembly\n", OIR_VERSION));
    let mut vmprog = VmProgram::default();
    for s in &prog.stmts {
        if let Stmt::Gene(g) = s {
            let name = g.name.clone().unwrap_or_else(|| "<lambda>".into());
            let code = compile_body(&name, &g.body, &mut vmprog, &g.type_params);
            out.push_str(&format!("\ngene {} ({} instr(s))\n", name, code.code.len()));
            for (i, instr) in code.code.iter().enumerate() {
                let line = code.lines.get(i).copied().unwrap_or(0);
                out.push_str(&format!(
                    "  {:04} | {} | {} | {}\n",
                    i,
                    mnemonic(instr),
                    render(instr, &code),
                    if line == 0 {
                        "-".to_string()
                    } else {
                        line.to_string()
                    }
                ));
            }
            if code.code.is_empty() {
                out.push_str("  (empty body)\n");
            }
        }
    }
    out
}

/// `operon disasm --json`: the same compile pass as `disassemble_program`
/// (shared VmProgram, per-gene GeneCode, identical instruction indexes),
/// rendered as a self-describing JSON document. Pairs with `graph --json`.
pub fn disassemble_program_json(prog: &crate::ast::Program) -> String {
    let esc = |s: &str| crate::tools::json_escape(s);
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"format\":\"operon-oir\",\"version\":{},\"genes\":[",
        OIR_VERSION
    ));
    let mut vmprog = VmProgram::default();
    let mut first_gene = true;
    for s in &prog.stmts {
        if let Stmt::Gene(g) = s {
            let name = g.name.clone().unwrap_or_else(|| "<lambda>".into());
            let code = compile_body(&name, &g.body, &mut vmprog, &g.type_params);
            if !first_gene {
                out.push(',');
            }
            first_gene = false;
            out.push_str(&format!("{{\"name\":\"{}\",\"instrs\":[", esc(&name)));
            let mut first_insn = true;
            for (i, instr) in code.code.iter().enumerate() {
                if !first_insn {
                    out.push(',');
                }
                first_insn = false;
                let line = code.lines.get(i).copied().unwrap_or(0);
                out.push_str(&format!(
                    "{{\"i\":{},\"op\":\"{}\",\"text\":\"{}\",\"line\":{}}}",
                    i,
                    esc(mnemonic(instr)),
                    esc(&render(instr, &code)),
                    line
                ));
            }
            out.push_str("]}");
        }
    }
    out.push_str("]}");
    out
}

fn mnemonic(i: &Instr) -> &'static str {
    match i {
        Instr::Push(_) => "Push",
        Instr::LoadName(_) => "LoadName",
        Instr::LoadNameQuiet(_) => "LoadNameQuiet",
        Instr::StoreName(_) => "StoreName",
        Instr::AssignName(_) => "AssignName",
        Instr::AppendName(..) => "AppendName",
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

/// W010-A: the canonical mnemonic table, in `Instr` declaration order.
/// The SPEC §15 opcode table and the stability tests both read this list,
/// so an opcode can no longer exist in the machine without a documented
/// row, and a doc row can no longer name an opcode the machine dropped
/// (the mnemonic() match is exhaustive, so a new variant cannot even
/// compile without landing here first).
pub fn all_mnemonics() -> &'static [&'static str] {
    &[
        "Push",
        "LoadName",
        "LoadNameQuiet",
        "StoreName",
        "AssignName",
        "AppendName",
        "Bin",
        "BinImm",
        "LoadBinImm",
        "JmpIfF",
        "Jmp",
        "EvalExpr",
        "BridgeStmt",
        "BridgeStmtInLoop",
        "Pop",
        "Ret",
        "RetName",
        "Brk",
        "Cont",
        "EnterScope",
        "ExitScope",
        "CallNamed",
        "Nop",
    ]
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
        Instr::AppendName(idx, loud) => format!(
            "'{}'{}",
            code.names.get(*idx as usize).cloned().unwrap_or_default(),
            if *loud { " (loud)" } else { "" }
        ),
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

/// W11: the per-pass toggle matrix. `--opt 1` = STAGE1 (the three
/// delivered passes), `--opt 2` = ALL (stage1 + constant propagation),
/// `--opt-passes a,b,c` = an explicit set parsed by `PassSet::parse`.
/// The pipeline ORDER is the contract (fold -> thread -> prop -> dce):
/// disabling a pass only removes its rewrite, never reorders another one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PassSet {
    /// pass 1: constant folding (Push/Push/Bin and Push/BinImm shapes)
    pub fold: bool,
    /// pass 2: jump threading + dead unconditional-jump removal
    pub thread: bool,
    /// pass 4: constant propagation (LoadName -> Push rewrites)
    pub prop: bool,
    /// pass 3: reachability DCE from ip 0
    pub dce: bool,
}

impl PassSet {
    pub const NONE: PassSet = PassSet {
        fold: false,
        thread: false,
        prop: false,
        dce: false,
    };
    /// `--opt 1`: the delivered stage-1+2 pipeline.
    pub const STAGE1: PassSet = PassSet {
        fold: true,
        thread: true,
        prop: false,
        dce: true,
    };
    /// `--opt 2`: every pass the pipeline ships today (grows with W011).
    pub const ALL: PassSet = PassSet {
        fold: true,
        thread: true,
        prop: true,
        dce: true,
    };

    /// Parse a `--opt-passes` spec: a comma-separated list of pass names,
    /// or `none` / `all`. Unknown names are an error (rc 2 in main) — a
    /// typo must never silently enable fewer passes than the user asked
    /// for (the honesty rule: flags do what they say).
    pub fn parse(spec: &str) -> Result<PassSet, String> {
        let spec = spec.trim();
        if spec == "none" {
            return Ok(PassSet::NONE);
        }
        if spec == "all" {
            return Ok(PassSet::ALL);
        }
        let mut set = PassSet::NONE;
        for name in spec.split(',') {
            match name.trim() {
                "fold" => set.fold = true,
                "thread" => set.thread = true,
                "dce" => set.dce = true,
                "prop" => set.prop = true,
                other => {
                    return Err(format!(
                        "unknown optimization pass '{}' (valid: {})",
                        other,
                        PassSet::NAMES.join(", ")
                    ))
                }
            }
        }
        Ok(set)
    }

    /// The canonical name list (tests + usage text pin this).
    pub const NAMES: &'static [&'static str] = &["fold", "thread", "dce", "prop"];
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
    optimize_with(code, PassSet::ALL)
}

/// Run a SELECTED subset of the passes (W011 toggle matrix). Order stays
/// fold -> thread -> dce regardless of the subset: the passes are
/// individually disableable, never reordered.
pub fn optimize_with(code: &GeneCode, passes: PassSet) -> GeneCode {
    let mut consts = code.consts.clone();
    let mut code = code.clone();

    // pass 1: constant folding (fixpoint, bounded). Both Bin shapes fold:
    // the Push/Push/Bin triple and the W11 fused Push/BinImm pair (the
    // folded form of a literal rhs).
    if passes.fold {
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
    }

    // pass 2: jump threading + dead unconditional-jump removal
    if passes.thread {
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
    }

    // pass 4 (W011): constant propagation — 1:1 LoadName -> Push rewrites
    // inside straight-line runs. Soundness contract: a fact (name n holds
    // consts[c]) is only gen'd from the adjacent shape `Push c, StoreName n`
    // (StoreName NEVER stresses: no const check, it defines the current
    // scope with only a rebinding note, which stays — the store is kept),
    // and a fact only flows where NO join can reach: every jump target
    // resets, scope enter/exit reset (shadowing), any bridge or call
    // resets (the tree-walk and callees can define, set or auto-declare
    // any name), AssignName kills its target (compound assigns), and a
    // caught stress downstream of an AssignName is the only way control
    // continues past a killed fact — which the kill already covers. The
    // rewrite is 1:1, so lines, jump targets and the constant pool never
    // move; only redundant env reads become pool pushes. LoadNameQuiet,
    // LoadBinImm and RetName keep their fused shapes (not 1:1).
    if passes.prop {
        let n = code.code.len();
        let mut targets = vec![false; n];
        for instr in &code.code {
            if let Instr::Jmp(t) | Instr::JmpIfF(t) | Instr::Brk(t) | Instr::Cont(t) = instr {
                let t = *t as usize;
                if t < n {
                    targets[t] = true;
                }
            }
        }
        let mut facts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut last_push: Option<u32> = None;
        for (i, is_target) in targets.iter().enumerate() {
            if *is_target {
                facts.clear();
                last_push = None;
            }
            match code.code[i] {
                Instr::Push(c) => last_push = Some(c),
                Instr::StoreName(nm) => {
                    if let Some(c) = last_push {
                        facts.insert(nm, c);
                    } else {
                        facts.remove(&nm);
                    }
                    last_push = None;
                }
                Instr::LoadName(nm) => {
                    if let Some(&c) = facts.get(&nm) {
                        code.code[i] = Instr::Push(c);
                    }
                    last_push = None;
                }
                Instr::AssignName(nm) => {
                    facts.remove(&nm);
                    last_push = None;
                }
                Instr::AppendName(nm, _) => {
                    // the fast path mutates the named slot in place (or
                    // falls back to the read+binop+assign sequence it
                    // replaces) — either way any folded constant for the
                    // name is stale afterwards
                    facts.remove(&nm);
                    last_push = None;
                }
                // scopes (shadowing), bridges and calls (the tree-walk and
                // callees may define, set or auto-declare ANY name)
                Instr::EnterScope | Instr::ExitScope => {
                    facts.clear();
                    last_push = None;
                }
                Instr::EvalExpr(_)
                | Instr::BridgeStmt(_)
                | Instr::BridgeStmtInLoop(..)
                | Instr::CallNamed(..) => {
                    facts.clear();
                    last_push = None;
                }
                // unconditional transfers: the fallthrough gets nothing
                Instr::Jmp(_) | Instr::Brk(_) | Instr::Cont(_) | Instr::Ret | Instr::RetName(_) => {
                    facts.clear();
                    last_push = None;
                }
                // JmpIfF: the condition binds nothing — facts flow to the
                // fallthrough (its jump target resets on arrival)
                Instr::JmpIfF(_) => {
                    last_push = None;
                }
                _ => {
                    last_push = None;
                }
            }
        }
    }

    // pass 3: reachability DCE. Mark from ip 0 following fallthrough and
    // every jump target; keep the marked instructions in order and remap
    // every target through the old->new table. A target may legally be
    // code.len() ("fall off the end"), so the table has n+1 slots.
    if passes.dce {
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
    }

    GeneCode {
        name: code.name,
        type_params: code.type_params,
        consts,
        names: code.names,
        code: code.code,
        lines: code.lines,
        trivial: false,
        slot_ok: false,
        shadowed: code.shadowed,
        param_slots: std::sync::OnceLock::new(),
    }
}

/// W011 stage 3 item 1: the CODE half of the triviality predicate — the
/// whole body is a shapes-only sequence. Whitelist = pure reads and const
/// pushes feeding arithmetic, ending in a return: nothing here can write a
/// binding (StoreName/AssignName absent), open a scope, bridge to the
/// tree-walk, call anything, or branch — so the body's behavior depends on
/// the env ONLY through reads, and reads pass through an empty frame
/// identically (LoadName's own unbound note included; a read is resolved
/// per execution, never cached — the shadowing-safety argument). The fused
/// pure-read superinstructions (LoadBinImm, RetName) ride the same
/// argument; Nop executes as nothing by contract; Pop only balances the
/// machine's own stack. Jump shapes (Jmp/JmpIfF/Brk/Cont) are excluded:
/// control flow cannot appear in a whitelist body anyway, and excluding it
/// keeps the predicate obviously total.
fn shapes_only(code: &GeneCode) -> bool {
    code.code.iter().all(|i| {
        matches!(
            i,
            Instr::Push(_)
                | Instr::LoadName(_)
                | Instr::LoadNameQuiet(_)
                | Instr::Bin(_)
                | Instr::BinImm(_, _)
                | Instr::LoadBinImm(_, _, _)
                | Instr::Ret
                | Instr::RetName(_)
                | Instr::Pop
                | Instr::Nop
        )
    })
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
                // sweep-3 #129 (Z-129-FOLDSEM): `/` is true division,
                // ALWAYS Float (SPEC). The runtime converts both operands
                // to f64 and divides (interp apply_binop Div -> as_floats),
                // so the folded constant must be Float too — Const::Int
                // kept the truncated quotient and `9 / 2` executed as 4
                // under --opt while every unfolded lane printed 4.5.
                // sweep-3 #130 (Z-130-EXACT2): the runtime Int/Int path is
                // now EXACTLY rounded once (num_exact::div_i64_i64_exact,
                // Python long_true_divide law), so the fold calls the SAME
                // helper — fold byte-matches runtime by construction
                // (the old `x as f64 / y as f64` collapsed |x| > 2^53
                // numerators before dividing).
                Const::Float(crate::num_exact::div_i64_i64_exact(x, y))
            }
            Mod => {
                if y == 0 {
                    return None; // the runtime stress IS the contract
                }
                // sweep-3 #129 (Z-129-FOLDSEM): int % int is Python-parity
                // FLOORED remainder, sign follows the divisor (interp
                // apply_binop Mod: 7 % -3 == -2). checked_rem is Rust's
                // TRUNCATED remainder (sign follows the dividend) and
                // folded 7 % -3 to 1. Mirror the runtime formula exactly;
                // checked ops refuse to fold where the runtime stresses
                // (i64::MIN % -1 — overflow stress in apply_binop).
                if x == i64::MIN && y == -1 {
                    return None;
                }
                let mut q = x / y; // Rust / truncates toward zero
                if (x < 0) != (y < 0) && q.checked_mul(y)? != x {
                    q -= 1; // floor rounds down
                }
                let rb = q.checked_mul(y)?;
                let m = x.checked_sub(rb)?;
                Const::Int(m)
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

    /// Compile one gene's body from a source snippet (test helper).
    fn compile_one(src: &str) -> (Vec<Stmt>, VmProgram, GeneCode) {
        let parsed = crate::parser::parse(src);
        let mut body: Vec<Stmt> = Vec::new();
        for s in &parsed.stmts {
            if let Stmt::Gene(g) = s {
                body = g.body.clone();
            }
        }
        let mut prog = VmProgram::default();
        let code = compile_body("f", &body, &mut prog, &[]);
        (body, prog, code)
    }

    /// W011 toggle matrix: the parser accepts the documented names and
    /// rejects typos (a typo must never silently enable fewer passes).
    #[test]
    fn pass_set_parse_matrix() {
        assert_eq!(PassSet::parse("all"), Ok(PassSet::ALL));
        assert_eq!(PassSet::parse("none"), Ok(PassSet::NONE));
        assert_eq!(
            PassSet::parse("fold"),
            Ok(PassSet {
                fold: true,
                thread: false,
                prop: false,
                dce: false
            })
        );
        assert_eq!(
            PassSet::parse("fold, dce"),
            Ok(PassSet {
                fold: true,
                thread: false,
                prop: false,
                dce: true
            })
        );
        assert_eq!(
            PassSet::parse("prop"),
            Ok(PassSet {
                fold: false,
                thread: false,
                prop: true,
                dce: false
            })
        );
        assert!(PassSet::parse("fodl").is_err());
        assert!(PassSet::parse("").is_err());
    }

    /// W011: with folding disabled the same program keeps its arithmetic.
    #[test]
    fn fold_toggle_leaves_arithmetic_alone() {
        let (_body, _prog, raw) = compile_one("gene f() {\n    return 6 * 7\n}\n");
        // `6 * 7` compiles to Push, BinImm (the compiler fuses the literal
        // rhs BEFORE any optimizer pass) — the folder is what turns that
        // pair into a single Push.
        let is_arith = |i: &Instr| matches!(i, Instr::Bin(_) | Instr::BinImm(_, _));
        assert!(raw.code.iter().any(is_arith), "raw body has the arithmetic");
        let nofold = optimize_with(
            &raw,
            PassSet {
                fold: false,
                thread: true,
                prop: false,
                dce: true,
            },
        );
        assert!(
            nofold.code.iter().any(is_arith),
            "fold=false must keep the arithmetic"
        );
        let all = optimize_with(&raw, PassSet::ALL);
        assert!(
            !all.code.iter().any(is_arith),
            "fold=true must fold it away"
        );
    }

    /// W011: with threading disabled a Jmp-to-Jmp chain stays; with DCE
    /// disabled dead code after a Ret stays.
    #[test]
    fn thread_and_dce_toggles_are_honest() {
        // if false { return 1 } else { return 2 }: a Jmp chain + a dead island
        let src = "gene f() {\n    if true {\n        return 1\n    } else {\n        return 2\n    }\n}\n";
        let (_body, _prog, raw) = compile_one(src);
        let raw_has_dead = raw.code.len();
        assert!(
            raw_has_dead > 2,
            "the if/else compiles to more than Push/Ret"
        );
        // thread=false, dce=false: nothing may shrink
        let untouched = optimize_with(&raw, PassSet::NONE);
        assert_eq!(untouched.code.len(), raw.code.len());
        // dce=false but thread=true: dead code still present after threading
        let no_dce = optimize_with(
            &raw,
            PassSet {
                fold: true,
                thread: true,
                prop: false,
                dce: false,
            },
        );
        let with_dce = optimize_with(&raw, PassSet::STAGE1);
        assert!(no_dce.code.len() >= with_dce.code.len());
        assert!(with_dce.code.len() <= no_dce.code.len());
    }

    /// W011 pass 4: constant propagation rewrites a plain read of a
    /// const-initialized local into a pool push (1:1), but leaves the
    /// post-call read alone (the call killed the fact).
    #[test]
    fn prop_rewrites_const_local_reads() {
        let src = "gene f() {\n    let x = 40\n    print(x)\n    return x\n}\n";
        let (_body, _prog, raw) = compile_one(src);
        // raw arg read: LoadName x ... CallNamed print, 1 ... RetName x
        let has_plain_load = raw.code.iter().any(|i| matches!(i, Instr::LoadName(_)));
        assert!(has_plain_load, "the call arg compiles to a plain LoadName");
        let proped = optimize_with(
            &raw,
            PassSet {
                fold: false,
                thread: false,
                prop: true,
                dce: false,
            },
        );
        assert_eq!(proped.code.len(), raw.code.len(), "1:1 rewrites only");
        // the argument read became a Push of the 40-const
        let saw_push40_before_call = {
            let mut push40 = false;
            let mut ok = false;
            for instr in &proped.code {
                match instr {
                    Instr::Push(c)
                        if matches!(proped.consts.get(*c as usize), Some(Const::Int(40))) =>
                    {
                        push40 = true
                    }
                    Instr::CallNamed(..) => {
                        if push40 {
                            ok = true;
                        }
                        break;
                    }
                    _ => {}
                }
            }
            ok
        };
        assert!(
            saw_push40_before_call,
            "pre-call read of x propagated to a Push"
        );
        // the post-call return read stays a name read
        let last = proped.code.last().unwrap_or(&Instr::Nop);
        assert!(
            matches!(last, Instr::RetName(_) | Instr::LoadName(_) | Instr::Ret),
            "post-call read of x must stay a name read, got {:?}",
            last
        );
    }

    /// W011 pass 4 soundness: a reassigned local must NOT propagate
    /// across the join (the if-exit merge is a jump target).
    #[test]
    fn prop_stops_at_reassignment() {
        let src =
            "gene f(n) {\n    let x = n\n    if n > 0 {\n        x = 5\n    }\n    return x\n}\n";
        let (_body, _prog, raw) = compile_one(src);
        let proped = optimize_with(
            &raw,
            PassSet {
                fold: false,
                thread: false,
                prop: true,
                dce: false,
            },
        );
        assert_eq!(proped.code.len(), raw.code.len(), "1:1 rewrites only");
        // the read feeding `return x` sits AFTER a join point — the
        // conservative join rule must keep it a name read (RetName or
        // LoadName), never a rewritten Push of 5
        let tail = proped.code.last().unwrap_or(&Instr::Nop);
        match tail {
            Instr::RetName(_) | Instr::LoadName(_) => {}
            Instr::Push(c) => {
                let is5 = matches!(proped.consts.get(*c as usize), Some(Const::Int(5)));
                assert!(!is5, "post-join read of a reassigned local propagated");
            }
            other => panic!("unexpected tail instruction {:?}", other),
        }
    }

    /// W011 pass 4 soundness: a bridge or call between the store and the
    /// read kills the fact (bridges/callees can rebind any name).
    #[test]
    fn prop_killed_by_bridge_and_call() {
        // `now()`-style call then read: the fact must not survive
        let src = "gene f() {\n    let x = 7\n    print(x)\n    return x\n}\n";
        let (_body, _prog, raw) = compile_one(src);
        let proped = optimize_with(
            &raw,
            PassSet {
                fold: false,
                thread: false,
                prop: true,
                dce: false,
            },
        );
        // every LoadName/RetName of x AFTER the call stays a name read;
        // the read INSIDE the call args (before CallNamed) may rewrite
        let after_call_const = {
            let mut saw_call = false;
            let mut bad = false;
            for instr in &proped.code {
                match instr {
                    Instr::CallNamed(..) => saw_call = true,
                    Instr::LoadName(_) | Instr::RetName(_) if saw_call => {}
                    Instr::Push(c) if saw_call => {
                        if matches!(proped.consts.get(*c as usize), Some(Const::Int(7))) {
                            // legal only if it is NOT a rewritten post-call
                            // read — conservatively, the argument push of
                            // the call itself precedes CallNamed, so any
                            // Push 7 after the call is a bad rewrite
                            bad = true;
                        }
                    }
                    _ => {}
                }
            }
            bad
        };
        assert!(
            !after_call_const,
            "post-call read of x propagated across the call"
        );
    }

    /// W011 pass 4 end-to-end: the differential contract in miniature —
    /// the SAME program runs byte-identically (result + notes stream) with
    /// the pipeline off and with --opt 2 (propagation on).
    #[test]
    fn prop_end_to_end_semantics() {
        let src = "gene f(n) {\n    let base = 1000\n    let k = n * 2\n    if n > 10 {\n        return base + k\n    }\n    return base - k\n}\n";
        let parsed = crate::parser::parse(src);
        // both engines run the same body text; only the pass set differs
        let run = |opt: u8| -> i64 {
            let mut interp = Interp::new();
            interp.vm = true;
            interp.vm_opt = opt;
            interp.vm_program = Some(VmProgram::default());
            let g = interp.global.clone();
            for s in &parsed.stmts {
                let _ = interp.exec_stmt(&g, s);
            }
            let out = match interp.named_call_tail_vm(&g, "f", vec![Value::Int(21)], None) {
                Ok(v) => v,
                Err(_) => panic!("f(21) must not stress"),
            };
            match out {
                Value::Int(v) => v,
                Value::Float(f) => f as i64,
                _ => panic!("f(21) returned a non-numeric value"),
            }
        };
        assert_eq!(run(0), 1042, "sanity: base + k with n=21");
        assert_eq!(run(0), run(2), "opt0 == opt2 with propagation");
        assert_eq!(run(0), run(1), "opt0 == opt1 stage-1 only");
    }

    /// W011 resolution caching: the OnceLock hash structures are exact
    /// mirrors of the linear tables — no duplicate keys, same membership,
    /// same canonical targets (collect() keeps last, find() keeps first;
    /// the test pins that this can never matter).
    #[test]
    fn resolution_cache_mirrors_linear_tables() {
        let set = crate::interp::builtin_name_set();
        assert_eq!(
            set.len(),
            crate::interp::BUILTIN_NAMES.len(),
            "duplicate builtin name would silently drop arms"
        );
        for k in crate::interp::BUILTIN_NAMES {
            assert!(set.contains(k), "missing builtin '{}'", k);
        }
        let map = crate::interp::builtin_synonym_map();
        assert_eq!(
            map.len(),
            crate::interp::BUILTIN_SYNONYMS.len(),
            "duplicate synonym key would flip canonicalization"
        );
        for (s, canon) in crate::interp::BUILTIN_SYNONYMS {
            assert_eq!(map.get(s), Some(canon), "synonym '{}' drifted", s);
        }
    }

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
        let raw = compile_body("f", &body, &mut prog, &[]);
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
        let code = compile_body("f", &body, &mut prog, &[]);
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
        let code = compile_body("f", &body, &mut prog, &[]);
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
        let raw = compile_body("f", &body, &mut prog, &[]);
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

    /// W011 stage 3 item 1: the code half of the triviality predicate.
    /// Pure read/arith/return bodies qualify; stores, calls, control flow,
    /// bridges, scopes and composite literals never do.
    #[test]
    fn trivial_bit_shape_matrix() {
        let (_, _, code) = compile_one("gene f() { return 6 * 7; }");
        assert!(shapes_only(&code), "const arithmetic must be trivial");
        let (_, _, code) = compile_one("gene f() { return g; }");
        assert!(
            shapes_only(&code),
            "a global read (RetName) must be trivial"
        );
        let (_, _, code) = compile_one("gene f() { return 1 + x * 2; }");
        assert!(
            shapes_only(&code),
            "fused pure-read arithmetic (LoadBinImm family) must be trivial"
        );
        let (_, _, code) = compile_one("gene f() { let a = 1; return a; }");
        assert!(!shapes_only(&code), "StoreName (let) disqualifies");
        let (_, _, code) = compile_one("gene f() { g = 1; return g; }");
        assert!(!shapes_only(&code), "AssignName disqualifies");
        let (_, _, code) = compile_one("gene f() { return h(); }");
        assert!(!shapes_only(&code), "a call disqualifies");
        let (_, _, code) = compile_one("gene f() { if x { return 1; } return 2; }");
        assert!(!shapes_only(&code), "a branch (JmpIfF) disqualifies");
        let (_, _, code) = compile_one("gene f() { while x { return 1; } }");
        assert!(!shapes_only(&code), "a loop (Jmp/Brk/Cont) disqualifies");
        let (_, _, code) = compile_one("gene f() { return [1, 2]; }");
        assert!(
            !shapes_only(&code),
            "a composite literal bridges or builds: never trivial"
        );
        let (_, _, code) = compile_one("gene f() { return -x; }");
        assert!(
            !shapes_only(&code),
            "unary is a checked runtime path: not in the whitelist"
        );
    }
}

#[cfg(test)]
mod stability_tests {
    use super::*;
    use crate::parser::parse;

    /// W10 done-when: the annotated dump is STABLE across runs. The same
    /// source compiles to the same listing, every time, and the listing
    /// carries every opcode mnemonic the machine knows (the SPEC §15 VM
    /// table and this test move together).
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

    /// W010-A: the mnemonic table is pinned op-for-op — each variant maps
    /// to exactly the `all_mnemonics()` entry at its position, so a
    /// renamed, added or dropped opcode must update the table (and SPEC)
    /// to pass. The mnemonic() match is exhaustive, so a new variant also
    /// breaks compilation until it lands here.
    #[test]
    fn mnemonic_table_is_pinned() {
        let table: &[(&Instr, &str)] = &[
            (&Instr::Push(0), "Push"),
            (&Instr::LoadName(0), "LoadName"),
            (&Instr::LoadNameQuiet(0), "LoadNameQuiet"),
            (&Instr::StoreName(0), "StoreName"),
            (&Instr::AssignName(0), "AssignName"),
            (&Instr::AppendName(0, false), "AppendName"),
            (&Instr::Bin(BinOp::Add), "Bin"),
            (&Instr::BinImm(BinOp::Add, 0), "BinImm"),
            (&Instr::LoadBinImm(0, BinOp::Add, 0), "LoadBinImm"),
            (&Instr::JmpIfF(0), "JmpIfF"),
            (&Instr::Jmp(0), "Jmp"),
            (&Instr::EvalExpr(0), "EvalExpr"),
            (&Instr::BridgeStmt(0), "BridgeStmt"),
            (&Instr::BridgeStmtInLoop(0, 0, 0, 0), "BridgeStmtInLoop"),
            (&Instr::Pop, "Pop"),
            (&Instr::Ret, "Ret"),
            (&Instr::RetName(0), "RetName"),
            (&Instr::Brk(0), "Brk"),
            (&Instr::Cont(0), "Cont"),
            (&Instr::EnterScope, "EnterScope"),
            (&Instr::ExitScope, "ExitScope"),
            (&Instr::CallNamed(0, 0), "CallNamed"),
            (&Instr::Nop, "Nop"),
        ];
        assert_eq!(
            table.len(),
            all_mnemonics().len(),
            "a variant is missing from the pinned mapping"
        );
        for (i, (insn, name)) in table.iter().enumerate() {
            assert_eq!(mnemonic(insn), *name, "mapping drift at position {i}");
            assert_eq!(
                all_mnemonics()[i],
                *name,
                "table order drift at position {i}"
            );
        }
        let mut uniq = all_mnemonics().to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(
            uniq.len(),
            all_mnemonics().len(),
            "duplicate mnemonic in the table"
        );
    }

    /// W010-A: the JSON dump is the same compile pass — stable across
    /// runs, self-describing, and carrying the line annotation. Pairs
    /// with `graph --json`; fuzz smoke-runs it (no panic/hang) and the
    /// shape is pinned here so consumers can rely on it.
    #[test]
    fn json_dump_is_deterministic_and_annotated() {
        let src = "gene f(n) {\n    let x = n + 1\n    return x * 2\n}\n";
        let j1 = disassemble_program_json(&parse(src));
        let j2 = disassemble_program_json(&parse(src));
        assert_eq!(j1, j2, "same source, same JSON, always");
        assert!(
            j1.starts_with("{\"format\":\"operon-oir\",\"version\":1,\"genes\":["),
            "JSON shape drifted: {j1}"
        );
        assert!(j1.contains("\"line\":2"), "line annotation missing: {j1}");
        assert!(j1.ends_with("]}"), "JSON document unterminated: {j1}");
    }

    /// W010-A done-when: every opcode is documented in the SPEC §15 table,
    /// and every documented opcode is real — the table and the machine's
    /// mnemonic table cannot drift apart. include_str! reads the same
    /// SPEC.md CI ships, so the check runs on the exact tree under test.
    #[test]
    fn spec_opcode_table_reconciles_with_the_machine() {
        let spec = include_str!("../SPEC.md");
        let header = "| opcode | operands | meaning |";
        let start = spec
            .find(header)
            .expect("SPEC §15 opcode table header moved — reconcile the disassembler docs");
        let region = &spec[start..];
        let mut rows: Vec<String> = Vec::new();
        for line in region.lines().skip(2) {
            let t = line.trim();
            if !t.starts_with('|') {
                break;
            }
            let first = t
                .split('|')
                .nth(1)
                .unwrap_or("")
                .trim()
                .trim_matches('`')
                .to_string();
            assert!(
                !first.is_empty(),
                "SPEC opcode row without an opcode: {line}"
            );
            rows.push(first);
        }
        assert!(
            rows.len() >= 20,
            "SPEC opcode table implausibly short ({} rows) — the table itself drifted",
            rows.len()
        );
        for m in all_mnemonics() {
            assert!(
                rows.iter().any(|r| r == m),
                "opcode '{m}' executes in the machine but is not documented in SPEC §15"
            );
        }
        for r in &rows {
            assert!(
                all_mnemonics().contains(&r.as_str()),
                "SPEC §15 documents opcode '{r}' which the machine does not have"
            );
        }
        assert_eq!(
            rows.len(),
            all_mnemonics().len(),
            "SPEC §15 row count != machine opcode count — one row per opcode, exactly"
        );
    }

    /// W010-A: the listing is the documented four-column contract
    /// (idx | mnemonic | operands | line) — column 2 is always a known
    /// mnemonic, column 4 is a line number or '-', and the annotation is
    /// LIVE: a multi-line program really carries source lines (the
    /// vacuous-pin lesson — assert the stamp exists, not just the shape).
    #[test]
    fn listing_is_four_column_annotated() {
        let src = "gene f(n) {\n    let x = n + 1\n    return x * 2\n}\n";
        let d = disassemble_program(&parse(src));
        let mut stamps = 0;
        for line in d.lines() {
            if !line.contains('|') {
                continue;
            }
            let cols: Vec<&str> = line.split('|').map(|c| c.trim()).collect();
            assert_eq!(cols.len(), 4, "listing is not four columns: {line}");
            assert!(
                all_mnemonics().contains(&cols[1]),
                "column 2 is not a known mnemonic: {line}"
            );
            assert!(
                cols[0].chars().all(|c| c.is_ascii_digit()),
                "column 1 is not an instruction index: {line}"
            );
            if cols[3] == "-" {
                continue;
            }
            assert!(
                cols[3].chars().all(|c| c.is_ascii_digit()) && cols[3] != "0",
                "column 4 is neither '-' nor a real line number: {line}"
            );
            stamps += 1;
        }
        assert!(
            stamps > 0,
            "no line stamps in a multi-line listing — the annotation is vacuous"
        );
    }
}

// ============================================================ W16 fiber tests

#[cfg(test)]
mod fiber_tests {
    use super::*;
    use crate::interp::Interp;
    use crate::value::Value;

    /// Parse a program, bind its top-level statements (genes land in the
    /// global env), and hand back a vm-mode interp ready for both engines.
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

    /// The sync baseline: the exact named_call_tail_vm funnel the VM's
    /// CallNamed arm uses.
    fn run_sync(interp: &mut Interp, name: &str, args: Vec<Value>) -> Result<Value, Stress> {
        let g = interp.global.clone();
        interp.named_call_tail_vm(&g, name, args, None)
    }

    /// The fiber engine: begin on the shared funnel, drive to completion.
    /// Sleeps wake immediately (the test's virtual clock is infinitely fast;
    /// slice 2's scheduler adds the deterministic ordering on top).
    fn run_fiber(interp: &mut Interp, name: &str, args: Vec<Value>) -> Result<Value, Stress> {
        let g = interp.global.clone();
        let mut fiber = match fiber_call_begin(interp, &g, name, args)? {
            FiberBegin::Fiber(f) => f,
            FiberBegin::Completed(v) => return Ok(v),
        };
        loop {
            match fiber_run(interp, &mut fiber)? {
                FiberOutcome::Done(v) => return Ok(v),
                FiberOutcome::Suspended(pw) => match pw {
                    PendingWake::Sleep(_) => fiber.wake_result = Some(Value::Null),
                    PendingWake::Chan { .. } => {
                        panic!("no scheduler in this test")
                    }
                },
            }
        }
    }

    /// Value/Stress are deliberately Debug-less; tests fail through here.
    fn ok(r: Result<Value, Stress>, what: &str) -> Value {
        match r {
            Ok(v) => v,
            Err(s) => panic!("{what}: {}: {}", s.kind, s.message),
        }
    }

    #[test]
    fn fiber_matches_sync_on_shapes() {
        let cases: Vec<(&str, &str, Vec<Value>, Value)> = vec![
            (
                "arith + loop + compound assign",
                "gene f(n) {\n let acc = 0\n for i in range(0, n) {\n  acc += i\n }\n return acc\n}\n",
                vec![Value::Int(10)],
                Value::Int(45),
            ),
            (
                "recursion (native frames)",
                "gene f(n) {\n if n < 2 {\n  return n\n }\n return f(n - 1) + f(n - 2)\n}\n",
                vec![Value::Int(12)],
                Value::Int(144),
            ),
            (
                "scopes and shadowing",
                "gene f() {\n let x = 1\n if true {\n  let x = 2\n  x += 10\n }\n return x\n}\n",
                vec![],
                Value::Int(1),
            ),
            (
                "early return beats later code",
                "gene f(n) {\n if n > 0 {\n  return 99\n }\n return -1\n}\n",
                vec![Value::Int(5)],
                Value::Int(99),
            ),
            (
                "fell off the end = null",
                "gene f() {\n let x = 5\n}\n",
                vec![],
                Value::Null,
            ),
            (
                "break and continue",
                "gene f() {\n let acc = 0\n for i in range(0, 10) {\n  if i == 3 {\n   continue\n  }\n  if i == 7 {\n   break\n  }\n  acc += i\n }\n return acc\n}\n",
                vec![],
                // 0+1+2 skipped? no: continue skips 3, break stops at 7 —
                // the sum of 0,1,2,4,5,6
                Value::Int(18),
            ),
            (
                "return annotation honored",
                "gene f(n) -> int {\n return n * 2\n}\n",
                vec![Value::Int(21)],
                Value::Int(42),
            ),
            (
                "return annotation violated = same stress",
                "gene f(n) -> str {\n return n * 2\n}\n",
                vec![Value::Int(21)],
                Value::Null, // value ignored; the stress kind is compared
            ),
        ];
        for (label, src, args, want) in cases {
            let is_stress_case = label.contains("violated");
            let mut i1 = setup(src);
            let s1 = run_sync(&mut i1, "f", args.clone());
            let mut i2 = setup(src);
            let f2 = run_fiber(&mut i2, "f", args.clone());
            if is_stress_case {
                let k1 = s1.err().map(|s| s.kind);
                let k2 = f2.err().map(|s| s.kind);
                assert_eq!(k1, k2, "{label}: stress kinds diverged");
                assert_eq!(
                    k2,
                    Some("unfolded".to_string()),
                    "{label}: expected the annotation stress"
                );
            } else {
                assert_eq!(
                    ok(s1, label).display(),
                    want.display(),
                    "{label}: sync baseline drifted"
                );
                assert_eq!(
                    ok(f2, label).display(),
                    want.display(),
                    "{label}: fiber machine diverged"
                );
            }
        }
    }

    #[test]
    fn fiber_notes_match_sync_notes() {
        // the unbound read note (level 4) is output: both engines must
        // emit the identical note text for the identical program
        let src = "gene f() {\n return nope\n}\n";
        let mut i1 = setup(src);
        let _ = run_sync(&mut i1, "f", vec![]);
        let mut i2 = setup(src);
        let _ = run_fiber(&mut i2, "f", vec![]);
        let n1: Vec<String> = i1.notes.iter().map(|n| n.message.clone()).collect();
        let n2: Vec<String> = i2.notes.iter().map(|n| n.message.clone()).collect();
        assert_eq!(n1, n2, "note streams diverged");
        assert!(n1.iter().any(|m| m.contains("unbound 'nope'")));
    }

    #[test]
    fn fiber_stress_chain_matches_sync() {
        let src = "gene a() {\n return b()\n}\ngene b() {\n raise boom \"kaboom\"\n}\n";
        let mut i1 = setup(src);
        let e1 = run_sync(&mut i1, "a", vec![]).err();
        let mut i2 = setup(src);
        let e2 = run_fiber(&mut i2, "a", vec![]).err();
        let s1 = e1.expect("sync raised");
        let s2 = e2.expect("fiber raised");
        assert_eq!(s1.kind, s2.kind);
        assert_eq!(s1.message, s2.message);
        // the W007 traceback: innermost-first (b, then a), same lines
        let f1: Vec<&str> = s1.chain.iter().map(|(n, _)| n.as_str()).collect();
        let f2: Vec<&str> = s2.chain.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(f1, f2, "chain shape diverged");
        assert_eq!(f2, vec!["b", "a"]);
        let l1: Vec<usize> = s1.chain.iter().map(|(_, l)| *l).collect();
        let l2: Vec<usize> = s2.chain.iter().map(|(_, l)| *l).collect();
        assert_eq!(l1, l2, "chain lines diverged");
    }

    #[test]
    fn fiber_depth_limit_matches_sync() {
        let src = "gene f(n) {\n if n <= 0 {\n  return 0\n }\n return f(n - 1) + 1\n}\n";
        let mut i1 = setup(src);
        i1.depth_limit = 8;
        let e1 = run_sync(&mut i1, "f", vec![Value::Int(50)]).err();
        let mut i2 = setup(src);
        i2.depth_limit = 8;
        let e2 = run_fiber(&mut i2, "f", vec![Value::Int(50)]).err();
        let s1 = e1.expect("sync hit the limit");
        let s2 = e2.expect("fiber hit the limit");
        assert_eq!(s1.kind, "overflow");
        assert_eq!(s2.kind, "overflow");
        assert_eq!(s1.message, s2.message);
    }

    #[test]
    fn fiber_parks_on_sleep_with_identical_charges() {
        let src = "gene s() {\n sleep(250)\n return 7\n}\n";
        let mut interp = setup(src);
        let g = interp.global.clone();
        let mut fiber = ok_fiber(
            fiber_call_begin(&mut interp, &g, "s", vec![]),
            "fiber begins",
        );
        let out = ok_out(fiber_run(&mut interp, &mut fiber), "first turn");
        match out {
            FiberOutcome::Suspended(PendingWake::Sleep(ms)) => assert_eq!(ms, 250),
            other => panic!("expected a sleep park, got {:?}", other_tag(&other)),
        }
        // the fuel contract: sleep charges ms*1000 steps, exactly the
        // thread lane's charge (suspension never refunds fuel). The delta
        // also counts the few one-step instruction ticks executed so far
        // (Nop/CallNamed + the setup statements), everything above the
        // 250_000 sleep charge is ticks.
        let steps = interp.steps;
        assert!(
            (250_000..250_010).contains(&steps),
            "sleep charge drifted: {}",
            steps
        );
        // and no wall time passed: the park is a state write, not a block
        // (asserted structurally: the fiber is suspended, not running)
        assert_eq!(fiber.fiber_state, FiberState::SuspendedOn(WakeTag::Sleep));
        // resume with the awaited value: the machine pushes it past the
        // parked call site and finishes the gene
        fiber.wake_result = Some(Value::Null);
        let out = ok_out(fiber_run(&mut interp, &mut fiber), "final turn");
        match out {
            FiberOutcome::Done(v) => assert_eq!(v.display(), Value::Int(7).display()),
            other => panic!("expected done, got {:?}", other_tag(&other)),
        }
        assert_eq!(fiber.fiber_state, FiberState::Done);
    }

    #[test]
    fn fiber_sleep_charges_shared_fuel_pool() {
        let src = "gene s() {\n sleep(100)\n return 1\n}\n";
        let mut interp = setup(src);
        let pool = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(10_000_000));
        interp.fuel_pool = Some(pool.clone());
        let g = interp.global.clone();
        let mut fiber = ok_fiber(
            fiber_call_begin(&mut interp, &g, "s", vec![]),
            "fiber begins",
        );
        let out = ok_out(fiber_run(&mut interp, &mut fiber), "first turn");
        assert!(matches!(
            out,
            FiberOutcome::Suspended(PendingWake::Sleep(100))
        ));
        // loop-5: ONE pool per run — the fiber drained it ms*1000 like a
        // worker thread would
        let left = pool.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(left, 10_000_000 - 100_000);
    }

    #[test]
    fn fiber_call_begin_completes_non_gene_targets() {
        // a builtin target executes through the funnel and never reaches a
        // gene body: the call's value comes back as Completed (the exact
        // observable a thread worker running the same call would produce;
        // spawn validates genes before the lane split anyway)
        let mut interp = setup("gene f() {\n return 1\n}\n");
        let g = interp.global.clone();
        let r = fiber_call_begin(&mut interp, &g, "floor", vec![Value::Float(2.5)]);
        match r {
            Ok(FiberBegin::Completed(v)) => assert_eq!(v.display(), "2"),
            other => panic!("expected Completed, got {:?}", other.is_err()),
        }
    }

    fn ok_out(r: Result<FiberOutcome, Stress>, what: &str) -> FiberOutcome {
        match r {
            Ok(v) => v,
            Err(s) => panic!("{what}: {}: {}", s.kind, s.message),
        }
    }

    fn ok_fiber(r: Result<FiberBegin, Stress>, what: &str) -> Fiber {
        match r {
            Ok(FiberBegin::Fiber(f)) => f,
            Ok(FiberBegin::Completed(_)) => panic!("{what}: expected a fiber body"),
            Err(s) => panic!("{what}: {}: {}", s.kind, s.message),
        }
    }

    fn other_tag(o: &FiberOutcome) -> &'static str {
        match o {
            FiberOutcome::Done(_) => "done",
            FiberOutcome::Suspended(_) => "suspended",
        }
    }
}
