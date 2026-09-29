//! typecheck.rs — TYPED-MODE: the Operon static type checker (TYPED-MODE.md).
//!
//! A pure AST pass over a parsed `Program`. It never evaluates, never
//! mutates the interpreter, and never rejects a program the parser accepted —
//! it emits `lint::Finding`s in the `check` stream (T-series codes, W041
//! scheme). The dynamic side is untouched: typed mode is a compile-time gate
//! (`operon run --typed`, `operon check --typed`), nothing else.
//!
//! Design law (TYPED-MODE.md §1): additive. Programs that opt nothing in
//! check trivially clean; `Any` is the universal escape hatch so dynamic
//! code inside a typed file stays dynamic.

use crate::ast::{Expr, MatchPat, Pat, PhenoDef, Program, Stmt, TypeAnn};
use crate::lint::{Finding, Sev};
use std::collections::HashMap;
use std::sync::Arc;

// ================================================================= Ty lattice

/// The checker-internal type lattice (TYPED-MODE.md §3).
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    /// The universal type: dynamic values, unknown shapes, escape hatch.
    Any,
    /// Bottom: unreachable (a match arm that cannot run, a branch that
    /// always stresses). Absorbs into everything.
    Never,
    Null,
    Bool,
    Int,
    Float,
    Str,
    Bytes,
    List(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    Gene,
    Seq,
    Channel,
    Weak,
    /// A named phenotype (user class instance).
    Pheno(String),
    /// A trait-typed value (`let d: Drawable = ...`).
    TraitObj(String),
    /// `some(v)` / `none()`, payload-typed.
    OptionT(Box<Ty>),
    /// `ok(v)` / `err(e)`.
    ResultT(Box<Ty>, Box<Ty>),
    /// Flattened alternation (`a | b`).
    Union(Vec<Ty>),
    /// A generic type parameter in scope inside a generic gene body.
    Param(String),
}

impl Ty {
    pub fn render(&self) -> String {
        match self {
            Ty::Any => "any".into(),
            Ty::Never => "never".into(),
            Ty::Null => "null".into(),
            Ty::Bool => "bool".into(),
            Ty::Int => "int".into(),
            Ty::Float => "float".into(),
            Ty::Str => "str".into(),
            Ty::Bytes => "bytes".into(),
            Ty::List(t) => format!("list[{}]", t.render()),
            Ty::Map(k, v) => format!("map[{}, {}]", k.render(), v.render()),
            Ty::Gene => "gene".into(),
            Ty::Seq => "sequence".into(),
            Ty::Channel => "channel".into(),
            Ty::Weak => "weak".into(),
            Ty::Pheno(n) => n.clone(),
            Ty::TraitObj(n) => n.clone(),
            Ty::OptionT(t) => format!("{}?", t.render()),
            Ty::ResultT(t, e) => format!("result[{}, {}]", t.render(), e.render()),
            Ty::Union(alts) => alts
                .iter()
                .map(|a| a.render())
                .collect::<Vec<_>>()
                .join(" | "),
            Ty::Param(p) => p.clone(),
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int | Ty::Float | Ty::Never)
    }
}

/// Flatten unions (recursively) into leaf alternatives.
fn flat(t: &Ty) -> Vec<Ty> {
    match t {
        Ty::Union(alts) => {
            let mut out = Vec::new();
            for a in alts {
                out.extend(flat(a));
            }
            out
        }
        other => vec![other.clone()],
    }
}

/// Normalize a type: flatten unions, drop `Never`, dedupe, collapse `Any`.
pub fn mk_union(parts: Vec<Ty>) -> Ty {
    let mut alts: Vec<Ty> = Vec::new();
    for p in parts {
        for leaf in flat(&p) {
            if matches!(leaf, Ty::Any) {
                return Ty::Any;
            }
            if matches!(leaf, Ty::Never) {
                continue;
            }
            if !alts.contains(&leaf) {
                alts.push(leaf);
            }
        }
    }
    match alts.len() {
        0 => Ty::Never,
        1 => alts.pop().unwrap(),
        _ => Ty::Union(alts),
    }
}

/// Structural type equality through unions (order-insensitive).
pub fn ty_eq(a: &Ty, b: &Ty) -> bool {
    let fa = flat(a);
    let fb = flat(b);
    if fa.len() != fb.len() {
        return false;
    }
    fa.iter().all(|x| fb.contains(x))
}

/// Is `actual` acceptable where `formal` is declared? (TYPED-MODE.md §4/§6:
/// int widens to float, unions commute, containers check element-wise,
/// phenotypes walk their lineage, `Any` anywhere is the escape hatch.)
pub fn ty_compat(actual: &Ty, formal: &Ty) -> bool {
    ty_compat_deep(actual, formal, &[])
}

fn ty_compat_deep(actual: &Ty, formal: &Ty, phenos: &[PhenoInfo]) -> bool {
    if matches!(formal, Ty::Any)
        || matches!(actual, Ty::Any)
        || matches!(actual, Ty::Never)
        || matches!(formal, Ty::Never)
    {
        return true;
    }
    // union formal: any alternative accepts
    if let Ty::Union(alts) = formal {
        return alts.iter().any(|f| ty_compat_deep(actual, f, phenos));
    }
    // union actual: every member must be accepted
    if let Ty::Union(alts) = actual {
        return alts.iter().all(|a| ty_compat_deep(a, formal, phenos));
    }
    match (formal, actual) {
        // numeric widening: int is acceptable where float is declared
        (Ty::Float, Ty::Int) => true,
        // null matches optional declarations (runtime law, W01)
        (Ty::OptionT(_), Ty::Null) => true,
        (Ty::OptionT(f), Ty::OptionT(a)) => ty_compat_deep(a, f, phenos),
        (Ty::ResultT(ft, fe), Ty::ResultT(at, ae)) => {
            ty_compat_deep(at, ft, phenos) && ty_compat_deep(ae, fe, phenos)
        }
        (Ty::List(f), Ty::List(a)) => ty_compat_deep(a, f, phenos),
        (Ty::Map(fk, fv), Ty::Map(ak, av)) => {
            ty_compat_deep(ak, fk, phenos) && ty_compat_deep(av, fv, phenos)
        }
        (Ty::Pheno(want), Ty::Pheno(got)) => want == got || pheno_derives(got, want, phenos),
        (Ty::TraitObj(trait_name), Ty::Pheno(pn)) => pheno_implements(pn, trait_name, phenos),
        (Ty::TraitObj(_), Ty::TraitObj(tn)) => {
            formal == actual || {
                let _ = tn;
                false
            }
        }
        (Ty::Param(p), _) | (_, Ty::Param(p)) => {
            // parameters unify by name; bounds are checked at the solving site
            if let Ty::Param(q) = actual {
                p == q
            } else {
                true // an argument solves the parameter
            }
        }
        _ => formal == actual,
    }
}

// ================================================================= phenotype/trait info

/// Resolved, checker-facing phenotype record (lineage pre-walked).
#[derive(Debug, Clone)]
pub struct PhenoInfo {
    pub name: String,
    pub parent: Option<String>,
    pub implements: Vec<String>,
    /// required-method names this phenotype itself defines
    pub own_methods: Vec<String>,
    pub fields: Vec<String>,
    pub line: usize,
}

pub fn pheno_derives(got: &str, want: &str, phenos: &[PhenoInfo]) -> bool {
    let mut cur = Some(got.to_string());
    let mut hops = 0;
    while let Some(c) = cur {
        if c == want {
            return true;
        }
        hops += 1;
        if hops > 64 {
            return false; // lineage cycle guard
        }
        cur = phenos
            .iter()
            .find(|p| p.name == c)
            .and_then(|p| p.parent.clone());
    }
    false
}

pub fn pheno_implements(pn: &str, trait_name: &str, phenos: &[PhenoInfo]) -> bool {
    let mut cur = Some(pn.to_string());
    let mut hops = 0;
    while let Some(c) = cur {
        if let Some(p) = phenos.iter().find(|p| p.name == c) {
            if p.implements.iter().any(|t| t == trait_name) {
                return true;
            }
            hops += 1;
            if hops > 64 {
                return false;
            }
            cur = p.parent.clone();
        } else {
            return false;
        }
    }
    false
}

// ================================================================= bounds

/// Does `ty` satisfy the declared bound? Builtins: `numeric`, `comparable`.
/// A trait name requires the type to be a phenotype implementing it.
pub fn satisfies_bound(ty: &Ty, bound: &str, phenos: &[PhenoInfo]) -> bool {
    match bound {
        "numeric" => match ty {
            Ty::Int | Ty::Float | Ty::Any | Ty::Never | Ty::Param(_) => true,
            Ty::Union(alts) => alts.iter().all(|a| satisfies_bound(a, bound, phenos)),
            _ => false,
        },
        "comparable" => match ty {
            Ty::Int
            | Ty::Float
            | Ty::Str
            | Ty::Bytes
            | Ty::Bool
            | Ty::Null
            | Ty::Any
            | Ty::Never
            | Ty::Param(_) => true,
            Ty::List(t) => satisfies_bound(t, bound, phenos),
            Ty::Map(k, v) => satisfies_bound(k, bound, phenos) && satisfies_bound(v, bound, phenos),
            Ty::Union(alts) => alts.iter().all(|a| satisfies_bound(a, bound, phenos)),
            _ => false,
        },
        // trait bound: phenotype lineage must implement it; anything else
        // that is not statically resolvable passes (Any/Param escape hatch)
        trait_name => match ty {
            Ty::Pheno(pn) => pheno_implements(pn, trait_name, phenos),
            Ty::TraitObj(tn) => tn == trait_name,
            Ty::Any | Ty::Never | Ty::Param(_) => true,
            Ty::Union(alts) => alts.iter().all(|a| satisfies_bound(a, bound, phenos)),
            _ => false,
        },
    }
}

// ================================================================= builtins

/// (min_args, max_args, return type) for the builtins whose shape the
/// checker can state. max == usize::MAX = variadic. Anything NOT in this
/// table is treated as `(0, MAX, Any)` — the lint wrong-arity pass owns
/// arity findings, so the checker stays type-only.
fn builtin_sig(name: &str, args: &[Ty]) -> Option<(usize, usize, Ty)> {
    let ret = match name {
        "len" => Some(Ty::Int),
        "type_name" => Some(Ty::Str),
        "str" => Some(Ty::Str),
        "int" => Some(Ty::Int),
        "float" => Some(Ty::Float),
        "bool" => Some(Ty::Bool),
        "num" => Some(mk_union(vec![Ty::Int, Ty::Float])),
        "abs" => Some(abs_ret(args.first())),
        "keys" => Some(Ty::List(Box::new(Ty::Any))),
        "values" => Some(Ty::List(Box::new(Ty::Any))),
        "items" => Some(Ty::List(Box::new(Ty::Any))),
        "has" => Some(Ty::Bool),
        "del" => Some(Ty::Bool),
        "contains" => Some(Ty::Bool),
        "print" | "println" | "eprint" | "eprintln" => Some(Ty::Null),
        "some" => Some(Ty::OptionT(Box::new(
            args.first().cloned().unwrap_or(Ty::Any),
        ))),
        "none" => Some(Ty::OptionT(Box::new(Ty::Any))),
        "ok" => Some(Ty::ResultT(
            Box::new(args.first().cloned().unwrap_or(Ty::Any)),
            Box::new(Ty::Any),
        )),
        "err" => Some(Ty::ResultT(
            Box::new(Ty::Any),
            Box::new(args.first().cloned().unwrap_or(Ty::Any)),
        )),
        "range" => Some(Ty::Seq),
        "floor" | "ceil" | "round" | "sqrt" | "sin" | "cos" | "tan" | "log" | "exp" => {
            Some(Ty::Float)
        }
        "json_parse" => Some(Ty::Any),
        "json_stringify" | "json" => Some(Ty::Str),
        _ => None,
    }?;
    let (lo, hi) = match name {
        "none" => (0, 0),
        "print" | "println" | "eprint" | "eprintln" => (0, usize::MAX),
        _ => (1, usize::MAX),
    };
    Some((lo, hi, ret))
}

fn abs_ret(arg: Option<&Ty>) -> Ty {
    match arg {
        Some(Ty::Int) => Ty::Int,
        Some(Ty::Float) => Ty::Float,
        Some(Ty::Union(alts)) => {
            let parts: Vec<Ty> = alts
                .iter()
                .map(|a| match a {
                    Ty::Int => Ty::Int,
                    Ty::Float => Ty::Float,
                    other => other.clone(),
                })
                .collect();
            mk_union(parts)
        }
        _ => Ty::Any,
    }
}

// ================================================================= method surface

/// The result of looking up a method on a statically known receiver.
pub enum MethodHit {
    /// method exists; this is its return type (given arg types for generics)
    Found(Ty),
    /// receiver type is known, method does not exist on it
    Missing,
    /// receiver type unknown (Any) — never flagged
    Open,
}

/// Builtin method surface per receiver type, mirrored from
/// `interp.rs::call_method` (TYPED-MODE.md §6).
pub fn method_hit(recv: &Ty, name: &str, _args: &[Ty], phenos: &[PhenoInfo]) -> MethodHit {
    match recv {
        Ty::Any | Ty::Never | Ty::Param(_) => MethodHit::Open,
        Ty::Str => match name {
            "upper" | "lower" | "trim" | "replace" | "repeat" | "slice" | "join" => {
                MethodHit::Found(Ty::Str)
            }
            "at" => MethodHit::Found(mk_union(vec![Ty::Str, Ty::Null])),
            "split" => MethodHit::Found(Ty::List(Box::new(Ty::Str))),
            "contains" | "starts" | "ends" => MethodHit::Found(Ty::Bool),
            "len" => MethodHit::Found(Ty::Int),
            _ => MethodHit::Missing,
        },
        Ty::Bytes => match name {
            "slice" => MethodHit::Found(Ty::Bytes),
            "len" => MethodHit::Found(Ty::Int),
            _ => MethodHit::Missing,
        },
        Ty::List(el) => match name {
            "map" => MethodHit::Found(Ty::List(Box::new(Ty::Any))),
            "filter" | "sort" | "reverse" | "slice" => MethodHit::Found(Ty::List(el.clone())),
            "reduce" => MethodHit::Found(Ty::Any),
            "each" => MethodHit::Found(Ty::Null),
            "join" => MethodHit::Found(Ty::Str),
            "contains" => MethodHit::Found(Ty::Bool),
            "index_of" => MethodHit::Found(Ty::Int),
            "len" => MethodHit::Found(Ty::Int),
            "push" | "pop" | "get" => MethodHit::Found((**el).clone()),
            _ => MethodHit::Missing,
        },
        Ty::Map(_k, v) => match name {
            "keys" => MethodHit::Found(Ty::List(Box::new(Ty::Any))),
            "values" => MethodHit::Found(Ty::List(Box::new((**v).clone()))),
            "items" => MethodHit::Found(Ty::List(Box::new(Ty::List(Box::new(Ty::Any))))),
            "has" => MethodHit::Found(Ty::Bool),
            "get" => MethodHit::Found(mk_union(vec![v.as_ref().clone(), Ty::Null])),
            "del" => MethodHit::Found(Ty::Bool),
            "len" => MethodHit::Found(Ty::Int),
            _ => MethodHit::Missing,
        },
        Ty::Seq => match name {
            "next" | "collect" => MethodHit::Found(Ty::Any),
            _ => MethodHit::Missing,
        },
        Ty::OptionT(_) | Ty::ResultT(_, _) => {
            // variants carry no builtin methods at runtime (the unknown
            // method note fires); `?!` and match are the destructurers
            MethodHit::Missing
        }
        Ty::Pheno(pn) => {
            // known phenotype: walk lineage + implemented-trait defaults
            match pheno_method(pn, name, phenos) {
                true => MethodHit::Found(Ty::Any),
                false => MethodHit::Missing,
            }
        }
        Ty::TraitObj(tn) => match trait_has_method(tn, name) {
            Some(true) => MethodHit::Found(Ty::Any),
            _ => MethodHit::Open, // unknown trait shapes stay open
        },
        // int/float/bool/null/gene/channel/weak carry NO methods at runtime
        // (call_method falls to "unknown method; null") — the flagship
        // `x = 10; x.name()` catch.
        _ => MethodHit::Missing,
    }
}

/// Is `mname` a method of phenotype `pn` or its lineage? (checker-side
/// view; the trait-default dispatch order is runtime law).
fn pheno_method(pn: &str, mname: &str, phenos: &[PhenoInfo]) -> bool {
    let mut cur = Some(pn.to_string());
    let mut hops = 0;
    while let Some(c) = cur {
        if let Some(p) = phenos.iter().find(|p| p.name == c) {
            if p.own_methods.iter().any(|m| m == mname) {
                return true;
            }
            // trait default methods also dispatch
            for t in &p.implements {
                if trait_has_method(t, mname) == Some(true) {
                    return true;
                }
            }
            hops += 1;
            if hops > 64 {
                return false;
            }
            cur = p.parent.clone();
        } else {
            // phenotype not declared in this file (module boundary) — open
            return true;
        }
    }
    false
}

/// Trait shape: Some(required) if the trait is declared and the method is
/// required, Some(false) if present as default, None if unknown trait.
pub fn trait_has_method(trait_name: &str, mname: &str) -> Option<bool> {
    // filled by the Checker from the program (static table below)
    TRAIT_TABLE.with(|t| {
        t.borrow()
            .get(&(trait_name.to_string(), mname.to_string()))
            .copied()
    })
}

thread_local! {
    /// (trait, method) -> required. Rebuilt per check_program; the checker
    /// is a one-shot CLI pass, a thread-local table is the cheapest lawful
    /// shape that keeps method_hit's signature allocation-free.
    static TRAIT_TABLE: std::cell::RefCell<HashMap<(String, String), bool>> =
        std::cell::RefCell::new(HashMap::new());
}

// ================================================================= checker

/// A gene's static signature.
#[derive(Debug, Clone)]
struct GeneSig {
    type_params: Vec<(String, Option<String>)>,
    /// formal param types, aligned with the def's params (None = unannotated)
    params: Vec<Option<Ty>>,
    /// declared or inferred return type; `None` while inferring (recursion)
    ret: Option<Ty>,
    /// the gene is being body-inferred right now (recursive calls -> Any)
    inferring: bool,
    /// the return annotation, if declared (for the final unify)
    ret_ann: Option<Ty>,
    /// definition line (kept for future span-aware diagnostics)
    #[allow(dead_code)]
    line: usize,
}

/// One scoped binding frame.
struct Scope {
    vars: HashMap<String, Ty>,
}

/// The checker pass. One instance per file; findings accumulate.
pub struct Checker {
    phenos: Vec<PhenoInfo>,
    genes: HashMap<String, GeneSig>,
    /// statement line hints, threaded for findings (best effort, A13 spans)
    findings: Vec<Finding>,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    pub fn new() -> Self {
        Checker {
            phenos: Vec::new(),
            genes: HashMap::new(),
            findings: Vec::new(),
        }
    }

    fn finding(&mut self, line: usize, code: &'static str, rule: &str, sev: Sev, msg: String) {
        self.findings.push(Finding {
            line,
            code,
            rule: rule.to_string(),
            sev,
            message: msg,
        });
    }

    // ------------------------------------------------ pass 1: collect defs

    fn collect_program(&mut self, prog: &Program) {
        // trait table first (phenotype method resolution reads it)
        TRAIT_TABLE.with(|t| {
            let mut t = t.borrow_mut();
            t.clear();
            for st in &prog.stmts {
                collect_traits_stmt(st, &mut t);
            }
        });
        for st in &prog.stmts {
            self.collect_stmt(st);
        }
    }

    fn collect_stmt(&mut self, st: &Stmt) {
        match st {
            Stmt::Gene(g) | Stmt::Seq(g) => {
                self.collect_gene(g);
                // nested genes inside the body also collect
                for s in &g.body {
                    self.collect_stmt(s);
                }
            }
            Stmt::Pheno(p) => {
                self.collect_pheno(p);
                for m in &p.methods {
                    self.collect_gene(m);
                }
            }
            Stmt::Splice(s) => {
                for (_, g) in &s.variants {
                    self.collect_gene(g);
                }
            }
            Stmt::Trait(t) => {
                for m in &t.methods {
                    if let Some(d) = &m.default {
                        self.collect_gene(d);
                    }
                }
            }
            // nested statement bodies may hold genes (if/for/match/...)
            other => {
                for nested in stmt_bodies(other) {
                    for s in nested {
                        self.collect_stmt(s);
                    }
                }
            }
        }
    }

    fn collect_gene(&mut self, g: &Arc<crate::ast::GeneDef>) {
        let Some(name) = &g.name else { return };
        let tps: Vec<String> = g.type_params.iter().map(|(p, _)| p.clone()).collect();
        let params: Vec<Option<Ty>> = g
            .param_anns
            .iter()
            .map(|a| a.as_ref().map(|a| self.ann_to_ty_in(a, &tps)))
            .collect();
        let ret_ann = g.ret_ann.as_ref().map(|a| self.ann_to_ty_in(a, &tps));
        // annotated genes know their return immediately; unannotated ones
        // are inferred in pass 2 (ret starts None, `inferring` guards cycles)
        let type_params = g.type_params.clone();
        self.genes.insert(
            name.clone(),
            GeneSig {
                type_params,
                params,
                ret: ret_ann.clone(),
                inferring: false,
                ret_ann,
                line: g.line,
            },
        );
    }

    fn collect_pheno(&mut self, p: &PhenoDef) {
        // static trait contracts (TYPED-MODE.md §8): every REQUIRED method
        // of an implemented trait must exist on the phenotype or its lineage
        let info = PhenoInfo {
            name: p.name.clone(),
            parent: p.parent.clone(),
            implements: p.implements.clone(),
            own_methods: p.methods.iter().filter_map(|m| m.name.clone()).collect(),
            fields: p.fields.iter().map(|(f, _)| f.clone()).collect(),
            line: p.line,
        };
        for tname in &p.implements {
            let required: Vec<String> = trait_required_methods(tname);
            if trait_declared(tname) && required.is_empty() {
                // trait declared with only defaults: nothing required
                continue;
            }
            for req in required {
                let have = info.own_methods.iter().any(|m| m == &req)
                    || parent_has_method(&info.parent, &req, &self.phenos);
                if !have {
                    self.finding(
                        p.line,
                        "T07",
                        "trait-method-missing",
                        Sev::Error,
                        format!(
                            "phenotype '{}' implements '{}' but does not define required method '{}()'",
                            p.name, tname, req
                        ),
                    );
                }
            }
        }
        self.phenos.push(info);
    }

    // ------------------------------------------------ annotations

    /// TypeAnn -> Ty. Case-normalized (`Int`≡`int`); generic annotations
    /// become structural types; unknown names degrade to Any (T04 finding
    /// happens at the declaration site, here we stay silent and soft).
    fn ann_to_ty(&self, ann: &TypeAnn) -> Ty {
        self.ann_to_ty_in(ann, &[])
    }

    /// Type-param-aware resolution: inside `gene first<T>(...)`, the
    /// annotation `T` (and `T?`, `list[T]`, unions of it) IS the parameter.
    fn ann_to_ty_in(&self, ann: &TypeAnn, tps: &[String]) -> Ty {
        match ann {
            TypeAnn::Named(n) => {
                if tps.iter().any(|p| p == n) {
                    Ty::Param(n.clone())
                } else {
                    named_to_ty(n, &[])
                }
            }
            TypeAnn::Union(alts) => {
                mk_union(alts.iter().map(|a| self.ann_to_ty_in(a, tps)).collect())
            }
            TypeAnn::Optional(inner) => Ty::OptionT(Box::new(self.ann_to_ty_in(inner, tps))),
            TypeAnn::Generic(head, args) => {
                // a generic head that IS a parameter name resolves to it
                if tps.iter().any(|p| p == head) {
                    return Ty::Param(head.clone());
                }
                let a: Vec<Ty> = args.iter().map(|x| self.ann_to_ty_in(x, tps)).collect();
                generic_to_ty(head, &a)
            }
        }
    }

    /// T04: an annotation naming nothing we know (and not a declared
    /// phenotype/trait/type parameter context) is reported once at its site.
    fn ann_unknown_names(&mut self, ann: &TypeAnn, type_params: &[String], line: usize) {
        match ann {
            TypeAnn::Named(n) => {
                let norm = n.to_ascii_lowercase();
                let known = matches!(
                    norm.as_str(),
                    "any"
                        | "null"
                        | "bool"
                        | "int"
                        | "float"
                        | "str"
                        | "bytes"
                        | "list"
                        | "map"
                        | "gene"
                        | "sequence"
                        | "channel"
                        | "weak"
                        | "phenotype"
                        | "option"
                        | "result"
                );
                let is_typaram = type_params.iter().any(|p| p == n);
                if !known && !is_typaram {
                    // capitalized names: phenotype or trait — only flag when
                    // nothing of that name is declared in this file
                    let declared = self.phenos.iter().any(|p| &p.name == n) || trait_declared(n);
                    if !declared {
                        self.finding(
                            line,
                            "T04",
                            "unknown-type",
                            Sev::Warning,
                            format!(
                                "type annotation '{}' names nothing known; treated as any",
                                n
                            ),
                        );
                    }
                }
            }
            TypeAnn::Union(alts) => {
                for a in alts {
                    self.ann_unknown_names(a, type_params, line);
                }
            }
            TypeAnn::Optional(inner) => self.ann_unknown_names(inner, type_params, line),
            TypeAnn::Generic(head, args) => {
                let norm = head.to_ascii_lowercase();
                let known = matches!(norm.as_str(), "list" | "map" | "option" | "result" | "any");
                if !known {
                    self.finding(
                        line,
                        "T04",
                        "unknown-type",
                        Sev::Warning,
                        format!(
                            "type annotation '{}[...]' names nothing known; treated as any",
                            head
                        ),
                    );
                }
                for a in args {
                    self.ann_unknown_names(a, type_params, line);
                }
            }
        }
    }

    // ------------------------------------------------ pass 2: bodies

    fn infer_gene_bodies(&mut self, prog: &Program) {
        // order: collect all named gene defs (flattened), infer bodies
        let mut defs: Vec<Arc<crate::ast::GeneDef>> = Vec::new();
        for st in &prog.stmts {
            collect_gene_defs(st, &mut defs);
        }
        for g in defs {
            let name = match &g.name {
                Some(n) => n.clone(),
                None => continue, // anonymous lambda bodies check at their site
            };
            let line = g.line;
            // unknown-name findings for the signature annotations
            let tp_names: Vec<String> = g.type_params.iter().map(|(p, _)| p.clone()).collect();
            for a in g.param_anns.iter().flatten() {
                self.ann_unknown_names(a, &tp_names, line);
            }
            if let Some(a) = &g.ret_ann {
                self.ann_unknown_names(a, &tp_names, line);
            }
            let mut returns: Vec<Ty> = Vec::new();
            let mut env = Env {
                scopes: vec![Scope {
                    vars: HashMap::new(),
                }],
            };
            // bind params (annotations resolve with the gene's type params)
            for (i, (pname, _)) in g.params.iter().enumerate() {
                let ty = g
                    .param_anns
                    .get(i)
                    .and_then(|a| a.as_ref())
                    .map(|a| self.ann_to_ty_in(a, &tp_names))
                    .unwrap_or(Ty::Any);
                env.bind(pname.clone(), ty);
            }
            // bind type params as Param ty
            for (p, _) in &g.type_params {
                env.bind(p.clone(), Ty::Param(p.clone()));
            }
            // self-recursion: mark inferring so calls inside see ret Any
            if let Some(sig) = self.genes.get_mut(&name) {
                sig.inferring = true;
            }
            let fallthrough = self.check_stmts_ctx(&g.body, &mut env, &mut returns, Some(&g), line);
            if let Some(sig) = self.genes.get_mut(&name) {
                sig.inferring = false;
            }
            // finalize WITHOUT holding a genes borrow across self.finding
            let formal = self.genes.get(&name).and_then(|s| s.ret_ann.clone());
            let mut all_returns = returns.clone();
            if fallthrough {
                all_returns.push(Ty::Null); // a body that can fall through returns null
            }
            let inferred = mk_union(all_returns);
            if let Some(f) = &formal {
                if fallthrough && !returns.is_empty() && !matches!(f, Ty::Any) {
                    self.finding(
                        line,
                        "T10",
                        "return-missing",
                        Sev::Warning,
                        format!(
                            "gene '{}' is annotated '-> {}' but can fall through to null",
                            name,
                            f.render()
                        ),
                    );
                }
                if !ty_compat(&inferred, f) {
                    self.finding(
                        line,
                        "T01",
                        "type-mismatch",
                        Sev::Error,
                        format!(
                            "gene '{}' is annotated '-> {}' but returns {}",
                            name,
                            f.render(),
                            inferred.render()
                        ),
                    );
                }
                if let Some(sig) = self.genes.get_mut(&name) {
                    sig.ret = Some(f.clone());
                }
            } else if let Some(sig) = self.genes.get_mut(&name) {
                sig.ret = Some(inferred);
            }
        }
    }

    // ------------------------------------------------ statements

    /// Check a body. `returns` collects inferred return types; `ctx` is the
    /// enclosing gene (for return-annotation unification). Returns whether
    /// the body can fall through (no terminal last statement).
    fn check_stmts_ctx(
        &mut self,
        stmts: &[Stmt],
        env: &mut Env,
        returns: &mut Vec<Ty>,
        ctx: Option<&Arc<crate::ast::GeneDef>>,
        line_hint: usize,
    ) -> bool {
        let mut hint = line_hint;
        for st in stmts {
            hint = self.check_stmt(st, env, returns, ctx, hint);
        }
        // fallthrough unless the last statement is terminal
        !matches!(
            stmts.last(),
            Some(Stmt::Return(_))
                | Some(Stmt::Raise(..))
                | Some(Stmt::Break)
                | Some(Stmt::Continue)
        )
    }

    /// Statement check; returns an updated line hint.
    fn check_stmt(
        &mut self,
        st: &Stmt,
        env: &mut Env,
        returns: &mut Vec<Ty>,
        _ctx: Option<&Arc<crate::ast::GeneDef>>,
        line_hint: usize,
    ) -> usize {
        match st {
            Stmt::Let(name, e) => {
                let hint = expr_line_deep(e).unwrap_or(line_hint);
                let ty = self.infer(e, env, hint);
                env.bind(name.clone(), ty);
                hint
            }
            Stmt::LetConst(name, e) => {
                let hint = expr_line_deep(e).unwrap_or(line_hint);
                let ty = self.infer(e, env, hint);
                env.bind(name.clone(), ty);
                hint
            }
            Stmt::LetAnn(name, ann, e) => {
                let hint = expr_line_deep(e).unwrap_or(line_hint);
                self.ann_unknown_names(ann, &[], hint);
                let formal = self.ann_to_ty(ann);
                let actual = self.infer(e, env, hint);
                if !ty_compat(&actual, &formal) {
                    self.finding(
                        hint,
                        "T01",
                        "type-mismatch",
                        Sev::Error,
                        format!(
                            "'{}' is annotated '{}' but is assigned {}",
                            name,
                            formal.render(),
                            actual.render()
                        ),
                    );
                }
                env.bind(name.clone(), formal);
                hint
            }
            Stmt::LetPat(pat, e) => {
                let hint = expr_line_deep(e).unwrap_or(line_hint);
                let ty = self.infer(e, env, hint);
                bind_pat(pat, ty, env);
                hint
            }
            Stmt::Assign(name, op, e) => {
                let hint = expr_line_deep(e).unwrap_or(line_hint);
                let vt = self.infer(e, env, hint);
                let existing = env.get(name);
                match (existing, op) {
                    (Some(bt), None) => {
                        if !ty_compat(&vt, &bt) {
                            self.finding(
                                hint,
                                "T09",
                                "assign-type-change",
                                Sev::Warning,
                                format!(
                                    "assignment changes '{}' from {} to {}",
                                    name,
                                    bt.render(),
                                    vt.render()
                                ),
                            );
                        }
                    }
                    (Some(bt), Some(binop)) => {
                        let joined = self.binop_ty(binop, &bt, &vt, hint);
                        let _ = joined;
                    }
                    _ => {} // unknown binding: dynamic law (lint owns phantoms)
                }
                if op.is_none() {
                    // re-typing is legal dynamically; the binding widens
                    env.rebind(name.clone(), vt);
                }
                hint
            }
            Stmt::For(var, iter, body) => {
                let hint = expr_line_deep(iter).unwrap_or(line_hint);
                let it = self.infer(iter, env, hint);
                let elem = match it {
                    Ty::List(el) => *el,
                    Ty::Map(k, v) => mk_union(vec![*k, *v]),
                    Ty::Str => Ty::Str,
                    Ty::Any => Ty::Any,
                    _ => Ty::Any,
                };
                env.push();
                env.bind(var.clone(), elem);
                for s in body {
                    self.check_stmt(s, env, returns, _ctx, hint);
                }
                env.pop();
                hint
            }
            Stmt::Match(scrut, arms, mline) => {
                // the match's own line (TYPED-MODE stamp) is the honest
                // location for exhaustiveness findings
                let hint = if *mline > 0 {
                    *mline
                } else {
                    expr_line_deep(scrut).unwrap_or(line_hint)
                };
                let st_ty = self.infer(scrut, env, hint);
                self.check_exhaustive(&st_ty, arms, hint);
                for (pat, body) in arms {
                    env.push();
                    if let Some((bind_name, bind_ty)) = pat_binding(pat, &st_ty) {
                        env.bind(bind_name, bind_ty);
                    }
                    for s in body {
                        self.check_stmt(s, env, returns, _ctx, hint);
                    }
                    env.pop();
                }
                hint
            }
            Stmt::Return(Some(e)) => {
                let hint = expr_line_deep(e).unwrap_or(line_hint);
                let ty = self.infer(e, env, hint);
                returns.push(ty);
                hint
            }
            Stmt::Return(None) => {
                returns.push(Ty::Null);
                line_hint
            }
            Stmt::ExprStmt(e) => {
                let hint = expr_line_deep(e).unwrap_or(line_hint);
                self.infer(e, env, hint);
                hint
            }
            // bodies with their own flow
            Stmt::If(arms, els) => {
                for (cond, body) in arms {
                    let hint = expr_line_deep(cond).unwrap_or(line_hint);
                    self.infer(cond, env, hint);
                    env.push();
                    for s in body {
                        self.check_stmt(s, env, returns, _ctx, hint);
                    }
                    env.pop();
                }
                if let Some(eb) = els {
                    env.push();
                    for s in eb {
                        self.check_stmt(s, env, returns, _ctx, line_hint);
                    }
                    env.pop();
                }
                line_hint
            }
            Stmt::While(cond, body) => {
                let hint = expr_line_deep(cond).unwrap_or(line_hint);
                self.infer(cond, env, hint);
                env.push();
                for s in body {
                    self.check_stmt(s, env, returns, _ctx, hint);
                }
                env.pop();
                hint
            }
            Stmt::Loop(body) | Stmt::Scope(body) | Stmt::Block(body) => {
                env.push();
                for s in body {
                    self.check_stmt(s, env, returns, _ctx, line_hint);
                }
                env.pop();
                line_hint
            }
            Stmt::Gene(g) | Stmt::Seq(g) => {
                // nested gene: its body was collected in pass 1 and checked
                // in the flattened pass-2 walk; nothing here
                let _ = g;
                line_hint
            }
            _ => line_hint,
        }
    }

    // ------------------------------------------------ expressions

    /// Infer the static type of an expression. `hint` is the best-effort
    /// source line for findings.
    fn infer(&mut self, e: &Expr, env: &mut Env, hint: usize) -> Ty {
        match e {
            Expr::Null => Ty::Null,
            Expr::Bool(_) => Ty::Bool,
            Expr::Int(_) => Ty::Int,
            Expr::Float(_) => Ty::Float,
            Expr::Str(_) => Ty::Str,
            Expr::Bytes(_) => Ty::Bytes,
            Expr::Interp(_) => Ty::Str,
            Expr::List(items) => {
                let mut parts = Vec::new();
                for i in items {
                    parts.push(self.infer(i, env, hint));
                }
                Ty::List(Box::new(mk_union(parts)))
            }
            Expr::Map(pairs) => {
                let mut ks = Vec::new();
                let mut vs = Vec::new();
                for (k, v) in pairs {
                    ks.push(self.infer(k, env, hint));
                    vs.push(self.infer(v, env, hint));
                }
                Ty::Map(Box::new(mk_union(ks)), Box::new(mk_union(vs)))
            }
            Expr::Ident(name) => match env.get(name) {
                Some(t) => t,
                None => match self.genes.get(name) {
                    Some(sig) => sig.ret.clone().unwrap_or(Ty::Any),
                    None => {
                        let _ = builtin_sig(name, &[]);
                        Ty::Any
                    }
                },
            },
            Expr::Unary(op, x) => {
                let t = self.infer(x, env, hint);
                match op {
                    crate::ast::UnOp::Not => Ty::Bool,
                    crate::ast::UnOp::Neg => match t {
                        Ty::Int => Ty::Int,
                        Ty::Float => Ty::Float,
                        Ty::Union(alts) => mk_union(
                            alts.iter()
                                .map(|a| match a {
                                    Ty::Int => Ty::Int,
                                    _ => Ty::Float,
                                })
                                .collect(),
                        ),
                        _ => Ty::Any,
                    },
                    crate::ast::UnOp::BitNot => Ty::Int,
                }
            }
            Expr::Binary(op, l, r, _line) => {
                let lt = self.infer(l, env, hint);
                let rt = self.infer(r, env, hint);
                self.binop_ty(op, &lt, &rt, hint)
            }
            Expr::Ternary(_, a, b) => {
                let at = self.infer(a, env, hint);
                let bt = self.infer(b, env, hint);
                mk_union(vec![at, bt])
            }
            Expr::Index(t, i, _line) => {
                let tt = self.infer(t, env, hint);
                self.infer(i, env, hint);
                match tt {
                    Ty::List(el) => *el,
                    Ty::Map(_, v) => *v,
                    Ty::Str => Ty::Str,
                    Ty::Bytes => Ty::Int,
                    _ => Ty::Any,
                }
            }
            Expr::Member(x, k) => {
                let xt = self.infer(x, env, hint);
                self.member_ty(&xt, k, hint)
            }
            Expr::MemberSafe(x, k) => {
                let xt = self.infer(x, env, hint);
                let inner = self.member_ty(&xt, k, hint);
                mk_union(vec![inner, Ty::Null])
            }
            Expr::Method(recv, name, args, mline) => {
                // the method call carries its own line (dx parity with
                // Call/Index) — the flagship T02 diagnostic locates itself
                let line = if *mline > 0 { *mline } else { hint };
                let xt = self.infer(recv, env, line);
                let mut ats = Vec::new();
                for a in args {
                    ats.push(self.infer(a, env, line));
                }
                self.method_call_ty(&xt, name, &ats, line)
            }
            Expr::MethodSafe(recv, name, args, mline) => {
                let line = if *mline > 0 { *mline } else { hint };
                let xt = self.infer(recv, env, line);
                let mut ats = Vec::new();
                for a in args {
                    ats.push(self.infer(a, env, line));
                }
                let inner = self.method_call_ty(&xt, name, &ats, line);
                mk_union(vec![inner, Ty::Null])
            }
            Expr::Call(callee, args, call_line) => {
                let line = if *call_line > 0 { *call_line } else { hint };
                // named callee: gene / builtin / binding
                if let Expr::Ident(name) = &**callee {
                    let mut ats = Vec::new();
                    for a in args {
                        ats.push(self.infer(a, env, line));
                    }
                    return self.call_named(name, &ats, line);
                }
                self.infer(callee, env, line);
                for a in args {
                    self.infer(a, env, line);
                }
                Ty::Any
            }
            Expr::Propagate(x, _line) => {
                let xt = self.infer(x, env, hint);
                match xt {
                    Ty::OptionT(t) => *t,
                    Ty::ResultT(t, _) => *t,
                    Ty::Any => Ty::Any,
                    other => {
                        self.finding(
                            hint,
                            "T06",
                            "propagate-non-variant",
                            Sev::Warning,
                            format!(
                                "'?!' propagates Option/Result, but the operand is {}",
                                other.render()
                            ),
                        );
                        Ty::Any
                    }
                }
            }
            Expr::Collect {
                var,
                iter,
                filter,
                body,
            } => {
                let it = self.infer(iter, env, hint);
                let elem = match it {
                    Ty::List(el) => *el,
                    _ => Ty::Any,
                };
                env.push();
                env.bind(var.clone(), elem);
                if let Some(f) = filter {
                    self.infer(f, env, hint);
                }
                let bt = self.infer(body, env, hint);
                env.pop();
                Ty::List(Box::new(bt))
            }
            Expr::Lambda(g) => {
                // anonymous gene: check its body with an inline env
                let mut local = Env {
                    scopes: vec![Scope {
                        vars: HashMap::new(),
                    }],
                };
                for (i, (pname, _)) in g.params.iter().enumerate() {
                    let ty = g
                        .param_anns
                        .get(i)
                        .and_then(|a| a.as_ref())
                        .map(|a| self.ann_to_ty(a))
                        .unwrap_or(Ty::Any);
                    local.bind(pname.clone(), ty);
                }
                let mut dummy = Vec::new();
                self.check_stmts_ctx(&g.body, &mut local, &mut dummy, None, g.line);
                Ty::Gene
            }
            Expr::New(name, args) => {
                for a in args {
                    self.infer(a, env, hint);
                }
                Ty::Pheno(name.clone())
            }
            Expr::FateNew(_) => Ty::Any,
        }
    }

    /// Member (field or method-value) access on a known receiver.
    fn member_ty(&mut self, recv: &Ty, name: &str, hint: usize) -> Ty {
        match recv {
            Ty::Any | Ty::Never | Ty::Param(_) => Ty::Any,
            Ty::Map(_, _) => Ty::Any, // dynamic key space
            Ty::Pheno(pn) => {
                // field of a phenotype declared in this file?
                if let Some(p) = self.phenos.iter().find(|p| &p.name == pn) {
                    if p.fields.iter().any(|f| f == name) {
                        return Ty::Any; // field default exprs are inferable; Any is the lawful soft value
                    }
                    if pheno_method(pn, name, &self.phenos) {
                        return Ty::Gene; // method value
                    }
                    self.finding(
                        hint,
                        "T02",
                        "unknown-member",
                        Sev::Error,
                        format!("phenotype '{}' has no member '{}'", pn, name),
                    );
                    return Ty::Any;
                }
                Ty::Any // phenotype from a module: open
            }
            Ty::TraitObj(_) => Ty::Any,
            other => match method_hit(other, name, &[], &self.phenos) {
                MethodHit::Found(_) => Ty::Gene, // method value
                MethodHit::Missing => {
                    self.finding(
                        hint,
                        "T02",
                        "unknown-member",
                        Sev::Error,
                        format!(
                            "{} has no member '{}' (dynamic law: null with a note)",
                            other.render(),
                            name
                        ),
                    );
                    Ty::Any
                }
                MethodHit::Open => Ty::Any,
            },
        }
    }

    /// Method call on a receiver: surface + arity + return type.
    fn method_call_ty(&mut self, recv: &Ty, name: &str, args: &[Ty], hint: usize) -> Ty {
        match method_hit(recv, name, args, &self.phenos) {
            MethodHit::Found(t) => t,
            MethodHit::Missing => {
                self.finding(
                    hint,
                    "T02",
                    "unknown-member",
                    Sev::Error,
                    format!(
                        "{} has no method '{}'() (dynamic law: null with a note)",
                        recv.render(),
                        name
                    ),
                );
                Ty::Any
            }
            MethodHit::Open => Ty::Any,
        }
    }

    /// Named call: gene signature (with generic solving), builtin, or open.
    fn call_named(&mut self, name: &str, args: &[Ty], line: usize) -> Ty {
        // local binding that happens to be callable: open. The signature is
        // cloned up-front so finding/unify can borrow &mut self freely.
        let sig = match self.genes.get(name) {
            Some(s) if !s.inferring => s.clone(),
            Some(_) => return Ty::Any, // recursive call into a gene being inferred
            None => {
                return match builtin_sig(name, args) {
                    Some((_lo, _hi, ret)) => ret,
                    None => Ty::Any,
                };
            }
        };
        let mut solver: HashMap<String, Ty> = HashMap::new();
        // unify args against formal param annotations
        for (i, a) in args.iter().enumerate() {
            if let Some(Some(formal)) = sig.params.get(i) {
                self.unify_arg(a, formal, &mut solver, line, name, i);
            }
        }
        // bounds on solved parameters (T08)
        for (p, bound) in &sig.type_params {
            if let (Some(b), Some(solved)) = (bound, solver.get(p)) {
                if !satisfies_bound(solved, b, &self.phenos) {
                    self.finding(
                        line,
                        "T08",
                        "bound-violation",
                        Sev::Error,
                        format!(
                            "gene '{}' type parameter '{}' = {} violates bound '{}'",
                            name,
                            p,
                            solved.render(),
                            b
                        ),
                    );
                }
            }
        }
        // return type with the solver applied
        let base = sig.ret.unwrap_or(Ty::Any);
        self.apply_solver(&base, &solver)
    }

    /// Unify one argument against a formal type, solving type parameters.
    fn unify_arg(
        &mut self,
        actual: &Ty,
        formal: &Ty,
        solver: &mut HashMap<String, Ty>,
        line: usize,
        gene: &str,
        param_idx: usize,
    ) {
        match formal {
            Ty::Param(p) => match solver.get(p) {
                Some(prev) => {
                    let joined = mk_union(vec![prev.clone(), actual.clone()]);
                    solver.insert(p.clone(), joined);
                }
                None => {
                    solver.insert(p.clone(), actual.clone());
                }
            },
            Ty::List(f) => {
                if let Ty::List(a) = actual {
                    self.unify_arg(a, f, solver, line, gene, param_idx);
                } else if !ty_compat(actual, formal) {
                    self.arg_mismatch(actual, formal, line, gene, param_idx);
                }
            }
            Ty::OptionT(f) => {
                if let Ty::OptionT(a) = actual {
                    self.unify_arg(a, f, solver, line, gene, param_idx);
                } else if !ty_compat(actual, formal) {
                    self.arg_mismatch(actual, formal, line, gene, param_idx);
                }
            }
            Ty::ResultT(ft, fe) => {
                if let Ty::ResultT(at, ae) = actual {
                    self.unify_arg(at, ft, solver, line, gene, param_idx);
                    self.unify_arg(ae, fe, solver, line, gene, param_idx);
                } else if !ty_compat(actual, formal) {
                    self.arg_mismatch(actual, formal, line, gene, param_idx);
                }
            }
            Ty::Union(alts) => {
                // a formal union containing a parameter still checks the
                // non-parameter alternatives
                let hard: Vec<&Ty> = alts.iter().filter(|a| !matches!(a, Ty::Param(_))).collect();
                if hard.is_empty() {
                    for a in alts {
                        self.unify_arg(actual, a, solver, line, gene, param_idx);
                    }
                } else if !hard.iter().any(|f| ty_compat(actual, f)) {
                    self.arg_mismatch(actual, formal, line, gene, param_idx);
                }
            }
            _ => {
                if !ty_compat(actual, formal) {
                    self.arg_mismatch(actual, formal, line, gene, param_idx);
                }
            }
        }
    }

    fn arg_mismatch(&mut self, actual: &Ty, formal: &Ty, line: usize, gene: &str, idx: usize) {
        self.finding(
            line,
            "T03",
            "arg-type-mismatch",
            Sev::Error,
            format!(
                "argument {} of '{}' expects {}, got {}",
                idx + 1,
                gene,
                formal.render(),
                actual.render()
            ),
        );
    }

    fn apply_solver(&self, ty: &Ty, solver: &HashMap<String, Ty>) -> Ty {
        match ty {
            Ty::Param(p) => solver.get(p).cloned().unwrap_or(Ty::Any),
            Ty::List(t) => Ty::List(Box::new(self.apply_solver(t, solver))),
            Ty::Map(k, v) => Ty::Map(
                Box::new(self.apply_solver(k, solver)),
                Box::new(self.apply_solver(v, solver)),
            ),
            Ty::OptionT(t) => Ty::OptionT(Box::new(self.apply_solver(t, solver))),
            Ty::ResultT(t, e) => Ty::ResultT(
                Box::new(self.apply_solver(t, solver)),
                Box::new(self.apply_solver(e, solver)),
            ),
            Ty::Union(alts) => {
                mk_union(alts.iter().map(|a| self.apply_solver(a, solver)).collect())
            }
            other => other.clone(),
        }
    }

    /// Binary operator result type following the EXACT dynamic lattice of
    /// interp.rs apply_binop (TYPED-MODE.md §6). Impossible combinations
    /// (can only stress at runtime) are T03 findings.
    fn binop_ty(&mut self, op: &crate::ast::BinOp, lt: &Ty, rt: &Ty, hint: usize) -> Ty {
        use crate::ast::BinOp::*;
        // short-circuit/boolean families first
        match op {
            And | Or => return mk_union(vec![lt.clone(), rt.clone()]),
            Nullish => {
                // a ?? b: non-null side of a, or b
                let non_null = match lt {
                    Ty::OptionT(t) => (**t).clone(),
                    Ty::Union(alts) => mk_union(
                        alts.iter()
                            .filter(|a| !matches!(a, Ty::Null))
                            .cloned()
                            .collect(),
                    ),
                    Ty::Null => Ty::Never,
                    other => other.clone(),
                };
                return mk_union(vec![non_null, rt.clone()]);
            }
            Eq | Neq | Lt | Le | Gt | Ge | In => {
                // comparisons always produce bool; ordering ops on
                // non-comparable pairs are left to the runtime (Any today)
                if matches!(op, Lt | Le | Gt | Ge) {
                    let ok = |t: &Ty| {
                        matches!(
                            t,
                            Ty::Int
                                | Ty::Float
                                | Ty::Str
                                | Ty::Bytes
                                | Ty::Bool
                                | Ty::Any
                                | Ty::Never
                                | Ty::Param(_)
                        ) || matches!(t, Ty::Union(_))
                    };
                    if !ok(lt) || !ok(rt) {
                        self.finding(
                            hint,
                            "T03",
                            "arg-type-mismatch",
                            Sev::Error,
                            format!(
                                "ordering '{}' needs comparable operands, found {} and {}",
                                op_name(op),
                                lt.render(),
                                rt.render()
                            ),
                        );
                    }
                }
                return Ty::Bool;
            }
            _ => {}
        }
        // TYPED-MODE soft law: any operand typed Any (or a parameter) leaves
        // the result open — the dynamic side may still succeed; never flag.
        if matches!(lt, Ty::Any | Ty::Param(_) | Ty::Never)
            || matches!(rt, Ty::Any | Ty::Param(_) | Ty::Never)
        {
            return Ty::Any;
        }
        // arithmetic families
        let num_union = |a: &Ty, b: &Ty| -> Option<Ty> {
            // Int op Int -> Int; any Float -> Float; else None (impossible)
            match (a, b) {
                (Ty::Int, Ty::Int) => Some(Ty::Int),
                (Ty::Float, _) | (_, Ty::Float) => Some(Ty::Float),
                _ => None,
            }
        };
        match op {
            Add => {
                if let Some(t) = num_union(lt, rt) {
                    return t;
                }
                if matches!(lt, Ty::Str) && matches!(rt, Ty::Str) {
                    return Ty::Str;
                }
                if let (Ty::List(a), Ty::List(b)) = (lt, rt) {
                    return Ty::List(Box::new(mk_union(vec![(**a).clone(), (**b).clone()])));
                }
                self.impossible_binop(op, lt, rt, hint);
                Ty::Any
            }
            Sub => match num_union(lt, rt) {
                Some(t) => t,
                None => {
                    self.impossible_binop(op, lt, rt, hint);
                    Ty::Any
                }
            },
            Mul => {
                // str/bytes repetition (Python parity)
                if (matches!(lt, Ty::Str) && matches!(rt, Ty::Int))
                    || (matches!(lt, Ty::Int) && matches!(rt, Ty::Str))
                {
                    return Ty::Str;
                }
                if (matches!(lt, Ty::Bytes) && matches!(rt, Ty::Int))
                    || (matches!(lt, Ty::Int) && matches!(rt, Ty::Bytes))
                {
                    return Ty::Bytes;
                }
                match num_union(lt, rt) {
                    Some(t) => t,
                    None => {
                        self.impossible_binop(op, lt, rt, hint);
                        Ty::Any
                    }
                }
            }
            Div => {
                // ALWAYS float (as_floats), for any numeric pair
                if lt.is_numeric() && rt.is_numeric() {
                    Ty::Float
                } else {
                    self.impossible_binop(op, lt, rt, hint);
                    Ty::Any
                }
            }
            FloorDiv | Mod => {
                if matches!(lt, Ty::Int) && matches!(rt, Ty::Int) {
                    Ty::Int
                } else if lt.is_numeric() && rt.is_numeric() {
                    Ty::Float
                } else {
                    self.impossible_binop(op, lt, rt, hint);
                    Ty::Any
                }
            }
            Pow => {
                if matches!(lt, Ty::Int) && matches!(rt, Ty::Int) {
                    Ty::Int // non-negative exponent path; negative promotes
                } else if lt.is_numeric() && rt.is_numeric() {
                    Ty::Float
                } else {
                    self.impossible_binop(op, lt, rt, hint);
                    Ty::Any
                }
            }
            BitAnd | BitOr | BitXor | Shl | Shr => {
                let ok =
                    |t: &Ty| matches!(t, Ty::Int | Ty::Bool | Ty::Any | Ty::Never | Ty::Param(_));
                if ok(lt) && ok(rt) {
                    Ty::Int
                } else {
                    self.impossible_binop(op, lt, rt, hint);
                    Ty::Any
                }
            }
            _ => Ty::Any,
        }
    }

    fn impossible_binop(&mut self, op: &crate::ast::BinOp, lt: &Ty, rt: &Ty, hint: usize) {
        self.finding(
            hint,
            "T03",
            "arg-type-mismatch",
            Sev::Error,
            format!(
                "cannot apply '{}' to {} and {} (this pair can only stress at runtime)",
                op_name(op),
                lt.render(),
                rt.render()
            ),
        );
    }

    // ------------------------------------------------ exhaustiveness

    /// T05: variant/literal exhaustiveness per scrutinee type
    /// (TYPED-MODE.md §9). Guarded arms cover nothing; bind/wild cover all.
    fn check_exhaustive(&mut self, scrut: &Ty, arms: &[(MatchPat, Vec<Stmt>)], hint: usize) {
        let pats: Vec<&MatchPat> = arms.iter().map(|(p, _)| p).collect();
        let missing: Vec<String> = match scrut {
            Ty::OptionT(_) => {
                let mut m = Vec::new();
                if !pats.iter().any(|p| covers_tag(p, "Some")) {
                    m.push("Some(_)".into());
                }
                if !pats.iter().any(|p| covers_tag(p, "None")) {
                    m.push("None".into());
                }
                m
            }
            Ty::ResultT(_, _) => {
                let mut m = Vec::new();
                if !pats.iter().any(|p| covers_tag(p, "Ok")) {
                    m.push("Ok(_)".into());
                }
                if !pats.iter().any(|p| covers_tag(p, "Err")) {
                    m.push("Err(_)".into());
                }
                m
            }
            Ty::Bool => {
                let mut m = Vec::new();
                if !pats.iter().any(|p| covers_bool_pat(p, true)) {
                    m.push("true".into());
                }
                if !pats.iter().any(|p| covers_bool_pat(p, false)) {
                    m.push("false".into());
                }
                m
            }
            Ty::Union(alts) => {
                // each member must be coverable by some arm
                let mut m = Vec::new();
                for member in alts {
                    if !pats.iter().any(|p| pat_covers_ty(p, member)) {
                        m.push(member.render());
                    }
                }
                // bind/wild anywhere makes the whole thing covered
                if pats
                    .iter()
                    .any(|p| matches!(p, MatchPat::Wild | MatchPat::Bind(_)))
                {
                    m.clear();
                }
                m
            }
            _ => Vec::new(),
        };
        if !missing.is_empty() {
            self.finding(
                hint,
                "T05",
                "non-exhaustive-match",
                Sev::Error,
                format!(
                    "match over {} is not exhaustive: missing {}",
                    scrut.render(),
                    missing.join(", ")
                ),
            );
        }
    }

    // ------------------------------------------------ entry

    /// Check the top-level statement flow of the program (module scope).
    fn check_toplevel(&mut self, prog: &Program) {
        let mut env = Env {
            scopes: vec![Scope {
                vars: HashMap::new(),
            }],
        };
        let mut returns = Vec::new();
        for st in &prog.stmts {
            self.check_stmt(st, &mut env, &mut returns, None, 0);
        }
    }
}

fn op_name(op: &crate::ast::BinOp) -> &'static str {
    use crate::ast::BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        FloorDiv => "//",
        Mod => "%",
        Pow => "**",
        BitAnd => "&",
        BitOr => "|",
        BitXor => "^",
        Shl => "<<",
        Shr => ">>",
        Eq => "==",
        Neq => "!=",
        Lt => "<",
        Le => "<=",
        Gt => ">",
        Ge => ">=",
        And => "and",
        Or => "or",
        In => "in",
        Nullish => "??",
    }
}

/// Does this pattern cover the variant tag? (bind/wild cover everything;
/// guards cover nothing; or-patterns cover any alternative)
fn covers_tag(pat: &MatchPat, tag: &str) -> bool {
    match pat {
        MatchPat::Wild | MatchPat::Bind(_) => true,
        MatchPat::Variant(t, _) => t == tag,
        MatchPat::Or(alts) => alts.iter().any(|a| covers_tag(a, tag)),
        // guarded arms cover NOTHING (the condition may be false)
        MatchPat::Guard(_, _) => false,
        _ => false,
    }
}

/// Does this pattern cover the boolean literal `v`? (top-level fn: it
/// recurses through or/guard alternatives)
fn covers_bool_pat(pat: &MatchPat, v: bool) -> bool {
    match pat {
        MatchPat::Lit(Expr::Bool(b)) => *b == v,
        MatchPat::Or(alts) => alts.iter().any(|a| covers_bool_pat(a, v)),
        MatchPat::Guard(inner, _) => covers_bool_pat(inner, v),
        MatchPat::Wild | MatchPat::Bind(_) => true,
        _ => false,
    }
}

/// Could this pattern match a value of the given type? (union-member
/// coverage; conservative)
fn pat_covers_ty(pat: &MatchPat, ty: &Ty) -> bool {
    match pat {
        MatchPat::Wild | MatchPat::Bind(_) => true,
        MatchPat::Lit(e) => matches!(
            (e, ty),
            (Expr::Null, Ty::Null)
                | (Expr::Bool(_), Ty::Bool)
                | (Expr::Int(_), Ty::Int)
                | (Expr::Float(_), Ty::Float)
                | (Expr::Str(_), Ty::Str)
                | (Expr::Bytes(_), Ty::Bytes)
        ),
        MatchPat::Multi(lits) => lits
            .iter()
            .any(|l| pat_covers_ty(&MatchPat::Lit(l.clone()), ty)),
        MatchPat::Or(alts) => alts.iter().any(|a| pat_covers_ty(a, ty)),
        MatchPat::Guard(inner, _) => pat_covers_ty(inner, ty),
        MatchPat::Variant(tag, _) => match ty {
            Ty::OptionT(_) => tag == "Some" || tag == "None",
            Ty::ResultT(_, _) => tag == "Ok" || tag == "Err",
            _ => false,
        },
        MatchPat::ListPat { .. } => matches!(ty, Ty::List(_)),
        MatchPat::MapPat { .. } => matches!(ty, Ty::Map(_, _)),
    }
}

// ================================================================= entry

/// Run the static type check over a parsed program. Returns the T-series
/// findings in source order (line 0 findings last for stable output).
pub fn check_program(prog: &Program) -> Vec<Finding> {
    let mut c = Checker::new();
    c.collect_program(prog);
    c.infer_gene_bodies(prog);
    c.check_toplevel(prog);
    let mut out = c.findings;
    out.sort_by_key(|f| f.line);
    out
}

// ================================================================= env

pub struct Env {
    scopes: Vec<Scope>,
}

impl Env {
    pub fn bind(&mut self, name: String, ty: Ty) {
        if let Some(s) = self.scopes.last_mut() {
            s.vars.insert(name, ty);
        }
    }
    pub fn rebind(&mut self, name: String, ty: Ty) {
        // widest scope wins dynamically; typed mode widens the innermost
        // visible binding (documented: assignment re-types in dynamic law,
        // static mode tracks the observed type forward)
        for s in self.scopes.iter_mut().rev() {
            if let Some(slot) = s.vars.get_mut(&name) {
                *slot = ty;
                return;
            }
        }
        self.bind(name, ty);
    }
    pub fn get(&self, name: &str) -> Option<Ty> {
        for s in self.scopes.iter().rev() {
            if let Some(t) = s.vars.get(name) {
                return Some(t.clone());
            }
        }
        None
    }
    pub fn push(&mut self) {
        self.scopes.push(Scope {
            vars: HashMap::new(),
        });
    }
    pub fn pop(&mut self) {
        self.scopes.pop();
    }
}

// ================================================================= helpers

/// First stamped line in an expression tree (A13 spans: Call/Index/Binary/
/// Propagate carry their source line). Best-effort statement lines.
pub fn expr_line_deep(e: &Expr) -> Option<usize> {
    match e {
        Expr::Call(_, args, l) => Some(args.iter().find_map(expr_line_deep).unwrap_or(*l)),
        Expr::Index(_, _, l) => Some(*l),
        Expr::Binary(_, l, r, ln) => expr_line_deep(l)
            .or_else(|| expr_line_deep(r))
            .or(Some(*ln)),
        Expr::Propagate(_, l) => Some(*l),
        Expr::Unary(_, x) => expr_line_deep(x),
        Expr::Member(x, _) => expr_line_deep(x),
        Expr::MemberSafe(x, _) => expr_line_deep(x),
        Expr::Method(x, _, args, l) => expr_line_deep(x)
            .or_else(|| args.iter().find_map(expr_line_deep))
            .or(Some(*l)),
        Expr::MethodSafe(x, _, args, l) => expr_line_deep(x)
            .or_else(|| args.iter().find_map(expr_line_deep))
            .or(Some(*l)),
        Expr::Ternary(c, a, b) => expr_line_deep(c)
            .or_else(|| expr_line_deep(a))
            .or_else(|| expr_line_deep(b)),
        Expr::List(items) => items.iter().find_map(expr_line_deep),
        Expr::Map(pairs) => pairs
            .iter()
            .find_map(|(k, v)| expr_line_deep(k).or_else(|| expr_line_deep(v))),
        _ => None,
    }
}

/// Nested statement bodies of a statement (checker-facing subset).
fn stmt_bodies(st: &Stmt) -> Vec<&[Stmt]> {
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
        | Stmt::For(_, _, b)
        | Stmt::ForPat(_, _, b)
        | Stmt::Tad(_, b) => vec![b.as_slice()],
        Stmt::Match(_, arms, _) => arms.iter().map(|(_, b)| &b[..]).collect(),
        Stmt::Stress { body, rescue, .. } => {
            let mut out = vec![body.as_slice()];
            if let Some((_, rb)) = rescue {
                out.push(rb.as_slice());
            }
            out
        }
        Stmt::Frame { body, .. } => vec![body.as_slice()],
        Stmt::Module(_, b) => vec![b.as_slice()],
        _ => vec![],
    }
}

fn collect_gene_defs(st: &Stmt, out: &mut Vec<Arc<crate::ast::GeneDef>>) {
    match st {
        Stmt::Gene(g) | Stmt::Seq(g) => {
            out.push(g.clone());
            for s in &g.body {
                collect_gene_defs(s, out);
            }
        }
        Stmt::Pheno(p) => {
            for m in &p.methods {
                out.push(m.clone());
            }
        }
        Stmt::Trait(t) => {
            for m in &t.methods {
                if let Some(d) = &m.default {
                    out.push(d.clone());
                }
            }
        }
        Stmt::Splice(s) => {
            for (_, g) in &s.variants {
                out.push(g.clone());
            }
        }
        other => {
            for nested in stmt_bodies(other) {
                for s in nested {
                    collect_gene_defs(s, out);
                }
            }
        }
    }
}

fn collect_traits_stmt(st: &Stmt, table: &mut HashMap<(String, String), bool>) {
    match st {
        Stmt::Trait(t) => {
            for m in &t.methods {
                table.insert((t.name.clone(), m.name.clone()), m.required);
            }
        }
        other => {
            for nested in stmt_bodies(other) {
                for s in nested {
                    collect_traits_stmt(s, table);
                }
            }
        }
    }
}

fn trait_declared(name: &str) -> bool {
    TRAIT_TABLE.with(|t| t.borrow().keys().any(|(tn, _)| tn == name))
}

fn trait_required_methods(name: &str) -> Vec<String> {
    TRAIT_TABLE.with(|t| {
        t.borrow()
            .iter()
            .filter(|((tn, _), required)| tn == name && **required)
            .map(|((_, m), _)| m.clone())
            .collect()
    })
}

fn parent_has_method(parent: &Option<String>, mname: &str, phenos: &[PhenoInfo]) -> bool {
    let mut cur = parent.clone();
    let mut hops = 0;
    while let Some(c) = cur {
        if let Some(p) = phenos.iter().find(|p| p.name == c) {
            if p.own_methods.iter().any(|m| m == mname) {
                return true;
            }
            hops += 1;
            if hops > 64 {
                return false;
            }
            cur = p.parent.clone();
        } else {
            return false;
        }
    }
    false
}

/// primitive annotation name -> Ty (case-normalized)
fn named_to_ty(n: &str, args: &[Ty]) -> Ty {
    match n.to_ascii_lowercase().as_str() {
        "any" => Ty::Any,
        "null" => Ty::Null,
        "bool" => Ty::Bool,
        "int" => Ty::Int,
        "float" => Ty::Float,
        "str" => Ty::Str,
        "bytes" => Ty::Bytes,
        "list" => match args {
            [] => Ty::List(Box::new(Ty::Any)),
            [t] => Ty::List(Box::new(t.clone())),
            _ => Ty::List(Box::new(Ty::Any)),
        },
        "map" => match args {
            [] => Ty::Map(Box::new(Ty::Any), Box::new(Ty::Any)),
            [k, v] => Ty::Map(Box::new(k.clone()), Box::new(v.clone())),
            _ => Ty::Map(Box::new(Ty::Any), Box::new(Ty::Any)),
        },
        "gene" => Ty::Gene,
        "sequence" => Ty::Seq,
        "channel" => Ty::Channel,
        "weak" => Ty::Weak,
        "phenotype" => Ty::Pheno("any".into()),
        "option" => match args {
            [t] => Ty::OptionT(Box::new(t.clone())),
            _ => Ty::OptionT(Box::new(Ty::Any)),
        },
        "result" => match args {
            [t, e] => Ty::ResultT(Box::new(t.clone()), Box::new(e.clone())),
            [t] => Ty::ResultT(Box::new(t.clone()), Box::new(Ty::Any)),
            _ => Ty::ResultT(Box::new(Ty::Any), Box::new(Ty::Any)),
        },
        // unknown: capitalized names resolve to a phenotype or trait guess
        // (checked against declared shapes by the caller); here: Any.
        _ => Ty::Any,
    }
}

/// generic head annotation -> Ty (structural option/result/list/map)
fn generic_to_ty(head: &str, args: &[Ty]) -> Ty {
    named_to_ty(head, args)
}

/// pattern binding for a match arm: (name, type) for the arm scope.
fn pat_binding(pat: &MatchPat, scrut: &Ty) -> Option<(String, Ty)> {
    match pat {
        MatchPat::Bind(n) => Some((n.clone(), scrut.clone())),
        MatchPat::Variant(tag, payload) => {
            // Some(p)/Ok(p) bind the payload type; None/Err tag-only forms
            // bind the whole variant (None binds Null-ish Any)
            let inner = match (tag.as_str(), scrut) {
                ("Some", Ty::OptionT(t)) => (**t).clone(),
                ("Some", _) => Ty::Any,
                ("Ok", Ty::ResultT(t, _)) => (**t).clone(),
                ("Ok", _) => Ty::Any,
                ("Err", Ty::ResultT(_, e)) => (**e).clone(),
                ("Err", _) => Ty::Any,
                _ => Ty::Any,
            };
            match payload {
                Some(p) => pat_binding(p, &inner),
                None => None, // tag-only: no binding
            }
        }
        MatchPat::Or(alts) => alts.iter().find_map(|a| pat_binding(a, scrut)),
        MatchPat::Guard(inner, _) => pat_binding(inner, scrut),
        MatchPat::ListPat { elems, rest } => {
            let el = match scrut {
                Ty::List(t) => (**t).clone(),
                _ => Ty::Any,
            };
            let _ = elems;
            rest.clone().map(|r| (r, Ty::List(Box::new(el))))
        }
        MatchPat::MapPat { keys } => keys.first().map(|(k, _)| (k.clone(), Ty::Any)),
        _ => None,
    }
}

/// destructuring let/for binding (soft: types degrade to Any fast)
fn bind_pat(pat: &Pat, ty: Ty, env: &mut Env) {
    match pat {
        Pat::Bind(n) => env.bind(n.clone(), ty),
        Pat::List { elems, rest } => {
            let el = match ty {
                Ty::List(t) => *t,
                _ => Ty::Any,
            };
            for e in elems {
                bind_pat(e, el.clone(), env);
            }
            if let Some(r) = rest {
                env.bind(r.clone(), Ty::List(Box::new(el)));
            }
        }
        Pat::Map { keys } => {
            for k in keys {
                env.bind(k.clone(), Ty::Any);
            }
        }
    }
}
