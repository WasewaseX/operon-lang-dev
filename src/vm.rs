//! vm.rs — W09 (A2/A3): the bytecode interpreter.
//!
//! One method on `Interp` (`vm_exec`) executes a `FuncCode` against an `Env`
//! frame. Rule W09-1 (docs/VM.md): every semantics-bearing operation
//! delegates to the same funnel the tree-walk uses, so byte-parity is by
//! construction; the VM owns control flow, scoping, and operand plumbing
//! only. Rule W09-2: `cur_line` stamps ride `Insn::Line` at exactly the
//! points the tree-walk arms stamp.

use crate::compile::{FuncCode, Insn};
use crate::interp::{ann_matches, charge_clone, Env, Flow, Interp};
use crate::value::{SeqState, Stress, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// RISC silencing decision from `CallStart`, consumed by `CallFinish`.
enum Mark {
    /// captured; the replacement was pre-resolved at decision time (the
    /// tree-walk resolves `env.get(to)` before evaluating arguments)
    Redirect(Value),
    /// degraded: arguments are skipped, result is null
    Degraded,
    /// ordinary named call through call_named
    Normal,
}

/// Loop iteration state (For statement).
enum Iter {
    /// materialized items + cursor (list clone / string chars / map keys)
    Items(Vec<Value>, usize),
    /// lazy sequence pulls
    Seq(Rc<RefCell<SeqState>>),
}

/// Runtime loop record: `end` is the LoopPop pc, `top` the continue target.
struct VLoop {
    end: usize,
    top: usize,
    base_env: Rc<Env>,
    base_catches: usize,
}

/// Runtime catch record pushed by `CatchEnter`; holds the frame state to
/// restore when the handler fires.
struct RCatch {
    start: usize,
    end: usize,
    handler: usize,
    kind: Option<String>,
    base_env: Rc<Env>,
    base_stack: usize,
    base_iters: usize,
    base_marks: usize,
    base_loops: usize,
}

impl Interp {
    /// The RISC silencing gate, mirrored instruction-for-instruction from
    /// eval's Call arm (reg-bio-3 C9): filter, immunity, survival product,
    /// xorshift draw, notes. The decision happens HERE — before argument
    /// evaluation, exactly like the tree-walk — so the RNG stream position
    /// and note order match byte-for-byte.
    fn risc_gate(&mut self, env: &Rc<Env>, name: &str) -> Result<Mark, Stress> {
        let entries: Vec<(String, Option<String>, f64, u32)> = self
            .silences
            .iter()
            .filter(|(f, _, _, _)| f == name)
            .cloned()
            .collect();
        let Some((_, first_to, first_s, first_sites)) = entries.first().cloned() else {
            return Ok(Mark::Normal);
        };
        // acetylated genes are immune (checked BEFORE any draw — immunity
        // consumes no randomness)
        let immune = match env.get(name) {
            Some(crate::value::Value::Gene(d, _)) => d.acetylate,
            _ => false,
        };
        if immune {
            return Ok(Mark::Normal);
        }
        let mut surv = 1.0f64;
        for (_, _, s, sites) in &entries {
            let base = 1.0 - *s;
            let mut k = 0;
            while k < *sites {
                surv *= base;
                k += 1;
            }
        }
        let p = 1.0 - surv;
        // capture decision: deterministic draw on the shared mirrored
        // xorshift64* stream; only when genuinely probabilistic
        let captured = if p >= 1.0 {
            true
        } else {
            let mut x = self.rng;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.rng = x;
            let u = ((x >> 11) as f64) / 9_007_199_254_740_992.0;
            if u < p {
                true
            } else {
                // escape: the call proceeds through the pinned funnel; note
                // once per gene
                if self.risc_escaped.insert(name.to_string()) {
                    self.note(
                        0,
                        4,
                        format!(
                            "RISC escape: '{}' escaped silencing (strength {}, sites {})",
                            name,
                            crate::value::format_float(first_s),
                            first_sites
                        ),
                    );
                }
                false
            }
        };
        if !captured {
            return Ok(Mark::Normal);
        }
        match first_to {
            Some(to) => {
                self.note(
                    0,
                    4,
                    format!("RISC: call to '{}' silenced → '{}'", name, to),
                );
                let target = env.get(&to).unwrap_or(Value::Null);
                Ok(Mark::Redirect(target))
            }
            // reg-bio (F-4): pure degradation — no replacement executes
            None => {
                self.note(
                    0,
                    4,
                    format!("RISC: call to '{}' degraded (no replacement)", name),
                );
                Ok(Mark::Degraded)
            }
        }
    }

    /// Execute compiled bytecode for one gene body. Returns the same `Flow`
    /// contract as `exec_block` so the call funnel's boundary semantics
    /// (propagation conversion, return annotations, profiler close) apply
    /// unchanged above this line.
    pub fn vm_exec(&mut self, code: &FuncCode, env0: &Rc<Env>) -> Result<Flow, Stress> {
        let insns = &code.insns;
        let consts = &code.consts;
        let names = &code.names;
        let mut env: Rc<Env> = env0.clone();
        let mut stack: Vec<Value> = Vec::new();
        let mut marks: Vec<Mark> = Vec::new();
        let mut iters: Vec<Iter> = Vec::new();
        let mut catches: Vec<RCatch> = Vec::new();
        let mut loops: Vec<VLoop> = Vec::new();
        let mut pc: usize = 0;
        // the stress riding between the catch machinery and RescueBind
        let mut pending: Option<Stress> = None;

        'exec: loop {
            if pc >= insns.len() {
                return Ok(Flow::Norm);
            }
            // one dispatch step; errors go through the catch machinery below
            let step: Result<(), Stress> = 'insn: {
                match &insns[pc] {
                    // ---- operands ----
                    Insn::Const(k) => {
                        stack.push(consts[*k as usize].clone());
                    }
                    Insn::Pop => {
                        stack.pop();
                    }
                    Insn::Dup => {
                        let v = stack.last().expect("dup on empty stack").clone();
                        stack.push(v);
                    }
                    Insn::Load(k) => {
                        let name = &names[*k as usize];
                        match env.get(name) {
                            Some(v) => {
                                // sec-r5 (F-9): the read is a charged deep copy
                                charge_clone(&v)?;
                                stack.push(v);
                            }
                            None => {
                                self.note(0, 4, format!("unbound '{}' read as null", name));
                                stack.push(Value::Null);
                            }
                        }
                    }
                    Insn::LoadPlain(k) => {
                        let name = &names[*k as usize];
                        stack.push(env.get(name).unwrap_or(Value::Null));
                    }
                    Insn::Define(k) => {
                        let v = stack.pop().expect("define on empty stack");
                        let name = &names[*k as usize];
                        if env.get(name).is_some() {
                            self.note(0, 4, format!("rebinding '{}'", name));
                        }
                        env.define(name, v);
                    }
                    Insn::DefineAnn(k, ann) => {
                        let v = stack.pop().expect("define on empty stack");
                        let name = &names[*k as usize];
                        // W01: soft contract — mismatch = catchable unfolded
                        // stress; the binding does NOT happen
                        if !ann_matches(&v, ann) {
                            break 'insn Err(Stress::new(
                                "unfolded",
                                format!(
                                    "type annotation violated: '{}' expects {}, got {}",
                                    name,
                                    ann.render(),
                                    v.type_name()
                                ),
                            ));
                        }
                        if env.get(name).is_some() {
                            self.note(0, 4, format!("rebinding '{}'", name));
                        }
                        env.define(name, v);
                    }
                    Insn::Store(k) => {
                        let v = stack.pop().expect("store on empty stack");
                        let name = &names[*k as usize];
                        if !env.set(name, v) {
                            self.note(0, 4, format!("'{}' was not declared; auto-declared", name));
                        }
                    }
                    Insn::StoreOp(k, op) => {
                        // order parity: the RHS was evaluated first; the
                        // current-value read happens now, without charge or
                        // note (the Assign op= arm)
                        let v = stack.pop().expect("store-op on empty stack");
                        let name = &names[*k as usize];
                        let cur = env.get(name).unwrap_or(Value::Null);
                        let newv = self.apply_binop(&env, *op, &cur, &v)?;
                        if !env.set(name, newv) {
                            self.note(0, 4, format!("'{}' was not declared; auto-declared", name));
                        }
                    }
                    // ---- operators ----
                    Insn::Line(l) => {
                        self.cur_line = *l as usize;
                    }
                    Insn::Bin(op) => {
                        let r = stack.pop().expect("bin on empty stack");
                        let l = stack.pop().expect("bin on empty stack");
                        match self.apply_binop(&env, *op, &l, &r) {
                            Ok(v) => stack.push(v),
                            Err(e) => break 'insn Err(e),
                        }
                    }
                    Insn::AndJmp(t) => {
                        let short = {
                            let lv = stack.last().expect("andjmp on empty stack");
                            !lv.truthy()
                        };
                        if short {
                            // the left value IS the result; skip the right side
                            pc = *t as usize;
                            continue 'exec;
                        }
                        stack.pop(); // the right-side value overwrites it
                    }
                    Insn::OrJmp(t) => {
                        let short = {
                            let lv = stack.last().expect("orjmp on empty stack");
                            lv.truthy()
                        };
                        if short {
                            pc = *t as usize;
                            continue 'exec;
                        }
                        stack.pop();
                    }
                    Insn::NullishJmp(t) => {
                        let short = {
                            let lv = stack.last().expect("nullishjmp on empty stack");
                            !matches!(lv, Value::Null)
                        };
                        if short {
                            pc = *t as usize;
                            continue 'exec;
                        }
                        stack.pop();
                    }
                    Insn::Un(op) => {
                        let v = stack.pop().expect("un on empty stack");
                        // exact mirror of the Unary arm
                        let out = match op {
                            crate::ast::UnOp::Neg => match v {
                                Value::Int(i) => i.checked_neg().map(Value::Int).ok_or_else(|| {
                                    Stress::new("overflow", "int overflow in negation (i64::MIN)")
                                }),
                                Value::Float(f) => Ok(Value::Float(-f)),
                                other => Err(Stress::new(
                                    "unfolded",
                                    format!("cannot negate {}", other.type_name()),
                                )),
                            },
                            crate::ast::UnOp::Not => Ok(Value::Bool(!v.truthy())),
                            crate::ast::UnOp::BitNot => match v {
                                Value::Int(i) => Ok(Value::Int(!i)),
                                Value::Bool(b) => Ok(Value::Int(!(b as i64))),
                                other => Err(Stress::new(
                                    "unfolded",
                                    format!("cannot bit-invert {}", other.type_name()),
                                )),
                            },
                        };
                        match out {
                            Ok(v) => stack.push(v),
                            Err(e) => break 'insn Err(e),
                        }
                    }
                    Insn::Index => {
                        let iv = stack.pop().expect("index on empty stack");
                        let tv = stack.pop().expect("index on empty stack");
                        // exact mirror of the Index arm (cur_line was stamped
                        // at arm entry by the Line insn; operand evaluations
                        // that stamp did so identically on both engines)
                        let out = match (&tv, &iv) {
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
                        };
                        match out {
                            Ok(v) => stack.push(v),
                            Err(e) => break 'insn Err(e),
                        }
                    }
                    Insn::Member(k) => {
                        let tv = stack.pop().expect("member on empty stack");
                        let key = &names[*k as usize];
                        match self.member_value(tv, key) {
                            Ok(v) => stack.push(v),
                            Err(e) => break 'insn Err(e),
                        }
                    }
                    Insn::MemberSafe(k) => {
                        let tv = stack.pop().expect("member on empty stack");
                        let key = &names[*k as usize];
                        if matches!(tv, Value::Null) {
                            stack.push(Value::Null);
                        } else {
                            match self.member_value(tv, key) {
                                Ok(v) => stack.push(v),
                                Err(e) => break 'insn Err(e),
                            }
                        }
                    }
                    Insn::Method(k, argc) => {
                        let name = &names[*k as usize];
                        let args = stack.split_off(stack.len() - *argc as usize);
                        let tv = stack.pop().expect("method on empty stack");
                        match self.call_method(&env, tv, name, args) {
                            Ok(v) => stack.push(v),
                            Err(e) => break 'insn Err(e),
                        }
                    }
                    Insn::MethodSafe(k, argc) => {
                        let name = &names[*k as usize];
                        let args = stack.split_off(stack.len() - *argc as usize);
                        let tv = stack.pop().expect("method on empty stack");
                        if matches!(tv, Value::Null) {
                            stack.push(Value::Null);
                        } else {
                            match self.call_method(&env, tv, name, args) {
                                Ok(v) => stack.push(v),
                                Err(e) => break 'insn Err(e),
                            }
                        }
                    }
                    Insn::MakeList(n) => {
                        let items = stack.split_off(stack.len() - *n as usize);
                        stack.push(Value::List(Rc::new(RefCell::new(items))));
                    }
                    Insn::MakeMap(n) => {
                        let flat = stack.split_off(stack.len() - 2 * *n as usize);
                        let m: crate::value::MapRef =
                            Rc::new(RefCell::new(crate::value::MapStore::default()));
                        // per-pair insert order mirrors the arm (later
                        // duplicate keys overwrite, keeping first position)
                        for pair in flat.chunks(2) {
                            let kv = &pair[0];
                            let vv = &pair[1];
                            let key = match crate::value::key_scalar(kv) {
                                Some(k) => k,
                                None => {
                                    self.note(0, 4, "map key must be scalar; key stringified");
                                    Value::Str(kv.display())
                                }
                            };
                            self.map_insert(&m, key, vv.clone());
                        }
                        stack.push(Value::Map(m));
                    }
                    // ---- control ----
                    Insn::Jmp(t) => {
                        pc = *t as usize;
                        continue 'exec;
                    }
                    Insn::JmpIfFalse(t) => {
                        let v = stack.pop().expect("jmpiffalse on empty stack");
                        if !v.truthy() {
                            pc = *t as usize;
                            continue 'exec;
                        }
                    }
                    Insn::Tick => {
                        self.tick()?;
                    }
                    Insn::PushScope => {
                        env = Env::new(Some(env.clone()));
                    }
                    Insn::PopScope => {
                        let parent = env.parent.clone().expect("pop without push");
                        env = parent;
                    }
                    Insn::Ret => {
                        let v = stack.pop().expect("ret on empty stack");
                        return Ok(Flow::Ret(v));
                    }
                    Insn::RetNull => {
                        return Ok(Flow::Ret(Value::Null));
                    }
                    // ---- calls ----
                    Insn::CallStart(k, line) => {
                        self.cur_line = *line as usize;
                        let name = &names[*k as usize];
                        match self.risc_gate(&env, name) {
                            Ok(m) => marks.push(m),
                            Err(e) => break 'insn Err(e),
                        }
                    }
                    Insn::IfDegraded(argc, t) => {
                        if matches!(marks.last(), Some(Mark::Degraded)) {
                            marks.pop();
                            stack.push(Value::Null);
                            // arguments were skipped; land past CallFinish
                            pc = *t as usize;
                            continue 'exec;
                        }
                        let _ = argc;
                    }
                    Insn::CallFinish(argc, k) => {
                        let args = if *argc > 0 {
                            stack.split_off(stack.len() - *argc as usize)
                        } else {
                            Vec::new()
                        };
                        let mark = marks.pop().expect("callfinish without mark");
                        let name = &names[*k as usize];
                        let out = match mark {
                            Mark::Redirect(target) => self.call_value(&env, &target, args),
                            Mark::Normal => self.call_named(&env, name, args),
                            Mark::Degraded => Ok(Value::Null),
                        };
                        match out {
                            Ok(v) => stack.push(v),
                            Err(e) => break 'insn Err(e),
                        }
                    }
                    Insn::CallValue(argc) => {
                        let args = if *argc > 0 {
                            stack.split_off(stack.len() - *argc as usize)
                        } else {
                            Vec::new()
                        };
                        let cv = stack.pop().expect("callvalue on empty stack");
                        match self.call_value(&env, &cv, args) {
                            Ok(v) => stack.push(v),
                            Err(e) => break 'insn Err(e),
                        }
                    }
                    // ---- loops ----
                    Insn::IterMake => {
                        let itv = stack.pop().expect("itermake on empty stack");
                        if let Value::Seq(_def, st) = itv {
                            iters.push(Iter::Seq(st));
                        } else {
                            // materialize exactly like the For arm
                            let items: Vec<Value> = match itv {
                                Value::List(l) => l.borrow().clone(),
                                Value::Str(s) => {
                                    s.chars().map(|c| Value::Str(c.to_string())).collect()
                                }
                                Value::Map(m) => {
                                    m.borrow().iter().map(|(k, _)| k.clone()).collect()
                                }
                                other => {
                                    self.note(
                                        0,
                                        4,
                                        format!(
                                            "cannot iterate {}; loop skipped",
                                            other.type_name()
                                        ),
                                    );
                                    Vec::new()
                                }
                            };
                            iters.push(Iter::Items(items, 0));
                        }
                    }
                    Insn::IterNext(t) => {
                        let item = match iters.last_mut().expect("iternext without iter") {
                            Iter::Seq(st) => {
                                // the sequence path ticks BEFORE the pull
                                self.tick()?;
                                self.seq_pull(st)?
                            }
                            Iter::Items(items, cur) => {
                                if *cur < items.len() {
                                    let v = items[*cur].clone();
                                    *cur += 1;
                                    // the materialized path ticks AFTER fetch
                                    self.tick()?;
                                    Some(v)
                                } else {
                                    None
                                }
                            }
                        };
                        match item {
                            Some(v) => stack.push(v),
                            None => {
                                pc = *t as usize;
                                continue 'exec;
                            }
                        }
                    }
                    Insn::IterBindName(k) => {
                        let v = stack.pop().expect("iterbind on empty stack");
                        let name = &names[*k as usize];
                        env.define(name, v);
                    }
                    Insn::IterEnd => {
                        iters.pop();
                    }
                    Insn::LoopEnter(top, end) => {
                        loops.push(VLoop {
                            end: *end as usize,
                            top: *top as usize,
                            base_env: env.clone(),
                            base_catches: catches.len(),
                        });
                    }
                    Insn::LoopPop => {
                        loops.pop();
                    }
                    // ---- stress ----
                    Insn::CatchEnter(kind, end, handler) => {
                        catches.push(RCatch {
                            start: pc + 1,
                            end: *end as usize,
                            handler: *handler as usize,
                            kind: kind.map(|k| names[k as usize].clone()),
                            base_env: env.clone(),
                            base_stack: stack.len(),
                            base_iters: iters.len(),
                            base_marks: marks.len(),
                            base_loops: loops.len(),
                        });
                    }
                    Insn::CatchLeave => {
                        catches.pop();
                    }
                    Insn::CatchTrim(n) => {
                        catches.truncate(*n as usize);
                    }
                    Insn::RescueTicks => {
                        // rescue entry is real work: charged exactly like the
                        // arm (a retry-spin cannot dodge the fuel counter)
                        for _ in 0..64 {
                            self.tick()?;
                        }
                    }
                    Insn::RescueBind(k) => {
                        let s = pending.take().expect("rescuebind without stress");
                        if let Some(k) = k {
                            let m = self.stress_map(&s);
                            env.define(&names[*k as usize], m);
                        }
                    }
                    Insn::RaiseStmt(kind, line) => {
                        let mv = stack.pop().expect("raise on empty stack");
                        break 'insn Err(Stress {
                            kind: kind
                                .map(|k| names[k as usize].clone())
                                .unwrap_or_else(|| "unfolded".into()),
                            message: mv.display(),
                            // W007: the raise carries its OWN line
                            line: *line as usize,
                            chain: Vec::new(),
                            // D-014: raise can never forge propagation
                            prop: None,
                        });
                    }
                    // ---- delegation (total coverage) ----
                    Insn::Stmt(s) => match self.exec_stmt(&env, s) {
                        Ok(Flow::Norm) => {}
                        Ok(Flow::Ret(v)) => return Ok(Flow::Ret(v)),
                        Ok(Flow::Brk) => {
                            // arrived from delegated content: restore to the
                            // nearest compiled loop, or pass through
                            if let Some(l) = loops.last() {
                                catches.truncate(l.base_catches);
                                env = l.base_env.clone();
                                pc = l.end; // the LoopPop pops the record
                                continue 'exec;
                            }
                            return Ok(Flow::Brk);
                        }
                        Ok(Flow::Cont) => {
                            if let Some(l) = loops.last() {
                                catches.truncate(l.base_catches);
                                env = l.base_env.clone();
                                pc = l.top;
                                continue 'exec;
                            }
                            return Ok(Flow::Cont);
                        }
                        Err(e) => break 'insn Err(e),
                    },
                    Insn::Expr(e) => {
                        let v = self.eval(&env, e)?;
                        stack.push(v);
                    }
                }
                pc += 1;
                Ok(())
            };
            // ---- catch machinery (runs only on a step error) ----
            if let Err(err) = step {
                // W06 (D-014): propagation is a RETURN, pre-armed BEFORE kind
                // matching at every catch site
                if let Some(p) = err.prop {
                    return Ok(Flow::Ret(p));
                }
                let mut s = Some(err);
                let mut handled = false;
                for ci in (0..catches.len()).rev() {
                    let in_range = pc >= catches[ci].start && pc <= catches[ci].end;
                    if !in_range {
                        continue;
                    }
                    let kind_ok = match &catches[ci].kind {
                        None => true,
                        Some(k) => s.as_ref().is_some_and(|st| k == &st.kind || k == "any"),
                    };
                    if kind_ok {
                        let c = catches.remove(ci);
                        stack.truncate(c.base_stack);
                        iters.truncate(c.base_iters);
                        marks.truncate(c.base_marks);
                        loops.truncate(c.base_loops);
                        env = c.base_env;
                        pending = s.take();
                        pc = c.handler;
                        handled = true;
                        break;
                    }
                    // kind mismatch: keep searching outward (the arm's
                    // `return Err` semantics)
                }
                if !handled {
                    return Err(s.unwrap());
                }
            }
        }
    }
}
