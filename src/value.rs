//! value.rs — runtime values, display, truthiness, comparison, deep equality.

use crate::ast::GeneDef;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::Arc;

pub type ListRef = Rc<RefCell<Vec<Value>>>;
pub type MapRef = Rc<RefCell<Vec<(Value, Value)>>>;
pub type EnvRef = Rc<crate::interp::Env>;

/// Message a sequence worker sends to its consumer over the rendezvous channel.
pub enum SeqMsg {
    Yield(crate::genes::SendValue),
    Done(Vec<crate::ast::Note>, Option<(String, String)>), // notes, stress(kind,msg)
}

/// Lazily-pulled sequence state shared by the consumer side.
pub struct SeqState {
    pub rx: Option<std::sync::mpsc::Receiver<SeqMsg>>,
    pub done: bool,
    pub stress: Option<(String, String)>,
}

#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(ListRef),
    Map(MapRef),
    Gene(Arc<GeneDef>, Option<EnvRef>),
    Seq(Arc<GeneDef>, Rc<RefCell<SeqState>>),
    Obj(Arc<crate::ast::PhenoDef>, MapRef),
}

pub struct Stress {
    pub kind: String,   // unfolded | missing | overflow | burned | interference
    pub message: String,
}

impl Stress {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Stress { kind: kind.to_string(), message: message.into() }
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Gene(_, _) => "gene",
            Value::Seq(_, _) => "sequence",
            Value::Obj(_, _) => "phenotype",
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.borrow().is_empty(),
            Value::Map(m) => !m.borrow().is_empty(),
            Value::Gene(_, _) | Value::Seq(_, _) | Value::Obj(_, _) => true,
        }
    }

    /// Human display (promote / interpolation): strings appear bare.
    pub fn display(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            other => other.repr(),
        }
    }

    /// Structural display (inside containers): strings quoted.
    pub fn repr(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(true) => "true".into(),
            Value::Bool(false) => "false".into(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => format_float(*f),
            Value::Str(s) => format!("\"{}\"", escape_str(s)),
            Value::List(l) => {
                let items: Vec<String> = l.borrow().iter().map(|v| v.repr()).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Map(m) => {
                let items: Vec<String> = m
                    .borrow()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", key_repr(k), v.repr()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            Value::Gene(d, _) => match &d.name {
                Some(n) => format!("<gene {}>", n),
                None => "<gene lambda>".into(),
            },
            Value::Seq(d, _) => match &d.name {
                Some(n) => format!("<sequence {}>", n),
                None => "<sequence lambda>".into(),
            },
            Value::Obj(d, _) => format!("<phenotype {}>", d.name),
        }
    }

    /// Deep equality (maps order-insensitive).
    pub fn deep_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => (*a as f64) == *b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => {
                let la = a.borrow();
                let lb = b.borrow();
                la.len() == lb.len() && la.iter().zip(lb.iter()).all(|(x, y)| x.deep_eq(y))
            }
            (Value::Map(a), Value::Map(b)) => {
                let ma = a.borrow();
                let mb = b.borrow();
                if ma.len() != mb.len() {
                    return false;
                }
                ma.iter().all(|(k, v)| {
                    mb.iter().any(|(k2, v2)| k.deep_eq(k2) && v.deep_eq(v2))
                })
            }
            (Value::Gene(d1, _), Value::Gene(d2, _)) => Arc::ptr_eq(d1, d2),
            (Value::Seq(d1, _), Value::Seq(d2, _)) => Arc::ptr_eq(d1, d2),
            (Value::Obj(d1, _), Value::Obj(d2, _)) => Arc::ptr_eq(d1, d2),
            _ => false,
        }
    }
}

/// Python-compatible float repr: shortest round-trip digits; scientific
/// notation when the decimal exponent is < -4 or >= 16; ".0" suffix on
/// integral positional values. Matches CPython repr() so the Rust core and
/// the Python oracle print identical text.
pub fn format_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let s = format!("{:e}", f); // shortest digits, e.g. "1.2345e3", "5e-7"
    let (mant, exp_txt) = match s.split_once('e') {
        Some(p) => p,
        None => return s,
    };
    let exp: i32 = exp_txt.parse().unwrap_or(0);
    if exp < -4 || exp >= 16 {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{}{:02}", mant, sign, exp.abs())
    } else {
        let m = format!("{}", f); // shortest positional form
        if m.contains('.') {
            m
        } else {
            format!("{}.0", m)
        }
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out
}

fn key_repr(k: &Value) -> String {
    match k {
        Value::Str(s) if is_identlike(s) => s.clone(),
        other => other.repr(),
    }
}

fn is_identlike(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !s.chars().next().unwrap().is_ascii_digit()
}

/// Convert a value to a map key scalar (used by map literals / index).
pub fn key_scalar(v: &Value) -> Option<Value> {
    match v {
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_) => {
            Some(v.clone())
        }
        _ => None,
    }
}
