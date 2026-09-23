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
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String),
    Method(Box<Expr>, String, Vec<Expr>),
    Lambda(Arc<GeneDef>),
    Collect {
        var: String,
        iter: Box<Expr>,
        filter: Option<Box<Expr>>,
        body: Box<Expr>,
    },
    FateNew(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    In,
}

#[derive(Debug, Clone)]
pub struct GeneDef {
    pub name: Option<String>,
    pub params: Vec<(String, Option<Expr>)>,
    pub guard: Option<(Expr, Vec<Stmt>)>,
    pub body: Vec<Stmt>,
    pub acetylate: bool,
    pub methylate: bool,
    pub m6a: bool,
}

#[derive(Debug, Clone)]
pub struct SpliceDef {
    pub root: String,
    pub variants: Vec<(String, Arc<GeneDef>)>, // (variant name, gene)
}

#[derive(Debug, Clone)]
pub struct FateDef {
    pub name: String,
    pub states: Vec<(String, Vec<String>)>, // state -> allowed targets
    pub enter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RegEdge {
    pub from: String,
    pub to: String,
    pub strength: f64,
    pub inhibit: bool,
}

#[derive(Debug, Clone)]
pub enum MatchPat {
    Lit(Expr),          // literal-only pattern
    Multi(Vec<Expr>),   // comma-separated literals
    Bind(String),       // identifier binds value
    Wild,               // _
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let(String, Expr),
    Assign(String, Option<BinOp>, Expr), // name (op=)? expr
    IndexAssign(Expr, Expr, Option<BinOp>, Expr), // target[i] (op=)? expr
    MemberAssign(Expr, String, Option<BinOp>, Expr), // target.k (op=)? expr
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
    Block(Vec<Stmt>), // bare scoped block (Total Grammar repair product)
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
