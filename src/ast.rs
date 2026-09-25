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

#[derive(Debug, Clone)]
pub struct GeneDef {
    pub name: Option<String>,
    /// A13 (dx-r2): source line of the definition — runtime gate notes
    /// (grn veto, methylation silencing, etc.) render real locations.
    pub line: usize,
    pub params: Vec<(String, Option<Expr>)>,
    pub guard: Option<(Expr, Vec<Stmt>)>,
    pub body: Vec<Stmt>,
    pub acetylate: bool,
    pub methylate: bool,
    pub m6a: bool,
    pub seq: bool, // sequence (generator) definition
}

#[derive(Debug, Clone)]
pub struct PhenoDef {
    pub name: String,
    /// A13: source line of the definition (dx-r2 spans).
    pub line: usize,
    pub parent: Option<String>,
    pub fields: Vec<(String, Expr)>, // field name -> default expr
    pub methods: Vec<Arc<GeneDef>>,
}

#[derive(Debug, Clone)]
pub struct SpliceDef {
    pub root: String,
    /// A13: source line of the definition (dx-r2 spans).
    pub line: usize,
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
    For(String, Expr, Vec<Stmt>),
    Return(Option<Expr>),
    Break,
    Continue,
    ExprStmt(Expr),
    Match(Expr, Vec<(MatchPat, Vec<Stmt>)>),
    Use(String, Option<String>), // path, alias
    Raise(Option<String>, Expr), // kind, message
    Stress {
        kind: Option<String>,
        body: Vec<Stmt>,
        rescue: Option<(Option<String>, Vec<Stmt>)>,
    },
    Gene(Arc<GeneDef>),
    Splice(Arc<SpliceDef>),
    Silence(String, Option<String>),
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
    /// proof frames gathered (file path -> is implicit)
    pub proofs: Vec<Vec<Stmt>>,
    pub named_frames: Vec<(String, Vec<Stmt>)>,
    pub anchor_exports: Vec<String>, // top-level (outside tads)
    pub tad_exports: Vec<(String, Vec<String>)>, // tad name -> exported names
    pub tad_members: Vec<(String, Vec<String>)>, // tad name -> all defined names
    pub ires: Vec<String>,
}
