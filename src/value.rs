//! value.rs, runtime values, display, truthiness, comparison, deep equality.

use crate::ast::GeneDef;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::sync::Arc;

pub type ListRef = Rc<RefCell<Vec<Value>>>;

/// dx-r3 (re-audit perf #7): maps keep their insertion-ordered Vec (repr,
/// keys(), iteration order are part of the language contract) but gain a
/// hash memo over (type tag, display) -> position, so the hot lookups,
/// `m[k]`, `has`, `del`, member access, `map_insert`, are O(1) instead of
/// a linear deep_eq scan (the collections bench ran 40–55x CPython).
/// The memo is a PREFILTER: candidate positions are always verified with
/// deep_eq, so exotic equalities stay exact.
#[derive(Default)]
pub struct MapStore {
    pub items: Vec<(Value, Value)>,
    memo: std::collections::HashMap<(u8, String), usize>,
}

/// sec-r5 (F-12): non-scalar keys (lists/maps) miss the hash memo and fall
/// back to a linear deep_eq scan. Unbounded, that was quadratic CPU that
/// burned zero fuel, 25k list-keyed inserts was a live hang. The scan is
/// now capped: beyond this many entries a non-scalar key is treated as
/// absent (SPEC §9b). Scalar keys keep exact semantics via the memo.
const NON_SCALAR_SCAN_CAP: usize = 512;

fn key_tag(v: &Value) -> (u8, String) {
    match v {
        Value::Null => (0, String::new()),
        Value::Bool(b) => (1, b.to_string()),
        Value::Int(i) => (2, i.to_string()),
        Value::Float(f) => (3, f.to_string()),
        Value::Str(s) => (4, s.clone()),
        // W029: bytes keys hit the memo via a length-tagged prefilter, the
        // memo is only a PREFILTER (candidates are deep_eq-verified and a
        // miss falls back to the exact full scan), so same-length collisions
        // stay exact, just O(n) instead of O(1).
        Value::Bytes(b) => (5, format!("b{}", b.len())),
        // non-scalar keys are legal but rare, they simply miss the memo
        // and fall back to the linear scan
        _ => (255, String::new()),
    }
}

impl MapStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn from_vec(items: Vec<(Value, Value)>) -> Self {
        let mut s = MapStore {
            items,
            memo: std::collections::HashMap::new(),
        };
        s.rebuild();
        s
    }
    pub fn rebuild(&mut self) {
        self.memo.clear();
        for (i, (k, _)) in self.items.iter().enumerate() {
            let tag = key_tag(k);
            if tag.0 != 255 {
                self.memo.insert(tag, i); // last write wins = deep_eq semantics
            }
        }
    }
    /// Exact position of `key` (deep_eq verified), O(1) for scalar keys.
    pub fn position(&self, key: &Value) -> Option<usize> {
        let tag = key_tag(key);
        if tag.0 != 255 {
            if let Some(&i) = self.memo.get(&tag) {
                if let Some((k, _)) = self.items.get(i) {
                    if k.deep_eq(key) {
                        return Some(i);
                    }
                }
            }
            // scalar keys keep exact semantics: full scan on memo miss
            return self.items.iter().position(|(k, _)| k.deep_eq(key));
        }
        // sec-r5 (F-12): non-scalar keys, bounded scan (see NON_SCALAR_SCAN_CAP)
        self.items
            .iter()
            .take(NON_SCALAR_SCAN_CAP)
            .position(|(k, _)| k.deep_eq(key))
    }
    /// Upsert preserving insertion order (existing key keeps its position).
    pub fn insert(&mut self, key: Value, val: Value) {
        if let Some(i) = self.position(&key) {
            self.items[i].1 = val;
            return;
        }
        let tag = key_tag(&key);
        self.items.push((key, val));
        if tag.0 != 255 {
            self.memo.insert(tag, self.items.len() - 1);
        }
    }
    /// Delete by key; returns true when something was removed. Positions
    /// after the removed slot shift, so the memo is rebuilt.
    pub fn del(&mut self, key: &Value) -> bool {
        match self.position(key) {
            Some(i) => {
                self.items.remove(i);
                self.rebuild();
                true
            }
            None => false,
        }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    /// Vec-compatible passthroughs: existing call sites keep compiling.
    pub fn iter(&self) -> std::slice::Iter<'_, (Value, Value)> {
        self.items.iter()
    }
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, (Value, Value)> {
        self.items.iter_mut()
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.memo.clear();
    }
    pub fn extend<I: IntoIterator<Item = (Value, Value)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
    pub fn get(&self, i: usize) -> Option<&(Value, Value)> {
        self.items.get(i)
    }
}

impl std::ops::Index<usize> for MapStore {
    type Output = (Value, Value);
    fn index(&self, i: usize) -> &(Value, Value) {
        &self.items[i]
    }
}

impl FromIterator<(Value, Value)> for MapStore {
    fn from_iter<I: IntoIterator<Item = (Value, Value)>>(iter: I) -> Self {
        MapStore::from_vec(iter.into_iter().collect())
    }
}

pub type MapRef = Rc<RefCell<MapStore>>;
pub type EnvRef = Rc<crate::interp::Env>;

/// W015: shared channel state. The buffer holds the WIRE form (SendValue),
/// the same serialization every value crosses a thread boundary through, so
/// a channel is a thread-safe conduit by construction: nothing aliased ever
/// sits in the queue (SPEC §13, §19d). Unbounded FIFO by design; growth is
/// charged to the aggregate allocation ceiling at send time. The handle
/// type (Value::Channel) itself stays non-Send like every other Value, it
/// crosses spawn boundaries only through the snapshot's dedicated live
/// handle lane (SnapVal::Channel), never through data serialization.
pub struct ChannelShared {
    pub state: std::sync::Mutex<ChanState>,
    pub wake: std::sync::Condvar,
}

pub struct ChanState {
    pub queue: std::collections::VecDeque<crate::genes::SendValue>,
    pub closed: bool,
}

impl ChannelShared {
    pub fn new() -> Self {
        ChannelShared {
            state: std::sync::Mutex::new(ChanState {
                queue: std::collections::VecDeque::new(),
                closed: false,
            }),
            wake: std::sync::Condvar::new(),
        }
    }
}

impl Default for ChannelShared {
    fn default() -> Self {
        Self::new()
    }
}

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

/// W06 (D-014): the four Option/Result variant tags. Option = Some|None,
/// Result = Ok|Err, families are distinct (Some(x) != Ok(x)) so a value
/// always remembers which contract it carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VTag {
    SomeV,
    NoneV,
    OkV,
    ErrV,
}

impl VTag {
    /// Option family (Some/None) vs Result family (Ok/Err).
    pub fn family(self) -> &'static str {
        match self {
            VTag::SomeV | VTag::NoneV => "option",
            VTag::OkV | VTag::ErrV => "result",
        }
    }
    pub fn tag_name(self) -> &'static str {
        match self {
            VTag::SomeV => "Some",
            VTag::NoneV => "None",
            VTag::OkV => "Ok",
            VTag::ErrV => "Err",
        }
    }
}

#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// W029: first-class immutable bytes. Immutability keeps the memory
    /// model simple (no frozen interplay) and matches Python bytes; sharing
    /// is Rc (assignment shares, like lists, SPEC §19a table row).
    Bytes(Rc<Vec<u8>>),
    List(ListRef),
    Map(MapRef),
    Gene(Arc<GeneDef>, Option<EnvRef>),
    Seq(Arc<GeneDef>, Rc<RefCell<SeqState>>),
    Obj(Arc<crate::ast::PhenoDef>, MapRef),
    /// W06 (D-014): first-class Option/Result variants. NoneV carries no
    /// payload; the other three always do.
    Variant(VTag, Option<Box<Value>>),
    /// W015: a channel handle (unbounded FIFO buffer, created empty). The
    /// handle is a behavior value like a gene: it never serializes through
    /// the data membrane (send refuses it as a payload, nested handles
    /// degrade to null), it crosses spawn boundaries LIVE via the snapshot
    /// handle lane, and identity (Arc::ptr_eq) is the equality rule.
    Channel(Arc<ChannelShared>),
    /// W013 (D-013): an opaque weak handle (the `weak()` builtin). Holds a
    /// WEAK reference to the target's backing store, it does NOT keep the
    /// value alive (there is no tracing GC, an Rc value with zero strong
    /// refs is freed immediately, so `strengthen` after the last strong ref
    /// returns null). Like a channel handle it never serializes through the
    /// data membrane (nested handles degrade to null, send refuses it at
    /// the top level) and its repr is address-free so the two cores agree.
    Weak(WeakHandle),
}

/// W013: the backing store behind a `weak()` handle. Phenotype instances
/// carry the (shared, per-class) definition Arc so `strengthen` can rebuild
/// the SAME instance (same fields Rc) while the instance is still alive;
/// holding the definition never keeps an instance alive.
#[derive(Clone)]
pub enum WeakHandle {
    List(std::rc::Weak<RefCell<Vec<Value>>>),
    Map(std::rc::Weak<RefCell<MapStore>>),
    Obj(std::rc::Weak<RefCell<MapStore>>, Arc<crate::ast::PhenoDef>),
}

impl WeakHandle {
    /// The referenced value, or None once the last strong reference is gone
    /// (no GC: freeing is immediate at strong-count zero, never deferred).
    pub fn upgrade_value(&self) -> Option<Value> {
        match self {
            WeakHandle::List(w) => w.upgrade().map(Value::List),
            WeakHandle::Map(w) => w.upgrade().map(Value::Map),
            WeakHandle::Obj(w, d) => w.upgrade().map(|m| Value::Obj(d.clone(), m)),
        }
    }

    /// Identity rule for weak handles (mirrors the channel rule): same
    /// target address. Two handles compare equal iff they point at the same
    /// allocation; a dead handle keeps its address so the comparison stays
    /// total, and a dead target is unobservable past `strengthen` (null).
    fn same_target(&self, other: &WeakHandle) -> bool {
        match (self, other) {
            (WeakHandle::List(a), WeakHandle::List(b)) => Weak::as_ptr(a) == Weak::as_ptr(b),
            (WeakHandle::Map(a), WeakHandle::Map(b)) => Weak::as_ptr(a) == Weak::as_ptr(b),
            (WeakHandle::Obj(a, _), WeakHandle::Obj(b, _)) => Weak::as_ptr(a) == Weak::as_ptr(b),
            _ => false,
        }
    }
}

pub struct Stress {
    pub kind: String, // unfolded | missing | overflow | burned | interference | unwrap
    pub message: String,
    /// dx-r3 (re-audit): source line the hard error originated from, the
    /// primary diagnostic gets a location, matching mainstream norms.
    pub line: usize,
    /// W007: gene call chain, captured as the stress unwinds through the
    /// call funnel, INNERMOST frame first, (gene name, call-site line).
    /// Rendered on uncaught stress (main.rs) and exposed on rescue bindings
    /// via stress_map ("chain" key). Capped at 64 frames (note-cap
    /// discipline): a bounded chain is a contained chain.
    pub chain: Vec<(String, usize)>,
    /// W06 (D-014): propagation marker. Some(_) means this Stress is NOT a
    /// failure, it is a `?!` propagation unwinding to the nearest enclosing
    /// gene boundary, carrying the variant value to return. The payload is
    /// the marker itself: no user path (raise/stress statements, builtins)
    /// can construct a Stress with a payload, so rescue can never catch or
    /// spoof propagation. Every catch site must convert payload-carrying
    /// Stress into Flow::Ret BEFORE kind matching.
    pub prop: Option<Value>,
}

impl Stress {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Stress {
            kind: kind.to_string(),
            message: message.into(),
            line: 0,
            chain: Vec::new(),
            prop: None,
        }
    }
    /// dx-r3: a located hard error (call sites inside eval stamp cur_line).
    pub fn at(line: usize, kind: &str, message: impl Into<String>) -> Self {
        Stress {
            kind: kind.to_string(),
            message: message.into(),
            line,
            chain: Vec::new(),
            prop: None,
        }
    }
    /// W06 (D-014): a propagation signal, a variant value unwinding to the
    /// nearest enclosing gene boundary, where it becomes the gene's return
    /// value. Never contained by rescue (catch sites pre-arm on `prop`).
    pub fn prop(line: usize, value: Value) -> Self {
        Stress {
            kind: "propagate".to_string(),
            message: String::new(),
            line,
            chain: Vec::new(),
            prop: Some(value),
        }
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
            Value::Bytes(_) => "bytes",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Gene(_, _) => "gene",
            Value::Seq(_, _) => "sequence",
            Value::Obj(_, _) => "phenotype",
            Value::Variant(t, _) => t.family(),
            Value::Channel(_) => "channel",
            // W013: the handle type is its own name; the TARGET's type is
            // not leaked (the handle may outlive the target).
            Value::Weak(_) => "weak",
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Bytes(b) => !b.is_empty(),
            Value::List(l) => !l.borrow().is_empty(),
            Value::Map(m) => !m.borrow().is_empty(),
            Value::Gene(_, _) | Value::Seq(_, _) | Value::Obj(_, _) => true,
            // W015: a channel handle is truthy like the other behavior
            // handles (genes, sequences, phenotypes).
            Value::Channel(_) => true,
            // W013: a weak handle is truthy like any other handle, it says
            // nothing about whether the target is still alive.
            Value::Weak(_) => true,
            // W06: a carried success is truthy; a carried failure is falsy,
            // `if (result)` reads naturally without unwrapping.
            Value::Variant(VTag::SomeV, _) | Value::Variant(VTag::OkV, _) => true,
            Value::Variant(VTag::NoneV, _) | Value::Variant(VTag::ErrV, _) => false,
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
    /// Cycle-safe: a container that (transitively) contains itself renders
    /// with a `[...]` / `{...}` marker (CPython behavior), never recurses
    /// forever. Depth is capped too, so very deep (non-cyclic) nesting
    /// degrades gracefully instead of exhausting the native stack.
    /// sec-r5 (F-10): the visited set is NOT unwound on exit, unwinding
    /// made DAG-shaped values (l=[l,l] chains) re-walk exponentially (a
    /// 45-deep chain is 2^45 node visits: a live hang). Memoized: a shared
    /// subtree renders once; later references render the cycle marker.
    pub fn repr(&self) -> String {
        let mut seen: HashSet<usize> = HashSet::new();
        self.repr_g(&mut seen, 0)
    }

    fn repr_g(&self, seen: &mut HashSet<usize>, depth: u32) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(true) => "true".into(),
            Value::Bool(false) => "false".into(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => format_float(*f),
            Value::Str(s) => format!("\"{}\"", escape_str(s)),
            // W029: bytes repr mirrors mainstream b"..." spelling, printable
            // ASCII raw, the C escape set short-form, everything else \xNN.
            Value::Bytes(b) => format!("b\"{}\"", escape_bytes(b)),
            Value::List(l) => {
                let id = Rc::as_ptr(l) as *const u8 as usize;
                if depth > 256 || !seen.insert(id) {
                    return "[...]".into();
                }
                let items: Vec<String> = l
                    .borrow()
                    .iter()
                    .map(|v| v.repr_g(seen, depth + 1))
                    .collect();
                // sec-r5 (F-10): visited id stays, memoized DAG containment
                format!("[{}]", items.join(", "))
            }
            Value::Map(m) => {
                let id = Rc::as_ptr(m) as *const u8 as usize;
                if depth > 256 || !seen.insert(id) {
                    return "{...}".into();
                }
                let items: Vec<String> = m
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}: {}",
                            key_repr_g(k, seen, depth),
                            v.repr_g(seen, depth + 1)
                        )
                    })
                    .collect();
                // sec-r5 (F-10): visited id stays, memoized DAG containment
                format!("{{{}}}", items.join(", "))
            }
            Value::Gene(d, _) => match &d.name {
                Some(n) => format!("<gene {}>", n),
                None => "<gene lambda>".into(),
            },
            // W06: variant repr mirrors mainstream constructor syntax; the
            // payload renders through repr_g so depth/cycle caps apply.
            Value::Variant(VTag::NoneV, _) => "None".into(),
            Value::Variant(t, Some(p)) => {
                format!("{}({})", t.tag_name(), p.repr_g(seen, depth + 1))
            }
            // a Some/Ok/Err with no payload cannot be constructed (builtins
            // enforce arity); render defensively rather than panic.
            Value::Variant(t, None) => t.tag_name().into(),
            Value::Seq(d, _) => match &d.name {
                Some(n) => format!("<sequence {}>", n),
                None => "<sequence lambda>".into(),
            },
            Value::Obj(d, _) => format!("<phenotype {}>", d.name),
            // W015: address-free repr on purpose. A pointer-bearing repr
            // would make `print(ch)` diverge between the two cores (and
            // between runs); channels render anonymously like lambdas do.
            Value::Channel(_) => "<channel>".into(),
            // W013: address-free for the same reason; the target's identity
            // is observable only through strengthen(), never through repr.
            Value::Weak(_) => "<weak>".into(),
        }
    }

    /// Deep equality (maps order-insensitive). Cycle-safe: identity is
    /// checked first (a structure equals itself), and a pair of containers
    /// already being compared short-circuits to true; depth-capped.
    /// sec-r5 (F-10): the compared-pair set is NOT unwound on exit, for
    /// trees this is invisible (pairs are unique anyway); for DAG-shaped
    /// values it memoizes "this pair already verified equal", keeping the
    /// comparison linear instead of exponential (a 40-deep l=[l,l] twin
    /// chain was a live hang).
    pub fn deep_eq(&self, other: &Value) -> bool {
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        self.deep_eq_g(other, &mut seen, 0)
    }

    fn deep_eq_g(&self, other: &Value, seen: &mut HashSet<(usize, usize)>, depth: u32) -> bool {
        // sec-r5: 100k native frames sat within ~2 MB of the 8 MB main stack
        // (one layout change from SIGSEGV); 16k keeps comfortable headroom.
        if depth > 16_000 {
            return false;
        }
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                (*a as f64) == *b
            }
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::List(a), Value::List(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true; // a structure equals itself
                }
                let pair = (
                    Rc::as_ptr(a) as *const u8 as usize,
                    Rc::as_ptr(b) as *const u8 as usize,
                );
                if !seen.insert(pair) {
                    return true; // already comparing this pair (cycle)
                }
                let la = a.borrow();
                let lb = b.borrow();
                let ok = la.len() == lb.len()
                    && la
                        .iter()
                        .zip(lb.iter())
                        .all(|(x, y)| x.deep_eq_g(y, seen, depth + 1));
                // sec-r5 (F-10): pair stays in `seen`, DAG memoization
                ok
            }
            (Value::Map(a), Value::Map(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true;
                }
                let pair = (
                    Rc::as_ptr(a) as *const u8 as usize,
                    Rc::as_ptr(b) as *const u8 as usize,
                );
                if !seen.insert(pair) {
                    return true;
                }
                let ma = a.borrow();
                let mb = b.borrow();
                let ok = ma.len() == mb.len()
                    && ma.iter().all(|(k, v)| {
                        mb.iter().any(|(k2, v2)| {
                            k.deep_eq_g(k2, seen, depth + 1) && v.deep_eq_g(v2, seen, depth + 1)
                        })
                    });
                // sec-r5 (F-10): pair stays in `seen`, DAG memoization
                ok
            }
            (Value::Gene(d1, _), Value::Gene(d2, _)) => Arc::ptr_eq(d1, d2),
            (Value::Seq(d1, _), Value::Seq(d2, _)) => Arc::ptr_eq(d1, d2),
            // W015: channels are behavior handles, identity is the equality
            // rule (a copied handle to the same buffer IS the same channel;
            // two distinct buffers never compare equal even when empty).
            (Value::Channel(a), Value::Channel(b)) => Arc::ptr_eq(a, b),
            // builder-B parity finding (W34 stage 2, PR #28 pin): instances are
            // DATA, not handles, equal iff same class name AND deep-equal
            // field values. The old Arc::ptr_eq on the shared PhenoDef made
            // any two same-class instances == regardless of their fields
            // (the def pointer identifies the TYPE, not the instance state).
            // Closures (Gene) and streams (Seq) above stay identity-based:
            // they are behavior handles, not data. Oracle mirrors this arm
            // op-for-op (ObjInst in deep_eq).
            (Value::Obj(d1, ma), Value::Obj(d2, mb)) => {
                if d1.name != d2.name {
                    return false;
                }
                if Rc::ptr_eq(ma, mb) {
                    return true; // a structure equals itself
                }
                let pair = (
                    Rc::as_ptr(ma) as *const u8 as usize,
                    Rc::as_ptr(mb) as *const u8 as usize,
                );
                if !seen.insert(pair) {
                    return true; // already comparing this pair (cycle)
                }
                let fa = ma.borrow();
                let fb = mb.borrow();
                let ok = fa.len() == fb.len()
                    && fa.iter().all(|(k, v)| {
                        fb.iter().any(|(k2, v2)| {
                            k.deep_eq_g(k2, seen, depth + 1) && v.deep_eq_g(v2, seen, depth + 1)
                        })
                    });
                // sec-r5 (F-10): pair stays in `seen`, DAG memoization
                ok
            }
            // W06: variants are equal iff same tag and payloads are equal;
            // families are distinct (Some(x) != Ok(x)) because the tag IS the
            // contract. None == None (no payload to compare).
            (Value::Variant(t1, p1), Value::Variant(t2, p2)) => {
                t1 == t2
                    && match (p1, p2) {
                        (None, None) => true,
                        (Some(a), Some(b)) => a.deep_eq_g(b, seen, depth + 1),
                        _ => false,
                    }
            }
            // W013: weak handles are identity-based like channels, a handle
            // never equals the target it references (data vs handle).
            (Value::Weak(a), Value::Weak(b)) => a.same_target(b),
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
    if !(-4..16).contains(&exp) {
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

/// W029: the bytes side of the repr contract (see Value::repr_g Bytes arm).
/// Printable ASCII renders raw; \n \t \r " \\ render short-form; every other
/// byte renders \xNN. The oracle implements this function op-for-op.
fn escape_bytes(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
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
    out
}

fn key_repr_g(k: &Value, seen: &mut HashSet<usize>, depth: u32) -> String {
    match k {
        Value::Str(s) if is_identlike(s) => s.clone(),
        other => other.repr_g(seen, depth + 1),
    }
}

fn is_identlike(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        // ast-grep-ignore: no-unwrap-in-src
        && !s.chars().next().unwrap().is_ascii_digit()
}

/// Convert a value to a map key scalar (used by map literals / index).
pub fn key_scalar(v: &Value) -> Option<Value> {
    match v {
        Value::Null
        | Value::Bool(_)
        | Value::Int(_)
        | Value::Float(_)
        | Value::Str(_)
        | Value::Bytes(_) => Some(v.clone()),
        _ => None,
    }
}

/// sec-r5 (F-12): true when a map key hits the hash memo (O(1) upsert).
/// Non-scalar keys fall back to a linear deep_eq scan per operation,
/// callers charge that scan to the fuel budget.
pub fn key_is_scalar(v: &Value) -> bool {
    matches!(
        v,
        Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Str(_)
            | Value::Bytes(_)
    )
}

// ---------------------------------------------------------------- cycles
// W013 (D-013): live DETECTED-cycle bookkeeping. The engine detects a
// reference cycle at container insertion time: inserting `v` into container
// `C` where `C` is already reachable from `v` proves `C` is reachable from
// itself (C participates in a reference cycle), so C is registered once.
// The registry holds WEAK references only, accounting never extends a
// value's lifetime (D-013 rejected a tracing GC; this is honest accounting).
//
// The reported count re-verifies lazily: a registration dies when the
// container is reclaimed (weak upgrade fails, the cycle was broken and the
// strong count hit zero) or when the container is no longer reachable from
// itself (the program broke the cycle edge by mutation). Insertion
// increments, reclamation decrements.
//
// Walk scope: list items, map VALUES, phenotype field values, variant
// payloads. Map keys are not walked (the oracle stores container keys
// display-stringified, a pre-existing divergence the corpus avoids), and
// gene/env capture cycles are engine-internal, not container edges, so
// they are not counted either. The count is a detected-cycle gauge, not an
// exact strongly-connected-component census.

/// A weak handle to a registered cycle member's backing store.
enum CycleWeak {
    List(Weak<RefCell<Vec<Value>>>),
    Map(Weak<RefCell<MapStore>>),
}

struct CycleEntry {
    weak: CycleWeak,
    addr: usize,
}

thread_local! {
    // Rc containers are thread-local by construction (the spawn membrane
    // never aliases across workers, SPEC 19d), so per-thread registries are
    // the correct scope: a worker cell accounts its own cycles.
    static CYCLE_REGS: RefCell<CycleRegistry> = RefCell::new(CycleRegistry {
        regs: Vec::new(),
        index: HashSet::new(),
    });
}

/// The registry plus an address index. The index is a pure accelerator:
/// the soak gate (scripts/soak_cycles.op, 300k leaked pairs) exposed an
/// O(N^2) prune-and-scan per registration (12s at 50k, hang at 300k); the
/// gauge contract is unchanged (identical counts, identical outputs), the
/// membership test just stopped walking the whole Vec. Dead entries prune
/// lazily ON TOUCH instead of eagerly per registration, so a hot
/// no-dead-entries registration is O(1).
struct CycleRegistry {
    regs: Vec<CycleEntry>,
    index: HashSet<usize>,
}

fn list_addr(l: &ListRef) -> usize {
    Rc::as_ptr(l) as *const u8 as usize
}
fn map_addr(m: &MapRef) -> usize {
    Rc::as_ptr(m) as *const u8 as usize
}

/// The container's backing-store address (phenotype instances are tracked
/// through their field map, the per-instance container).
fn container_addr(v: &Value) -> Option<usize> {
    match v {
        Value::List(l) => Some(list_addr(l)),
        Value::Map(m) => Some(map_addr(m)),
        Value::Obj(_, m) => Some(map_addr(m)),
        _ => None,
    }
}

/// True when `v` is a value kind that can (transitively) hold a container;
/// scalars short-circuit the insertion walk.
fn can_contain(v: &Value) -> bool {
    matches!(
        v,
        Value::List(_) | Value::Map(_) | Value::Obj(_, _) | Value::Variant(_, _)
    )
}

/// Push the container-shaped children of `v` onto the walk worklist (map
/// KEYS are deliberately not walked, see the section comment).
fn push_children(v: &Value, stack: &mut Vec<Value>) {
    match v {
        Value::List(l) => stack.extend(l.borrow().iter().cloned()),
        Value::Map(m) => stack.extend(m.borrow().iter().map(|(_, v)| v.clone())),
        Value::Obj(_, m) => stack.extend(m.borrow().iter().map(|(_, v)| v.clone())),
        Value::Variant(_, Some(p)) => stack.push((**p).clone()),
        _ => {}
    }
}

/// Deterministic walk budget (rt_p22a armor): native graph walks are NOT
/// fuel-accounted interpreter steps, so every cycle walk carries a hard
/// visit cap. The cap never binds on sane graphs (corpus graphs visit
/// dozens of nodes); on adversarial graphs it bounds the native work per
/// container mutation and per memory() call. The cut is conservative AND
/// deterministic (both cores visit in the same order and cut at the same
/// node): insertion detection that runs out of budget does not register
/// (best effort), liveness verification that runs out of budget keeps the
/// entry counted (persistence is the D-013 default, death must be PROVEN).
const WALK_BUDGET: u64 = 100_000;

/// Does the walk from `v` reach the container at `root_addr`? Inserting `v`
/// into that container then makes the container reachable from itself.
/// Some(reached) when the walk completed, None when the budget ran out
/// (inconclusive).
fn walk_reaches(root_addr: usize, v: &Value) -> Option<bool> {
    let mut seen: HashSet<usize> = HashSet::new();
    let mut stack: Vec<Value> = vec![v.clone()];
    let mut budget = WALK_BUDGET;
    walk_reaches_root(root_addr, &mut seen, &mut stack, &mut budget)
}

/// Register `target` as a live detected cycle (called after a positive
/// detection). Dedupe: once per address, and once per subgraph, if any
/// container reachable from the target is already registered, the cycle
/// group was already counted through that member.
fn cycle_register_addr(addr: usize, weak: CycleWeak) {
    CYCLE_REGS.with(|cell| {
        let mut reg = cell.borrow_mut();
        // dedupe: this exact container already the registered member. The
        // index hit is O(1); on a hit the entry is verified alive (a dead
        // entry prunes on touch, its address may have been reused).
        if reg.index.contains(&addr) {
            if let Some(e) = reg.regs.iter().find(|e| e.addr == addr) {
                let alive = match &e.weak {
                    CycleWeak::List(w) => w.upgrade().is_some(),
                    CycleWeak::Map(w) => w.upgrade().is_some(),
                };
                if alive {
                    return; // this container is already the registered member
                }
            }
            reg.index.remove(&addr);
            reg.regs.retain(|e| e.addr != addr);
        }
        // subgraph dedupe: walk the target's reachable containers, if any
        // is registered the group is already counted (one entry per cycle).
        // Budgeted: an inconclusive dedupe registers anyway, the detection
        // at the insertion already proved a cycle at the target.
        let mut seen: HashSet<usize> = HashSet::new();
        let mut stack: Vec<Value> = Vec::new();
        match &weak {
            CycleWeak::List(w) => {
                if let Some(l) = w.upgrade() {
                    stack.extend(l.borrow().iter().cloned());
                }
            }
            CycleWeak::Map(w) => {
                if let Some(m) = w.upgrade() {
                    stack.extend(m.borrow().iter().map(|(_, v)| v.clone()));
                }
            }
        }
        let mut budget = WALK_BUDGET;
        while let Some(cur) = stack.pop() {
            if budget == 0 {
                break; // inconclusive: register the proven cycle
            }
            budget -= 1;
            match container_addr(&cur) {
                Some(a) => {
                    if reg.index.contains(&a) {
                        // verify the hit is a LIVE registered member (dead
                        // entries prune on touch, addresses get reused)
                        if let Some(e) = reg.regs.iter().find(|e| e.addr == a) {
                            let alive = match &e.weak {
                                CycleWeak::List(w) => w.upgrade().is_some(),
                                CycleWeak::Map(w) => w.upgrade().is_some(),
                            };
                            if alive {
                                return; // a member of this subgraph is registered
                            }
                            reg.index.remove(&a);
                            reg.regs.retain(|e| e.addr != a);
                        } else {
                            reg.index.remove(&a);
                        }
                    }
                    if seen.insert(a) {
                        push_children(&cur, &mut stack);
                    }
                }
                None => {
                    if let Value::Variant(_, Some(p)) = &cur {
                        stack.push((**p).clone());
                    }
                }
            }
        }
        reg.index.insert(addr);
        reg.regs.push(CycleEntry { weak, addr });
    });
}

/// W013: insertion-time cycle detection. Called at every container
/// mutation that adds an edge (push/insert/index write/map insert/field
/// write) BEFORE the edge lands: if the inserted value already reaches the
/// target container, the new edge closes a cycle and the target registers.
pub fn cycle_note_insert(target: &Value, v: &Value) {
    let root = match container_addr(target) {
        Some(a) => a,
        None => return,
    };
    if !can_contain(v) {
        return;
    }
    // an inconclusive detection walk (budget exhausted) does not register:
    // best-effort detection, the conservative direction for the GAUGE is
    // persistence of what is already registered
    if walk_reaches(root, v) == Some(true) {
        let weak = match target {
            Value::List(l) => CycleWeak::List(Rc::downgrade(l)),
            Value::Map(m) | Value::Obj(_, m) => CycleWeak::Map(Rc::downgrade(m)),
            _ => return,
        };
        cycle_register_addr(root, weak);
    }
}

/// W013: the memory() field. Registrations whose container was reclaimed
/// (weak dead) or whose cycle was broken by mutation (no longer reachable
/// from itself) are pruned; the survivors are the live detected cycles.
pub fn live_cycle_count() -> i64 {
    CYCLE_REGS.with(|cell| {
        let mut reg = cell.borrow_mut();
        let mut n: i64 = 0;
        // ONE budget shared by every verification in this call: the whole
        // re-verify pass is bounded no matter how many cycles are registered.
        let mut budget = WALK_BUDGET;
        reg.regs.retain(|e| {
            let verdict = match &e.weak {
                CycleWeak::List(w) => match w.upgrade() {
                    None => Some(false),
                    Some(l) => {
                        // walk from the container's children: the container
                        // itself is the root, reaching it again is a cycle
                        let mut seen: HashSet<usize> = HashSet::new();
                        seen.insert(e.addr);
                        let mut stack: Vec<Value> = l.borrow().iter().cloned().collect();
                        walk_reaches_root(e.addr, &mut seen, &mut stack, &mut budget)
                    }
                },
                CycleWeak::Map(w) => match w.upgrade() {
                    None => Some(false),
                    Some(m) => {
                        let mut seen: HashSet<usize> = HashSet::new();
                        seen.insert(e.addr);
                        let mut stack: Vec<Value> =
                            m.borrow().iter().map(|(_, v)| v.clone()).collect();
                        walk_reaches_root(e.addr, &mut seen, &mut stack, &mut budget)
                    }
                },
            };
            // Some(false): proven reclaimed or broken, drop the entry.
            // Some(true): proven alive. None: budget exhausted, the entry
            // stays counted (death must be PROVEN, persistence is D-013).
            let still = verdict.unwrap_or(true);
            if still {
                n += 1;
            }
            still
        });
        // the index mirrors the post-prune Vec (rebuild is O(N) and this is
        // the one place that bulk-prunes)
        reg.index = reg.regs.iter().map(|e| e.addr).collect();
        n
    })
}

/// Worklist walk seeded past the root (children of the candidate container):
/// reaching `root_addr` again proves the container is reachable from itself.
/// Some(reached) when the walk completed, None when the shared budget ran
/// out mid-walk (inconclusive, callers keep the conservative answer).
fn walk_reaches_root(
    root_addr: usize,
    seen: &mut HashSet<usize>,
    stack: &mut Vec<Value>,
    budget: &mut u64,
) -> Option<bool> {
    while let Some(cur) = stack.pop() {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        match container_addr(&cur) {
            Some(a) => {
                if a == root_addr {
                    return Some(true);
                }
                if seen.insert(a) {
                    push_children(&cur, stack);
                }
            }
            None => {
                if let Value::Variant(_, Some(p)) = &cur {
                    stack.push((**p).clone());
                }
            }
        }
    }
    Some(false)
}
