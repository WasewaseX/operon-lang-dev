//! ast.rs — Operon AST. Uses Arc throughout so gene definitions can be
//! transferred into worker threads (spawn) without Rc's thread restrictions.

use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Note {
    pub line: usize,
    pub rung: u8, // 1 canonical, 2 synonym, 3 wobble, 4 fallback
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// W029: bytes literal b"..." — the raw bytes after escape processing;
    /// no interpolation ever.
    Bytes(Vec<u8>),
    Interp(Vec<InterpPart>),
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Ident(String),
    Unary(UnOp, Box<Expr>),
    /// dx-r4: source line of the operator — hard type errors locate themselves.
    Binary(BinOp, Box<Expr>, Box<Expr>, usize),
    /// A13 (dx-r2): source line of the call site — runtime builtin notes
    /// (denials, timeouts, gate changes) carry the caller's location.
    Call(Box<Expr>, Vec<Expr>, usize),
    /// dx-r4: source line of the bracket — index errors locate themselves.
    Index(Box<Expr>, Box<Expr>, usize),
    Member(Box<Expr>, String),
    /// L1a: `a?.k` — Null receiver yields Null (silent); otherwise identical
    /// to Member.
    MemberSafe(Box<Expr>, String),
    Method(Box<Expr>, String, Vec<Expr>),
    /// L1a: `a?.k(args)` — Null-safe method call (same contract).
    MethodSafe(Box<Expr>, String, Vec<Expr>),
    Lambda(Arc<GeneDef>),
    Collect {
        var: String,
        iter: Box<Expr>,
        filter: Option<Box<Expr>>,
        body: Box<Expr>,
    },
    FateNew(String),
    New(String, Vec<Expr>),                   // phenotype constructor
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>), // cond ? a : b
    /// W06 (D-014): `e?!` — Option/Result propagation. Some/Ok unwraps to the
    /// payload; None/Err unwinds to the nearest enclosing gene boundary and
    /// becomes that gene's return value. Line stamps the propagation signal.
    Propagate(Box<Expr>, usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    In,
    /// L1a: `a ?? b` — coalesces Null only (not falsy); right-assoc,
    /// short-circuit (b unevaluated when a is non-null).
    Nullish,
}

#[derive(Debug, Clone, Default)]
pub struct GeneDef {
    pub name: Option<String>,
    /// A13 (dx-r2): source line of the definition — runtime gate notes
    /// (grn veto, methylation silencing, etc.) render real locations.
    pub line: usize,
    /// W074: `##` doc-comment lines attached to this declaration (metadata
    /// only — never evaluated, mirrored by fmt/hover/`operon doc`).
    pub doc: Vec<String>,
    pub params: Vec<(String, Option<Expr>)>,
    pub guard: Option<(Expr, Vec<Stmt>)>,
    pub body: Vec<Stmt>,
    pub acetylate: bool,
    pub methylate: bool,
    pub m6a: bool,
    /// reg-bio-3 (C10): gene dosage — `@copies n`. Copies amplify the
    /// concentration the gene feeds its GRN edges (transcript amount),
    /// NOT the call's return value. Clamped 1..=64 at parse.
    pub copies: u32,
    pub seq: bool, // sequence (generator) definition
    /// loop-9 (F-5): a CIS riboswitch aptamer in this transcript's own 5'UTR
    /// — `(ligand, bound_means_on, threshold)`. The metabolite pool is
    /// cell-wide; the SENSOR is per-gene. `off` class (TPP/purine/SAM):
    /// bound -> terminator hairpin -> OFF. `on` class (adenine/glycine
    /// activators): bound -> RBS exposed -> ON.
    pub riboswitch: Option<(String, bool, f64)>,
    /// loop-9 (F-2): per-gene PROMOTER IDENTITY — `@burst kon koff`. The
    /// telegraph layer's rates for THIS gene (overrides the global
    /// expr_on/.cell rates). Burst size ~ k_tx/k_off, burst frequency ~
    /// k_on: different promoters have different (kon, koff) — that is
    /// their identity. Rides the gene Arc, so workers inherit for free.
    pub burst: Option<(f64, f64)>,
    /// W01 (L2c): soft type annotations — parallel to `params` (same
    /// length; None where unannotated). Enforced at the call funnel as
    /// catchable `unfolded` Stress (SPEC §7a); never a parse rejection.
    pub param_anns: Vec<Option<TypeAnn>>,
    /// W01: return annotation — `gene f() -> int { }`. Checked when the
    /// gene produces its return value (including a `?!`-propagated one).
    pub ret_ann: Option<TypeAnn>,
}

/// W01 (L2c): the annotation grammar — `int`, `float`, `str`, `bool`,
/// `list`, `map`, `gene`, `sequence`, `phenotype`, `any`, unions (`int |
/// str`), optionals (`int?`). Matching is by `Value::type_name()` with
/// documented numeric widening (`float` accepts int; `int` refuses float)
/// and `any` accepting everything. This is a SOFT contract: violations are
/// recoverable Stress, never parse rejections.
#[derive(Debug, Clone)]
pub enum TypeAnn {
    Named(String),
    Union(Vec<TypeAnn>),
    Optional(Box<TypeAnn>),
}

impl TypeAnn {
    /// Canonical rendering (messages, fmt roundtrip, hover). Must match the
    /// oracle's ann_render op-for-op.
    pub fn render(&self) -> String {
        match self {
            TypeAnn::Named(n) => n.clone(),
            TypeAnn::Union(alts) => alts
                .iter()
                .map(|a| a.render())
                .collect::<Vec<_>>()
                .join(" | "),
            TypeAnn::Optional(inner) => format!("{}?", inner.render()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PhenoDef {
    pub name: String,
    /// A13: source line of the definition (dx-r2 spans).
    pub line: usize,
    /// W074: doc-comment lines (metadata only).
    pub doc: Vec<String>,
    pub parent: Option<String>,
    /// W04: implemented traits, in declaration order (`implements A, B`).
    /// Method dispatch falls back to trait DEFAULT methods in this order
    /// after the phenotype's own lineage misses.
    pub implements: Vec<String>,
    pub fields: Vec<(String, Expr)>, // field name -> default expr
    pub methods: Vec<Arc<GeneDef>>,
}

/// W04 (SPEC §8b): a trait declaration — a named set of method contracts.
/// A method without a body is REQUIRED (the implementing phenotype must
/// provide it; construction notes a contract break, calls wobble); a
/// method with a body is a DEFAULT (used when the phenotype lineage has
/// no method of that name).
#[derive(Debug, Clone)]
pub struct TraitMethod {
    pub name: String,
    pub line: usize,
    pub required: bool,
    /// Full gene definition (params + body) parsed by the normal gene
    /// parser — default methods are ordinary genes with a `self` binding.
    pub default: Option<Arc<GeneDef>>,
}
#[derive(Clone)]
pub struct TraitDef {
    pub name: String,
    pub line: usize,
    pub doc: Vec<String>,
    pub methods: Vec<TraitMethod>,
}
impl std::fmt::Debug for TraitDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraitDef")
            .field("name", &self.name)
            .field(
                "methods",
                &self.methods.iter().map(|m| &m.name).collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct SpliceDef {
    pub root: String,
    /// A13: source line of the definition (dx-r2 spans).
    pub line: usize,
    /// W074: doc-comment lines (metadata only).
    pub doc: Vec<String>,
    pub variants: Vec<(String, Arc<GeneDef>)>, // (variant name, gene)
}

/// reg-bio (F-5): inline ring kinetics — `repressilator a -> b -> c alpha 20;`.
/// Each field layers onto the interpreter's current `RepressiParams` (last
/// declaration wins per-field); None leaves the value untouched, so `.cell`
/// configuration and inline overrides compose.
#[derive(Debug, Clone, Default)]
pub struct RepressiOverrides {
    pub alpha: Option<f64>,
    pub gamma: Option<f64>,
    pub hill: Option<u32>,
    pub basal: Option<f64>,
    pub noise: Option<f64>,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct FateDef {
    pub name: String,
    /// A13: source line of the definition (dx-r2 spans).
    pub line: usize,
    /// W074: doc-comment lines (metadata only).
    pub doc: Vec<String>,
    pub states: Vec<(String, Vec<String>)>, // state -> allowed targets
    pub enter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RegEdge {
    pub from: String,
    pub to: String,
    pub strength: f64,
    pub inhibit: bool,
    pub threshold: Option<f64>, // Hill-style dose threshold
    /// reg-bio (F-2): per-edge Hill exponent (1..=8). None = the historical
    /// n=2 dose-response (bit-identical legacy path). Only meaningful on
    /// edges carrying a threshold (dose-response shape).
    pub hill: Option<u32>,
    /// reg-bio (F-3): cis-regulatory OR membership. An `any` edge passes the
    /// call gate when its own threshold passes — the gene fires if ALL
    /// non-any thresholded edges pass AND at least one `any` edge passes.
    /// Inhibiting edges ignore `any` (inhibitors already veto with OR
    /// semantics: any inhibitor above its threshold vetoes).
    pub any: bool,
    /// reg-bio-2 (D2b): occupancy repression. An inhibiting edge marked
    /// `occupy` composes multiplicatively — child *= 1 − influence — the
    /// thermodynamic Kⁿ/(Kⁿ+Rⁿ) survival form where repression can never
    /// overshoot and full occupancy means full silencing. Legacy edges
    /// subtract once (bit-identical default).
    pub occupy: bool,
    /// reg-bio-2 (B7): synergistic pooling. `sum` activating (thresholded)
    /// edges targeting the same gene pool their weighted inputs —
    /// P = min(1, Σ strength·level) — and ONE Hill function of P drives
    /// both the gate and the fire influence, so two sub-threshold inputs
    /// can fire together (enhanceosome synergy). Edge groups are keyed by
    /// (target, threshold, hill); edges without `sum` are untouched.
    pub sum: bool,
    /// reg-bio-2 (A5/C7): an `attenuates` edge vetoes like an inhibitor but
    /// reports the RNA-level mechanism — transcription attenuation (leader
    /// peptide / terminator hairpin outcome), not TF occlusion.
    pub attenuates: bool,
}

/// reg-bio-2 (A4): an allosteric binding record — `bind tf inducer lig k v;`
/// or `bind tf cofactor lig k v;`. Ligand binding modulates the regulator's
/// DNA-available fraction: an INDUCER reduces affinity (allolactose on LacI
/// — binding relieves repression), a COFACTOR increases it (tryptophan on
/// TrpR — binding enables repression). At every regulation read of `tf`:
///   occ = L / (k + L)
///   free = level × Π(1 − occ) over inducers × Π occ over cofactors
#[derive(Debug, Clone)]
pub struct BindDef {
    pub tf: String,
    pub ligand: String,
    /// "inducer" reduces DNA affinity; "cofactor" increases it
    pub inducer: bool,
    /// dissociation-style constant (occ = L/(k+L)); default 0.1
    pub k: f64,
}

/// reg-bio-2 (C1): the translation layer. A `translates` edge models
/// mRNA→protein production: at every engine update point (a `grn_fire`
/// pulse, a decay-clock tick) the target protein node integrates one Euler
/// step of the classic two-tier ODE
///     p ← p + rate·Δcalls − decay·p      (clamped 0..1)
/// where Δcalls is the source gene's call-count delta since the last
/// integration — transcripts accumulate in the call counters, protein
/// accumulates here, lagging and smoothing the transcriptional bursts.
/// Protein nodes live in `grn_levels`, so any GRN gate can read a protein
/// as its regulator (two-tier regulation).
#[derive(Debug, Clone)]
pub struct TransEdge {
    pub from: String,
    pub to: String,
    /// translation rate per transcript call (default 1.0)
    pub rate: Option<f64>,
    /// protein decay fraction per integration (default 0.0 — stable product)
    pub decay: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum MatchPat {
    Lit(Expr),        // literal-only pattern
    Multi(Vec<Expr>), // comma-separated literals
    Bind(String),     // identifier binds value
    Wild,             // _
    /// W02 (match-v2): variant constructor pattern — `Some(p)`, `None`,
    /// `Ok(p)`, `Err(p)`. The payload is itself a pattern (nestable);
    /// `None` payload = tag-only form (matches the tag with any payload).
    /// Unknown capitalized tags fall back to Bind with a note (Total
    /// Grammar: never a rejection).
    Variant(String, Option<Box<MatchPat>>),
    /// W02: list pattern — `[a, b, *rest]`. Element patterns nest; `*rest`
    /// binds the remaining tail as a List. Without `*rest` the length must
    /// match exactly.
    ListPat {
        elems: Vec<MatchPat>,
        rest: Option<String>,
    },
    /// W02: map pattern — `{x, y: p}`. Each key must be present; an
    /// optional sub-pattern is matched against the value.
    MapPat {
        keys: Vec<(String, Option<Box<MatchPat>>)>,
    },
    /// W02: or-pattern — `p1 | p2 | ...`; alternatives tried in order,
    /// first match binds.
    Or(Vec<MatchPat>),
    /// W02: guarded arm — `pat if cond`; the condition sees the pattern's
    /// bindings. Guard false (or contained) = arm misses, matching moves on.
    Guard(Box<MatchPat>, Expr),
}

/// L1a: destructuring patterns (let / for). `Bind` binds the whole item;
/// `List` destructures a List (elements are patterns; `*rest` captures the
/// tail); `Map` destructures a Map (each name reads that key). Soft-miss
/// semantics: wrong container type or missing element/key binds Null with a
/// note — Total Grammar: a pattern never hard-fails a run.
#[derive(Debug, Clone)]
pub enum Pat {
    Bind(String),
    List {
        elems: Vec<Pat>,
        rest: Option<String>, // *rest — tail after the fixed elements
    },
    Map {
        keys: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let(String, Expr),
    /// W05: `const NAME = expr` — an immutable binding. The bound value is
    /// deep-frozen (lists/maps inside it can never be mutated — mutation
    /// raises a catchable `frozen` Stress), and the NAME can never be
    /// reassigned (rebinding defines anew; assignment is the `frozen` stress).
    LetConst(String, Expr),
    /// W01 (L2c): annotated definition — `let n: int = 3`. The annotation is
    /// checked when the statement binds (mismatch = catchable `unfolded`
    /// Stress, SPEC §7a); the binding itself is an ordinary `let`.
    LetAnn(String, TypeAnn, Expr),
    Assign(String, Option<BinOp>, Expr), // name (op=)? expr
    IndexAssign(Expr, Expr, Option<BinOp>, Expr), // target[i] (op=)? expr
    MemberAssign(Expr, String, Option<BinOp>, Expr), // target.k (op=)? expr
    /// L1a: destructuring definition — `let [a, b] = e`, `let {x, y} = e`,
    /// `let [a, *rest] = e` (patterns nest).
    LetPat(Pat, Expr),
    /// L1a: destructuring for-loop — `for [k, v] in pairs { ... }`.
    ForPat(Pat, Expr, Vec<Stmt>),
    /// L1a: multiple assignment / swap — `a, b = b, a`; RHS evaluated fully
    /// (left to right) before any target is assigned. `let a, b = 1, 2` sets
    /// `define` (fresh bindings); the bare form assigns existing names.
    MultiAssign(Vec<Expr>, Vec<Expr>, bool),
    If(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>),
    While(Expr, Vec<Stmt>),
    Loop(Vec<Stmt>),
    /// W17: structured-concurrency block. Tasks spawned inside register on
    /// this scope and are joined at block exit, in spawn order, on every
    /// flow path (normal, return/break/continue, or stress).
    Scope(Vec<Stmt>),
    For(String, Expr, Vec<Stmt>),
    Return(Option<Expr>),
    Break,
    Continue,
    ExprStmt(Expr),
    Match(Expr, Vec<(MatchPat, Vec<Stmt>)>),
    Use(String, Option<String>),        // path, alias
    Raise(Option<String>, Expr, usize), // kind, message, statement line (W007)
    Stress {
        kind: Option<String>,
        body: Vec<Stmt>,
        rescue: Option<(Option<String>, Vec<Stmt>)>,
    },
    Gene(Arc<GeneDef>),
    Splice(Arc<SpliceDef>),
    /// W04 (SPEC §8b): `trait Name { gene m(); gene n() { … } }`.
    Trait(Arc<TraitDef>),
    /// reg-bio-3 (C9): stoichiometric RISC — `silence old -> new strength s
    /// sites n;`. The strength is the per-site capture probability (clamped
    /// 0..=1, default 1.0); each statement is one binding site (sites n
    /// composes multiplicatively: survival = (1-s)^n). Omitted strength and
    /// sites reproduce the legacy binary redirect bit-identically.
    Silence(String, Option<String>, f64, u32),
    /// reg-bio-3 (A1/A7): the polycistronic transcription unit — the
    /// namesake construct. `operon lac { lacZ rbs 1.0; lacY rbs 0.6; }`:
    /// ONE promoter drives N cistrons on ONE transcript; a call to any
    /// cistron is a transcription attempt of the WHOLE unit, so edges
    /// targeting the unit gate every member. Member order is load-bearing
    /// (RBS gradient + polarity exposure); each member's `rbs` multiplies
    /// its translation rate (Shine-Dalgarno strength, clamped 0..=1).
    Operon(String, Vec<(String, f64)>),
    Enhance(Vec<String>),
    Ires(String),
    Fate(Arc<FateDef>),
    Regulate(Vec<RegEdge>, Vec<TransEdge>, Vec<BindDef>),
    /// reg-bio-2 (A4): a small-molecule ligand pool — `ligand iptg;`.
    /// Ligands are metabolites, not genes: their levels are set via
    /// `ligand_set(name, v)` or the `.cell [ligand.<name>]` bath config,
    /// and they drive gates directly (a ligand named as an edge source is
    /// a riboswitch-style, protein-free gate).
    Ligand(String),
    /// loop-9 (C8): a quorum-sensing signal species — `autoinducer ahl;`.
    /// The species registers into the process-global SHARED medium (the
    /// environment, not the cytoplasm): `secrete` charges it, `quorum`
    /// reads it, and a species named as an edge source is a LuxR·AHL-style
    /// population gate — my secretion raises YOUR activation.
    Autoinducer(String),
    Toggle(String, String),
    /// reg-bio-2 (C11): a decoy binding site — `decoy d for tf capacity c;`.
    /// The decoy node absorbs its regulator without producing output:
    /// every regulation read of `tf` sees the free fraction
    /// max(0, level(tf) − c·level(d)) — competitive titration.
    Decoy(String, String, f64),
    Repressilator(Vec<String>, Option<f64>, RepressiOverrides),
    Frame {
        name: String,
        is_proof: bool,
        body: Vec<Stmt>,
    },
    Edit(String, Vec<(String, String)>), // target, (from, to) replacements
    AnchorExport(Vec<String>),
    AnchorImport(Vec<String>),
    Tad(String, Vec<Stmt>),
    Block(Vec<Stmt>),     // bare scoped block (Total Grammar repair product)
    Seq(Arc<GeneDef>),    // sequence definition (generator)
    Yield(Option<Expr>),  // yield inside a sequence body
    Pheno(Arc<PhenoDef>), // phenotype definition (user class)
}

#[derive(Debug, Clone, Default)]
pub struct Program {
    pub stmts: Vec<Stmt>,
    pub notes: Vec<Note>,
    /// W074: a `##` block at the very top of the file that does NOT hug a
    /// declaration (blank-line separated) becomes the module doc.
    pub module_doc: Vec<String>,
    /// proof frames gathered (file path -> is implicit)
    pub proofs: Vec<Vec<Stmt>>,
    pub named_frames: Vec<(String, Vec<Stmt>)>,
    pub anchor_exports: Vec<String>, // top-level (outside tads)
    pub tad_exports: Vec<(String, Vec<String>)>, // tad name -> exported names
    pub tad_members: Vec<(String, Vec<String>)>, // tad name -> all defined names
    pub ires: Vec<String>,
    /// W24: top-level names marked with the contextual `pub` marker.
    /// Inert by default; under `.cell modules.visibility = strict` the
    /// module exports ONLY these names (migration-safe: a strict module
    /// with zero pub marks keeps default-open, with a note).
    pub pub_exports: Vec<String>,
}
