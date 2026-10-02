//! tools.rs, toolchain subcommands: run entry resolution, check/NMD grading,
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
    /// dx-r1 (audit W5): time top-level statements during load, without
    /// this, `operon profile` reported 0.0 µs for any script without main().
    pub profile: bool,
    /// W097-A: per-call span capture (Chrome-trace feed). Set by the CLI's
    /// `--chrome <file>` flag; consumed by `operon profile`. Must live on
    /// Opts (not just the interp) because load_file constructs the interp
    /// BEFORE the profile() wrapper can flag it — top-level gene calls
    /// executed during load must be in the timeline too (the dx-r1 lesson).
    pub spans: bool,
    /// dx-r3 (re-audit / A14): when set, program stdout (promote) is
    /// captured here from the moment the interp exists, the test runner
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
    // W097-A: span capture rides the same pre-load wiring as profiling —
    // load-time calls land in the timeline, not just run_entry() calls.
    interp.spans = opts.spans;
    // dx-r3: capture program stdout from load time (test runner)
    interp.stdout_sink = opts.stdout_sink.clone();
    // A13 (dx-r2): diagnostics render file:line
    interp.file = file.to_string();
    // base dir for module resolution (relative to the importing file)
    interp.base_dir = std::path::Path::new(file)
        .parent()
        .map(|p| p.to_string_lossy().to_string());
    // W19/W23: vendored dependency roots, when an operon.lock sits beside
    // the program (or in the CWD), its resolved packages join the module
    // resolution chain as root 7 (SPEC §8 table), so `use my-lib/…` runs
    // offline from the vendored cache.
    crate::pkg::apply_lock(&mut interp);

    // methylation layer: CLI --cell, else operon.cell auto-detect.
    // SECURITY POLICY: an auto-detected cell config may configure entry/
    // variant/quiet keys, but its allow.* keys are IGNORED, capability
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
                // cell values may carry a comma-separated grant list
                // ("py = math, json"), CLI flags stay one-per-flag
                "read" | "write" | "run" | "net" | "env" | "py" => {
                    let mut res = Ok(());
                    for g in v.split(',') {
                        let g = g.trim();
                        if g.is_empty() {
                            continue;
                        }
                        res = interp.caps.add_grant(rest, g);
                        if res.is_err() {
                            break;
                        }
                    }
                    res
                }
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
    // reg-bio (F-6): enhancer dose (default ENHANCE_DELTA = 0.25)
    if let Some(v) = interp.cell.get("enhance.delta") {
        match v.trim().parse::<f64>() {
            Ok(f) if (0.0..=1.0).contains(&f) => interp.enhance_delta = f,
            _ => interp.note(
                0,
                4,
                format!(
                    "cell key 'enhance.delta = {}' ignored: needs a number in 0..=1",
                    v
                ),
            ),
        }
    }
    // reg-bio (F-1): the telegraph promoter layer, opt-in stochastic
    // expression. kon/koff are switch probabilities per call attempt.
    if interp
        .cell
        .get("expression.stochastic")
        .map(|v| v.trim() == "true")
        .unwrap_or(false)
    {
        interp.expr_stochastic = true;
    }
    if let Some(v) = interp.cell.get("expression.kon") {
        match v.trim().parse::<f64>() {
            Ok(f) if (0.0..=1.0).contains(&f) => interp.expr_kon = f,
            _ => interp.note(
                0,
                4,
                format!(
                    "cell key 'expression.kon = {}' ignored: needs a number in 0..=1",
                    v
                ),
            ),
        }
    }
    if let Some(v) = interp.cell.get("expression.koff") {
        match v.trim().parse::<f64>() {
            Ok(f) if (0.0..=1.0).contains(&f) => interp.expr_koff = f,
            _ => interp.note(
                0,
                4,
                format!(
                    "cell key 'expression.koff = {}' ignored: needs a number in 0..=1",
                    v
                ),
            ),
        }
    }
    // expression.seed reseeds the SHARED mirrored xorshift64* stream the
    // promoter draws ride, reproducible bursting across runs/implementations
    if let Some(v) = interp.cell.get("expression.seed") {
        match v.trim().parse::<i64>() {
            Ok(s) => interp.rng = if s == 0 { 0x9E3779B97F4A7C15 } else { s as u64 },
            Err(_) => interp.note(
                0,
                4,
                format!(
                    "cell key 'expression.seed = {}' ignored: needs an integer",
                    v
                ),
            ),
        }
    }
    // reg-bio (F-5): repressilator kinetics, plasmid engineering surface.
    // Defaults are the historical constants; unset keys change nothing.
    if let Some(v) = interp.cell.get("repressi.alpha") {
        match v.trim().parse::<f64>() {
            Ok(f) if f > 0.0 => interp.repressi_params.alpha = f,
            _ => interp.note(
                0,
                4,
                format!(
                    "cell key 'repressi.alpha = {}' ignored: needs a number > 0",
                    v
                ),
            ),
        }
    }
    if let Some(v) = interp.cell.get("repressi.gamma") {
        match v.trim().parse::<f64>() {
            Ok(f) if f >= 0.0 => interp.repressi_params.gamma = f,
            _ => interp.note(
                0,
                4,
                format!(
                    "cell key 'repressi.gamma = {}' ignored: needs a number >= 0",
                    v
                ),
            ),
        }
    }
    if let Some(v) = interp.cell.get("repressi.hill") {
        match v.trim().parse::<u32>() {
            Ok(n) if (1..=8).contains(&n) => interp.repressi_params.hill = n,
            _ => interp.note(
                0,
                4,
                format!(
                    "cell key 'repressi.hill = {}' ignored: needs an integer 1..=8",
                    v
                ),
            ),
        }
    }
    if let Some(v) = interp.cell.get("repressi.basal") {
        match v.trim().parse::<f64>() {
            Ok(f) if f >= 0.0 => interp.repressi_params.basal = f,
            _ => interp.note(
                0,
                4,
                format!(
                    "cell key 'repressi.basal = {}' ignored: needs a number >= 0",
                    v
                ),
            ),
        }
    }
    if let Some(v) = interp.cell.get("repressi.noise") {
        match v.trim().parse::<f64>() {
            Ok(f) if (0.0..=1.0).contains(&f) => interp.repressi_params.noise = f,
            _ => interp.note(
                0,
                4,
                format!(
                    "cell key 'repressi.noise = {}' ignored: needs a number in 0..=1",
                    v
                ),
            ),
        }
    }
    if let Some(v) = interp.cell.get("repressi.seed") {
        match v.trim().parse::<i64>() {
            Ok(s) => {
                interp.repressi_params.seed = if s == 0 { 0x9E3779B97F4A7C15 } else { s as u64 }
            }
            Err(_) => interp.note(
                0,
                4,
                format!("cell key 'repressi.seed = {}' ignored: needs an integer", v),
            ),
        }
    }

    // execute top-level (gene defs bind, silences load, regulate registers…)
    // Top-Grammar containment: uncaught stress here is absorbed per statement.
    let genv = interp.global.clone();
    for stmt in &prog.stmts {
        if let Err(s) = interp.exec_stmt(&genv, stmt) {
            // W06 (D-014): propagation with no enclosing gene, the variant
            // value passes through (Total Grammar: noted, never rejected).
            // Never leaked as kind "propagate": the payload marker converts.
            if let Some(v) = s.prop {
                interp.note(
                    s.line,
                    4,
                    format!("propagation reached top level: {} passes through", v.repr()),
                );
            } else {
                interp.note(
                    s.line,
                    4,
                    format!("stress contained: [{}] {}", s.kind, s.message),
                );
            }
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
            let invoked = match target {
                Value::Null => l.interp.call_named(&genv, &entry, vec![argv], None),
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
            };
            match invoked {
                Ok(v) => Ok(v),
                Err(mut s) => {
                    // W007: the entry gene is invoked by the RUNTIME (no call
                    // expression), so its call_gene frame carries a stale
                    // top-level line. The entry invocation is the OUTERMOST
                    // gene frame, rewrite its line to 0 (renders as a bare
                    // `at entry`) instead of appending a duplicate. Deep
                    // chains capped at 64 keep their innermost frames only.
                    if s.chain.is_empty() {
                        s.chain.push((entry.clone(), 0));
                    } else if let Some(last) = s.chain.last_mut() {
                        if last.0 == entry {
                            last.1 = 0;
                        } else if s.chain.len() < 64 {
                            s.chain.push((entry.clone(), 0));
                        }
                    }
                    Err(s)
                }
            }
        }
        None => Ok(Value::Null),
    }
}

// ------------------------------------------------------------ check
/// W101 slice 6: a phantom call with its evidence. `line` is the call site
/// when the AST carries it (Expr::Call stamps its line), else 0 — an unknown
/// location stays 0, never guessed. `suggestions` is the did-you-mean
/// shortlist against defined genes + module exports + builtins; `fix` is the
/// machine-applicable edit when the name token is locatable on its line.
pub struct PhantomCall {
    pub name: String,
    pub line: usize,
    pub suggestions: Vec<String>,
    pub fix: Option<crate::diag::SuggestedFix>,
}

pub struct CheckReport {
    pub score: i64,
    pub letter: char,
    pub notes: usize,
    pub wobbles: usize,
    pub fallbacks: usize,
    pub nmd: Vec<(String, String)>, // kind, message
    pub phantoms: Vec<PhantomCall>,
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
    let (rep, mut prog) = check_source(&src2, nmd, None);

    if purge {
        for s in prog.stmts.iter_mut() {
            purge_stmt(s);
        }
        let out = crate::tools::format_program(&prog);
        let _ = std::fs::write(file, out);
    }
    rep
}

/// lsp-r1 (P0): where a `use`d module's source might live, the same
/// candidate list the runtime's gene loader resolves (document-relative,
/// CWD-relative, std/, exe-relative std/). The LSP launches from arbitrary
/// working directories (editors pick the CWD), so without the exe-relative
/// candidate `use math` + `mean(...)` produced FALSE "phantom call"
/// diagnostics, the demo-works/real-code-breaks failure mode, reproduced
/// live by the loop-5-b audit.
pub fn module_candidates(path: &str, base_dir: Option<&str>) -> Vec<std::path::PathBuf> {
    // W025 stage 2: a wildcard `use a/b/*` carries the `*` in the path
    // string; the check side reads module SOURCE for gene names, so strip
    // the wildcard tail and read the base path (the runtime descends nested
    // tables for the same import, here the flat file's names are the honest
    // superset the checker can see without running the program).
    let path = path.strip_suffix("/*").unwrap_or(path);
    let p = format!("{}.op", path.trim_end_matches(".op"));
    let mut out = Vec::new();
    if let Some(base) = base_dir {
        out.push(std::path::PathBuf::from(base).join(&p));
    }
    out.push(std::path::PathBuf::from(&p));
    out.push(std::path::PathBuf::from("std").join(&p));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("std").join(&p));
        }
    }
    // dev-checkout candidate: the cargo manifest dir (compiled in). A source
    // checkout runs operon-ls from target/release/, where exe-relative std/
    // does not exist, the stdlib sits at the manifest root. Installed
    // binaries are covered by the exe-relative candidate above.
    out.push(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("std")
            .join(&p),
    );
    out
}

/// In-memory core of `check`, the file-based `check()` delegates here, and
/// non-CLI tools (operon-ls) call it directly on editor buffers. Returns the
/// report AND the parsed program (the LSP hover table comes from it).
/// `base_dir` (when known, e.g. the LSP document's directory) is searched
/// first for `use`d modules.
pub fn check_source(src: &str, nmd: bool, base_dir: Option<&str>) -> (CheckReport, Program) {
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

    // all called names + call-site lines (for phantoms, their did-you-mean
    // suggestions, AND the NMD untranslated detector)
    let mut defined: HashSet<String> = HashSet::new();
    let mut called: Vec<(String, usize)> = Vec::new();
    collect_calls(&prog, &mut defined, &mut called);
    // genes exported by `use`d modules count as defined (they are callable)
    let mut module_genes: HashSet<String> = HashSet::new();
    for s in &prog.stmts {
        if let Stmt::Use(path, _) = s {
            for cand in module_candidates(path, base_dir) {
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
    for (c, line) in &called {
        if !defined.contains(c)
            && !module_genes.contains(c)
            && !crate::interp::BUILTIN_NAMES.contains(&c.as_str())
            // builtin synonyms (print/echo/say/show for promote) are real
            // builtins on the wire; without this check `print` read as a
            // phantom (surfaced by the batch-2 check probes)
            && !crate::interp::BUILTIN_SYNONYMS.iter().any(|(s, _)| s == c)
        {
            // W101 slice 6: did-you-mean against every name this file can
            // legitimately call, plus the machine-applicable fix when the
            // token is locatable on the call line.
            let mut cands: Vec<&str> = defined.iter().map(|s| s.as_str()).collect();
            cands.extend(module_genes.iter().map(|s| s.as_str()));
            cands.extend(crate::interp::BUILTIN_NAMES.iter().copied());
            let suggestions = crate::diag::did_you_mean(c, &cands);
            let fix = if *line > 0 {
                let line_text = src.lines().nth(line - 1).unwrap_or("");
                crate::diag::Span::of_token_word(*line, line_text, c).and_then(|sp| {
                    suggestions.first().map(|s| crate::diag::SuggestedFix {
                        line: sp.line,
                        column: sp.col,
                        length: sp.len,
                        replacement: s.clone(),
                        note: format!("replace '{}' with '{}'", c, s),
                    })
                })
            } else {
                None
            };
            rep.phantoms.push(PhantomCall {
                name: c.clone(),
                line: *line,
                suggestions,
                fix,
            });
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
                for cand in module_candidates(path, base_dir) {
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
        if defined.contains(name) && called.iter().any(|(n, _)| n == name) {
            let cs = crate::ffi::codon_score(name) as i64;
            bonus += (cs - 50) / 20; // 50-100 → 0..2 per hot gene
        }
    }
    rep.score += bonus.min(6);

    if nmd {
        let calledv: Vec<String> = called.iter().map(|(n, _)| n.clone()).collect();
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
        | Stmt::Scope(b)
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

fn collect_calls(prog: &Program, defined: &mut HashSet<String>, called: &mut Vec<(String, usize)>) {
    fn walk_expr(e: &Expr, called: &mut Vec<(String, usize)>) {
        match e {
            Expr::Call(f, args, line) => {
                if let Expr::Ident(n) = &**f {
                    // record EVERY named call with its line, the NMD
                    // untranslated detector needs the full transcription
                    // record, not just phantoms
                    called.push((n.clone(), *line));
                }
                walk_expr(f, called);
                for a in args {
                    walk_expr(a, called);
                }
            }
            Expr::Method(r, _, args, _) => {
                walk_expr(r, called);
                for a in args {
                    walk_expr(a, called);
                }
            }
            Expr::MethodSafe(r, _, args, _) => {
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
    fn walk_stmts(
        stmts: &[Stmt],
        defined: &mut HashSet<String>,
        called: &mut Vec<(String, usize)>,
    ) {
        for s in stmts {
            walk_stmt(s, defined, called);
        }
    }
    fn walk_stmt(s: &Stmt, defined: &mut HashSet<String>, called: &mut Vec<(String, usize)>) {
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
            | Stmt::LetAnn(_, _, e)
            | Stmt::Assign(_, _, e)
            | Stmt::ExprStmt(e)
            | Stmt::Return(Some(e))
            | Stmt::Raise(_, e, _) => walk_expr(e, called),
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
            Stmt::Scope(b) => walk_stmts(b, defined, called),
            Stmt::For(_, it, b) => {
                walk_expr(it, called);
                walk_stmts(b, defined, called);
            }
            Stmt::Match(sub, cases, _) => {
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
    // dx-r1 (audit W5): profiling must be ON before load, top-level
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

// ------------------------------------------------------------ chrome trace
/// W097-A: write the captured call spans as a Chrome Trace Format JSON
/// file (about://tracing / ui.perfetto.dev render `ph="X"` events
/// directly; W096's done-when names both). Returns (written, dropped)
/// for the operator notice.
///
/// Shape contract (pinned by tests/profile_spans.rs):
/// - `traceEvents`: two `ph="M"` metadata events (process/thread names),
///   then one `ph="X"` complete event per captured gene call, in
///   completion order: `{name, cat:"gene", ph:"X", pid:1, tid:1,
///   ts:<µs f64>, dur:<µs f64 inclusive>, args:{depth:<ancestors>}}`.
/// - `ts`/`dur` are microseconds on the same monotonic clock the
///   aggregate profiler uses (close_timing), so trace intervals and the
///   `--json` self-time table reconcile.
/// - ONE process, ONE thread: the interpreter is a single logical
///   timeline (fibers ride a deterministic virtual clock on that same
///   thread; attributing spans to fiber ids would claim a
///   thread-parallelism the scheduler does not have — W098's
///   WORKER-TELEMETRY.md owns worker attribution when it lands).
/// - `otherData`: self-describing metadata (format tag, version, source
///   file, unit, clock, total/dropped/cap) — the W096 law that a trace
///   must never be silent about its own limits.
pub fn write_chrome_trace(l: &Loaded, file: &str, path: &str) -> (usize, u64) {
    let mut events: Vec<String> = Vec::with_capacity(l.interp.span_log.len() + 2);
    events.push(
        "{\"name\":\"process_name\",\"ph\":\"M\",\"pid\":1,\"tid\":1,\"args\":{\"name\":\"operon\"}}"
            .to_string(),
    );
    events.push(
        "{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":1,\"args\":{\"name\":\"fibers (virtual clock)\"}}"
            .to_string(),
    );
    for s in &l.interp.span_log {
        events.push(format!(
            "{{\"name\":\"{}\",\"cat\":\"gene\",\"ph\":\"X\",\"pid\":1,\"tid\":1,\"ts\":{:.3},\"dur\":{:.3},\"args\":{{\"depth\":{}}}}}",
            json_escape(&s.name),
            s.start_us,
            s.dur_us,
            s.depth
        ));
    }
    let out = format!(
        "{{\"traceEvents\":[{}],\"displayTimeUnit\":\"us\",\"otherData\":{{\"format\":\"operon-chrome-trace\",\"version\":\"{}\",\"file\":\"{}\",\"unit\":\"microseconds\",\"clock\":\"monotonic\",\"total_spans\":{},\"dropped_spans\":{},\"span_cap\":{}}}}}",
        events.join(","),
        env!("CARGO_PKG_VERSION"),
        json_escape(file),
        l.interp.span_log.len(),
        l.interp.spans_dropped,
        crate::interp::SPAN_CAP,
    );
    std::fs::write(path, out)
        .unwrap_or_else(|e| crate::die(&format!("cannot write '{}': {}", path, e)));
    (l.interp.span_log.len(), l.interp.spans_dropped)
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
            // W06 (D-014): propagation abandoning a proof frame is a failure
            // (the frame did not complete), rendered with the variant repr,
            // never a leaked "propagate" kind.
            Err(s) if s.prop.is_some() => rep.failures.push(format!(
                "proof #{} failed: propagation left the proof frame ({})",
                i + 1,
                s.prop.unwrap().repr()
            )),
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
                // a proof frame must run to completion, early return/break is
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
                // W06 (D-014): propagation abandoning a proof frame is a
                // failure (the frame did not complete), never a leaked kind.
                Err(s) if s.prop.is_some() => {
                    rep.failed += 1;
                    file_failed = true;
                    proof_failures.push(format!(
                        "{} proof #{}: propagation left the proof frame ({})",
                        f,
                        i + 1,
                        s.prop.unwrap().repr()
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
            "operon test, {} file(s), {} proof(s): {} passed, {} failed ({} assertion(s) exercised)",
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

/// W49 (ROADMAP-100): expand test paths into a sorted explicit file list,
/// lets the CLI implement `--list` and `--filter` without re-running the
/// discovery logic differently from the real runner.
pub fn collect_test_files(paths: &[String]) -> Vec<String> {
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
    files.dedup();
    files
}

/// W49: the `--filter` selection rule, discovery plus a substring match on
/// the file path. Proof frames are anonymous in the grammar (`frame proof {`
/// carries no name), so the file is the selectable test unit; `--list`
/// shows exactly what was selected. `filter = None` is byte-identical to
/// bare discovery, the historical no-flags behavior.
pub fn select_test_files(paths: &[String], filter: Option<&str>) -> Vec<String> {
    let mut files = collect_test_files(paths);
    if let Some(f) = filter {
        files.retain(|p| p.contains(f));
    }
    files
}

/// W49: how many proof frames does this file declare? (cheap text scan used
/// by `operon test --list`; the runner remains the authority for pass/fail).
pub fn count_proof_frames(path: &str) -> usize {
    match std::fs::read_to_string(path) {
        Ok(s) => s.matches("frame proof").count(),
        Err(_) => 0,
    }
}

fn collect_op_files(dir: &Path, out: &mut Vec<String>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                // the red-team suite is adversarial by design (hangs, bombs,
                // escapes), it is exercised by scripts/redteam.sh with
                // containment expectations, never by the proof runner
                if p.file_name().map(|n| n == "redteam").unwrap_or(false) {
                    continue;
                }
                // substrate-r1: tests/granted/ holds capability-granted
                // proofs, meaningless (would all deny) under the default
                // runner's zero-grant sandbox. Exercised explicitly by
                // scripts/test.sh with an operator cell file.
                if p.file_name().map(|n| n == "granted").unwrap_or(false) {
                    continue;
                }
                // W18: cancellation timing proofs need real OS threads and
                // mid-flight cancel ordering the sequential oracle cannot
                // observe. Exercised explicitly (Rust side) by
                // scripts/test.sh; the oracle walker skips it too.
                if p.file_name().map(|n| n == "timing").unwrap_or(false) {
                    continue;
                }
                // W08r: the async corpus is cell-gated (io.pool = "fiber",
                // the deterministic virtual clock). Without the cell the
                // thread lane charges parked workers per real millisecond,
                // so fuel burn depends on the RUNNER'S SPEED — CI's slow
                // shared runners burned the pool on identical files that
                // pass locally. Exercised explicitly by scripts/test.sh
                // with tests/async/async.cell; lane parity is pinned by
                // tests/async_parity.rs (byte-identical two-lane programs).
                if p.file_name().map(|n| n == "async").unwrap_or(false) {
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

/// W47 (ROADMAP-100): formatter configuration.
///
/// `indent`, spaces per nesting level. The default is 2 because that is the
/// de-facto house style of every checked-in .op file (std/, tests/, examples/);
/// a formatter whose default reformats the whole corpus is a broken default.
///
/// `quotes`, how plain string literals re-emit. `Double` is the canonical
/// form (SPEC §3). `Single` re-emits `'…'` only when it is byte-lossless
/// (content has no `'`, no backslash, no brace, no newline/tab, i.e. both
/// spellings denote the same value with zero escaping); anything else falls
/// back to double. Quote-style `preserve` is impossible BY DESIGN: the AST
/// stores the string's VALUE, not which quote character the source used, so
/// there is nothing to preserve.
///
/// Scope notes (honesty): keyword canonicalization is inherent to fmt, the
/// parser repairs synonym spellings into the canonical AST, and fmt prints
/// the AST, so `--canonical` would be a no-op flag and is deliberately not
/// shipped. Soft `--width` wrapping is deferred (W47-v2): it changes token
/// layout and must not ship before the byte-stability law is proven over it.
///
/// Threading: fmt is a single-threaded-per-call operation (CLI exits after;
/// the LSP serves requests sequentially). The active config is a thread-local
/// installed by `format_program_with` and restored by a Drop guard, which
/// keeps every `fmt_*` signature unchanged (rustfmt makes the same trade).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QuoteMode {
    Double,
    Single,
}

#[derive(Clone, Copy, Debug)]
pub struct FmtConfig {
    pub indent: usize,
    pub quotes: QuoteMode,
    /// W47-v2: soft line-width limit (`None` = off, the historical behavior,
    /// and what LSP formatting uses so editors keep their own wrap policy).
    /// When set, a post-print pass breaks lines longer than `width` at
    /// parser-proven-safe comma points only (see `wrap_width`).
    pub width: Option<usize>,
}

impl Default for FmtConfig {
    fn default() -> Self {
        FmtConfig {
            indent: 2,
            quotes: QuoteMode::Double,
            width: None,
        }
    }
}

thread_local! {
    static FMT_CTX: std::cell::RefCell<FmtConfig> = std::cell::RefCell::new(FmtConfig::default());
}

/// Restores the default formatter config on scope exit, even on panic.
struct FmtGuard;
impl Drop for FmtGuard {
    fn drop(&mut self) {
        FMT_CTX.with(|c| *c.borrow_mut() = FmtConfig::default());
    }
}

pub fn format_program(prog: &Program) -> String {
    format_program_with(prog, &FmtConfig::default())
}

pub fn format_program_with(prog: &Program, cfg: &FmtConfig) -> String {
    FMT_CTX.with(|c| *c.borrow_mut() = *cfg);
    let _guard = FmtGuard;
    let mut out = String::new();
    // W074: module doc prints at the top, followed by a blank line.
    for l in &prog.module_doc {
        out.push_str("## ");
        out.push_str(l);
        out.push('\n');
    }
    if !prog.module_doc.is_empty() {
        out.push('\n');
    }
    for s in &prog.stmts {
        fmt_stmt(s, 0, &mut out);
    }
    // W47-v2: the width pass runs AFTER the canonical render, so it is a pure
    // function of (canonical text, width, indent). Idempotence survives by
    // construction: fmt re-parses the wrapped text to the SAME AST, re-renders
    // the SAME canonical text, and re-wraps it identically.
    match cfg.width {
        Some(w) if w >= 1 => wrap_width(&out, w, cfg.indent),
        _ => out,
    }
}

fn active_indent() -> usize {
    FMT_CTX.with(|c| c.borrow().indent)
}

fn active_quotes() -> QuoteMode {
    FMT_CTX.with(|c| c.borrow().quotes)
}

fn indent(n: usize) -> String {
    " ".repeat(n * active_indent())
}

/// W47: re-emit a plain string literal under the active quote mode.
/// `Single` is taken only when BOTH spellings denote the identical value
/// with no escaping at all; the lexer repairs `'` to `"` with a note, so
/// the single-quoted output is legal (if slightly noisy) on re-parse.
fn str_lit(s: &str) -> String {
    match active_quotes() {
        QuoteMode::Double => format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
                .replace('{', "\\{")
                .replace('}', "\\}")
        ),
        QuoteMode::Single
            if !s.contains('\'')
                && !s.contains('\\')
                && !s.contains('{')
                && !s.contains('}')
                && !s.contains('\n')
                && !s.contains('\t') =>
        {
            format!("'{}'", s)
        }
        QuoteMode::Single => format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
                .replace('{', "\\{")
                .replace('}', "\\}")
        ),
    }
}

/// W47: parse a minimal zero-dependency formatter config (`.operon-fmt.toml`
/// or the `[fmt]` section of `operon.toml`). Only `indent` (positive
/// integer), `quotes` (`double`|`single`) and `width` (0..=10000, 0 = off)
/// are meaningful; section headers and comments are ignored; unknown keys
/// and out-of-range values are reported (not errors, Total Grammar spirit,
/// forward-compatible) so the caller can surface them on stderr. Malformed
/// input never panics: the offending key keeps its current (base) value.
pub fn parse_fmt_config(src: &str) -> (FmtConfig, Vec<String>) {
    parse_fmt_config_from(FmtConfig::default(), src)
}

/// W47: layered variant. Applies `src` keys ON TOP of `base`, so the CLI can
/// stack defaults <- `operon.toml` [fmt] <- `.operon-fmt.toml` <- flags with
/// one parser and one note format.
pub fn parse_fmt_config_from(base: FmtConfig, src: &str) -> (FmtConfig, Vec<String>) {
    let mut cfg = base;
    let mut unknown: Vec<String> = Vec::new();
    for raw in src.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let (k, v) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim().trim_matches('"').trim_matches('\'')),
            None => continue,
        };
        match k {
            "indent" => match v.parse::<usize>() {
                Ok(n) if (1..=16).contains(&n) => cfg.indent = n,
                _ => unknown.push(format!("indent = {v} (want 1..=16)")),
            },
            "quotes" => match v {
                "double" => cfg.quotes = QuoteMode::Double,
                "single" => cfg.quotes = QuoteMode::Single,
                other => unknown.push(format!("quotes = {other} (want double|single)")),
            },
            // W47-v2: `width = 0` means explicitly off; absent means off.
            "width" => match v.parse::<usize>() {
                Ok(0) => cfg.width = None,
                Ok(n) if n <= 10_000 => cfg.width = Some(n),
                _ => unknown.push(format!("width = {v} (want 0..=10000, 0 = off)")),
            },
            other => unknown.push(other.to_string()),
        }
    }
    (cfg, unknown)
}

/// W47: extract the body lines of one `[section]` from a minimal TOML
/// document (the `operon.toml` project manifest, W19's hand-written dialect).
/// pkg.rs's `parse_manifest` is deliberately NOT reused: it REJECTS unknown
/// keys with a hard error, while fmt config must fall back to defaults with
/// a note, never error. Returns the section body (header excluded, key lines
/// verbatim so `parse_fmt_config_from` stays the single key parser) plus
/// notes for malformed headers. A missing section returns an empty body and
/// no notes, so absent config = today's exact output, byte-identical.
pub fn extract_toml_section(src: &str, section: &str) -> (String, Vec<String>) {
    let mut body = String::new();
    let mut notes: Vec<String> = Vec::new();
    let mut inside = false;
    for raw in src.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            // any header (well-formed or not) ends the current section
            inside = false;
            if line.ends_with(']') && line.len() >= 3 {
                let name = line[1..line.len() - 1].trim();
                inside = name == section;
            } else {
                notes.push(format!("malformed section header, ignored: {line}"));
            }
            continue;
        }
        if inside && !line.is_empty() {
            body.push_str(line);
            body.push('\n');
        }
    }
    (body, notes)
}

// ---------------------------------------------------------------------------
// W47-v2: the `--width` post-print line wrapper.
//
// DESIGN CONTRACT (why this is safe without touching the parser):
//
// The parser already tolerates newlines inside bracketed GROUPS wherever a
// group's element loop calls `eat_newlines_inline()` at its top, call args
// (bare calls, method calls, ?. calls), gene/sequence parameter lists (with
// annotations and defaults), and list literals. A newline after a comma in
// those positions is pure whitespace: the AST cannot change.
//
// The wrapper therefore breaks ONLY at commas whose enclosing bracket stack
// consists entirely of `(` and `[`. Everything else is out of scope by law:
//   - `{` groups are never broken (a text pass cannot tell a block brace
//     from a map-literal brace; both are legal in fmt output);
//   - string literals (including their interpolation regions) are opaque,
//     a comma inside a string is never a break point;
//   - openers never break (the first element stays on the opener line);
//   - closers stay glued to the last element (no dedicated closer line).
// When a line has no breakable comma it stays long, honestly, visibly.
//
// Determinism: wrap_width is a pure function of (text, width, indent_unit).
// It runs AFTER the canonical render, so fmt∘fmt re-renders the same
// canonical text and re-wraps it identically, the byte-stability law holds
// by construction, and tests/fmt_width.rs proves it corpus-wide together
// with the two stronger laws: AST identity and zero re-parse notes.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct WidthBreak {
    /// byte offset just AFTER the comma (where the line may split)
    pos: usize,
    /// number of enclosing brackets at the comma (>=1: never at depth 0)
    depth: usize,
}

/// Scan one rendered line for parser-safe break points.
/// `fmt` output is canonical: only double-quoted strings with `\` escapes,
/// no comments, no tabs. Blocks/maps contribute `{`/`}` which poison the
/// bracket stack.
///
/// Strings are modeled as a FRAME STACK, because interpolation carries REAL
/// code, including nested strings, inside the quotes:
/// `"{cv.csv_escape(tricky, ",")}"`. A naive in-string flag closes the
/// string at the nested `"` and the comma inside `","` looks like code.
/// Here a `"` in code PUSHES a Str frame; a `"` in a Str frame pops it; a
/// `{` in a Str frame pushes a Code frame (the interpolation region); a `}`
/// popping that frame returns to the enclosing string. A comma is a
/// candidate only when NO Str frame is open (the frame stack is just the
/// bottom Code frame), a newline anywhere inside a string would change the
/// string's VALUE, so string regions are opaque end to end.
fn scan_width_breaks(line: &str) -> Vec<WidthBreak> {
    // Code(Some(base)) = an interpolation region inside a string; `base` is
    // the bracket-stack depth at its opening `{`, so a `}` closing a MAP
    // LITERAL inside the interpolation is distinguishable from the `}` that
    // closes the interpolation itself. Code(None) = real line-level code.
    #[derive(Clone, Copy, PartialEq)]
    enum Frame {
        Code(Option<usize>),
        Str,
        Esc, // inside a `\x` escape pair within a Str frame
    }
    let mut out = Vec::new();
    let mut frames: Vec<Frame> = vec![Frame::Code(None)];
    // bracket stack: true = break-friendly (`(`/`[`), false = poisoned (`{`)
    let mut stack: Vec<bool> = Vec::new();
    for (i, ch) in line.char_indices() {
        let top = *frames.last().expect("frame stack never empty");
        match top {
            Frame::Esc => {
                frames.pop(); // the escaped char itself is opaque
            }
            Frame::Str => match ch {
                '\\' => frames.push(Frame::Esc),
                '"' => {
                    frames.pop(); // string ends
                }
                '{' => frames.push(Frame::Code(Some(stack.len()))), // interp opens
                _ => {}                                             // string content: opaque
            },
            Frame::Code(base) => match ch {
                '"' => frames.push(Frame::Str),
                // '#'-comment runs to end of line: nothing after it is code,
                // so no break may exist past it. A wrapped comment loses its
                // '##' prefix on continuation and re-parses as CODE (the
                // std/binary.op corpus failure: "..., say so with a value"
                // became a say() call). Breaks found before the comment stay
                // valid; saturate and stop.
                '#' => break,
                '(' | '[' => stack.push(true),
                '{' => stack.push(false),
                '}' => match base {
                    // interp region closes only when its own `{`-depth is
                    // back; deeper `}`s belong to code braces (maps, blocks)
                    Some(b) if stack.len() == b => {
                        frames.pop();
                    }
                    _ => {
                        stack.pop(); // saturate on unbalanced lines: never panic
                    }
                },
                ')' | ']' => {
                    stack.pop();
                }
                // breakable ONLY at real code depth: inside a (/[
                // group, never at depth 0, never inside ANY string
                // frame (including interpolation code, where a newline
                // would land in the string's VALUE).
                ',' if base.is_none() && !stack.is_empty() && stack.iter().all(|&f| f) => {
                    out.push(WidthBreak {
                        pos: i + ch.len_utf8(),
                        depth: stack.len(),
                    });
                }
                _ => {}
            },
        }
    }
    out
}

fn wrap_width(text: &str, width: usize, indent_unit: usize) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    // split_inclusive keeps every existing '\n' byte-exact (blank lines,
    // the double blank between top-level definitions, nothing moves).
    for line in text.split_inclusive('\n') {
        let (content, nl) = match line.strip_suffix('\n') {
            Some(c) => (c, "\n"),
            None => (line, ""),
        };
        let segs = wrap_line(content, width, indent_unit);
        for (k, seg) in segs.iter().enumerate() {
            if k > 0 {
                out.push('\n'); // between wrapped segments of one source line
            }
            out.push_str(seg);
        }
        out.push_str(nl); // the source line's newline rides its last segment
    }
    out
}

fn wrap_line(line: &str, width: usize, indent_unit: usize) -> Vec<String> {
    if line.chars().count() <= width || indent_unit == 0 {
        return vec![line.to_string()];
    }
    let breaks = scan_width_breaks(line);
    if breaks.is_empty() {
        return vec![line.to_string()];
    }
    let dmin = breaks.iter().map(|b| b.depth).min().unwrap_or(1);
    let chosen: Vec<WidthBreak> = breaks.iter().filter(|b| b.depth == dmin).copied().collect();
    let base = line.len() - line.trim_start_matches(' ').len();
    let cont_indent = base + dmin * indent_unit;
    let mut segments: Vec<String> = Vec::new();
    let mut prev = 0usize;
    for b in &chosen {
        segments.push(line[prev..b.pos].to_string());
        prev = b.pos;
    }
    segments.push(line[prev..].to_string()); // tail keeps the closer glued
    let mut res: Vec<String> = Vec::with_capacity(segments.len());
    for (k, seg) in segments.into_iter().enumerate() {
        if k == 0 {
            res.push(seg);
        } else {
            let body = seg.trim_start();
            if body.is_empty() {
                return vec![line.to_string()]; // degenerate: refuse to wrap
            }
            res.push(format!("{}{}", " ".repeat(cont_indent), body));
        }
    }
    // recursion: continuation segments may still overflow via deeper groups.
    // Each level strictly increases the break depth, so this terminates.
    let mut final_res = Vec::with_capacity(res.len());
    for seg in &res {
        if seg.chars().count() > width {
            let sub = wrap_line(seg, width, indent_unit);
            if sub.len() == 1 {
                final_res.push(seg.clone());
            } else {
                final_res.extend(sub);
            }
        } else {
            final_res.push(seg.clone());
        }
    }
    final_res
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
    // W074: doc comments print above their declaration (metadata roundtrip).
    let doc: &[String] = match s {
        Stmt::Gene(g) | Stmt::Seq(g) => &g.doc,
        Stmt::Splice(sp) => &sp.doc,
        Stmt::Pheno(p) => &p.doc,
        Stmt::Trait(t) => &t.doc,
        Stmt::Fate(f) => &f.doc,
        _ => &[],
    };
    for l in doc {
        out.push_str(&indent(ind));
        out.push_str("## ");
        out.push_str(l);
        out.push('\n');
    }
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
            // W04: implements clause round-trips canonically
            if !p.implements.is_empty() {
                out.push_str(&format!("implements {} ", p.implements.join(", ")));
            }
            out.push_str("{\n");
            for (fname, fexpr) in &p.fields {
                out.push_str(&indent(ind + 1));
                out.push_str(&format!("let {} = {}\n", fname, fmt_expr(fexpr)));
            }
            for g in &p.methods {
                for l in &g.doc {
                    out.push_str(&indent(ind + 1));
                    out.push_str("## ");
                    out.push_str(l);
                    out.push('\n');
                }
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
        // W04: trait declaration round-trip
        Stmt::Trait(t) => {
            out.push_str(&indent(ind));
            out.push_str(&format!("trait {} {{\n", t.name));
            for m in &t.methods {
                out.push_str(&indent(ind + 1));
                match &m.default {
                    Some(g) => {
                        out.push_str(&format!(
                            "gene {}({}) ",
                            g.name.clone().unwrap_or_default(),
                            fmt_params(&g.params)
                        ));
                        fmt_block(&g.body, ind + 1, out);
                        out.push('\n');
                    }
                    None => {
                        out.push_str(&format!("gene {}();\n", m.name));
                    }
                }
            }
            out.push_str(&indent(ind));
            out.push_str("}\n\n");
        }
        Stmt::Let(n, e) => {
            out.push_str(&format!("let {} = {}\n", n, fmt_expr(e)));
        }
        // W05: const roundtrip (immutable binding, deep-freeze semantics)
        Stmt::LetConst(n, e) => {
            out.push_str(&format!("const {} = {}\n", n, fmt_expr(e)));
        }
        // W01 (L2c): annotated definition roundtrip
        Stmt::LetAnn(n, ann, e) => {
            out.push_str(&format!("let {}: {} = {}\n", n, ann.render(), fmt_expr(e)));
        }
        // W01-s2: type alias roundtrip (parse-time metadata, inert at runtime)
        Stmt::TypeAlias(n, target, _) => {
            out.push_str(&format!("type {} = {}\n", n, target.render()));
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
        Stmt::Scope(b) => {
            out.push_str("scope ");
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
        Stmt::Match(sub, cases, _) => {
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
        Stmt::Raise(k, e, _) => match k {
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
            // W64: deprecation marks round-trip canonically (metadata,
            // never evaluated; SPEC §3 marks table)
            if let Some(d) = &g.deprecated {
                match &d.since {
                    Some(s) => out.push_str(&format!(
                        "@deprecated({}, since={}) ",
                        str_lit(&d.message),
                        str_lit(s)
                    )),
                    None => out.push_str(&format!("@deprecated({}) ", str_lit(&d.message))),
                }
            }
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
                Some(n) => out.push_str(&format!(
                    "gene {}({}){}",
                    n,
                    fmt_params_ann(&g.params, &g.param_anns),
                    match &g.ret_ann {
                        Some(a) => format!(" -> {}", a.render()),
                        None => String::new(),
                    }
                )),
                None => out.push_str(&format!(
                    "gene({}){}",
                    fmt_params_ann(&g.params, &g.param_anns),
                    match &g.ret_ann {
                        Some(a) => format!(" -> {}", a.render()),
                        None => String::new(),
                    }
                )),
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
        Stmt::Silence(f, t, s, sites) => {
            let suffix = if *s < 1.0 || *sites > 1 {
                format!(
                    " strength {} sites {}",
                    crate::value::format_float(*s),
                    *sites
                )
            } else {
                String::new()
            };
            match t {
                Some(t) => out.push_str(&format!("silence {} -> {}{}\n", f, t, suffix)),
                None => out.push_str(&format!("silence {}{}\n", f, suffix)),
            }
        }
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
        Stmt::Regulate(edges, trans, binds) => {
            out.push_str("regulate {\n");
            for e in edges {
                out.push_str(&indent(ind + 1));
                // reg-bio-2: canonical round-trip of the edge keywords
                // (order: strength -> threshold -> hill -> any -> occupy -> sum)
                let mut line = format!(
                    "{} {} {}{}",
                    e.from,
                    // reg-bio-2 (A5): attenuator edges round-trip their verb
                    if e.attenuates {
                        "attenuates"
                    } else if e.inhibit {
                        "inhibits"
                    } else {
                        "activates"
                    },
                    e.to,
                    if e.strength != 1.0 {
                        format!(" strength {}", e.strength)
                    } else {
                        String::new()
                    }
                );
                if let Some(t) = e.threshold {
                    line.push_str(&format!(" threshold {}", t));
                }
                if let Some(h) = e.hill {
                    line.push_str(&format!(" hill {}", h));
                }
                if e.any {
                    line.push_str(" any");
                }
                if e.occupy {
                    line.push_str(" occupy");
                }
                if e.sum {
                    line.push_str(" sum");
                }
                line.push_str(";\n");
                out.push_str(&line);
            }
            // reg-bio-2 (C1): translation edges round-trip canonically
            for t in trans {
                out.push_str(&indent(ind + 1));
                out.push_str(&format!(
                    "{} translates {}{}{};\n",
                    t.from,
                    t.to,
                    if let Some(r) = t.rate {
                        format!(" rate {}", r)
                    } else {
                        String::new()
                    },
                    if let Some(d) = t.decay {
                        format!(" decay {}", d)
                    } else {
                        String::new()
                    }
                ));
            }
            // reg-bio-2 (A4): allosteric bindings round-trip canonically
            for b in binds {
                out.push_str(&indent(ind + 1));
                out.push_str(&format!(
                    "bind {} {} {} k {};\n",
                    b.tf,
                    if b.inducer { "inducer" } else { "cofactor" },
                    b.ligand,
                    b.k
                ));
            }
            out.push_str(&indent(ind));
            out.push_str("}\n\n");
        }
        Stmt::Toggle(a, b) => out.push_str(&format!("toggle {}, {};\n", a, b)),
        // reg-bio-3 (A1/A7): round-trip the polycistronic unit
        Stmt::Operon(name, members) => {
            out.push_str(&format!("operon {} {{\n", name));
            for (g, rbs) in members {
                if *rbs != 1.0 {
                    out.push_str(&format!(
                        "    {} rbs {};\n",
                        g,
                        crate::value::format_float(*rbs)
                    ));
                } else {
                    out.push_str(&format!("    {};\n", g));
                }
            }
            out.push_str("}\n\n");
        }
        // reg-bio-2 (C11): decoy round-trip (canonical form)
        Stmt::Decoy(d, tf, cap) => {
            out.push_str(&format!("decoy {} for {} capacity {};\n", d, tf, cap))
        }
        // reg-bio-2 (A4): ligand declaration round-trip
        Stmt::Ligand(name) => out.push_str(&format!("ligand {};\n", name)),
        Stmt::Autoinducer(name) => out.push_str(&format!("autoinducer {};\n", name)),
        Stmt::Repressilator(ring, period, ov) => {
            out.push_str(&format!("repressilator {}", ring.join(" -> ")));
            if let Some(p) = period {
                out.push_str(&format!(" period {}", p));
            }
            // reg-bio (F-5): round-trip the inline kinetics (canonical order)
            if let Some(a) = ov.alpha {
                out.push_str(&format!(" alpha {}", a));
            }
            if let Some(g) = ov.gamma {
                out.push_str(&format!(" gamma {}", g));
            }
            if let Some(h) = ov.hill {
                out.push_str(&format!(" hill {}", h));
            }
            if let Some(b) = ov.basal {
                out.push_str(&format!(" basal {}", b));
            }
            if let Some(n) = ov.noise {
                out.push_str(&format!(" noise {}", n));
            }
            if let Some(s) = ov.seed {
                out.push_str(&format!(" seed {}", s));
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
        Stmt::Module(n, body) => {
            // W025 stage 2: nested sub-module declarations round-trip
            // canonically beside tads.
            out.push_str(&format!("module {} ", n));
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
        // W02 (match-v2): roundtrip forms for the new patterns, fmt output
        // re-parses to the same AST (checked by the fmt roundtrip gate).
        MatchPat::Variant(tag, None) => tag.clone(),
        MatchPat::Variant(tag, Some(p)) => format!("{}({})", tag, fmt_pat(p)),
        MatchPat::ListPat { elems, rest } => {
            let mut parts: Vec<String> = elems.iter().map(fmt_pat).collect();
            if let Some(r) = rest {
                parts.push(format!("*{}", r));
            }
            format!("[{}]", parts.join(", "))
        }
        MatchPat::MapPat { keys } => {
            let parts: Vec<String> = keys
                .iter()
                .map(|(k, sub)| match sub {
                    None => k.clone(),
                    Some(p) => format!("{}: {}", k, fmt_pat(p)),
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        MatchPat::Or(alts) => alts.iter().map(fmt_pat).collect::<Vec<_>>().join(" | "),
        MatchPat::Guard(p, c) => format!("{} if {}", fmt_pat(p), fmt_expr(c)),
    }
}

fn fmt_params(ps: &[(String, Option<Expr>)]) -> String {
    fmt_params_ann(ps, &[])
}

/// W01 (L2c): params with annotations, roundtrip form `name: T = default`.
/// `anns` may be shorter than `ps` (unannotated definitions).
fn fmt_params_ann(ps: &[(String, Option<Expr>)], anns: &[Option<TypeAnn>]) -> String {
    ps.iter()
        .enumerate()
        .map(|(i, (n, d))| {
            let mut s = n.clone();
            if let Some(Some(a)) = anns.get(i) {
                s.push_str(": ");
                s.push_str(&a.render());
            }
            if let Some(e) = d {
                s.push_str(" = ");
                s.push_str(&fmt_expr(e));
            }
            s
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

/// A map key that parsed to Expr::Null (a non-name, non-string literal key
/// like `2.5:`; the parser notes it and keeps Null) prints in its WIRE
/// canonical form `"null"`, the string the runtime's key stringification
/// produces. Printing the bare word would REPARSE as the string key "null"
/// and make the canonical form unstable: fix_corpus law 1 caught exactly
/// that on the W015 channels differential (float key round trip).
fn fmt_map_key(k: &Expr) -> String {
    if matches!(k, Expr::Null) {
        str_lit("null") // quote-mode aware: single-quote configs stay stable
    } else {
        fmt_prec(k, 0)
    }
}

fn fmt_prec(e: &Expr, parent: u8) -> String {
    let needs_paren = nest_prec(e) < parent;
    let body = match e {
        // W08r: the position marker is metadata — fmt renders the wrapped
        // expression only, so output is byte-identical to pre-marker source
        Expr::At(inner, _) => fmt_prec(inner, parent),
        Expr::Null => "null".into(),
        Expr::Bool(true) => "true".into(),
        Expr::Bool(false) => "false".into(),
        Expr::Int(i) => i.to_string(),
        Expr::Float(f) => crate::value::format_float(*f),
        Expr::Str(s) => str_lit(s), // W47: quote mode aware (single only when byte-lossless)
        // W029: bytes literals round-trip byte-exactly, the same escape set
        // the lexer parses (short forms + \xNN), re-emitted deterministically.
        Expr::Bytes(b) => {
            let mut out = String::from("b\"");
            for &byte in b {
                match byte {
                    b'\n' => out.push_str("\\n"),
                    b'\t' => out.push_str("\\t"),
                    b'\r' => out.push_str("\\r"),
                    b'"' => out.push_str("\\\""),
                    b'\\' => out.push_str("\\\\"),
                    0x20..=0x7e => out.push(byte as char),
                    other => out.push_str(&format!("\\x{:02x}", other)),
                }
            }
            out.push('"');
            out
        }
        Expr::Interp(parts) => {
            let mut out = String::from("\"");
            for p in parts {
                match p {
                    InterpPart::Lit(s) => {
                        // literal segments may contain braces that came from
                        // \\{ \\} escapes, re-escape them or fmt corrupts the file
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
                .map(|(k, v)| format!("{}: {}", fmt_map_key(k), fmt_prec(v, 0)))
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
            // BinOp::Pow is NOT left-assoc in the grammar: parse_pow takes
            // its LEFT operand from parse_postfix (purer than unary) and its
            // RIGHT operand re-enters parse_unary (which is what makes it
            // right-associative). Rendering the left operand at the generic
            // left-assoc rule reparsed-broke two AST shapes into DIFFERENT
            // meanings: Pow(Neg(a), b) printed as `-a ** b` reparses as
            // Neg(Pow(a, b)) — `(-8) ** 0.5` (nan) silently became
            // `-8 ** 0.5` (-2.83…) — and Pow(Pow(a, b), c) printed as
            // `a ** b ** c` reparses as Pow(a, Pow(b, c)). fix_corpus law 1
            // caught both on tests/differential/numeric_abuse.op (the first
            // corpus program with a pow-over-neg spelling; W006-D). Pow's
            // left operand therefore renders at 11 (postfix-or-purer: bare
            // exactly when the grammar accepts it there); the right operand
            // keeps the generic tighter rule (parse_unary accepts Neg/Not).
            let (lp, rp) = if matches!(op, BinOp::Pow) {
                (11, p + 1)
            } else {
                (p, p + 1)
            };
            let left = fmt_prec(a, lp);
            let right = fmt_prec(b, rp);
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
        Expr::MethodSafe(t, m, args, _) => format!(
            "{}?.{}({})",
            fmt_expr(t),
            m,
            args.iter()
                .map(|x| fmt_prec(x, 0))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Method(t, m, args, _) => format!(
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
            // cond slot: a nested ternary MUST be parenthesized, `a ? 0 : 2
            // ? 3 : 4` re-parses as `a ? 0 : (2 ? 3 : 4)` and silently changes
            // meaning (wave-3 Critic-Q semantics bug)
            format!("{} ? {} : {}", fmt_prec(c, 2), fmt_expr(a), fmt_expr(b))
        }
        // W06 (D-014): postfix `?!` binds tighter than every binary/ternary,
        // the operand renders at max precedence and needs no parentheses.
        Expr::Propagate(e, _) => format!("{}?!", fmt_prec(e, 12)),
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

// ------------------------------------------------------------ ast dump
// W039 (ROADMAP-100): `operon ast`, the Total Grammar structural window.
//
// The dump is a hand-written compact walker, NOT the derived Debug: it must
// be deterministic, stable across rustc versions, and purely structural.
// Repair notes are W38 `explain`'s surface and never appear here; the dump
// works on ANY parse (Total Grammar: every file parses, possibly with
// repairs). One internal tree feeds two renderers, the indented text tree
// and the --json form, so the two views cannot drift apart.

/// One node of the dump tree. Leaf items carry scalar payloads, node items
/// carry subtrees. Builders keep leaves before child nodes so the text
/// renderer can inline the leaves on the opener line.
enum DumpNode {
    /// string payload, rendered quoted + escaped in both renderers
    Str(String),
    /// bare token (kind labels, numbers, operators), unquoted in the text tree
    Tok(String),
    /// a labelled subtree
    Node(String, Vec<DumpNode>),
}

fn ds(s: &str) -> DumpNode {
    DumpNode::Str(s.to_string())
}

fn dt<D: std::fmt::Display>(t: D) -> DumpNode {
    DumpNode::Tok(t.to_string())
}

/// f64 payload: Debug (not Display) so 1.0 never renders as "1" and floats
/// stay distinguishable from ints in the dump.
fn df(f: f64) -> DumpNode {
    DumpNode::Tok(format!("{f:?}"))
}

fn dn(kind: &str, items: Vec<DumpNode>) -> DumpNode {
    DumpNode::Node(kind.to_string(), items)
}

fn d_doc(doc: &[String]) -> Option<DumpNode> {
    if doc.is_empty() {
        None
    } else {
        Some(dn("Doc", doc.iter().map(|s| ds(s)).collect()))
    }
}

fn dunop(op: &UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "not",
        UnOp::BitNot => "~",
    }
}

/// compound assignment spelling (`+=`), source syntax, not bare operator
fn dassign_op(op: &BinOp) -> String {
    format!("{}=", fmt_op(*op))
}

fn d_args(args: &[Expr]) -> DumpNode {
    dn("Args", args.iter().map(d_expr).collect())
}

fn d_body(stmts: &[Stmt]) -> DumpNode {
    dn("Body", stmts.iter().map(d_stmt).collect())
}

fn d_ann(a: &TypeAnn) -> DumpNode {
    match a {
        TypeAnn::Named(n) => dn("Ann", vec![ds(n)]),
        TypeAnn::Union(alts) => dn("AnnUnion", alts.iter().map(d_ann).collect()),
        TypeAnn::Optional(inner) => dn("AnnOptional", vec![d_ann(inner)]),
        TypeAnn::Alias { name, target } => dn("AnnAlias", vec![ds(name), d_ann(target)]),
        TypeAnn::Generic(name, args) => dn(
            "AnnGeneric",
            std::iter::once(ds(name))
                .chain(args.iter().map(d_ann))
                .collect(),
        ),
    }
}

/// bytes payload as lowercase hex (b"\xff\x00" dumps as "ff00")
fn d_hex(bs: &[u8]) -> String {
    let mut out = String::with_capacity(bs.len() * 2);
    for b in bs {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn d_expr(e: &Expr) -> DumpNode {
    match e {
        // W08r: dump stays byte-identical with pre-marker output — the
        // marker is transparent metadata, not program structure
        Expr::At(inner, _) => d_expr(inner),
        Expr::Null => dn("Null", vec![]),
        Expr::Bool(b) => dn("Bool", vec![dt(*b)]),
        Expr::Int(n) => dn("Int", vec![dt(n)]),
        Expr::Float(f) => dn("Float", vec![df(*f)]),
        Expr::Str(s) => dn("Str", vec![ds(s)]),
        Expr::Bytes(bs) => dn("Bytes", vec![ds(&d_hex(bs))]),
        Expr::Interp(parts) => dn(
            "Interp",
            parts
                .iter()
                .map(|p| match p {
                    InterpPart::Lit(s) => dn("Lit", vec![ds(s)]),
                    InterpPart::Expr(x) => dn("Expr", vec![d_expr(x)]),
                })
                .collect(),
        ),
        Expr::List(items) => dn("List", items.iter().map(d_expr).collect()),
        Expr::Map(pairs) => dn(
            "Map",
            pairs
                .iter()
                .map(|(k, v)| dn("Pair", vec![d_expr(k), d_expr(v)]))
                .collect(),
        ),
        Expr::Ident(s) => dn("Ident", vec![ds(s)]),
        Expr::Unary(op, x) => dn("Unary", vec![dt(dunop(op)), d_expr(x)]),
        // dx-r4/A13 source-line stamps are metadata, not structure: omitted
        Expr::Binary(op, l, r, _) => dn("Binary", vec![dt(fmt_op(*op)), d_expr(l), d_expr(r)]),
        Expr::Call(f, args, _) => dn("Call", vec![d_expr(f), d_args(args)]),
        Expr::Index(obj, idx, _) => dn("Index", vec![d_expr(obj), d_expr(idx)]),
        Expr::Member(obj, name) => dn("Member", vec![d_expr(obj), ds(name)]),
        Expr::MemberSafe(obj, name) => dn("MemberSafe", vec![d_expr(obj), ds(name)]),
        Expr::Method(obj, name, args, _) => dn("Method", vec![d_expr(obj), ds(name), d_args(args)]),
        Expr::MethodSafe(obj, name, args, _) => {
            dn("MethodSafe", vec![d_expr(obj), ds(name), d_args(args)])
        }
        Expr::Lambda(g) => d_gene("Lambda", g),
        Expr::Collect {
            var,
            iter,
            filter,
            body,
        } => {
            let mut items = vec![ds(var), d_expr(iter)];
            if let Some(f) = filter {
                items.push(dn("When", vec![d_expr(f)]));
            }
            items.push(dn("Do", vec![d_expr(body)]));
            dn("Collect", items)
        }
        Expr::FateNew(s) => dn("FateNew", vec![ds(s)]),
        Expr::New(name, args) => dn("New", vec![ds(name), d_args(args)]),
        Expr::Ternary(c, a, b) => dn("Ternary", vec![d_expr(c), d_expr(a), d_expr(b)]),
        Expr::Propagate(x, _) => dn("Propagate", vec![d_expr(x)]),
    }
}

fn d_pat(p: &MatchPat) -> DumpNode {
    match p {
        MatchPat::Lit(e) => dn("Lit", vec![d_expr(e)]),
        MatchPat::Multi(vs) => dn("Multi", vs.iter().map(d_expr).collect()),
        MatchPat::Bind(s) => dn("Bind", vec![ds(s)]),
        MatchPat::Wild => dn("Wild", vec![]),
        MatchPat::Variant(name, sub) => {
            let mut items = vec![ds(name)];
            if let Some(sp) = sub {
                items.push(d_pat(sp));
            }
            dn("Variant", items)
        }
        MatchPat::ListPat { elems, rest } => {
            let mut items: Vec<DumpNode> = elems.iter().map(d_pat).collect();
            if let Some(r) = rest {
                items.push(dn("Rest", vec![ds(r)]));
            }
            dn("ListPat", items)
        }
        MatchPat::MapPat { keys } => dn(
            "MapPat",
            keys.iter()
                .map(|(k, sub)| {
                    let mut ki = vec![ds(k)];
                    if let Some(sp) = sub {
                        ki.push(d_pat(sp));
                    }
                    dn("Key", ki)
                })
                .collect(),
        ),
        MatchPat::Or(alts) => dn("Or", alts.iter().map(d_pat).collect()),
        MatchPat::Guard(p, cond) => dn("Guard", vec![d_pat(p), d_expr(cond)]),
    }
}

fn d_destructure(p: &Pat) -> DumpNode {
    match p {
        Pat::Bind(s) => dn("Bind", vec![ds(s)]),
        Pat::List { elems, rest } => {
            let mut items: Vec<DumpNode> = elems.iter().map(d_destructure).collect();
            if let Some(r) = rest {
                items.push(dn("Rest", vec![ds(r)]));
            }
            dn("PatList", items)
        }
        Pat::Map { keys } => dn("PatMap", keys.iter().map(|k| ds(k)).collect()),
    }
}

/// gene/sequence/lambda/method definition; `kind` names the call site
fn d_gene(kind: &str, g: &GeneDef) -> DumpNode {
    let mut items: Vec<DumpNode> = Vec::new();
    if let Some(n) = &g.name {
        items.push(ds(n));
    }
    if let Some(d) = d_doc(&g.doc) {
        items.push(d);
    }
    let mut params: Vec<DumpNode> = Vec::new();
    for (i, (name, default)) in g.params.iter().enumerate() {
        let mut pi = vec![ds(name)];
        if let Some(Some(a)) = g.param_anns.get(i) {
            pi.push(d_ann(a));
        }
        if let Some(d) = default {
            pi.push(dn("Default", vec![d_expr(d)]));
        }
        params.push(dn("Param", pi));
    }
    items.push(dn("Params", params));
    let mut marks: Vec<DumpNode> = Vec::new();
    if g.acetylate {
        marks.push(dt("acetylate"));
    }
    if g.methylate {
        marks.push(dt("methylate"));
    }
    if g.m6a {
        marks.push(dt("m6a"));
    }
    if g.copies != 1 {
        marks.push(dt(format!("copies={}", g.copies)));
    }
    if !marks.is_empty() {
        items.push(dn("Marks", marks));
    }
    if let Some((lig, on, thr)) = &g.riboswitch {
        items.push(dn("Riboswitch", vec![ds(lig), dt(*on), df(*thr)]));
    }
    if let Some((kon, koff)) = &g.burst {
        items.push(dn("Burst", vec![df(*kon), df(*koff)]));
    }
    if let Some((cond, gb)) = &g.guard {
        items.push(dn("Guard", vec![d_expr(cond), d_body(gb)]));
    }
    if let Some(a) = &g.ret_ann {
        items.push(dn("Ret", vec![d_ann(a)]));
    }
    items.push(d_body(&g.body));
    dn(kind, items)
}

fn d_pheno(p: &PhenoDef) -> DumpNode {
    let mut items: Vec<DumpNode> = vec![ds(&p.name)];
    if let Some(d) = d_doc(&p.doc) {
        items.push(d);
    }
    if let Some(parent) = &p.parent {
        items.push(dn("From", vec![ds(parent)]));
    }
    if !p.implements.is_empty() {
        items.push(dn(
            "Implements",
            p.implements.iter().map(|i| ds(i)).collect(),
        ));
    }
    for (name, default) in &p.fields {
        items.push(dn("Field", vec![ds(name), d_expr(default)]));
    }
    for m in &p.methods {
        items.push(d_gene("Method", m));
    }
    dn("Pheno", items)
}

fn d_edge(e: &RegEdge) -> DumpNode {
    let mut items = vec![ds(&e.from), ds(&e.to), df(e.strength)];
    if e.inhibit {
        items.push(dt("inhibit"));
    }
    if let Some(t) = e.threshold {
        items.push(dt("threshold"));
        items.push(df(t));
    }
    if let Some(h) = e.hill {
        items.push(dt("hill"));
        items.push(dt(h));
    }
    if e.any {
        items.push(dt("any"));
    }
    if e.occupy {
        items.push(dt("occupy"));
    }
    if e.sum {
        items.push(dt("sum"));
    }
    if e.attenuates {
        items.push(dt("attenuates"));
    }
    dn("Edge", items)
}

fn d_overrides(ov: &RepressiOverrides) -> DumpNode {
    let mut items: Vec<DumpNode> = Vec::new();
    if let Some(v) = ov.alpha {
        items.push(dn("Alpha", vec![df(v)]));
    }
    if let Some(v) = ov.gamma {
        items.push(dn("Gamma", vec![df(v)]));
    }
    if let Some(v) = ov.hill {
        items.push(dn("Hill", vec![dt(v)]));
    }
    if let Some(v) = ov.basal {
        items.push(dn("Basal", vec![df(v)]));
    }
    if let Some(v) = ov.noise {
        items.push(dn("Noise", vec![df(v)]));
    }
    if let Some(v) = ov.seed {
        items.push(dn("Seed", vec![dt(v)]));
    }
    dn("Overrides", items)
}

fn d_stmt(s: &Stmt) -> DumpNode {
    match s {
        Stmt::Let(name, e) => dn("Let", vec![ds(name), d_expr(e)]),
        Stmt::LetConst(name, e) => dn("LetConst", vec![ds(name), d_expr(e)]),
        Stmt::LetAnn(name, ann, e) => dn("LetAnn", vec![ds(name), d_ann(ann), d_expr(e)]),
        Stmt::TypeAlias(name, target, _) => dn("TypeAlias", vec![ds(name), d_ann(target)]),
        Stmt::Assign(name, op, e) => {
            let mut items = vec![ds(name)];
            if let Some(op) = op {
                items.push(dt(dassign_op(op)));
            }
            items.push(d_expr(e));
            dn("Assign", items)
        }
        Stmt::IndexAssign(t, i, op, e) => {
            let mut items = vec![d_expr(t), d_expr(i)];
            if let Some(op) = op {
                items.push(dt(dassign_op(op)));
            }
            items.push(d_expr(e));
            dn("IndexAssign", items)
        }
        Stmt::MemberAssign(t, name, op, e) => {
            let mut items = vec![d_expr(t), ds(name)];
            if let Some(op) = op {
                items.push(dt(dassign_op(op)));
            }
            items.push(d_expr(e));
            dn("MemberAssign", items)
        }
        Stmt::LetPat(pat, e) => dn("LetPat", vec![d_destructure(pat), d_expr(e)]),
        Stmt::ForPat(pat, iter, body) => dn(
            "ForPat",
            vec![d_destructure(pat), d_expr(iter), d_body(body)],
        ),
        Stmt::MultiAssign(targets, values, define) => {
            let mut items = vec![dt(if *define { "define" } else { "assign" })];
            items.extend(targets.iter().map(d_expr));
            items.push(dn("Values", values.iter().map(d_expr).collect()));
            dn("MultiAssign", items)
        }
        Stmt::If(arms, els) => {
            let mut items: Vec<DumpNode> = arms
                .iter()
                .map(|(c, b)| dn("Arm", vec![d_expr(c), d_body(b)]))
                .collect();
            if let Some(b) = els {
                items.push(dn("Else", vec![d_body(b)]));
            }
            dn("If", items)
        }
        Stmt::While(c, b) => dn("While", vec![d_expr(c), d_body(b)]),
        Stmt::Loop(b) => dn("Loop", vec![d_body(b)]),
        Stmt::Scope(b) => dn("Scope", vec![d_body(b)]),
        Stmt::For(v, i, b) => dn("For", vec![ds(v), d_expr(i), d_body(b)]),
        Stmt::Return(e) => dn("Return", e.iter().map(d_expr).collect()),
        Stmt::Break => dn("Break", vec![]),
        Stmt::Continue => dn("Continue", vec![]),
        Stmt::ExprStmt(e) => dn("ExprStmt", vec![d_expr(e)]),
        Stmt::Match(e, arms, _) => {
            let mut items = vec![d_expr(e)];
            items.extend(
                arms.iter()
                    .map(|(p, b)| dn("Arm", vec![d_pat(p), d_body(b)])),
            );
            dn("Match", items)
        }
        Stmt::Use(path, alias) => {
            let mut items = vec![ds(path)];
            if let Some(a) = alias {
                items.push(ds(a));
            }
            dn("Use", items)
        }
        // W007 statement line is metadata, not structure: omitted
        Stmt::Raise(kind, msg, _) => {
            let mut items: Vec<DumpNode> = kind.iter().map(|k| ds(k)).collect();
            items.push(d_expr(msg));
            dn("Raise", items)
        }
        Stmt::Stress { kind, body, rescue } => {
            let mut items: Vec<DumpNode> = kind.iter().map(|k| ds(k)).collect();
            items.push(d_body(body));
            if let Some((k, rb)) = rescue {
                let mut ritems: Vec<DumpNode> = k.iter().map(|k| ds(k)).collect();
                ritems.push(d_body(rb));
                items.push(dn("Rescue", ritems));
            }
            dn("Stress", items)
        }
        Stmt::Gene(g) => d_gene("Gene", g),
        Stmt::Splice(sp) => dn(
            "Splice",
            vec![ds(&sp.root)]
                .into_iter()
                .chain(
                    sp.variants
                        .iter()
                        .map(|(n, g)| dn("Variant", vec![ds(n), d_gene("Gene", g)])),
                )
                .collect(),
        ),
        Stmt::Trait(t) => dn(
            "Trait",
            vec![ds(&t.name)]
                .into_iter()
                .chain(d_doc(&t.doc))
                .chain(t.methods.iter().map(|m| {
                    let mut mi = vec![ds(&m.name)];
                    if m.required {
                        mi.push(dt("required"));
                    }
                    if let Some(g) = &m.default {
                        mi.push(d_gene("Default", g));
                    }
                    dn("Method", mi)
                }))
                .collect(),
        ),
        Stmt::Silence(old, new, strength, sites) => {
            let mut items = vec![ds(old)];
            if let Some(n) = new {
                items.push(ds(n));
            }
            items.push(df(*strength));
            items.push(dt(sites));
            dn("Silence", items)
        }
        Stmt::Operon(name, members) => dn(
            "Operon",
            vec![ds(name)]
                .into_iter()
                .chain(
                    members
                        .iter()
                        .map(|(n, rbs)| dn("Cistron", vec![ds(n), df(*rbs)])),
                )
                .collect(),
        ),
        Stmt::Enhance(names) => dn("Enhance", names.iter().map(|n| ds(n)).collect()),
        Stmt::Ires(n) => dn("Ires", vec![ds(n)]),
        Stmt::Fate(f) => {
            let mut items = vec![ds(&f.name)];
            if let Some(e) = &f.enter {
                items.push(ds(e));
            }
            items.extend(
                f.states
                    .iter()
                    .map(|(s, tg)| dn("State", vec![ds(s), ds(&tg.join("|"))])),
            );
            dn("Fate", items)
        }
        Stmt::Regulate(edges, trans, binds) => {
            let mut items: Vec<DumpNode> = edges.iter().map(d_edge).collect();
            items.extend(trans.iter().map(|t| {
                let mut ti = vec![ds(&t.from), ds(&t.to)];
                if let Some(r) = t.rate {
                    ti.push(dt("rate"));
                    ti.push(df(r));
                }
                if let Some(d) = t.decay {
                    ti.push(dt("decay"));
                    ti.push(df(d));
                }
                dn("Trans", ti)
            }));
            items.extend(binds.iter().map(|b| {
                dn(
                    "Bind",
                    vec![
                        ds(&b.tf),
                        ds(&b.ligand),
                        dt(if b.inducer { "inducer" } else { "cofactor" }),
                        df(b.k),
                    ],
                )
            }));
            dn("Regulate", items)
        }
        Stmt::Ligand(n) => dn("Ligand", vec![ds(n)]),
        Stmt::Autoinducer(n) => dn("Autoinducer", vec![ds(n)]),
        Stmt::Toggle(a, b) => dn("Toggle", vec![ds(a), ds(b)]),
        Stmt::Decoy(d, tf, c) => dn("Decoy", vec![ds(d), ds(tf), df(*c)]),
        Stmt::Repressilator(ring, period, ov) => {
            let mut items: Vec<DumpNode> = ring.iter().map(|r| ds(r)).collect();
            if let Some(p) = period {
                items.push(dt("period"));
                items.push(df(*p));
            }
            items.push(d_overrides(ov));
            dn("Repressilator", items)
        }
        Stmt::Frame {
            name,
            is_proof,
            body,
        } => {
            let kind = if *is_proof { "Proof" } else { "Frame" };
            dn(kind, vec![ds(name), d_body(body)])
        }
        Stmt::Edit(target, reps) => dn(
            "Edit",
            vec![ds(target)]
                .into_iter()
                .chain(reps.iter().map(|(f, t)| dn("Rep", vec![ds(f), ds(t)])))
                .collect(),
        ),
        Stmt::AnchorExport(names) => dn("AnchorExport", names.iter().map(|n| ds(n)).collect()),
        Stmt::AnchorImport(names) => dn("AnchorImport", names.iter().map(|n| ds(n)).collect()),
        Stmt::Tad(name, body) => dn("Tad", vec![ds(name), d_body(body)]),
        // W025 stage 2: nested sub-module tables dump beside tads
        Stmt::Module(name, body) => dn("Module", vec![ds(name), d_body(body)]),
        Stmt::Block(body) => dn("Block", vec![d_body(body)]),
        Stmt::Seq(g) => d_gene("Seq", g),
        Stmt::Yield(e) => dn("Yield", e.iter().map(d_expr).collect()),
        Stmt::Pheno(p) => d_pheno(p),
    }
}

fn d_program(prog: &Program) -> DumpNode {
    dn("Program", prog.stmts.iter().map(d_stmt).collect())
}

/// W039: the `operon ast` text form. A deterministic, purely structural
/// tree: one `(Kind ...)` node per AST node, string leaves quoted and
/// escaped, child nodes indented two spaces per level. `pretty = false`
/// renders the same tree on one line; that is the deep-nesting fallback,
/// the pretty form is quadratic in source nesting depth (indent x depth).
pub fn ast_dump(prog: &Program, pretty: bool) -> String {
    sexpr(&d_program(prog), 0, pretty)
}

fn sexpr(n: &DumpNode, ind: usize, pretty: bool) -> String {
    match n {
        DumpNode::Str(s) => format!("\"{}\"", json_escape(s)),
        DumpNode::Tok(t) => t.clone(),
        DumpNode::Node(kind, items) => {
            let kids: Vec<&DumpNode> = items
                .iter()
                .filter(|i| matches!(i, DumpNode::Node(..)))
                .collect();
            let mut out = String::new();
            out.push('(');
            out.push_str(kind);
            if kids.is_empty() || !pretty {
                for i in items {
                    out.push(' ');
                    out.push_str(&sexpr(i, ind, pretty));
                }
                out.push(')');
            } else {
                // leaves stay on the opener line, child nodes hang below
                for i in items {
                    if matches!(i, DumpNode::Node(..)) {
                        continue;
                    }
                    out.push(' ');
                    out.push_str(&sexpr(i, ind, pretty));
                }
                for k in kids {
                    out.push('\n');
                    out.push_str(&"  ".repeat(ind + 1));
                    out.push_str(&sexpr(k, ind + 1, pretty));
                }
                out.push('\n');
                out.push_str(&"  ".repeat(ind));
                out.push(')');
            }
            out
        }
    }
}

/// W039: the same dump tree as JSON (what `operon ast --json` embeds in its
/// report object). One `["Kind", ...]` array per node, payloads and labels
/// as JSON strings, child nodes as nested arrays, so the --json form is the
/// text form's exact structure by construction.
pub fn ast_dump_json(prog: &Program) -> String {
    djson(&d_program(prog))
}

fn djson(n: &DumpNode) -> String {
    match n {
        DumpNode::Str(s) => format!("\"{}\"", json_escape(s)),
        DumpNode::Tok(t) => format!("\"{}\"", json_escape(t)),
        DumpNode::Node(kind, items) => {
            let mut out = String::new();
            out.push_str("[\"");
            out.push_str(&json_escape(kind));
            out.push('"');
            for i in items {
                out.push(',');
                out.push_str(&djson(i));
            }
            out.push(']');
            out
        }
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
        // W101 slice 7: the derived E2xxx code rides the tag when the note's
        // message maps to a catalog family (docs/diagnostics-inventory.md);
        // uncoded notes render exactly as before.
        let code = crate::diag::note_code(&n.message);
        let tag_s = if code.is_empty() {
            tag.to_string()
        } else {
            format!("{} {}", tag, code)
        };
        // A13 (dx-r2): real locations when the note carries a line
        if n.line > 0 {
            let _ = writeln!(w, "[{}] {}:{}: {}", tag_s, l.interp.file, n.line, n.message);
        } else {
            let _ = writeln!(w, "[{}] {}", tag_s, n.message);
        }
    }
}

// ---------------------------------------------------------------- fix (W65)

/// W65 (ROADMAP-100): what one `operon fix` pass changed.
#[derive(Debug, Clone, PartialEq)]
pub struct FixReport {
    /// synonym/wobble canonicalizations the parser+fmt pass surfaced
    /// (rung-2 and rung-3 repair notes on the migrated source).
    pub canonicalized: usize,
    /// `const` → `let` keyword migrations applied at the source level.
    /// RETIRED (W05 hotfix): `const` is a live immutable binding, so the
    /// rewrite would change program meaning. Field kept for --json shape
    /// stability; always 0.
    pub const_to_let: usize,
    /// `expr::field` → `expr.field` migrations applied at the source level
    /// (dx-r3: old tutorials taught the unsupported `::` spelling).
    pub s_dot: usize,
}

/// W65: the `operon fix` core, parse → migrate → canonicalize.
///
/// Pipeline: (1) source-level legacy-token migrations, string/comment-aware;
/// (2) parse with Total Grammar repairs; (3) emit the canonical formatter
/// output. The result re-parses without rung-2+ notes on canonically valid
/// input, and the canonical MEANING of the program never changes: fix is a
/// surface-syntax migrator, not a rewriter.
pub fn fix_source(src: &str) -> (String, FixReport) {
    let (migrated, const_n, sdot_n) = migrate_source(src);
    let prog = parser::parse(&migrated);
    let canonicalized = prog.notes.iter().filter(|n| n.rung >= 2).count();
    let out = format_program(&prog);
    (
        out,
        FixReport {
            canonicalized,
            const_to_let: const_n,
            s_dot: sdot_n,
        },
    )
}

/// W65: the source-level migration pass. Walks the whole source char-wise
/// (not line-wise) so triple-quoted, raw, and escaped strings stay intact;
/// edits only CODE regions: comments and every string form are copied
/// verbatim. `use` lines are copied verbatim too: since W025 the `::`
/// separator in a use path is CURRENT sugar (exact spelling of `/`), not the
/// legacy call syntax the `expr::field` migration exists to retire, so
/// rewriting it would churn the canonical form for nothing (fix_corpus
/// law 1 caught this on tests/differential/namespaces.op). Returns
/// (new_source, const_count, s_dot_count).
fn migrate_source(src: &str) -> (String, usize, usize) {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(src.len() + 16);
    let mut i = 0usize;
    let const_n = 0usize; // retired migration (W05 hotfix), kept for report shape, always 0
    let mut sdot_n = 0usize;
    // only whitespace since the last newline: a word here is a statement
    // keyword, which is how use lines are recognized
    let mut at_stmt_start = true;

    while i < n {
        let c = chars[i];
        // W25 follow-up (2026-09-27): `::` is LIVE exact sugar in use paths
        // (use std::bio — dev1's W25). A use line is copied VERBATIM: the
        // separators are free spelling variants there (W25 contract), so the
        // dx-r3 expr::field → expr.field repair below must never touch them
        // (fix_corpus law 1: fix never changes canonical meaning — the
        // formatter preserves the spelling the author chose). Expression
        // context keeps the dx-r3 repair (law 3 pins it).
        if c == 'u'
            && (i == 0 || out.ends_with('\n'))
            && i + 3 < n
            && chars[i + 1] == 's'
            && chars[i + 2] == 'e'
            && (chars[i + 3] == ' ' || chars[i + 3] == '\t')
        {
            while i < n && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        // comments: verbatim to end of line (the newline itself re-enters code)
        if c == '#' {
            while i < n && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        // raw strings r"...": the r must be a standalone token (start or
        // preceded by a non-id char) and followed directly by a quote
        if c == 'r'
            && i + 1 < n
            && chars[i + 1] == '"'
            && (i == 0 || !(chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_'))
        {
            out.push(c);
            out.push('"');
            i += 2;
            while i < n && chars[i] != '"' {
                if chars[i] == '\n' {
                    out.push('\n');
                } else {
                    out.push(chars[i]);
                }
                i += 1;
            }
            if i < n {
                out.push('"');
                i += 1;
            }
            continue;
        }
        // triple-quoted strings """...""": verbatim, may span lines
        if c == '"' && i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
            out.push_str("\"\"\"");
            i += 3;
            while i < n {
                if chars[i] == '"' && i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
                    out.push_str("\"\"\"");
                    i += 3;
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        // plain strings: verbatim, backslash escapes respected
        if c == '"' {
            out.push(c);
            i += 1;
            while i < n {
                if chars[i] == '\\' && i + 1 < n {
                    out.push(chars[i]);
                    out.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    out.push('"');
                    i += 1;
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        // identifiers: the only code region fix rewrites
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            out.push_str(&word);
            // a `use` line is path syntax, never a method call: copy the
            // whole line verbatim so use-path separators stay as written
            if at_stmt_start && word == "use" {
                while i < n && chars[i] != '\n' {
                    out.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            at_stmt_start = false;
            // migration 2: expr::field → expr.field (dx-r3 legacy spelling)
            if i + 1 < n && chars[i] == ':' && chars[i + 1] == ':' {
                let followed_by_id =
                    i + 2 < n && (chars[i + 2].is_ascii_alphanumeric() || chars[i + 2] == '_');
                if followed_by_id {
                    out.push('.');
                    i += 2;
                    sdot_n += 1;
                }
            }
            continue;
        }
        if c == '\n' {
            at_stmt_start = true;
        } else if c != ' ' && c != '\t' && c != '\r' {
            at_stmt_start = false;
        }
        out.push(c);
        i += 1;
    }
    (out, const_n, sdot_n)
}
// ------------------------------------------------------------ docgen (W073)
/// W073: `operon doc`, markdown API reference rendered from the AST.
/// Parse-only (like check/fmt): no run, no capabilities beyond reading the
/// input file. Docs come from W074 `##` comments; signatures reuse the
/// formatter's parameter renderer so doc output can never drift from fmt.
fn doc_gene_signature(kind: &str, g: &GeneDef) -> String {
    let name = g.name.clone().unwrap_or_default();
    let mut s = format!(
        "{} {}({})",
        kind,
        name,
        fmt_params_ann(&g.params, &g.param_anns)
    );
    if let Some(r) = &g.ret_ann {
        s.push_str(" -> ");
        s.push_str(&r.render());
    }
    let mut marks: Vec<&str> = Vec::new();
    if g.acetylate {
        marks.push("@acetylate");
    }
    if g.methylate {
        marks.push("@methylate");
    }
    if g.m6a {
        marks.push("@m6a");
    }
    if g.copies > 1 {
        marks.push("@copies");
    }
    if g.riboswitch.is_some() {
        marks.push("@riboswitch");
    }
    if g.burst.is_some() {
        marks.push("@burst");
    }
    if !marks.is_empty() {
        s.push(' ');
        s.push_str(&marks.join(" "));
    }
    s
}

fn doc_md_lines(doc: &[String], out: &mut String) {
    if doc.is_empty() {
        return;
    }
    for l in doc {
        out.push_str(l);
        out.push('\n');
    }
    out.push('\n');
}

/// Render one file's API as markdown. `path` is shown in the header only.
pub fn doc_markdown(path: &str, prog: &Program) -> String {
    let stem = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", stem));
    doc_md_lines(&prog.module_doc, &mut out);
    out.push_str(&format!(
        "> Generated by `operon doc {}`, parse-only API surface (signatures, marks, `##` docs). \
Behavioral docs live in SPEC.md; network/regulation graphs: `operon graph`.\n\n",
        path
    ));

    for s in &prog.stmts {
        match s {
            Stmt::Gene(g) | Stmt::Seq(g) => {
                if g.name.is_none() {
                    continue;
                }
                let kind = if matches!(s, Stmt::Seq(_)) {
                    "sequence"
                } else {
                    "gene"
                };
                out.push_str(&format!("## `{}`\n\n", doc_gene_signature(kind, g)));
                doc_md_lines(&g.doc, &mut out);
            }
            Stmt::Splice(sp) => {
                out.push_str(&format!("## `splice {}`\n\n", sp.root));
                doc_md_lines(&sp.doc, &mut out);
                for (vname, vgene) in &sp.variants {
                    out.push_str(&format!(
                        "- variant `{}`, `{}`\n",
                        vname,
                        doc_gene_signature("gene", vgene)
                    ));
                }
                out.push('\n');
            }
            Stmt::Pheno(p) => {
                match &p.parent {
                    Some(par) => {
                        out.push_str(&format!("## `phenotype {} from {}`\n\n", p.name, par))
                    }
                    None => out.push_str(&format!("## `phenotype {}`\n\n", p.name)),
                }
                doc_md_lines(&p.doc, &mut out);
                for (fname, fexpr) in &p.fields {
                    out.push_str(&format!("- field `let {} = {}`\n", fname, fmt_expr(fexpr)));
                }
                for m in &p.methods {
                    out.push_str(&format!("- method `{}`\n", doc_gene_signature("gene", m)));
                }
                out.push('\n');
            }
            Stmt::Fate(f) => {
                out.push_str(&format!("## `fate {}`\n\n", f.name));
                doc_md_lines(&f.doc, &mut out);
                for (state, targets) in &f.states {
                    out.push_str(&format!(
                        "- state `{}` → {}\n",
                        state,
                        if targets.is_empty() {
                            "terminal".to_string()
                        } else {
                            targets.join(", ")
                        }
                    ));
                }
                if let Some(e) = &f.enter {
                    out.push_str(&format!("- enters at `{}`\n", e));
                }
                out.push('\n');
            }
            _ => {}
        }
    }
    out
}

/// Render one file's API as the playground-facing JSON document.
pub fn doc_json(path: &str, prog: &Program) -> String {
    fn jstr(s: &str) -> String {
        format!("\"{}\"", json_escape(s))
    }
    fn jarr(items: &[String]) -> String {
        format!(
            "[{}]",
            items.iter().map(|i| jstr(i)).collect::<Vec<_>>().join(",")
        )
    }
    fn gene_marks(g: &GeneDef) -> Vec<String> {
        let mut marks: Vec<String> = Vec::new();
        if g.acetylate {
            marks.push("@acetylate".into());
        }
        if g.methylate {
            marks.push("@methylate".into());
        }
        if g.m6a {
            marks.push("@m6a".into());
        }
        if g.copies > 1 {
            marks.push("@copies".into());
        }
        if g.riboswitch.is_some() {
            marks.push("@riboswitch".into());
        }
        if g.burst.is_some() {
            marks.push("@burst".into());
        }
        marks
    }
    let stem = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    let mut out = String::new();
    out.push_str("{\"module\":");
    out.push_str(&jstr(&stem));
    out.push_str(",\"doc\":");
    out.push_str(&jarr(&prog.module_doc));
    out.push_str(",\"genes\":[");
    let mut items: Vec<String> = Vec::new();
    for s in &prog.stmts {
        if let Stmt::Gene(g) | Stmt::Seq(g) = s {
            if g.name.is_none() {
                continue;
            }
            let kind = if matches!(s, Stmt::Seq(_)) {
                "sequence"
            } else {
                "gene"
            };
            items.push(format!(
                "{{\"name\":{},\"signature\":{},\"kind\":{},\"doc\":{},\"marks\":{}}}",
                jstr(g.name.as_deref().unwrap_or("")),
                jstr(&doc_gene_signature(kind, g)),
                jstr(kind),
                jarr(&g.doc),
                jarr(&gene_marks(g))
            ));
        }
    }
    out.push_str(&items.join(","));
    out.push_str("]}");
    out
}
