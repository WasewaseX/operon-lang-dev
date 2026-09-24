//! tools.rs — toolchain subcommands: run entry resolution, check/NMD grading,
//! formatter, profile, crispr knockout screens, bench, test runner, build.

use crate::ast::*;
use crate::genes;
use crate::interp::{Flow, Interp};
use crate::parser;
use crate::value::{Stress, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::Write as _;
use std::path::Path;
use std::rc::Rc;

#[derive(Clone)]
pub struct Opts {
    pub cell: Option<String>,
    pub variant: Option<String>,
    pub rna: Option<String>,
    pub entry: Option<String>,
    pub use_ires: bool,
    pub frame: Option<String>,
    pub args: Vec<String>,
    pub quiet: bool,
    pub caps: crate::interp::Caps,
    /// dx-r1 (audit W5): time top-level statements during load — without
    /// this, `operon profile` reported 0.0 µs for any script without main().
    pub profile: bool,
    /// dx-r3 (re-audit / A14): when set, program stdout (promote) is
    /// captured here from the moment the interp exists — the test runner
    /// uses it so top-level output can't leak into the report either.
    pub stdout_sink: Option<std::rc::Rc<std::cell::RefCell<Vec<String>>>>,
}

pub struct Loaded {
    pub interp: Interp,
    pub prog: Program,
}

pub fn load_file(file: &str, opts: &Opts) -> Result<Loaded, String> {
    let mut src =
        std::fs::read_to_string(file).map_err(|e| format!("cannot read '{}': {}", file, e))?;
    let stem = Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut interp = Interp::new();
    interp.profiling = opts.profile;
    // dx-r3: capture program stdout from load time (test runner)
    interp.stdout_sink = opts.stdout_sink.clone();
    // A13 (dx-r2): diagnostics render file:line
    interp.file = file.to_string();
    // base dir for module resolution (relative to the importing file)
    interp.base_dir = std::path::Path::new(file)
        .parent()
        .map(|p| p.to_string_lossy().to_string());

    // methylation layer: CLI --cell, else operon.cell auto-detect.
    // SECURITY POLICY: an auto-detected cell config may configure entry/
    // variant/quiet keys, but its allow.* keys are IGNORED — capability
    // grants must come from the operator (CLI --allow-* / explicit --cell),
    // never silently from a file that happens to sit in the project.
    let cell_path = opts.cell.clone().or_else(|| {
        if Path::new("operon.cell").exists() {
            Some("operon.cell".to_string())
        } else {
            None
        }
    });
    let cell_is_explicit = opts.cell.is_some();
    if let Some(cp) = &cell_path {
        match std::fs::read_to_string(cp) {
            Ok(txt) => {
                interp.cell = genes::parse_cell(&txt);
            }
            Err(e) => interp.note(0, 4, format!("cell config '{}' unreadable: {}", cp, e)),
        }
    }
    if let Some(v) = &opts.variant {
        interp.cell.insert("cli.variant".into(), v.clone());
    }

    // RNA edit patches (hot patches)
    if let Some(rna_path) = &opts.rna {
        match std::fs::read_to_string(rna_path) {
            Ok(patch) => {
                let (s2, applied) = genes::apply_rna(&src, &patch, &stem);
                for a in &applied {
                    interp.note(0, 1, format!("rna edit applied: {}", a));
                }
                if applied.is_empty() {
                    interp.note(0, 4, format!("rna patch '{}' matched nothing", rna_path));
                }
                src = s2;
            }
            Err(e) => interp.note(0, 4, format!("rna patch '{}' unreadable: {}", rna_path, e)),
        }
    }

    let prog = parser::parse(&src);
    // surface parse-time notes at runtime too (Total Grammar transparency)
    for n in &prog.notes {
        interp.notes.push(n.clone());
    }
    interp.ires = prog.ires.clone();
    interp.cli_args = opts.args.clone();
    // capability grants: CLI flags + .cell allow.* keys (CLI wins)
    interp.caps = opts.caps.clone();
    let cell_pairs: Vec<(String, String)> = interp
        .cell
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (k, v) in cell_pairs {
        if let Some(rest) = k.strip_prefix("allow.") {
            if !cell_is_explicit {
                interp.note(0, 1, format!("cell key '{}={}' ignored: auto-detected operon.cell cannot grant capabilities (pass --cell explicitly)", k, v));
                continue;
            }
            let added = match rest {
                "read" => interp.caps.add_grant("read", &v),
                "write" => interp.caps.add_grant("write", &v),
                "run" => interp.caps.add_grant("run", &v),
                "net" => interp.caps.add_grant("net", &v),
                "env" => interp.caps.add_grant("env", &v),
                // sec-r2 (audit C-11): exit is a boolean capability
                "exit" => {
                    interp.caps.exit_allowed = v == "true";
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(s) = added {
                interp.note(0, 4, format!("invalid cell grant: {}", s.message));
            }
        }
    }
    // .cell entry (morphogen gradient: CLI --entry > cell entry > main > ires)
    if opts.entry.is_none() {
        if let Some(e) = interp.cell.get("entry") {
            interp.cell_entry = Some(e.clone());
        }
    }
    // .cell methylate.quiet
    if interp
        .cell
        .get("methylate.quiet")
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        interp.methyl_quiet = true;
    }
    // .cell methylate.threshold (T2b graded silencing gate; default 3)
    if let Some(t) = interp.cell.get("methylate.threshold") {
        match t.trim().parse::<u32>() {
            Ok(v) => interp.methyl_threshold = v,
            Err(_) => interp.note(
                0,
                4,
                format!(
                    "cell key 'methylate.threshold = {}' ignored: needs a non-negative integer",
                    t
                ),
            ),
        }
    }

    // execute top-level (gene defs bind, silences load, regulate registers…)
    // Top-Grammar containment: uncaught stress here is absorbed per statement.
    let genv = interp.global.clone();
    for stmt in &prog.stmts {
        if let Err(s) = interp.exec_stmt(&genv, stmt) {
            interp.note(
                0,
                4,
                format!("stress contained: [{}] {}", s.kind, s.message),
            );
        }
    }

    // frame selection: --frame name runs that named frame instead of entry
    if let Some(fname) = &opts.frame {
        if let Some((_, body)) = prog.named_frames.iter().find(|(n, _)| n == fname) {
            let _ = interp.exec_block(&genv, body);
        } else {
            interp.note(0, 4, format!("frame '{}' not declared", fname));
        }
    }

    Ok(Loaded { interp, prog })
}

pub fn resolve_entry(l: &mut Loaded, opts: &Opts) -> Option<String> {
    if let Some(e) = &opts.entry {
        return Some(e.clone());
    }
    if let Some(e) = &l.interp.cell_entry {
        return Some(e.clone());
    }
    // explicit --ires overrides the canonical main entry (cap-independent
    // initiation is chosen deliberately by the caller)
    if opts.use_ires {
        if let Some(first) = l.interp.ires.first().cloned() {
            l.interp.note(
                0,
                1,
                format!("cap-independent entry via --ires '{}'", first),
            );
            return Some(first);
        }
    }
    if l.prog
        .stmts
        .iter()
        .any(|s| matches!(s, Stmt::Gene(g) if g.name.as_deref() == Some("main")))
    {
        return Some("main".into());
    }
    if !l.interp.ires.is_empty() {
        if let Some(first) = l.interp.ires.first().cloned() {
            l.interp
                .note(0, 1, format!("cap-independent entry via ires '{}'", first));
            return Some(first);
        }
    }
    None
}

/// Run the entry gene (or nothing if no entry). Returns its value.
pub fn run_entry(l: &mut Loaded, opts: &Opts) -> Result<Value, Stress> {
    match resolve_entry(l, opts) {
        Some(entry) => {
            let argv = Value::List(std::rc::Rc::new(std::cell::RefCell::new(
                opts.args.iter().map(|a| Value::Str(a.clone())).collect(),
            )));
            let genv = l.interp.global.clone();
            let target = match genv.get(&entry) {
                Some(v @ Value::Gene(_, _)) => v,
                _ => {
                    // not a gene or missing: named-call path handles wobble/phantom
                    Value::Null
                }
            };
            match target {
                Value::Null => l.interp.call_named(&genv, &entry, vec![argv]),
                v => {
                    let has_params = match &v {
                        Value::Gene(d, _) => !d.params.is_empty(),
                        _ => false,
                    };
                    if has_params {
                        l.interp.call_value(&genv, &v, vec![argv])
                    } else {
                        l.interp.call_value(&genv, &v, vec![])
                    }
                }
            }
        }
        None => Ok(Value::Null),
    }
}

// ------------------------------------------------------------ check
pub struct CheckReport {
    pub score: i64,
    pub letter: char,
    pub notes: usize,
    pub wobbles: usize,
    pub fallbacks: usize,
    pub nmd: Vec<(String, String)>, // kind, message
    pub phantoms: Vec<String>,
    pub parsed: bool,
}

pub fn check(file: &str, opts: &Opts, nmd: bool, purge: bool) -> CheckReport {
    let mut rep = CheckReport {
        score: 100,
        letter: 'A',
        notes: 0,
        wobbles: 0,
        fallbacks: 0,
        nmd: Vec::new(),
        phantoms: Vec::new(),
        parsed: true,
    };
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            rep.parsed = false;
            rep.score = 50;
            rep.letter = 'F';
            rep.nmd
                .push(("error".into(), format!("cannot read '{}': {}", file, e)));
            return rep;
        }
    };
    let stem = Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut src2 = src.clone();
    if let Some(rna_path) = &opts.rna {
        if let Ok(patch) = std::fs::read_to_string(rna_path) {
            let (s2, _) = genes::apply_rna(&src, &patch, &stem);
            src2 = s2;
        }
    }
    let (rep, mut prog) = check_source(&src2, nmd);

    if purge {
        for s in prog.stmts.iter_mut() {
            purge_stmt(s);
        }
        let out = crate::tools::format_program(&prog);
        let _ = std::fs::write(file, out);
    }
    rep
}

/// In-memory core of `check` — the file-based `check()` delegates here, and
/// non-CLI tools (operon-ls) call it directly on editor buffers. Returns the
/// report AND the parsed program (the LSP hover table comes from it).
pub fn check_source(src: &str, nmd: bool) -> (CheckReport, Program) {
    let mut rep = CheckReport {
        score: 100,
        letter: 'A',
        notes: 0,
        wobbles: 0,
        fallbacks: 0,
        nmd: Vec::new(),
        phantoms: Vec::new(),
        parsed: true,
    };
    let prog = parser::parse(src);
    let rung2 = prog.notes.iter().filter(|n| n.rung == 2).count();
    rep.wobbles = prog.notes.iter().filter(|n| n.rung == 3).count();
    rep.fallbacks = prog.notes.iter().filter(|n| n.rung == 4).count();
    rep.notes = prog.notes.len();
    rep.score -= (rung2 as i64) + (rep.wobbles as i64) * 2 + (rep.fallbacks as i64) * 3;

    // all called names (for phantoms AND the NMD untranslated detector)
    let mut defined: HashSet<String> = HashSet::new();
    let mut called: Vec<String> = Vec::new();
    collect_calls(&prog, &mut defined, &mut called);
    // genes exported by `use`d modules count as defined (they are callable)
    let mut module_genes: HashSet<String> = HashSet::new();
    for s in &prog.stmts {
        if let Stmt::Use(path, _) = s {
            for cand in [format!("{}.op", path), format!("std/{}.op", path)] {
                if let Ok(msrc) = std::fs::read_to_string(&cand) {
                    let mp = parser::parse(&msrc);
                    for st in &mp.stmts {
                        match st {
                            Stmt::Gene(g) => {
                                if let Some(n) = &g.name {
                                    module_genes.insert(n.clone());
                                }
                            }
                            Stmt::Seq(g) => {
                                if let Some(n) = &g.name {
                                    module_genes.insert(n.clone());
                                }
                            }
                            Stmt::Splice(sp) => {
                                module_genes.insert(sp.root.clone());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    for c in &called {
        if !defined.contains(c)
            && !module_genes.contains(c)
            && !crate::interp::BUILTIN_NAMES.contains(&c.as_str())
        {
            rep.phantoms.push(c.clone());
        }
    }
    rep.score -= (rep.phantoms.len() as i64) * 2;

    // CTCF anchor import verification: every imported anchor must exist in
    // this file or be exported by a used module
    let mut imported: Vec<String> = Vec::new();
    for s in &prog.stmts {
        if let Stmt::AnchorImport(names) = s {
            imported.extend(names.clone());
        }
    }
    if !imported.is_empty() {
        let mut module_exports: HashSet<String> = HashSet::new();
        for s in &prog.stmts {
            if let Stmt::Use(path, _) = s {
                for cand in [format!("{}.op", path), format!("std/{}.op", path)] {
                    if let Ok(msrc) = std::fs::read_to_string(&cand) {
                        let mp = parser::parse(&msrc);
                        module_exports.extend(mp.anchor_exports.clone());
                        for (_, exps) in &mp.tad_exports {
                            module_exports.extend(exps.clone());
                        }
                        for st in &mp.stmts {
                            match st {
                                Stmt::Gene(g) => {
                                    if let Some(n) = &g.name {
                                        module_exports.insert(n.clone());
                                    }
                                }
                                Stmt::Splice(sp) => {
                                    module_exports.insert(sp.root.clone());
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        for n in &imported {
            if !defined.contains(n) && !module_exports.contains(n) {
                rep.nmd.push((
                    "anchor".into(),
                    format!("anchor import '{}' not found in file or used modules", n),
                ));
                rep.score -= 2;
            }
        }
    }

    // super-enhancer bonus: `enhance`d genes that are actually hot (called)
    // and codon-optimal in their naming earn back up to +6
    let mut enhanced: Vec<String> = Vec::new();
    for s in &prog.stmts {
        if let Stmt::Enhance(names) = s {
            enhanced.extend(names.clone());
        }
    }
    let mut bonus = 0i64;
    for name in &enhanced {
        if defined.contains(name) && called.contains(name) {
            let cs = crate::ffi::codon_score(name) as i64;
            bonus += (cs - 50) / 20; // 50-100 → 0..2 per hot gene
        }
    }
    rep.score += bonus.min(6);

    if nmd {
        let calledv: Vec<String> = called.clone();
        let findings = genes::nmd_sweep(&prog, &calledv, &enhanced);
        for f in &findings {
            rep.nmd.push((f.kind.to_string(), f.message.clone()));
        }
        rep.score -= findings
            .iter()
            .map(|f| if f.kind == "premature-stop" { 4 } else { 1 })
            .sum::<i64>();
    }
    rep.score = rep.score.max(50);
    rep.letter = grade_letter(rep.score);
    (rep, prog)
}

fn purge_stmt(s: &mut Stmt) {
    match s {
        Stmt::Gene(g) => {
            let g = std::sync::Arc::make_mut(g);
            genes::purge_premature_stops(&mut g.body);
        }
        Stmt::If(branches, els) => {
            for (_, b) in branches.iter_mut() {
                genes::purge_premature_stops(b);
            }
            if let Some(e) = els {
                genes::purge_premature_stops(e);
            }
        }
        Stmt::While(_, b)
        | Stmt::Loop(b)
        | Stmt::For(_, _, b)
        | Stmt::ForPat(_, _, b)
        | Stmt::Block(b)
        | Stmt::Tad(_, b) => {
            genes::purge_premature_stops(b);
        }
        Stmt::Frame { body, .. } => genes::purge_premature_stops(body),
        Stmt::Stress { body, rescue, .. } => {
            genes::purge_premature_stops(body);
            if let Some((_, rb)) = rescue {
                genes::purge_premature_stops(rb);
            }
        }
        _ => {}
    }
}

fn collect_calls(prog: &Program, defined: &mut HashSet<String>, called: &mut Vec<String>) {
    fn walk_expr(e: &Expr, called: &mut Vec<String>) {
        match e {
            Expr::Call(f, args, _) => {
                if let Expr::Ident(n) = &**f {
                    // record EVERY named call — the NMD untranslated detector
                    // needs the full transcription record, not just phantoms
                    called.push(n.clone());
                }
                walk_expr(f, called);
                for a in args {
                    walk_expr(a, called);
                }
            }
            Expr::Method(r, _, args) => {
                walk_expr(r, called);
                for a in args {
                    walk_expr(a, called);
                }
            }
            Expr::MethodSafe(r, _, args) => {
                walk_expr(r, called);
                for a in args {
                    walk_expr(a, called);
                }
            }
            Expr::Unary(_, a) | Expr::Member(a, _) | Expr::MemberSafe(a, _) => walk_expr(a, called),
            Expr::Binary(_, a, b, _) | Expr::Index(a, b, _) => {
                walk_expr(a, called);
                walk_expr(b, called);
            }
            Expr::List(xs) => xs.iter().for_each(|x| walk_expr(x, called)),
            Expr::Map(pairs) => pairs.iter().for_each(|(k, v)| {
                walk_expr(k, called);
                walk_expr(v, called);
            }),
            Expr::Interp(ps) => ps.iter().for_each(|p| {
                if let InterpPart::Expr(x) = p {
                    walk_expr(x, called)
                }
            }),
            Expr::Collect {
                iter, filter, body, ..
            } => {
                walk_expr(iter, called);
                if let Some(f) = filter {
                    walk_expr(f, called);
                }
                walk_expr(body, called);
            }
            _ => {}
        }
    }
    fn walk_stmts(stmts: &[Stmt], defined: &mut HashSet<String>, called: &mut Vec<String>) {
        for s in stmts {
            walk_stmt(s, defined, called);
        }
    }
    fn walk_stmt(s: &Stmt, defined: &mut HashSet<String>, called: &mut Vec<String>) {
        match s {
            Stmt::Gene(g) => {
                if let Some(n) = &g.name {
                    defined.insert(n.clone());
                }
                walk_stmts(&g.body, defined, called);
            }
            Stmt::Splice(sp) => {
                defined.insert(sp.root.clone());
                for (_, d) in &sp.variants {
                    walk_stmts(&d.body, defined, called);
                }
            }
            Stmt::Let(_, e)
            | Stmt::Assign(_, _, e)
            | Stmt::ExprStmt(e)
            | Stmt::Return(Some(e))
            | Stmt::Raise(_, e) => walk_expr(e, called),
            Stmt::IndexAssign(t, i, _, e) => {
                walk_expr(t, called);
                walk_expr(i, called);
                walk_expr(e, called);
            }
            Stmt::MemberAssign(t, _, _, e) => {
                walk_expr(t, called);
                walk_expr(e, called);
            }
            Stmt::If(bs, els) => {
                for (c, b) in bs {
                    walk_expr(c, called);
                    walk_stmts(b, defined, called);
                }
                if let Some(eb) = els {
                    walk_stmts(eb, defined, called);
                }
            }
            Stmt::While(c, b) => {
                walk_expr(c, called);
                walk_stmts(b, defined, called);
            }
            Stmt::Loop(b) => walk_stmts(b, defined, called),
            Stmt::For(_, it, b) => {
                walk_expr(it, called);
                walk_stmts(b, defined, called);
            }
            Stmt::Match(sub, cases) => {
                walk_expr(sub, called);
                for (p, b) in cases {
                    match p {
                        MatchPat::Lit(e) => walk_expr(e, called),
                        MatchPat::Multi(ls) => ls.iter().for_each(|e| walk_expr(e, called)),
                        _ => {}
                    }
                    walk_stmts(b, defined, called);
                }
            }
            Stmt::Stress { body, rescue, .. } => {
                walk_stmts(body, defined, called);
                if let Some((_, rb)) = rescue {
                    walk_stmts(rb, defined, called);
                }
            }
            Stmt::Frame { body, .. } | Stmt::Block(body) | Stmt::Tad(_, body) => {
                walk_stmts(body, defined, called)
            }
            _ => {}
        }
    }
    let mut d0 = defined.clone();
    // first pass: collect all gene names (forward refs allowed)
    for s in &prog.stmts {
        if let Stmt::Gene(g) = s {
            if let Some(n) = &g.name {
                d0.insert(n.clone());
            }
        }
        if let Stmt::Splice(sp) = s {
            d0.insert(sp.root.clone());
        }
    }
    let mut called2 = Vec::new();
    for s in &prog.stmts {
        walk_stmt(s, &mut d0, &mut called2);
    }
    *called = called2;
    *defined = d0;
}

pub fn grade_letter(score: i64) -> char {
    if score >= 90 {
        'A'
    } else if score >= 80 {
        'B'
    } else if score >= 70 {
        'C'
    } else if score >= 60 {
        'D'
    } else {
        'F'
    }
}

// ------------------------------------------------------------ profile
pub fn profile(file: &str, opts: &Opts) -> Loaded {
    // dx-r1 (audit W5): profiling must be ON before load — top-level
    // statements execute during load, and a script without a main() gene
    // (the common shape) previously reported 0.0 µs for everything. No
    // re-run: re-executing would double the program's side effects.
    let mut opts = opts.clone();
    opts.profile = true;
    let mut l = match load_file(file, &opts) {
        Ok(l) => l,
        Err(e) => crate::die(&format!("{}: {}", file, e)),
    };
    l.interp.profiling = true;
    let _ = run_entry(&mut l, &opts);
    l
}

// ------------------------------------------------------------ crispr
pub struct CrisprReport {
    pub knockout: String,
    pub proofs_total: usize,
    pub survivors: usize,
    pub failures: Vec<String>,
}

pub fn crispr(file: &str, opts: &Opts, knockout: &str) -> CrisprReport {
    let mut l = match load_file(file, opts) {
        Ok(l) => l,
        Err(e) => crate::die(&format!("{}: {}", file, e)),
    };
    let genv = l.interp.global.clone();
    // guide RNA: replace the gene body with return null
    if let Some(Value::Gene(d, _)) = genv.get(knockout) {
        let mut d2 = (*d).clone();
        d2.body = vec![Stmt::Return(Some(Expr::Null))];
        genv.define(knockout, Value::Gene(std::sync::Arc::new(d2), None));
        l.interp.note(
            0,
            1,
            format!("knockout: '{}' body replaced with return null", knockout),
        );
    } else {
        l.interp.note(
            0,
            4,
            format!("knockout target '{}' not found; screen skipped", knockout),
        );
    }
    let mut rep = CrisprReport {
        knockout: knockout.to_string(),
        proofs_total: 0,
        survivors: 0,
        failures: Vec::new(),
    };
    l.interp.proof_mode = true;
    for (i, proof) in l.prog.proofs.iter().enumerate() {
        rep.proofs_total += 1;
        let r = l.interp.exec_block(&genv, proof);
        match r {
            Ok(Flow::Norm) => {
                if l.interp.asserts_run == 0 {
                    rep.failures
                        .push(format!("proof #{}: no assertion exercised", i + 1));
                } else {
                    rep.survivors += 1;
                }
            }
            Ok(_) => rep.failures.push(format!("proof #{}: exited early", i + 1)),
            Err(s) => rep.failures.push(format!(
                "proof #{} failed: [{}] {}",
                i + 1,
                s.kind,
                s.message
            )),
        }
    }
    rep
}

// ------------------------------------------------------------ bench
pub struct BenchReport {
    pub iters: usize,
    pub min_ms: f64,
    pub avg_ms: f64,
}

pub fn bench(file: &str, opts: &Opts, iters: usize) -> BenchReport {
    let mut times = Vec::new();
    for _ in 0..iters {
        let t0 = crate::ffi::now_ns();
        let mut l = match load_file(file, opts) {
            Ok(l) => l,
            Err(e) => crate::die(&format!("{}: {}", file, e)),
        };
        let _ = run_entry(&mut l, opts);
        times.push((crate::ffi::now_ns() - t0) / 1e6);
    }
    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let avg = times.iter().sum::<f64>() / iters.max(1) as f64;
    BenchReport {
        iters,
        min_ms: min,
        avg_ms: avg,
    }
}

// ------------------------------------------------------------ test runner
pub struct TestReport {
    pub files: usize,
    pub proofs: usize,
    pub passed: usize,
    pub failed: usize,
    pub failures: Vec<String>,
    pub notes: usize,
    pub asserts: u64,
}

pub fn run_tests(paths: &[String], opts: &Opts, json: bool) -> TestReport {
    let mut rep = TestReport {
        files: 0,
        proofs: 0,
        passed: 0,
        failed: 0,
        failures: Vec::new(),
        notes: 0,
        asserts: 0,
    };
    let mut total_asserts = 0u64;
    let mut files: Vec<String> = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            collect_op_files(path, &mut files);
        } else {
            files.push(p.clone());
        }
    }
    files.sort();
    for f in &files {
        // dx-r3 (re-audit / A14 leftover): capture each file's program
        // stdout (promote output) instead of streaming it into the report;
        // captured lines are shown ONLY for the failing file
        let sink = Rc::new(RefCell::new(Vec::new()));
        let captured = sink.clone();
        let mut file_opts = opts.clone();
        file_opts.stdout_sink = Some(sink);
        let mut l = match load_file(f, &file_opts) {
            Ok(l) => l,
            Err(e) => {
                rep.failed += 1;
                rep.failures.push(format!("{}: load error: {}", f, e));
                continue;
            }
        };
        rep.files += 1;
        rep.notes += l.interp.notes.len();
        let genv = l.interp.global.clone();
        l.interp.proof_mode = true;
        let mut file_failed = false;
        let mut proof_failures: Vec<String> = Vec::new();
        for (i, proof) in l.prog.proofs.iter().enumerate() {
            rep.proofs += 1;
            let asserts_before = l.interp.asserts_run;
            match l.interp.exec_block(&genv, proof) {
                // a proof frame must run to completion — early return/break is
                // an integrity failure, not a pass (proofs are guard slides)
                Ok(Flow::Norm) => {
                    if l.interp.asserts_run == asserts_before {
                        rep.failed += 1;
                        file_failed = true;
                        proof_failures.push(format!(
                            "{} proof #{}: no assertion exercised (vacuous proof)",
                            f,
                            i + 1
                        ));
                    } else {
                        rep.passed += 1;
                    }
                }
                Ok(_) => {
                    rep.failed += 1;
                    file_failed = true;
                    proof_failures.push(format!(
                        "{} proof #{}: exited early (return/break inside proof)",
                        f,
                        i + 1
                    ));
                }
                Err(s) => {
                    rep.failed += 1;
                    file_failed = true;
                    proof_failures.push(format!(
                        "{} proof #{}: [{}] {}",
                        f,
                        i + 1,
                        s.kind,
                        s.message
                    ));
                }
            }
        }
        rep.failures.append(&mut proof_failures);
        if file_failed {
            let out = captured.borrow();
            if !out.is_empty() {
                rep.failures
                    .push(format!("{} captured stdout:\n{}", f, out.join("\n")));
            }
        }
        total_asserts += l.interp.asserts_run;
        rep.asserts = total_asserts;
    }
    if !json {
        println!(
            "operon test — {} file(s), {} proof(s): {} passed, {} failed ({} assertion(s) exercised)",
            rep.files, rep.proofs, rep.passed, rep.failed, total_asserts
        );
        for f in &rep.failures {
            eprintln!("  FAIL {}", f);
        }
        if rep.notes > 0 {
            eprintln!("  ({} wobble note(s) absorbed during test runs)", rep.notes);
        }
    }
    rep
}

fn collect_op_files(dir: &Path, out: &mut Vec<String>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                // the red-team suite is adversarial by design (hangs, bombs,
                // escapes) — it is exercised by scripts/redteam.sh with
                // containment expectations, never by the proof runner
                if p.file_name().map(|n| n == "redteam").unwrap_or(false) {
                    continue;
                }
                collect_op_files(&p, out);
            } else if p.extension().map(|x| x == "op").unwrap_or(false) {
                out.push(p.to_string_lossy().to_string());
            }
        }
    }
}

// ------------------------------------------------------------ formatter
pub fn format_program(prog: &Program) -> String {
    let mut out = String::new();
    for s in &prog.stmts {
        fmt_stmt(s, 0, &mut out);
    }
    out
}

fn indent(n: usize) -> String {
    "  ".repeat(n)
}

fn fmt_block(stmts: &[Stmt], ind: usize, out: &mut String) {
    out.push_str("{\n");
    for s in stmts {
        fmt_stmt(s, ind + 1, out);
    }
    out.push_str(&indent(ind));
    out.push('}');
}

fn fmt_stmt(s: &Stmt, ind: usize, out: &mut String) {
    out.push_str(&indent(ind));
    match s {
        Stmt::Seq(g) => {
            out.push_str(&format!(
                "sequence {}({}) ",
                g.name.clone().unwrap_or_default(),
                fmt_params(&g.params)
            ));
            fmt_block(&g.body, ind, out);
            out.push_str("\n\n");
        }
        Stmt::Yield(Some(e)) => out.push_str(&format!("yield {}\n", fmt_expr(e))),
        Stmt::Yield(None) => out.push_str("yield\n"),
        Stmt::Pheno(p) => {
            match &p.parent {
                Some(par) => out.push_str(&format!("phenotype {} from {} ", p.name, par)),
                None => out.push_str(&format!("phenotype {} ", p.name)),
            }
            out.push_str("{\n");
            for (fname, fexpr) in &p.fields {
                out.push_str(&indent(ind + 1));
                out.push_str(&format!("let {} = {}\n", fname, fmt_expr(fexpr)));
            }
            for g in &p.methods {
                if g.acetylate {
                    out.push_str(&indent(ind + 1));
                    out.push_str("@acetylate ");
                }
                if g.methylate {
                    out.push_str(&indent(ind + 1));
                    out.push_str("@methylate ");
                }
                if g.m6a {
                    out.push_str(&indent(ind + 1));
                    out.push_str("@m6a ");
                }
                out.push_str(&indent(ind + 1));
                out.push_str(&format!(
                    "gene {}({}) ",
                    g.name.clone().unwrap_or_default(),
                    fmt_params(&g.params)
                ));
                fmt_block(&g.body, ind + 1, out);
                out.push('\n');
            }
            out.push_str(&indent(ind));
            out.push_str("}\n\n");
        }
        Stmt::Let(n, e) => {
            out.push_str(&format!("let {} = {}\n", n, fmt_expr(e)));
        }
        Stmt::LetPat(p, e) => {
            out.push_str(&format!(
                "let {} = {}\n",
                fmt_destructure_pat(p),
                fmt_expr(e)
            ));
        }
        Stmt::ForPat(p, it, b) => {
            out.push_str(&format!(
                "for {} in {} ",
                fmt_destructure_pat(p),
                fmt_expr(it)
            ));
            fmt_block(b, ind, out);
            out.push('\n');
        }
        Stmt::MultiAssign(ts, vs, true) => {
            let tj: Vec<String> = ts.iter().map(fmt_expr).collect();
            let vj: Vec<String> = vs.iter().map(fmt_expr).collect();
            out.push_str(&format!("let {} = {}\n", tj.join(", "), vj.join(", ")));
        }
        Stmt::MultiAssign(ts, vs, false) => {
            let tj: Vec<String> = ts.iter().map(fmt_expr).collect();
            let vj: Vec<String> = vs.iter().map(fmt_expr).collect();
            out.push_str(&format!("{} = {}\n", tj.join(", "), vj.join(", ")));
        }
        Stmt::Assign(n, None, e) => {
            out.push_str(&format!("{} = {}\n", n, fmt_expr(e)));
        }
        Stmt::Assign(n, Some(op), e) => {
            out.push_str(&format!("{} {}= {}\n", n, fmt_op(*op), fmt_expr(e)));
        }
        Stmt::IndexAssign(t, i, None, e) => {
            out.push_str(&format!(
                "{}[{}] = {}\n",
                fmt_expr(t),
                fmt_expr(i),
                fmt_expr(e)
            ));
        }
        Stmt::IndexAssign(t, i, Some(op), e) => {
            out.push_str(&format!(
                "{}[{}] {}= {}\n",
                fmt_expr(t),
                fmt_expr(i),
                fmt_op(*op),
                fmt_expr(e)
            ));
        }
        Stmt::MemberAssign(t, k, None, e) => {
            out.push_str(&format!("{}.{} = {}\n", fmt_expr(t), k, fmt_expr(e)));
        }
        Stmt::MemberAssign(t, k, Some(op), e) => {
            out.push_str(&format!(
                "{}.{} {}= {}\n",
                fmt_expr(t),
                k,
                fmt_op(*op),
                fmt_expr(e)
            ));
        }
        Stmt::If(branches, els) => {
            for (i, (c, b)) in branches.iter().enumerate() {
                if i > 0 {
                    out.push_str(&indent(ind));
                }
                out.push_str(&format!(
                    "{} {} ",
                    if i == 0 { "if" } else { "elif" },
                    fmt_expr(c)
                ));
                fmt_block(b, ind, out);
                out.push('\n');
            }
            if let Some(eb) = els {
                out.push_str(&indent(ind));
                out.push_str("else ");
                fmt_block(eb, ind, out);
                out.push('\n');
            }
        }
        Stmt::While(c, b) => {
            out.push_str(&format!("while {} ", fmt_expr(c)));
            fmt_block(b, ind, out);
            out.push('\n');
        }
        Stmt::Loop(b) => {
            out.push_str("loop ");
            fmt_block(b, ind, out);
            out.push('\n');
        }
        Stmt::For(n, it, b) => {
            out.push_str(&format!("for {} in {} ", n, fmt_expr(it)));
            fmt_block(b, ind, out);
            out.push('\n');
        }
        Stmt::Return(Some(e)) => out.push_str(&format!("return {}\n", fmt_expr(e))),
        Stmt::Return(None) => out.push_str("return\n"),
        Stmt::Break => out.push_str("break\n"),
        Stmt::Continue => out.push_str("continue\n"),
        Stmt::ExprStmt(e) => out.push_str(&format!("{}\n", fmt_expr(e))),
        Stmt::Match(sub, cases) => {
            out.push_str(&format!("match {} ", fmt_expr(sub)));
            out.push_str("{\n");
            for (p, b) in cases {
                out.push_str(&indent(ind + 1));
                out.push_str(&format!("case {} ", fmt_pat(p)));
                fmt_block(b, ind + 1, out);
                out.push('\n');
            }
            out.push_str(&indent(ind));
            out.push_str("}\n");
        }
        Stmt::Use(p, alias) => {
            match alias {
                Some(a) => out.push_str(&format!("use {} as {}\n", p, a)),
                None => out.push_str(&format!("use {}\n", p)),
            };
        }
        Stmt::Raise(k, e) => match k {
            Some(k) => out.push_str(&format!("raise {}, {}\n", k, fmt_expr(e))),
            None => out.push_str(&format!("raise {}\n", fmt_expr(e))),
        },
        Stmt::Stress { kind, body, rescue } => {
            match kind {
                Some(k) => out.push_str(&format!("stress {} ", k)),
                None => out.push_str("stress "),
            }
            fmt_block(body, ind, out);
            out.push('\n');
            if let Some((bind, rb)) = rescue {
                out.push_str(&indent(ind));
                match bind {
                    Some(b) => out.push_str(&format!("rescue ({}) ", b)),
                    None => out.push_str("rescue "),
                }
                fmt_block(rb, ind, out);
                out.push('\n');
            }
        }
        Stmt::Gene(g) => {
            if g.acetylate {
                out.push_str("@acetylate ");
            }
            if g.methylate {
                out.push_str("@methylate ");
            }
            if g.m6a {
                out.push_str("@m6a ");
            }
            match &g.name {
                Some(n) => out.push_str(&format!("gene {}({})", n, fmt_params(&g.params))),
                None => out.push_str(&format!("gene({})", fmt_params(&g.params))),
            }
            if let Some((c, gb)) = &g.guard {
                out.push_str(&format!(" guard ({}) ", fmt_expr(c)));
                fmt_block(gb, ind, out);
                out.push(' ');
            }
            fmt_block(&g.body, ind, out);
            out.push_str("\n\n");
        }
        Stmt::Splice(sp) => {
            out.push_str(&format!("splice {} ", sp.root));
            out.push_str("{\n");
            for (vn, d) in &sp.variants {
                out.push_str(&indent(ind + 1));
                // T2c round-trip: variant marks must survive fmt
                if d.m6a {
                    out.push_str("@m6a ");
                }
                if d.methylate {
                    out.push_str("@methylate ");
                }
                if d.acetylate {
                    out.push_str("@acetylate ");
                }
                // round-trip variant params (retired the SPEC §11 build caveat)
                if d.params.is_empty() {
                    out.push_str(&format!("variant {} ", vn));
                } else {
                    let ps: Vec<String> = d
                        .params
                        .iter()
                        .map(|(n, defv)| match defv {
                            Some(e) => format!("{} = {}", n, fmt_expr(e)),
                            None => n.clone(),
                        })
                        .collect();
                    out.push_str(&format!("variant {}({}) ", vn, ps.join(", ")));
                }
                fmt_block(&d.body, ind + 1, out);
                out.push('\n');
            }
            out.push_str(&indent(ind));
            out.push_str("}\n\n");
        }
        Stmt::Silence(f, t) => match t {
            Some(t) => out.push_str(&format!("silence {} -> {}\n", f, t)),
            None => out.push_str(&format!("silence {}\n", f)),
        },
        Stmt::Enhance(ns) => out.push_str(&format!("enhance {};\n", ns.join(", "))),
        Stmt::Ires(n) => out.push_str(&format!("ires {};\n", n)),
        Stmt::Fate(f) => {
            out.push_str(&format!("fate {} ", f.name));
            out.push_str("{\n");
            for (st, tg) in &f.states {
                out.push_str(&indent(ind + 1));
                if tg.is_empty() {
                    out.push_str(&format!("state {};\n", st));
                } else {
                    out.push_str(&format!("state {} -> {};\n", st, tg.join(", ")));
                }
            }
            if let Some(e) = &f.enter {
                out.push_str(&indent(ind + 1));
                out.push_str(&format!("enter {};\n", e));
            }
            out.push_str(&indent(ind));
            out.push_str("}\n\n");
        }
        Stmt::Regulate(edges) => {
            out.push_str("regulate {\n");
            for e in edges {
                out.push_str(&indent(ind + 1));
                out.push_str(&format!(
                    "{} {} {}{};\n",
                    e.from,
                    if e.inhibit { "inhibits" } else { "activates" },
                    e.to,
                    if e.strength != 1.0 {
                        format!(" strength {}", e.strength)
                    } else {
                        String::new()
                    }
                ));
            }
            out.push_str(&indent(ind));
            out.push_str("}\n\n");
        }
        Stmt::Toggle(a, b) => out.push_str(&format!("toggle {}, {};\n", a, b)),
        Stmt::Repressilator(ring, period) => {
            out.push_str(&format!("repressilator {}", ring.join(" -> ")));
            if let Some(p) = period {
                out.push_str(&format!(" period {}", p));
            }
            out.push_str(";\n");
        }
        Stmt::Frame {
            name,
            is_proof,
            body,
        } => {
            if *is_proof {
                out.push_str("frame proof ");
            } else {
                out.push_str(&format!("frame {} ", name));
            }
            fmt_block(body, ind, out);
            out.push_str("\n\n");
        }
        Stmt::Edit(t, reps) => {
            out.push_str(&format!("edit {} ", t));
            out.push_str("{\n");
            for (f, to) in reps {
                out.push_str(&indent(ind + 1));
                // escape pattern text: a raw quote inside a pattern would
                // corrupt the formatted file
                out.push_str(&format!(
                    "replace \"{}\" -> \"{}\";\n",
                    json_escape(f),
                    json_escape(to)
                ));
            }
            out.push_str(&indent(ind));
            out.push_str("}\n\n");
        }
        Stmt::AnchorExport(ns) => out.push_str(&format!("anchor export {};\n", ns.join(", "))),
        Stmt::AnchorImport(ns) => out.push_str(&format!("anchor import {};\n", ns.join(", "))),
        Stmt::Tad(n, body) => {
            out.push_str(&format!("tad {} ", n));
            fmt_block(body, ind, out);
            out.push_str("\n\n");
        }
        Stmt::Block(body) => {
            fmt_block(body, ind, out);
            out.push('\n');
        }
    }
}

fn fmt_pat(p: &MatchPat) -> String {
    match p {
        MatchPat::Lit(e) => fmt_expr(e),
        MatchPat::Multi(ls) => ls.iter().map(fmt_expr).collect::<Vec<_>>().join(", "),
        MatchPat::Bind(n) => n.clone(),
        MatchPat::Wild => "_".into(),
    }
}

fn fmt_params(ps: &[(String, Option<Expr>)]) -> String {
    ps.iter()
        .map(|(n, d)| match d {
            Some(e) => format!("{} = {}", n, fmt_expr(e)),
            None => n.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// L1a: canonical form of a destructuring pattern.
fn fmt_destructure_pat(p: &Pat) -> String {
    match p {
        Pat::Bind(n) => n.clone(),
        Pat::List { elems, rest } => {
            let mut parts: Vec<String> = elems.iter().map(fmt_destructure_pat).collect();
            if let Some(r) = rest {
                parts.push(format!("*{}", r));
            }
            format!("[{}]", parts.join(", "))
        }
        Pat::Map { keys } => format!("{{{}}}", keys.join(", ")),
    }
}

pub fn fmt_op(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::FloorDiv => "//",
        BinOp::Mod => "%",
        BinOp::Pow => "**",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Eq => "==",
        BinOp::Neq => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::In => "in",
        BinOp::Nullish => "??",
    }
}

fn prec_of(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::Nullish | BinOp::And => 2,
        BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In => 3,
        BinOp::BitOr => 4,
        BinOp::BitXor => 5,
        BinOp::BitAnd => 6,
        BinOp::Shl | BinOp::Shr => 7,
        BinOp::Add | BinOp::Sub => 8,
        BinOp::Mul | BinOp::Div | BinOp::FloorDiv | BinOp::Mod => 9,
        BinOp::Pow => 10,
    }
}

/// Precedence of an expression when nested (11 = atom/postfix, no parens ever).
fn nest_prec(e: &Expr) -> u8 {
    match e {
        Expr::Binary(op, _, _, _) => prec_of(*op),
        Expr::Ternary(_, _, _) => 1,
        Expr::Unary(crate::ast::UnOp::Not, _) => 2,
        Expr::Unary(crate::ast::UnOp::BitNot, _) => 11,
        Expr::Unary(crate::ast::UnOp::Neg, _) => 10,
        _ => 11,
    }
}

pub fn fmt_expr(e: &Expr) -> String {
    fmt_prec(e, 0)
}

fn fmt_prec(e: &Expr, parent: u8) -> String {
    let needs_paren = nest_prec(e) < parent;
    let body = match e {
        Expr::Null => "null".into(),
        Expr::Bool(true) => "true".into(),
        Expr::Bool(false) => "false".into(),
        Expr::Int(i) => i.to_string(),
        Expr::Float(f) => crate::value::format_float(*f),
        Expr::Str(s) => format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
                .replace('{', "\\{")
                .replace('}', "\\}")
        ),
        Expr::Interp(parts) => {
            let mut out = String::from("\"");
            for p in parts {
                match p {
                    InterpPart::Lit(s) => {
                        // literal segments may contain braces that came from
                        // \\{ \\} escapes — re-escape them or fmt corrupts the file
                        out.push_str(
                            &s.replace('\\', "\\\\")
                                .replace('"', "\\\"")
                                .replace('\n', "\\n")
                                .replace('\t', "\\t")
                                .replace('{', "\\{")
                                .replace('}', "\\}"),
                        );
                    }
                    InterpPart::Expr(x) => {
                        out.push('{');
                        out.push_str(&fmt_expr(x));
                        out.push('}');
                    }
                }
            }
            out.push('"');
            out
        }
        Expr::List(xs) => format!(
            "[{}]",
            xs.iter()
                .map(|x| fmt_prec(x, 0))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Map(pairs) => format!(
            "{{{}}}",
            pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", fmt_prec(k, 0), fmt_prec(v, 0)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Ident(n) => n.clone(),
        Expr::Unary(crate::ast::UnOp::Neg, a) => {
            let inner = fmt_prec(a, 11);
            if nest_prec(a) < 10 {
                format!("-({})", inner)
            } else {
                format!("-{}", inner)
            }
        }
        Expr::Unary(crate::ast::UnOp::BitNot, a) => {
            let inner = fmt_prec(a, 11);
            if nest_prec(a) < 11 {
                format!("~({})", inner)
            } else {
                format!("~{}", inner)
            }
        }
        Expr::Unary(crate::ast::UnOp::Not, a) => {
            let inner = fmt_prec(a, 3);
            if nest_prec(a) <= 2 {
                format!("not ({})", inner)
            } else {
                format!("not {}", inner)
            }
        }
        Expr::Binary(op, a, b, _) => {
            let p = prec_of(*op);
            // left-assoc: left child may reuse p, right child must be tighter
            let left = fmt_prec(a, p);
            let right = fmt_prec(b, p + 1);
            format!("{} {} {}", left, fmt_op(*op), right)
        }
        Expr::Call(f, args, _) => format!(
            "{}({})",
            fmt_expr(f),
            args.iter()
                .map(|x| fmt_prec(x, 0))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Index(t, i, _) => format!("{}[{}]", fmt_expr(t), fmt_expr(i)),
        Expr::Member(t, k) => format!("{}.{}", fmt_expr(t), k),
        Expr::MemberSafe(t, k) => format!("{}?.{}", fmt_expr(t), k),
        Expr::MethodSafe(t, m, args) => format!(
            "{}?.{}({})",
            fmt_expr(t),
            m,
            args.iter()
                .map(|x| fmt_prec(x, 0))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Method(t, m, args) => format!(
            "{}.{}({})",
            fmt_expr(t),
            m,
            args.iter()
                .map(|x| fmt_prec(x, 0))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Lambda(g) => format!(
            "gene({}) => {}",
            fmt_params(&g.params),
            fmt_body_inline(&g.body)
        ),
        Expr::FateNew(n) => format!("{}()", n),
        Expr::Collect {
            var,
            iter,
            filter,
            body,
        } => {
            let f = match filter {
                Some(f) => format!(" if {}", fmt_expr(f)),
                None => String::new(),
            };
            format!(
                "for {} in {}{} collect {}",
                var,
                fmt_expr(iter),
                f,
                fmt_expr(body)
            )
        }
        Expr::Ternary(c, a, b) => {
            // cond slot: a nested ternary MUST be parenthesized — `a ? 0 : 2
            // ? 3 : 4` re-parses as `a ? 0 : (2 ? 3 : 4)` and silently changes
            // meaning (wave-3 Critic-Q semantics bug)
            format!("{} ? {} : {}", fmt_prec(c, 2), fmt_expr(a), fmt_expr(b))
        }
        Expr::New(n, args) => format!(
            "new {}({})",
            n,
            args.iter()
                .map(|x| fmt_prec(x, 0))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    if needs_paren {
        format!("({})", body)
    } else {
        body
    }
}

fn fmt_body_inline(body: &[Stmt]) -> String {
    if let Some(Stmt::Return(Some(e))) = body.first() {
        fmt_expr(e)
    } else {
        "null".into()
    }
}

// ------------------------------------------------------------ json
pub fn json_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other if (other as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", other as u32)),
            other => out.push(other),
        }
    }
    out
}

pub fn flush_notes(l: &Loaded, quiet: bool) {
    if quiet {
        return;
    }
    let err = std::io::stderr();
    let mut w = err.lock();
    for n in &l.interp.notes {
        let tag = match n.rung {
            1 => "info",
            2 => "synonym",
            3 => "wobble",
            _ => "fallback",
        };
        // A13 (dx-r2): real locations when the note carries a line
        if n.line > 0 {
            let _ = writeln!(w, "[{}] {}:{}: {}", tag, l.interp.file, n.line, n.message);
        } else {
            let _ = writeln!(w, "[{}] {}", tag, n.message);
        }
    }
}
