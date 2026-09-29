//! lint.rs, W41/W42/W43/W48/W66 (ROADMAP-100): the shared static-analysis rule
//! engine. Two CLI surfaces read it with DIFFERENT rule streams (W048):
//! `operon check` runs the correctness stream (arity, const safety, plus the
//! check-side phantom/NMD/cell sweeps in tools.rs), `operon lint` runs the
//! style/quality stream. One engine, one finding type, two views. The
//! interpreter and the oracle are untouched, so runtime semantics and
//! differential parity cannot drift from anything in here.
//!
//! Philosophy (Total Grammar): lint findings are ADVISORY. Nothing here
//! rejects a program; `--strict` escalation is a CLI-side policy (W37
//! contract), never an interpreter behavior.
//!
//! ------------------------------------------------------------ W048 streams
//! `rule_stream()` partitions the rule set by OWNING COMMAND. The stable
//! code stays attached to the rule wherever it prints:
//!
//!   check (correctness)  E01 wrong-arity, W02 const-reassign,
//!                        W09 nmd:<kind>/anchor-import (tools.rs sweep),
//!                        W10 cell-unknown-key, W11 cell-type-mismatch,
//!                        W01 phantom-call (tools.rs sweep)
//!   lint (style/quality) N01 unused-gene, N02 unused-import,
//!                        W07 unused-binding, W08 dead-const,
//!                        W03 constant-condition, N03 infinite-loop-suspect,
//!                        W04 unreachable-code, W05 duplicate-match-arm,
//!                        W06 unreachable-match-arm, N12 shadowed-binding
//!   check --typed       T01 type-mismatch, T02 unknown-member,
//!                        T03 arg-type-mismatch, T04 unknown-type,
//!                        T05 non-exhaustive-match, T06 propagate-non-variant,
//!                        T07 trait-method-missing, T08 bound-violation,
//!                        T09 assign-type-change, T10 return-missing
//!                        (TYPED-MODE: the static type checker, src/typecheck.rs)
//!
//! N04 (`repair:r<n>`, parser repair notes) is lint-owned BY CONTRACT but
//! not emitted by the engine yet: the corpus contains files that exercise
//! Total Grammar recovery on purpose (examples/total_grammar.op and
//! friends), so surfacing repairs as default-on lint findings would fire
//! on intentional code. Parser repairs stay on their dedicated surfaces
//! (`operon explain`, check's repair section) until a lint opt-in lands.
//!
//! ------------------------------------------------------------ W041 codes
//! Stable diagnostic code scheme, mirrored in `operon check --help`. The
//! letter is the severity stream (E error, W warning, N note), the number is
//! the stable identity. NEVER renumber; new rules append. The rule name is
//! carried alongside every finding so comments like `// allow: unused-gene`
//! stay readable.
//!
//!   E00  unreadable-file        file could not be read (check-level)
//!   E01  wrong-arity            call/new/method arg count vs a known def
//!                                (W043: promoted to error per board)
//!   W01  phantom-call           callee undefined in file/modules/builtins
//!   W02  const-reassign         assignment to a const-bound name
//!   W03  constant-condition     if/while over a literal constant
//!   W04  unreachable-code       statement after return/raise
//!   W05  duplicate-match-arm    same literal twice in one match
//!   W06  unreachable-match-arm  arm after an unguarded catch-all or a
//!                                strictly broader earlier arm (W042/W002)
//!   W07  unused-binding         let never read (W042)
//!   W08  dead-const             const never referenced (W042)
//!   W09  nmd:<kind>/anchor      NMD + anchor-import sweep findings
//!   W10  cell-unknown-key       .cell key outside the schema (W66)
//!   W11  cell-type-mismatch     .cell key expects a number (W66)
//!   W12  deprecated-use         call to an @deprecated-marked gene (W64)
//!   N01  unused-gene            defined, never called (library surface?)
//!   N02  unused-import          module imported, never referenced
//!   N03  infinite-loop-suspect  while over an always-true literal
//!   N04  repair:r<n>            parser repair note, rung n (W037/W038)
//!   N12  shadowed-binding       re-let of a name in the same block (W048)
//!   N99  <unknown>              fallback for rules not in the table yet
//!
//! Location convention: findings carry a 1-based line when the AST span
//! knows it (call sites do, A13); line 0 means file-level (def-order hint
//! findings where the statement carries no span yet).
//!
//! Suppression (the allow mechanism): a source comment `// allow: rule1,
//! rule2` (or `# allow: ...`) on the finding's line — or on the line
//! directly above it — drops findings whose rule OR code matches; bare
//! `// allow:` suppresses everything on that line. Best effort by design:
//! def-hint findings (line approximated by statement order) may sit one or
//! two lines off, call-site findings (real spans) are exact.

use crate::ast::{Expr, MatchPat, Program, Stmt};
use crate::genes;
use std::collections::{HashMap, HashSet};

// ------------------------------------------------------------ model

/// W048: which CLI surface owns a rule. The partition is by COMMAND PURPOSE:
/// `check` answers "is this program correct", `lint` answers "is this code
/// clean". See the stream table in the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// correctness rules surfaced by `operon check`
    Check,
    /// style/quality rules surfaced by `operon lint`
    Lint,
}

/// W048: rule ownership. Rule names OR stable codes resolve; dynamic rules
/// match by prefix. Anything not named defaults to the lint stream (new
/// style rules are append-only; the correctness set is closed, reviewed).
pub fn rule_stream(rule: &str) -> Stream {
    match rule {
        "wrong-arity" | "E01" | "const-reassign" | "W02" | "phantom-call" | "W01"
        | "cell-unknown-key" | "W10" | "cell-type-mismatch" | "W11" | "deprecated-use" | "W12" => {
            Stream::Check
        }
        r if r.starts_with("nmd:") || r == "anchor-import" || r.starts_with("W09") => Stream::Check,
        _ => Stream::Lint,
    }
}

/// W048: keep only the findings owned by one stream.
pub fn filter_stream(findings: Vec<Finding>, s: Stream) -> Vec<Finding> {
    findings
        .into_iter()
        .filter(|f| rule_stream(&f.rule) == s)
        .collect()
}

/// W048: the `operon lint` surface of the engine, style/quality rules only.
pub fn lint_style(prog: &Program) -> Vec<Finding> {
    filter_stream(lint(prog), Stream::Lint)
}

/// W048: the `operon check` surface of the engine, correctness rules only
/// (the phantom/NMD/cell sweeps stay in tools.rs, they need module loading).
pub fn lint_correctness(prog: &Program) -> Vec<Finding> {
    filter_stream(lint(prog), Stream::Check)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sev {
    Style = 0,
    Warning = 1,
    Error = 2,
}

impl Sev {
    pub fn name(&self) -> &'static str {
        match self {
            Sev::Style => "style",
            Sev::Warning => "warning",
            Sev::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    /// W041: stable diagnostic code (see the scheme table in the module doc).
    pub code: &'static str,
    pub rule: String,
    pub sev: Sev,
    pub message: String,
}

impl Finding {
    fn new(line: usize, rule: &str, sev: Sev, message: String) -> Finding {
        Finding {
            line,
            code: rule_code(rule),
            rule: rule.to_string(),
            sev,
            message,
        }
    }
}

/// W041: rule name -> stable diagnostic code. Dynamic rule names
/// (`nmd:<kind>`, `repair:r<n>`) are matched by prefix; anything not in the
/// table lands on N99 rather than inventing a code ad hoc.
pub fn rule_code(rule: &str) -> &'static str {
    match rule {
        "wrong-arity" => "E01",
        "phantom-call" => "W01",
        "const-reassign" => "W02",
        "constant-condition" => "W03",
        "unreachable-code" => "W04",
        "duplicate-match-arm" => "W05",
        "unreachable-match-arm" => "W06",
        "unused-binding" => "W07",
        "dead-const" => "W08",
        "deprecated-use" => "W12",
        "cell-unknown-key" => "W10",
        "cell-type-mismatch" => "W11",
        "unused-gene" => "N01",
        "unused-import" => "N02",
        "infinite-loop-suspect" => "N03",
        "shadowed-binding" => "N12",
        r if r.starts_with("nmd:") || r == "anchor-import" => "W09",
        r if r.starts_with("repair:") => "N04",
        // TYPED-MODE: T-series, the static type checker's stream
        // (src/typecheck.rs, TYPED-MODE.md §10)
        "type-mismatch" => "T01",
        "unknown-member" => "T02",
        "arg-type-mismatch" => "T03",
        "unknown-type" => "T04",
        "non-exhaustive-match" => "T05",
        "propagate-non-variant" => "T06",
        "trait-method-missing" => "T07",
        "bound-violation" => "T08",
        "assign-type-change" => "T09",
        "return-missing" => "T10",
        _ => "N99",
    }
}

/// (min_args, max_args), max == usize::MAX means unbounded (defaults/variadic
/// shapes). Collected from gene/splice/sequence/phenotype-method definitions.
pub type Arity = (usize, usize);

// ------------------------------------------------------------ arity table
// W43: wrong-arity static detection. Conservative by design: a call site is
// only reported when the callee name is unambiguous (defined at top level,
// not shadowed by any binding we can see).

pub fn collect_arities(prog: &Program) -> HashMap<String, Arity> {
    let mut m: HashMap<String, Arity> = HashMap::new();
    for st in &prog.stmts {
        collect_stmt_arity(st, &mut m);
    }
    m
}

fn param_arity(params: &[(String, Option<Expr>)]) -> Arity {
    let required = params.iter().filter(|(_, d)| d.is_none()).count();
    (required, params.len())
}

fn collect_stmt_arity(st: &Stmt, m: &mut HashMap<String, Arity>) {
    match st {
        Stmt::Gene(g) | Stmt::Seq(g) => {
            if let Some(name) = &g.name {
                m.insert(name.clone(), param_arity(&g.params));
            }
        }
        Stmt::Splice(s) => {
            // splice defs rebind a root gene; conservative: record 0..MAX so
            // splice call sites are never arity-flagged
            m.entry(s.root.clone()).or_insert((0, usize::MAX));
        }
        Stmt::Tad(name, stmts) => {
            for s in stmts {
                if let Stmt::Gene(g) = s {
                    if let Some(gn) = &g.name {
                        m.insert(format!("{}.{}", name, gn), param_arity(&g.params));
                    }
                }
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------ lint pass

pub fn lint(prog: &Program) -> Vec<Finding> {
    let mut out = Vec::new();
    let arities = collect_arities(prog);

    // defined genes + called names (for unused-gene / unused-import)
    let mut defined: HashMap<String, usize> = HashMap::new(); // name -> def line
    let mut called: HashSet<String> = HashSet::new();
    let mut imported: Vec<(String, usize)> = Vec::new();
    walk_all(&prog.stmts, &mut |st, line| match st {
        Stmt::Gene(g) | Stmt::Seq(g) => {
            if let Some(n) = &g.name {
                defined.entry(n.clone()).or_insert(line);
            }
        }
        Stmt::Use(path, alias) => {
            // the alias is the callable prefix in this file (`use std/set as
            // st` -> members are `st.x`); without an alias the last segment is
            let last = path.rsplit('/').next().unwrap_or(path).to_string();
            imported.push((alias.clone().unwrap_or(last), line));
        }
        _ => {}
    });
    collect_calls_stmts(&prog.stmts, &mut called);
    // W48 corpus law: proof frames and named frames are CONSUMERS. A gene
    // used only inside a proof frame IS used (the test suite is the
    // consumer); flagging it was the loudest false positive on tests/*.op.
    for body in &prog.proofs {
        collect_calls_stmts(body, &mut called);
    }
    for (_, body) in &prog.named_frames {
        collect_calls_stmts(body, &mut called);
    }
    // Genes defined inside a splice variant are the splice's dispatch
    // surface: they are reached through the ROOT name (often from another
    // file), so in-file call counts say nothing. Exempt them wholesale.
    let mut splice_hooks: HashSet<String> = HashSet::new();
    for st in &prog.stmts {
        if let Stmt::Splice(s) = st {
            for (_, g) in &s.variants {
                collect_gene_names(&g.body, &mut splice_hooks);
            }
        }
    }

    // W42: every name read anywhere (see collect_reads_stmts). A bare
    // reference to a gene (e.g. `spawn(quick, ...)` passing the gene for
    // dynamic dispatch) counts as a use: the call happens through the
    // captured reference, the documented dynamic-call escape hatch.
    let mut reads: HashSet<String> = HashSet::new();
    collect_reads_stmts(&prog.stmts, &mut reads);
    for body in &prog.proofs {
        collect_reads_stmts(body, &mut reads);
    }
    for (_, body) in &prog.named_frames {
        collect_reads_stmts(body, &mut reads);
    }

    // W24 module surface: a file with explicit `pub` marks is a library
    // module (default-open exports everything, and consumers may reference
    // any name, sometimes only via strings: `has(mod, "NAME")`). In-file
    // "unused" means nothing there, so the def-side rules stay silent.
    let is_module = !prog.pub_exports.is_empty();
    // W48 corpus law: a file with NO `main` gene is a library module too
    // (W24 default-open: every gene is importable surface; the linter is
    // file-local and cannot see the consumers). This is what keeps std/*.op
    // and the `use`-target helper modules (tests/differential/mod_res_lib*)
    // clean without pub marks. Programs (with main) get the full check.
    let has_main = prog
        .stmts
        .iter()
        .any(|s| matches!(s, Stmt::Gene(g) if g.name.as_deref() == Some("main")));
    let is_library = is_module || !has_main;

    // W42: unused-gene (only when defined AND never called or referenced
    // anywhere in-file; exported genes (anchor/tad exports, pub marks) and
    // splice-variant hooks are library surface, skip them)
    let exported: HashSet<String> = prog.anchor_exports.iter().cloned().collect();
    for (name, line) in &defined {
        if name == "main"
            || exported.contains(name)
            || splice_hooks.contains(name)
            || is_library
            || reads.contains(name)
        {
            continue;
        }
        if !called.contains(name) {
            out.push(Finding::new(
                *line,
                "unused-gene",
                Sev::Style,
                format!(
                    "gene '{}' is defined but never called in this file (library code? export it)",
                    name
                ),
            ));
        }
    }
    // W42: unused-import (module imported, none of its members referenced)
    for (name, line) in &imported {
        if !called
            .iter()
            .any(|c| c == name || c.starts_with(&format!("{}.", name)))
        {
            out.push(Finding::new(
                *line,
                "unused-import",
                Sev::Style,
                format!("module '{}' is imported but never referenced", name),
            ));
        }
    }

    // W05: const-reassign (static best-effort), assignment to a name that
    // was const-bound in this file and never re-bound by a plain let. The
    // check is name-based and scope-insensitive BY DESIGN: any let/const
    // re-binding of the name anywhere retires the finding, so the rule can
    // only stay silent on shadowed bindings, never misfire on them.
    // W48 corpus law: an assignment lexically inside a `stress` block is an
    // EXPLICIT containment probe (the catchable frozen stress is the point,
    // see tests/const_freeze.op), so the scan tracks stress nesting and
    // stays silent there. Outside stress it stays a warning.
    let mut const_names: HashSet<String> = HashSet::new();
    let mut let_rebound: HashSet<String> = HashSet::new();
    walk_all(&prog.stmts, &mut |st, _| match st {
        Stmt::LetConst(n, _) => {
            const_names.insert(n.clone());
        }
        Stmt::Let(n, _) | Stmt::LetAnn(n, _, _) => {
            let_rebound.insert(n.clone());
        }
        Stmt::LetPat(..) => {
            // destructuring re-binds pieces by name, retire nothing specific;
            // a pattern re-binding the const's name is rare and the runtime
            // stress stays the backstop
        }
        _ => {}
    });
    const_reassign_scan(&prog.stmts, false, &const_names, &let_rebound, &mut out);

    // per-statement rules over the whole tree
    walk_all(&prog.stmts, &mut |st, line| {
        match st {
            // W42: constant-condition
            Stmt::If(arms, _) => {
                if let Some((cond, _)) = arms.first() {
                    if let Some(msg) = constant_condition(cond) {
                        out.push(Finding::new(line, "constant-condition", Sev::Warning, msg));
                    }
                }
            }
            Stmt::While(cond, _) => {
                if let Some(msg) = constant_condition(cond) {
                    let always_true = msg.contains("always true");
                    out.push(Finding::new(line, "constant-condition", Sev::Warning, msg));
                    if always_true {
                        out.push(Finding::new(
                            line,
                            "infinite-loop-suspect",
                            Sev::Style,
                            "`while` over an always-true literal, if this is intentional, \
                             document it; fuel bounds contain it at runtime"
                                .to_string(),
                        ));
                    }
                }
            }
            // W42: unreachable-code (return/raise/raise-like followed by siblings)
            Stmt::Return(_) | Stmt::Raise(_, _, _) => {} // W007: raise carries its line
            _ => {}
        }
    });
    unreachable_scan(&prog.stmts, 1, &mut out);
    duplicate_match_arms(&prog.stmts, &mut out);
    unreachable_match_arms(&prog.stmts, &mut out);

    // W64: deprecated-use at call sites. The def-side map fills from
    // top-level AND nested defs (the scan registers nested marks as it
    // descends); proofs and named frames are ordinary consumers.
    let mut deprecations: DepMap = HashMap::new();
    walk_all(&prog.stmts, &mut |st, _| {
        if let Stmt::Gene(g) | Stmt::Seq(g) = st {
            if let (Some(n), Some(d)) = (&g.name, &g.deprecated) {
                deprecations
                    .entry(n.clone())
                    .or_insert((d.message.clone(), d.since.clone()));
            }
        }
    });
    if !deprecations.is_empty() {
        deprecated_scan(&prog.stmts, &mut deprecations, &mut out);
        for body in &prog.proofs {
            deprecated_scan(body, &mut deprecations, &mut out);
        }
        for (_, body) in &prog.named_frames {
            deprecated_scan(body, &mut deprecations, &mut out);
        }
    }

    // W43: wrong-arity at call sites. Free genes +, now, phenotype methods
    // and `new` constructors where the receiver/phenotype is statically
    // known in this file (see PhenoKnowledge below).
    let mut shadowed: HashSet<String> = HashSet::new();
    collect_shadowed_bindings(&prog.stmts, &mut shadowed);
    let knowledge = PhenoKnowledge::collect(prog);
    arity_scan(&prog.stmts, &arities, &shadowed, &knowledge, &mut out);

    // W42 (critique-pinned set): unused binding + dead const. Name-level,
    // file-wide, conservative: any read of the NAME anywhere (including
    // inside gene bodies, closures, patterns, rescue blocks, proof frames)
    // retires the finding, so the rule can only stay silent on scope tricks,
    // never misfire on them. `_`-prefixed names are exempt by convention;
    // library files (no main gene / pub marks) are exempt wholesale — their
    // bindings are surface. W48 corpus law: a binding is also exempt when
    // (a) it is lexically inside a `stress` block — the RHS evaluation IS
    // the probe (containment ceilings, frozen stresses), or (b) its
    // initializer calls something — the binding may exist FOR that call's
    // effect (spawn handles, profile snapshots), and only a pure dead store
    // is reported.
    let mut candidates: Vec<(String, usize, bool, bool)> = Vec::new(); // name, hint line, is_const, exempt
    collect_binding_candidates(&prog.stmts, false, &mut candidates);
    // name-level rule: one finding per name (first non-exempt binding wins
    // the hint line) even when several scopes bind the same name
    let mut reported: HashSet<String> = HashSet::new();
    for (name, line, is_const, exempt) in candidates {
        if exempt || is_library || name == "_" || name.starts_with('_') {
            continue;
        }
        if reads.contains(&name) || !reported.insert(name.clone()) {
            continue;
        }
        if is_const {
            out.push(Finding::new(
                line,
                "dead-const",
                Sev::Warning,
                format!(
                    "const '{}' is bound but never referenced in this file; \
                     drop it, or silence with '// allow: dead-const'",
                    name
                ),
            ));
        } else {
            out.push(Finding::new(
                line,
                "unused-binding",
                Sev::Warning,
                format!(
                    "binding '{}' is never read in this file (dead store); \
                     '_'-prefix or '// allow: unused-binding' silences",
                    name
                ),
            ));
        }
    }

    // N12 shadowed-binding: specified, DELIBERATELY not emitted. The corpus
    // proves a same-block re-`let` is idiomatic Operon (loop-counter resets
    // in tests/repressi_osc.op) and is pinned ON PURPOSE in tests/
    // const_freeze.op ("const redefinition rebinds"). Without liveness
    // analysis the rule cannot tell a reset from an accident, so default-on
    // would be a false-positive machine against the zero-tolerance corpus
    // bar. The N12 code stays reserved; see docs/specs/LINT.md.

    out.sort_by_key(|f| (f.line, f.rule.clone()));
    out
}

/// W48 helper: gene names defined anywhere under `stmts` (splice-variant
/// hooks and nested defs alike).
fn collect_gene_names(stmts: &[Stmt], out: &mut HashSet<String>) {
    for st in stmts {
        if let Stmt::Gene(g) = st {
            if let Some(n) = &g.name {
                out.insert(n.clone());
            }
        }
        for nested in stmt_nested_all(st) {
            collect_gene_names(nested, out);
        }
    }
}

/// W48: const-reassign walk that tracks `stress` nesting. Assignments inside
/// a stress block (or its rescue) are explicit containment probes and stay
/// silent; everything else keeps the W05 warning. Hint lines mirror
/// `walk_all` (statement index within the block), matching the historical
/// finding shape.
fn const_reassign_scan(
    stmts: &[Stmt],
    in_stress: bool,
    consts: &HashSet<String>,
    let_rebound: &HashSet<String>,
    out: &mut Vec<Finding>,
) {
    for (i, st) in stmts.iter().enumerate() {
        if let Stmt::Assign(n, _, _) = st {
            if !in_stress && consts.contains(n) && !let_rebound.contains(n) {
                out.push(Finding::new(
                    i + 1,
                    "const-reassign",
                    Sev::Warning,
                    format!(
                        "assignment to const '{}' raises a catchable frozen stress at runtime",
                        n
                    ),
                ));
            }
        }
        match st {
            Stmt::Stress { body, rescue, .. } => {
                const_reassign_scan(body, true, consts, let_rebound, out);
                if let Some((_, rb)) = rescue {
                    const_reassign_scan(rb, true, consts, let_rebound, out);
                }
            }
            _ => {
                for nested in stmt_nested_all(st) {
                    const_reassign_scan(nested, in_stress, consts, let_rebound, out);
                }
            }
        }
    }
}

/// W48: binding-candidate walk with the stress/call-effect exemptions. A
/// candidate is (name, hint line, is_const, exempt). Exempt candidates never
/// win the name-level slot. Hint lines mirror `walk_all` (statement index
/// within the block), matching the historical finding shape.
fn collect_binding_candidates(
    stmts: &[Stmt],
    in_stress: bool,
    out: &mut Vec<(String, usize, bool, bool)>,
) {
    for (i, st) in stmts.iter().enumerate() {
        let hint = i + 1;
        match st {
            Stmt::Let(n, e) => out.push((n.clone(), hint, false, in_stress || expr_has_call(e))),
            Stmt::LetAnn(n, _, e) => {
                out.push((n.clone(), hint, false, in_stress || expr_has_call(e)))
            }
            Stmt::LetConst(n, e) => {
                out.push((n.clone(), hint, true, in_stress || expr_has_call(e)))
            }
            _ => {}
        }
        let next_stress = matches!(st, Stmt::Stress { .. });
        for nested in stmt_nested_all(st) {
            collect_binding_candidates(nested, in_stress || next_stress, out);
        }
    }
}

/// W48: does this expression tree contain a call (or method/new/collection
/// walk)? Conservative in the SILENT direction: any plausible effect makes
/// the binding exempt from unused-binding.
fn expr_has_call(e: &Expr) -> bool {
    match e {
        Expr::Call(..) | Expr::Method(..) | Expr::MethodSafe(..) | Expr::New(..) => true,
        Expr::Collect {
            iter, filter, body, ..
        } => {
            expr_has_call(iter)
                || filter.as_ref().map(|f| expr_has_call(f)).unwrap_or(false)
                || expr_has_call(body)
        }
        Expr::Unary(_, a) => expr_has_call(a),
        Expr::Binary(_, a, b, _) => expr_has_call(a) || expr_has_call(b),
        Expr::Index(a, b, _) => expr_has_call(a) || expr_has_call(b),
        Expr::Member(a, _) | Expr::MemberSafe(a, _) => expr_has_call(a),
        Expr::List(items) => items.iter().any(expr_has_call),
        Expr::Map(pairs) => pairs
            .iter()
            .any(|(k, v)| expr_has_call(k) || expr_has_call(v)),
        Expr::Interp(parts) => parts.iter().any(|p| match p {
            crate::ast::InterpPart::Expr(x) => expr_has_call(x),
            _ => false,
        }),
        Expr::Ternary(a, b, c) => expr_has_call(a) || expr_has_call(b) || expr_has_call(c),
        Expr::Propagate(a, _) => expr_has_call(a),
        _ => false,
    }
}

/// W41: `lint` plus the allow-comment suppression pass (see the module-doc
/// scheme table). Kept for compatibility; the CLI surfaces apply
/// `apply_allows` to their stream-filtered findings.
pub fn lint_with_source(prog: &Program, src: &str) -> Vec<Finding> {
    let mut out = lint(prog);
    apply_allows(&mut out, src);
    out
}

/// W41/W048: the allow-comment suppression pass (see the module-doc scheme
/// table), public so BOTH CLI surfaces (lint and check) apply the same
/// line-local mechanism to their stream-filtered findings.
pub fn apply_allows(out: &mut Vec<Finding>, src: &str) {
    suppress_allowed(out, src);
}

/// W048: CLI-side `--allow rule1,rule2`, layered ON TOP of the comment
/// mechanism. A rule named here is dropped file-wide (rule name OR stable
/// code), while `// allow:` comments stay line-local. Bare `all` is NOT
/// special: `--allow` names rules, it does not switch the engine off.
pub fn apply_cli_allows(out: &mut Vec<Finding>, rules: &[String]) {
    if rules.is_empty() {
        return;
    }
    out.retain(|f| {
        !rules
            .iter()
            .any(|r| r.as_str() == f.rule || r.as_str() == f.code)
    });
}

/// Parse `allow:` suppression comments: line -> Some(None) means "allow
/// everything on this line", Some(Some(rules)) means "allow these rules".
fn parse_allow(line: &str) -> Option<Option<Vec<String>>> {
    // comment start: `#` (the language comment) or `//` (C-style tolerance)
    let start = line.find('#').or_else(|| line.find("//"))?;
    let comment = line[start + 1..].trim();
    let rest = comment.strip_prefix("allow:")?.trim();
    if rest.is_empty() {
        return Some(None);
    }
    Some(Some(
        rest.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    ))
}

fn suppress_allowed(out: &mut Vec<Finding>, src: &str) {
    let mut allows: HashMap<usize, Option<Vec<String>>> = HashMap::new();
    for (i, line) in src.lines().enumerate() {
        if let Some(rules) = parse_allow(line) {
            allows.insert(i + 1, rules);
        }
    }
    if allows.is_empty() {
        return;
    }
    out.retain(|f| {
        if f.line == 0 {
            return true; // file-level findings are not line-suppressible
        }
        for l in [f.line, f.line.saturating_sub(1)] {
            if let Some(rules) = allows.get(&l) {
                match rules {
                    None => return false,
                    Some(v)
                        if v.iter()
                            .any(|r| r.as_str() == f.rule || r.as_str() == f.code) =>
                    {
                        return false
                    }
                    _ => {}
                }
            }
        }
        true
    });
}

// ------------------------------------------------------------ W43: phenotype knowledge

/// What lint.rs can know about phenotype methods without running anything:
/// same-file phenotype defs (+ their `from` lineage, + trait DEFAULT
/// methods), and the let-bound receivers statically constructed with
/// `new Pheno(...)`. Anything ambiguous (re-bound, assigned, shadowed,
/// defined in another module) is simply absent => silent. This is the
/// documented escape hatch: dynamic dispatch (map values, captured
/// references, field-held callables, unknown receivers) is excluded.
struct PhenoKnowledge {
    /// var name -> phenotype name ("" tombstone = bound to something else)
    receiver_pheno: HashMap<String, String>,
    phenos: HashMap<String, Vec<(String, Arity)>>,
    parents: HashMap<String, String>,
    pheno_traits: HashMap<String, Vec<String>>,
    traits: HashMap<String, Vec<(String, Arity)>>,
}

impl PhenoKnowledge {
    fn collect(prog: &Program) -> PhenoKnowledge {
        let mut phenos: HashMap<String, Vec<(String, Arity)>> = HashMap::new();
        let mut parents: HashMap<String, String> = HashMap::new();
        let mut pheno_traits: HashMap<String, Vec<String>> = HashMap::new();
        let mut traits: HashMap<String, Vec<(String, Arity)>> = HashMap::new();
        walk_all(&prog.stmts, &mut |st, _| match st {
            Stmt::Pheno(p) => {
                let methods = p
                    .methods
                    .iter()
                    // a method whose params mention `self` has legacy shape
                    // the runtime binds oddly; stay silent on it
                    .filter(|g| !g.params.iter().any(|(n, _)| n == "self"))
                    .map(|g| (g.name.clone().unwrap_or_default(), param_arity(&g.params)))
                    .collect();
                phenos.insert(p.name.clone(), methods);
                if let Some(par) = &p.parent {
                    parents.insert(p.name.clone(), par.clone());
                }
                if !p.implements.is_empty() {
                    pheno_traits.insert(p.name.clone(), p.implements.clone());
                }
            }
            Stmt::Trait(t) => {
                let defaults = t
                    .methods
                    .iter()
                    .filter_map(|m| m.default.as_ref())
                    .filter(|g| !g.params.iter().any(|(n, _)| n == "self"))
                    .map(|g| (g.name.clone().unwrap_or_default(), param_arity(&g.params)))
                    .collect();
                traits.insert(t.name.clone(), defaults);
            }
            _ => {}
        });
        // receiver bindings: only plain lets whose RHS is `new Pheno(...)`;
        // any re-binding or assignment tombstones the name permanently.
        let mut receiver_pheno: HashMap<String, String> = HashMap::new();
        collect_receiver_knowledge(&prog.stmts, &mut receiver_pheno);
        PhenoKnowledge {
            receiver_pheno,
            phenos,
            parents,
            pheno_traits,
            traits,
        }
    }

    /// First-hit dispatch order mirrors the runtime: own methods, then the
    /// `from` lineage, then trait DEFAULT methods in `implements` order.
    fn method_arity(&self, pheno: &str, name: &str, depth: usize) -> Option<Arity> {
        if depth > 16 {
            return None; // lineage cycle cap: stay silent, never loop
        }
        if let Some(list) = self.phenos.get(pheno) {
            if let Some((_, a)) = list.iter().find(|(n, _)| n == name) {
                return Some(*a);
            }
        }
        if let Some(par) = self.parents.get(pheno) {
            if let Some(a) = self.method_arity(par, name, depth + 1) {
                return Some(a);
            }
        }
        if let Some(ts) = self.pheno_traits.get(pheno) {
            for t in ts {
                if let Some(list) = self.traits.get(t) {
                    if let Some((_, a)) = list.iter().find(|(n, _)| n == name) {
                        return Some(*a);
                    }
                }
            }
        }
        None
    }
}

/// Receiver knowledge walker: block-scoped nesting only. Gene / phenotype /
/// trait / splice bodies are separate scopes; a `let` there must never
/// create receiver knowledge for other scopes.
fn collect_receiver_knowledge(stmts: &[Stmt], rec: &mut HashMap<String, String>) {
    for st in stmts {
        match st {
            Stmt::Let(n, e) | Stmt::LetAnn(n, _, e) => {
                let ph = match e {
                    Expr::New(pn, _) => Some(pn.clone()),
                    _ => None,
                };
                match (rec.get(n).cloned(), ph) {
                    (None, Some(p)) => {
                        rec.insert(n.clone(), p);
                    }
                    _ => {
                        // re-bound, assigned, or bound to a non-constructor:
                        // tombstone (empty string) forever
                        rec.insert(n.clone(), String::new());
                    }
                }
            }
            Stmt::Assign(n, _, _) => {
                rec.insert(n.clone(), String::new());
            }
            Stmt::MultiAssign(targets, _, _) => {
                for t in targets {
                    if let Expr::Ident(n) = t {
                        rec.insert(n.clone(), String::new());
                    }
                }
            }
            _ => {}
        }
        match st {
            Stmt::If(arms, els) => {
                for (_, b) in arms {
                    collect_receiver_knowledge(b, rec);
                }
                if let Some(e) = els {
                    collect_receiver_knowledge(e, rec);
                }
            }
            Stmt::While(_, b)
            | Stmt::Loop(b)
            | Stmt::Scope(b)
            | Stmt::Block(b)
            | Stmt::Tad(_, b)
            | Stmt::For(_, _, b)
            | Stmt::ForPat(_, _, b) => collect_receiver_knowledge(b, rec),
            Stmt::Frame { body, .. } => collect_receiver_knowledge(body, rec),
            Stmt::Stress { body, rescue, .. } => {
                collect_receiver_knowledge(body, rec);
                if let Some((_, rb)) = rescue {
                    collect_receiver_knowledge(rb, rec);
                }
            }
            _ => {}
        }
    }
}

fn constant_condition(e: &Expr) -> Option<String> {
    match e {
        Expr::Int(v) => Some(format!(
            "condition is the constant {}, {}",
            v,
            if *v != 0 {
                "always true"
            } else {
                "always false"
            }
        )),
        Expr::Bool(b) => Some(format!(
            "condition is the constant {}, {}",
            b,
            if *b { "always true" } else { "always false" }
        )),
        _ => None,
    }
}

// return/raise followed by any statement in the same block
fn unreachable_scan(stmts: &[Stmt], line_hint: usize, out: &mut Vec<Finding>) {
    let mut terminated_at: Option<(usize, &'static str)> = None;
    for (i, st) in stmts.iter().enumerate() {
        if let Some((_, kind)) = terminated_at {
            out.push(Finding::new(
                stmt_line(st).unwrap_or(line_hint),
                "unreachable-code",
                Sev::Warning,
                format!("statement after `{}` is unreachable", kind),
            ));
            let _ = i;
        }
        match st {
            Stmt::Return(_) => terminated_at = Some((i, "return")),
            Stmt::Raise(_, _, _) => terminated_at = Some((i, "raise")),
            _ => terminated_at = None,
        }
        // recurse into nested blocks
        if let Some(nested) = nested_stmts(st) {
            unreachable_scan(nested, stmt_line(st).unwrap_or(line_hint), out);
        }
    }
}

fn duplicate_match_arms(stmts: &[Stmt], out: &mut Vec<Finding>) {
    for st in stmts {
        if let Stmt::Match(_, arms, _) = st {
            let mut seen: HashMap<String, usize> = HashMap::new();
            for (pat, body) in arms {
                if let crate::ast::MatchPat::Lit(e) = pat {
                    let key = lit_key(e);
                    if let Some(first) = seen.get(&key) {
                        out.push(Finding::new(
                            body.first().and_then(stmt_line).unwrap_or(1),
                            "duplicate-match-arm",
                            Sev::Warning,
                            format!(
                                "duplicate literal match arm (same literal first seen at arm {})",
                                first
                            ),
                        ));
                    } else {
                        seen.insert(key, seen.len() + 1);
                    }
                }
            }
        }
        if let Some(nested) = nested_stmts(st) {
            duplicate_match_arms(nested, out);
        }
    }
}

fn lit_key(e: &Expr) -> String {
    match e {
        Expr::Int(v) => format!("i:{}", v),
        Expr::Bool(b) => format!("b:{}", b),
        Expr::Str(s) => format!("s:{}", s),
        Expr::Float(v) => format!("f:{}", v.to_bits()),
        _ => format!("o:{:?}", e),
    }
}

/// W42/W06 (W002 stage 2, check-side slice): unreachable match arms. An arm
/// can never run when an EARLIER unguarded arm already matches every value
/// it can match (first-match-wins is the runtime contract, SPEC §5a). Two
/// reasons, in strength order:
///
///   1. unguarded catch-all: `_`, a binding pattern, or an or-pattern
///      containing either (the legacy comma-run holds plain literals only,
///      so it can never hide a catch-all; the parser degrades unknown
///      capitalized tags to Bind at parse time, so a catch-all here is
///      always runtime-true);
///   2. a strictly broader shape: `Some(_)` before `Some(x)`, `[a, b]`
///      before `[1, 2]`, `{x}` before `{x: 1, ...}`, a literal inside an
///      earlier multi/or run, nested payloads likewise.
///
/// Conservatism is the design bar (zero false positives): a GUARD on the
/// earlier arm makes it fallible, so a guarded arm subsumes nothing; a guard
/// on the LATER arm is transparent (it only shrinks a set already covered,
/// the guard itself can never evaluate). Structural coverage is claimed
/// only where the pattern algebra proves it (see `pat_subsumes`); identical
/// plain literal arms stay duplicate-match-arm's story (W05) so one symptom
/// has one owner. This is the check-time slice of W002 stage 2: full
/// exhaustiveness/reachability lives with the pattern algebra in the
/// semantic lane, this one needs nothing but the AST.
fn unreachable_match_arms(stmts: &[Stmt], out: &mut Vec<Finding>) {
    for st in stmts {
        if let Stmt::Match(_, arms, _) = st {
            for (i, (pat, body)) in arms.iter().enumerate() {
                // scan earlier UNGUARDED arms only: `if` makes an arm able to
                // miss, so it guarantees nothing about what follows
                let mut catchall: Option<usize> = None;
                let mut broader: Option<usize> = None;
                for (j, (earlier, _)) in arms.iter().take(i).enumerate() {
                    if matches!(earlier, MatchPat::Guard(_, _)) {
                        continue;
                    }
                    if catchall.is_none() && pat_is_catchall(earlier) {
                        catchall = Some(j);
                    }
                    if broader.is_none() && arm_subsumes(earlier, pat) {
                        broader = Some(j);
                    }
                }
                let finding = match catchall {
                    Some(_) => Some(format!(
                        "match arm {} is unreachable: an earlier arm matches every value \
                         (unguarded catch-all)",
                        i + 1
                    )),
                    None => broader.map(|j| {
                        format!(
                            "match arm {} is unreachable: arm {} already matches every value \
                             it can match (broader arm first)",
                            i + 1,
                            j + 1
                        )
                    }),
                };
                if let Some(msg) = finding {
                    // first body statement that knows its line; the AST keeps
                    // statement spans only on expression statements (A13), so
                    // an arm opening with spanless statements (a plain
                    // assignment) stays file-anchored at line 1
                    let line = body.iter().find_map(stmt_line).unwrap_or(1);
                    out.push(Finding::new(
                        line,
                        "unreachable-match-arm",
                        Sev::Warning,
                        msg,
                    ));
                }
            }
            // arm bodies are descended exactly once, via stmt_nested_all
            // below: the pre-stage-2 code ALSO recursed into them manually,
            // so an offender nested inside an arm body was reported twice
        }
        for nested in stmt_nested_all(st) {
            unreachable_match_arms(nested, out);
        }
    }
}

fn pat_is_catchall(pat: &MatchPat) -> bool {
    match pat {
        MatchPat::Wild | MatchPat::Bind(_) => true,
        MatchPat::Or(alts) => alts.iter().any(pat_is_catchall),
        _ => false, // guarded catch-alls can fail: never treated as catch-all
    }
}

/// Arm-level subsumption entry: identical plain literal arms are deferred to
/// duplicate-match-arm (W05), which already reports them with an arm-number
/// hint. Everything composite (multi-runs, or-alternatives, nested shapes,
/// guards on the later arm) stays here: W05 cannot see those.
fn arm_subsumes(earlier: &MatchPat, later: &MatchPat) -> bool {
    if let (MatchPat::Lit(a), MatchPat::Lit(b)) = (earlier, later) {
        if let (Some(k1), Some(k2)) = (lit_key_prim(a), lit_key_prim(b)) {
            if k1 == k2 {
                return false; // W05's finding, not this rule's
            }
        }
    }
    pat_subsumes(earlier, later)
}

/// Conservative structural subsumption: true only when EVERY value `later`
/// can match is guaranteed to be matched by `earlier` first. Runtime mirror
/// is `Interp::match_pat` (first-match-wins, deep_eq literals); when the
/// algebra cannot prove coverage the answer is false, so the rule can stay
/// silent on hard shapes (float cross-matching, computed literals) but never
/// invent an unreachable arm.
fn pat_subsumes(earlier: &MatchPat, later: &MatchPat) -> bool {
    // a guard on the LATER side is transparent: the arm matches a subset of
    // the inner pattern's values, and a covered subset stays covered
    let later = match later {
        MatchPat::Guard(inner, _) => inner,
        p => p,
    };
    // alternation on the later side is reachable through ANY alternative,
    // so coverage must hold for every one of them
    match later {
        MatchPat::Or(alts) => return alts.iter().all(|a| pat_subsumes(earlier, a)),
        MatchPat::Multi(ls) => {
            return ls
                .iter()
                .all(|e| pat_subsumes(earlier, &MatchPat::Lit(e.clone())))
        }
        _ => {}
    }
    match earlier {
        // a guard can miss: an earlier guarded arm guarantees nothing
        MatchPat::Guard(_, _) => false,
        // catch-alls match every value (the parser degrades unknown
        // capitalized tags to Bind, so Variant here is always a known family)
        MatchPat::Wild | MatchPat::Bind(_) => true,
        MatchPat::Or(alts) => alts.iter().any(|a| pat_subsumes(a, later)),
        MatchPat::Multi(ls) => ls
            .iter()
            .any(|e| pat_subsumes(&MatchPat::Lit(e.clone()), later)),
        MatchPat::Lit(e) => match later {
            // float keys are excluded on purpose: NaN never deep_eq-equals
            // itself, so an earlier float literal guarantees nothing
            MatchPat::Lit(e2) => match (lit_key_prim(e), lit_key_prim(e2)) {
                (Some(k1), Some(k2)) => k1 == k2,
                _ => false,
            },
            _ => false,
        },
        MatchPat::Variant(tag, payload) => match later {
            MatchPat::Variant(tag2, payload2) => {
                // tags are case-sensitive and families are distinct
                tag == tag2
                    && match (payload, payload2) {
                        // tag-only form matches the tag with ANY payload,
                        // payload-less values included: it covers every
                        // payload-carrying later pattern on the same tag
                        (None, _) => true,
                        // a payload-carrying earlier arm does NOT cover the
                        // tag-only form: the runtime misses a payload pattern
                        // against a payload-less value (match_pat's
                        // (Some, None) arm), so coverage would be a guess
                        (Some(_), None) => false,
                        (Some(p1), Some(p2)) => pat_subsumes(p1, p2),
                    }
            }
            _ => false,
        },
        MatchPat::ListPat { elems, rest } => match later {
            MatchPat::ListPat {
                elems: later_elems,
                rest: later_rest,
            } => {
                let len_ok = match (rest, later_rest) {
                    // exact-length earlier: only an exact same-length later
                    // is fully covered; a later *rest matches longer lists
                    (None, _) => later_rest.is_none() && later_elems.len() == elems.len(),
                    // *rest earlier covers every later list at least as long
                    (Some(_), _) => later_elems.len() >= elems.len(),
                };
                len_ok
                    && elems
                        .iter()
                        .zip(later_elems)
                        .all(|(a, b)| pat_subsumes(a, b))
            }
            _ => false,
        },
        MatchPat::MapPat { keys } => match later {
            MatchPat::MapPat { keys: later_keys } => {
                // the earlier arm's constraints must all be implied by the
                // later arm: every key it requires must be required there
                // too, with a sub-pattern the later one cannot escape
                keys.iter().all(|(ek, ep)| {
                    later_keys.iter().any(|(lk, lp)| {
                        lk == ek
                            && match (ep, lp) {
                                (None, _) => true,
                                (Some(e), None) => pat_subsumes(e, &MatchPat::Wild),
                                (Some(e), Some(l)) => pat_subsumes(e, l),
                            }
                    })
                })
            }
            _ => false,
        },
    }
}

/// Primitive literal identity for subsumption. Only constructor-held values
/// whose runtime deep_eq is exact qualify; floats are excluded (NaN never
/// equals itself) and anything computed returns None, so the rule stays
/// silent where the runtime value could surprise the algebra. Bytes keys are
/// per-byte hex, not from_utf8_lossy: lossy is not injective (b"\xff" and
/// b"\xef\xbf\xbd" both collapse to U+FFFD), and equal keys here are claimed
/// as coverage, so a collision would invent an unreachable arm.
fn lit_key_prim(e: &Expr) -> Option<String> {
    match e {
        Expr::Int(v) => Some(format!("i:{}", v)),
        Expr::Bool(b) => Some(format!("b:{}", b)),
        Expr::Str(s) => Some(format!("s:{}", s)),
        Expr::Null => Some("n:".to_string()),
        Expr::Bytes(b) => {
            let hex: String = b.iter().map(|x| format!("{:02x}", x)).collect();
            Some(format!("y:{}", hex))
        }
        _ => None,
    }
}

/// W42: every name READ anywhere in the statement list. Name-level and
/// file-wide by design (conservative): it can only over-count reads, never
/// under-count, so unused-binding/dead-const can only stay silent, never
/// misfire. Exhaustive on purpose — a new Stmt/Expr variant must be
/// triaged here or a real read could be missed.
fn collect_reads_stmts(stmts: &[Stmt], reads: &mut HashSet<String>) {
    for st in stmts {
        for e in stmt_exprs_all(st) {
            collect_reads_expr(e, reads);
        }
        // compound assignment reads its target name (`x += 1`); plain
        // assignment does not (that is a dead store, the rule's whole point)
        if let Stmt::Assign(n, Some(_), _) = st {
            reads.insert(n.clone());
        }
        if let Stmt::Match(_, arms, _) = st {
            for (pat, body) in arms {
                collect_pat_reads(pat, reads);
                collect_reads_stmts(body, reads);
            }
        }
        for nested in stmt_nested_all(st) {
            collect_reads_stmts(nested, reads);
        }
    }
}

/// Pattern positions can embed reads: guard conditions see the bindings,
/// literal patterns may carry interpolated expressions.
fn collect_pat_reads(pat: &MatchPat, reads: &mut HashSet<String>) {
    match pat {
        MatchPat::Lit(e) => collect_reads_expr(e, reads),
        MatchPat::Multi(es) => {
            for e in es {
                collect_reads_expr(e, reads);
            }
        }
        MatchPat::Guard(_, cond) => collect_reads_expr(cond, reads),
        MatchPat::Variant(_, Some(inner)) => collect_pat_reads(inner, reads),
        MatchPat::ListPat { elems, .. } => {
            for p in elems {
                collect_pat_reads(p, reads);
            }
        }
        MatchPat::MapPat { keys } => {
            for (_, sub) in keys {
                if let Some(p) = sub {
                    collect_pat_reads(p, reads);
                }
            }
        }
        MatchPat::Or(alts) => {
            for p in alts {
                collect_pat_reads(p, reads);
            }
        }
        _ => {}
    }
}

fn collect_reads_expr(e: &Expr, reads: &mut HashSet<String>) {
    match e {
        Expr::Ident(n) => {
            reads.insert(n.clone());
        }
        Expr::Interp(parts) => {
            for p in parts {
                if let crate::ast::InterpPart::Expr(x) = p {
                    collect_reads_expr(x, reads);
                }
            }
        }
        Expr::List(items) => {
            for i in items {
                collect_reads_expr(i, reads);
            }
        }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_reads_expr(k, reads);
                collect_reads_expr(v, reads);
            }
        }
        Expr::Unary(_, a) => collect_reads_expr(a, reads),
        Expr::Binary(_, a, b, _) => {
            collect_reads_expr(a, reads);
            collect_reads_expr(b, reads);
        }
        Expr::Call(c, args, _) => {
            collect_reads_expr(c, reads);
            for a in args {
                collect_reads_expr(a, reads);
            }
        }
        Expr::Index(a, b, _) => {
            collect_reads_expr(a, reads);
            collect_reads_expr(b, reads);
        }
        Expr::Member(a, _) | Expr::MemberSafe(a, _) => collect_reads_expr(a, reads),
        Expr::Method(recv, _, args, _) | Expr::MethodSafe(recv, _, args, _) => {
            collect_reads_expr(recv, reads);
            for a in args {
                collect_reads_expr(a, reads);
            }
        }
        Expr::Lambda(g) => {
            for (_, d) in &g.params {
                if let Some(d) = d {
                    collect_reads_expr(d, reads);
                }
            }
            collect_reads_stmts(&g.body, reads);
        }
        Expr::Collect {
            iter, filter, body, ..
        } => {
            collect_reads_expr(iter, reads);
            if let Some(f) = filter {
                collect_reads_expr(f, reads);
            }
            collect_reads_expr(body, reads);
        }
        Expr::New(_, args) => {
            for a in args {
                collect_reads_expr(a, reads);
            }
        }
        Expr::Ternary(a, b, c) => {
            collect_reads_expr(a, reads);
            collect_reads_expr(b, reads);
            collect_reads_expr(c, reads);
        }
        Expr::Propagate(a, _) => collect_reads_expr(a, reads),
        _ => {}
    }
}

fn collect_shadowed_bindings(stmts: &[Stmt], shadowed: &mut HashSet<String>) {
    for st in stmts {
        match st {
            Stmt::Let(name, _) | Stmt::Assign(name, _, _) => {
                shadowed.insert(name.clone());
            }
            _ => {}
        }
        if let Some(nested) = nested_stmts(st) {
            collect_shadowed_bindings(nested, shadowed);
        }
    }
}

fn arity_scan(
    stmts: &[Stmt],
    arities: &HashMap<String, Arity>,
    shadowed: &HashSet<String>,
    knowledge: &PhenoKnowledge,
    out: &mut Vec<Finding>,
) {
    for st in stmts {
        scan_expr_arities(stmt_exprs_all(st), arities, shadowed, knowledge, out);
        arity_scan_nested(st, arities, shadowed, knowledge, out);
    }
}

/// Complete statement nesting for call-site walks: every variant that holds
/// statement bodies, scanned recursively (If: ALL arms + else, not just the
/// first; Match arms; frames; stress/rescue; defs' bodies). Exhaustive on
/// purpose — a new Stmt variant must be triaged here.
fn arity_scan_nested(
    st: &Stmt,
    arities: &HashMap<String, Arity>,
    shadowed: &HashSet<String>,
    knowledge: &PhenoKnowledge,
    out: &mut Vec<Finding>,
) {
    match st {
        Stmt::If(arms, els) => {
            for (_, b) in arms {
                arity_scan(b, arities, shadowed, knowledge, out);
            }
            if let Some(e) = els {
                arity_scan(e, arities, shadowed, knowledge, out);
            }
        }
        Stmt::While(_, b)
        | Stmt::Loop(b)
        | Stmt::Scope(b)
        | Stmt::Block(b)
        | Stmt::Tad(_, b)
        | Stmt::For(_, _, b)
        | Stmt::ForPat(_, _, b) => arity_scan(b, arities, shadowed, knowledge, out),
        Stmt::Frame { body, .. } => arity_scan(body, arities, shadowed, knowledge, out),
        Stmt::Match(_, arms, _) => {
            for (_, b) in arms {
                arity_scan(b, arities, shadowed, knowledge, out);
            }
        }
        Stmt::Stress { body, rescue, .. } => {
            arity_scan(body, arities, shadowed, knowledge, out);
            if let Some((_, rb)) = rescue {
                arity_scan(rb, arities, shadowed, knowledge, out);
            }
        }
        Stmt::Gene(g) | Stmt::Seq(g) => arity_scan(&g.body, arities, shadowed, knowledge, out),
        Stmt::Pheno(p) => {
            for m in &p.methods {
                arity_scan(&m.body, arities, shadowed, knowledge, out);
            }
        }
        Stmt::Trait(t) => {
            for m in &t.methods {
                if let Some(d) = &m.default {
                    arity_scan(&d.body, arities, shadowed, knowledge, out);
                }
            }
        }
        Stmt::Splice(s) => {
            for (_, g) in &s.variants {
                arity_scan(&g.body, arities, shadowed, knowledge, out);
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------ deprecated-use
// W64: `@deprecated("text", since="x.y")` marks ride the gene as metadata;
// the check surface turns CALL SITES of a marked gene into `deprecated-use`
// warnings carrying the migration text. Runtime is untouched (the
// interpreter never reads the field), so differential parity cannot drift.
// Conservative by design: only free-gene calls (Expr::Call over an ident)
// are flagged; method calls, dynamic dispatch through a bare gene
// reference, and defs inside other files are the documented escape hatches.
// A self-call inside the deprecated gene's own body is still a real use.

type DepMap = HashMap<String, (String, Option<String>)>;

fn deprecated_scan(stmts: &[Stmt], deps: &mut DepMap, out: &mut Vec<Finding>) {
    for st in stmts {
        // nested defs register their own marks before their bodies scan
        if let Stmt::Gene(g) | Stmt::Seq(g) = st {
            if let (Some(n), Some(d)) = (&g.name, &g.deprecated) {
                deps.entry(n.clone())
                    .or_insert((d.message.clone(), d.since.clone()));
            }
        }
        deprecated_scan_expr(stmt_exprs_all(st), deps, out);
        deprecated_scan_nested(st, deps, out);
    }
}

fn deprecated_scan_nested(st: &Stmt, deps: &mut DepMap, out: &mut Vec<Finding>) {
    match st {
        Stmt::If(arms, els) => {
            for (_, b) in arms {
                deprecated_scan(b, deps, out);
            }
            if let Some(e) = els {
                deprecated_scan(e, deps, out);
            }
        }
        Stmt::While(_, b)
        | Stmt::Loop(b)
        | Stmt::Scope(b)
        | Stmt::Block(b)
        | Stmt::Tad(_, b)
        | Stmt::For(_, _, b)
        | Stmt::ForPat(_, _, b) => deprecated_scan(b, deps, out),
        Stmt::Frame { body, .. } => deprecated_scan(body, deps, out),
        Stmt::Match(_, arms, _) => {
            for (_, b) in arms {
                deprecated_scan(b, deps, out);
            }
        }
        Stmt::Stress { body, rescue, .. } => {
            deprecated_scan(body, deps, out);
            if let Some((_, rb)) = rescue {
                deprecated_scan(rb, deps, out);
            }
        }
        Stmt::Gene(g) | Stmt::Seq(g) => deprecated_scan(&g.body, deps, out),
        Stmt::Pheno(p) => {
            for m in &p.methods {
                deprecated_scan(&m.body, deps, out);
            }
        }
        Stmt::Trait(t) => {
            for m in &t.methods {
                if let Some(d) = &m.default {
                    deprecated_scan(&d.body, deps, out);
                }
            }
        }
        Stmt::Splice(s) => {
            for (_, g) in &s.variants {
                deprecated_scan(&g.body, deps, out);
            }
        }
        _ => {}
    }
}

fn deprecated_scan_expr(exprs: Vec<&Expr>, deps: &DepMap, out: &mut Vec<Finding>) {
    for e in exprs {
        match e {
            Expr::Call(callee, args, line) => {
                if let Expr::Ident(name) = &**callee {
                    if let Some((msg, since)) = deps.get(name) {
                        let since_note = match since {
                            Some(s) => format!(" (since {})", s),
                            None => String::new(),
                        };
                        out.push(Finding::new(
                            *line,
                            "deprecated-use",
                            Sev::Warning,
                            format!(
                                "call to deprecated gene '{}'{}, {}, silence with '// allow: deprecated-use'",
                                name, since_note, msg
                            ),
                        ));
                    }
                }
                deprecated_scan_expr(
                    std::iter::once(&**callee).chain(args.iter()).collect(),
                    deps,
                    out,
                );
            }
            Expr::Method(recv, _, args, _) | Expr::MethodSafe(recv, _, args, _) => {
                deprecated_scan_expr(
                    std::iter::once(&**recv).chain(args.iter()).collect(),
                    deps,
                    out,
                );
            }
            Expr::New(_, args) => {
                deprecated_scan_expr(args.iter().collect(), deps, out);
            }
            Expr::Lambda(g) => {
                let defaults: Vec<&Expr> =
                    g.params.iter().filter_map(|(_, d)| d.as_ref()).collect();
                deprecated_scan_expr(defaults, deps, out);
                let mut inner = deps.clone();
                deprecated_scan(&g.body, &mut inner, out);
            }
            Expr::Propagate(a, _) => deprecated_scan_expr(vec![a], deps, out),
            Expr::Collect {
                iter, filter, body, ..
            } => {
                let mut parts = vec![&**iter, &**body];
                if let Some(f) = filter {
                    parts.push(f);
                }
                deprecated_scan_expr(parts, deps, out);
            }
            Expr::Unary(_, a) => deprecated_scan_expr(vec![a], deps, out),
            Expr::Binary(_, a, b, _) => deprecated_scan_expr(vec![a, b], deps, out),
            Expr::Index(a, b, _) => deprecated_scan_expr(vec![a, b], deps, out),
            Expr::Ternary(a, b, c) => deprecated_scan_expr(vec![a, b, c], deps, out),
            Expr::Member(a, _) | Expr::MemberSafe(a, _) => deprecated_scan_expr(vec![a], deps, out),
            Expr::List(items) => deprecated_scan_expr(items.iter().collect(), deps, out),
            Expr::Map(pairs) => {
                deprecated_scan_expr(pairs.iter().flat_map(|(k, v)| [k, v]).collect(), deps, out)
            }
            _ => {}
        }
    }
}

/// Exhaustive per-statement expression extractor (every variant that holds
/// an Expr). The old st_exprs missed if-conditions, match scrutinees, index/
/// member assignments, multi-assign and destructuring iterables, so call
/// sites inside them were invisible to arity + call collection. Exhaustive
/// on purpose — a new Stmt variant must be triaged here.
fn stmt_exprs_all(st: &Stmt) -> Vec<&Expr> {
    match st {
        Stmt::Let(_, e) | Stmt::LetConst(_, e) | Stmt::LetAnn(_, _, e) => vec![e],
        Stmt::LetPat(_, e) => vec![e],
        Stmt::Assign(_, _, e) => vec![e],
        Stmt::IndexAssign(t, i, _, v) => vec![t, i, v],
        Stmt::MemberAssign(t, _, _, v) => vec![t, v],
        Stmt::MultiAssign(ts, vs, _) => ts.iter().chain(vs.iter()).collect(),
        Stmt::If(arms, _) => arms.iter().map(|(c, _)| c).collect(),
        Stmt::While(c, _) => vec![c],
        Stmt::For(_, it, _) => vec![it],
        Stmt::ForPat(_, it, _) => vec![it],
        Stmt::Return(Some(e)) => vec![e],
        Stmt::ExprStmt(e) => vec![e],
        Stmt::Match(scrut, _, _) => vec![scrut],
        Stmt::Raise(_, e, _) => vec![e],
        Stmt::Yield(Some(e)) => vec![e],
        _ => vec![],
    }
}

/// W043: one shared wrong-arity report for free genes, phenotype methods and
/// `new` constructors. Promoted to ERROR per board W043 ("check error before
/// execution"); still Total Grammar: advisory, the run recovers at runtime.
fn arity_finding(
    line: usize,
    callee_desc: &str,
    min: usize,
    max: usize,
    n: usize,
    out: &mut Vec<Finding>,
) {
    let msg = if n < min {
        format!(
            "{} expects {} argument(s), got {}, \
             missing args become Null at runtime (checkable via rescue)",
            callee_desc,
            plural(min),
            n
        )
    } else {
        format!(
            "{} accepts at most {} argument(s), got {}, \
             extras are ignored at runtime",
            callee_desc,
            plural(max),
            n
        )
    };
    out.push(Finding::new(line, "wrong-arity", Sev::Error, msg));
}

fn scan_expr_arities(
    exprs: Vec<&Expr>,
    arities: &HashMap<String, Arity>,
    shadowed: &HashSet<String>,
    knowledge: &PhenoKnowledge,
    out: &mut Vec<Finding>,
) {
    for e in exprs {
        match e {
            Expr::Call(callee, args, line) => {
                if let Expr::Ident(name) = &**callee {
                    if !shadowed.contains(name) {
                        if let Some((min, max)) = arities.get(name) {
                            let n = args.len();
                            if n < *min || n > *max {
                                arity_finding(*line, &format!("'{}'", name), *min, *max, n, out);
                            }
                        }
                    }
                }
                scan_expr_arities(
                    std::iter::once(&**callee).chain(args.iter()).collect(),
                    arities,
                    shadowed,
                    knowledge,
                    out,
                );
            }
            Expr::Method(recv, mname, args, _) | Expr::MethodSafe(recv, mname, args, _) => {
                // W43: method arity when the receiver is a statically known
                // phenotype value (`let p = new Pheno(...)`, single binding,
                // never re-bound) and the method resolves on the same-file
                // lineage. Unknown receivers/methods are the escape hatch.
                if let Expr::Ident(v) = &**recv {
                    if let Some(pheno) = knowledge.receiver_pheno.get(v) {
                        if !pheno.is_empty() {
                            if let Some((min, max)) = knowledge.method_arity(pheno, mname, 0) {
                                let n = args.len();
                                if n < min || n > max {
                                    arity_finding(
                                        expr_line(recv).unwrap_or(0),
                                        &format!("phenotype '{}' method '{}'", pheno, mname),
                                        min,
                                        max,
                                        n,
                                        out,
                                    );
                                }
                            }
                        }
                    }
                }
                scan_expr_arities(
                    std::iter::once(&**recv).chain(args.iter()).collect(),
                    arities,
                    shadowed,
                    knowledge,
                    out,
                );
            }
            Expr::New(pname, args) => {
                // `new Pheno(...)` forwards to the phenotype's `init`
                if let Some((min, max)) = knowledge.method_arity(pname, "init", 0) {
                    let n = args.len();
                    if n < min || n > max {
                        arity_finding(
                            0,
                            &format!("phenotype '{}' constructor (init)", pname),
                            min,
                            max,
                            n,
                            out,
                        );
                    }
                }
                scan_expr_arities(args.iter().collect(), arities, shadowed, knowledge, out);
            }
            Expr::Lambda(g) => {
                // closure bodies are ordinary code: scan them, and let
                // param defaults flow through the expression scan
                let defaults: Vec<&Expr> =
                    g.params.iter().filter_map(|(_, d)| d.as_ref()).collect();
                scan_expr_arities(defaults, arities, shadowed, knowledge, out);
                arity_scan(&g.body, arities, shadowed, knowledge, out);
            }
            Expr::Propagate(a, _) => scan_expr_arities(vec![a], arities, shadowed, knowledge, out),
            Expr::Collect {
                iter, filter, body, ..
            } => {
                let mut parts = vec![&**iter, &**body];
                if let Some(f) = filter {
                    parts.push(f);
                }
                scan_expr_arities(parts, arities, shadowed, knowledge, out);
            }
            Expr::Unary(_, a) => scan_expr_arities(vec![a], arities, shadowed, knowledge, out),
            Expr::Binary(_, a, b, _) => {
                scan_expr_arities(vec![a, b], arities, shadowed, knowledge, out)
            }
            Expr::Index(a, b, _) => {
                scan_expr_arities(vec![a, b], arities, shadowed, knowledge, out)
            }
            Expr::Ternary(a, b, c) => {
                scan_expr_arities(vec![a, b, c], arities, shadowed, knowledge, out)
            }
            Expr::Member(a, _) | Expr::MemberSafe(a, _) => {
                scan_expr_arities(vec![a], arities, shadowed, knowledge, out)
            }
            Expr::List(items) => {
                scan_expr_arities(items.iter().collect(), arities, shadowed, knowledge, out)
            }
            Expr::Map(pairs) => scan_expr_arities(
                pairs.iter().flat_map(|(k, v)| [k, v]).collect(),
                arities,
                shadowed,
                knowledge,
                out,
            ),
            Expr::Interp(parts) => {
                for p in parts {
                    if let crate::ast::InterpPart::Expr(x) = p {
                        scan_expr_arities(vec![x], arities, shadowed, knowledge, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1".to_string()
    } else {
        format!("{}", n)
    }
}

// ------------------------------------------------------------ W66: .cell schema

/// Exact keys consumed by the engine (generated sweep of `cell.get` call sites;
/// source of truth noted in docs/specs/CELL-SCHEMA.md).
pub const CELL_KEYS: &[&str] = &[
    "cli.variant",
    "enhance.delta",
    "entry",
    "expression.koff",
    "expression.kon",
    "expression.seed",
    "grn.decay",
    "grn.decay_calls",
    "methylate.threshold",
    "m6a.reader.decay",
    "m6a.reader.min_level",
    "m6a.reader.translation",
    "quorum.dilution",
    "rho.catch",
    "rho.queue_floor",
    "rho.termination",
    "ribosome.drain",
    "ribosome.queue_cap",
    "repressi.alpha",
    "repressi.basal",
    "repressi.gamma",
    "repressi.hill",
    "repressi.noise",
    "repressi.seed",
    "run.timeout_ms",
];

/// Dynamic key families: `allow.read`, `allow.write`, `allow.net`, ...
pub const CELL_KEY_PREFIXES: &[&str] = &["allow."];

/// Keys whose values must parse as a number.
pub const CELL_NUMERIC_KEYS: &[&str] = &[
    "enhance.delta",
    "expression.koff",
    "expression.kon",
    "expression.seed",
    "grn.decay",
    "grn.decay_calls",
    "methylate.threshold",
    "m6a.reader.decay",
    "m6a.reader.min_level",
    "quorum.dilution",
    "rho.catch",
    "rho.queue_floor",
    "ribosome.drain",
    "ribosome.queue_cap",
    "repressi.alpha",
    "repressi.basal",
    "repressi.gamma",
    "repressi.hill",
    "repressi.noise",
    "repressi.seed",
    "run.timeout_ms",
];

/// W66: validate a .cell payload; findings are advisory (unknown key = warning).
pub fn lint_cell(cell_src: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let map = genes::parse_cell(cell_src);
    let mut keys: Vec<(&String, &String)> = map.iter().collect();
    keys.sort();
    for (k, v) in keys {
        let known =
            CELL_KEYS.contains(&k.as_str()) || CELL_KEY_PREFIXES.iter().any(|p| k.starts_with(p));
        if !known {
            out.push(Finding::new(
                0,
                "cell-unknown-key",
                Sev::Warning,
                format!(
                    ".cell key '{}' is not in the schema (docs/specs/CELL-SCHEMA.md), \
                     typo? it will be silently ignored",
                    k
                ),
            ));
            continue;
        }
        if CELL_NUMERIC_KEYS.contains(&k.as_str()) && v.trim().parse::<f64>().is_err() {
            out.push(Finding::new(
                0,
                "cell-type-mismatch",
                Sev::Warning,
                format!(".cell key '{}' expects a number, got '{}'", k, v),
            ));
        }
    }
    out
}

// ------------------------------------------------------------ tree helpers

fn stmt_line(st: &Stmt) -> Option<usize> {
    // A13 spans: Expr::Call/ExprStmt carry line; statements mostly derive from
    // their expressions, v1 uses the expression lines that exist.
    match st {
        Stmt::ExprStmt(e) => expr_line(e),
        Stmt::Return(Some(e)) => expr_line(e),
        _ => None,
    }
}

fn expr_line(e: &Expr) -> Option<usize> {
    match e {
        Expr::Call(_, _, l) | Expr::Index(_, _, l) => Some(*l),
        Expr::Binary(_, _, _, l) => Some(*l),
        _ => None,
    }
}

fn nested_stmts(st: &Stmt) -> Option<&[Stmt]> {
    match st {
        Stmt::If(arms, els) => {
            // v1: scan the first arm + else, full multi-arm traversal below
            if let Some((_, b)) = arms.first() {
                return Some(b);
            }
            els.as_deref()
        }
        Stmt::While(_, b) | Stmt::Loop(b) | Stmt::For(_, _, b) => Some(b),
        Stmt::Block(b) => Some(b),
        Stmt::Tad(_, b) => Some(b),
        Stmt::Gene(g) | Stmt::Seq(g) => Some(&g.body),
        _ => None,
    }
}

/// Complete nesting: every statement that holds statement bodies, all of
/// them (If: ALL arms + else; Match arms; frames; stress/rescue; defs).
/// The legacy nested_stmts stays for the passes that predate it
/// (unreachable_scan, duplicate_match_arms); new passes use this one.
/// Exhaustive on purpose — a new Stmt variant must be triaged here.
fn stmt_nested_all(st: &Stmt) -> Vec<&[Stmt]> {
    match st {
        Stmt::If(arms, els) => {
            let mut out: Vec<&[Stmt]> = arms.iter().map(|(_, b)| &b[..]).collect();
            if let Some(e) = els {
                out.push(e);
            }
            out
        }
        Stmt::While(_, b)
        | Stmt::Loop(b)
        | Stmt::Scope(b)
        | Stmt::Block(b)
        | Stmt::Tad(_, b)
        | Stmt::For(_, _, b)
        | Stmt::ForPat(_, _, b) => vec![b.as_slice()],
        Stmt::Frame { body, .. } => vec![body.as_slice()],
        Stmt::Match(_, arms, _) => arms.iter().map(|(_, b)| &b[..]).collect(),
        Stmt::Stress { body, rescue, .. } => {
            let mut out = vec![body.as_slice()];
            if let Some((_, rb)) = rescue {
                out.push(rb.as_slice());
            }
            out
        }
        Stmt::Gene(g) | Stmt::Seq(g) => vec![&g.body[..]],
        Stmt::Pheno(p) => p.methods.iter().map(|m| &m.body[..]).collect(),
        Stmt::Trait(t) => t
            .methods
            .iter()
            .filter_map(|m| m.default.as_ref())
            .map(|d| &d.body[..])
            .collect(),
        Stmt::Splice(s) => s.variants.iter().map(|(_, g)| &g.body[..]).collect(),
        _ => vec![],
    }
}

fn walk_all<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Stmt, usize)) {
    for (i, st) in stmts.iter().enumerate() {
        let line = i + 1; // stable, only used for def-ordering hints
        f(st, line);
        for nested in stmt_nested_all(st) {
            walk_all(nested, f);
        }
    }
}

fn collect_calls_stmts(stmts: &[Stmt], called: &mut HashSet<String>) {
    for st in stmts {
        // W48 corpus law: the bio layer references genes by NAME in
        // declarative statements — regulate edges, toggle alleles, operon
        // units and members, silence sites, decoy targets, repressilator
        // nodes, enhance marks. Those operands ARE uses (the runtime gates
        // the named gene even when this file never calls it); ignoring them
        // misfired unused-gene across the reg-bio test corpus.
        match st {
            Stmt::Regulate(edges, trans, binds) => {
                for e in edges {
                    called.insert(e.from.clone());
                    called.insert(e.to.clone());
                }
                for t in trans {
                    called.insert(t.from.clone());
                    called.insert(t.to.clone());
                }
                for b in binds {
                    called.insert(b.tf.clone());
                    called.insert(b.ligand.clone());
                }
            }
            Stmt::Silence(old, new, _, _) => {
                called.insert(old.clone());
                if let Some(n) = new {
                    called.insert(n.clone());
                }
            }
            Stmt::Operon(unit, members) => {
                called.insert(unit.clone());
                for (m, _) in members {
                    called.insert(m.clone());
                }
            }
            Stmt::Toggle(a, b) => {
                called.insert(a.clone());
                called.insert(b.clone());
            }
            Stmt::Decoy(d, tf, _) => {
                called.insert(d.clone());
                called.insert(tf.clone());
            }
            Stmt::Repressilator(nodes, _, _) => {
                for n in nodes {
                    called.insert(n.clone());
                }
            }
            Stmt::Enhance(names) => {
                for n in names {
                    called.insert(n.clone());
                }
            }
            _ => {}
        }
        for e in stmt_exprs_all(st) {
            collect_calls_expr(e, called);
        }
        if let Stmt::Match(_, arms, _) = st {
            for (_, body) in arms {
                collect_calls_stmts(body, called);
            }
        }
        for nested in stmt_nested_all(st) {
            collect_calls_stmts(nested, called);
        }
    }
}

fn collect_calls_expr(e: &Expr, called: &mut HashSet<String>) {
    match e {
        Expr::Call(c, args, _) => {
            if let Expr::Ident(n) = &**c {
                called.insert(n.clone());
            } else if let Expr::Member(_, m) = &**c {
                called.insert(m.clone());
            }
            collect_calls_expr(c, called);
            for a in args {
                collect_calls_expr(a, called);
            }
        }
        Expr::Method(recv, mname, args, _) | Expr::MethodSafe(recv, mname, args, _) => {
            // the method name and the dotted `recv.name` form both count as
            // references (module-member calls are the dotted shape)
            called.insert(mname.clone());
            if let Expr::Ident(v) = &**recv {
                called.insert(format!("{}.{}", v, mname));
            }
            collect_calls_expr(recv, called);
            for a in args {
                collect_calls_expr(a, called);
            }
        }
        Expr::Unary(_, a) => collect_calls_expr(a, called),
        Expr::Binary(_, a, b, _) => {
            collect_calls_expr(a, called);
            collect_calls_expr(b, called);
        }
        Expr::Index(a, b, _) => {
            collect_calls_expr(a, called);
            collect_calls_expr(b, called);
        }
        Expr::Ternary(a, b, c) => {
            collect_calls_expr(a, called);
            collect_calls_expr(b, called);
            collect_calls_expr(c, called);
        }
        Expr::Member(a, _) | Expr::MemberSafe(a, _) => collect_calls_expr(a, called),
        Expr::List(items) => {
            for i in items {
                collect_calls_expr(i, called);
            }
        }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_calls_expr(k, called);
                collect_calls_expr(v, called);
            }
        }
        Expr::Interp(parts) => {
            for p in parts {
                if let crate::ast::InterpPart::Expr(x) = p {
                    collect_calls_expr(x, called);
                }
            }
        }
        Expr::Lambda(g) => {
            // a gene called only from a closure body IS called
            collect_calls_stmts(&g.body, called);
        }
        Expr::New(_, args) => {
            for a in args {
                collect_calls_expr(a, called);
            }
        }
        Expr::Propagate(a, _) => collect_calls_expr(a, called),
        Expr::Collect {
            iter, filter, body, ..
        } => {
            collect_calls_expr(iter, called);
            if let Some(f) = filter {
                collect_calls_expr(f, called);
            }
            collect_calls_expr(body, called);
        }
        _ => {}
    }
}

// ------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn lint_src(src: &str) -> Vec<Finding> {
        lint(&parser::parse(src))
    }

    fn has_rule(f: &[Finding], rule: &str) -> bool {
        f.iter().any(|x| x.rule == rule)
    }

    #[test]
    fn wrong_arity_too_few() {
        let f = lint_src("gene add(a, b) {\n    return a + b\n}\nlet x = add(1)\n");
        assert!(has_rule(&f, "wrong-arity"), "{:?}", f);
    }

    #[test]
    fn arity_ok_not_flagged() {
        let f = lint_src("gene add(a, b) {\n    return a + b\n}\nlet x = add(1, 2)\n");
        assert!(!has_rule(&f, "wrong-arity"), "{:?}", f);
    }

    #[test]
    fn arity_defaults_extend_max() {
        let f = lint_src("gene g(a, b, c) {\n    return a\n}\nlet x = g(1)\n");
        assert!(has_rule(&f, "wrong-arity"));
    }

    #[test]
    fn unused_gene_flagged() {
        // a main gene makes this file a program; library files (no main) are
        // exempt, see the W048 corpus law in lint()
        let f = lint_src("gene lonely() {\n    return 1\n}\nmain {\n    print(1)\n}\n");
        assert!(has_rule(&f, "unused-gene"), "{:?}", f);
    }

    #[test]
    fn main_never_flagged_unused() {
        let f = lint_src("gene main() {\n    return 1\n}\n");
        assert!(!has_rule(&f, "unused-gene"), "{:?}", f);
    }

    #[test]
    fn constant_condition_flagged() {
        let f = lint_src("if 1 { } else { }");
        assert!(has_rule(&f, "constant-condition"), "{:?}", f);
    }

    #[test]
    fn unreachable_after_return() {
        let f = lint_src("gene g() {\n    return 1\n    promote(\"dead\")\n}\ng()");
        assert!(has_rule(&f, "unreachable-code"), "{:?}", f);
    }

    #[test]
    fn unknown_cell_key() {
        let f = lint_cell("methylate.threshold = 2\nbogus.key = 1\n");
        assert!(has_rule(&f, "cell-unknown-key"), "{:?}", f);
        assert!(!has_rule(&f, "cell-type-mismatch"), "{:?}", f);
    }

    #[test]
    fn cell_type_mismatch() {
        let f = lint_cell("methylate.threshold = abc\n");
        assert!(has_rule(&f, "cell-type-mismatch"), "{:?}", f);
    }

    #[test]
    fn allow_prefix_accepted() {
        let f = lint_cell("allow.read = /tmp\n");
        assert!(f.is_empty(), "{:?}", f);
    }

    #[test]
    fn shadowed_name_not_arity_checked() {
        // local binding named `add` shadows the gene, must stay silent
        let f = lint_src(
            "gene add(a, b) {\n    return a + b\n}\ngene g() {\n    let add = 3\n    return add(1)\n}\ng()\n",
        );
        assert!(!has_rule(&f, "wrong-arity"), "{:?}", f);
    }

    // ------------------------------------------------------------ W048

    #[test]
    fn streams_partition_the_rules() {
        // the split is by rule, not by severity: wrong-arity (E) and
        // const-reassign (W) belong to check; the W-severity quality rules
        // belong to lint
        let src = "gene add(a, b) {\n    return a + b\n}\nconst k = 1\nif 1 { }\nlet dead = 7\nk = 2\nadd(1)\n";
        let all = lint_src(src);
        assert!(has_rule(&all, "wrong-arity"), "{:?}", all);
        assert!(has_rule(&all, "const-reassign"), "{:?}", all);
        assert!(has_rule(&all, "constant-condition"), "{:?}", all);
        let check = filter_stream(all.clone(), Stream::Check);
        assert!(has_rule(&check, "wrong-arity"));
        assert!(has_rule(&check, "const-reassign"));
        assert!(!has_rule(&check, "constant-condition"), "{:?}", check);
        let style = filter_stream(all, Stream::Lint);
        assert!(has_rule(&style, "constant-condition"));
        assert!(!has_rule(&style, "wrong-arity"), "{:?}", style);
        assert!(!has_rule(&style, "const-reassign"), "{:?}", style);
        assert_eq!(lint_correctness(&parser::parse(src)), check);
        assert_eq!(lint_style(&parser::parse(src)), style);
    }

    #[test]
    fn shadowed_binding_deliberately_silent() {
        // same-block re-`let` is idiomatic (see the N12 comment above), so
        // the rule is reserved, not emitted: pinned here so a future
        // liveness analysis makes the decision consciously, not by accident
        let f = lint_src("let x = 1\nlet x = 2\nprint(x)\n");
        assert!(!has_rule(&f, "shadowed-binding"), "{:?}", f);
        // separate blocks (if arms): never interact either way
        let f2 = lint_src("if true {\n    let y = 1\n} else {\n    let y = 2\n}\n");
        assert!(!has_rule(&f2, "shadowed-binding"), "{:?}", f2);
    }

    #[test]
    fn const_reassign_in_stress_is_silent() {
        // explicit containment probe: assignment inside stress stays silent
        let f = lint_src(
            "const k = 1\nstress {\n    k = 2\n} rescue (e) {\n    print(e.kind)\n}\nprint(k)\n",
        );
        assert!(!has_rule(&f, "const-reassign"), "{:?}", f);
        // outside stress it still fires
        let f2 = lint_src("const k = 1\nk = 2\n");
        assert!(has_rule(&f2, "const-reassign"), "{:?}", f2);
    }

    #[test]
    fn effect_bindings_are_not_dead_stores() {
        // the initializer calls something: the binding may exist for the call
        let f =
            lint_src("gene make() {\n    return 1\n}\nlet h = make()\nmain {\n    print(1)\n}\n");
        assert!(!has_rule(&f, "unused-binding"), "{:?}", f);
        // a pure dead store is still reported
        let f2 = lint_src("let dead = 7\nmain {\n    print(1)\n}\n");
        assert!(has_rule(&f2, "unused-binding"), "{:?}", f2);
    }

    #[test]
    fn proof_frame_usage_retires_unused_gene() {
        let f = lint_src(
            "gene helper() {\n    return 21\n}\nframe proof {\n    assert(helper() == 21, \"ok\")\n}\n",
        );
        assert!(!has_rule(&f, "unused-gene"), "{:?}", f);
    }

    #[test]
    fn library_file_without_main_is_exempt() {
        // no main gene: library module (W24 default-open), def-side silent
        let f = lint_src("gene lonely() {\n    return 1\n}\nlet dead = 7\n");
        assert!(!has_rule(&f, "unused-gene"), "{:?}", f);
        assert!(!has_rule(&f, "unused-binding"), "{:?}", f);
    }

    #[test]
    fn cli_allow_drops_rules_by_name_and_code() {
        let mut f = lint_src("let dead = 7\n");
        apply_cli_allows(&mut f, &["unused-binding".to_string()]);
        assert!(f.is_empty(), "{:?}", f);
        let mut f2 = lint_src("let dead = 7\n");
        apply_cli_allows(&mut f2, &["W07".to_string()]);
        assert!(f2.is_empty(), "{:?}", f2);
    }

    // ---------------------------------------------- W06 unreachable-match-arm

    fn w06_hits(src: &str) -> Vec<Finding> {
        lint_src(src)
            .into_iter()
            .filter(|x| x.rule == "unreachable-match-arm")
            .collect()
    }

    #[test]
    fn unreachable_arm_after_unguarded_catchall() {
        let f = w06_hits(
            "match 1 {\n    case _ {\n        print(1)\n    }\n    case 1 {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "{:?}", f);
        assert_eq!(f[0].code, "W06", "stable code never renumbers");
        assert_eq!(f[0].sev, Sev::Warning);
        assert!(f[0].message.contains("unguarded catch-all"), "{:?}", f[0]);
        assert_eq!(
            f[0].line, 6,
            "the finding points at the arm body's first line-bearing statement"
        );
    }

    #[test]
    fn binding_pattern_is_a_catchall_too() {
        // `case x` binds every value, so the arms after it are dead (this is
        // also what unknown capitalized tags degrade to at parse time)
        let f = w06_hits(
            "match 1 {\n    case x {\n        print(1)\n    }\n    case 1 {\n        print(2)\n    }\n    case 2 {\n        print(3)\n    }\n}\n",
        );
        assert_eq!(f.len(), 2, "both trailing arms are dead: {:?}", f);
    }

    #[test]
    fn guarded_catchall_subsumes_nothing() {
        // a guard can miss, so every arm after a guarded arm stays live
        let f = w06_hits(
            "match 1 {\n    case x if x > 5 {\n        print(1)\n    }\n    case 1 {\n        print(2)\n    }\n}\n",
        );
        assert!(f.is_empty(), "{:?}", f);
    }

    #[test]
    fn guarded_arm_after_catchall_is_still_dead() {
        // first-match-wins: the earlier unguarded `_` always hits, so the
        // later guard never even evaluates (the guard is on the DEAD side)
        let f = w06_hits(
            "match 1 {\n    case _ {\n        print(1)\n    }\n    case x if x > 5 {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "{:?}", f);
        assert!(f[0].message.contains("unguarded catch-all"), "{:?}", f[0]);
    }

    #[test]
    fn or_pattern_catchall_and_literal_or() {
        // an or-pattern CONTAINING a catch-all is a catch-all
        let f = w06_hits(
            "match 1 {\n    case 1 | _ {\n        print(1)\n    }\n    case 2 {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "{:?}", f);
        // a literal-only or-pattern is not: the 9 arm stays live
        let f2 = w06_hits(
            "match 9 {\n    case 1 | 2 {\n        print(1)\n    }\n    case 9 {\n        print(2)\n    }\n}\n",
        );
        assert!(f2.is_empty(), "{:?}", f2);
    }

    #[test]
    fn strictly_broader_variant_payload() {
        // Some(_) covers every Some payload; the None arm is a different
        // family and must stay live
        let f = w06_hits(
            "match some(1) {\n    case Some(_) {\n        print(1)\n    }\n    case Some(x) {\n        print(2)\n    }\n    case None {\n        print(3)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "the None arm stays live: {:?}", f);
        assert!(f[0].message.contains("broader arm first"), "{:?}", f[0]);
        assert!(f[0].message.contains("arm 1"), "{:?}", f[0]);
    }

    #[test]
    fn tag_only_variant_subsumes_payload_arms() {
        // bare `Some` matches the tag with ANY payload (payload-less values
        // included), so Some(x) after it is dead; the reverse order stays
        // live: the runtime misses a payload pattern against a payload-less
        // value, so Some(x) cannot prove coverage of the tag-only form
        let f = w06_hits(
            "match some(9) {\n    case Some {\n        print(1)\n    }\n    case Some(x) {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "{:?}", f);
        let f2 = w06_hits(
            "match some(9) {\n    case Some(x) {\n        print(1)\n    }\n    case Some {\n        print(2)\n    }\n}\n",
        );
        assert!(f2.is_empty(), "{:?}", f2);
    }

    #[test]
    fn nested_variant_payload_coverage() {
        let f = w06_hits(
            "match some(some(1)) {\n    case Some(Some(_)) {\n        print(1)\n    }\n    case Some(Some(n)) {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "{:?}", f);
        // the outer wildcard covers deeper payloads too
        let f2 = w06_hits(
            "match some(some(1)) {\n    case Some(_) {\n        print(1)\n    }\n    case Some(Some(n)) {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f2.len(), 1, "{:?}", f2);
        // different families never cover each other
        let f3 = w06_hits(
            "match some(1) {\n    case Ok(_) {\n        print(1)\n    }\n    case Some(n) {\n        print(2)\n    }\n}\n",
        );
        assert!(f3.is_empty(), "{:?}", f3);
    }

    #[test]
    fn list_shape_coverage() {
        // [a, b] matches any 2-list, so [1, 2] after it is dead
        let f = w06_hits(
            "match [1, 2] {\n    case [a, b] {\n        print(1)\n    }\n    case [1, 2] {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "{:?}", f);
        // length mismatch: both arms live
        let f2 = w06_hits(
            "match [1] {\n    case [_, _, _] {\n        print(1)\n    }\n    case [1] {\n        print(2)\n    }\n}\n",
        );
        assert!(f2.is_empty(), "{:?}", f2);
        // a *rest earlier arm covers exact shapes at least as long
        let f3 = w06_hits(
            "match [1, 2] {\n    case [1, *t] {\n        print(1)\n    }\n    case [1, 2] {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f3.len(), 1, "{:?}", f3);
        // an exact-length earlier arm cannot cover a *rest later arm (longer
        // lists fall through to the rest arm)
        let f4 = w06_hits(
            "match [1, 2] {\n    case [1, 2] {\n        print(1)\n    }\n    case [1, *t] {\n        print(2)\n    }\n}\n",
        );
        assert!(f4.is_empty(), "{:?}", f4);
    }

    #[test]
    fn map_shape_coverage() {
        // {x} (presence only) covers {x: 1}
        let f = w06_hits(
            "match {x: 1} {\n    case {x} {\n        print(1)\n    }\n    case {x: 1} {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "{:?}", f);
        // {x: 1} does NOT cover {x}: any value at x passes the later arm
        let f2 = w06_hits(
            "match {x: 5} {\n    case {x: 1} {\n        print(1)\n    }\n    case {x} {\n        print(2)\n    }\n}\n",
        );
        assert!(f2.is_empty(), "{:?}", f2);
        // a key the earlier arm never requires leaves the later arm live
        // (a map without y matches it and misses the earlier arm)
        let f3 = w06_hits(
            "match {x: 1} {\n    case {x, y} {\n        print(1)\n    }\n    case {x} {\n        print(2)\n    }\n}\n",
        );
        assert!(f3.is_empty(), "{:?}", f3);
    }

    #[test]
    fn multi_run_literal_covers_later_literal() {
        // legacy comma-run: `1, 2` covers a later `2`; W05 only sees plain
        // literal arms, so this shape is this rule's to report
        let f = w06_hits(
            "match 2 {\n    case 1, 2 {\n        print(1)\n    }\n    case 2 {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "{:?}", f);
    }

    #[test]
    fn identical_plain_literals_defer_to_duplicate_rule() {
        // one symptom, one owner: W05 reports identical literal arms with a
        // first-seen arm number; this rule stays out of that story
        let f = lint_src(
            "match 7 {\n    case 7 {\n        print(1)\n    }\n    case 7 {\n        print(2)\n    }\n}\n",
        );
        assert!(!has_rule(&f, "unreachable-match-arm"), "{:?}", f);
        assert!(has_rule(&f, "duplicate-match-arm"), "{:?}", f);
    }

    #[test]
    fn partially_covered_or_arm_stays_live() {
        // Some(_) covers the Some(1) alternative but not None: the arm as a
        // whole is reachable, so flagging it would be a false positive
        let f = w06_hits(
            "match none() {\n    case Some(_) {\n        print(1)\n    }\n    case Some(1) | None {\n        print(2)\n    }\n}\n",
        );
        assert!(f.is_empty(), "{:?}", f);
    }

    #[test]
    fn nested_offender_reported_once() {
        // regression pin: the pre-stage-2 walk descended arm bodies twice
        // (manual recursion + stmt_nested_all), double-reporting offenders
        // nested inside an arm body
        let f = w06_hits(
            "match 2 {\n    case Some(x) {\n        match 3 {\n            case _ {\n                print(1)\n            }\n            case 3 {\n                print(2)\n            }\n        }\n    }\n    case _ {\n        print(3)\n    }\n}\n",
        );
        assert_eq!(f.len(), 1, "one dead arm, one finding: {:?}", f);
    }

    #[test]
    fn floats_never_subsume() {
        // NaN never deep_eq-equals itself, so an earlier float literal
        // guarantees nothing; the rule stays silent on floats by design
        let f = w06_hits(
            "match 1.5 {\n    case 1.5 {\n        print(1)\n    }\n    case 2.5 {\n        print(2)\n    }\n}\n",
        );
        assert!(f.is_empty(), "{:?}", f);
    }

    #[test]
    fn bytes_keys_are_exact_not_lossy() {
        // b"\xff" and b"\xef\xbf\xbd" (the UTF-8 encoding of U+FFFD) collapse
        // to the same string under from_utf8_lossy; a lossy key would call
        // the second arm unreachable even though deep_eq compares raw bytes
        let f = w06_hits(
            "match some(b\"\\xef\\xbf\\xbd\") {\n    case Some(b\"\\xff\") {\n        print(1)\n    }\n    case Some(b\"\\xef\\xbf\\xbd\") {\n        print(2)\n    }\n}\n",
        );
        assert!(f.is_empty(), "{:?}", f);
        // identical byte payloads still subsume (first-match-wins) at the
        // nested position W05 cannot see
        let f2 = w06_hits(
            "match some(b\"ab\") {\n    case Some(b\"ab\") {\n        print(1)\n    }\n    case Some(b\"ab\") {\n        print(2)\n    }\n}\n",
        );
        assert_eq!(f2.len(), 1, "{:?}", f2);
    }
}
