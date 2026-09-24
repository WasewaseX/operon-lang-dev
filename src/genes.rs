//! genes.rs — the gene-features pipeline: methylation config (.cell), RNA
//! edit patches (.rna), splice variant selection, module loading with TAD
//! insulation, NMD sweep, ORF finder, and real-OS-thread tasks.

use crate::ast::*;
use crate::interp::{Env, Interp};
use crate::value::{Stress, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

// ------------------------------------------------------------ thread budget
// A run may hold at most MAX_THREADS live worker cells. Exceeding the cap is
// a catchable `overflow` stress, never a panic — thread bombs are contained.
const MAX_THREADS: usize = 256;
const WORKER_STACK: usize = 256 * 1024 * 1024; // match-class native stack
static LIVE_THREADS: AtomicUsize = AtomicUsize::new(0);

struct ThreadGuard;
impl Drop for ThreadGuard {
    fn drop(&mut self) {
        LIVE_THREADS.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn spawn_worker(f: impl FnOnce() + Send + 'static) -> Result<(), Stress> {
    let live = LIVE_THREADS.fetch_add(1, Ordering::Relaxed);
    if live >= MAX_THREADS {
        LIVE_THREADS.fetch_sub(1, Ordering::Relaxed);
        return Err(Stress::new(
            "overflow",
            format!(
                "thread cap ({}) reached — too many live workers",
                MAX_THREADS
            ),
        ));
    }
    let res = std::thread::Builder::new()
        .name("operon-worker".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _g = ThreadGuard;
            f();
        });
    match res {
        Ok(_) => Ok(()),
        Err(e) => {
            LIVE_THREADS.fetch_sub(1, Ordering::Relaxed);
            Err(Stress::new(
                "overflow",
                format!("cannot spawn worker thread: {}", e),
            ))
        }
    }
}

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
    // cyclic re-entry FIRST (the placeholder below is also in the cache, so
    // this check must precede the cache hit or the §19 note would be dead)
    if interp.loading.iter().any(|p| p == path) {
        // cyclic import: return the PLACEHOLDER map registered at load start.
        // When the loading module finishes, this same Rc<RefCell> is filled,
        // so the first importer sees the completed data too (wave-3 Critic-L:
        // the old empty-map return poisoned the top-level importer).
        interp.note(
            0,
            4,
            format!(
            "cyclic import of '{}' — module still loading; its map fills when loading completes",
            path
        ),
        );
        if let Some(v) = interp.modules.get(path) {
            return Ok(v.clone());
        }
        return Ok(Value::Map(Rc::new(RefCell::new(Vec::new()))));
    }
    if let Some(v) = interp.modules.get(path) {
        return Ok(v.clone());
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
    // the managed std library is the REAL std directory (exe-relative,
    // OPERON_STD override, or the interpreter's own tree) — never any path
    // that merely *contains* a `/std/` component
    fn is_std_tree(resolved: &str, std_env: &Option<String>) -> bool {
        let mut roots: Vec<String> = Vec::new();
        if let Some(e) = std_env {
            if let Ok(rc) = std::fs::canonicalize(e) {
                roots.push(rc.to_string_lossy().replace('\\', "/"));
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(bin) = exe.parent() {
                if let Some(root) = bin.parent() {
                    let c = root.join("std");
                    if c.is_dir() {
                        if let Ok(rc) = std::fs::canonicalize(&c) {
                            roots.push(rc.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
                // also the interpreter's own directory (bundled layouts)
                let c2 = bin.join("std");
                if c2.is_dir() {
                    if let Ok(rc) = std::fs::canonicalize(&c2) {
                        roots.push(rc.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        roots
            .iter()
            .any(|r| resolved == r.as_str() || resolved.starts_with(&format!("{}/", r)))
    }
    let managed = under(&rc_resolved, &interp.base_dir)
        || under(&rc_resolved, &cwd)
        || is_std_tree(&rc_resolved, &std_env);
    if !managed && interp.caps.enabled {
        if let Err(s) = interp.caps.check(&interp.caps.read, "read", &resolved) {
            // sec-r1 (audit C-7): keep the policy message for direct grants
            // debugging, but for the traversal class return the SAME string
            // resolve_path uses — otherwise real-vs-ghost is distinguishable
            let traversal = path.contains("..")
                || path.starts_with('/')
                || path.starts_with("\\\\")
                || path.contains(":\\")
                || path.contains(":/")
                || path.starts_with('~');
            if traversal {
                return Err(format!(
                    "module '{}' load failed (denied or nonexistent)",
                    path
                ));
            }
            return Err(format!(
                "module '{}' blocked: [{}] {}",
                path, s.kind, s.message
            ));
        }
    }
    // sec-r1 (audit C-7 TOCTOU): read the CANONICALIZED path, mirroring the
    // read_file builtin's check/open race fix — a symlink swapped between
    // the policy decision (canonicalize) and the read previously routed the
    // read outside the sandbox
    let rc_path = std::path::PathBuf::from(&rc_resolved);
    let src = std::fs::read_to_string(&rc_path)
        .map_err(|_| unified_load_fail(path, interp.caps.enabled))?;
    let stem = std::path::Path::new(&rc_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // module-level .rna sidecar
    let rna_sidecar = std::path::Path::new(&rc_path).with_extension("rna");
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

    // register the placeholder map BEFORE the body runs: cyclic re-entry
    // receives this exact Rc and sees it filled when loading completes
    interp.modules.insert(
        path.to_string(),
        Value::Map(Rc::new(RefCell::new(Vec::<(Value, Value)>::new()))),
    );
    interp.loading.push(path.to_string());
    let prog = crate::parser::parse(&src);
    for n in prog.notes {
        interp.notes.push(Note {
            line: n.line,
            rung: n.rung,
            message: format!("[{}] {}", path, n.message),
        });
    }

    // Execute module in a fresh child env of global (containment per stmt).
    let menv = Env::new(Some(interp.global.clone()));
    for stmt in &prog.stmts {
        if let Err(st) = interp.exec_stmt(&menv, stmt) {
            interp.note(
                0,
                4,
                format!("stress contained: [{}] {}", st.kind, st.message),
            );
        }
    }
    interp.loading.pop();

    // Exports: anchor export wins; else all top-level names; TAD insulation.
    let has_any_anchor =
        !prog.anchor_exports.is_empty() || prog.tad_exports.iter().any(|(_, e)| !e.is_empty());
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
    let modv = {
        // fill the placeholder registered before the body ran — same Rc,
        // so cyclic importers holding it observe the completed data
        if let Some(Value::Map(m)) = interp.modules.get(path) {
            {
                let mut target = m.borrow_mut();
                target.clear();
                target.extend(exports);
            }
            interp
                .modules
                .get(path)
                .cloned()
                .unwrap_or_else(|| Value::Map(Rc::new(RefCell::new(Vec::new()))))
        } else {
            let mv = Value::Map(Rc::new(RefCell::new(exports)));
            interp.modules.insert(path.to_string(), mv.clone());
            mv
        }
    };
    Ok(modv)
}

// NOTE: placeholder protocol — a module registers an EMPTY map in
// interp.modules before its body runs; completion fills THAT
// SAME map so cyclic importers holding the placeholder observe the data.

fn resolve_path(interp: &Interp, path: &str) -> Result<String, String> {
    let p = path.trim_end_matches(".op").to_string() + ".op";
    // sec-r1 (audit C-7): with the sandbox on, traversal/absolute module
    // names must not leak which outside paths exist — "not found" (ghost)
    // vs "blocked" (real file) was a filesystem existence oracle. That name
    // class gets ONE unified failure message; plain relative names inside
    // the project keep the precise "not found" for usable typos.
    let traversal = path.contains("..")
        || path.starts_with('/')
        || path.starts_with("\\\\")
        || path.contains(":\\")
        || path.contains(":/")
        || path.starts_with('~');
    let unified_err = |traversal_denied: bool| {
        if traversal_denied {
            format!("module '{}' load failed (denied or nonexistent)", path)
        } else {
            format!("module '{}' not found", path)
        }
    };
    let mut candidates: Vec<Option<std::path::PathBuf>> = Vec::new();
    // relative to the importing file's directory first (SPEC §8)
    if let Some(base) = &interp.base_dir {
        candidates.push(Some(std::path::PathBuf::from(base).join(&p)));
    }
    candidates.push(Some(std::path::PathBuf::from(&p)));
    candidates.push(Some(std::path::PathBuf::from("std").join(&p)));
    // explicit standard-library override (not double-joined with std/)
    candidates.push(
        std::env::var("OPERON_STD")
            .ok()
            .map(|d| std::path::PathBuf::from(d).join(&p)),
    );
    for c in candidates.into_iter().flatten() {
        if c.exists() {
            return Ok(c.to_string_lossy().to_string());
        }
    }
    Err(unified_err(interp.caps.enabled && traversal))
}

/// sec-r1 (audit C-7): single failure string for the traversal name class so
/// real-vs-ghost outside paths are indistinguishable in observable output.
fn unified_load_fail(path: &str, caps_enabled: bool) -> String {
    if caps_enabled {
        format!("module '{}' load failed (denied or nonexistent)", path)
    } else {
        format!("module '{}' load failed", path)
    }
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
                        message: format!(
                            "gene '{}' defined but never translated (dead transcript)",
                            name
                        ),
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
            Stmt::While(_, b)
            | Stmt::Loop(b)
            | Stmt::For(_, _, b)
            | Stmt::Block(b)
            | Stmt::Tad(_, b) => {
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
    arg_to_snap_d(v, 0)
}

fn arg_to_snap_d(v: &Value, d: u32) -> SnapArg {
    match v {
        Value::Gene(d2, _) if d2.name.is_some() => SnapArg::GeneRef(d2.name.clone().unwrap()),
        Value::Gene(d2, _) => SnapArg::Lambda(d2.clone()),
        other => SnapArg::Data(to_send_d(other, d)),
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

/// Maximum nesting depth of a value (cycle-safe: visited pointers never
/// re-expand). Walk stops at the SendValue cap.
fn value_depth(v: &Value, d: u32) -> u32 {
    if d > 100_000 {
        return d;
    }
    match v {
        Value::List(l) => {
            let mut maxd = d;
            for x in l.borrow().iter() {
                let dd = value_depth(x, d + 1);
                if dd > maxd {
                    maxd = dd;
                }
                if maxd > 100_000 {
                    return maxd;
                }
            }
            maxd
        }
        Value::Map(m) => {
            let mut maxd = d;
            for (_, x) in m.borrow().iter() {
                let dd = value_depth(x, d + 1);
                if dd > maxd {
                    maxd = dd;
                }
                if maxd > 100_000 {
                    return maxd;
                }
            }
            maxd
        }
        _ => d,
    }
}

pub fn to_send(v: &Value) -> SendValue {
    to_send_d(v, 0)
}

const SEND_DEPTH_CAP: u32 = 100_000;

fn to_send_d(v: &Value, d: u32) -> SendValue {
    if d > SEND_DEPTH_CAP {
        return SendValue::Stress("overflow".into(), "spawn argument nesting too deep".into());
    }
    match v {
        Value::Null => SendValue::Null,
        Value::Bool(b) => SendValue::Bool(*b),
        Value::Int(i) => SendValue::Int(*i),
        Value::Float(f) => SendValue::Float(*f),
        Value::Str(s) => SendValue::Str(s.clone()),
        Value::List(l) => SendValue::List(l.borrow().iter().map(|x| to_send_d(x, d + 1)).collect()),
        Value::Map(m) => SendValue::Map(
            m.borrow()
                .iter()
                .map(|(k, v)| (k.display(), to_send_d(v, d + 1)))
                .collect(),
        ),
        Value::Gene(_, _) => SendValue::Null,
        Value::Seq(_, _) => SendValue::Null,
        Value::Obj(pd, m) => {
            // objects cross the boundary as their field map (+ identity key)
            let mut out: Vec<(String, SendValue)> = Vec::new();
            out.push(("#phenotype".into(), SendValue::Str(pd.name.clone())));
            for (k, v) in m.borrow().iter() {
                out.push((k.display(), to_send_d(v, d + 1)));
            }
            SendValue::Map(out)
        }
    }
}

pub fn from_send(v: SendValue) -> Value {
    from_send_d(v, 0)
}

fn from_send_d(v: SendValue, d: u32) -> Value {
    if d > SEND_DEPTH_CAP {
        return Value::Null; // capped payload arrives as null (worker-side cap)
    }
    match v {
        SendValue::Null => Value::Null,
        SendValue::Bool(b) => Value::Bool(b),
        SendValue::Int(i) => Value::Int(i),
        SendValue::Float(f) => Value::Float(f),
        SendValue::Str(s) => Value::Str(s),
        SendValue::List(l) => Value::List(Rc::new(RefCell::new(
            l.into_iter().map(|x| from_send_d(x, d + 1)).collect(),
        ))),
        SendValue::Map(m) => Value::Map(Rc::new(RefCell::new(
            m.into_iter()
                .map(|(k, v)| (Value::Str(k), from_send_d(v, d + 1)))
                .collect(),
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

/// reg-r1 (regulation audit W1, severity HIGH): worker cells inherit the
/// parent's REGULATION state — GRN edges + levels, methylation counters +
/// threshold, toggle pairs, enhance marks. The old snapshot carried only
/// env values, so `spawn()` silently ungated the entire regulation layer:
/// a `@methylate`-silenced gene printed "chromatin repressed" WHILE it
/// executed inside a worker, and toggle/GRN gates were inert in every
/// cell. Least astonishment (D-008): a silenced gene stays silenced.
#[derive(Clone)]
pub struct RegulationSnap {
    pub grn_edges: Vec<RegEdge>,
    pub grn_levels: Vec<(String, f64)>,
    pub toggles: Vec<(String, String, bool)>,
    pub methyl_levels: Vec<(String, u32)>,
    pub methyl_threshold: u32,
    pub enhanced: Vec<String>,
    /// reg-r3 (re-audit): the repressilator ring rides the snapshot too —
    /// node names + the RAW ODE levels frozen at spawn, so a ring-sourced
    /// GRN gate vetoes identically inside the cell ("exactly as it does
    /// outside"). Workers are bounded cells: the ring is frozen at spawn
    /// like every other regulation state, not live-ticking.
    pub repressi_ring: Vec<String>,
    pub repressi_tick: u64,
    pub repressi_levels: Vec<f64>,
}

pub fn snapshot_regulation(interp: &Interp) -> RegulationSnap {
    // reg-r3: freeze the ring at the spawn tick (levels via the same pure
    // fold the host uses — bit-identical arithmetic)
    let (ring_tick, ring_levels) = {
        let tick = match &interp.repressi_atomic {
            Some(a) => a.load(std::sync::atomic::Ordering::SeqCst),
            None => interp.repressi_tick,
        };
        (
            tick,
            crate::interp::repressilator_levels(interp.repressi_ring.len(), tick),
        )
    };
    RegulationSnap {
        grn_edges: interp.grn_edges.clone(),
        grn_levels: interp
            .grn_levels
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        toggles: interp.toggles.clone(),
        methyl_levels: interp
            .methyl_levels
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        methyl_threshold: interp.methyl_threshold,
        enhanced: interp.enhanced.clone(),
        repressi_ring: interp.repressi_ring.clone(),
        repressi_tick: ring_tick,
        repressi_levels: ring_levels,
    }
}

pub fn bind_regulation(ti: &mut Interp, s: &RegulationSnap) {
    ti.grn_edges = s.grn_edges.clone();
    ti.grn_levels = s.grn_levels.iter().cloned().collect();
    ti.toggles = s.toggles.clone();
    ti.methyl_levels = s.methyl_levels.iter().cloned().collect();
    ti.methyl_threshold = s.methyl_threshold;
    ti.enhanced = s.enhanced.clone();
    // reg-r3: ring nodes resolve to the frozen spawn levels — pin the
    // worker's cache to (spawn_tick, levels) so ring_gate_level returns
    // exactly what the host saw (no live ticking inside the cell)
    ti.repressi_ring = s.repressi_ring.clone();
    ti.repressi_tick = s.repressi_tick;
    *ti.repressi_cache.borrow_mut() = (s.repressi_tick, s.repressi_levels.clone());
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
    clone_send_d(sv, 0)
}

fn clone_send_d(sv: &SendValue, d: u32) -> SendValue {
    if d > 2 * SEND_DEPTH_CAP {
        return SendValue::Stress("overflow".into(), "worker payload too deep".into());
    }
    match sv {
        SendValue::Null => SendValue::Null,
        SendValue::Bool(b) => SendValue::Bool(*b),
        SendValue::Int(i) => SendValue::Int(*i),
        SendValue::Float(f) => SendValue::Float(*f),
        SendValue::Str(s) => SendValue::Str(s.clone()),
        SendValue::List(l) => SendValue::List(l.iter().map(|x| clone_send_d(x, d + 1)).collect()),
        SendValue::Map(m) => SendValue::Map(
            m.iter()
                .map(|(k, v)| (k.clone(), clone_send_d(v, d + 1)))
                .collect(),
        ),
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
    reg: RegulationSnap,
    caps: crate::interp::Caps,
    fuel_pool: Option<Arc<AtomicI64>>,
) -> Result<Rc<RefCell<crate::value::SeqState>>, Stress> {
    let send_args: Vec<SnapArg> = args.iter().map(arg_to_snap).collect();
    let host_caps = caps;
    let (tx, rx) = mpsc::sync_channel::<crate::value::SeqMsg>(0);
    spawn_worker(move || {
        let mut ti = Interp::new();
        ti.fuel_pool = fuel_pool;
        let genv = Env::new(None);
        bind_snapshot(&genv, &snap);
        bind_regulation(&mut ti, &reg);
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
    })?;
    Ok(Rc::new(RefCell::new(crate::value::SeqState {
        rx: Some(rx),
        done: false,
        stress: None,
    })))
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
            format!(
                "spawn: gene '{}' contains closures; running inline (id 0)",
                name
            ),
        );
        let _ = interp.call_gene(def, None, args)?;
        return Ok(Value::Int(0));
    }
    if interp.tasks.len() >= 4096 {
        return Err(Stress::new(
            "overflow",
            "too many live tasks (4096) — join() your spawns",
        ));
    }
    // pre-flight depth check: a payload deeper than the SendValue cap must
    // fail the SPAWN (catchable stress), not smuggle a stress value across
    for a in &args {
        if value_depth(a, 0) > 100_000 {
            return Err(Stress::new("overflow", "spawn argument nesting too deep"));
        }
    }
    let snap = match &callee {
        Value::Gene(_, Some(cl)) => snapshot_with_closure(interp, Some(cl)),
        _ => snapshot_globals(interp),
    };
    let rsnap = snapshot_regulation(interp);
    let send_args: Vec<SnapArg> = args.iter().map(arg_to_snap).collect();
    let (tx, rx) = mpsc::channel::<(SendValue, Vec<Note>)>();
    let global_note = format!("[task {}]", name);
    let task_name = name.clone();
    let host_caps = interp.caps.clone();
    let host_fuel = interp.fuel_pool.clone();
    spawn_worker(move || {
        let mut ti = Interp::new();
        ti.fuel_pool = host_fuel;
        let genv = Env::new(None);
        bind_snapshot(&genv, &snap);
        bind_regulation(&mut ti, &rsnap);
        ti.global = genv.clone();
        ti.caps = host_caps; // worker cells inherit the host's grants
        let conv_args: Vec<Value> = send_args.iter().map(|a| arg_from_snap(a, &snap)).collect();
        // reg-r1: worker cells call through the SAME name-dispatch funnel as
        // the host — call_named evaluates the toggle/methyl/GRN gates that a
        // direct call_gene would bypass, so a repressed gene stays repressed
        let result = ti.call_named(&genv, &task_name, conv_args);
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
    })?;
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
            Expr::Call(a, args, _) | Expr::Method(a, _, args) => {
                expr_has(a) || args.iter().any(expr_has)
            }
            Expr::List(xs) => xs.iter().any(expr_has),
            Expr::Map(pairs) => pairs.iter().any(|(k, v)| expr_has(k) || expr_has(v)),
            Expr::Interp(ps) => ps
                .iter()
                .any(|p| matches!(p, InterpPart::Expr(e) if expr_has(e))),
            Expr::Collect {
                iter, filter, body, ..
            } => {
                expr_has(iter)
                    || filter.as_ref().map(|f| expr_has(f)).unwrap_or(false)
                    || expr_has(body)
            }
            _ => false,
        }
    }
    fn stmts_have(stmts: &[Stmt]) -> bool {
        stmts.iter().any(stmt_has)
    }
    fn stmt_has(s: &Stmt) -> bool {
        match s {
            Stmt::Let(_, e) | Stmt::Assign(_, _, e) | Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => {
                expr_has(e)
            }
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
                stmts_have(body)
                    || rescue
                        .as_ref()
                        .map(|(_, rb)| stmts_have(rb))
                        .unwrap_or(false)
            }
            Stmt::Gene(g) => stmts_have(&g.body),
            _ => false,
        }
    }
    stmts_have(&def.body)
        || def
            .guard
            .as_ref()
            .map(|(_, b)| stmts_have(b))
            .unwrap_or(false)
}

pub fn join_task(interp: &mut Interp, id: i64, timeout_ms: Option<u64>) -> Result<Value, Stress> {
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
    // optional deadline: join(id, ms) returns null when the worker exceeds it
    let received: Result<(crate::genes::SendValue, Vec<Note>), std::sync::mpsc::RecvTimeoutError> =
        match timeout_ms {
            Some(ms) => match handle
                .rx
                .recv_timeout(std::time::Duration::from_millis(ms.min(600_000)))
            {
                Ok(pair) => Ok(pair),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    interp.note(0, 4, format!("join timeout ({} ms) on task {}", ms, id));
                    interp.tasks.insert(id, handle); // keep the task joinable later
                    return Ok(Value::Null);
                }
                Err(_) => {
                    interp.note(0, 4, format!("task {} channel closed", id));
                    return Ok(Value::Null);
                }
            },
            None => match handle.rx.recv_timeout(std::time::Duration::from_secs(300)) {
                // sec-r3 (re-audit #3): the unbounded join was a free host
                // freeze — a worker looping `run("sleep", …)` kept the
                // channel open for as long as its fuel lasted. The default
                // join is now ceiling-bounded like the whole run (300 s):
                // timed-out tasks stay joinable and join returns null.
                Ok(pair) => Ok(pair),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    interp.note(
                        0,
                        4,
                        format!("join timeout (300000 ms ceiling) on task {}", id),
                    );
                    interp.tasks.insert(id, handle); // keep the task joinable later
                    return Ok(Value::Null);
                }
                Err(_) => {
                    interp.note(0, 4, format!("task {} channel closed", id));
                    return Ok(Value::Null);
                }
            },
        };
    match received {
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
