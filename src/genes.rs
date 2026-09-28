//! genes.rs, the gene-features pipeline: methylation config (.cell), RNA
//! edit patches (.rna), splice variant selection, module loading with TAD
//! insulation, NMD sweep, ORF finder, and real-OS-thread tasks.

use crate::ast::*;
use crate::interp::{Env, Interp};
use crate::value::{Stress, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

// ------------------------------------------------------------ thread budget
// A run may hold at most MAX_THREADS live worker cells. Exceeding the cap is
// a catchable `overflow` stress, never a panic, thread bombs are contained.
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
                "thread cap ({}) reached, too many live workers",
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
/// One replacement rule's fate under `apply_rna_checked`.
/// `hits` counts non-overlapping occurrences in the scanned region at the
/// time the rule runs (rules apply sequentially, mirroring apply_rna).
#[derive(Debug, Clone)]
pub struct RnaEdit {
    pub target: String,
    /// true when the rule is scoped to a gene (not stem/anywhere)
    pub gene_scoped: bool,
    /// gene mode: the gene span was found; stem/anywhere mode: always true
    pub target_found: bool,
    pub from: String,
    pub to: String,
    pub hits: usize,
    /// hits > 0 (the replacement actually changed text)
    pub applied: bool,
}

/// Detailed result of a dry-run/checked RNA application (W068 safety mode).
#[derive(Debug, Clone)]
pub struct RnaReport {
    pub edits: Vec<RnaEdit>,
    pub new_text: String,
}

impl RnaReport {
    pub fn applied(&self) -> usize {
        self.edits.iter().filter(|e| e.applied).count()
    }
    pub fn missed(&self) -> usize {
        self.edits.iter().filter(|e| !e.applied).count()
    }
    pub fn would_change(&self) -> bool {
        // the caller owns the original text; report carries only the result,
        // so "changed" is judged by whether any edit applied
        self.applied() > 0
    }
}

/// Apply an RNA edit patch to source text with a full per-rule report.
/// Semantics are IDENTICAL to `apply_rna` (same scan order, same all-
/// occurrences replacement, same gene-span scoping), this is the checked
/// engine; `apply_rna` is its lossy view (W068: no more silent misses).
pub fn apply_rna_checked(src: &str, patch_src: &str, file_stem: &str) -> RnaReport {
    let mut edits = Vec::new();
    let mut text = src.to_string();
    let patch = crate::parser::parse(patch_src);
    for s in &patch.stmts {
        if let Stmt::Edit(target, reps) = s {
            let gene_target = target != file_stem && target != "anywhere";
            if gene_target {
                if let Some((start, end)) = gene_span(&text, target) {
                    let seg = text[start..end].to_string();
                    let mut seg2 = seg.clone();
                    for (from, to) in reps {
                        let hits = count_occurrences(&seg2, from);
                        if hits > 0 {
                            seg2 = seg2.replace(from.as_str(), to);
                        }
                        edits.push(RnaEdit {
                            target: target.clone(),
                            gene_scoped: true,
                            target_found: true,
                            from: from.clone(),
                            to: to.clone(),
                            hits,
                            applied: hits > 0,
                        });
                    }
                    if seg2 != seg {
                        text.replace_range(start..end, &seg2);
                    }
                } else {
                    // gene not found: every rule of the statement is a miss
                    for (from, to) in reps {
                        edits.push(RnaEdit {
                            target: target.clone(),
                            gene_scoped: true,
                            target_found: false,
                            from: from.clone(),
                            to: to.clone(),
                            hits: 0,
                            applied: false,
                        });
                    }
                }
            } else {
                for (from, to) in reps {
                    let hits = count_occurrences(&text, from);
                    if hits > 0 {
                        text = text.replace(from.as_str(), to);
                    }
                    edits.push(RnaEdit {
                        target: target.clone(),
                        gene_scoped: false,
                        target_found: true,
                        from: from.clone(),
                        to: to.clone(),
                        hits,
                        applied: hits > 0,
                    });
                }
            }
        }
    }
    RnaReport {
        edits,
        new_text: text,
    }
}

/// Count non-overlapping occurrences of `needle` in `hay` (the same matches
/// `str::replace` would replace).
fn count_occurrences(hay: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0; // empty pattern: String::replace would insert everywhere,
                  // never a useful edit; treat as no-match rather than corrupt
    }
    let mut n = 0;
    let mut rest = hay;
    while let Some(p) = rest.find(needle) {
        n += 1;
        rest = &rest[p + needle.len()..];
    }
    n
}

/// Apply an RNA edit patch to source text. Target matches either the file
/// stem or a gene name; gene-scoped edits apply to that gene's source span
/// (best effort: from `gene <name>` to the next top-level `gene ` at col 0).
/// Lossy view over `apply_rna_checked`, kept for existing call sites
/// (`run --rna`, `build --rna`); new tooling should use the checked engine.
pub fn apply_rna(src: &str, patch_src: &str, file_stem: &str) -> (String, Vec<String>) {
    let report = apply_rna_checked(src, patch_src, file_stem);
    let applied = report
        .edits
        .iter()
        .filter(|e| e.applied)
        .map(|e| format!("{}: '{}' -> '{}'", e.target, e.from, e.to))
        .collect();
    (report.new_text, applied)
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
    // 3. loop-9 (F-4): runtime splice_shift (a bound splicing factor
    // overrides the static mark; the operator pins above override it)
    if let Some(v) = interp.splice_shift.get(&sp.root) {
        if let Some(found) = sp.variants.iter().find(|(n, _)| n == v) {
            return Some((found.0.clone(), found.1.clone()));
        }
    }
    // 4. m6a-marked variant
    if let Some(found) = sp.variants.iter().find(|(_, d)| d.m6a) {
        return Some((found.0.clone(), found.1.clone()));
    }
    // 5. first declared
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
                "cyclic import of '{}', module still loading; its map fills when loading completes",
                path
            ),
        );
        if let Some(v) = interp.modules.get(path) {
            return Ok(v.clone());
        }
        return Ok(Value::Map(Rc::new(RefCell::new(
            crate::value::MapStore::default(),
        ))));
    }
    if let Some(v) = interp.modules.get(path) {
        return Ok(v.clone());
    }
    let resolved = resolve_path(interp, path)?;
    // Module loading is a read, but of a runtime-managed tree: the
    // importing file's own project directory and the standard library are
    // ALWAYS importable (otherwise no `use` works under default-deny).
    // Imports that reach OUTSIDE those trees (arbitrary disk paths) require
    // an explicit read capability, a program cannot execute random files.
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
    // OPERON_STD override, or the interpreter's own tree), never any path
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
        || is_std_tree(&rc_resolved, &std_env)
        // W19/W23: a vendored dependency from the lockfile is a runtime-
        // managed tree too, the operator resolved + installed it (operon
        // mod add/install), so it imports under default-deny exactly like
        // std/; an arbitrary cache dir never appears here (the mapping is
        // name → pinned rev dir from the checked-in lockfile).
        || interp
            .lock_dirs
            .iter()
            .any(|(_, d)| under(&rc_resolved, &Some(d.clone())));
    if !managed && interp.caps.enabled {
        if let Err(s) = interp.caps.check(&interp.caps.read, "read", &resolved) {
            // sec-r1 (audit C-7): keep the policy message for direct grants
            // debugging, but for the traversal class return the SAME string
            // resolve_path uses, otherwise real-vs-ghost is distinguishable
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
    // read_file builtin's check/open race fix, a symlink swapped between
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
        Value::Map(Rc::new(RefCell::new(crate::value::MapStore::default()))),
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
                st.line,
                4,
                format!("stress contained: [{}] {}", st.kind, st.message),
            );
        }
    }
    interp.loading.pop();

    // Exports: anchor export wins; else all top-level names; TAD insulation.
    // W24: under `.cell modules.visibility = strict`, a module with pub
    // marks exports ONLY those names (anchors still win when present);
    // a strict module with zero pub marks keeps default-open (the
    // migration path) and notes it once. Default mode: `pub` is inert.
    let has_any_anchor =
        !prog.anchor_exports.is_empty() || prog.tad_exports.iter().any(|(_, e)| !e.is_empty());
    let strict = interp
        .cell
        .get("modules.visibility")
        .map(|s| s.eq_ignore_ascii_case("strict"))
        .unwrap_or(false);
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
    } else if strict && !prog.pub_exports.is_empty() {
        for name in &prog.pub_exports {
            if let Some(v) = menv.get(name) {
                exports.push((Value::Str(name.clone()), v));
            }
        }
        let total = menv
            .vars
            .borrow()
            .iter()
            .filter(|(k, _)| !k.starts_with('#'))
            .count();
        let hidden = total.saturating_sub(exports.len());
        if hidden > 0 {
            interp.note(
                0,
                4,
                format!(
                    "strict visibility: {} private name(s) hidden in '{}' (mark with pub or anchor export)",
                    hidden, path
                ),
            );
        }
    } else {
        if strict {
            interp.note(
                0,
                4,
                format!(
                    "strict visibility: '{}' has no pub marks, default-open retained (add pub or anchor export)",
                    path
                ),
            );
        }
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
        // fill the placeholder registered before the body ran, same Rc,
        // so cyclic importers holding it observe the completed data
        if let Some(Value::Map(m)) = interp.modules.get(path) {
            {
                let mut target = m.borrow_mut();
                target.clear();
                target.extend(exports);
            }
            interp.modules.get(path).cloned().unwrap_or_else(|| {
                Value::Map(Rc::new(RefCell::new(crate::value::MapStore::default())))
            })
        } else {
            let mv = Value::Map(Rc::new(RefCell::new(crate::value::MapStore::from_vec(
                exports,
            ))));
            interp.modules.insert(path.to_string(), mv.clone());
            mv
        }
    };
    Ok(modv)
}

// NOTE: placeholder protocol, a module registers an EMPTY map in
// interp.modules before its body runs; completion fills THAT
// SAME map so cyclic importers holding the placeholder observe the data.

fn resolve_path(interp: &Interp, path: &str) -> Result<String, String> {
    let p = path.trim_end_matches(".op").to_string() + ".op";
    // sec-r1 (audit C-7): with the sandbox on, traversal/absolute module
    // names must not leak which outside paths exist, "not found" (ghost)
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
    // dx-r5 (audit P0-1): a clean `curl | sh` install ships std/ beside the
    // binary, but resolve_path had no exe-relative candidate, `use std/…`
    // failed silently from any other CWD (silent-null-with-exit-0). Mirror
    // the caps layer's exe-relative std tree (genes.rs is_std_tree): the
    // install layout (…/bin/operon + …/std) and a dev layout (…/target/
    // release/operon + repo std/) both resolve.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // `use std/x`: p already carries the std/ component, so join the
            // bundled layout (…/bin/operon + …/bin/std) and the install/dev
            // layout (…/bin/operon + …/std) directly.
            candidates.push(Some(dir.join(&p)));
            if let Some(root) = dir.parent() {
                candidates.push(Some(root.join(&p)));
            }
            // bare `use x`: try the exe-relative std trees by file name
            if !p.starts_with("std/") {
                if let Some(fname) = std::path::Path::new(&p).file_name() {
                    candidates.push(Some(dir.join("std").join(fname)));
                    if let Some(root) = dir.parent() {
                        candidates.push(Some(root.join("std").join(fname)));
                    }
                }
            }
        }
    }
    // explicit standard-library override (not double-joined with std/)
    let std_env = std::env::var("OPERON_STD").ok();
    candidates.push(
        std_env
            .clone()
            .map(|d| std::path::PathBuf::from(d).join(&p)),
    );
    // W19/W23 (root 7): the vendored dependency cache from operon.lock.
    // A lock entry `name -> dir` resolves the leading path segment: with
    // lock entry (my-lib, ~/.operon/deps/my-lib-abc123), `use my-lib/util`
    // tries <dir>/util.op (and bare `use my-lib` → <dir>/my-lib.op... the
    // module's own root). Deterministic: the lock pins name→rev→dir, no
    // globbing, and the standard roots above win first (a local checkout
    // shadows the vendored copy, the developer's-tree-first rule).
    for (dep_name, dep_dir) in &interp.lock_dirs {
        let seg = format!("{}/", dep_name);
        let tail: Option<String> = if p == format!("{}.op", dep_name) {
            Some(format!("{}.op", dep_name))
        } else if p.starts_with(&seg) {
            Some(p[seg.len()..].to_string())
        } else {
            None
        };
        if let Some(t) = tail {
            candidates.push(Some(std::path::PathBuf::from(dep_dir).join(t)));
        }
    }
    // W070: per-root attempt detail for the PLAIN name class only. The
    // traversal class keeps its unified C-7 message, attempted-root detail
    // for outside paths would resurrect the existence oracle C-7 removed.
    // `is_file` (not `exists`) so a directory named `foo.op` reports as
    // "not a regular file" instead of "resolving" and dying in the parser.
    let mut attempts: Vec<String> = Vec::new();
    if interp.base_dir.is_none() {
        attempts.push("(no importing-file directory)".to_string());
    }
    let mut resolved: Option<String> = None;
    for c in candidates.into_iter().flatten() {
        let shown = c.to_string_lossy().to_string();
        if c.is_file() {
            resolved = Some(shown);
            break;
        }
        attempts.push(format!(
            "{} ({})",
            shown,
            if c.exists() {
                "not a regular file"
            } else {
                "missing"
            }
        ));
    }
    if let Some(r) = resolved {
        return Ok(r);
    }
    if let Some(d) = &std_env {
        attempts.push(format!("OPERON_STD={}", d));
    } else {
        attempts.push("OPERON_STD (unset)".to_string());
    }
    if traversal {
        return Err(unified_err(interp.caps.enabled && traversal));
    }
    Err(format!(
        "module '{}' not found, tried: {}",
        path,
        attempts.join("; ")
    ))
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
    /// W029: bytes cross spawn/sequence boundaries losslessly (the spawn
    /// wire snapshot-membrane rule: data values cross by serialization).
    Bytes(Vec<u8>),
    List(Vec<SendValue>),
    Map(Vec<(String, SendValue)>),
    Stress(String, String),
    /// W06 (D-014): Option/Result variants cross spawn/sequence boundaries
    /// losslessly (tag + payload); families are preserved.
    Variant(String, Option<Box<SendValue>>),
}

/// Sendable environment snapshot entry: gene definitions cross by Arc,
/// data values cross by serialization. W015: channel handles cross LIVE
/// (the Arc to the shared buffer), they are the one behavior value with a
/// thread-safe interior, so a spawned cell can send/recv on the same
/// channel as its host; the queue stores the wire form, so the membrane
/// still holds (nothing aliased is shared).
pub enum SnapVal {
    Gene(Arc<GeneDef>),
    Data(SendValue),
    Channel(Arc<crate::value::ChannelShared>),
}

/// Sendable argument: data serializes; named genes cross as references to
/// the snapshot (the worker re-binds them from its inherited repertoire).
/// Anonymous lambdas cannot cross, they arrive as null with a note.
/// W015: channel handles cross LIVE (same shared buffer on both sides).
pub enum SnapArg {
    Data(SendValue),
    GeneRef(String),
    Lambda(Arc<GeneDef>),
    Channel(Arc<crate::value::ChannelShared>),
}

pub fn arg_to_snap(v: &Value) -> SnapArg {
    arg_to_snap_d(v, 0)
}

fn arg_to_snap_d(v: &Value, d: u32) -> SnapArg {
    match v {
        Value::Gene(d2, _) if d2.name.is_some() => SnapArg::GeneRef(d2.name.clone().unwrap()),
        Value::Gene(d2, _) => SnapArg::Lambda(d2.clone()),
        // W015: a top-level channel argument crosses as a live handle
        Value::Channel(a) => SnapArg::Channel(a.clone()),
        other => SnapArg::Data(to_send_d(other, d)),
    }
}

pub fn arg_from_snap(a: &SnapArg, snap: &[(String, SnapVal)]) -> Value {
    match a {
        SnapArg::Data(sv) => from_send(clone_send(sv)),
        // W015: the live channel handle re-binds by Arc (same buffer)
        SnapArg::Channel(arc) => Value::Channel(arc.clone()),
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
pub(crate) fn value_depth(v: &Value, d: u32) -> u32 {
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
        Value::Bytes(b) => SendValue::Bytes(b.as_ref().clone()),
        Value::List(l) => SendValue::List(l.borrow().iter().map(|x| to_send_d(x, d + 1)).collect()),
        Value::Map(m) => SendValue::Map(
            m.borrow()
                .iter()
                .map(|(k, v)| (k.display(), to_send_d(v, d + 1)))
                .collect(),
        ),
        Value::Gene(_, _) => SendValue::Null,
        Value::Seq(_, _) => SendValue::Null,
        // W015: a channel handle nested inside a container degrades to null
        // during data serialization (same rule as genes/sequences); only
        // top-level handles cross live (SnapArg::Channel / SnapVal::Channel)
        Value::Channel(_) => SendValue::Null,
        // W013: a weak handle degrades to null like every other handle. It
        // could never keep its meaning across the snapshot membrane (the
        // copy's target is a different allocation), and a top-level weak is
        // refused by the spawn/send pre-flight before this arm is reached.
        Value::Weak(_) => SendValue::Null,
        Value::Variant(crate::value::VTag::NoneV, _) => SendValue::Variant("None".into(), None),
        Value::Variant(t, Some(p)) => {
            SendValue::Variant(t.tag_name().into(), Some(Box::new(to_send_d(p, d + 1))))
        }
        // a Some/Ok/Err without payload cannot be constructed; degrade to
        // the bare tag rather than panic at the boundary.
        Value::Variant(t, None) => SendValue::Variant(t.tag_name().into(), None),
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
        SendValue::Bytes(b) => Value::Bytes(Rc::new(b)),
        SendValue::List(l) => Value::List(Rc::new(RefCell::new(
            l.into_iter().map(|x| from_send_d(x, d + 1)).collect(),
        ))),
        SendValue::Map(m) => Value::Map(Rc::new(RefCell::new(
            m.into_iter()
                .map(|(k, v)| (Value::Str(k), from_send_d(v, d + 1)))
                .collect(),
        ))),
        SendValue::Stress(k, m) => Value::Map(Rc::new(RefCell::new(
            crate::value::MapStore::from_vec(vec![
                (Value::Str("kind".into()), Value::Str(k)),
                (Value::Str("message".into()), Value::Str(m)),
            ]),
        ))),
        SendValue::Variant(tag, payload) => {
            let t = match tag.as_str() {
                "Some" => crate::value::VTag::SomeV,
                "None" => crate::value::VTag::NoneV,
                "Ok" => crate::value::VTag::OkV,
                _ => crate::value::VTag::ErrV,
            };
            Value::Variant(t, payload.map(|p| Box::new(from_send_d(*p, d + 1))))
        }
    }
}

/// Snapshot the parent global environment for a worker thread: gene
/// definitions cross by Arc (they are immutable ASTs), data values cross by
/// SendValue serialization. This gives tasks and sequences visibility of the
/// whole module surface, cells share metabolites through signals, and worker
/// cells inherit the module's gene repertoire.
pub fn snapshot_globals(interp: &Interp) -> Vec<(String, SnapVal)> {
    let mut out: Vec<(String, SnapVal)> = Vec::new();
    for (name, v) in interp.global.vars.borrow().iter() {
        match v {
            Value::Gene(d, _) => out.push((name.clone(), SnapVal::Gene(d.clone()))),
            // W015: a global channel crosses as a live handle (same buffer)
            Value::Channel(a) => out.push((name.clone(), SnapVal::Channel(a.clone()))),
            other => {
                let sv = to_send(other);
                out.push((name.clone(), SnapVal::Data(sv)));
            }
        }
    }
    out
}

/// reg-r1 (regulation audit W1, severity HIGH): worker cells inherit the
/// parent's REGULATION state, GRN edges + levels, methylation counters +
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
    /// reg-r3 (re-audit): the repressilator ring rides the snapshot too,
    /// node names + the RAW ODE levels frozen at spawn, so a ring-sourced
    /// GRN gate vetoes identically inside the cell ("exactly as it does
    /// outside"). Workers are bounded cells: the ring is frozen at spawn
    /// like every other regulation state, not live-ticking.
    pub repressi_ring: Vec<String>,
    pub repressi_tick: u64,
    pub repressi_levels: Vec<f64>,
    /// reg-bio (F-1): the telegraph promoter layer rides the snapshot,
    /// promoter states are part of a cell's regulatory state, so a spawned
    /// cell (mitosis) inherits the parent's on/off promoters and burst
    /// counters, and later parent-side switches do not propagate
    /// (snapshot semantics, same contract as every other regulation field).
    pub promoter_states: Vec<(String, bool)>,
    pub burst_off: Vec<(String, u64)>,
    pub expr_stochastic: bool,
    pub expr_kon: f64,
    pub expr_koff: f64,
    /// reg-bio-2 (C2): decay-clock override rides the snapshot
    pub decay_clock_n: Option<u64>,
    pub decay_clock_f: Option<f64>,
    /// reg-bio (F-5): the ring's kinetic parameters ride the snapshot so a
    /// worker folding frozen ring levels uses the parent's α/γ/n/basal.
    pub repressi_params: crate::interp::RepressiParams,
    pub enhance_delta: f64,
    /// reg-bio-2 (C1/C11): the translation layer and decoy sites ride the
    /// snapshot, worker cells are whole regulatory cells (same contract
    /// as every other regulation field).
    pub trans_edges: Vec<crate::ast::TransEdge>,
    pub trans_last: Vec<(String, u64)>,
    pub decoys: Vec<(String, String, f64)>,
    /// reg-bio-2 (A4): ligand pools + allosteric bindings ride the snapshot
    pub ligands: Vec<String>,
    pub ligand_pools: Vec<(String, f64)>,
    pub grn_binds: Vec<crate::ast::BindDef>,
    /// reg-bio-3 (C9): RISC entries ride the snapshot, a spawned cell is a
    /// whole regulatory cell: silencing redirects inside it too (13b fix:
    /// §13's inventory previously missed silences).
    pub silences: Vec<(String, Option<String>, f64, u32)>,
    /// loop-9 (C8): signal species names ride the snapshot (a worker needs
    /// them for edge-source resolution). The MEDIUM itself does NOT, the
    /// shared pool is handed to the worker explicitly (environment, not
    /// cytoplasm: live, not frozen).
    pub signals: Vec<String>,
    /// loop-9 (F-6): resolved m6A reader knobs ride the snapshot so a
    /// worker folds the parent's reader math exactly.
    pub m6a_reader: (f64, f64, u32),
    /// loop-9 (F-3): per-gene promoter attempt telemetry rides the snapshot
    /// (name, attempts, on_total, episodes), worker cells are whole
    /// regulatory cells; telemetry must not desync from the host mid-burst.
    pub promoter_tel: Vec<(String, u64, u64, u64)>,
    /// loop-9 (R9): runtime burst overrides freeze at spawn (snapshot).
    pub burst_overrides: Vec<(String, f64, f64)>,
    /// loop-9 (F-4): runtime splice shifts freeze at spawn (snapshot
    /// contract, later parent-side shifts do not propagate).
    pub splice_shift: Vec<(String, String)>,
    pub risc_escaped: Vec<String>,
    /// reg-bio-3 (A1/A7): polycistronic units (membership, order, rbs,
    /// transcript counters) ride the snapshot.
    pub operons: Vec<crate::interp::OperonUnit>,
    /// reg-bio-3 (B3/B2/C10): m6A levels, generation counter, gene dosage.
    pub m6a_levels: Vec<(String, u32)>,
    pub generation: u64,
    pub copies: Vec<(String, u32)>,
    /// loop-10 (F-7/F-8): resolved Rho/queue knobs ride the snapshot,
    /// (armed, catch, queue_floor, queue_cap, drain). Worker cells fold the
    /// parent's termination math exactly (they do not inherit raw .cell).
    pub rho_knobs: (bool, f64, f64, f64, f64),
    /// loop-10 (F-8): the queue register freezes at spawn (snapshot
    /// contract, later parent-side queue growth does not propagate).
    pub ribo_queue: Vec<(String, f64)>,
}

pub fn snapshot_regulation(interp: &Interp) -> RegulationSnap {
    // reg-r3: freeze the ring at the spawn tick (levels via the same pure
    // fold the host uses, bit-identical arithmetic)
    let (ring_tick, ring_levels) = {
        let tick = match &interp.repressi_atomic {
            Some(a) => a.load(std::sync::atomic::Ordering::SeqCst),
            None => interp.repressi_tick,
        };
        (
            tick,
            crate::interp::repressilator_levels_p(
                interp.repressi_ring.len(),
                tick,
                interp.repressi_params,
            ),
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
        promoter_states: interp
            .promoter_states
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        burst_off: interp
            .burst_off
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        expr_stochastic: interp.expr_stochastic,
        expr_kon: interp.expr_kon,
        expr_koff: interp.expr_koff,
        decay_clock_n: interp.decay_clock_n,
        decay_clock_f: interp.decay_clock_f,
        repressi_params: interp.repressi_params,
        enhance_delta: interp.enhance_delta,
        trans_edges: interp.trans_edges.clone(),
        trans_last: interp
            .trans_last
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        decoys: interp.decoys.clone(),
        ligands: interp.ligands.clone(),
        ligand_pools: interp
            .ligand_pools
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        grn_binds: interp.grn_binds.clone(),
        silences: interp.silences.clone(),
        signals: interp.signals.clone(),
        splice_shift: interp
            .splice_shift
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        promoter_tel: interp
            .promoter_tel
            .iter()
            .map(|(k, (a, o, e))| (k.clone(), *a, *o, *e))
            .collect(),
        burst_overrides: interp
            .burst_overrides
            .iter()
            .map(|(k, (a, b))| (k.clone(), *a, *b))
            .collect(),
        // loop-10 R10-c (parity jury F-1m): prefer the pinned reader knobs
        // when already bound, the same worker-in-worker class as the rho
        // fix below (re-resolving from an empty worker cell silently
        // resets the reader to defaults at depth >= 2).
        m6a_reader: interp.m6a_reader_pins.unwrap_or((
            interp
                .cell
                .get("m6a.reader.decay")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v.clamp(0.0, 1.0))
                .unwrap_or(0.25),
            interp
                .cell
                .get("m6a.reader.translation")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v.clamp(0.0, 1.0))
                .unwrap_or(0.10),
            interp
                .cell
                .get("m6a.reader.min_level")
                .and_then(|v| v.parse::<u32>().ok())
                .map(|v| v.clamp(0, 3))
                .unwrap_or(2),
        )),
        risc_escaped: interp.risc_escaped.iter().cloned().collect(),
        operons: interp.operons.clone(),
        m6a_levels: interp
            .m6a_levels
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        generation: interp.generation,
        copies: interp.copies.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        // loop-10 R10-b (parity jury F-1): prefer the pinned knobs when
        // already bound (worker-in-worker snapshotting), re-resolving from
        // interp.cell (empty inside a worker) would silently disarm the
        // layer at depth >= 2 while the oracle stays armed.
        rho_knobs: interp.rho_pins.unwrap_or((
            interp
                .cell
                .get("rho.termination")
                .map(|v| v == "true")
                .unwrap_or(false),
            interp
                .cell
                .get("rho.catch")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v.clamp(0.0, 1.0))
                .unwrap_or(0.5),
            interp
                .cell
                .get("rho.queue_floor")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.5),
            interp
                .cell
                .get("ribosome.queue_cap")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(1.0),
            interp
                .cell
                .get("ribosome.drain")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.5),
        )),
        ribo_queue: interp
            .ribo_queue
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
    }
}

pub fn bind_regulation(ti: &mut Interp, s: &RegulationSnap) {
    ti.grn_edges = s.grn_edges.clone();
    ti.grn_levels = s.grn_levels.iter().cloned().collect();
    ti.toggles = s.toggles.clone();
    ti.methyl_levels = s.methyl_levels.iter().cloned().collect();
    ti.methyl_threshold = s.methyl_threshold;
    ti.enhanced = s.enhanced.clone();
    // reg-r3: ring nodes resolve to the frozen spawn levels, pin the
    // worker's cache to (spawn_tick, levels) so ring_gate_level returns
    // exactly what the host saw (no live ticking inside the cell)
    ti.repressi_ring = s.repressi_ring.clone();
    ti.repressi_tick = s.repressi_tick;
    *ti.repressi_cache.borrow_mut() = (s.repressi_tick, s.repressi_levels.clone());
    // reg-bio: promoter layer + ring kinetics + enhancer dose ride the
    // snapshot (worker cells are whole regulatory cells, not gate ghosts)
    ti.promoter_states = s.promoter_states.iter().cloned().collect();
    ti.burst_off = s.burst_off.iter().cloned().collect();
    ti.expr_stochastic = s.expr_stochastic;
    ti.expr_kon = s.expr_kon;
    ti.expr_koff = s.expr_koff;
    ti.decay_clock_n = s.decay_clock_n;
    ti.decay_clock_f = s.decay_clock_f;
    ti.repressi_params = s.repressi_params;
    ti.enhance_delta = s.enhance_delta;
    // reg-bio-2 (C1/C11): translation + decoy state restore
    ti.trans_edges = s.trans_edges.clone();
    ti.trans_last = s.trans_last.iter().cloned().collect();
    ti.decoys = s.decoys.clone();
    // reg-bio-2 (A4): ligand pools + allosteric bindings restore
    ti.ligands = s.ligands.clone();
    ti.ligand_pools = s.ligand_pools.iter().cloned().collect();
    ti.grn_binds = s.grn_binds.clone();
    // reg-bio-3: RISC entries, polycistronic units, m6A levels, generation
    // and dosage restore (worker cells are whole regulatory cells)
    ti.silences = s.silences.clone();
    ti.risc_escaped = s.risc_escaped.iter().cloned().collect();
    // loop-9 (C8): signal species ride the snapshot; the MEDIUM arc is
    // assigned by the spawner (shared, live, see spawn_task/seq_start).
    ti.signals = s.signals.clone();
    // loop-9 (F-6): resolved reader knobs pin the worker's math
    ti.m6a_reader_pins = Some(s.m6a_reader);
    ti.splice_shift = s.splice_shift.iter().cloned().collect();
    ti.promoter_tel = s
        .promoter_tel
        .iter()
        .map(|(k, a, o, e)| (k.clone(), (*a, *o, *e)))
        .collect();
    ti.burst_overrides = s
        .burst_overrides
        .iter()
        .map(|(k, a, b)| (k.clone(), (*a, *b)))
        .collect();
    ti.operons = s.operons.clone();
    ti.m6a_levels = s.m6a_levels.iter().cloned().collect();
    ti.generation = s.generation;
    ti.copies = s.copies.iter().cloned().collect();
    // loop-10 (F-7/F-8): Rho knobs pin the worker's termination math; the
    // queue register freezes at spawn (whole regulatory cell contract)
    ti.rho_pins = Some(s.rho_knobs);
    ti.ribo_queue = s.ribo_queue.iter().cloned().collect();
}

pub fn bind_snapshot(env: &Rc<Env>, snap: &[(String, SnapVal)]) {
    for (name, sv) in snap {
        let v = match sv {
            SnapVal::Gene(d) => Value::Gene(d.clone(), None),
            SnapVal::Channel(a) => Value::Channel(a.clone()),
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
                // W015: closure-captured channels cross live too (same buffer)
                Value::Channel(a) => out.push((name.clone(), SnapVal::Channel(a.clone()))),
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
        SendValue::Bytes(b) => SendValue::Bytes(b.clone()),
        SendValue::List(l) => SendValue::List(l.iter().map(|x| clone_send_d(x, d + 1)).collect()),
        SendValue::Map(m) => SendValue::Map(
            m.iter()
                .map(|(k, v)| (k.clone(), clone_send_d(v, d + 1)))
                .collect(),
        ),
        SendValue::Stress(k, m) => SendValue::Stress(k.clone(), m.clone()),
        // W06 (D-014): variants deep-clone tag + payload losslessly.
        SendValue::Variant(tag, p) => SendValue::Variant(
            tag.clone(),
            p.as_ref().map(|x| Box::new(clone_send_d(x, d + 1))),
        ),
    }
}

// ------------------------------------------------------------ sequences
/// Start a sequence worker: rendezvous channel makes pulls lazy, the worker
/// blocks on each yield until the consumer pulls the next value.
pub fn seq_start(
    def: Arc<GeneDef>,
    args: Vec<Value>,
    snap: Vec<(String, SnapVal)>,
    reg: RegulationSnap,
    caps: crate::interp::Caps,
    fuel_pool: Option<Arc<AtomicI64>>,
    medium: Option<Arc<Mutex<HashMap<String, u64>>>>,
) -> Result<Rc<RefCell<crate::value::SeqState>>, Stress> {
    let send_args: Vec<SnapArg> = args.iter().map(arg_to_snap).collect();
    let host_caps = caps;
    let (tx, rx) = mpsc::sync_channel::<crate::value::SeqMsg>(0);
    spawn_worker(move || {
        let mut ti = Interp::new();
        ti.fuel_pool = fuel_pool;
        // loop-9 (C8): the worker holds the SAME live medium (environment)
        ti.medium = medium;
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
            // W06 (D-014): propagation inside a sequence ends the stream,
            // sequences are streams, not answers, so the variant has no
            // return path; the stream ends cleanly (never leaked as a kind).
            Err(s) if s.prop.is_some() => {
                ti.note(0, 4, "propagation ended the sequence");
                (ti.notes, None)
            }
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
    // sequence (return value is dropped, sequences are streams, not answers).
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
            "too many live tasks (4096), join() your spawns",
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
    // W18: each task gets its own cancel flag + an observable lifecycle
    // phase. The worker's chain = every ancestor flag + its own, so
    // cancelling a cell cancels its whole descent at tick boundaries.
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let task_phase = Arc::new(Mutex::new(crate::interp::TaskState::Running));
    let mut worker_chain = interp.cancel_chain.clone();
    worker_chain.push(cancel_flag.clone());
    let task_phase_in = task_phase.clone();
    // loop-9 (C8): the worker holds the SAME live medium (environment,
    // my secretion raises your activation across cells). Spawning adds a
    // cell to the culture, so the medium materializes here even when the
    // host has not secreted yet, an empty shared pool reads exactly like
    // None (0.0 everywhere), so legacy behavior is unchanged.
    let host_medium = Some(interp.medium_arc());
    // reg-bio-2 (C3): the task id must be claimed BEFORE the worker body
    // runs, the worker's RNG stream is derived from it (decorrelated
    // promoter bursting across cells).
    let id = interp.next_task_id;
    interp.next_task_id += 1;
    let task_seed = 0x9E3779B97F4A7C15u64 ^ (id as u64).wrapping_mul(0x9E3779B97F4A7C15);
    spawn_worker(move || {
        let mut ti = Interp::new();
        ti.cancel_chain = worker_chain;
        ti.fuel_pool = host_fuel;
        // loop-9 (C8): live shared medium
        ti.medium = host_medium;
        // reg-bio-2 (C3): worker cells do NOT all start from the same default
        // seed, each derives its stream from its task id, so two cells
        // bursting under expr_on draw DIFFERENT promoter sequences. The old
        // behavior was synchronized bursting across cells (perfect
        // correlation), the exact opposite of extrinsic noise. Still fully
        // deterministic: same program → same ids → same streams. The Python
        // oracle mirrors the derivation (save / derive / restore) inline.
        ti.rng = task_seed;
        let genv = Env::new(None);
        bind_snapshot(&genv, &snap);
        bind_regulation(&mut ti, &rsnap);
        ti.global = genv.clone();
        ti.caps = host_caps; // worker cells inherit the host's grants
        let conv_args: Vec<Value> = send_args.iter().map(|a| arg_from_snap(a, &snap)).collect();
        // reg-r1: worker cells call through the SAME name-dispatch funnel as
        // the host, call_named evaluates the toggle/methyl/GRN gates that a
        // direct call_gene would bypass, so a repressed gene stays repressed
        let result = ti.call_named(&genv, &task_name, conv_args);
        let (rv, notes) = match result {
            Ok(v) => (to_send(&v), ti.notes),
            // W06 (D-014): a propagated variant IS the worker gene's return
            // value, converted at the boundary, never leaked as a failure.
            Err(s) if s.prop.is_some() => (to_send(&s.prop.unwrap()), ti.notes),
            Err(s) => (SendValue::Stress(s.kind, s.message), ti.notes),
        };
        let notes = notes
            .into_iter()
            .map(|mut n| {
                n.message = format!("{} {}", global_note, n.message);
                n
            })
            .collect();
        // W18: the worker itself records the terminal phase BEFORE the
        // result leaves, so a concurrent task_state read never sees a
        // half-finished task. A cancelled-run stress names the phase;
        // everything else (including a late cancel that never landed on
        // a tick) counts as done, because the gene did finish.
        let phase = match &rv {
            SendValue::Stress(k, _) if k == "cancelled" => crate::interp::TaskState::Cancelled,
            _ => crate::interp::TaskState::Done,
        };
        *task_phase_in.lock().unwrap() = phase;
        let _ = tx.send((rv, notes));
    })?;
    let _ = id; // claimed before spawn (C3); registered below
    interp.tasks.insert(
        id,
        crate::interp::TaskHandle {
            rx,
            cancel: cancel_flag,
            state: task_phase,
        },
    );
    // W17: a spawn inside an active scope block registers on the innermost
    // scope; the block reaps it at exit (structured concurrency).
    if let Some(top) = interp.scope_stack.last_mut() {
        top.push(id);
    }
    Ok(Value::Int(id))
}

fn def_has_lambda(def: &GeneDef) -> bool {
    fn expr_has(e: &Expr) -> bool {
        match e {
            Expr::Lambda(_) => true,
            Expr::Unary(_, a) | Expr::Member(a, _) => expr_has(a),
            Expr::Binary(_, a, b, _) | Expr::Index(a, b, _) => expr_has(a) || expr_has(b),
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

/// W18: ask a task to stop. The flag is observed by the worker at its next
/// fuel tick boundary; nothing is preempted and no data is touched. The
/// return value says whether the request landed on a task that can still
/// act on it (true) or whether the task was already finished or unknown
/// (false, with a rung-4 note saying which).
pub fn cancel_task(interp: &mut Interp, args: Vec<Value>) -> Result<Value, Stress> {
    let id = match args.first() {
        Some(Value::Int(i)) => *i,
        _ => -1,
    };
    if id <= 0 {
        // id 0 is the inline-closure task (already returned)
        interp.note(0, 4, "cancel() needs a task id");
        return Ok(Value::Bool(false));
    }
    if let Some(handle) = interp.tasks.get(&id) {
        let flag = handle.cancel.clone();
        let phase_cell = handle.state.clone();
        let mut phase = phase_cell.lock().unwrap();
        if *phase == crate::interp::TaskState::Running {
            flag.store(true, Ordering::Relaxed);
            *phase = crate::interp::TaskState::Cancelled;
            interp.note(0, 4, format!("cancel requested for task {}", id));
            return Ok(Value::Bool(true));
        }
        interp.note(0, 4, format!("task {} already finished", id));
        return Ok(Value::Bool(false));
    }
    if interp.task_tombstones.contains_key(&id) {
        interp.note(0, 4, format!("task {} already finished", id));
        return Ok(Value::Bool(false));
    }
    interp.note(0, 4, format!("task {} unknown or already joined", id));
    Ok(Value::Bool(false))
}

/// W18: read a task's lifecycle phase without joining. Live tasks answer
/// from the worker-owned phase; joined tasks answer from tombstones.
/// Unknown ids (or already-forgotten ones) are contained at the soft tier.
pub fn task_state_of(interp: &mut Interp, args: Vec<Value>) -> Result<Value, Stress> {
    let id = match args.first() {
        Some(Value::Int(i)) => *i,
        _ => -1,
    };
    if id == 0 {
        // inline-closure task: the body already ran to completion
        return Ok(Value::Str("done".into()));
    }
    if id < 0 {
        interp.note(0, 4, "task_state() needs a task id");
        return Ok(Value::Null);
    }
    if let Some(handle) = interp.tasks.get(&id) {
        let phase_cell = handle.state.clone();
        let phase = *phase_cell.lock().unwrap();
        return Ok(Value::Str(phase.as_str().into()));
    }
    if let Some(t) = interp.task_tombstones.get(&id) {
        let s = t.as_str();
        return Ok(Value::Str(s.into()));
    }
    interp.note(0, 4, format!("task {} unknown or already joined", id));
    Ok(Value::Null)
}

/// W15: task-group join-all. Joins every id in input order and returns
/// the results as a list. Join semantics are unchanged: an already-joined
/// or unknown id contributes null plus a note, so the result list stays
/// position-aligned with the input list.
pub fn wait_all_tasks(interp: &mut Interp, args: Vec<Value>) -> Result<Value, Stress> {
    let ids = match args.first() {
        Some(Value::List(l)) => l.borrow().clone(),
        _ => {
            interp.note(0, 4, "wait_all() needs a list of task ids");
            return Ok(Value::List(Rc::new(RefCell::new(vec![]))));
        }
    };
    let mut out = Vec::new();
    for idv in &ids {
        let id = match idv {
            Value::Int(i) => *i,
            _ => -1,
        };
        out.push(join_task(interp, id, None)?);
    }
    Ok(Value::List(Rc::new(RefCell::new(out))))
}

/// W15: select-style wait. Returns the id of the first task in the list
/// whose worker has finished (in wall-clock completion order; ties resolve
/// by scan order), or null + note when the timeout expires first. Polling
/// implementation: the worker records its terminal phase before its result
/// leaves, so a finished phase means join will not block for long. The
/// sequential oracle cannot observe completion ordering; its children are
/// all born finished, so it answers the first listed id (the differential
/// corpus pins only the ordering-free shapes).
pub fn wait_any_task(interp: &mut Interp, args: Vec<Value>) -> Result<Value, Stress> {
    let ids: Vec<i64> = match args.first() {
        Some(Value::List(l)) => l
            .borrow()
            .iter()
            .filter_map(|v| match v {
                Value::Int(i) => Some(*i),
                _ => None,
            })
            .collect(),
        _ => {
            interp.note(0, 4, "wait_any() needs a list of task ids");
            return Ok(Value::Null);
        }
    };
    let timeout_ms = match args.get(1) {
        Some(Value::Int(i)) if *i > 0 => (*i as u64).min(300_000),
        Some(Value::Float(f)) if *f > 0.0 => (*f as u64).min(300_000),
        _ => 30_000,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        for id in &ids {
            let done = if let Some(h) = interp.tasks.get(id) {
                let phase = *h.state.lock().unwrap();
                phase != crate::interp::TaskState::Running
            } else {
                interp.task_tombstones.contains_key(id)
            };
            if done {
                return Ok(Value::Int(*id));
            }
        }
        if std::time::Instant::now() >= deadline {
            interp.note(
                0,
                4,
                format!("wait_any timeout ({} ms); no task finished", timeout_ms),
            );
            return Ok(Value::Null);
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
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
                // freeze, a worker looping `run("sleep", …)` kept the
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
            // W18: the handle left the registry; keep the terminal phase
            // answerable for task_state(). A cancelled-run stress names
            // the phase, everything else is a completed task.
            let phase = match &v {
                SendValue::Stress(k, _) if k == "cancelled" => crate::interp::TaskState::Cancelled,
                _ => crate::interp::TaskState::Done,
            };
            interp.task_tombstones.insert(id, phase);
            Ok(from_send(v))
        }
        Err(_) => {
            interp.note(0, 4, format!("task {} channel closed", id));
            interp
                .task_tombstones
                .insert(id, crate::interp::TaskState::Done);
            Ok(Value::Null)
        }
    }
}
