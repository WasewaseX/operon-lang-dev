//! value.rs, runtime values, display, truthiness, comparison, deep equality.

use crate::ast::GeneDef;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;

pub type ListRef = Rc<RefCell<Vec<Value>>>;

/// dx-r3 (re-audit perf #7): maps keep their insertion-ordered Vec (repr,
/// keys(), iteration order are part of the language contract) but gain a
/// hash memo, so the hot lookups, `m[k]`, `has`, `del`, member access,
/// `map_insert`, are O(1) instead of a linear deep_eq scan. The memo is a
/// PREFILTER: candidate positions are always verified with deep_eq, and
/// every class the memo cannot PROVE is absent falls back to the exact
/// full scan, so exotic equalities stay exact.
///
/// P1 rework (2026-10-04, S34 "P1 Map representation: APPROVED"): the memo
/// was a std HashMap<(u8, u64), usize> — two SipHashes per probe (one over
/// the key's canonical bytes, one over the (u8, u64) tuple inside HashMap)
/// and, the real kill, `position()` fell back to a FULL linear deep_eq scan
/// on every memo miss: building N distinct keys was O(N^2) deep_eq scans,
/// every absent-key lookup scanned the whole store, and each `del` rebuilt
/// the whole memo (measured on c1e5039, docs/bench/2026-10-04-p1-map.md:
/// 8k build 149 ms with t(2N)/t(N) = 4.19; 50k absent lookups 1084 ms;
/// 2k batch dels 261 ms). Now:
///
/// 1. The memo is an open-addressed table keyed by the ALREADY-COMPUTED
///    64-bit hash (one hash per lookup, no tuple re-hash, no std HashMap).
///    Slots are (hash, position); probing compares stored hashes; growth
///    rehashes from items (amortized O(1)); deletion tombstones (probe
///    chains stay contiguous — miss-trust REQUIRES that probing only ever
///    stops at a real SLOT_EMPTY, never at a hole inside a cluster).
/// 2. The hash is dependency-free FNV-1a (str/bytes/canonical-float bytes)
///    or a Fibonacci mix (int bits). The memo is pure accelerator state
///    and never feeds observable order, so the hash choice is invisible
///    (the same argument the SipHash design relied on).
/// 3. MISS-TRUST, the O(1) absence proof: within a scalar equality class,
///    deep_eq-equal values ALWAYS hash equal (Str: same bytes; Int: bit
///    bijection through `as`; Bool/Null: constants; Bytes: same bytes),
///    and every scalar item's hash is stored — so an empty probe PROVES
///    the key is absent, no fallback scan. The soundness hole is exactly
///    one class: Int<->Float (deep_eq(Int 2, Float 2.0) is TRUE, and
///    0.0 == -0.0 under f64 ==) — repr hashing does not cross tags. The
///    store counts its numeric (Int/Float) keys: with zero of them a
///    numeric lookup is trivially absent; with any, numeric lookups take
///    the exact full scan. A u64 collision between two different keys
///    shares a slot: the deep_eq verify fails and the exact scan runs
///    (2^-64, harmless). The counter is exact by construction: the rehash
///    path recounts from items, the insert put-branch increments (D1 fix,
///    strict review 2026-10-05 — without it a numeric key landing in a
///    non-rebuilding put silently disarmed the sentinel and miss-trust
///    reported cross-class absences), and every removal decrements under
///    a debug_assert tripwire (D2 fix).
/// 4. `del` no longer rebuilds the memo: one O(capacity) walk decrements
///    stored positions past the removed slot and tombstones the removed
///    key's entry (batch del drops from quadratic to linear).
/// 5. Slot ownership (D3/D3' fixes, strict review 2026-10-05): a live
///    slot is NEVER stolen and NEVER cross-tombstoned. put() skips live
///    same-hash slots (both callers guarantee unique keys, so a live
///    same-hash slot always belongs to a DIFFERENT key — the old
///    overwrite handed the owner's memo entry to the newcomer, and once
///    the newcomer's entry was tombstoned the owner's probe ran to
///    EMPTY and miss-trust reported a false absence; craftable in pure
///    Operon because fnv1a(Float(2.0).to_string()) == fnv1a("2")), and
///    tombstoning happens only on the slot whose stored position
///    deep_eq-verifies the key against the pre-removal items. Together
///    with the within-class equal=>equal-hash law this restores the
///    miss-trust proof unconditionally: a live scalar key always owns a
///    live slot its own probe will reach before any SLOT_EMPTY.
///
/// Exactness contract (unchanged): memo hits are deep_eq-verified; the
/// full exact scan remains the fallback for every class the table cannot
/// prove (non-scalar keys keep the sec-r5 bounded scan). Scalar-key
/// behavior is bit-for-bit identical to the previous designs.
#[derive(Default)]
pub struct MapStore {
    pub items: Vec<(Value, Value)>,
    memo: Memo,
}

/// sec-r5 (F-12): non-scalar keys (lists/maps) miss the hash memo and fall
/// back to a linear deep_eq scan. Unbounded, that was quadratic CPU that
/// burned zero fuel, 25k list-keyed inserts was a live hang. The scan is
/// now capped: beyond this many entries a non-scalar key is treated as
/// absent (SPEC §9b). Scalar keys keep exact semantics via the memo.
const NON_SCALAR_SCAN_CAP: usize = 512;

/// FNV-1a 64-bit, dependency-free. Used ONLY inside the map memo (pure
/// accelerator state, deep_eq-verified, never observable).
fn fnv1a(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Key classes: which deep_eq equality class the key belongs to. The only
/// cross-CLASS equality deep_eq defines is Int<->Float (both directions).
const KC_NULL: u8 = 0;
const KC_BOOL: u8 = 1;
const KC_INT: u8 = 2;
const KC_FLOAT: u8 = 3;
const KC_STR: u8 = 4;
const KC_BYTES: u8 = 5;
const KC_OTHER: u8 = 255;

/// (class, 64-bit memo hash). Equal values within a class always hash
/// equal — the property miss-trust is built on (see MapStore docs).
fn key_class_hash(v: &Value) -> (u8, u64) {
    match v {
        Value::Null => (KC_NULL, 0x9e37_79b9_7f4a_7c15),
        Value::Bool(b) => (KC_BOOL, 0xd1b5_4a32_d192_ed03 + *b as u64),
        // `as` wrapping is a bijection i64 -> u64; the Fibonacci mix only
        // spreads bits across table slots, it cannot merge distinct ints.
        Value::Int(i) => (KC_INT, (*i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)),
        Value::Float(f) => {
            // canonical repr; f.to_string() may allocate but float keys are
            // not the hot path (loglens evidence)
            (KC_FLOAT, fnv1a(f.to_string().as_bytes()))
        }
        Value::Str(s) => (KC_STR, fnv1a(s.as_bytes())),
        // W029: bytes keys hash by CONTENT now (the length-tagged prefilter
        // predates the in-place hash); equal bytes hash equal, different
        // same-length bytes collide -> deep_eq verify fails -> exact scan.
        Value::Bytes(b) => (KC_BYTES, fnv1a(b)),
        // non-scalar keys are legal but rare, they simply miss the memo
        // and fall back to the (bounded) linear scan
        _ => (KC_OTHER, 0),
    }
}

const SLOT_EMPTY: u32 = u32::MAX;
const SLOT_TOMB: u32 = u32::MAX - 1;

/// P1: open-addressed memo, slot = (hash, position), linear probing,
/// load <= 1/2 (rehash beyond), tombstones on delete so probe chains stay
/// contiguous.
#[derive(Default)]
struct Memo {
    tab: Vec<(u64, u32)>,
    mask: usize, // cap - 1; meaningless while tab is empty
    used: usize, // live slots
    tombs: usize,
    num_numeric: usize, // Int/Float keyed items (the cross-equality class)
    num_float: usize,   // Float-keyed items only (perf-xlang-r2: Int lookups
                        // take the O(1) memo path unless a Float key exists;
                        // Float lookups keep the exact scan whenever ANY
                        // numeric key exists, which also covers the
                        // 0.0 == -0.0 same-class crossing)
}

impl Memo {
    fn get(&self, h: u64) -> Option<usize> {
        if self.tab.is_empty() {
            return None;
        }
        let mut i = (h as usize) & self.mask;
        loop {
            let (sh, sp) = self.tab[i];
            if sp == SLOT_EMPTY {
                return None;
            }
            // tombstones keep the probe alive but never match
            if sp != SLOT_TOMB && sh == h {
                return Some(sp as usize);
            }
            i = (i + 1) & self.mask;
        }
    }
    /// Insert for `h`. Never grows the table; callers rehash when load
    /// would exceed 1/2. NEVER overwrites a live slot: both callers
    /// (insert's new-key path, rehash over unique items) guarantee the
    /// key is not present, so a live same-hash slot belongs to another
    /// key and stealing it would break slot ownership (D3', see the
    /// MapStore docs) — skip it and land on the first tomb/empty.
    fn put(&mut self, h: u64, pos: usize) {
        if self.tab.is_empty() {
            return; // defensive; insert()/rehash() always size the table first
        }
        let mut i = (h as usize) & self.mask;
        let mut free: Option<usize> = None;
        loop {
            let (_, sp) = self.tab[i];
            if sp == SLOT_EMPTY {
                // end of the probe: take the first free slot seen
                let dst = free.unwrap_or(i);
                if self.tab[dst].1 == SLOT_TOMB {
                    self.tombs -= 1;
                } else {
                    self.used += 1;
                }
                self.tab[dst] = (h, pos as u32);
                return;
            }
            if sp == SLOT_TOMB && free.is_none() {
                free = Some(i);
            }
            // live slots (any hash, including equal hashes) are skipped:
            // load <= 1/2 guarantees an EMPTY exists, so this terminates
            i = (i + 1) & self.mask;
        }
    }
    /// Tombstone the slot that verifiably holds `key`: the first live
    /// same-hash slot whose stored position deep_eq-matches `key` in the
    /// PRE-removal items. Hash-only tombstoning (the original P1 form)
    /// could kill a same-hash NEIGHBOR's slot — u64 collisions, or the
    /// craftable Str/Float shared-string-hash shape fnv1a("2") ==
    /// fnv1a(Float(2.0).to_string()) — leaving the live owner unmemoized;
    /// its probe then ran to EMPTY and miss-trust reported a false
    /// absence (D3, strict review 2026-10-05). Probe chain stays
    /// contiguous either way.
    fn tomb_verified(&mut self, h: u64, key: &Value, items: &[(Value, Value)]) {
        if self.tab.is_empty() {
            return;
        }
        let mut i = (h as usize) & self.mask;
        loop {
            let (sh, sp) = self.tab[i];
            if sp == SLOT_EMPTY {
                return; // the key holds no memo slot (defensive)
            }
            if sp != SLOT_TOMB && sh == h {
                if let Some((k, _)) = items.get(sp as usize) {
                    if k.deep_eq(key) {
                        self.tab[i] = (0, SLOT_TOMB);
                        self.used -= 1;
                        self.tombs += 1;
                        return;
                    }
                }
            }
            i = (i + 1) & self.mask;
        }
    }
    /// Positions after a removed items slot shift by one.
    fn shift_positions(&mut self, removed: usize) {
        for s in self.tab.iter_mut() {
            if s.1 < SLOT_TOMB && (s.1 as usize) > removed {
                s.1 -= 1;
            }
        }
    }
    /// Full rehash from items (also recounts num_numeric).
    fn rehash(&mut self, items: &[(Value, Value)]) {
        let cap = (items.len() * 2).max(16).next_power_of_two();
        self.tab = vec![(0u64, SLOT_EMPTY); cap];
        self.mask = cap - 1;
        self.used = 0;
        self.tombs = 0;
        self.num_numeric = 0;
        self.num_float = 0;
        for (i, (k, _)) in items.iter().enumerate() {
            let (cls, h) = key_class_hash(k);
            if cls != KC_OTHER {
                if cls == KC_INT || cls == KC_FLOAT {
                    self.num_numeric += 1;
                }
                if cls == KC_FLOAT {
                    self.num_float += 1;
                }
                self.put(h, i);
            }
        }
    }
}

impl MapStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn from_vec(items: Vec<(Value, Value)>) -> Self {
        let mut s = MapStore {
            items,
            memo: Memo::default(),
        };
        s.rebuild();
        s
    }
    pub fn rebuild(&mut self) {
        self.memo.rehash(&self.items);
    }
    /// Exact position of `key` (deep_eq verified), O(1) for scalar keys.
    pub fn position(&self, key: &Value) -> Option<usize> {
        let (cls, h) = key_class_hash(key);
        self.position_h(key, cls, h)
    }

    /// z-s0-parity (#107.2): the fuel a `position()` call MAY pay, mirroring
    /// `position_h`'s branching: non-scalar keys pay the capped linear scan,
    /// int/float lookups on numerically-mixed maps pay the exact scan
    /// (cross-equality class), everything else is the O(1) memo probe.
    /// Callers charge this to the step budget so a 50k-entry map probed in
    /// a loop cannot burn quadratic CPU for free.
    pub fn lookup_scan_cost(&self, key: &Value) -> u64 {
        let (cls, _) = key_class_hash(key);
        if cls == KC_OTHER {
            return (self.items.len() as u64).min(NON_SCALAR_SCAN_CAP as u64);
        }
        if (cls == KC_INT && self.memo.num_float > 0)
            || (cls == KC_FLOAT && self.memo.num_numeric > 0)
        {
            return self.items.len() as u64;
        }
        1
    }
    /// O(1) string-key lookup without constructing a `Value::Str` (the
    /// `.`/`?.` member-read hot path — P4-safe). Same contract as
    /// `position()` restricted to the KC_STR scalar class: the memo hit is
    /// deep_eq-verified (string equality IS the Str/Str deep_eq), a failed
    /// verify falls back to the exact scan, and miss-trust proves absence
    /// (every live Str key's fnv1a hash is in the memo, equal strings hash
    /// equal). Non-Str keys never deep_eq-equal a string, so the exact scan
    /// may specialize to `Value::Str` entries.
    pub fn position_str(&self, key: &str) -> Option<usize> {
        let h = fnv1a(key.as_bytes());
        match self.memo.get(h) {
            Some(i) => match self.items.get(i) {
                Some((Value::Str(s), _)) if s == key => Some(i),
                // u64 collision class (incl. the craftable fnv1a("2") ==
                // fnv1a(Float(2.0) repr) shape documented on tomb_verified):
                // the exact scan the old design always used on a failed
                // verify, specialized to the Str class.
                _ => self
                    .items
                    .iter()
                    .position(|(k, _)| matches!(k, Value::Str(s) if s == key)),
            },
            None => None,
        }
    }
    fn position_h(&self, key: &Value, cls: u8, h: u64) -> Option<usize> {
        if cls == KC_OTHER {
            // sec-r5 (F-12): non-scalar keys, bounded scan
            return self
                .items
                .iter()
                .take(NON_SCALAR_SCAN_CAP)
                .position(|(k, _)| k.deep_eq(key));
        }
        if (cls == KC_INT && self.memo.num_float > 0)
            || (cls == KC_FLOAT && self.memo.num_numeric > 0)
        {
            // The Int<->Float cross-equality class (deep_eq(Int 2, Float
            // 2.0) is true; 0.0 == -0.0): repr hashing does not cross tags,
            // so a lookup that could be matched by the OTHER numeric class
            // needs the exact scan. An Int lookup on a map with ZERO Float
            // keys can only be matched by another Int (in-class equal =>
            // equal-hash), so the memo path + miss-trust stay sound; Float
            // lookups keep the exact scan whenever any numeric key exists
            // (0.0 and -0.0 are deep_eq-equal but hash apart by repr, so
            // the float class alone cannot trust a memo miss).
            return self.items.iter().position(|(k, _)| k.deep_eq(key));
        }
        match self.memo.get(h) {
            Some(i) => {
                if let Some((k, _)) = self.items.get(i) {
                    if k.deep_eq(key) {
                        return Some(i);
                    }
                }
                // u64 collision class: the exact scan the old design always
                // used on a failed verify
                self.items.iter().position(|(k, _)| k.deep_eq(key))
            }
            // Miss-trust (see the MapStore docs): within a scalar class,
            // equal values hash equal and every scalar hash is stored, so
            // an empty probe PROVES absence. No fallback scan.
            None => None,
        }
    }
    /// Upsert preserving insertion order (existing key keeps its position).
    pub fn insert(&mut self, key: Value, val: Value) {
        let (cls, h) = key_class_hash(&key);
        if cls != KC_OTHER {
            if let Some(i) = self.position_h(&key, cls, h) {
                self.items[i].1 = val;
                return;
            }
        }
        self.items.push((key, val));
        let pos = self.items.len() - 1;
        if cls != KC_OTHER {
            // keep load <= 1/2 (rehash also sizes the table on first insert)
            if self.memo.tab.is_empty() || (pos + 1) * 2 > self.memo.tab.len() {
                self.rebuild();
            } else {
                // D1 (strict review 2026-10-05): this branch bypasses
                // rehash's recount, so a numeric key landing here must
                // keep the sentinel exact itself — otherwise the counter
                // stays 0 while a numeric key exists and cross-class
                // lookups (has(2.0) on an int key) silently lose their
                // exact-scan path and miss-trust reports false absence.
                if cls == KC_INT || cls == KC_FLOAT {
                    self.memo.num_numeric += 1;
                }
                if cls == KC_FLOAT {
                    self.memo.num_float += 1;
                }
                self.memo.put(h, pos);
            }
        }
    }
    /// Delete by key; returns true when something was removed. Positions
    /// after the removed slot shift, so ONE O(capacity) memo walk
    /// decrements stored positions and the removed key's slot is tombstoned
    /// (P1: was a full O(N) rebuild per delete — batch deletes were
    /// quadratic; rehash-on-threshold keeps tombstones bounded).
    pub fn del(&mut self, key: &Value) -> bool {
        let (cls, h) = key_class_hash(key);
        match self.position_h(key, cls, h) {
            Some(i) => {
                // D3: tomb the verified slot BEFORE the removal — the
                // probe needs the pre-removal items to identify which
                // same-hash slot is really this key's (a hash-only tomb
                // could hit a colliding neighbor's slot).
                if cls != KC_OTHER {
                    self.memo.tomb_verified(h, key, &self.items);
                }
                // perf-xlang-r2: removing the LAST item shifts nothing, so
                // the O(capacity) memo walk is skipped entirely (pop-only
                // del patterns — stacks, tail eviction — drop to O(1)).
                if i + 1 == self.items.len() {
                    self.items.pop();
                } else {
                    self.items.remove(i);
                    if cls != KC_OTHER {
                        self.memo.shift_positions(i);
                    }
                }
                if cls != KC_OTHER {
                    if cls == KC_INT || cls == KC_FLOAT {
                        // D2 tripwire: exact since the D1 fix (put-branch
                        // counts, rehash recounts) — a 0 here means the
                        // counter lost an insert somewhere.
                        debug_assert!(
                            self.memo.num_numeric > 0,
                            "num_numeric underflow: numeric key removed without a counted insert"
                        );
                        self.memo.num_numeric = self.memo.num_numeric.saturating_sub(1);
                    }
                    if cls == KC_FLOAT {
                        self.memo.num_float = self.memo.num_float.saturating_sub(1);
                    }
                    if self.memo.tombs > self.memo.used {
                        self.rebuild();
                    }
                }
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
        self.memo = Memo::default();
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
    /// sec-r5 (F-10) + z-s0-parity (#119.2): the visited set is a PATH
    /// (unwound on exit), and each container's rendered form is memoized.
    /// A re-encounter of an already-rendered node re-emits the memoized
    /// bytes (oracle parity: CPython duplicates shared subtrees) charged
    /// against a per-call duplication budget; past the budget — and for
    /// true cycles — the `[...]` / `{...}` marker renders. This keeps the
    /// old exponential-DAG live-hang bounded (memo = each node renders
    /// once; re-emission is a memcpy) while shared-but-acyclic containers
    /// now print their full contents at every reference.
    pub fn repr(&self) -> String {
        let mut seen: HashSet<usize> = HashSet::new();
        let mut memo: HashMap<usize, String> = HashMap::new();
        let mut budget: u64 = REPR_DUP_BUDGET;
        self.repr_g(&mut seen, &mut memo, &mut budget, 0)
    }

    fn repr_g(
        &self,
        seen: &mut HashSet<usize>,
        memo: &mut HashMap<usize, String>,
        budget: &mut u64,
        depth: u32,
    ) -> String {
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
                if depth > 256 || seen.contains(&id) {
                    // cycle: the node is on the current render path
                    return "[...]".into();
                }
                if let Some(m) = memo.get(&id) {
                    // z-s0-parity (#119.2): shared-but-acyclic re-encounter
                    // re-emits the memoized render, budget-charged (oracle
                    // parity); past the budget, the containment marker.
                    return if *budget >= m.len() as u64 {
                        *budget -= m.len() as u64;
                        m.clone()
                    } else {
                        "[...]".into()
                    };
                }
                seen.insert(id);
                let items: Vec<String> = l
                    .borrow()
                    .iter()
                    .map(|v| v.repr_g(seen, memo, budget, depth + 1))
                    .collect();
                let s = format!("[{}]", items.join(", "));
                seen.remove(&id);
                memo.insert(id, s.clone());
                s
            }
            Value::Map(m) => {
                let id = Rc::as_ptr(m) as *const u8 as usize;
                if depth > 256 || seen.contains(&id) {
                    return "{...}".into();
                }
                if let Some(m) = memo.get(&id) {
                    return if *budget >= m.len() as u64 {
                        *budget -= m.len() as u64;
                        m.clone()
                    } else {
                        "{...}".into()
                    };
                }
                seen.insert(id);
                let items: Vec<String> = m
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}: {}",
                            key_repr_g(k, seen, memo, budget, depth),
                            v.repr_g(seen, memo, budget, depth + 1)
                        )
                    })
                    .collect();
                let s = format!("{{{}}}", items.join(", "));
                seen.remove(&id);
                memo.insert(id, s.clone());
                s
            }
            Value::Gene(d, _) => match &d.name {
                Some(n) => format!("<gene {}>", n),
                None => "<gene lambda>".into(),
            },
            // W06: variant repr mirrors mainstream constructor syntax; the
            // payload renders through repr_g so depth/cycle caps apply.
            Value::Variant(VTag::NoneV, _) => "None".into(),
            Value::Variant(t, Some(p)) => {
                format!(
                    "{}({})",
                    t.tag_name(),
                    p.repr_g(seen, memo, budget, depth + 1)
                )
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
    /// sec-r5 (F-10): pairs that complete EQUAL stay in `seen`, for trees
    /// this is invisible (pairs are unique anyway); for DAG-shaped values
    /// it memoizes "this pair already verified equal", keeping the
    /// comparison linear instead of exponential (a 40-deep l=[l,l] twin
    /// chain was a live hang).
    /// issue #102: pairs that complete UNEQUAL leave `seen` and move to
    /// `failed`. Before the failed set, a pair poisoned by a failed
    /// candidate attempt inside a map's any() loop made a later
    /// re-encounter hit the "already comparing" branch and return true —
    /// two maps sharing no genuinely equal keys could compare equal
    /// (non-symmetrical, iteration-order dependent). Re-encounters of a
    /// failed pair short-circuit false: deep_eq is deterministic within
    /// one top-level comparison (no Operon code runs mid-compare), so the
    /// recomputed verdict would be false anyway — and the short-circuit
    /// keeps adversarial DAG shapes from re-exploring exponentially.
    pub fn deep_eq(&self, other: &Value) -> bool {
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        let mut failed: HashSet<(usize, usize)> = HashSet::new();
        self.deep_eq_g(other, &mut seen, &mut failed, 0)
    }

    fn deep_eq_g(
        &self,
        other: &Value,
        seen: &mut HashSet<(usize, usize)>,
        failed: &mut HashSet<(usize, usize)>,
        depth: u32,
    ) -> bool {
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
            // issue #103: exact cross-class comparison — `as f64` rounds
            // any |i| above 2^53, so 9007199254740993 == 9007199254740992.0
            // wrongly held (and every nested container comparison rode this
            // arm). The oracle (Python int==float) is exact.
            (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
                int_float_exact_eq(*a, *b)
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
                if failed.contains(&pair) {
                    return false; // issue #102: already completed unequal
                }
                if !seen.insert(pair) {
                    return true; // in-progress ancestor (cycle) or verified equal
                }
                let la = a.borrow();
                let lb = b.borrow();
                let ok = la.len() == lb.len()
                    && la
                        .iter()
                        .zip(lb.iter())
                        .all(|(x, y)| x.deep_eq_g(y, seen, failed, depth + 1));
                // sec-r5 (F-10): verified-equal pair stays in `seen`, DAG
                // memoization; issue #102: failed pair unwinds into `failed`
                if !ok {
                    seen.remove(&pair);
                    failed.insert(pair);
                }
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
                if failed.contains(&pair) {
                    return false; // issue #102: already completed unequal
                }
                if !seen.insert(pair) {
                    return true;
                }
                let ma = a.borrow();
                let mb = b.borrow();
                let ok = ma.len() == mb.len()
                    && ma.iter().all(|(k, v)| {
                        mb.iter().any(|(k2, v2)| {
                            k.deep_eq_g(k2, seen, failed, depth + 1)
                                && v.deep_eq_g(v2, seen, failed, depth + 1)
                        })
                    });
                // sec-r5 (F-10): verified-equal pair stays in `seen`, DAG
                // memoization; issue #102: failed pair unwinds into `failed`
                if !ok {
                    seen.remove(&pair);
                    failed.insert(pair);
                }
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
                if failed.contains(&pair) {
                    return false; // issue #102: already completed unequal
                }
                if !seen.insert(pair) {
                    return true; // in-progress ancestor (cycle) or verified equal
                }
                let fa = ma.borrow();
                let fb = mb.borrow();
                let ok = fa.len() == fb.len()
                    && fa.iter().all(|(k, v)| {
                        fb.iter().any(|(k2, v2)| {
                            k.deep_eq_g(k2, seen, failed, depth + 1)
                                && v.deep_eq_g(v2, seen, failed, depth + 1)
                        })
                    });
                // sec-r5 (F-10): verified-equal pair stays in `seen`, DAG
                // memoization; issue #102: failed pair unwinds into `failed`
                if !ok {
                    seen.remove(&pair);
                    failed.insert(pair);
                }
                ok
            }
            // W06: variants are equal iff same tag and payloads are equal;
            // families are distinct (Some(x) != Ok(x)) because the tag IS the
            // contract. None == None (no payload to compare).
            (Value::Variant(t1, p1), Value::Variant(t2, p2)) => {
                t1 == t2
                    && match (p1, p2) {
                        (None, None) => true,
                        (Some(a), Some(b)) => a.deep_eq_g(b, seen, failed, depth + 1),
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

/// Exact i64-vs-f64 equality (issue #103). The old `(*a as f64) == *b`
/// rounds any |i| above 2^53, so 9007199254740993 == 9007199254740992.0
/// wrongly held. The oracle (Python int==float) is exact: equal iff the
/// float is integral and the mathematical values match. Every integral
/// f64 below 2^127 converts to i128 exactly; at or beyond that magnitude
/// no f64 equals an i64, so the i128 round-trip is total and exact.
/// Boundaries: -2^63 as f64 IS i64::MIN (true for i == i64::MIN); +2^63
/// is one past i64::MAX (always false); NaN/inf never equal an integer.
fn int_float_exact_eq(i: i64, f: f64) -> bool {
    if !f.is_finite() {
        return false;
    }
    let t = f.trunc();
    if t != f {
        return false;
    }
    // 2^127 as an f64 literal (exactly representable, power of two).
    const TWO_POW_127: f64 = 1.7014118346046923e38;
    if t.abs() >= TWO_POW_127 {
        return false;
    }
    (t as i128) == (i as i128)
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

fn key_repr_g(
    k: &Value,
    seen: &mut HashSet<usize>,
    memo: &mut HashMap<usize, String>,
    budget: &mut u64,
    depth: u32,
) -> String {
    match k {
        Value::Str(s) if is_identlike(s) => s.clone(),
        other => other.repr_g(seen, memo, budget, depth + 1),
    }
}

/// z-s0-parity (#119.2): per-call duplication budget for repr/json of
/// shared-but-acyclic containers. Realistic shared payloads re-render in
/// full (oracle parity); adversarial fan-out chains are capped here
/// instead of doubling output without bound.
pub const REPR_DUP_BUDGET: u64 = 4 * 1024 * 1024;

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
