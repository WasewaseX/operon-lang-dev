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
    Regulate(Vec<RegEdge>),
    Toggle(String, String),
    Repressilator(Vec<String>, Option<f64>),
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
