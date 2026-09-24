//! genes.rs — the gene-features pipeline: methylation config (.cell), RNA
//! edit patches (.rna), splice variant selection, module loading with TAD
//! insulation, NMD sweep, ORF finder, and real-OS-thread tasks.

use crate::ast::*;
use crate::interp::{Env, Interp};
use crate::value::{Stress, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{mpsc, Arc};

// ------------------------------------------------------------ .cell config
pub fn parse_cell(src: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for line in src.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_string();
            continue;
        }
        if let Some(eq) = t.find('=') {
            let k = t[..eq].trim();
            let v = t[eq + 1..].trim().trim_matches('"').to_string();
            let key = if section.is_empty() {
                k.to_string()
            } else {
                format!("{}.{}", section, k)
            };
            out.insert(key, v);
        }
    }
    out
}

// ------------------------------------------------------------ .rna edits
/// Apply an RNA edit patch to source text. Target matches either the file
/// stem or a gene name; gene-scoped edits apply to that gene's source span
/// (best effort: from `gene <name>` to the next top-level `gene ` at col 0).
pub fn apply_rna(src: &str, patch_src: &str, file_stem: &str) -> (String, Vec<String>) {
    let mut applied = Vec::new();
    let mut text = src.to_string();
    let patch = crate::parser::parse(patch_src);
    for s in &patch.stmts {
        if let Stmt::Edit(target, reps) = s {
            let gene_target = target != file_stem && target != "anywhere";
            let mut edited = Vec::new();
            if gene_target {
                if let Some((start, end)) = gene_span(&text, target) {
                    let seg = text[start..end].to_string();
                    let mut seg2 = seg.clone();
                    for (from, to) in reps {
                        if seg2.contains(from.as_str()) {
                            seg2 = seg2.replace(from.as_str(), to);
                            applied.push(format!("{}: '{}' -> '{}'", target, from, to));
                        }
                    }
                    if seg2 != seg {
                        text.replace_range(start..end, &seg2);
                        edited.push(true);
                    }
                    let _ = edited;
                }
            } else {
                for (from, to) in reps {
                    if text.contains(from.as_str()) {
                        text = text.replace(from.as_str(), to);
                        applied.push(format!("{}: '{}' -> '{}'", target, from, to));
                    }
                }
            }
        }
    }
    (text, applied)
}

fn gene_span(src: &str, gene: &str) -> Option<(usize, usize)> {
    let start = src.find(&format!("gene {}", gene))?;
    let rest = &src[start + 5..];
    let next = rest.find("\ngene ").map(|p| start + 5 + p + 1);
    let end = next.unwrap_or(src.len());
    Some((start, end))
}

// ------------------------------------------------------------ splice choice
pub fn choose_variant(interp: &Interp, sp: &SpliceDef) -> Option<(String, Arc<GeneDef>)> {
    if sp.variants.is_empty() {
        return None;
    }
    // 1. .cell  variant.<root>
    if let Some(v) = interp.cell.get(&format!("variant.{}", sp.root)) {
        if let Some(found) = sp.variants.iter().find(|(n, _)| n == v) {
            return Some((found.0.clone(), found.1.clone()));
        }
    }
    // 2. CLI --variant (staged into cell by the CLI as "cli.variant")
    if let Some(v) = interp.cell.get("cli.variant") {
        if let Some(found) = sp.variants.iter().find(|(n, _)| n == v) {
            return Some((found.0.clone(), found.1.clone()));
        }
    }
    // 3. m6a-marked variant
    if let Some(found) = sp.variants.iter().find(|(_, d)| d.m6a) {
        return Some((found.0.clone(), found.1.clone()));
    }
    // 4. first declared
    let first = sp.variants.first().unwrap();
    Some((first.0.clone(), first.1.clone()))
}

// ------------------------------------------------------------ modules
pub fn load_module(interp: &mut Interp, path: &str) -> Result<Value, String> {
    if let Some(v) = interp.modules.get(path) {
        return Ok(v.clone());
    }
    if interp.loading.iter().any(|p| p == path) {
        return Ok(Value::Map(Rc::new(RefCell::new(Vec::new())))); // cycle: partial module
    }
    let resolved = resolve_path(interp, path)?;
    // Module loading is a read, but of a runtime-managed tree: the
    // importing file's own project directory and the standard library are
    // ALWAYS importable (otherwise no `use` works under default-deny).
    // Imports that reach OUTSIDE those trees (arbitrary disk paths) require
    // an explicit read capability — a program cannot execute random files.
    // canonicalized Windows paths carry backslashes: normalize both sides to
    // forward slashes so prefix comparison is platform-neutral
    fn under(resolved: &str, root: &Option<String>) -> bool {
        match root {
            Some(r) => match std::fs::canonicalize(r) {
                Ok(rc) => {
                    let rs = rc.to_string_lossy().replace('\\', "/");
                    resolved == rs || resolved.starts_with(&format!("{}/", rs))
                }
                Err(_) => false,
            },
            None => false,
        }
    }
    let rc_resolved = std::fs::canonicalize(&resolved)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| resolved.replace('\\', "/"));
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"));
    let std_env = std::env::var("OPERON_STD").ok();
    let managed = under(&rc_resolved, &interp.base_dir)
        || under(&rc_resolved, &cwd)
        || rc_resolved.contains("/std/")
        || under(&rc_resolved, &std_env);
    if !managed && interp.caps.enabled {
        if let Err(s) = interp.caps.check(&interp.caps.read, "read", &resolved) {
            return Err(format!("module '{}' blocked: [{}] {}", path, s.kind, s.message));
        }
    }
    let src = std::fs::read_to_string(&resolved)
        .map_err(|e| format!("cannot read '{}': {}", resolved, e))?;
    let stem = std::path::Path::new(&resolved)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // module-level .rna sidecar
    let rna_sidecar = std::path::Path::new(&resolved).with_extension("rna");
    let src = if rna_sidecar.exists() {
        let patch = std::fs::read_to_string(&rna_sidecar).unwrap_or_default();
        let (s2, applied) = apply_rna(&src, &patch, &stem);
        for a in applied {
            interp.note(0, 1, format!("rna sidecar edit {}", a));
        }
        s2
    } else {
        src
    };

    interp.loading.push(path.to_string());
    let prog = crate::parser::parse(&src);
    for n in prog.notes {
        interp.notes.push(Note { line: n.line, rung: n.rung, message: format!("[{}] {}", path, n.message) });
    }

    // Execute module in a fresh child env of global (containment per stmt).
    let menv = Env::new(Some(interp.global.clone()));
    for stmt in &prog.stmts {
        if let Err(st) = interp.exec_stmt(&menv, stmt) {
            interp.note(0, 4, format!("stress contained: [{}] {}", st.kind, st.message));
        }
    }
    interp.loading.pop();

    // Exports: anchor export wins; else all top-level names; TAD insulation.
    let has_any_anchor = !prog.anchor_exports.is_empty() || prog.tad_exports.iter().any(|(_, e)| !e.is_empty());
    let mut exports: Vec<(Value, Value)> = Vec::new();
    let mut export_set: Vec<String> = prog.anchor_exports.clone();
    for (_, exps) in &prog.tad_exports {
        export_set.extend(exps.clone());
    }
    if has_any_anchor {
        for name in &export_set {
            if let Some(v) = menv.get(name) {
                exports.push((Value::Str(name.clone()), v));
            }
        }
    } else {
        for (k, v) in menv.vars.borrow().iter() {
            if !k.starts_with('#') {
                exports.push((Value::Str(k.clone()), v.clone()));
            }
        }
        exports.sort_by(|a, b| match (&a.0, &b.0) {
            (Value::Str(x), Value::Str(y)) => x.cmp(y),
            _ => std::cmp::Ordering::Equal,
        });
    }
    let modv = Value::Map(Rc::new(RefCell::new(exports)));
    interp.modules.insert(path.to_string(), modv.clone());
    Ok(modv)
}

fn resolve_path(interp: &Interp, path: &str) -> Result<String, String> {
    let p = path.trim_end_matches(".op").to_string() + ".op";
    let mut candidates: Vec<Option<std::path::PathBuf>> = Vec::new();
    // relative to the importing file's directory first (SPEC §8)
    if let Some(base) = &interp.base_dir {
        candidates.push(Some(std::path::PathBuf::from(base).join(&p)));
    }
    candidates.push(Some(std::path::PathBuf::from(&p)));
    candidates.push(Some(std::path::PathBuf::from("std").join(&p)));
    // explicit standard-library override (not double-joined with std/)
    candidates.push(std::env::var("OPERON_STD").ok().map(|d| std::path::PathBuf::from(d).join(&p)));
    for c in candidates.into_iter().flatten() {
        if c.exists() {
            return Ok(c.to_string_lossy().to_string());
        }
    }
    Err(format!("module '{}' not found", path))
}

// ------------------------------------------------------------ NMD sweep
pub struct NmdFinding {
    pub kind: &'static str, // "premature-stop" | "untranslated"
    pub message: String,
}

/// Nonsense-mediated decay: find unreachable statements after an unconditional
/// return, and untranslated transcripts (defined, never called, not exported,
/// not enhanced, not main).
pub fn nmd_sweep(prog: &Program, called: &[String], enhanced: &[String]) -> Vec<NmdFinding> {
    let mut out = Vec::new();
    fn scan_body(body: &[Stmt], out: &mut Vec<NmdFinding>) {
        let mut returned = false;
        for s in body {
            if returned {
                out.push(NmdFinding {
                    kind: "premature-stop",
                    message: "statement after unconditional return (premature stop codon)".into(),
                });
                // still recurse into it to find nested findings
                scan_inner(s, out);
            } else {
                scan_inner(s, out);
            }
            if matches!(s, Stmt::Return(_)) {
                returned = true;
            }
        }
    }
    fn scan_inner(s: &Stmt, out: &mut Vec<NmdFinding>) {
        match s {
            Stmt::Gene(g) => {
                scan_body(&g.body, out);
                if let Some((_, gb)) = &g.guard {
                    scan_body(gb, out);
                }
            }
            Stmt::If(branches, els) => {
                for (_, b) in branches {
                    scan_body(b, out);
                }
                if let Some(e) = els {
                    scan_body(e, out);
                }
            }
            Stmt::While(_, b) | Stmt::Loop(b) | Stmt::For(_, _, b) => scan_body(b, out),
            Stmt::Block(b) | Stmt::Tad(_, b) => scan_body(b, out),
            Stmt::Frame { body, .. } => scan_body(body, out),
            Stmt::Stress { body, rescue, .. } => {
                scan_body(body, out);
                if let Some((_, rb)) = rescue {
                    scan_body(rb, out);
                }
            }
            _ => {}
        }
    }

    for s in &prog.stmts {
        if let Stmt::Gene(g) = s {
            scan_body(&g.body, &mut out);
            if let Some((_, gb)) = &g.guard {
                scan_body(gb, &mut out);
            }
        } else {
            scan_inner(s, &mut out);
        }
    }

    // untranslated transcripts
    for s in &prog.stmts {
        if let Stmt::Gene(g) = s {
            if let Some(name) = &g.name {
                if name == "main" || enhanced.contains(name) || called.contains(name) {
                    continue;
                }
                let exported = prog.anchor_exports.contains(name)
                    || prog.tad_exports.iter().any(|(_, e)| e.contains(name));
                if !exported && !g.methylate {
                    out.push(NmdFinding {
                            kind: "untranslated",
                        message: format!("gene '{}' defined but never translated (dead transcript)", name),
                    });
                }
            }
        }
    }
    out
}

/// Purge premature stops: drop statements after the first unconditional return.
pub fn purge_premature_stops(stmts: &mut Vec<Stmt>) {
    let mut returned = false;
    let mut keep = Vec::new();
    for s in stmts.drain(..) {
        let mut s = s;
        if returned {
            // still purge inside kept structures? we drop the whole statement
            continue;
        }
        match &mut s {
            Stmt::Gene(g) => {
                let g = Arc::make_mut(g);
                purge_premature_stops(&mut g.body);
            }
            Stmt::If(branches, els) => {
                for (_, b) in branches.iter_mut() {
                    purge_premature_stops(b);
                }
                if let Some(e) = els {
                    purge_premature_stops(e);
                }
            }
            Stmt::While(_, b) | Stmt::Loop(b) | Stmt::For(_, _, b) | Stmt::Block(b) | Stmt::Tad(_, b) => {
                purge_premature_stops(b);
            }
            Stmt::Frame { body, .. } => purge_premature_stops(body),
            Stmt::Stress { body, rescue, .. } => {
                purge_premature_stops(body);
                if let Some((_, rb)) = rescue {
                    purge_premature_stops(rb);
                }
            }
            _ => {}
        }
        let is_return = matches!(s, Stmt::Return(_));
        keep.push(s);
        if is_return {
            returned = true;
        }
    }
    *stmts = keep;
}

// ------------------------------------------------------------ ORF finder
pub fn find_orfs(dna: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = dna.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i + 2 < n {
        let codon: String = chars[i..i + 3].iter().collect();
        if codon == "ATG" {
            // scan to stop
            let mut protein = String::new();
            let mut j = i;
            let mut stopped = false;
            while j + 2 < n {
                let c2: String = chars[j..j + 3].iter().collect();
                if c2 == "TAA" || c2 == "TAG" || c2 == "TGA" {
                    stopped = true;
                    break;
                }
                protein.push(crate::interp::codon_table_char(&c2));
                j += 3;
            }
            if stopped && !protein.is_empty() {
                out.push(protein);
            }
        }
        i += 1;
    }
    out
}

// ------------------------------------------------------------ threads
/// Sendable mirror of Value (thread boundary serialization).
pub enum SendValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<SendValue>),
    Map(Vec<(String, SendValue)>),
    Stress(String, String),
}

/// Sendable environment snapshot entry: gene definitions cross by Arc,
/// data values cross by serialization.
pub enum SnapVal {
    Gene(Arc<GeneDef>),
    Data(SendValue),
}

/// Sendable argument: data serializes; named genes cross as references to
/// the snapshot (the worker re-binds them from its inherited repertoire).
/// Anonymous lambdas cannot cross — they arrive as null with a note.
pub enum SnapArg {
    Data(SendValue),
    GeneRef(String),
    Lambda(Arc<GeneDef>),
}

pub fn arg_to_snap(v: &Value) -> SnapArg {
    match v {
        Value::Gene(d, _) if d.name.is_some() => SnapArg::GeneRef(d.name.clone().unwrap()),
        Value::Gene(d, _) => SnapArg::Lambda(d.clone()),
        other => SnapArg::Data(to_send(other)),
    }
}

pub fn arg_from_snap(a: &SnapArg, snap: &[(String, SnapVal)]) -> Value {
    match a {
        SnapArg::Data(sv) => from_send(clone_send(sv)),
        SnapArg::GeneRef(n) => {
            for (name, sv) in snap {
                if name == n {
                    if let SnapVal::Gene(d) = sv {
                        return Value::Gene(d.clone(), None);
                    }
                }
            }
            Value::Null
        }
        SnapArg::Lambda(d) => Value::Gene(d.clone(), None),
    }
}

pub fn to_send(v: &Value) -> SendValue {
    match v {
        Value::Null => SendValue::Null,
        Value::Bool(b) => SendValue::Bool(*b),
        Value::Int(i) => SendValue::Int(*i),
        Value::Float(f) => SendValue::Float(*f),
        Value::Str(s) => SendValue::Str(s.clone()),
        Value::List(l) => SendValue::List(l.borrow().iter().map(to_send).collect()),
        Value::Map(m) => SendValue::Map(
            m.borrow()
                .iter()
                .map(|(k, v)| (k.display(), to_send(v)))
                .collect(),
        ),
        Value::Gene(_, _) => SendValue::Null,
        Value::Seq(_, _) => SendValue::Null,
        Value::Obj(d, m) => {
            // objects cross the boundary as their field map (+ identity key)
            let mut out: Vec<(String, SendValue)> = Vec::new();
            out.push(("#phenotype".into(), SendValue::Str(d.name.clone())));
            for (k, v) in m.borrow().iter() {
                out.push((k.display(), to_send(v)));
            }
            SendValue::Map(out)
        }
    }
}

pub fn from_send(v: SendValue) -> Value {
    match v {
        SendValue::Null => Value::Null,
        SendValue::Bool(b) => Value::Bool(b),
        SendValue::Int(i) => Value::Int(i),
        SendValue::Float(f) => Value::Float(f),
        SendValue::Str(s) => Value::Str(s),
        SendValue::List(l) => Value::List(Rc::new(RefCell::new(l.into_iter().map(from_send).collect()))),
        SendValue::Map(m) => Value::Map(Rc::new(RefCell::new(
            m.into_iter().map(|(k, v)| (Value::Str(k), from_send(v))).collect(),
        ))),
        SendValue::Stress(k, m) => Value::Map(Rc::new(RefCell::new(vec![
            (Value::Str("kind".into()), Value::Str(k)),
            (Value::Str("message".into()), Value::Str(m)),
        ]))),
    }
}

/// Snapshot the parent global environment for a worker thread: gene
/// definitions cross by Arc (they are immutable ASTs), data values cross by
/// SendValue serialization. This gives tasks and sequences visibility of the
/// whole module surface — cells share metabolites through signals, and worker
/// cells inherit the module's gene repertoire.
pub fn snapshot_globals(interp: &Interp) -> Vec<(String, SnapVal)> {
    let mut out: Vec<(String, SnapVal)> = Vec::new();
    for (name, v) in interp.global.vars.borrow().iter() {
        match v {
            Value::Gene(d, _) => out.push((name.clone(), SnapVal::Gene(d.clone()))),
            other => {
                let sv = to_send(other);
                out.push((name.clone(), SnapVal::Data(sv)));
            }
        }
    }
    out
}

pub fn bind_snapshot(env: &Rc<Env>, snap: &[(String, SnapVal)]) {
    for (name, sv) in snap {
        let v = match sv {
            SnapVal::Gene(d) => Value::Gene(d.clone(), None),
            SnapVal::Data(sv) => from_send(clone_send(sv)),
        };
        env.define(name, v);
    }
}

/// Snapshot the global environment PLUS a closure's captured scope chain, so
/// spawned tasks see their lexical captures (SPEC §13: capture is by value
/// at spawn time). Genes cross by Arc; data crosses by serialization.
pub fn snapshot_with_closure(interp: &Interp, closure: Option<&Rc<Env>>) -> Vec<(String, SnapVal)> {
    let mut out = snapshot_globals(interp);
    let mut node = closure.cloned();
    let mut hops = 0usize;
    while let Some(env) = node {
        hops += 1;
        if hops > 64 || Rc::ptr_eq(&env, &interp.global) {
            break;
        }
        for (name, v) in env.vars.borrow().iter() {
            match v {
                Value::Gene(d, _) => out.push((name.clone(), SnapVal::Gene(d.clone()))),
                other => out.push((name.clone(), SnapVal::Data(to_send(other)))),
            }
        }
        node = env.parent.clone();
    }
    out
}

/// Deep-clone a SendValue (cloning via serialization round-trip).
fn clone_send(sv: &SendValue) -> SendValue {
    match sv {
        SendValue::Null => SendValue::Null,
        SendValue::Bool(b) => SendValue::Bool(*b),
        SendValue::Int(i) => SendValue::Int(*i),
        SendValue::Float(f) => SendValue::Float(*f),
        SendValue::Str(s) => SendValue::Str(s.clone()),
        SendValue::List(l) => SendValue::List(l.iter().map(clone_send).collect()),
        SendValue::Map(m) => SendValue::Map(m.iter().map(|(k, v)| (k.clone(), clone_send(v))).collect()),
        SendValue::Stress(k, m) => SendValue::Stress(k.clone(), m.clone()),
    }
}

// ------------------------------------------------------------ sequences
/// Start a sequence worker: rendezvous channel makes pulls lazy — the worker
/// blocks on each yield until the consumer pulls the next value.
pub fn seq_start(
    def: Arc<GeneDef>,
    args: Vec<Value>,
    snap: Vec<(String, SnapVal)>,
    caps: crate::interp::Caps,
) -> Rc<RefCell<crate::value::SeqState>> {
    let send_args: Vec<SnapArg> = args.iter().map(arg_to_snap).collect();
    let host_caps = caps;
    let (tx, rx) = mpsc::sync_channel::<crate::value::SeqMsg>(0);
    std::thread::spawn(move || {
        let mut ti = Interp::new();
        let genv = Env::new(None);
        bind_snapshot(&genv, &snap);
        ti.global = genv;
        ti.caps = host_caps; // worker cells inherit the host's grants
        let conv_args: Vec<Value> = send_args.iter().map(|a| arg_from_snap(a, &snap)).collect();
        // run the sequence body: params bound, Yield sends over the channel
        let result = run_seq_body(&mut ti, &def, conv_args, &tx);
        let (notes, stress) = match result {
            Ok(()) => (ti.notes, None),
            Err(s) => (ti.notes, Some((s.kind, s.message))),
        };
        let _ = tx.send(crate::value::SeqMsg::Done(notes, stress));
    });
    Rc::new(RefCell::new(crate::value::SeqState {
        rx: Some(rx),
        done: false,
        stress: None,
    }))
}

fn run_seq_body(
    ti: &mut Interp,
    def: &Arc<GeneDef>,
    args: Vec<Value>,
    tx: &mpsc::SyncSender<crate::value::SeqMsg>,
) -> Result<(), Stress> {
    ti.seq_tx = Some(tx.clone());
    let fenv = Env::new(Some(ti.global.clone()));
    for (i, (pname, default)) in def.params.iter().enumerate() {
        if pname.is_empty() || pname == "?" {
            continue;
        }
        if let Some(a) = args.get(i) {
            fenv.define(pname, a.clone());
        } else if let Some(d) = default {
            let dv = ti.eval(&fenv, d).unwrap_or(Value::Null);
            fenv.define(pname, dv);
        } else {
            fenv.define(pname, Value::Null);
        }
    }
    // Yield is the only interesting flow: everything else terminates the
    // sequence (return value is dropped — sequences are streams, not answers).
    let _ = ti.exec_block(&fenv, &def.body)?;
    Ok(())
}

pub fn spawn_task(interp: &mut Interp, callee: Value, args: Vec<Value>) -> Result<Value, Stress> {
    // Only top-level named genes can cross the thread boundary; the worker
    // inherits the whole module gene repertoire via snapshot (definitions by
    // Arc, data by SendValue serialization).
    let def: Arc<GeneDef> = match &callee {
        Value::Gene(d, None) => d.clone(),
        Value::Gene(d, Some(_)) => d.clone(),
        _ => {
            interp.note(0, 4, "spawn() needs a gene; null task");
            return Ok(Value::Int(-1));
        }
    };
    let name = def.name.clone().unwrap_or_else(|| "<lambda>".into());
    if def_has_lambda(&def) {
        interp.note(
            0,
            4,
            format!("spawn: gene '{}' contains closures; running inline (id 0)", name),
        );
        let _ = interp.call_gene(def, None, args)?;
        return Ok(Value::Int(0));
    }
    let snap = match &callee {
        Value::Gene(_, Some(cl)) => snapshot_with_closure(interp, Some(cl)),
        _ => snapshot_globals(interp),
    };
    let send_args: Vec<SnapArg> = args.iter().map(arg_to_snap).collect();
    let (tx, rx) = mpsc::channel::<(SendValue, Vec<Note>)>();
    let global_note = format!("[task {}]", name);
    let host_caps = interp.caps.clone();
    std::thread::spawn(move || {
        let mut ti = Interp::new();
        let genv = Env::new(None);
        bind_snapshot(&genv, &snap);
        ti.global = genv;
        ti.caps = host_caps; // worker cells inherit the host's grants
        let conv_args: Vec<Value> = send_args.iter().map(|a| arg_from_snap(a, &snap)).collect();
        let result = ti.call_gene(def, None, conv_args);
        let (rv, notes) = match result {
            Ok(v) => (to_send(&v), ti.notes),
            Err(s) => (SendValue::Stress(s.kind, s.message), ti.notes),
        };
        let notes = notes
            .into_iter()
            .map(|mut n| {
                n.message = format!("{} {}", global_note, n.message);
                n
            })
            .collect();
        let _ = tx.send((rv, notes));
    });
    let id = interp.next_task_id;
    interp.next_task_id += 1;
    interp.tasks.insert(id, crate::interp::TaskHandle { rx });
    Ok(Value::Int(id))
}

fn def_has_lambda(def: &GeneDef) -> bool {
    fn expr_has(e: &Expr) -> bool {
        match e {
            Expr::Lambda(_) => true,
            Expr::Unary(_, a) | Expr::Member(a, _) => expr_has(a),
            Expr::Binary(_, a, b) | Expr::Index(a, b) => expr_has(a) || expr_has(b),
            Expr::Call(a, args) | Expr::Method(a, _, args) => expr_has(a) || args.iter().any(expr_has),
            Expr::List(xs) => xs.iter().any(expr_has),
            Expr::Map(pairs) => pairs.iter().any(|(k, v)| expr_has(k) || expr_has(v)),
            Expr::Interp(ps) => ps
                .iter()
                .any(|p| matches!(p, InterpPart::Expr(e) if expr_has(e))),
            Expr::Collect { iter, filter, body, .. } => {
                expr_has(iter) || filter.as_ref().map(|f| expr_has(f)).unwrap_or(false) || expr_has(body)
            }
            _ => false,
        }
    }
    fn stmts_have(stmts: &[Stmt]) -> bool {
        stmts.iter().any(stmt_has)
    }
    fn stmt_has(s: &Stmt) -> bool {
        match s {
            Stmt::Let(_, e) | Stmt::Assign(_, _, e) | Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => expr_has(e),
            Stmt::IndexAssign(t, i, _, e) => expr_has(t) || expr_has(i) || expr_has(e),
            Stmt::MemberAssign(t, _, _, e) => expr_has(t) || expr_has(e),
            Stmt::If(bs, els) => {
                bs.iter().any(|(c, b)| expr_has(c) || stmts_have(b))
                    || els.as_ref().map(|e| stmts_have(e)).unwrap_or(false)
            }
            Stmt::While(c, b) => expr_has(c) || stmts_have(b),
            Stmt::Loop(b) => stmts_have(b),
            Stmt::For(_, it, b) => expr_has(it) || stmts_have(b),
            Stmt::Match(sub, cases) => {
                expr_has(sub)
                    || cases.iter().any(|(p, b)| {
                        let pe = match p {
                            MatchPat::Lit(e) => expr_has(e),
                            MatchPat::Multi(ls) => ls.iter().any(expr_has),
                            _ => false,
                        };
                        pe || stmts_have(b)
                    })
            }
            Stmt::Stress { body, rescue, .. } => {
                stmts_have(body) || rescue.as_ref().map(|(_, rb)| stmts_have(rb)).unwrap_or(false)
            }
            Stmt::Gene(g) => stmts_have(&g.body),
            _ => false,
        }
    }
    stmts_have(&def.body) || def.guard.as_ref().map(|(_, b)| stmts_have(b)).unwrap_or(false)
}

pub fn join_task(interp: &mut Interp, id: i64) -> Result<Value, Stress> {
    // join by id; id 0 means "inline run already returned" (see spawn)
    if id == 0 {
        return Ok(Value::Null);
    }
    if id < 0 {
        interp.note(0, 4, "join() needs a task id");
        return Ok(Value::Null);
    }
    let handle = match interp.tasks.remove(&id) {
        Some(h) => h,
        None => {
            interp.note(0, 4, format!("task {} already joined or unknown", id));
            return Ok(Value::Null);
        }
    };
    match handle.rx.recv() {
        Ok((v, notes)) => {
            for n in notes {
                interp.notes.push(n);
            }
            Ok(from_send(v))
        }
        Err(_) => {
            interp.note(0, 4, format!("task {} channel closed", id));
            Ok(Value::Null)
        }
    }
}
