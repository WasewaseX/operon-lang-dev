//! opt.rs — W011: the semantics-preserving optimization pipeline for the
//! bytecode VM (docs/OPTIMIZER.md).
//!
//! Governing rule (owner directive): EVERY optimization must preserve
//! semantics — the optimized bytecode must produce byte-identical stdout,
//! stderr, notes, stress traces, and RNG-stream positions as the unoptimized
//! code on every input. The passes here are therefore deliberately
//! conservative, and each one carries its soundness argument in its doc
//! comment. The parity gates are:
//!   * scripts/vm_parity.sh      — VM(O-default) == tree-walk, whole corpus
//!   * scripts/opt_parity.sh     — VM(O0/O1/O2 × fast 0/1) == tree-walk
//!   * bootstrap/harness.py      — Rust core == Python oracle (untouched)
//!
//! Pass inventory (the owner's W011 list):
//!   prop   — block-local constant propagation (O2)
//!   fold   — constant folding: pure_binop / pure-un / truthy branch folds
//!   thread — jump threading through unconditional-Jmp chains
//!   dce    — reachability dead-code elimination with full target remap
//! Engine-level fast paths (fast_engine flag, in interp.rs):
//!   trivial-body fast return, lazy traceback frames, builtin dispatch
//!   tables, list-method fast paths.
//!
//! Why there is NO compile-time gene inlining: a named call must run the
//! RISC silencing gate (notes + xorshift RNG stream + Redirect/Degraded
//! decisions from runtime config), the recursion-depth counter, guard
//! bodies, and annotation checks. Hoisting any of these to compile time is
//! observable (redteam pins all of them), so inlining lives INSIDE the
//! funnel as the trivial-body fast path instead (see interp.rs
//! exec_gene_body) — every gate still runs; only the frame construction
//! and dispatch for `return <literal>` bodies is skipped.

use crate::ast::{BinOp, UnOp};
use crate::compile::{FuncCode, Insn, Unit};
use crate::value::Value;

/// Individual optimization switches (the owner's "individual optimization
/// switches" item). `--opt-passes=fold,thread,dce,prop` on the CLI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassSet {
    pub prop: bool,
    pub fold: bool,
    pub thread: bool,
    pub dce: bool,
}

impl PassSet {
    pub const NONE: PassSet = PassSet {
        prop: false,
        fold: false,
        thread: false,
        dce: false,
    };
    /// O1: the always-safe structural set.
    pub const O1: PassSet = PassSet {
        prop: false,
        fold: true,
        thread: true,
        dce: true,
    };
    /// O2: O1 + block-local constant propagation.
    pub const O2: PassSet = PassSet {
        prop: true,
        fold: true,
        thread: true,
        dce: true,
    };

    pub fn enabled_names(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.prop {
            v.push("prop");
        }
        if self.fold {
            v.push("fold");
        }
        if self.thread {
            v.push("thread");
        }
        if self.dce {
            v.push("dce");
        }
        v
    }
}

/// Parse `--opt-passes=fold,thread,...` (comma-separated, order-free).
/// Unknown names are a hard error (dx-r1 typo guard discipline).
pub fn parse_passes(s: &str) -> Result<PassSet, String> {
    let mut ps = PassSet::NONE;
    for name in s.split(',') {
        match name.trim() {
            "prop" => ps.prop = true,
            "fold" => ps.fold = true,
            "thread" => ps.thread = true,
            "dce" => ps.dce = true,
            "" => {}
            other => return Err(format!("unknown optimization pass '{}'", other)),
        }
    }
    Ok(ps)
}

/// Optimization configuration: level (0/1/2), optional pass override,
/// engine fast-path switch, and the --dump-optimized report flag.
#[derive(Clone, Debug)]
pub struct OptConfig {
    pub level: u8,
    pub passes: PassSet,
    pub fast_engine: bool,
    pub dump: bool,
}

impl OptConfig {
    /// Level semantics (docs/OPTIMIZER.md §levels):
    ///   O0 — no compile-time passes (pure W09 bytecode)
    ///   O1 — fold + thread + dce (structural, always safe)
    ///   O2 — O1 + block-local constant propagation
    pub fn level(level: u8) -> OptConfig {
        let passes = match level {
            0 => PassSet::NONE,
            1 => PassSet::O1,
            _ => PassSet::O2,
        };
        OptConfig {
            level,
            passes,
            fast_engine: true,
            dump: false,
        }
    }

    /// CLI override: `--opt=N` plus optional `--opt-passes=a,b,c` (which
    /// pins the set exactly, overriding the level's defaults) and
    /// `--no-fast`.
    pub fn from_cli(level: u8, passes: Option<&str>, fast: bool) -> Result<OptConfig, String> {
        let mut cfg = OptConfig::level(level);
        cfg.fast_engine = fast;
        if let Some(p) = passes {
            cfg.passes = parse_passes(p)?;
            // an explicit pass list pins the level's identity for dumps
            cfg.level = if cfg.passes == PassSet::NONE {
                0
            } else if cfg.passes == PassSet::O2 {
                2
            } else {
                1
            };
        }
        Ok(cfg)
    }
}

impl Default for OptConfig {
    fn default() -> Self {
        // --vm default: O1 + engine fast paths. O1 is the always-safe set;
        // the parity gates run the whole corpus through this default.
        OptConfig::level(1)
    }
}

/// Per-function optimization report line (--dump-optimized).
pub struct OptStats {
    pub funcs: Vec<(String, usize, usize)>,
    pub passes: Vec<&'static str>,
    pub level: u8,
}

impl OptStats {
    pub fn render(&self) -> String {
        let mut out = format!(
            "optimizer: level O{} passes={} fast=on\n",
            self.level,
            if self.passes.is_empty() {
                "none".to_string()
            } else {
                self.passes.join(",")
            }
        );
        for (name, before, after) in &self.funcs {
            out.push_str(&format!("  {} insns {} -> {}\n", name, before, after));
        }
        out
    }
}

/// Optimize every compiled function in the unit. Deterministic: same input
/// FuncCode + config → same output FuncCode (all passes are single pass in
/// index order, no HashMap iteration influences decisions).
/// `Rc::make_mut` mutates in place — compile_program hands out uniquely
/// owned codes, so this never clones.
pub fn optimize_unit(unit: &mut Unit, cfg: &OptConfig) -> OptStats {
    let mut stats = OptStats {
        funcs: Vec::new(),
        passes: cfg.passes.enabled_names(),
        level: cfg.level,
    };
    for (_key, code) in unit.funcs.iter_mut() {
        // compile_program hands out uniquely owned codes — get_mut is exact
        let c =
            std::rc::Rc::get_mut(code).expect("freshly compiled FuncCode must be uniquely owned");
        let before = c.insns.len();
        let name = c.name.clone();
        optimize_code(c, &cfg.passes);
        let after = c.insns.len();
        stats.funcs.push((name, before, after));
    }
    stats
}

/// Optimize one function body with the given pass set. Order:
/// prop → fold (fixpoint) → thread → dce. prop feeds fold (propagated
/// constants create foldable windows); dce last removes the Nops fold
/// leaves behind and remaps every stored target.
pub fn optimize_code(code: &mut FuncCode, ps: &PassSet) -> bool {
    let mut changed = false;
    if ps.prop {
        changed |= pass_prop(code);
    }
    if ps.fold {
        // Each fold round strictly reduces the number of non-Nop insns,
        // so this terminates; the cap is belt-and-braces.
        for _ in 0..32 {
            if !pass_fold_once(code) {
                break;
            }
            changed = true;
        }
    }
    if ps.thread {
        changed |= pass_thread(code);
    }
    if ps.dce {
        changed |= pass_dce(code);
    }
    changed
}

// ------------------------------------------------------------------ prop

/// Block-local constant propagation.
///
/// Soundness argument:
///   * A binding `k` is tracked ONLY between a `Define(k)` whose value is
///     the immediately-preceding `Const(c)` and later `Load(k)`s in the
///     SAME basic block (the walk resets at every jump target — join
///     points cannot be reached with stale state).
///   * Any insn that could rebind k removes k: Store/StoreOp/Define/
///     DefineAnn/IterBindName of k (this function's own stream) — and ANY
///     call-shaped insn (CallFinish/CallValue/Method/MethodSafe) clears
///     ALL tracked names, because a closure that captured k's cell could
///     Store(k) from another FuncCode. Delegated Stmt/Expr can define or
///     assign arbitrary names in the current scope → clear all.
///   * Scope frames: PushScope/PopScope clear all (a same-depth re-entry
///     is a different frame; Load would resolve a different binding).
///   * The unbound-read note (Total Grammar) cannot fire: the dominating
///     Define executed on this straight-line path.
///   * charge_clone (sec-r5 F-9) only charges Str > 64 KiB — large-string
///     constants are NOT propagated, so the Load's charge is preserved
///     exactly (scalar loads charge nothing).
///   * The replacement pushes the SAME const value (Const clones from the
///     pool exactly as Load clones from the env).
fn pass_prop(code: &mut FuncCode) -> bool {
    let targets = all_targets(code);
    let mut bound: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut last_const: Option<u32> = None;
    let mut changed = false;
    let n = code.insns.len();
    for pc in 0..n {
        if targets.contains(&(pc as u32)) {
            // join point: state from the fall-through path is not dominant
            bound.clear();
            last_const = None;
        }
        // classify on a shallow clone of the discriminant + payloads
        let kill_all;
        let name_kill;
        match &code.insns[pc] {
            Insn::Const(c) => {
                last_const = Some(*c);
                kill_all = false;
                name_kill = None;
            }
            Insn::Define(k) => {
                if let Some(c) = last_const {
                    bound.insert(*k, c);
                } else {
                    bound.remove(k);
                }
                last_const = None;
                kill_all = false;
                name_kill = None;
            }
            Insn::DefineAnn(k, _)
            | Insn::Store(k)
            | Insn::StoreOp(k, _)
            | Insn::IterBindName(k) => {
                last_const = None;
                kill_all = false;
                name_kill = Some(*k);
            }
            Insn::Load(k) => {
                last_const = None;
                kill_all = false;
                name_kill = None;
                if let Some(c) = bound.get(k) {
                    let big_str =
                        matches!(&code.consts[*c as usize], Value::Str(s) if s.len() > 64 * 1024);
                    if !big_str {
                        code.insns[pc] = Insn::Const(*c);
                        changed = true;
                        continue;
                    }
                }
            }
            // call-shaped / delegated / scope / control joins: clear ALL
            Insn::CallFinish(_, _)
            | Insn::CallValue(_)
            | Insn::Method(_, _)
            | Insn::MethodSafe(_, _)
            | Insn::Stmt(_)
            | Insn::Expr(_)
            | Insn::IterNext(_)
            | Insn::RaiseStmt(_, _)
            | Insn::RescueBind(_)
            | Insn::CatchEnter(_, _, _)
            | Insn::CatchLeave
            | Insn::CatchTrim(_)
            | Insn::PushScope
            | Insn::PopScope
            | Insn::LoopEnter(_, _)
            | Insn::LoopPop
            | Insn::Ret
            | Insn::RetNull
            | Insn::Jmp(_) => {
                last_const = None;
                kill_all = true;
                name_kill = None;
            }
            // everything else carries state (reads, pure ops, stamps, the
            // RISC decision insn whose arguments follow it)
            Insn::Line(_)
            | Insn::Tick
            | Insn::LoadPlain(_)
            | Insn::Bin(_)
            | Insn::Un(_)
            | Insn::Index
            | Insn::Member(_)
            | Insn::MemberSafe(_)
            | Insn::MakeList(_)
            | Insn::MakeMap(_)
            | Insn::AndJmp(_)
            | Insn::OrJmp(_)
            | Insn::NullishJmp(_)
            | Insn::JmpIfFalse(_)
            | Insn::CallStart(_, _)
            | Insn::IfDegraded(_, _)
            | Insn::IterMake
            | Insn::IterEnd
            | Insn::RescueTicks
            | Insn::Dup
            | Insn::Pop
            | Insn::Nop => {
                last_const = None;
                kill_all = false;
                name_kill = None;
            }
        }
        if kill_all {
            bound.clear();
        } else if let Some(k) = name_kill {
            bound.remove(&k);
        }
    }
    changed
}

// ------------------------------------------------------------------ fold

/// One constant-folding round. Returns whether anything changed.
///
/// Soundness argument:
///   * Binary/unary folds call `crate::interp::pure_binop` / `pure_un` —
///     the SAME functions the VM's Bin/Un arms evaluate through (agreement
///     by construction). If the op would raise (overflow, division by
///     zero, type error) or needs runtime state (mem_charge for string/
///     list concat, str_repeat), folding is REFUSED and the runtime raises
///     or charges identically.
///   * Branch folds call `Value::truthy()` — the same function the VM's
///     AndJmp/OrJmp/NullishJmp/JmpIfFalse arms branch on. The rewrites
///     preserve the exact stack shape of each path (see the per-arm
///     comments).
///   * Line/Tick stamps inside the folded window are preserved (the
///     back-scan skips only Nop/Line/Tick, and Line stamps are never
///     rewritten), so the cur_line trajectory — and therefore every
///     later traceback line — is unchanged.
fn pass_fold_once(code: &mut FuncCode) -> bool {
    let n = code.insns.len();
    for pc in 0..n {
        // shallow descriptor: (op-kind, payload, target)
        enum Opd {
            Bin(BinOp),
            Un(UnOp),
            AndJmp(u32),
            OrJmp(u32),
            NullishJmp(u32),
            JmpIfFalse(u32),
        }
        let opd = match &code.insns[pc] {
            Insn::Bin(op) => Some(Opd::Bin(*op)),
            Insn::Un(op) => Some(Opd::Un(*op)),
            Insn::AndJmp(t) => Some(Opd::AndJmp(*t)),
            Insn::OrJmp(t) => Some(Opd::OrJmp(*t)),
            Insn::NullishJmp(t) => Some(Opd::NullishJmp(*t)),
            Insn::JmpIfFalse(t) => Some(Opd::JmpIfFalse(*t)),
            _ => None,
        };
        let Some(opd) = opd else { continue };
        // back-scan: skip Nop/Line/Tick (stack-neutral, stamp-preserving)
        let skippable = |i: &Insn| matches!(i, Insn::Nop | Insn::Line(_) | Insn::Tick);
        let mut idx = pc;
        let slot_r = loop {
            if idx == 0 {
                return false; // ran out of insns — no fold
            }
            idx -= 1;
            if skippable(&code.insns[idx]) {
                continue;
            }
            break idx;
        };
        let slot_l = if matches!(opd, Opd::Bin(_)) {
            let mut idx = slot_r;
            let found = loop {
                if idx == 0 {
                    return false;
                }
                idx -= 1;
                if skippable(&code.insns[idx]) {
                    continue;
                }
                break idx;
            };
            if !matches!(code.insns[found], Insn::Const(_)) {
                continue;
            }
            found
        } else {
            slot_r // 1-operand forms reuse the single slot
        };
        if !matches!(code.insns[slot_r], Insn::Const(_)) {
            continue;
        }
        // payload extraction (const indices / targets)
        let const_at = |i: usize| -> u32 {
            match &code.insns[i] {
                Insn::Const(k) => *k,
                _ => unreachable!("window checked"),
            }
        };
        match opd {
            Opd::Bin(op) => {
                let l = &code.consts[const_at(slot_l) as usize];
                let r = &code.consts[const_at(slot_r) as usize];
                if let Some(Ok(v)) = crate::interp::pure_binop(op, l, r, 0) {
                    let k = konst_push(code, v);
                    code.insns[slot_l] = Insn::Const(k);
                    code.insns[slot_r] = Insn::Nop;
                    code.insns[pc] = Insn::Nop;
                    return true;
                }
            }
            Opd::Un(op) => {
                let v = &code.consts[const_at(slot_r) as usize];
                if let Some(Ok(v2)) = pure_un(op, v) {
                    let k = konst_push(code, v2);
                    code.insns[slot_r] = Insn::Const(k);
                    code.insns[pc] = Insn::Nop;
                    return true;
                }
            }
            Opd::AndJmp(t) => {
                let v = &code.consts[const_at(slot_r) as usize];
                if v.truthy() {
                    // short-circuit ALWAYS: left value stays, jump to t
                    code.insns[pc] = Insn::Jmp(t);
                } else {
                    // never short-circuits: pop the left, fall through
                    code.insns[pc] = Insn::Pop;
                }
                return true;
            }
            Opd::OrJmp(t) => {
                let v = &code.consts[const_at(slot_r) as usize];
                if v.truthy() {
                    code.insns[pc] = Insn::Jmp(t);
                } else {
                    code.insns[pc] = Insn::Pop;
                }
                return true;
            }
            Opd::NullishJmp(t) => {
                let v = &code.consts[const_at(slot_r) as usize];
                if !matches!(v, Value::Null) {
                    code.insns[pc] = Insn::Jmp(t);
                } else {
                    code.insns[pc] = Insn::Pop;
                }
                return true;
            }
            Opd::JmpIfFalse(t) => {
                let v = &code.consts[const_at(slot_r) as usize];
                if v.truthy() {
                    code.insns[pc] = Insn::Pop; // condition consumed, fall through
                } else {
                    code.insns[pc] = Insn::Jmp(t); // pop + jump (the arm pops first)
                }
                return true;
            }
        }
    }
    false
}

/// Append a folded constant to the pool (dedupe mirrors FuncCompiler::konst:
/// deep-equal + same discriminant reuses the existing slot — keeps codegen
/// deterministic and pools small). Floats dedupe on BIT pattern, not `==`:
/// IEEE says 0.0 == -0.0, and collapsing the sign of a folded zero would
/// change printed output (rt_p5a/rt_p5e pin -0.0).
fn konst_push(code: &mut FuncCode, v: Value) -> u32 {
    for (i, c) in code.consts.iter().enumerate() {
        if std::mem::discriminant(c) == std::mem::discriminant(&v) {
            let same = match (c, &v) {
                (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
                _ => c.deep_eq(&v),
            };
            if same {
                return i as u32;
            }
        }
    }
    code.consts.push(v);
    (code.consts.len() - 1) as u32
}

/// The pure subset of the VM's Un arm (vm.rs). Errors mean "the runtime
/// would raise exactly this" — the folder refuses and leaves the insn.
fn pure_un(op: UnOp, v: &Value) -> Option<Result<Value, crate::value::Stress>> {
    use crate::value::Stress;
    Some(match op {
        UnOp::Neg => match v {
            Value::Int(i) => i
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| Stress::new("overflow", "int overflow in negation (i64::MIN)")),
            Value::Float(f) => Ok(Value::Float(-f)),
            other => Err(Stress::new(
                "unfolded",
                format!("cannot negate {}", other.type_name()),
            )),
        },
        UnOp::Not => Ok(Value::Bool(!v.truthy())),
        UnOp::BitNot => match v {
            Value::Int(i) => Ok(Value::Int(!i)),
            Value::Bool(b) => Ok(Value::Int(!((*b) as i64))),
            other => Err(Stress::new(
                "unfolded",
                format!("cannot bit-invert {}", other.type_name()),
            )),
        },
    })
}

// ---------------------------------------------------------------- thread

/// Jump threading: retarget every single-target jump through chains of
/// unconditional `Jmp`s.
///
/// Soundness argument: an unconditional Jmp executes no side effects, so
/// `Jmp(a)` followed by `insns[a] == Jmp(b)` lands at b with the identical
/// stack, env, marks, and cur_line. Conditional jumps (JmpIfFalse and
/// friends) are retargeted only THROUGH the chain — their own condition
/// and pop still happen at the original site. Cycles are left untouched
/// (visited set) — a jump into a cycle loops forever either way, and the
/// original target keeps that behavior byte-for-byte.
fn pass_thread(code: &mut FuncCode) -> bool {
    let mut changed = false;
    let n = code.insns.len();
    for pc in 0..n {
        let t0 = match &code.insns[pc] {
            Insn::Jmp(t)
            | Insn::JmpIfFalse(t)
            | Insn::AndJmp(t)
            | Insn::OrJmp(t)
            | Insn::NullishJmp(t)
            | Insn::IfDegraded(_, t)
            | Insn::IterNext(t) => *t as usize,
            _ => continue,
        };
        let mut t = t0;
        let mut hops = 0;
        let mut visited = std::collections::HashSet::new();
        while hops < 64 {
            // a target of insns.len() is the function's implicit end (the
            // exec loop returns Flow::Norm there) — legal, but there is no
            // insn to inspect, so the chain ends here
            if t >= code.insns.len() {
                break;
            }
            if !visited.insert(t) {
                break; // cycle — leave as-is
            }
            match &code.insns[t] {
                Insn::Jmp(t2) if *t2 as usize != t => {
                    t = *t2 as usize;
                    hops += 1;
                }
                _ => break,
            }
        }
        if t != t0 && (t as u32) <= u32::MAX {
            let newt = t as u32;
            match &mut code.insns[pc] {
                Insn::Jmp(f)
                | Insn::JmpIfFalse(f)
                | Insn::AndJmp(f)
                | Insn::OrJmp(f)
                | Insn::NullishJmp(f)
                | Insn::IfDegraded(_, f)
                | Insn::IterNext(f) => *f = newt,
                _ => unreachable!(),
            }
            changed = true;
        }
        let _ = n;
    }
    changed
}

// ------------------------------------------------------------------- dce

/// Every pc that appears as a stored target in ANY insn.
fn all_targets(code: &FuncCode) -> std::collections::HashSet<u32> {
    let mut t = std::collections::HashSet::new();
    for insn in &code.insns {
        for x in insn_targets(insn) {
            t.insert(x);
        }
    }
    t
}

/// Static targets stored in one insn. IMPORTANT: this must cover EVERY
/// insn form that stores a pc — the VM jumps to these at runtime through
/// paths the static fall-through walk does not model (catch handlers,
/// loop records), so dce adds them all as reachability edges.
fn insn_targets(i: &Insn) -> Vec<u32> {
    match i {
        Insn::Jmp(t)
        | Insn::JmpIfFalse(t)
        | Insn::AndJmp(t)
        | Insn::OrJmp(t)
        | Insn::NullishJmp(t)
        | Insn::IfDegraded(_, t)
        | Insn::IterNext(t) => vec![*t],
        Insn::LoopEnter(top, end) => vec![*top, *end],
        Insn::CatchEnter(_, leave, handler) => vec![*leave, *handler],
        _ => Vec::new(),
    }
}

/// Reachability dead-code elimination with full target remap.
///
/// Soundness argument:
///   * Only insns with NO static reachability from pc 0 are removed — they
///     can never execute, so no tick, note, charge, stamp, or RNG draw can
///     differ.
///   * The reachability walk is a conservative SUPERSET: delegated
///     Stmt/Expr (which may Ret/Brk/Cont/raise through the tree-walk) are
///     modeled as plain fall-through, and every stored target of every
///     insn is added as an edge — including catch handlers (reachable
///     only through the catch machinery) and loop records (VLoop.end is
///     consumed by delegated breaks).
///   * Jmp is jump-only (no fall-through edge); Ret/RetNull/RaiseStmt are
///     terminal (a RaiseStmt's handler reachability comes from the
///     matching CatchEnter's target edges).
///   * During the renumber: Nops and no-op `Jmp(next)` are dropped, and
///     `JmpIfFalse(next)` canonicalizes to `Pop` (the arm pops the
///     condition on the fall-through path — identical stack shape). A
///     canonicalization is skipped if the insn is itself a jump target
///     (target insns must keep their pc for the remap to be total).
///   * ALL stored targets are remapped: the jump family, IfDegraded,
///     IterNext, LoopEnter(top,end), CatchEnter(kind,leave,handler). The
///     catch range check (`pc ∈ [start, end]`) stays correct because the
///     remap is order-preserving on reachable insns: p ≤ end iff
///     map(p) ≤ map(end).
fn pass_dce(code: &mut FuncCode) -> bool {
    let n = code.insns.len();
    // 1. reachability
    let mut reach = vec![false; n];
    let mut work: Vec<usize> = vec![0];
    while let Some(pc) = work.pop() {
        if pc >= n || reach[pc] {
            continue;
        }
        reach[pc] = true;
        match &code.insns[pc] {
            Insn::Jmp(t) => work.push(*t as usize),
            Insn::Ret | Insn::RetNull | Insn::RaiseStmt(_, _) => {}
            other => {
                for t in insn_targets(other) {
                    work.push(t as usize);
                }
                work.push(pc + 1);
            }
        }
    }
    // 2. drop candidates: Nops and no-op Jmp-next, never at target pcs
    let targets = all_targets(code);
    let mut drop = vec![false; n];
    let mut any_drop = false;
    for pc in 0..n {
        if !reach[pc] || targets.contains(&(pc as u32)) {
            continue;
        }
        match &code.insns[pc] {
            Insn::Nop => drop[pc] = true,
            // `Jmp(next)` is a no-op jump (fallthrough equivalent); so is a
            // trailing `Jmp(end)` on the LAST insn (falling off the end also
            // returns Flow::Norm). A Jmp(end) anywhere ELSE is load-bearing:
            // dropping it would execute the insn after it. Bounds guard:
            // reach[pc+1] would panic at pc+1 == n.
            Insn::Jmp(t)
                if (*t as usize == pc + 1 && pc + 1 < n && reach[pc + 1])
                    || (*t as usize == n && pc + 1 == n) =>
            {
                drop[pc] = true
            }
            _ => {}
        }
        any_drop |= drop[pc];
    }
    // 3. tentative index over reachable insns (for JmpIfFalse→Pop check)
    let mut tent = vec![usize::MAX; n];
    let mut k = 0usize;
    for pc in 0..n {
        if reach[pc] && !drop[pc] {
            tent[pc] = k;
            k += 1;
        }
    }
    let mut pop_rewrite: Vec<usize> = Vec::new();
    for pc in 0..n {
        if reach[pc] && !drop[pc] {
            if let Insn::JmpIfFalse(t) = &code.insns[pc] {
                let tt = *t as usize;
                if tt < n && reach[tt] && tent[tt] == tent[pc] + 1 {
                    pop_rewrite.push(pc);
                }
            }
        }
    }
    let new_n = k;
    if new_n == n && !any_drop && pop_rewrite.is_empty() {
        return false;
    }
    // 4. emit remapped body
    let old = std::mem::take(&mut code.insns);
    let mut newi: Vec<Insn> = Vec::with_capacity(new_n);
    let mut final_map = vec![usize::MAX; n];
    let mut i = 0usize;
    for pc in 0..n {
        if !reach[pc] || drop[pc] {
            continue;
        }
        final_map[pc] = i;
        i += 1;
    }
    let remap = |t: u32| -> u32 {
        // the implicit function end (t == old n) maps to the new end
        if t as usize == n {
            return new_n as u32;
        }
        let m = final_map[t as usize];
        debug_assert!(m != usize::MAX, "remap of unreachable target");
        m as u32
    };
    for pc in 0..n {
        if !reach[pc] || drop[pc] {
            continue;
        }
        let insn = &old[pc];
        if pop_rewrite.contains(&pc) {
            newi.push(Insn::Pop);
            continue;
        }
        newi.push(match insn {
            Insn::Jmp(t) => Insn::Jmp(remap(*t)),
            Insn::JmpIfFalse(t) => Insn::JmpIfFalse(remap(*t)),
            Insn::AndJmp(t) => Insn::AndJmp(remap(*t)),
            Insn::OrJmp(t) => Insn::OrJmp(remap(*t)),
            Insn::NullishJmp(t) => Insn::NullishJmp(remap(*t)),
            Insn::IfDegraded(a, t) => Insn::IfDegraded(*a, remap(*t)),
            Insn::IterNext(t) => Insn::IterNext(remap(*t)),
            Insn::LoopEnter(top, end) => Insn::LoopEnter(remap(*top), remap(*end)),
            Insn::CatchEnter(kind, leave, handler) => {
                Insn::CatchEnter(*kind, remap(*leave), remap(*handler))
            }
            other => clone_insn(other),
        });
    }
    code.insns = newi;
    true
}

/// Insn has no Clone (Stmt/Expr carry Rc clones — cheap, but explicit
/// matching documents the no-op-ness). This mirrors every variant.
fn clone_insn(i: &Insn) -> Insn {
    match i {
        Insn::Const(k) => Insn::Const(*k),
        Insn::Pop => Insn::Pop,
        Insn::Dup => Insn::Dup,
        Insn::Load(k) => Insn::Load(*k),
        Insn::LoadPlain(k) => Insn::LoadPlain(*k),
        Insn::Define(k) => Insn::Define(*k),
        Insn::DefineAnn(k, a) => Insn::DefineAnn(*k, a.clone()),
        Insn::Store(k) => Insn::Store(*k),
        Insn::StoreOp(k, op) => Insn::StoreOp(*k, *op),
        Insn::Line(l) => Insn::Line(*l),
        Insn::Bin(op) => Insn::Bin(*op),
        Insn::AndJmp(t) => Insn::AndJmp(*t),
        Insn::OrJmp(t) => Insn::OrJmp(*t),
        Insn::NullishJmp(t) => Insn::NullishJmp(*t),
        Insn::Un(op) => Insn::Un(*op),
        Insn::Index => Insn::Index,
        Insn::Member(k) => Insn::Member(*k),
        Insn::MemberSafe(k) => Insn::MemberSafe(*k),
        Insn::Method(k, n) => Insn::Method(*k, *n),
        Insn::MethodSafe(k, n) => Insn::MethodSafe(*k, *n),
        Insn::MakeList(n) => Insn::MakeList(*n),
        Insn::MakeMap(n) => Insn::MakeMap(*n),
        Insn::Jmp(t) => Insn::Jmp(*t),
        Insn::JmpIfFalse(t) => Insn::JmpIfFalse(*t),
        Insn::Tick => Insn::Tick,
        Insn::PushScope => Insn::PushScope,
        Insn::PopScope => Insn::PopScope,
        Insn::Ret => Insn::Ret,
        Insn::RetNull => Insn::RetNull,
        Insn::CallStart(k, l) => Insn::CallStart(*k, *l),
        Insn::IfDegraded(a, t) => Insn::IfDegraded(*a, *t),
        Insn::CallFinish(a, k) => Insn::CallFinish(*a, *k),
        Insn::CallValue(a) => Insn::CallValue(*a),
        Insn::IterMake => Insn::IterMake,
        Insn::IterNext(t) => Insn::IterNext(*t),
        Insn::IterBindName(k) => Insn::IterBindName(*k),
        Insn::IterEnd => Insn::IterEnd,
        Insn::LoopPop => Insn::LoopPop,
        Insn::LoopEnter(top, end) => Insn::LoopEnter(*top, *end),
        Insn::CatchEnter(k, leave, handler) => Insn::CatchEnter(*k, *leave, *handler),
        Insn::CatchLeave => Insn::CatchLeave,
        Insn::CatchTrim(n) => Insn::CatchTrim(*n),
        Insn::RescueTicks => Insn::RescueTicks,
        Insn::RescueBind(k) => Insn::RescueBind(*k),
        Insn::RaiseStmt(k, l) => Insn::RaiseStmt(*k, *l),
        Insn::Stmt(s) => Insn::Stmt(s.clone()),
        Insn::Expr(e) => Insn::Expr(e.clone()),
        Insn::Nop => Insn::Nop,
    }
}

// ----------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinOp, UnOp};
    use crate::value::Stress;

    fn fc(name: &str, insns: Vec<Insn>) -> FuncCode {
        FuncCode {
            name: name.into(),
            insns,
            consts: Vec::new(),
            names: Vec::new(),
        }
    }
    fn konst(code: &mut FuncCode, v: Value) -> u32 {
        code.consts.push(v);
        (code.consts.len() - 1) as u32
    }

    fn sig(code: &FuncCode) -> Vec<&'static str> {
        code.insns
            .iter()
            .map(|i| match i {
                Insn::Const(_) => "C",
                Insn::Pop => "pop",
                Insn::Nop => "nop",
                Insn::Bin(_) => "bin",
                Insn::Un(_) => "un",
                Insn::Jmp(_) => "jmp",
                Insn::JmpIfFalse(_) => "jif",
                Insn::AndJmp(_) => "ajmp",
                Insn::OrJmp(_) => "ojmp",
                Insn::NullishJmp(_) => "njmp",
                Insn::Ret => "ret",
                Insn::Load(_) => "load",
                Insn::Define(_) => "def",
                Insn::Store(_) => "store",
                Insn::CallStart(_, _) => "cs",
                Insn::CallFinish(_, _) => "cf",
                _ => "?",
            })
            .collect()
    }

    #[test]
    fn fold_int_arith() {
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(2));
        let b = konst(&mut c, Value::Int(3));
        c.insns = vec![
            Insn::Const(a),
            Insn::Const(b),
            Insn::Bin(BinOp::Add),
            Insn::Ret,
        ];
        assert!(pass_fold_once(&mut c));
        assert_eq!(sig(&c), vec!["C", "nop", "nop", "ret"]);
        assert!(matches!(c.consts.last(), Some(Value::Int(5))));
    }

    #[test]
    fn fold_refuses_overflow_divzero_and_charging() {
        // i64::MAX + 1 — overflow raises at runtime; fold must refuse
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(i64::MAX));
        let b = konst(&mut c, Value::Int(1));
        c.insns = vec![Insn::Const(a), Insn::Const(b), Insn::Bin(BinOp::Add)];
        assert!(!pass_fold_once(&mut c));
        // 1 / 0 — division raises at runtime
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(1));
        let b = konst(&mut c, Value::Int(0));
        c.insns = vec![Insn::Const(a), Insn::Const(b), Insn::Bin(BinOp::Div)];
        assert!(!pass_fold_once(&mut c));
        // "a" + "b" — mem_charge combo; fold must refuse (sandbox parity)
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Str("a".into()));
        let b = konst(&mut c, Value::Str("b".into()));
        c.insns = vec![Insn::Const(a), Insn::Const(b), Insn::Bin(BinOp::Add)];
        assert!(!pass_fold_once(&mut c));
    }

    #[test]
    fn fold_float_bits_exact() {
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Float(0.1));
        let b = konst(&mut c, Value::Float(0.2));
        c.insns = vec![Insn::Const(a), Insn::Const(b), Insn::Bin(BinOp::Add)];
        assert!(pass_fold_once(&mut c));
        let expected_bits = (0.1f64 + 0.2f64).to_bits();
        let ok = matches!(
            c.consts.last(),
            Some(Value::Float(f)) if f.to_bits() == expected_bits
        );
        assert!(ok, "folded float must be bit-identical to runtime add");
    }

    #[test]
    fn fold_unary() {
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(5));
        c.insns = vec![Insn::Const(a), Insn::Un(UnOp::Neg)];
        assert!(pass_fold_once(&mut c));
        assert!(matches!(c.consts.last(), Some(Value::Int(-5))));
        // negation of i64::MIN overflows — refused
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(i64::MIN));
        c.insns = vec![Insn::Const(a), Insn::Un(UnOp::Neg)];
        assert!(!pass_fold_once(&mut c));
    }

    #[test]
    fn fold_branch_shapes() {
        // Const(true); JmpIfFalse(t) → Pop (condition consumed, fall through)
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Bool(true));
        c.insns = vec![Insn::Const(a), Insn::JmpIfFalse(2), Insn::Ret];
        assert!(pass_fold_once(&mut c));
        assert_eq!(sig(&c), vec!["C", "pop", "ret"]);
        // Const(false); JmpIfFalse(t) → Jmp(t)
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Bool(false));
        c.insns = vec![Insn::Const(a), Insn::JmpIfFalse(2), Insn::Ret];
        assert!(pass_fold_once(&mut c));
        assert_eq!(sig(&c), vec!["C", "jmp", "ret"]);
        // AndJmp on truthy const → Jmp (value stays on stack)
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Str("x".into()));
        c.insns = vec![Insn::Const(a), Insn::AndJmp(2), Insn::Ret];
        assert!(pass_fold_once(&mut c));
        assert_eq!(sig(&c), vec!["C", "jmp", "ret"]);
        // AndJmp on falsy const → Pop (non-short path pops the left)
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(0));
        c.insns = vec![Insn::Const(a), Insn::AndJmp(2), Insn::Ret];
        assert!(pass_fold_once(&mut c));
        assert_eq!(sig(&c), vec!["C", "pop", "ret"]);
    }

    #[test]
    fn fold_preserves_negative_zero_sign() {
        // 0.0 * -1 folds to -0.0; the pool must NOT reuse an existing +0.0
        // slot (IEEE equality would collapse the sign and flip the output)
        let mut c = fc("t", vec![]);
        let zero = konst(&mut c, Value::Float(0.0));
        let neg1 = konst(&mut c, Value::Int(-1));
        c.insns = vec![Insn::Const(zero), Insn::Const(neg1), Insn::Bin(BinOp::Mul)];
        assert!(pass_fold_once(&mut c));
        // the folded const must be a NEW slot with the -0.0 bit pattern
        let last = c.consts.last().unwrap();
        match last {
            Value::Float(f) if f.to_bits() == (-0.0f64).to_bits() => {}
            other => panic!(
                "folded zero lost its sign: discriminant {:?}",
                std::mem::discriminant(other)
            ),
        }
        assert_ne!(
            c.consts.len(),
            2,
            "the -0.0 result must not dedupe into the +0.0 slot"
        );
    }

    #[test]
    fn fold_preserves_line_stamp() {
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(2));
        let b = konst(&mut c, Value::Int(3));
        c.insns = vec![
            Insn::Line(7),
            Insn::Const(a),
            Insn::Const(b),
            Insn::Bin(BinOp::Mul),
            Insn::Ret,
        ];
        assert!(pass_fold_once(&mut c));
        assert!(matches!(c.insns[0], Insn::Line(7)));
        assert!(matches!(c.insns[1], Insn::Const(_)));
    }

    #[test]
    fn fold_chain_via_fixpoint() {
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(1));
        let b = konst(&mut c, Value::Int(2));
        let d = konst(&mut c, Value::Int(3));
        c.insns = vec![
            Insn::Const(a),
            Insn::Const(b),
            Insn::Bin(BinOp::Add),
            Insn::Const(d),
            Insn::Bin(BinOp::Mul),
            Insn::Ret,
        ];
        let ps = PassSet {
            prop: false,
            fold: true,
            thread: false,
            dce: true,
        };
        optimize_code(&mut c, &ps);
        assert_eq!(sig(&c), vec!["C", "ret"]); // Nops dropped by dce
        assert!(matches!(c.consts.last(), Some(Value::Int(9))));
    }

    #[test]
    fn thread_follows_jmp_chains_and_stops_at_cycles() {
        let mut c = fc(
            "t",
            vec![
                Insn::Jmp(1), // 0 → should retarget to 3
                Insn::Jmp(2), // 1
                Insn::Jmp(3), // 2
                Insn::Ret,    // 3
            ],
        );
        assert!(pass_thread(&mut c));
        assert!(matches!(c.insns[0], Insn::Jmp(3)));
        // cycle: 0 → 1 → 0 — left untouched
        let mut c = fc("t", vec![Insn::Jmp(1), Insn::Jmp(0)]);
        assert!(!pass_thread(&mut c));
    }

    #[test]
    fn dce_removes_unreachable_and_remaps_everything() {
        // layout: 0 Const 1 JmpIfFalse(3) [kept] 2 Jmp(5)
        //         3 Ret  4 (unreachable) Ret  5 Ret
        // dropping insn 4 shifts only insn 5 → target 3 keeps index 3
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Bool(true));
        c.insns = vec![
            Insn::Const(a),      // 0
            Insn::JmpIfFalse(3), // 1
            Insn::Jmp(5),        // 2 — reachable via the branch
            Insn::Ret,           // 3
            Insn::Ret,           // 4 — unreachable
            Insn::Ret,           // 5
        ];
        assert!(pass_dce(&mut c));
        assert_eq!(c.insns.len(), 5);
        assert!(matches!(c.insns[1], Insn::JmpIfFalse(3)));
        assert!(matches!(c.insns[2], Insn::Jmp(4)));
        // dce reports no change (all four insns are reachable) but the
        // handler + leave MUST survive the walk — they are edge targets
        let mut c = fc(
            "t",
            vec![
                Insn::CatchEnter(None, 2, 3), // 0
                Insn::Ret,                    // 1
                Insn::CatchLeave,             // 2 (leave)
                Insn::Ret,                    // 3 (handler — only via catch)
            ],
        );
        assert!(!pass_dce(&mut c), "all insns reachable — nothing to drop");
        assert_eq!(c.insns.len(), 4, "handler + leave must survive");
        match &c.insns[0] {
            Insn::CatchEnter(_, leave, handler) => {
                assert_eq!(*leave, 2);
                assert_eq!(*handler, 3);
            }
            other => panic!(
                "expected CatchEnter, got {:?}",
                std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn dce_keeps_loop_record_targets() {
        // LoopEnter(top=2, end=4): both targets must survive even though
        // the static walk only falls through — VLoop.end is consumed by
        // delegated breaks at runtime.
        let mut c = fc(
            "t",
            vec![
                Insn::LoopEnter(2, 4), // 0
                Insn::Tick,            // 1
                Insn::RetNull,         // 2 (top)
                Insn::Tick,            // 3
                Insn::LoopPop,         // 4 (end)
            ],
        );
        assert!(pass_dce(&mut c));
        // the Tick between RetNull (terminal) and LoopPop is genuinely
        // unreachable in this synthetic layout — dropped; the record
        // targets remap to the kept pcs (top → 2, end → 3)
        assert_eq!(c.insns.len(), 4);
        assert!(matches!(c.insns[0], Insn::LoopEnter(2, 3)));
    }

    #[test]
    fn prop_replaces_loads_and_respects_kills() {
        // let x = 5; f(x) shape: Const; Define(0); CallStart; Load(0); CallFinish
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(5));
        let _ = konst(&mut c, Value::Null);
        c.names = vec!["x".into(), "f".into()];
        c.insns = vec![
            Insn::Const(a),         // 0
            Insn::Define(0),        // 1
            Insn::CallStart(1, 1),  // 2 — gate runs, does NOT kill
            Insn::Load(0),          // 3 → Const(a)
            Insn::CallFinish(1, 1), // 4 — kills
            Insn::Load(0),          // 5 — after the call: NOT propagated
            Insn::Ret,              // 6
        ];
        assert!(pass_prop(&mut c));
        assert!(matches!(c.insns[3], Insn::Const(_)));
        assert!(matches!(c.insns[5], Insn::Load(0)));
        // Store kills — the binding is gone, so the Load must survive and
        // pass_prop reports NO change (nothing was replaced)
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(5));
        let b = konst(&mut c, Value::Int(9));
        c.names = vec!["x".into()];
        c.insns = vec![
            Insn::Const(a),  // 0
            Insn::Define(0), // 1
            Insn::Const(b),  // 2
            Insn::Store(0),  // 3 — kill
            Insn::Load(0),   // 4 — NOT propagated
            Insn::Ret,       // 5
        ];
        assert!(!pass_prop(&mut c));
        assert!(matches!(c.insns[4], Insn::Load(0)));
        // scope transitions kill — same contract: no replacement
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(5));
        c.names = vec!["x".into()];
        c.insns = vec![
            Insn::Const(a),  // 0
            Insn::Define(0), // 1
            Insn::PushScope, // 2 — kill-all
            Insn::PopScope,  // 3
            Insn::Load(0),   // 4 — NOT propagated (may resolve elsewhere)
            Insn::Ret,       // 5
        ];
        assert!(!pass_prop(&mut c));
        assert!(matches!(c.insns[4], Insn::Load(0)));
    }

    #[test]
    fn prop_refuses_large_strings_charge_parity() {
        let big = "x".repeat(65 * 1024);
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Str(big));
        c.names = vec!["s".into()];
        c.insns = vec![
            Insn::Const(a),  // 0
            Insn::Define(0), // 1
            Insn::Load(0),   // 2 — Load carries the >64KiB clone charge
            Insn::Ret,       // 3
        ];
        assert!(!pass_prop(&mut c));
        assert!(matches!(c.insns[2], Insn::Load(0)));
    }

    #[test]
    fn prop_then_fold_end_to_end() {
        // let a = 2; let b = 3; return a * b;  (straight line)
        let mut c = fc("t", vec![]);
        let ka = konst(&mut c, Value::Int(2));
        let kb = konst(&mut c, Value::Int(3));
        c.names = vec!["a".into(), "b".into()];
        c.insns = vec![
            Insn::Const(ka),       // 0
            Insn::Define(0),       // 1
            Insn::Const(kb),       // 2
            Insn::Define(1),       // 3
            Insn::Line(4),         // 4
            Insn::Load(0),         // 5 → Const
            Insn::Load(1),         // 6 → Const
            Insn::Bin(BinOp::Mul), // 7 → folds
            Insn::Ret,             // 8
        ];
        let ps = PassSet::O2;
        optimize_code(&mut c, &ps);
        // dce removes the fold's Nops, so the window collapses to one Const
        let s = sig(&c);
        assert_eq!(s, vec!["C", "def", "C", "def", "?", "C", "ret"]);
        assert!(matches!(c.consts.last(), Some(Value::Int(6))));
    }

    #[test]
    fn join_points_block_stale_propagation() {
        //       0 Const(5) 1 Define 2 JmpIfFalse(4) 3 Load(0)→propagated
        //       4 (join) Load(0) NOT propagated
        let mut c = fc("t", vec![]);
        let a = konst(&mut c, Value::Int(5));
        let b = konst(&mut c, Value::Bool(true));
        c.names = vec!["x".into()];
        c.insns = vec![
            Insn::Const(a),      // 0
            Insn::Define(0),     // 1
            Insn::Const(b),      // 2
            Insn::JmpIfFalse(5), // 3
            Insn::Load(0),       // 4 → propagated (same straight line)
            Insn::Ret,           // 5 (join — target)
            Insn::Load(0),       // 6 — after join: NOT propagated
            Insn::Ret,           // 7
        ];
        assert!(pass_prop(&mut c));
        assert!(matches!(c.insns[4], Insn::Const(_)));
        assert!(matches!(c.insns[6], Insn::Load(0)));
    }

    #[test]
    fn pure_binop_agrees_with_stress_kinds() {
        // spot-check the contract: Ok results and the exact Stress kinds
        // the runtime raises (full cross-product lives in interp tests).
        let r = crate::interp::pure_binop(BinOp::Add, &Value::Int(2), &Value::Int(3), 0);
        assert!(matches!(r, Some(Ok(Value::Int(5)))));
        let r = crate::interp::pure_binop(BinOp::Div, &Value::Int(1), &Value::Int(0), 0);
        match r {
            Some(Err(s)) => assert_eq!(s.kind, "unfolded"),
            other => panic!("expected Err, got {:?}", other.map(|x| x.is_ok())),
        }
        let r = crate::interp::pure_binop(
            BinOp::Add,
            &Value::Str("a".into()),
            &Value::Str("b".into()),
            0,
        );
        assert!(r.is_none(), "string concat charges — must be None");
    }

    #[test]
    fn stress_kind_spot() {
        // pure_un overflow keeps the exact kind/message the VM raises
        let err = match pure_un(UnOp::Neg, &Value::Int(i64::MIN)) {
            Some(Err(e)) => e,
            _ => panic!("expected overflow Err"),
        };
        assert_eq!(err.kind, "overflow");
        assert_eq!(err.message, "int overflow in negation (i64::MIN)");
        let _ = Stress::new("unfolded", "constructor availability");
    }
}
