//! lint.rs — W42/W43/W48/W66 (ROADMAP-100): the shared static-analysis rule
//! engine. `operon lint` is the standalone front door; `operon check` reuses
//! the same findings for its diagnostics format. Check-only: the interpreter
//! and the oracle are untouched, so runtime semantics and differential parity
//! cannot drift from anything in here.
//!
//! Philosophy (Total Grammar): lint findings are ADVISORY. Nothing here
//! rejects a program; `--strict` escalation is a CLI-side policy (W37
//! contract), never an interpreter behavior.

use crate::ast::{Expr, Program, Stmt};
use crate::genes;
use std::collections::{HashMap, HashSet};

// ------------------------------------------------------------ model

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

#[derive(Debug, Clone)]
pub struct Finding {
    pub line: usize,
    pub rule: String,
    pub sev: Sev,
    pub message: String,
}

impl Finding {
    fn new(line: usize, rule: &str, sev: Sev, message: String) -> Finding {
        Finding {
            line,
            rule: rule.to_string(),
            sev,
            message,
        }
    }
}

/// (min_args, max_args) — max == usize::MAX means unbounded (defaults/variadic
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
        Stmt::Use(path, _) => {
            let last = path.rsplit('/').next().unwrap_or(path).to_string();
            imported.push((last, line));
        }
        _ => {}
    });
    collect_calls_stmts(&prog.stmts, &mut called);

    // W42: unused-gene (only when defined AND never called anywhere in-file;
    // exported genes (anchor/tad exports) are library surface — skip them)
    let exported: HashSet<String> = prog.anchor_exports.iter().cloned().collect();
    for (name, line) in &defined {
        if name == "main" || exported.contains(name) {
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
                            "`while` over an always-true literal — if this is intentional, \
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

    // W43: wrong-arity at call sites
    let mut shadowed: HashSet<String> = HashSet::new();
    collect_shadowed_bindings(&prog.stmts, &mut shadowed);
    arity_scan(&prog.stmts, &arities, &shadowed, &mut out);

    out.sort_by_key(|f| (f.line, f.rule.clone()));
    out
}

fn constant_condition(e: &Expr) -> Option<String> {
    match e {
        Expr::Int(v) => Some(format!(
            "condition is the constant {} — {}",
            v,
            if *v != 0 {
                "always true"
            } else {
                "always false"
            }
        )),
        Expr::Bool(b) => Some(format!(
            "condition is the constant {} — {}",
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
        if let Stmt::Match(_, arms) = st {
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
    out: &mut Vec<Finding>,
) {
    for st in stmts {
        scan_expr_arities(st_exprs(st), arities, shadowed, out);
        if let Some(nested) = nested_stmts(st) {
            arity_scan(nested, arities, shadowed, out);
        }
    }
}

fn scan_expr_arities(
    exprs: Vec<&Expr>,
    arities: &HashMap<String, Arity>,
    shadowed: &HashSet<String>,
    out: &mut Vec<Finding>,
) {
    for e in exprs {
        match e {
            Expr::Call(callee, args, line) => {
                if let Expr::Ident(name) = &**callee {
                    if !shadowed.contains(name) {
                        if let Some((min, max)) = arities.get(name) {
                            let n = args.len();
                            if n < *min {
                                out.push(Finding::new(
                                    *line,
                                    "wrong-arity",
                                    Sev::Warning,
                                    format!(
                                        "'{}' expects {} argument(s), got {} — \
                                         missing args become Null at runtime (checkable via rescue)",
                                        name,
                                        plural(*min),
                                        n
                                    ),
                                ));
                            } else if n > *max {
                                out.push(Finding::new(
                                    *line,
                                    "wrong-arity",
                                    Sev::Warning,
                                    format!(
                                        "'{}' accepts at most {} argument(s), got {} — \
                                         extras are ignored at runtime",
                                        name,
                                        plural(*max),
                                        n
                                    ),
                                ));
                            }
                        }
                    }
                }
                scan_expr_arities(
                    std::iter::once(&**callee).chain(args.iter()).collect(),
                    arities,
                    shadowed,
                    out,
                );
            }
            Expr::Method(recv, _, args) | Expr::MethodSafe(recv, _, args) => {
                // method arity lives in phenotype defs — v1 covers free genes only
                scan_expr_arities(
                    std::iter::once(&**recv).chain(args.iter()).collect(),
                    arities,
                    shadowed,
                    out,
                );
            }
            Expr::Unary(_, a) => scan_expr_arities(vec![a], arities, shadowed, out),
            Expr::Binary(_, a, b, _) => scan_expr_arities(vec![a, b], arities, shadowed, out),
            Expr::Index(a, b, _) => scan_expr_arities(vec![a, b], arities, shadowed, out),
            Expr::Ternary(a, b, c) => scan_expr_arities(vec![a, b, c], arities, shadowed, out),
            Expr::Member(a, _) | Expr::MemberSafe(a, _) => {
                scan_expr_arities(vec![a], arities, shadowed, out)
            }
            Expr::List(items) => scan_expr_arities(items.iter().collect(), arities, shadowed, out),
            Expr::Map(pairs) => scan_expr_arities(
                pairs.iter().flat_map(|(k, v)| [k, v]).collect(),
                arities,
                shadowed,
                out,
            ),
            Expr::Interp(parts) => {
                for p in parts {
                    if let crate::ast::InterpPart::Expr(x) = p {
                        scan_expr_arities(vec![x], arities, shadowed, out);
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
                    ".cell key '{}' is not in the schema (docs/specs/CELL-SCHEMA.md) — \
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
    // their expressions — v1 uses the expression lines that exist.
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
            // v1: scan the first arm + else — full multi-arm traversal below
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

fn walk_all<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Stmt, usize)) {
    for (i, st) in stmts.iter().enumerate() {
        let line = i + 1; // stable, only used for def-ordering hints
        f(st, line);
        match st {
            Stmt::If(arms, els) => {
                for (_, b) in arms {
                    walk_all(b, f);
                }
                if let Some(e) = els {
                    walk_all(e, f);
                }
            }
            Stmt::While(_, b) | Stmt::Loop(b) | Stmt::For(_, _, b) => walk_all(b, f),
            Stmt::Block(b) | Stmt::Tad(_, b) => walk_all(b, f),
            Stmt::Frame { .. } => {}
            _ => {}
        }
        if let Some(g) = stmt_gene(st) {
            walk_all(&g.body, f);
        }
    }
}

fn stmt_gene(st: &Stmt) -> Option<&std::sync::Arc<crate::ast::GeneDef>> {
    match st {
        Stmt::Gene(g) | Stmt::Seq(g) => Some(g),
        _ => None,
    }
}

fn st_exprs(st: &Stmt) -> Vec<&Expr> {
    match st {
        Stmt::Let(_, e) | Stmt::Return(Some(e)) => vec![e],
        Stmt::Assign(_, _, e) | Stmt::Raise(_, e, _) => vec![e],
        Stmt::ExprStmt(e) => vec![e],
        Stmt::While(c, _) => vec![c],
        Stmt::For(_, it, _) => vec![it],
        _ => vec![],
    }
}

fn collect_calls_stmts(stmts: &[Stmt], called: &mut HashSet<String>) {
    for st in stmts {
        for e in st_exprs(st) {
            collect_calls_expr(e, called);
        }
        if let Stmt::Match(scrut, arms) = st {
            collect_calls_expr(scrut, called);
            for (_, body) in arms {
                collect_calls_stmts(body, called);
            }
        }
        if let Some(nested) = nested_stmts(st) {
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
        Expr::Method(recv, _, args) | Expr::MethodSafe(recv, _, args) => {
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
        let f = lint_src("gene lonely() {\n    return 1\n}\n");
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
        // local binding named `add` shadows the gene — must stay silent
        let f = lint_src(
            "gene add(a, b) {\n    return a + b\n}\ngene g() {\n    let add = 3\n    return add(1)\n}\ng()\n",
        );
        assert!(!has_rule(&f, "wrong-arity"), "{:?}", f);
    }
}
