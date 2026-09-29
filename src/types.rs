//! types.rs — W01-s2 (static type system stage 2): the compile-time type
//! checker. Companion to the W01 stage-1 soft annotations (SPEC §7a): stage
//! 1 catches violations at RUNTIME as recoverable `unfolded` Stress, stage
//! 2 catches them BEFORE the program runs, as ordinary check findings on
//! the same stable-code ladder.
//!
//! Philosophy (unchanged from W01 + Total Grammar):
//! - **Additive, never destructive.** The dynamic semantics are untouched.
//!   Unannotated code checks nothing (its types flow as `Unknown`), a gene
//!   with annotations gets its boundaries verified, `operon check` reports
//!   everything and `operon run` keeps running exactly as before.
//! - **Conservative by construction.** A finding fires only when the
//!   checker can PROVE the runtime would take the corresponding soft hit
//!   (a mismatched argument at an annotated boundary, a method a receiver
//!   family cannot have, a non-exhaustive optional/result match). The
//!   checker reuses the runtime's own matching rules (`accepts` mirrors
//!   `interp::ann_matches`), so checker and runtime cannot drift apart on
//!   what counts as a violation.
//! - **No parse rejections.** Malformed annotations degraded to `any` at
//!   parse time; unknown type names are a warning here, not an error.
//!
//! Inference: a bottom-up Hindley-Milner-flavored pass over each gene body
//! (literals, operators, calls, indexing, methods, ternaries, collects),
//! with inference variables resolved by unification. Unannotated genes
//! still get their bodies walked, so the classic dynamic bug
//!
//!   x = 10
//!   x.name()
//!
//! is caught at check time (int has no methods), while fully dynamic code
//! stays finding-free.
//!
//! What this module is NOT: a type-directed optimizer, a monomorphizer,
//! or a runtime change. The VM and interpreter are untouched; findings are
//! `lint::Finding`s on the check stream (`operon check`), suppressed by
//! the same `// allow:` mechanism as every other rule.

use crate::ast::*;
use crate::lint::{rule_code, Finding, Sev};
use std::collections::HashMap;
use std::rc::Rc;

// ------------------------------------------------------------ checker Ty

/// The checker's type lattice. Deliberately parallel to the runtime value
/// families (`Value::type_name()`) plus optionality and results, so the
/// mapping `ty_of_ann` / `accepts` stays a thin, auditable bridge.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    /// No information (unannotated, uninferable). Compatible with
    /// everything; never produces a finding.
    Unknown,
    /// The `any` annotation. Compatible with everything.
    Any,
    Null,
    Bool,
    Int,
    Float,
    Str,
    Bytes,
    List(Rc<Ty>),
    Map(Rc<Ty>, Rc<Ty>),
    Gene,
    Seq,
    /// A phenotype instance (user class), named for method/field lookup.
    Pheno(String),
    /// `T?` / `Option<T>`.
    Opt(Rc<Ty>),
    /// `Result<T, E>`.
    Res(Rc<Ty>, Rc<Ty>),
    /// A union `a | b`; also the widening result of a name bound twice.
    Union(Vec<Ty>),
    /// An inference variable, resolved through `subs`.
    Var(u32),
    /// A generic parameter of the gene under check (`T` in `gene f<T>`).
    /// Per-call instantiation replaces it with a fresh Var.
    GVar(String),
    /// A trait value bound (bounds only; not first-class at runtime).
    Trait(String),
}

impl Ty {
    /// Human rendering for findings, ann-style (must read like source).
    pub fn show(&self) -> String {
        match self {
            Ty::Unknown => "unknown".into(),
            Ty::Any => "any".into(),
            Ty::Null => "null".into(),
            Ty::Bool => "bool".into(),
            Ty::Int => "int".into(),
            Ty::Float => "float".into(),
            Ty::Str => "str".into(),
            Ty::Bytes => "bytes".into(),
            Ty::List(t) => format!("list<{}>", t.show()),
            Ty::Map(k, v) => format!("map<{}, {}>", k.show(), v.show()),
            Ty::Gene => "gene".into(),
            Ty::Seq => "sequence".into(),
            Ty::Pheno(n) => n.clone(),
            Ty::Opt(t) => format!("option<{}>", t.show()),
            Ty::Res(t, e) => format!("result<{}, {}>", t.show(), e.show()),
            Ty::Union(alts) => alts
                .iter()
                .map(|t| t.show())
                .collect::<Vec<_>>()
                .join(" | "),
            Ty::Var(_) => "unknown".into(),
            Ty::GVar(n) => n.clone(),
            Ty::Trait(n) => n.clone(),
        }
    }

    fn is_unknownish(&self) -> bool {
        matches!(self, Ty::Unknown | Ty::Any | Ty::Var(_))
    }
}

// ------------------------------------------------------- ann -> Ty bridge

/// Convert an annotation to the checker lattice. Aliases arrive already
/// resolved (the parser resolves them; a `TypeAnn::Alias` carries its
/// target). Type parameters stay `GVar` until instantiation.
pub fn ty_of_ann(ann: &TypeAnn) -> Ty {
    match ann {
        TypeAnn::Named(n) => ty_of_name(n),
        TypeAnn::Union(alts) => Ty::Union(alts.iter().map(ty_of_ann).collect()),
        TypeAnn::Optional(inner) => Ty::Opt(Rc::new(ty_of_ann(inner))),
        TypeAnn::App { head, args } => match (head.as_str(), args.as_slice()) {
            ("list" | "List" | "seq" | "Seq", [el]) => Ty::List(Rc::new(ty_of_ann(el))),
            ("map" | "Map", [k, v]) => Ty::Map(Rc::new(ty_of_ann(k)), Rc::new(ty_of_ann(v))),
            ("option" | "Option" | "optional" | "opt", [inner]) => {
                Ty::Opt(Rc::new(ty_of_ann(inner)))
            }
            ("result" | "Result", [ok, err]) => {
                Ty::Res(Rc::new(ty_of_ann(ok)), Rc::new(ty_of_ann(err)))
            }
            // unknown head: the runtime degrades to family-name matching;
            // here Unknown means "no findings either way" (the unknown-name
            // warning is emitted separately).
            _ => Ty::Unknown,
        },
        TypeAnn::TypeVar(n) => Ty::GVar(n.clone()),
        TypeAnn::Alias { target, .. } => ty_of_ann(target),
    }
}

fn ty_of_name(n: &str) -> Ty {
    match n {
        "any" => Ty::Any,
        "null" => Ty::Null,
        "bool" => Ty::Bool,
        "int" => Ty::Int,
        "float" => Ty::Float,
        "str" | "string" => Ty::Str,
        "bytes" => Ty::Bytes,
        "list" => Ty::List(Rc::new(Ty::Unknown)),
        "map" => Ty::Map(Rc::new(Ty::Unknown), Rc::new(Ty::Unknown)),
        "gene" | "fn" | "function" => Ty::Gene,
        "sequence" | "generator" => Ty::Seq,
        "option" | "optional" => Ty::Opt(Rc::new(Ty::Unknown)),
        "result" => Ty::Res(Rc::new(Ty::Unknown), Rc::new(Ty::Unknown)),
        "phenotype" | "class" | "struct" | "record" | "prototype" => Ty::Pheno("phenotype".into()),
        other => Ty::Pheno(other.into()),
    }
}

// ------------------------------------------------------------ matching

/// The checker's mirror of `interp::ann_matches`: does `actual` satisfy
/// `expected`? The TWO documented relaxations carry over verbatim (`any`
/// accepts everything; `float` accepts int while `int` refuses float —
/// no silent narrowing). Unresolved sides accept everything, so the
/// checker only speaks when it is sure.
fn accepts(expected: &Ty, actual: &Ty) -> bool {
    match (expected, actual) {
        _ if expected.is_unknownish() || actual.is_unknownish() => true,
        (Ty::GVar(_), _) | (_, Ty::GVar(_)) => true, // bounds checked apart
        (Ty::Trait(_), _) | (_, Ty::Trait(_)) => true,
        // widening: int is acceptable where float is declared
        (Ty::Float, Ty::Int) => true,
        // an optional accepts null first, then whatever the inner accepts
        // (an actual Opt unwraps to its inner, a plain value compares as-is)
        (Ty::Opt(inner), other) => match other {
            Ty::Null => true,
            Ty::Opt(a_inner) => accepts(inner, a_inner),
            plain => accepts(inner, plain),
        },
        // expected union: any alternative satisfies (runtime rule)
        (Ty::Union(alts), other) => alts.iter().any(|a| accepts(a, other)),
        // actual union: EVERY member must be accepted to be sure
        // (per-member dispatch re-enters, so Opt members see Null members)
        (expected, Ty::Union(members)) => members.iter().all(|m| accepts(expected, m)),
        (Ty::List(e), Ty::List(a)) => accepts(e, a),
        (Ty::Map(ke, ve), Ty::Map(ka, va)) => accepts(ke, ka) && accepts(ve, va),
        (Ty::Res(t1, e1), Ty::Res(t2, e2)) => accepts(t1, t2) && accepts(e1, e2),
        (Ty::Pheno(n), Ty::Pheno(m)) => n == m || n == "phenotype",
        (a, b) => a == b,
    }
}

// ---------------------------------------------------------- substitutions

/// Inference-variable substitution table (union-find-lite).
#[derive(Default)]
struct Subs {
    map: Vec<Option<Ty>>,
}

impl Subs {
    fn fresh(&mut self) -> Ty {
        self.map.push(None);
        Ty::Var((self.map.len() - 1) as u32)
    }

    fn resolve(&self, ty: &Ty) -> Ty {
        let mut cur = ty.clone();
        let mut hops = 0usize;
        while let Ty::Var(i) = cur {
            match self.map.get(i as usize).and_then(|x| x.as_ref()) {
                Some(next) if hops < 256 => {
                    cur = next.clone();
                    hops += 1;
                }
                _ => return Ty::Var(i),
            }
        }
        cur
    }

    fn bind(&mut self, i: u32, ty: Ty) {
        if let Some(slot) = self.map.get_mut(i as usize) {
            if slot.is_none() {
                *slot = Some(ty);
            }
        }
    }

    /// Structural unification, widening int+float to float, absorbing
    /// Null into an optional. Returns the unified type, or None on a
    /// definite conflict.
    fn unify(&mut self, a: &Ty, b: &Ty) -> Option<Ty> {
        let a = self.resolve(a);
        let b = self.resolve(b);
        match (&a, &b) {
            // SAME variable: identity. The Var arms MUST precede the
            // unknownish guards: a Var is "unknownish" (it accepts
            // anything in `accepts`) but here it must BIND, never
            // short-circuit, or inference variables never resolve.
            (Ty::Var(i), Ty::Var(j)) if i == j => return Some(a),
            (Ty::Var(i), _) => {
                self.bind(*i, b.clone());
                return Some(b);
            }
            (_, Ty::Var(i)) => {
                self.bind(*i, a.clone());
                return Some(a);
            }
            _ if a.is_unknownish() => return Some(b),
            _ if b.is_unknownish() => return Some(a),
            (Ty::GVar(_), _) | (_, Ty::GVar(_)) => return Some(a),
            (Ty::Trait(_), _) | (_, Ty::Trait(_)) => return Some(a),
            (Ty::Null, Ty::Opt(x)) | (Ty::Opt(x), Ty::Null) => return Some(Ty::Opt(x.clone())),
            (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => return Some(Ty::Float),
            (Ty::List(x), Ty::List(y)) => return Some(Ty::List(Rc::new(self.unify(x, y)?))),
            (Ty::Map(k1, v1), Ty::Map(k2, v2)) => {
                let k = self.unify(k1, k2)?;
                let v = self.unify(v1, v2)?;
                return Some(Ty::Map(Rc::new(k), Rc::new(v)));
            }
            (Ty::Opt(x), Ty::Opt(y)) => return Some(Ty::Opt(Rc::new(self.unify(x, y)?))),
            (Ty::Res(t1, e1), Ty::Res(t2, e2)) => {
                let t = self.unify(t1, t2)?;
                let e = self.unify(e1, e2)?;
                return Some(Ty::Res(Rc::new(t), Rc::new(e)));
            }
            (Ty::Union(xs), other) | (other, Ty::Union(xs)) => {
                let mut out: Vec<Ty> = Vec::new();
                for x in xs.clone() {
                    let u = self.unify(&x, other)?;
                    if !out.contains(&u) {
                        out.push(u);
                    }
                }
                let t = if out.len() == 1 {
                    out.pop().unwrap()
                } else {
                    Ty::Union(out)
                };
                return Some(t);
            }
            _ => {}
        }
        if a == b {
            Some(a)
        } else {
            None
        }
    }
}

// -------------------------------------------------------------- symbols

/// A collected gene signature (declared + pre-converted annotation types).
#[derive(Debug, Clone)]
pub struct GeneSig {
    pub name: String,
    /// (param name, default expression) — the GeneDef shape
    pub params: Vec<(String, Option<Expr>)>,
    /// parallel to params (None where unannotated) — the GeneDef shape
    pub param_anns: Vec<Option<TypeAnn>>,
    pub ret: Option<TypeAnn>,
    pub type_params: Vec<(String, Option<TypeAnn>)>,
    pub line: usize,
    pub seq: bool,
    /// method receiver (phenotype name) when collected from a phenotype
    pub recv: Option<String>,
}

/// A collected trait signature (method contracts).
#[derive(Debug, Clone)]
pub struct TraitSig {
    pub name: String,
    pub required: Vec<String>,
    pub defaults: Vec<String>,
    pub line: usize,
}

/// A collected phenotype signature (fields, methods, implemented traits).
#[derive(Debug, Clone, Default)]
pub struct PhenoSig {
    pub name: String,
    pub parent: Option<String>,
    pub implements: Vec<String>,
    /// field name -> declared/inferable type (None when not inferable)
    pub fields: Vec<(String, Option<Ty>)>,
    pub methods: Vec<GeneSig>,
    pub line: usize,
}

/// Everything the checker knows about one file.
#[derive(Debug, Default)]
pub struct Symbols {
    pub genes: HashMap<String, GeneSig>,
    pub phenos: HashMap<String, PhenoSig>,
    pub traits: HashMap<String, TraitSig>,
    /// top-level binding hints (annotated lets + literal lets)
    pub globals: Vec<(String, Ty)>,
}

fn collect_symbols(prog: &Program) -> Symbols {
    let mut syms = Symbols::default();
    collect_stmts_symbols(&prog.stmts, &mut syms);
    for body in &prog.proofs {
        collect_stmts_symbols(body, &mut syms);
    }
    syms
}

fn collect_stmts_symbols(stmts: &[Stmt], syms: &mut Symbols) {
    for st in stmts {
        collect_stmt_symbols(st, syms);
    }
}

fn collect_stmt_symbols(st: &Stmt, syms: &mut Symbols) {
    match st {
        Stmt::Gene(g) | Stmt::Seq(g) => {
            if let Some(n) = &g.name {
                syms.genes.insert(
                    n.clone(),
                    GeneSig {
                        name: n.clone(),
                        params: g.params.clone(),
                        param_anns: g.param_anns.clone(),
                        ret: g.ret_ann.clone(),
                        type_params: g.type_params.clone(),
                        line: g.line,
                        seq: g.seq,
                        recv: None,
                    },
                );
            }
        }
        Stmt::Pheno(p) => {
            let mut fields = Vec::new();
            for (fname, fexpr) in &p.fields {
                fields.push((fname.clone(), literal_ty(fexpr)));
            }
            let mut methods = Vec::new();
            for m in &p.methods {
                if let Some(mn) = &m.name {
                    methods.push(GeneSig {
                        name: mn.clone(),
                        params: m.params.clone(),
                        param_anns: m.param_anns.clone(),
                        ret: m.ret_ann.clone(),
                        type_params: m.type_params.clone(),
                        line: m.line,
                        seq: false,
                        recv: Some(p.name.clone()),
                    });
                }
            }
            syms.phenos.insert(
                p.name.clone(),
                PhenoSig {
                    name: p.name.clone(),
                    parent: p.parent.clone(),
                    implements: p.implements.clone(),
                    fields,
                    methods,
                    line: p.line,
                },
            );
        }
        Stmt::Trait(t) => {
            syms.traits.insert(
                t.name.clone(),
                TraitSig {
                    name: t.name.clone(),
                    required: t
                        .methods
                        .iter()
                        .filter(|m| m.required)
                        .map(|m| m.name.clone())
                        .collect(),
                    defaults: t
                        .methods
                        .iter()
                        .filter(|m| !m.required)
                        .map(|m| m.name.clone())
                        .collect(),
                    line: t.line,
                },
            );
        }
        Stmt::LetAnn(n, ann, e) => {
            syms.globals.push((n.clone(), ty_of_ann(ann)));
            syms.globals
                .push((n.clone(), literal_ty(e).unwrap_or(Ty::Unknown)));
        }
        Stmt::Let(n, e) => {
            if let Some(t) = literal_ty(e) {
                syms.globals.push((n.clone(), t));
            }
        }
        _ => {}
    }
}

/// The type of an obviously-typed expression (used for top-level hints
/// and phenotype field types). Everything else is None (Unknown).
fn literal_ty(e: &Expr) -> Option<Ty> {
    match e {
        Expr::Null => Some(Ty::Null),
        Expr::Bool(_) => Some(Ty::Bool),
        Expr::Int(_) => Some(Ty::Int),
        Expr::Float(_) => Some(Ty::Float),
        Expr::Str(_) | Expr::Interp(_) => Some(Ty::Str),
        Expr::Bytes(_) => Some(Ty::Bytes),
        Expr::List(items) => {
            let mut subs = Subs::default();
            let t = subs.fresh();
            let mut ok = true;
            for it in items {
                let it_ty = literal_ty(it).unwrap_or(Ty::Unknown);
                if subs.unify(&t, &it_ty).is_none() {
                    ok = false;
                    break;
                }
            }
            if ok {
                let el = subs.resolve(&t);
                Some(Ty::List(Rc::new(el)))
            } else {
                None
            }
        }
        _ => None,
    }
}

// ------------------------------------------------------------ diagnostics

/// W01-s2 stable diagnostic codes (appended to the W041 ladder, never
/// renumbered; all ride the CHECK stream — they answer "is this program
/// correct about its types", not "is this code clean").
///
///   E02  type-mismatch            annotated boundary violated at check
///                                 time (the runtime would raise the
///                                 catchable `unfolded` Stress there)
///   E03  no-method                a receiver family with a definite,
///                                 closed method surface has no such
///                                 method (the runtime notes "has no
///                                 method" and yields null)
///   W13  unknown-type-name        annotation names nothing known (typo
///                                 armor: the runtime matches nothing)
///   W14  null-return              a provable null return crosses a
///                                 non-optional return annotation
///   W15  non-exhaustive-match     an optional/result match misses a tag
///                                 with no catch-all (warning per the
///                                 W02 build note; --strict escalates)
///   W16  trait-bound-violation    a call argument cannot satisfy a
///                                 declared `T: Trait` bound
fn finding(line: usize, rule: &str, sev: Sev, message: String) -> Finding {
    Finding {
        line,
        code: rule_code(rule),
        rule: rule.to_string(),
        sev,
        message,
    }
}

impl<'a> Checker<'a> {
    /// Push a boundary finding unless we are inside a stress body (the
    /// corpus probes runtime failures there on purpose — the W48 corpus
    /// law: findings must not fire on intentional code).
    fn boundary_finding(&mut self, line: usize, rule: &str, message: String) {
        if self.in_stress {
            return;
        }
        self.findings.push(finding(line, rule, Sev::Error, message));
    }
}

/// Best-effort source line of a statement (the AST stamps lines on
/// call/index/binary/propagate nodes; statements inherit the first
/// stamped descendant, 0 = file-level per the W041 convention).
fn expr_line(e: &Expr) -> usize {
    match e {
        Expr::Call(_, _, l) | Expr::Index(_, _, l) | Expr::Propagate(_, l) => *l,
        Expr::Binary(_, _, _, l) => *l,
        Expr::Unary(_, a) => expr_line(a),
        Expr::Ternary(a, _, _) => expr_line(a),
        Expr::List(items) => items.iter().map(expr_line).find(|l| *l > 0).unwrap_or(0),
        Expr::Map(items) => items
            .iter()
            .map(|(a, b)| expr_line(a).max(expr_line(b)))
            .find(|l| *l > 0)
            .unwrap_or(0),
        _ => 0,
    }
}

fn stmt_line(st: &Stmt) -> usize {
    match st {
        Stmt::Let(_, e)
        | Stmt::LetConst(_, e)
        | Stmt::LetAnn(_, _, e)
        | Stmt::Assign(_, _, e)
        | Stmt::Return(Some(e))
        | Stmt::ExprStmt(e) => expr_line(e),
        Stmt::IndexAssign(t, i, _, e) => expr_line(t).max(expr_line(i)).max(expr_line(e)),
        Stmt::MemberAssign(t, _, _, e) => expr_line(t).max(expr_line(e)),
        Stmt::For(_, it, _) | Stmt::ForPat(_, it, _) => expr_line(it),
        Stmt::While(c, _) => expr_line(c),
        Stmt::If(arms, _) => arms.first().map(|(c, _)| expr_line(c)).unwrap_or(0),
        Stmt::Match(e, _) => expr_line(e),
        Stmt::Raise(_, e, l) => {
            let _ = e;
            *l
        }
        Stmt::Gene(g) | Stmt::Seq(g) => g.line,
        Stmt::TypeAlias(_, _, l) => *l,
        _ => 0,
    }
}

// ------------------------------------------------------------ method table

/// The closed primitive method surface, per receiver family (mirrors the
/// arms of `Interp::call_method`). A receiver whose family is known AND
/// whose surface is closed (int/float/bool/null/gene accept NOTHING) makes
/// a missing method a definite finding; list/map/str/bytes/seq methods are
/// also closed (an unknown name notes and yields null at runtime).
fn method_ret(recv: &Ty, name: &str) -> Option<Ty> {
    match recv {
        Ty::Str => Some(match name {
            "upper" | "lower" | "trim" | "at" | "join" | "replace" | "repeat" | "slice" => Ty::Str,
            "split" => Ty::List(Rc::new(Ty::Str)),
            "contains" | "starts" | "ends" => Ty::Bool,
            "len" => Ty::Int,
            _ => return None, // closed surface, missing method
        }),
        Ty::List(el) => Some(match name {
            "len" => Ty::Int,
            "push" => recv.clone(),
            "pop" | "get" => Ty::Opt(el.clone()), // null on empty / out of range
            "filter" | "sort" | "reverse" | "slice" => recv.clone(),
            "map" => Ty::List(Rc::new(Ty::Unknown)),
            "reduce" | "each" => Ty::Unknown,
            "contains" => Ty::Bool,
            "index_of" => Ty::Int,
            "join" => Ty::Str,
            _ => return None,
        }),
        Ty::Map(k, v) => Some(match name {
            "keys" => Ty::List(k.clone()),
            "values" => Ty::List(v.clone()),
            "items" => Ty::List(Rc::new(Ty::Unknown)), // (k, v) pair lists
            "has" => Ty::Bool,
            "get" => Ty::Opt(v.clone()), // null on missing, or the default
            "del" => Ty::Null,
            "len" => Ty::Int,
            _ => return None, // the gene-member fallback reads as dynamic
        }),
        Ty::Bytes => Some(match name {
            "slice" => Ty::Bytes,
            "len" => Ty::Int,
            _ => return None,
        }),
        Ty::Seq => Some(match name {
            "next" => Ty::Unknown,
            "collect" => Ty::List(Rc::new(Ty::Unknown)),
            _ => return None,
        }),
        // `x?.k()` safe-call chains: the receiver may be null, so the
        // surface belongs to the INNER value; too dynamic to be definite
        Ty::Opt(_) => Some(Ty::Unknown),
        // closed primitive families: NOTHING is a method here (the runtime
        // notes "int has no method 'x'" and yields null)
        Ty::Int | Ty::Float | Ty::Bool | Ty::Null | Ty::Gene => None,
        _ => Some(Ty::Unknown), // phenotypes resolved by the caller; unions too
    }
}

// ------------------------------------------------------------ environment

#[derive(Debug, Clone)]
struct Binding {
    ty: Ty,
    /// the name was pinned by an annotation (param ann, let ann, const):
    /// assignments keep getting checked against it
    declared: bool,
}

struct Scope {
    vars: Vec<(String, Binding)>,
}

impl Scope {
    fn get(&self, name: &str) -> Option<&Binding> {
        self.vars
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b)
    }
    fn define(&mut self, name: &str, ty: Ty, declared: bool) {
        match self.vars.iter_mut().rev().find(|(n, _)| n == name) {
            Some((_, b)) if !declared && !b.declared => {
                // widen: dynamic re-binding of the same name (union)
                let merged = merge_tys(b.ty.clone(), ty);
                b.ty = merged;
            }
            Some((_, b)) => {
                b.ty = ty;
                b.declared = b.declared || declared;
            }
            None => self.vars.push((name.to_string(), Binding { ty, declared })),
        }
    }
    fn child() -> Scope {
        Scope { vars: Vec::new() }
    }
}

fn merge_tys(a: Ty, b: Ty) -> Ty {
    if a == b {
        return a;
    }
    match a {
        Ty::Unknown => b,
        Ty::Union(mut xs) => {
            if !xs.contains(&b) {
                xs.push(b);
            }
            Ty::Union(xs)
        }
        other => Ty::Union(vec![other, b]),
    }
}

// ------------------------------------------------------------ the checker

pub struct Checker<'a> {
    syms: Symbols,
    subs: Subs,
    findings: Vec<Finding>,
    prog: &'a Program,
    /// alias name -> target ann (for unknown-name resolution + W13)
    aliases: HashMap<String, TypeAnn>,
    /// best-effort source line of the statement under check (the AST does
    /// not span every node; the interpreter uses the same cur_line pattern)
    cur_line: usize,
    /// inside a `stress { }` body: the code EXPECTS runtime failures, so
    /// boundary findings (E02/E03) are intentional probes, not mistakes.
    /// Declaration-level diagnostics (W13/W14/W15) still apply.
    in_stress: bool,
}

impl<'a> Checker<'a> {
    pub fn new(prog: &'a Program) -> Checker<'a> {
        let mut aliases = HashMap::new();
        for (n, t) in &prog.type_aliases {
            aliases.insert(n.clone(), t.clone());
        }
        Checker {
            syms: collect_symbols(prog),
            subs: Subs::default(),
            findings: Vec::new(),
            prog,
            aliases,
            cur_line: 0,
            in_stress: false,
        }
    }

    // ---- name resolution (the W13 typo-armor pass) ----

    fn check_ann_names(&mut self, ann: &TypeAnn, line: usize) {
        match ann {
            TypeAnn::Named(n) => self.check_type_name(n, line),
            TypeAnn::Union(alts) => {
                for a in alts {
                    self.check_ann_names(a, line);
                }
            }
            TypeAnn::Optional(inner) => self.check_ann_names(inner, line),
            TypeAnn::App { head, args } => {
                self.check_type_name(head, line);
                for a in args {
                    self.check_ann_names(a, line);
                }
            }
            // a TypeVar is declared at its gene; the enclosing gene's own
            // list was validated when the signature was checked
            TypeAnn::TypeVar(_) => {}
            TypeAnn::Alias { name, target } => {
                if !self.aliases.contains_key(name) {
                    // parse-time resolution failed (declared after use?)
                    self.findings.push(finding(
                        line,
                        "unknown-type-name",
                        Sev::Warning,
                        format!("type alias '{}' is not declared in this file", name),
                    ));
                }
                self.check_ann_names(target, line);
            }
        }
    }

    fn check_type_name(&mut self, n: &str, line: usize) {
        let known = matches!(
            n,
            "any"
                | "null"
                | "bool"
                | "int"
                | "float"
                | "str"
                | "string"
                | "bytes"
                | "list"
                | "List"
                | "map"
                | "Map"
                | "gene"
                | "fn"
                | "function"
                | "sequence"
                | "generator"
                | "seq"
                | "Seq"
                | "option"
                | "Option"
                | "optional"
                | "opt"
                | "result"
                | "Result"
                | "phenotype"
                | "class"
                | "struct"
                | "record"
                | "prototype"
        ) || self.aliases.contains_key(n)
            || self.syms.phenos.contains_key(n)
            || self.syms.traits.contains_key(n);
        if !known {
            self.findings.push(finding(
                line,
                "unknown-type-name",
                Sev::Warning,
                format!(
                    "annotation '{}' names no known type, alias, phenotype or trait; \
the runtime matches nothing (every value stresses)",
                    n
                ),
            ));
        }
    }
}

impl<'a> Checker<'a> {
    // ================================================== expression inference

    fn infer(&mut self, scope: &mut Scope, e: &Expr) -> Ty {
        match e {
            Expr::Null => Ty::Null,
            Expr::Bool(_) => Ty::Bool,
            Expr::Int(_) => Ty::Int,
            Expr::Float(_) => Ty::Float,
            Expr::Str(_) | Expr::Interp(_) => Ty::Str,
            Expr::Bytes(_) => Ty::Bytes,
            Expr::List(items) => {
                let t = self.subs.fresh();
                for it in items {
                    let it_ty = self.infer(scope, it);
                    if self.subs.unify(&t, &it_ty).is_none() {
                        return Ty::Unknown;
                    }
                }
                let el = self.subs.resolve(&t);
                Ty::List(Rc::new(el))
            }
            Expr::Map(items) => {
                let k = self.subs.fresh();
                let v = self.subs.fresh();
                for (ke, ve) in items {
                    let kt = self.infer(scope, ke);
                    let vt = self.infer(scope, ve);
                    if self.subs.unify(&k, &kt).is_none() || self.subs.unify(&v, &vt).is_none() {
                        return Ty::Unknown;
                    }
                }
                let kr = self.subs.resolve(&k);
                let vr = self.subs.resolve(&v);
                Ty::Map(Rc::new(kr), Rc::new(vr))
            }
            Expr::Ident(n) => match scope.get(n) {
                Some(b) => b.ty.clone(),
                None => match self.syms.globals.iter().rev().find(|(g, _)| g == n) {
                    Some((_, t)) => t.clone(),
                    None => match self.syms.genes.contains_key(n) {
                        true => Ty::Gene,
                        false => Ty::Unknown, // dynamic global: no findings
                    },
                },
            },
            Expr::Unary(op, a) => match op {
                UnOp::Not => {
                    let _ = self.infer(scope, a);
                    Ty::Bool
                }
                UnOp::Neg | UnOp::BitNot => {
                    let t = self.infer(scope, a);
                    match t {
                        Ty::Float => Ty::Float,
                        Ty::Unknown | Ty::Var(_) | Ty::Any | Ty::GVar(_) => t,
                        _ => Ty::Int,
                    }
                }
            },
            Expr::Binary(op, l, r, _) => {
                let lt = self.infer(scope, l);
                let rt = self.infer(scope, r);
                self.infer_binop(*op, lt, rt)
            }
            Expr::Ternary(c, a, b) => {
                let _ = self.infer(scope, c);
                let at = self.infer(scope, a);
                let bt = self.infer(scope, b);
                match self.subs.unify(&at, &bt) {
                    Some(t) => t,
                    None => Ty::Union(vec![at, bt]),
                }
            }
            Expr::Index(target, _, _) => {
                let tt = self.infer(scope, target);
                match tt {
                    Ty::List(el) => (*el).clone(),
                    Ty::Map(_, v) => (*v).clone(),
                    Ty::Str => Ty::Str,
                    Ty::Bytes => Ty::Int,
                    _ => Ty::Unknown,
                }
            }
            Expr::Member(target, field) => {
                let it = self.infer(scope, target);
                let tt = self.subs.resolve(&it);
                match tt {
                    Ty::Pheno(pname) => self.pheno_field_ty(&pname, field),
                    _ => Ty::Unknown,
                }
            }
            Expr::MemberSafe(target, field) => {
                // `x?.k` — same shape, null propagates; the member type
                // becomes optional when the receiver may be null
                let it = self.infer(scope, target);
                let tt = self.subs.resolve(&it);
                match tt {
                    Ty::Pheno(pname) => {
                        let f = self.pheno_field_ty(&pname, field);
                        Ty::Opt(Rc::new(f))
                    }
                    Ty::Opt(inner) => match self.subs.resolve(&inner) {
                        Ty::Pheno(pname) => {
                            let f = self.pheno_field_ty(&pname, field);
                            Ty::Opt(Rc::new(f))
                        }
                        _ => Ty::Unknown,
                    },
                    _ => Ty::Unknown,
                }
            }
            Expr::Method(target, name, args) => {
                let tt = self.infer(scope, target);
                for a in args {
                    let _ = self.infer(scope, a);
                }
                self.infer_method(tt, name)
            }
            Expr::MethodSafe(target, name, args) => {
                let tt = self.infer(scope, target);
                for a in args {
                    let _ = self.infer(scope, a);
                }
                match self.subs.resolve(&tt) {
                    Ty::Opt(inner) => {
                        let inner_ty = self.subs.resolve(&inner);
                        self.infer_method(inner_ty, name)
                    }
                    other => self.infer_method(other, name),
                }
            }
            Expr::Call(callee, args, line) => self.infer_call(scope, callee, args, *line),
            Expr::Lambda(_) => Ty::Gene,
            Expr::Collect { iter, body, .. } => {
                let _ = self.infer(scope, iter);
                let bt = self.infer(scope, body);
                Ty::List(Rc::new(bt))
            }
            Expr::FateNew(_) => Ty::Unknown,
            Expr::New(name, args) => {
                for a in args {
                    let _ = self.infer(scope, a);
                }
                Ty::Pheno(name.clone())
            }
            Expr::Propagate(inner, _) => {
                // `e?!` unwraps: Option<T> -> T, Result<T, E> -> T
                let it = self.infer(scope, inner);
                match it {
                    Ty::Opt(t) => (*t).clone(),
                    Ty::Res(t, _) => (*t).clone(),
                    other => other,
                }
            }
        }
    }

    fn infer_binop(&mut self, op: BinOp, lt: Ty, rt: Ty) -> Ty {
        match op {
            BinOp::Eq
            | BinOp::Neq
            | BinOp::Lt
            | BinOp::Le
            | BinOp::Gt
            | BinOp::Ge
            | BinOp::And
            | BinOp::Or
            | BinOp::In => Ty::Bool,
            BinOp::Nullish => {
                // coalesce: the left's non-null side union the right
                let l = self.subs.resolve(&lt);
                let r = rt;
                match l {
                    Ty::Opt(inner) => {
                        let i = self.subs.resolve(&inner);
                        match self.subs.unify(&i, &r) {
                            Some(t) => t,
                            None => Ty::Union(vec![i, r]),
                        }
                    }
                    Ty::Null => r,
                    other => match self.subs.unify(&other, &r) {
                        Some(t) => t,
                        None => Ty::Union(vec![other, r]),
                    },
                }
            }
            // numeric lattice: int stays int unless a float touches it
            BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::FloorDiv
            | BinOp::Mod
            | BinOp::Pow => {
                let l = self.subs.resolve(&lt);
                let r = self.subs.resolve(&rt);
                match (&l, &r) {
                    (Ty::Str, Ty::Str) if op == BinOp::Add => Ty::Str,
                    (Ty::Unknown, _)
                    | (_, Ty::Unknown)
                    | (Ty::Any, _)
                    | (_, Ty::Any)
                    | (Ty::Var(_), _)
                    | (_, Ty::Var(_))
                    | (Ty::GVar(_), _)
                    | (_, Ty::GVar(_)) => Ty::Unknown,
                    (Ty::Str, _) if op == BinOp::Add => Ty::Unknown, // promote-ish
                    (_, Ty::Str) if op == BinOp::Add => Ty::Unknown,
                    (Ty::Float, _) | (_, Ty::Float) | (Ty::Int, _) | (_, Ty::Int) => {
                        if op == BinOp::Div {
                            // check the runtime contract: / is float division
                            Ty::Float
                        } else {
                            match (&l, &r) {
                                (Ty::Float, _) | (_, Ty::Float) => Ty::Float,
                                _ => Ty::Int,
                            }
                        }
                    }
                    _ => Ty::Unknown,
                }
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => Ty::Int,
        }
    }

    fn infer_method(&mut self, recv: Ty, name: &str) -> Ty {
        let recv = self.subs.resolve(&recv);
        match &recv {
            Ty::Pheno(pname) => {
                if let Some(sig) = self.pheno_method(pname, name) {
                    return sig.ret.as_ref().map(ty_of_ann).unwrap_or(Ty::Unknown);
                }
                // field-as-callable or trait default: dynamic, no finding
                Ty::Unknown
            }
            Ty::Union(members) => {
                // every member must agree; one definite miss is a finding
                let mut ret: Option<Ty> = None;
                for m in members.clone() {
                    let r = self.subs.resolve(&m);
                    match method_ret(&r, name) {
                        Some(t) => {
                            ret = Some(match ret {
                                None => t,
                                Some(prev) => self.subs.unify(&prev, &t).unwrap_or(Ty::Unknown),
                            });
                        }
                        None if r.is_unknownish() => return Ty::Unknown,
                        None => {
                            self.boundary_finding(
                                self.cur_line,
                                "no-method",
                                format!(
                                    "{} has no method '{}' (the runtime yields null)",
                                    r.show(),
                                    name
                                ),
                            );
                            return Ty::Unknown;
                        }
                    }
                }
                ret.unwrap_or(Ty::Unknown)
            }
            other => match method_ret(other, name) {
                Some(t) => t,
                None => {
                    self.boundary_finding(
                        self.cur_line,
                        "no-method",
                        format!(
                            "{} has no method '{}' (the runtime yields null)",
                            other.show(),
                            name
                        ),
                    );
                    Ty::Unknown
                }
            },
        }
    }

    fn pheno_method(&self, pname: &str, name: &str) -> Option<GeneSig> {
        let mut cur = Some(pname.to_string());
        let mut hops = 0;
        while let Some(p) = cur {
            if hops > 32 {
                break;
            }
            if let Some(sig) = self.syms.phenos.get(&p) {
                if let Some(m) = sig.methods.iter().find(|m| m.name == name) {
                    return Some(m.clone());
                }
                cur = sig.parent.clone();
            } else {
                break;
            }
            hops += 1;
        }
        None
    }

    fn pheno_field_ty(&self, pname: &str, field: &str) -> Ty {
        let mut cur = Some(pname.to_string());
        let mut hops = 0;
        while let Some(p) = cur {
            if hops > 32 {
                break;
            }
            if let Some(sig) = self.syms.phenos.get(&p) {
                if let Some((_, t)) = sig.fields.iter().find(|(f, _)| f == field) {
                    if let Some(t) = t {
                        return t.clone();
                    }
                    return Ty::Unknown;
                }
                cur = sig.parent.clone();
            } else {
                break;
            }
            hops += 1;
        }
        Ty::Unknown
    }

    fn infer_call(
        &mut self,
        scope: &mut Scope,
        callee: &Expr,
        args: &[Expr],
        call_line: usize,
    ) -> Ty {
        // the arg types are inferred FIRST (they feed both the boundary
        // checks and the return inference)
        let arg_tys: Vec<Ty> = args.iter().map(|a| self.infer(scope, a)).collect();
        match callee {
            Expr::Ident(name) => self.infer_named_call(scope, name, args, arg_tys, call_line),
            other => {
                let ct = self.infer(scope, other);
                let _ = ct;
                Ty::Unknown
            }
        }
    }

    fn infer_named_call(
        &mut self,
        _scope: &mut Scope,
        name: &str,
        _args: &[Expr],
        arg_tys: Vec<Ty>,
        call_line: usize,
    ) -> Ty {
        if let Some(sig) = self.syms.genes.get(name).cloned() {
            return self.check_call_site(&sig, name, arg_tys, call_line);
        }
        // builtin surface: return-type inference only (arg checking stays
        // with the runtime soft contracts; no duplicate findings)
        let unknown = Ty::Unknown;
        match name {
            "len" => Ty::Int,
            "keys" => Ty::List(Rc::new(Ty::Unknown)),
            "values" => Ty::List(Rc::new(Ty::Unknown)),
            "has" => Ty::Bool,
            "str" | "type_name" | "type" => Ty::Str,
            "num" => Ty::Unknown,
            "abs" | "min" | "max" | "sum" => Ty::Unknown,
            "clock" => Ty::Float,
            "some" => {
                let t = arg_tys.first().cloned().unwrap_or(Ty::Unknown);
                Ty::Opt(Rc::new(t))
            }
            "none" => Ty::Opt(Rc::new(Ty::Unknown)),
            "ok" => {
                let t = arg_tys.first().cloned().unwrap_or(Ty::Unknown);
                Ty::Res(Rc::new(t), Rc::new(Ty::Unknown))
            }
            "err" => {
                let t = arg_tys.first().cloned().unwrap_or(Ty::Unknown);
                Ty::Res(Rc::new(Ty::Unknown), Rc::new(t))
            }
            "push" | "pop" | "insert" | "remove" | "del" => unknown,
            _ => unknown,
        }
    }

    /// One call to a KNOWN gene signature: boundary checks + generic
    /// instantiation. This is where "catching it before runtime" happens
    /// for annotated genes: each check mirrors a runtime `unfolded` Stress
    /// one-for-one.
    fn check_call_site(
        &mut self,
        sig: &GeneSig,
        name: &str,
        arg_tys: Vec<Ty>,
        call_line: usize,
    ) -> Ty {
        // generic instantiation: each declared type parameter becomes a
        // fresh inference variable; arguments unify into it
        let mut tvars: HashMap<String, Ty> = HashMap::new();
        for (tp, _bound) in &sig.type_params {
            tvars.insert(tp.clone(), self.subs.fresh());
        }

        // argument boundary checks (mirror the runtime funnel, position by
        // position; extra args are the lint E01/arity lane, not ours)
        for (i, ann) in sig.param_anns.iter().enumerate() {
            let Some(ann) = ann else { continue };
            let Some(actual) = arg_tys.get(i) else {
                continue; // missing arg: the arity/default lane
            };
            let pname = sig.params.get(i).map(|(n, _)| n.as_str()).unwrap_or("?");
            let expected = self.instantiate(ty_of_ann(ann), &tvars);
            let actual = self.subs.resolve(actual);
            if !accepts(&expected, &actual) {
                self.boundary_finding(
                    if call_line > 0 {
                        call_line
                    } else {
                        self.cur_line
                    },
                    "type-mismatch",
                    format!(
                        "argument '{}' for gene '{}' expects {}, got {}",
                        pname,
                        name,
                        ann.render(),
                        actual.show()
                    ),
                );
            }
        }

        // trait bounds: check the ACTUAL argument at the position whose
        // annotation is that type parameter (the tvar itself is only a
        // fresh variable until instantiation; the argument is the evidence)
        for (i, ann) in sig.param_anns.iter().enumerate() {
            let Some(TypeAnn::TypeVar(tp)) = ann else {
                continue;
            };
            let Some((_, Some(bound))) = sig.type_params.iter().find(|(n, _)| n == tp) else {
                continue;
            };
            let bound_ty = ty_of_ann(bound);
            let tname = match &bound_ty {
                Ty::Trait(t) | Ty::Pheno(t) => t.clone(),
                _ => continue,
            };
            self.check_trait_bound(&tname, tp, arg_tys.get(i));
        }

        sig.ret
            .as_ref()
            .map(|r| {
                let t = ty_of_ann(r);
                self.instantiate(t, &tvars)
            })
            .unwrap_or(Ty::Unknown)
    }

    fn instantiate(&mut self, ty: Ty, tvars: &HashMap<String, Ty>) -> Ty {
        match ty {
            Ty::GVar(n) => tvars.get(&n).cloned().unwrap_or(Ty::Unknown),
            Ty::List(t) => Ty::List(Rc::new(self.instantiate((*t).clone(), tvars))),
            Ty::Map(k, v) => Ty::Map(
                Rc::new(self.instantiate((*k).clone(), tvars)),
                Rc::new(self.instantiate((*v).clone(), tvars)),
            ),
            Ty::Opt(t) => Ty::Opt(Rc::new(self.instantiate((*t).clone(), tvars))),
            Ty::Res(t, e) => Ty::Res(
                Rc::new(self.instantiate((*t).clone(), tvars)),
                Rc::new(self.instantiate((*e).clone(), tvars)),
            ),
            Ty::Union(alts) => Ty::Union(
                alts.into_iter()
                    .map(|t| self.instantiate(t, tvars))
                    .collect(),
            ),
            other => other,
        }
    }

    /// A `T: Trait` bound: provable satisfaction requires the argument to
    /// be a phenotype implementing the trait (or inheriting it). Anything
    /// else is unverifiable at check time (no finding) — the runtime stays
    /// the arbiter.
    fn check_trait_bound(&mut self, tname: &str, tparam: &str, arg: Option<&Ty>) {
        if !self.syms.traits.contains_key(tname) {
            self.findings.push(finding(
                0,
                "unknown-type-name",
                Sev::Warning,
                format!(
                    "bound '{}' on type parameter '{}' names no declared trait",
                    tname, tparam
                ),
            ));
            return;
        }
        let Some(arg) = arg else { return };
        let arg = self.subs.resolve(arg);
        let Ty::Pheno(pname) = arg else {
            return; // primitives/unknown: unverifiable, no finding
        };
        if !self.pheno_implements(&pname, tname) {
            self.findings.push(finding(
                0,
                "trait-bound-violation",
                Sev::Warning,
                format!(
                    "type parameter '{}' bound to trait '{}': phenotype '{}' does not implement it",
                    tparam, tname, pname
                ),
            ));
        }
    }

    fn pheno_implements(&self, pname: &str, tname: &str) -> bool {
        let mut cur = Some(pname.to_string());
        let mut hops = 0;
        while let Some(p) = cur {
            if hops > 32 {
                break;
            }
            match self.syms.phenos.get(&p) {
                Some(sig) => {
                    if sig.implements.iter().any(|t| t == tname) {
                        return true;
                    }
                    cur = sig.parent.clone();
                }
                None => break,
            }
            hops += 1;
        }
        false
    }
}

impl<'a> Checker<'a> {
    // ================================================== statement checking

    fn check_gene_body(&mut self, sig: &GeneSig, body: &[Stmt]) {
        let mut scope = Scope::child();
        // bind params: annotated ones pin their type, unannotated ones are
        // dynamic (Unknown). Type parameters stay GVars: the body checks
        // against the generic; per-call instantiation happened at call sites.
        for (i, (pname, _default)) in sig.params.iter().enumerate() {
            if pname.is_empty() || pname == "?" {
                continue;
            }
            let ann = sig.param_anns.get(i).and_then(|a| a.as_ref());
            let t = match ann {
                Some(a) => ty_of_ann(a),
                None => Ty::Unknown,
            };
            scope.define(pname, t, ann.is_some());
        }
        let ret_ty = sig.ret.as_ref().map(ty_of_ann);
        let mut ctx = FnCtx {
            name: sig.name.clone(),
            ret_ty,
            ret_ann_text: sig.ret.as_ref().map(|r| r.render()),
            saw_return: false,
            can_fall_through: true,
            def_line: sig.line,
        };
        self.check_block(&mut scope, body, &mut ctx);
        // implicit null return: the body can complete without a return
        // while the annotation refuses null
        if let (Some(ret), Some(text)) = (&ctx.ret_ty, &ctx.ret_ann_text) {
            if ctx.can_fall_through && !accepts(ret, &Ty::Null) {
                // a gene whose last statement is a return does NOT fall
                // through; check_block updated the flag
                self.findings.push(finding(
                    sig.line,
                    "null-return",
                    Sev::Warning,
                    format!(
                        "gene '{}' can complete without a return but its return \
annotation '{}' refuses null",
                        sig.name, text
                    ),
                ));
            }
        }
    }

    fn check_block(&mut self, scope: &mut Scope, stmts: &[Stmt], ctx: &mut FnCtx) {
        for st in stmts {
            self.check_stmt(scope, st, ctx);
            if matches!(st, Stmt::Return(..) | Stmt::Raise(..)) {
                ctx.can_fall_through = false;
            }
        }
    }

    fn check_stmt(&mut self, scope: &mut Scope, st: &Stmt, ctx: &mut FnCtx) {
        let line = stmt_line(st);
        if line > 0 {
            self.cur_line = line;
        }
        match st {
            Stmt::Let(name, e) => {
                let t = self.infer(scope, e);
                scope.define(name, t, false);
            }
            Stmt::LetConst(name, e) => {
                let t = self.infer(scope, e);
                scope.define(name, t, true); // frozen: assignments checked
            }
            Stmt::LetAnn(name, ann, e) => {
                self.check_ann_names(ann, stmt_line(st).max(self.cur_line));
                let expected = ty_of_ann(ann);
                let actual = self.infer(scope, e);
                let actual = self.subs.resolve(&actual);
                if !accepts(&expected, &actual) {
                    self.boundary_finding(
                        stmt_line(st),
                        "type-mismatch",
                        format!(
                            "declaration of '{}' expects {}, got {}",
                            name,
                            ann.render(),
                            actual.show()
                        ),
                    );
                }
                scope.define(name, expected, true);
            }
            Stmt::Assign(name, _, e) => {
                let t = self.infer(scope, e);
                let t = self.subs.resolve(&t);
                match scope.get(name) {
                    Some(b) if b.declared => {
                        if !accepts(&b.ty, &t) {
                            self.boundary_finding(
                                stmt_line(st),
                                "type-mismatch",
                                format!(
                                    "assignment to '{}' expects {}, got {}",
                                    name,
                                    b.ty.show(),
                                    t.show()
                                ),
                            );
                        }
                    }
                    Some(b) => {
                        let merged = merge_tys(b.ty.clone(), t);
                        scope.define(name, merged, false);
                    }
                    None => {
                        scope.define(name, t, false);
                    }
                }
            }
            Stmt::IndexAssign(t, i, _, e) => {
                let _ = self.infer(scope, t);
                let _ = self.infer(scope, i);
                let _ = self.infer(scope, e);
            }
            Stmt::MemberAssign(t, _, _, e) => {
                let _ = self.infer(scope, t);
                let _ = self.infer(scope, e);
            }
            Stmt::ExprStmt(e) => {
                let _ = self.infer(scope, e);
            }
            Stmt::Return(e) => {
                ctx.saw_return = true;
                match (e, &ctx.ret_ty) {
                    (Some(expr), Some(ret)) => {
                        let actual = self.infer(scope, expr);
                        let actual = self.subs.resolve(&actual);
                        if !accepts(ret, &actual) {
                            let line = match stmt_line(st) {
                                0 => ctx.def_line,
                                l => l,
                            };
                            self.boundary_finding(
                                line,
                                "type-mismatch",
                                format!(
                                    "return of gene '{}' expects {}, got {}",
                                    ctx.name,
                                    ctx.ret_ann_text.clone().unwrap_or_default(),
                                    actual.show()
                                ),
                            );
                        }
                    }
                    (Some(expr), None) => {
                        let _ = self.infer(scope, expr);
                    }
                    (None, Some(ret)) => {
                        if !accepts(ret, &Ty::Null) {
                            self.findings.push(finding(
                                stmt_line(st),
                                "null-return",
                                Sev::Warning,
                                format!(
                                    "bare return in gene '{}' crosses its \
non-optional return annotation '{}'",
                                    ctx.name,
                                    ctx.ret_ann_text.clone().unwrap_or_default()
                                ),
                            ));
                        }
                    }
                    (None, None) => {}
                }
            }
            Stmt::If(arms, els) => {
                for (cond, body) in arms {
                    let _ = self.infer(scope, cond);
                    let mut inner = Scope::child();
                    inner.vars = scope.vars.clone();
                    let mut sub = FnCtx {
                        name: ctx.name.clone(),
                        ret_ty: ctx.ret_ty.clone(),
                        ret_ann_text: ctx.ret_ann_text.clone(),
                        saw_return: ctx.saw_return,
                        can_fall_through: true,
                        def_line: ctx.def_line,
                    };
                    self.check_block(&mut inner, body, &mut sub);
                }
                if let Some(els) = els {
                    let mut inner = Scope::child();
                    inner.vars = scope.vars.clone();
                    let mut sub = FnCtx {
                        name: ctx.name.clone(),
                        ret_ty: ctx.ret_ty.clone(),
                        ret_ann_text: ctx.ret_ann_text.clone(),
                        saw_return: ctx.saw_return,
                        can_fall_through: true,
                        def_line: ctx.def_line,
                    };
                    self.check_block(&mut inner, els, &mut sub);
                }
            }
            Stmt::While(cond, body) => {
                let _ = self.infer(scope, cond);
                let mut inner = Scope::child();
                inner.vars = scope.vars.clone();
                self.check_block(&mut inner, body, ctx);
            }
            Stmt::Loop(body) => {
                let mut inner = Scope::child();
                inner.vars = scope.vars.clone();
                self.check_block(&mut inner, body, ctx);
            }
            Stmt::For(var, iter, body) | Stmt::ForPat(Pat::Bind(var), iter, body) => {
                let it = self.infer(scope, iter);
                let el = match self.subs.resolve(&it) {
                    Ty::List(el) => (*el).clone(),
                    Ty::Map(k, _) => (*k).clone(), // iterating a map: keys
                    _ => Ty::Unknown,
                };
                let mut inner = Scope::child();
                inner.vars = scope.vars.clone();
                inner.define(var, el, false);
                self.check_block(&mut inner, body, ctx);
            }
            Stmt::ForPat(_, _, _) => {}
            Stmt::Match(scrutinee, arms) => {
                let st = self.infer(scope, scrutinee);
                self.check_exhaustive(&st, arms, scrutinee);
                for (pat, body) in arms {
                    let mut inner = Scope::child();
                    inner.vars = scope.vars.clone();
                    self.bind_pattern(&mut inner, pat, &st);
                    self.check_block(&mut inner, body, ctx);
                }
            }
            Stmt::Block(body) | Stmt::Scope(body) => {
                let mut inner = Scope::child();
                inner.vars = scope.vars.clone();
                self.check_block(&mut inner, body, ctx);
            }
            Stmt::Stress { body, rescue, .. } => {
                let was = self.in_stress;
                self.in_stress = true;
                self.check_block(scope, body, ctx);
                self.in_stress = was;
                if let Some((_, rb)) = rescue {
                    self.check_block(scope, rb, ctx);
                }
            }
            // gene/sequence definitions: their bodies are checked exactly
            // once by collect_bodies (any depth), never here — a double
            // check would duplicate every body finding
            Stmt::Gene(_) | Stmt::Seq(_) => {}
            Stmt::TypeAlias(_, ann, l) => self.check_ann_names(ann, *l),
            // definitions, module plumbing and the bio layer carry no
            // expression-level type surface the checker verifies
            _ => {}
        }
    }

    fn bind_pattern(&mut self, scope: &mut Scope, pat: &MatchPat, scrut: &Ty) {
        match pat {
            MatchPat::Bind(n) => {
                scope.define(n, scrut.clone(), false);
            }
            MatchPat::Variant(tag, payload) => {
                let Some(payload) = payload else { return };
                let inner: Ty = match self.subs.resolve(scrut) {
                    Ty::Opt(t) if tag == "Some" => (*t).clone(),
                    Ty::Opt(t) if tag == "None" => (*t).clone(),
                    Ty::Res(t, _) if tag == "Ok" => (*t).clone(),
                    Ty::Res(_, e) if tag == "Err" => (*e).clone(),
                    _ => Ty::Unknown,
                };
                self.bind_pattern(scope, payload, &inner);
            }
            MatchPat::ListPat { elems, rest } => {
                for el in elems {
                    self.bind_pattern(scope, el, &Ty::Unknown);
                }
                if let Some(r) = rest {
                    scope.define(r, Ty::List(Rc::new(Ty::Unknown)), false);
                }
            }
            MatchPat::MapPat { keys } => {
                for (k, sub) in keys {
                    if let Some(sub) = sub {
                        self.bind_pattern(scope, sub, &Ty::Unknown);
                    } else {
                        scope.define(k, Ty::Unknown, false);
                    }
                }
            }
            MatchPat::Or(alts) => {
                for a in alts {
                    self.bind_pattern(scope, a, scrut);
                }
            }
            MatchPat::Guard(inner, _) => self.bind_pattern(scope, inner, scrut),
            _ => {}
        }
    }
}

/// Per-gene checking context: return annotation + fall-through bookkeeping.
struct FnCtx {
    name: String,
    ret_ty: Option<Ty>,
    ret_ann_text: Option<String>,
    saw_return: bool,
    can_fall_through: bool,
    /// the gene's declaration line: the fallback location for return
    /// findings whose statement carries no span (a `return "str"` has no
    /// stamped node; the gene line is real and `// allow:`-addressable)
    def_line: usize,
}

impl<'a> Checker<'a> {
    // ================================================== exhaustiveness

    /// W02-REMAIN (partial): exhaustiveness over the four built-in variant
    /// families. A scrutinee that resolves to an optional/result (or a
    /// union containing them) must be covered: every required tag needs an
    /// arm, or the match needs a catch-all (`_`, a bare binding, or an
    /// or-pattern containing one). Guarded arms never count as coverage
    /// (their condition can fail — the Rust rule). Warning severity per
    /// the W02 build note ("lint warning, not hard error").
    fn check_exhaustive(&mut self, scrut: &Ty, arms: &[(MatchPat, Vec<Stmt>)], scrut_expr: &Expr) {
        // a DIRECT variant constructor carries its tag on its face:
        // `match some(x)` can only be Some, so a missing None arm is not
        // an uncertainty — partial matching is the idiom (match_v2 corpus)
        if let Expr::Call(callee, _, _) = scrut_expr {
            if let Expr::Ident(name) = &**callee {
                if matches!(name.as_str(), "some" | "none" | "ok" | "err") {
                    return;
                }
            }
        }
        let mut required: Vec<&'static str> = Vec::new();
        match self.subs.resolve(scrut) {
            Ty::Opt(_) => {
                required.push("Some");
                required.push("None");
            }
            Ty::Res(_, _) => {
                required.push("Ok");
                required.push("Err");
            }
            Ty::Union(members) => {
                for m in members {
                    match m {
                        Ty::Opt(_) => {
                            if !required.contains(&"Some") {
                                required.push("Some");
                            }
                            if !required.contains(&"None") {
                                required.push("None");
                            }
                        }
                        Ty::Res(_, _) => {
                            if !required.contains(&"Ok") {
                                required.push("Ok");
                            }
                            if !required.contains(&"Err") {
                                required.push("Err");
                            }
                        }
                        _ => return, // a mixed union: tags unknown, skip
                    }
                }
            }
            _ => return, // plain values: no variant coverage contract
        }
        if required.is_empty() {
            return;
        }
        let mut missing: Vec<&'static str> = required.clone();
        let mut catchall = false;
        for (pat, _) in arms {
            self.pattern_covers(pat, &mut missing, &mut catchall);
        }
        if !missing.is_empty() && !catchall {
            self.findings.push(finding(
                expr_line(scrut_expr),
                "non-exhaustive-match",
                Sev::Warning,
                format!(
                    "match on {} is not exhaustive: missing {} (add the arm(s) or a catch-all)",
                    scrut.show(),
                    missing.join(", ")
                ),
            ));
        }
    }

    fn pattern_covers(&self, pat: &MatchPat, missing: &mut Vec<&'static str>, catchall: &mut bool) {
        match pat {
            MatchPat::Wild | MatchPat::Bind(_) => *catchall = true,
            MatchPat::Variant(tag, _) => missing.retain(|m| m != tag),
            MatchPat::Or(alts) => {
                for a in alts {
                    self.pattern_covers(a, missing, catchall);
                }
            }
            // a guarded arm covers NOTHING (the condition may fail)
            MatchPat::Guard(inner, _) => {
                let mut m2 = missing.clone();
                let mut c2 = false;
                self.pattern_covers(inner, &mut m2, &mut c2);
                // coverage of the tag itself is NOT taken; only a nested
                // catch-all inside the guarded pattern could count, and a
                // guarded catch-all still cannot prove coverage
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------ public API

/// One resolved signature row for `operon sig`.
#[derive(Debug, Clone)]
pub struct Signature {
    pub text: String,
    pub declared: bool,
    pub line: usize,
}

/// The check-time entry point: all type findings for one parsed program.
/// Findings ride the check stream of the shared rule engine (same Finding
/// shape, same allow-suppression, same stable-code ladder).
pub fn check(prog: &Program) -> Vec<Finding> {
    let mut c = Checker::new(prog);
    c.run();
    c.findings
}

impl<'a> Checker<'a> {
    fn run(&mut self) {
        // W13 over every annotation that appears anywhere, at the line of
        // its declaring statement (real lines are allow-suppressible)
        let all_anns: Vec<(TypeAnn, usize)> = collect_all_anns(self.prog);
        for (ann, line) in &all_anns {
            self.check_ann_names(ann, *line);
        }

        // the program's own top-level statements: the script body is a
        // pseudo-gene (no params, no return annotation) so the classic
        // dynamic bug (x = 10; x.name()) is caught at check time
        let mut scope = Scope::child();
        for (n, t) in self.syms.globals.clone() {
            scope.define(&n, t, true);
        }
        let mut ctx = FnCtx {
            name: "<main>".to_string(),
            ret_ty: None,
            ret_ann_text: None,
            saw_return: false,
            can_fall_through: true,
            def_line: 0,
        };
        let stmts = self.prog.stmts.clone();
        self.check_block(&mut scope, &stmts, &mut ctx);

        // gene bodies + proof frames included (the proof suite is a
        // consumer: its calls and bindings type-check too)
        let bodies: Vec<(GeneSig, Vec<Stmt>)> = collect_bodies(self.prog);
        for (sig, body) in &bodies {
            self.check_gene_body(sig, body);
        }
    }
}

fn collect_bodies(prog: &Program) -> Vec<(GeneSig, Vec<Stmt>)> {
    let mut out = Vec::new();
    collect_bodies_stmts(&prog.stmts, &mut out);
    for body in &prog.proofs {
        collect_bodies_stmts(body, &mut out);
    }
    for (_, body) in &prog.named_frames {
        collect_bodies_stmts(body, &mut out);
    }
    out
}

fn collect_bodies_stmts(stmts: &[Stmt], out: &mut Vec<(GeneSig, Vec<Stmt>)>) {
    for st in stmts {
        match st {
            Stmt::Gene(g) | Stmt::Seq(g) => {
                if let Some(n) = &g.name {
                    out.push((
                        GeneSig {
                            name: n.clone(),
                            params: g.params.clone(),
                            param_anns: g.param_anns.clone(),
                            ret: g.ret_ann.clone(),
                            type_params: g.type_params.clone(),
                            line: g.line,
                            seq: g.seq,
                            recv: None,
                        },
                        g.body.clone(),
                    ));
                }
            }
            Stmt::Pheno(p) => {
                for m in &p.methods {
                    if let Some(mn) = &m.name {
                        out.push((
                            GeneSig {
                                name: format!("{}.{}", p.name, mn),
                                params: m.params.clone(),
                                param_anns: m.param_anns.clone(),
                                ret: m.ret_ann.clone(),
                                type_params: m.type_params.clone(),
                                line: m.line,
                                seq: false,
                                recv: Some(p.name.clone()),
                            },
                            m.body.clone(),
                        ));
                    }
                }
            }
            Stmt::Trait(t) => {
                for m in &t.methods {
                    if let (Some(d), Some(mn)) = (&m.default, Some(&m.name)) {
                        out.push((
                            GeneSig {
                                name: format!("{}::{}", t.name, mn),
                                params: d.params.clone(),
                                param_anns: d.param_anns.clone(),
                                ret: d.ret_ann.clone(),
                                type_params: d.type_params.clone(),
                                line: m.line,
                                seq: false,
                                recv: None,
                            },
                            d.body.clone(),
                        ));
                    }
                }
            }
            other => {
                for nested in stmt_nested_all_pub(other) {
                    collect_bodies_stmts(nested, out);
                }
            }
        }
    }
}

/// Re-export of the lint traversal for nested statement bodies (kept local
/// to avoid a private-item dependency: mirrors lint::stmt_nested_all).
fn stmt_nested_all_pub(st: &Stmt) -> Vec<&[Stmt]> {
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
        Stmt::Match(_, arms) => arms.iter().map(|(_, b)| &b[..]).collect(),
        Stmt::Stress { body, rescue, .. } => {
            let mut out = vec![body.as_slice()];
            if let Some((_, rb)) = rescue {
                out.push(rb.as_slice());
            }
            out
        }
        _ => vec![],
    }
}

fn collect_all_anns(prog: &Program) -> Vec<(TypeAnn, usize)> {
    let mut out = Vec::new();
    collect_anns_stmts(&prog.stmts, &mut out);
    for body in &prog.proofs {
        collect_anns_stmts(body, &mut out);
    }
    out
}

fn collect_anns_stmts(stmts: &[Stmt], out: &mut Vec<(TypeAnn, usize)>) {
    for st in stmts {
        let line = stmt_line(st);
        match st {
            Stmt::Gene(g) | Stmt::Seq(g) => {
                for (_, a) in &g.type_params {
                    if let Some(a) = a {
                        out.push((a.clone(), g.line));
                    }
                }
                for a in g.param_anns.iter().flatten() {
                    out.push((a.clone(), g.line));
                }
                if let Some(r) = &g.ret_ann {
                    out.push((r.clone(), g.line));
                }
            }
            Stmt::LetAnn(_, a, _) => out.push((a.clone(), line)),
            Stmt::TypeAlias(_, a, l) => out.push((a.clone(), *l)),
            Stmt::Pheno(p) => {
                for m in &p.methods {
                    for a in m.param_anns.iter().flatten() {
                        out.push((a.clone(), m.line));
                    }
                    if let Some(r) = &m.ret_ann {
                        out.push((r.clone(), m.line));
                    }
                }
            }
            other => {
                for nested in stmt_nested_all_pub(other) {
                    collect_anns_stmts(nested, out);
                }
            }
        }
    }
}

/// The `operon sig` surface: every gene's signature, declared (from its
/// annotations) or inferred (from its returns). Inference for unannotated
/// genes walks the body and unions the types of every `return` expression.
pub fn signatures(prog: &Program) -> Vec<Signature> {
    let syms = collect_symbols(prog);
    let mut out = Vec::new();
    let mut c = Checker::new(prog);
    for st in &prog.stmts {
        collect_sigs_stmt(st, &syms, &mut c, &mut out);
    }
    out
}

fn collect_sigs_stmt(st: &Stmt, _syms: &Symbols, c: &mut Checker, out: &mut Vec<Signature>) {
    match st {
        Stmt::Gene(g) | Stmt::Seq(g) => {
            if let Some(n) = &g.name {
                let declared = g.ret_ann.is_some() || g.param_anns.iter().any(|a| a.is_some());
                let params: Vec<String> = g
                    .params
                    .iter()
                    .enumerate()
                    .map(
                        |(i, (p, _))| match g.param_anns.get(i).and_then(|a| a.as_ref()) {
                            Some(a) => format!("{}: {}", p, a.render()),
                            None => p.clone(),
                        },
                    )
                    .collect();
                let ret = match &g.ret_ann {
                    Some(r) => r.render(),
                    None => {
                        // infer: union of return-expression types
                        let sig = GeneSig {
                            name: n.clone(),
                            params: g.params.clone(),
                            param_anns: g.param_anns.clone(),
                            ret: None,
                            type_params: g.type_params.clone(),
                            line: g.line,
                            seq: g.seq,
                            recv: None,
                        };
                        let mut scope = Scope::child();
                        for (i, (pname, _)) in sig.params.iter().enumerate() {
                            let ann = sig.param_anns.get(i).and_then(|a| a.as_ref());
                            let t = match ann {
                                Some(a) => ty_of_ann(a),
                                None => Ty::Unknown,
                            };
                            scope.define(pname, t, ann.is_some());
                        }
                        let mut ctx = FnCtx {
                            name: n.clone(),
                            ret_ty: None,
                            ret_ann_text: None,
                            saw_return: false,
                            can_fall_through: true,
                            def_line: sig.line,
                        };
                        c.check_block(&mut scope, &g.body, &mut ctx);
                        c.findings.clear(); // sig dump is diagnostic-free
                        let t = infer_returns(c, &g.body, &mut scope);
                        match t {
                            Ty::Unknown => String::new(),
                            other => other.show(),
                        }
                    }
                };
                let tp = fmt_type_params(&g.type_params);
                let text = if ret.is_empty() {
                    format!("gene {}{}({})", n, tp, params.join(", "))
                } else {
                    format!("gene {}{}({}) -> {}", n, tp, params.join(", "), ret)
                };
                out.push(Signature {
                    text,
                    declared,
                    line: g.line,
                });
            }
        }
        other => {
            for nested in stmt_nested_all_pub(other) {
                for s in nested {
                    collect_sigs_stmt(s, _syms, c, out);
                }
            }
        }
    }
}

/// Canonical type-parameter list rendering, `<T, U: Show>` (mirrors the
/// fmt form; empty list renders empty).
fn fmt_type_params(tps: &[(String, Option<TypeAnn>)]) -> String {
    if tps.is_empty() {
        return String::new();
    }
    let inner: Vec<String> = tps
        .iter()
        .map(|(n, bound)| match bound {
            Some(b) => format!("{}: {}", n, b.render()),
            None => n.clone(),
        })
        .collect();
    format!("<{}>", inner.join(", "))
}

fn infer_returns(c: &mut Checker, body: &[Stmt], scope: &mut Scope) -> Ty {
    let mut acc: Option<Ty> = None;
    collect_return_tys(c, body, scope, &mut acc);
    match acc {
        None => Ty::Unknown,
        Some(t) => c.subs.resolve(&t),
    }
}

fn collect_return_tys(c: &mut Checker, stmts: &[Stmt], scope: &mut Scope, acc: &mut Option<Ty>) {
    for st in stmts {
        match st {
            Stmt::Return(Some(e)) => {
                let t = c.infer(scope, e);
                match acc {
                    None => *acc = Some(t),
                    Some(prev) => {
                        let merged = match c.subs.unify(prev, &t) {
                            Some(m) => m,
                            None => Ty::Union(vec![prev.clone(), t]),
                        };
                        *acc = Some(merged);
                    }
                }
            }
            Stmt::Gene(g) | Stmt::Seq(g) => {
                // a nested gene's returns belong to the nested gene
                let _ = g;
            }
            other => {
                for nested in stmt_nested_all_pub(other) {
                    let mut inner = Scope::child();
                    inner.vars = scope.vars.clone();
                    collect_return_tys(c, nested, &mut inner, acc);
                }
            }
        }
    }
}
