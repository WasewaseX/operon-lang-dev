//! types.rs — the Operon static type lattice (TYPED-MODE.md §3).
//!
//! Pure representation and laws: the `Ty` lattice, assignability
//! (`ty_compat`), unions, generic bounds, the builtin method surface, and
//! the trait tables. No AST walking happens here — the program pass that
//! produces findings lives in `typeck.rs`.
//!
//! Design law (TYPED-MODE.md §1): additive. The dynamic side is untouched;
//! these types exist only to power the compile-time gate.

use crate::ast::{Expr, MatchPat};
use std::collections::HashMap;

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
pub fn builtin_sig(name: &str, args: &[Ty]) -> Option<(usize, usize, Ty)> {
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

pub fn abs_ret(arg: Option<&Ty>) -> Ty {
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
pub fn pheno_method(pn: &str, mname: &str, phenos: &[PhenoInfo]) -> bool {
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
    pub static TRAIT_TABLE: std::cell::RefCell<HashMap<(String, String), bool>> =
        std::cell::RefCell::new(HashMap::new());
}

// =================================================================== binop + pattern coverage laws
pub fn op_name(op: &crate::ast::BinOp) -> &'static str {
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
pub fn covers_tag(pat: &MatchPat, tag: &str) -> bool {
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
pub fn covers_bool_pat(pat: &MatchPat, v: bool) -> bool {
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
pub fn pat_covers_ty(pat: &MatchPat, ty: &Ty) -> bool {
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
// ==================================================================== trait tables + named-type constructors
pub fn trait_declared(name: &str) -> bool {
    TRAIT_TABLE.with(|t| t.borrow().keys().any(|(tn, _)| tn == name))
}

pub fn trait_required_methods(name: &str) -> Vec<String> {
    TRAIT_TABLE.with(|t| {
        t.borrow()
            .iter()
            .filter(|((tn, _), required)| tn == name && **required)
            .map(|((_, m), _)| m.clone())
            .collect()
    })
}

pub fn parent_has_method(parent: &Option<String>, mname: &str, phenos: &[PhenoInfo]) -> bool {
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
pub fn named_to_ty(n: &str, args: &[Ty]) -> Ty {
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
pub fn generic_to_ty(head: &str, args: &[Ty]) -> Ty {
    named_to_ty(head, args)
}
