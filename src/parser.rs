//! parser.rs — the Total Grammar parser.
//!
//! Law: no token stream is rejected. The ladder:
//!   rung 1 — canonical match
//!   rung 2 — synonym keywords (noted, repaired)
//!   rung 3 — wobble: edit-distance repair when a keyword is REQUIRED
//!   rung 4 — semantic fallback: skip/auto-close/stringify, always noted
//!
//! Keyword repair only happens at positions where a keyword is syntactically
//! required — identifiers in binding positions are never touched.

use crate::ast::*;
use crate::lexer::{lex, Tok};

const KEYWORDS: &[&str] = &[
    "gene", "let", "if", "elif", "else", "while", "loop", "for", "in", "return", "break",
    "continue", "match", "case", "use", "tad", "anchor", "export", "import", "enhance",
    "silence", "stress", "rescue", "raise", "fate", "state", "regulate", "activates",
    "inhibits", "strength", "toggle", "repressilator", "period", "frame", "proof", "guard",
    "splice", "variant", "edit", "replace", "ires", "as", "collect", "enter",
    "phenotype", "sequence", "yield", "new", "threshold", "from", "self",
];

const MARKS: &[&str] = &["acetylate", "methylate", "m6a"];

/// Words that end a `use` path — the alias introducer and statement enders.
fn use_path_boundary(w: &str) -> bool {
    w == "as" || w == "from"
}

pub fn is_canonical(w: &str) -> bool {
    KEYWORDS.contains(&w)
}

fn synonym(w: &str) -> Option<&'static str> {
    Some(match w {
        "fn" | "func" | "fun" | "def" | "funct" | "sub" | "lambda" | "proc" => "gene",
        "var" | "val" | "const" => "let",
        "elseif" => "elif",
        "foreach" | "each" => "for",
        "import" | "include" | "require" => "use",
        "ret" => "return",
        "stop" => "break",
        "next" | "skip" => "continue",
        "yes" | "on" => "true",
        "no" | "off" => "false",
        "nil" | "none" | "nothing" => "null",
        "unless" => "ifnot",
        "class" | "struct" | "record" | "prototype" | "type" => "phenotype",
        "generator" | "gen" | "stream" | "iter" => "sequence",
        _ => return None,
    })
}

fn wobble_keyword(word: &str) -> Option<&'static str> {
    let max = if word.chars().count() <= 4 { 1 } else { 2 };
    let mut best: Option<(&str, i32)> = None;
    for k in KEYWORDS {
        let d = crate::ffi::edit_distance(word, k);
        if d <= max {
            match best {
                Some((_, bd)) if d >= bd => {}
                _ => best = Some((k, d)),
            }
        }
    }
    best.map(|(k, _)| k)
}

pub struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    notes: Vec<Note>,
}

pub fn parse(src: &str) -> Program {
    let lexed = lex(src);
    let mut notes = lexed.notes;
    let mut p = Parser { toks: lexed.toks, pos: 0, notes: Vec::new() };
    let stmts = p.parse_program();
    notes.append(&mut p.notes);
    let mut prog = Program {
        proofs: Vec::new(),
        named_frames: Vec::new(),
        anchor_exports: Vec::new(),
        tad_exports: Vec::new(),
        tad_members: Vec::new(),
        ires: Vec::new(),
        notes,
        stmts,
    };
    collect_structure(&mut prog);
    prog
}

/// Extract file structure: proof frames, named frames, anchors, tads, ires.
fn collect_structure(prog: &mut Program) {
    fn walk(stmts: &[Stmt], prog: &mut Program, current_tad: Option<&str>) {
        for s in stmts {
            match s {
                Stmt::Frame { name, is_proof, body } => {
                    if *is_proof {
                        prog.proofs.push(body.clone());
                    } else {
                        prog.named_frames.push((name.clone(), body.clone()));
                    }
                    walk(body, prog, current_tad);
                }
                Stmt::Tad(tname, body) => {
                    let mut members: Vec<String> = Vec::new();
                    let mut exports: Vec<String> = Vec::new();
                    for t in body {
                        match t {
                            Stmt::AnchorExport(names) => exports.extend(names.clone()),
                            Stmt::Let(n, _) => members.push(n.clone()),
                            Stmt::Gene(g) => {
                                if let Some(n) = &g.name {
                                    members.push(n.clone())
                                }
                            }
                            Stmt::Splice(sp) => members.push(sp.root.clone()),
                            _ => {}
                        }
                    }
                    prog.tad_exports.push((tname.clone(), exports));
                    prog.tad_members.push((tname.clone(), members));
                    walk(body, prog, Some(tname));
                }
                Stmt::AnchorExport(names) => {
                    if current_tad.is_none() {
                        prog.anchor_exports.extend(names.clone());
                    }
                }
                Stmt::Ires(name) => prog.ires.push(name.clone()),
                Stmt::Gene(g) => {
                    if let Some(b) = &g.guard {
                        walk(&b.1, prog, current_tad);
                    }
                    walk(&g.body, prog, current_tad);
                }
                _ => {}
            }
        }
    }
    let stmts = prog.stmts.clone();
    walk(&stmts, prog, None);
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }
    fn line(&self) -> usize {
        self.toks[self.pos].1
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].0.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn note(&mut self, line: usize, rung: u8, msg: impl Into<String>) {
        self.notes.push(Note { line, rung, message: msg.into() });
    }
    fn eat_newlines(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.next();
        }
    }
    /// Skip tokens to end of line (rung-4 recovery).
    fn skip_line(&mut self) {
        while !matches!(self.peek(), Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof) {
            self.next();
        }
    }

    // ---- keyword recognition with the ladder --------------------------
    /// Consume an identifier expected to be `word` (canonical). Applies the
    /// synonym then wobble rungs. Returns true on success.
    fn expect_kw(&mut self, word: &str) -> bool {
        if let Tok::Ident(w) = self.peek().clone() {
            if w == word {
                self.next();
                return true;
            }
            if let Some(canon) = synonym(&w) {
                if canon == word {
                    let line = self.line();
                    self.note(line, 2, format!("synonym '{}' repaired to '{}'", w, word));
                    self.next();
                    return true;
                }
            }
            if let Some(canon) = wobble_keyword(&w) {
                if canon == word {
                    let line = self.line();
                    self.note(
                        line,
                        3,
                        format!("wobble: '{}' repaired to keyword '{}'", w, word),
                    );
                    self.next();
                    return true;
                }
            }
        }
        false
    }

    fn at_kw(&self, word: &str) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w == word)
    }

    // ---- program -------------------------------------------------------
    fn parse_program(&mut self) -> Vec<Stmt> {
        let mut out = Vec::new();
        loop {
            self.eat_newlines();
            if matches!(self.peek(), Tok::Eof) {
                break;
            }
            if matches!(self.peek(), Tok::RBrace) {
                let line = self.line();
                self.note(line, 4, "unmatched '}' skipped");
                self.next();
                continue;
            }
            let before = self.pos;
            match self.parse_stmt() {
                Some(s) => out.push(s),
                None => {}
            }
            if self.pos == before {
                // no progress: hard fallback
                let line = self.line();
                self.note(line, 4, format!("token '{:?}' skipped", self.peek()));
                self.next();
            }
        }
        out
    }

    // ---- statements ----------------------------------------------------
    fn parse_stmt(&mut self) -> Option<Stmt> {
        match self.peek().clone() {
            Tok::Mark(m) => {
                let line = self.line();
                self.next();
                let mark = self.repair_mark(m, line)?;
                let mut marks = vec![mark];
                // allow stacked marks
                while let Tok::Mark(m2) = self.peek().clone() {
                    let l2 = self.line();
                    self.next();
                    if let Some(r) = self.repair_mark(m2, l2) {
                        marks.push(r);
                    }
                }
                if !self.expect_kw("gene") {
                    let line = self.line();
                    self.note(line, 4, "mark must precede 'gene'; skipped line");
                    self.skip_line();
                    return None;
                }
                return Some(self.parse_gene_def(marks));
            }
            Tok::Ident(w) => self.parse_word_stmt(&w),
            Tok::LBrace => {
                // bare block as a scoped statement (semantic: sequential scope)
                let line = self.line();
                self.note(line, 4, "bare block treated as scoped statements");
                let body = self.parse_block()?;
                Some(Stmt::Block(body))
            }
            other => {
                let line = self.line();
                self.note(line, 4, format!("unexpected token '{:?}' at statement position", other));
                self.next();
                None
            }
        }
    }

    fn repair_mark(&mut self, m: String, line: usize) -> Option<String> {
        if MARKS.contains(&m.as_str()) {
            return Some(m);
        }
        let max = if m.chars().count() <= 4 { 1 } else { 2 };
        let mut best: Option<(&str, i32)> = None;
        for k in MARKS {
            let d = crate::ffi::edit_distance(&m, k);
            if d <= max {
                match best {
                    Some((_, bd)) if d >= bd => {}
                    _ => best = Some((k, d)),
                }
            }
        }
        if let Some((k, _)) = best {
            self.note(line, 3, format!("wobble: '@{}' repaired to '@{}'", m, k));
            return Some(k.to_string());
        }
        self.note(line, 4, format!("unknown mark '@{}' skipped", m));
        None
    }

    fn parse_word_stmt(&mut self, w: &str) -> Option<Stmt> {
        // Statement-arm keywords: only these accept rung-2/3 repair. A bare
        // identifier that repairs to a NON-statement word (e.g. `n` → `in`)
        // keeps its original spelling — binding positions are never repaired.
        const ARMS: &[&str] = &[
            "gene", "let", "if", "ifnot", "elif", "else", "while", "loop", "for", "return",
            "break", "continue", "match", "use", "tad", "anchor", "enhance", "silence",
            "stress", "rescue", "raise", "fate", "regulate", "toggle", "repressilator",
            "frame", "splice", "edit", "ires", "phenotype", "sequence", "yield",
        ];
        let mut word = w.to_string();
        // Expression-head detection: `i += 1`, `x = 2`, `f(...)`, `a[0]`,
        // `s.trim()` — a word followed by these cannot be a keyword position,
        // so rung-2/3 repair is suppressed (protects short variable names
        // like `i` from repairing to `if`).
        let t1 = self.toks.get(self.pos + 1).map(|t| t.0.clone());
        let expr_head = matches!(
            t1,
            Some(Tok::Eq)
                | Some(Tok::LParen)
                | Some(Tok::LBrack)
                | Some(Tok::Dot)
                | Some(Tok::PlusEq)
                | Some(Tok::MinusEq)
                | Some(Tok::StarEq)
                | Some(Tok::SlashEq)
                | Some(Tok::DSlashEq)
                | Some(Tok::PercentEq)
        );
        if !is_canonical(&word) && !expr_head {
            if let Some(canon) = synonym(&word) {
                if ARMS.contains(&canon) {
                    let line = self.line();
                    self.note(line, 2, format!("synonym '{}' repaired to '{}'", word, canon));
                    word = canon.to_string();
                }
            } else if let Some(canon) = wobble_keyword(&word) {
                if ARMS.contains(&canon) {
                    let line = self.line();
                    self.note(
                        line,
                        3,
                        format!("wobble: '{}' repaired to keyword '{}'", word, canon),
                    );
                    word = canon.to_string();
                }
            }
        }
        match word.as_str() {
            "gene" => {
                self.next();
                Some(self.parse_gene_def(vec![]))
            }
            "let" => {
                self.next();
                let name = self.expect_ident()?;
                let line = self.line();
                if matches!(self.peek(), Tok::Eq) {
                    self.next();
                    let e = self.parse_expr();
                    self.end_stmt();
                    Some(Stmt::Let(name, e))
                } else {
                    self.note(line, 4, format!("'let {}' without value binds null", name));
                    self.end_stmt();
                    Some(Stmt::Let(name, Expr::Null))
                }
            }
            "if" | "ifnot" => {
                self.next();
                let neg = word == "ifnot";
                let mut cond = self.parse_expr();
                if neg {
                    cond = Expr::Unary(UnOp::Not, Box::new(cond));
                }
                let body = self.parse_block().unwrap_or_default();
                let mut branches = vec![(cond, body)];
                let mut els = None;
                loop {
                    self.eat_newlines();
                    if self.at_kw("elif") || self.at_kw("elseif") {
                        let line = self.line();
                        if self.at_kw("elseif") {
                            self.note(line, 2, "synonym 'elseif' repaired to 'elif'");
                        }
                        self.next();
                        let c2 = self.parse_expr();
                        let b2 = self.parse_block().unwrap_or_default();
                        branches.push((c2, b2));
                    } else if self.expect_kw("else") {
                        els = Some(self.parse_block().unwrap_or_default());
                        break;
                    } else {
                        break;
                    }
                }
                Some(Stmt::If(branches, els))
            }
            "while" => {
                self.next();
                let cond = self.parse_expr();
                let body = self.parse_block().unwrap_or_default();
                Some(Stmt::While(cond, body))
            }
            "loop" => {
                self.next();
                let body = self.parse_block().unwrap_or_default();
                Some(Stmt::Loop(body))
            }
            "for" => {
                self.next();
                let name = self.expect_ident()?;
                if !self.expect_kw("in") {
                    let line = self.line();
                    self.note(line, 4, format!("'for {}' missing 'in'; iterating null", name));
                }
                let iter = self.parse_expr();
                let body = self.parse_block().unwrap_or_default();
                Some(Stmt::For(name, iter, body))
            }
            "return" => {
                self.next();
                let e = match self.peek() {
                    Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof => Expr::Null,
                    _ => self.parse_expr(),
                };
                self.end_stmt();
                Some(Stmt::Return(Some(e)))
            }
            "break" => {
                self.next();
                self.end_stmt();
                Some(Stmt::Break)
            }
            "continue" => {
                self.next();
                self.end_stmt();
                Some(Stmt::Continue)
            }
            "match" => {
                self.next();
                let subject = self.parse_expr();
                let mut cases: Vec<(MatchPat, Vec<Stmt>)> = Vec::new();
                if matches!(self.peek(), Tok::LBrace) {
                    self.next();
                    loop {
                        self.eat_newlines();
                        if matches!(self.peek(), Tok::RBrace) {
                            self.next();
                            break;
                        }
                        if matches!(self.peek(), Tok::Eof) {
                            let line = self.line();
                            self.note(line, 4, "match block auto-closed at end of file");
                            break;
                        }
                        if !self.expect_kw("case") {
                            let line = self.line();
                            self.note(line, 4, "expected 'case' in match; skipped token");
                            self.next();
                            continue;
                        }
                        let pat = self.parse_pattern();
                        let body = if matches!(self.peek(), Tok::FatArrow) {
                            let line = self.line();
                            self.note(line, 2, "'=> expr' case form repaired to block form");
                            self.next();
                            let e = self.parse_expr();
                            self.end_stmt();
                            vec![Stmt::ExprStmt(e)]
                        } else {
                            self.parse_block().unwrap_or_default()
                        };
                        cases.push((pat, body));
                    }
                } else {
                    let line = self.line();
                    self.note(line, 4, "match without cases; treated as null");
                }
                Some(Stmt::Match(subject, cases))
            }
            "use" => {
                self.next();
                let path = self.parse_use_path();
                let mut alias = None;
                if self.expect_kw("as") {
                    alias = Some(self.expect_ident().unwrap_or_else(|| "mod".to_string()));
                }
                self.end_stmt();
                Some(Stmt::Use(path, alias))
            }
            "tad" => {
                self.next();
                let name = self.expect_ident()?;
                let body = self.parse_block().unwrap_or_default();
                Some(Stmt::Tad(name, body))
            }
            "anchor" => {
                self.next();
                let is_export = self.expect_kw("export");
                if !is_export {
                    self.expect_kw("import");
                }
                let mut names = vec![self.expect_ident().unwrap_or_default()];
                while matches!(self.peek(), Tok::Comma) {
                    self.next();
                    names.push(self.expect_ident().unwrap_or_default());
                }
                names.retain(|n| !n.is_empty());
                self.end_stmt();
                Some(if is_export {
                    Stmt::AnchorExport(names)
                } else {
                    Stmt::AnchorImport(names)
                })
            }
            "enhance" => {
                self.next();
                let mut names = vec![self.expect_ident().unwrap_or_default()];
                while matches!(self.peek(), Tok::Comma) {
                    self.next();
                    names.push(self.expect_ident().unwrap_or_default());
                }
                names.retain(|n| !n.is_empty());
                self.end_stmt();
                Some(Stmt::Enhance(names))
            }
            "silence" => {
                self.next();
                let from = self.expect_ident()?;
                let mut to = None;
                if matches!(self.peek(), Tok::Arrow) {
                    self.next();
                    to = Some(self.expect_ident()?);
                }
                self.end_stmt();
                Some(Stmt::Silence(from, to))
            }
            "stress" => {
                self.next();
                let kind = if let Tok::Ident(w) = self.peek().clone() {
                    if !is_canonical(&w) || w == "missing" || w == "unfolded" || w == "overflow" || w == "burned" || w == "any" {
                        // only treat as kind if followed by a block
                        if matches!(&self.toks[self.pos + 1].0, Tok::LBrace) {
                            self.next();
                            Some(w)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                let body = self.parse_block().unwrap_or_default();
                let mut rescue = None;
                self.eat_newlines();
                if self.expect_kw("rescue") {
                    let binding = if matches!(self.peek(), Tok::LParen) {
                        self.next();
                        let b = match self.peek().clone() {
                            Tok::Ident(n) => {
                                self.next();
                                Some(n)
                            }
                            _ => None,
                        };
                        if matches!(self.peek(), Tok::RParen) {
                            self.next();
                        }
                        b
                    } else {
                        None
                    };
                    let rbody = self.parse_block().unwrap_or_default();
                    rescue = Some((binding, rbody));
                }
                Some(Stmt::Stress { kind, body, rescue })
            }
            "raise" => {
                self.next();
                // raise kind, msg   |   raise msg
                let save = self.pos;
                if let Tok::Ident(k) = self.peek().clone() {
                    if matches!(k.as_str(), "unfolded" | "missing" | "overflow" | "burned") {
                        self.next();
                        if matches!(self.peek(), Tok::Comma) {
                            self.next();
                            let msg = self.parse_expr();
                            self.end_stmt();
                            return Some(Stmt::Raise(Some(k), msg));
                        }
                        self.pos = save;
                    }
                }
                let msg = self.parse_expr();
                self.end_stmt();
                Some(Stmt::Raise(None, msg))
            }
            "fate" => {
                self.next();
                let name = self.expect_ident()?;
                let mut states: Vec<(String, Vec<String>)> = Vec::new();
                let mut enter = None;
                if matches!(self.peek(), Tok::LBrace) {
                    self.next();
                    loop {
                        self.eat_newlines();
                        match self.peek().clone() {
                            Tok::RBrace => {
                                self.next();
                                break;
                            }
                            Tok::Eof => {
                                let line = self.line();
                                self.note(line, 4, "fate block auto-closed");
                                break;
                            }
                            Tok::Ident(w) if w == "state" || synonym(&w) == Some("state") || crate::ffi::edit_distance(&w, "state") <= 1 => {
                                if w != "state" {
                                    let line = self.line();
                                    self.note(line, 3, format!("wobble: '{}' repaired to 'state'", w));
                                }
                                self.next();
                                let from = self.expect_ident().unwrap_or_default();
                                let mut targets = Vec::new();
                                if matches!(self.peek(), Tok::Arrow) {
                                    self.next();
                                    targets.push(self.expect_ident().unwrap_or_default());
                                    while matches!(self.peek(), Tok::Comma) {
                                        self.next();
                                        targets.push(self.expect_ident().unwrap_or_default());
                                    }
                                }
                                targets.retain(|t| !t.is_empty());
                                states.push((from, targets));
                                self.end_stmt();
                            }
                            Tok::Ident(w) if w == "enter" || crate::ffi::edit_distance(&w, "enter") <= 1 => {
                                if w != "enter" {
                                    let line = self.line();
                                    self.note(line, 3, format!("wobble: '{}' repaired to 'enter'", w));
                                }
                                self.next();
                                enter = Some(self.expect_ident().unwrap_or_default());
                                self.end_stmt();
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "unexpected token in fate block; skipped");
                                self.next();
                            }
                        }
                    }
                }
                let fname = name.clone();
                Some(Stmt::Fate(std::sync::Arc::new(FateDef { name: fname, states, enter })))
            }
            "regulate" => {
                self.next();
                let mut edges = Vec::new();
                if matches!(self.peek(), Tok::LBrace) {
                    self.next();
                    loop {
                        self.eat_newlines();
                        match self.peek().clone() {
                            Tok::RBrace => {
                                self.next();
                                break;
                            }
                            Tok::Eof => break,
                            Tok::Ident(from) => {
                                self.next();
                                let inhibit = if self.expect_kw("activates") {
                                    false
                                } else if self.expect_kw("inhibits") {
                                    true
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "regulate edge missing 'activates'/'inhibits'; edge dropped");
                                    self.skip_line();
                                    continue;
                                };
                                let to = self.expect_ident().unwrap_or_default();
                                let mut strength = 1.0;
                                if self.expect_kw("strength") {
                                    match self.peek().clone() {
                                        Tok::Float(f) => {
                                            self.next();
                                            strength = f;
                                        }
                                        Tok::Int(i) => {
                                            self.next();
                                            strength = i as f64;
                                        }
                                        _ => {
                                            let line = self.line();
                                            self.note(line, 4, "strength needs a number; using 1.0");
                                        }
                                    }
                                }
                                let mut threshold = None;
                                if self.expect_kw("threshold") {
                                    match self.peek().clone() {
                                        Tok::Float(f) => {
                                            self.next();
                                            threshold = Some(f);
                                        }
                                        Tok::Int(i) => {
                                            self.next();
                                            threshold = Some(i as f64);
                                        }
                                        _ => {
                                            let line = self.line();
                                            self.note(line, 4, "threshold needs a number; ignored");
                                        }
                                    }
                                }
                                edges.push(RegEdge { from, to, strength, inhibit, threshold });
                                self.end_stmt();
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "unexpected token in regulate block; skipped");
                                self.next();
                            }
                        }
                    }
                }
                Some(Stmt::Regulate(edges))
            }
            "toggle" => {
                self.next();
                let a = self.expect_ident()?;
                let b = if matches!(self.peek(), Tok::Comma) {
                    self.next();
                    self.expect_ident().unwrap_or_default()
                } else {
                    let line = self.line();
                    self.note(line, 4, "toggle needs two genes; second bound to first");
                    a.clone()
                };
                self.end_stmt();
                Some(Stmt::Toggle(a, b))
            }
            "repressilator" => {
                self.next();
                let mut ring = vec![self.expect_ident()?];
                while matches!(self.peek(), Tok::Arrow) {
                    self.next();
                    ring.push(self.expect_ident().unwrap_or_default());
                }
                ring.retain(|r| !r.is_empty());
                let mut period = None;
                if self.expect_kw("period") {
                    match self.peek().clone() {
                        Tok::Int(i) => {
                            self.next();
                            period = Some(i as f64);
                        }
                        Tok::Float(f) => {
                            self.next();
                            period = Some(f);
                        }
                        _ => {}
                    }
                }
                self.end_stmt();
                Some(Stmt::Repressilator(ring, period))
            }
            "frame" => {
                self.next();
                let is_proof = self.expect_kw("proof");
                let name = if is_proof {
                    "proof".to_string()
                } else {
                    self.expect_ident().unwrap_or_else(|| "frame".into())
                };
                let body = self.parse_block().unwrap_or_default();
                Some(Stmt::Frame { name, is_proof, body })
            }
            "splice" => {
                self.next();
                let root = self.expect_ident()?;
                let mut variants = Vec::new();
                if matches!(self.peek(), Tok::LBrace) {
                    self.next();
                    loop {
                        self.eat_newlines();
                        match self.peek().clone() {
                            Tok::RBrace => {
                                self.next();
                                break;
                            }
                            Tok::Eof => {
                                let line = self.line();
                                self.note(line, 4, "splice block auto-closed");
                                break;
                            }
                            Tok::Ident(w)
                                if w == "variant" || synonym(&w) == Some("variant") || crate::ffi::edit_distance(&w, "variant") <= 1 =>
                            {
                                if w != "variant" {
                                    let line = self.line();
                                    self.note(line, 3, format!("wobble: '{}' repaired to 'variant'", w));
                                }
                                self.next();
                                let vname = self.expect_ident().unwrap_or_else(|| "v".into());
                                // optional parameter list on variants
                                let mut params: Vec<(String, Option<Expr>)> = Vec::new();
                                if matches!(self.peek(), Tok::LParen) {
                                    self.next();
                                    loop {
                                        self.eat_newlines_inline();
                                        if matches!(self.peek(), Tok::RParen) {
                                            self.next();
                                            break;
                                        }
                                        if matches!(self.peek(), Tok::Eof) {
                                            break;
                                        }
                                        let before = self.pos;
                                        let pname = self.expect_ident().unwrap_or_default();
                                        let default = if matches!(self.peek(), Tok::Eq) {
                                            self.next();
                                            Some(self.parse_expr())
                                        } else {
                                            None
                                        };
                                        params.push((pname, default));
                                        if matches!(self.peek(), Tok::Comma) {
                                            self.next();
                                        }
                                        if self.pos == before {
                                            self.note(self.line(), 4, "unclosed variant parameter list; auto-closed");
                                            break;
                                        }
                                    }
                                }
                                let body = self.parse_block().unwrap_or_default();
                                variants.push((
                                    vname,
                                    std::sync::Arc::new(GeneDef {
                                        name: Some(root.clone()),
                                        params,
                                        guard: None,
                                        body,
                                        acetylate: false,
                                        methylate: false,
                                        m6a: false,
                                        seq: false,
                                    }),
                                ));
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "unexpected token in splice block; skipped");
                                self.next();
                            }
                        }
                    }
                }
                if variants.is_empty() {
                    let line = self.line();
                    self.note(line, 4, format!("splice '{}' has no variants; binds null", root));
                }
                Some(Stmt::Splice(std::sync::Arc::new(SpliceDef { root, variants })))
            }
            "edit" => {
                self.next();
                let target = self.expect_ident()?;
                let mut reps = Vec::new();
                if matches!(self.peek(), Tok::LBrace) {
                    self.next();
                    loop {
                        self.eat_newlines();
                        match self.peek().clone() {
                            Tok::RBrace => {
                                self.next();
                                break;
                            }
                            Tok::Eof => break,
                            _ => {
                                if self.expect_kw("replace") {
                                    let from = match self.peek().clone() {
                                        Tok::Str(s) | Tok::Interp(s) => {
                                            self.next();
                                            s
                                        }
                                        other => {
                                            let line = self.line();
                                            self.note(line, 4, "edit needs string patterns");
                                            self.next();
                                            format!("{:?}", other)
                                        }
                                    };
                                    if matches!(self.peek(), Tok::Arrow) {
                                        self.next();
                                    }
                                    let to = match self.peek().clone() {
                                        Tok::Str(s) | Tok::Interp(s) => {
                                            self.next();
                                            s
                                        }
                                        other => {
                                            format!("{:?}", other)
                                        }
                                    };
                                    reps.push((from, to));
                                    self.end_stmt();
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "expected 'replace' in edit block; skipped");
                                    self.next();
                                }
                            }
                        }
                    }
                }
                Some(Stmt::Edit(target, reps))
            }
            "ires" => {
                self.next();
                let name = self.expect_ident()?;
                self.end_stmt();
                Some(Stmt::Ires(name))
            }
            "phenotype" => {
                self.next();
                let name = self.expect_ident()?;
                let mut parent = None;
                if self.at_kw("from") {
                    self.next();
                    parent = Some(self.expect_ident().unwrap_or_default());
                }
                let mut fields = Vec::new();
                let mut methods = Vec::new();
                if matches!(self.peek(), Tok::LBrace) {
                    self.next();
                    loop {
                        self.eat_newlines();
                        match self.peek().clone() {
                            Tok::RBrace => {
                                self.next();
                                break;
                            }
                            Tok::Eof => {
                                let line = self.line();
                                self.note(line, 4, "phenotype body auto-closed");
                                break;
                            }
                            Tok::Mark(m) => {
                                // marks on methods: reuse mark parsing
                                let line = self.line();
                                self.next();
                                if let Some(mark) = self.repair_mark(m, line) {
                                    let marks = vec![mark];
                                    if self.expect_kw("gene") {
                                        if let Some(Stmt::Gene(g)) = Some(self.parse_gene_def(marks)) {
                                            methods.push(g);
                                        }
                                    } else {
                                        self.note(line, 4, "mark inside phenotype must precede 'gene'; skipped");
                                        self.skip_line();
                                    }
                                }
                            }
                            Tok::Ident(w) if w == "gene" || synonym(&w) == Some("gene") => {
                                if w != "gene" {
                                    let line = self.line();
                                    self.note(line, 2, format!("synonym '{}' repaired to 'gene'", w));
                                }
                                self.next();
                                if let Some(Stmt::Gene(g)) = Some(self.parse_gene_def(vec![])) {
                                    methods.push(g);
                                }
                            }
                            Tok::Ident(w) if w == "let" || synonym(&w) == Some("let") => {
                                if w != "let" {
                                    let line = self.line();
                                    self.note(line, 2, format!("synonym '{}' repaired to 'let'", w));
                                }
                                self.next();
                                let fname = self.expect_ident().unwrap_or_default();
                                let default = if matches!(self.peek(), Tok::Eq) {
                                    self.next();
                                    self.parse_expr()
                                } else {
                                    Expr::Null
                                };
                                self.end_stmt();
                                if !fname.is_empty() && fname != "?" {
                                    fields.push((fname, default));
                                }
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "unexpected token in phenotype body; skipped");
                                let before = self.pos;
                                self.next();
                                if self.pos == before {
                                    break;
                                }
                            }
                        }
                    }
                }
                let pname = name.clone();
                Some(Stmt::Pheno(std::sync::Arc::new(PhenoDef { name: pname, parent, fields, methods })))
            }
            "sequence" => {
                self.next();
                let def = self.parse_gene_def(vec![]);
                match def {
                    Stmt::Gene(g) => {
                        let mut g2 = (*g).clone();
                        g2.seq = true;
                        Some(Stmt::Seq(std::sync::Arc::new(g2)))
                    }
                    other => Some(other),
                }
            }
            "yield" => {
                self.next();
                let e = match self.peek() {
                    Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof => None,
                    _ => Some(self.parse_expr()),
                };
                self.end_stmt();
                Some(Stmt::Yield(e))
            }
            _ => {
                // bare-name definition: `main { ... }` and `route(req) { ... }`
                // are gene definitions — the C-like entry/route idiom.
                // Lookahead: NAME '{' or NAME '(' ... ')' '{' (a plain call
                // like `promote(x)` is followed by other tokens, not '{').
                let is_def = {
                    let toks = &self.toks;
                    let n = toks.len();
                    let mut j = self.pos + 1;
                    let mut looks = false;
                    if j < n && matches!(toks[j].0, Tok::LBrace) {
                        looks = true;
                    } else if j < n && matches!(toks[j].0, Tok::LParen) {
                        // scan to the matching close paren, then expect '{'
                        let mut depth = 0i32;
                        while j < n {
                            match toks[j].0 {
                                Tok::LParen => depth += 1,
                                Tok::RParen => {
                                    depth -= 1;
                                    if depth == 0 {
                                        j += 1;
                                        // skip newlines/semis between ) and {
                                        while j < n
                                            && matches!(toks[j].0, Tok::Newline | Tok::Semi)
                                        {
                                            j += 1;
                                        }
                                        looks = j < n && matches!(toks[j].0, Tok::LBrace);
                                        break;
                                    }
                                }
                                Tok::Eof => break,
                                _ => {}
                            }
                            j += 1;
                        }
                    }
                    looks
                };
                if !is_canonical(&word) && is_def {
                    let line = self.line();
                    self.note(line, 4, format!("bare name block '{word}' treated as gene definition"));
                    self.next(); // consume the name
                    let mut params: Vec<(String, Option<Expr>)> = Vec::new();
                    if matches!(self.peek(), Tok::LParen) {
                        self.next(); // (
                        loop {
                            self.eat_newlines_inline();
                            if matches!(self.peek(), Tok::RParen) {
                                self.next();
                                break;
                            }
                            if matches!(self.peek(), Tok::Eof) {
                                self.note(self.line(), 4, "parameter list auto-closed");
                                break;
                            }
                            let before = self.pos;
                            let pname = self.expect_ident().unwrap_or_default();
                            let default = if matches!(self.peek(), Tok::Eq) {
                                self.next();
                                Some(self.parse_expr())
                            } else {
                                None
                            };
                            params.push((pname, default));
                            if matches!(self.peek(), Tok::Comma) {
                                self.next();
                            }
                            if self.pos == before {
                                self.note(self.line(), 4, "unclosed parameter list; auto-closed");
                                break;
                            }
                        }
                    }
                    let body = self.parse_block().unwrap_or_default();
                    return Some(Stmt::Gene(std::sync::Arc::new(GeneDef {
                        name: Some(word.clone()),
                        params,
                        guard: None,
                        body,
                        acetylate: false,
                        methylate: false,
                        m6a: false,
                        seq: false,
                    })));
                }
                self.parse_assign_or_expr(w)
            }
        }
    }

    fn parse_use_path(&mut self) -> String {
        // path := segment (('/' | '.' | '-') segment)* ; segments are plain
        // identifiers. A boundary word ('as', statement keyword, etc.) ends
        // the path — it is never glued into it.
        let mut cur = String::new();
        loop {
            match self.peek().clone() {
                Tok::Ident(w) => {
                    if use_path_boundary(&w) {
                        break;
                    }
                    cur.push_str(&w);
                    self.next();
                    // after a segment, only a separator may continue the path
                    if !matches!(self.peek(), Tok::Slash | Tok::Dot | Tok::Minus) {
                        break;
                    }
                }
                Tok::Slash => {
                    cur.push('/');
                    self.next();
                }
                Tok::Dot => {
                    cur.push('.');
                    self.next();
                }
                Tok::Minus => {
                    cur.push('-');
                    self.next();
                }
                Tok::Str(s) => {
                    cur.push_str(&s);
                    self.next();
                    if !matches!(self.peek(), Tok::Slash | Tok::Dot | Tok::Minus) {
                        break;
                    }
                }
                _ => break,
            }
        }
        cur
    }

    fn expect_ident(&mut self) -> Option<String> {
        match self.peek().clone() {
            Tok::Ident(w) => {
                self.next();
                Some(w)
            }
            other => {
                let line = self.line();
                self.note(line, 4, format!("expected a name, found '{:?}'; used '?'", other));
                Some("?".to_string())
            }
        }
    }

    /// Consume statement end: newline or ';' (tolerant).
    fn end_stmt(&mut self) {
        if matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.next();
        }
    }

    fn parse_assign_or_expr(&mut self, w: &str) -> Option<Stmt> {
        // name = ... | name += ... | expression statement
        enum A {
            Plain,
            Compound(BinOp),
            None,
        }
        let t1 = self.toks.get(self.pos + 1).map(|t| t.0.clone());
        let kind = match &t1 {
            Some(Tok::Eq) => A::Plain,
            Some(Tok::PlusEq) => A::Compound(BinOp::Add),
            Some(Tok::MinusEq) => A::Compound(BinOp::Sub),
            Some(Tok::StarEq) => A::Compound(BinOp::Mul),
            Some(Tok::SlashEq) => A::Compound(BinOp::Div),
            Some(Tok::DSlashEq) => A::Compound(BinOp::FloorDiv),
            Some(Tok::PercentEq) => A::Compound(BinOp::Mod),
            _ => A::None,
        };
        match kind {
            A::Plain => {
                self.next(); // name
                self.next(); // =
                let val = self.parse_expr();
                self.end_stmt();
                Some(Stmt::Assign(w.to_string(), None, val))
            }
            A::Compound(op) => {
                self.next(); // name
                self.next(); // op=
                let val = self.parse_expr();
                self.end_stmt();
                Some(Stmt::Assign(w.to_string(), Some(op), val))
            }
            A::None => {
                // expression statement (may be index/member assignment)
                let e = self.parse_expr();
                match self.peek().clone() {
                    Tok::Eq => {
                        self.next();
                        let val = self.parse_expr();
                        self.end_stmt();
                        match e {
                            Expr::Index(t, i) => {
                                Some(Stmt::IndexAssign(*t, *i, None, val))
                            }
                            Expr::Member(t, k) => Some(Stmt::MemberAssign(*t, k, None, val)),
                            other => {
                                let line = self.line();
                                self.note(
                                    line,
                                    4,
                                    "assignment target must be a name, index or member; value computed and dropped",
                                );
                                Some(Stmt::ExprStmt(other))
                            }
                        }
                    }
                    Tok::PlusEq | Tok::MinusEq | Tok::StarEq | Tok::SlashEq | Tok::DSlashEq
                    | Tok::PercentEq => {
                        let op = match self.peek().clone() {
                            Tok::PlusEq => BinOp::Add,
                            Tok::MinusEq => BinOp::Sub,
                            Tok::StarEq => BinOp::Mul,
                            Tok::SlashEq => BinOp::Div,
                            Tok::DSlashEq => BinOp::FloorDiv,
                            _ => BinOp::Mod,
                        };
                        self.next(); // op=
                        let val = self.parse_expr();
                        self.end_stmt();
                        match e {
                            Expr::Index(t, i) => Some(Stmt::IndexAssign(*t, *i, Some(op), val)),
                            Expr::Member(t, k) => Some(Stmt::MemberAssign(*t, k, Some(op), val)),
                            other => {
                                let line = self.line();
                                self.note(line, 4, "compound assignment target invalid; dropped");
                                Some(Stmt::ExprStmt(other))
                            }
                        }
                    }
                    _ => {
                        // same-line garbage check BEFORE consuming the terminator
                        if !matches!(
                            self.peek(),
                            Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof
                        ) {
                            let line = self.line();
                            self.note(line, 4, "expression statement not terminated; rest of line skipped");
                            self.skip_line();
                        }
                        self.end_stmt();
                        Some(Stmt::ExprStmt(e))
                    }
                }
            }
        }
    }

    fn parse_gene_def(&mut self, marks: Vec<String>) -> Stmt {
        let acetylate = marks.iter().any(|m| m == "acetylate");
        let methylate = marks.iter().any(|m| m == "methylate");
        let m6a = marks.iter().any(|m| m == "m6a");
        let name = match self.peek().clone() {
            Tok::Ident(w) if w != "guard" => {
                self.next();
                Some(w)
            }
            _ => None,
        };
        let mut params = Vec::new();
        if matches!(self.peek(), Tok::LParen) {
            self.next();
            loop {
                self.eat_newlines_inline();
                if matches!(self.peek(), Tok::RParen) {
                    self.next();
                    break;
                }
                if matches!(self.peek(), Tok::Eof) {
                    let line = self.line();
                    self.note(line, 4, "parameter list auto-closed");
                    break;
                }
                let before = self.pos;
                let pname = self.expect_ident().unwrap_or_default();
                let default = if matches!(self.peek(), Tok::Eq) {
                    self.next();
                    Some(self.parse_expr())
                } else {
                    None
                };
                params.push((pname, default));
                if matches!(self.peek(), Tok::Comma) {
                    self.next();
                }
                if self.pos == before {
                    // no progress: non-parameter token inside the signature —
                    // auto-close instead of spinning (Total Grammar recovery)
                    let line = self.line();
                    self.note(line, 4, "unclosed parameter list; auto-closed");
                    break;
                }
            }
        }
        // uORF leading guard
        let mut guard = None;
        self.eat_newlines_inline();
        if self.expect_kw("guard") {
            if matches!(self.peek(), Tok::LParen) {
                self.next();
                let cond = self.parse_expr();
                if matches!(self.peek(), Tok::RParen) {
                    self.next();
                } else {
                    let line = self.line();
                    self.note(line, 4, "guard condition auto-closed");
                }
                self.expect_kw("else");
                let gbody = self.parse_block().unwrap_or_default();
                guard = Some((cond, gbody));
            } else {
                let line = self.line();
                self.note(line, 4, "guard without condition ignored");
            }
        }
        // body
        self.eat_newlines_inline();
        if matches!(self.peek(), Tok::FatArrow) {
            self.next();
            let e = self.parse_expr();
            self.end_stmt();
            let def = GeneDef {
                name,
                params,
                guard,
                body: vec![Stmt::Return(Some(e))],
                acetylate,
                methylate,
                m6a,
                seq: false,
            };
            return Stmt::Gene(std::sync::Arc::new(def));
        }
        let body = self.parse_block().unwrap_or_default();
        let def = GeneDef { name, params, guard, body, acetylate, methylate, m6a, seq: false };
        Stmt::Gene(std::sync::Arc::new(def))
    }

    /// consume newlines/semis inside nested constructs (parens, param lists)
    fn eat_newlines_inline(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.next();
        }
    }

    pub fn parse_block(&mut self) -> Option<Vec<Stmt>> {
        if !matches!(self.peek(), Tok::LBrace) {
            // tolerate single statement without braces
            let line = self.line();
            self.note(line, 4, "block without braces; single statement accepted");
            let s = self.parse_stmt()?;
            return Some(vec![s]);
        }
        self.next(); // {
        let mut out = Vec::new();
        loop {
            self.eat_newlines();
            match self.peek().clone() {
                Tok::RBrace => {
                    self.next();
                    break;
                }
                Tok::Eof => {
                    let line = self.line();
                    self.note(line, 4, "block auto-closed at end of file");
                    break;
                }
                _ => {
                    let before = self.pos;
                    match self.parse_stmt() {
                        Some(s) => out.push(s),
                        None => {}
                    }
                    if self.pos == before {
                        let line = self.line();
                        self.note(line, 4, format!("token '{:?}' skipped", self.peek()));
                        self.next();
                    }
                }
            }
        }
        Some(out)
    }

    // ---- expressions ----------------------------------------------------
    pub fn parse_expr(&mut self) -> Expr {
        // ternary: cond ? a : b (right-assoc, lowest precedence)
        let cond = self.parse_or();
        if matches!(self.peek(), Tok::Question) {
            let line = self.line();
            self.next();
            let a = self.parse_expr();
            if matches!(self.peek(), Tok::Colon) {
                self.next();
            } else {
                self.note(line, 4, "ternary missing ':'; else-branch is null");
            }
            let b = self.parse_expr();
            return Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b));
        }
        cond
    }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        loop {
            let is_or = self.at_kw("or")
                || matches!(self.peek(), Tok::PipePipe);
            if is_or {
                if self.at_kw("or") {
                    self.next();
                } else {
                    self.next();
                }
                let right = self.parse_and();
                left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_not();
        loop {
            let is_and = self.at_kw("and") || matches!(self.peek(), Tok::AmpAmp);
            if is_and {
                self.next();
                let right = self.parse_not();
                left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_not(&mut self) -> Expr {
        if self.at_kw("not") || matches!(self.peek(), Tok::Bang) {
            self.next();
            let e = self.parse_not();
            return Expr::Unary(UnOp::Not, Box::new(e));
        }
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Expr {
        let mut left = self.parse_bitor();
        loop {
            let op = match self.peek() {
                Tok::EqEq => Some(BinOp::Eq),
                Tok::Neq => Some(BinOp::Neq),
                Tok::Lt => Some(BinOp::Lt),
                Tok::Le => Some(BinOp::Le),
                Tok::Gt => Some(BinOp::Gt),
                Tok::Ge => Some(BinOp::Ge),
                Tok::Ident(w) if w == "in" => Some(BinOp::In),
                _ => None,
            };
            if let Some(op) = op {
                self.next();
                let right = self.parse_bitor();
                left = Expr::Binary(op, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_bitor(&mut self) -> Expr {
        let mut left = self.parse_bitxor();
        while matches!(self.peek(), Tok::Pipe) {
            self.next();
            let right = self.parse_bitxor();
            left = Expr::Binary(BinOp::BitOr, Box::new(left), Box::new(right));
        }
        left
    }

    fn parse_bitxor(&mut self) -> Expr {
        let mut left = self.parse_bitand();
        while matches!(self.peek(), Tok::Caret) {
            self.next();
            let right = self.parse_bitand();
            left = Expr::Binary(BinOp::BitXor, Box::new(left), Box::new(right));
        }
        left
    }

    fn parse_bitand(&mut self) -> Expr {
        let mut left = self.parse_shift();
        while matches!(self.peek(), Tok::Amp) {
            self.next();
            let right = self.parse_shift();
            left = Expr::Binary(BinOp::BitAnd, Box::new(left), Box::new(right));
        }
        left
    }

    fn parse_shift(&mut self) -> Expr {
        let mut left = self.parse_add();
        loop {
            let op = match self.peek() {
                Tok::Shl => Some(BinOp::Shl),
                Tok::Shr => Some(BinOp::Shr),
                _ => None,
            };
            if let Some(op) = op {
                self.next();
                let right = self.parse_add();
                left = Expr::Binary(op, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_add(&mut self) -> Expr {
        let mut left = self.parse_mul();
        loop {
            let op = match self.peek() {
                Tok::Plus => Some(BinOp::Add),
                Tok::Minus => Some(BinOp::Sub),
                _ => None,
            };
            if let Some(op) = op {
                self.next();
                let right = self.parse_mul();
                left = Expr::Binary(op, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_mul(&mut self) -> Expr {
        let mut left = self.parse_unary();
        loop {
            let op = match self.peek() {
                Tok::Star => Some(BinOp::Mul),
                Tok::Slash => Some(BinOp::Div),
                Tok::DSlash => Some(BinOp::FloorDiv),
                Tok::Percent => Some(BinOp::Mod),
                _ => None,
            };
            if let Some(op) = op {
                self.next();
                let right = self.parse_unary();
                left = Expr::Binary(op, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_unary(&mut self) -> Expr {
        if matches!(self.peek(), Tok::Minus) {
            self.next();
            let e = self.parse_unary();
            return Expr::Unary(UnOp::Neg, Box::new(e));
        }
        if matches!(self.peek(), Tok::Tilde) {
            self.next();
            let e = self.parse_unary();
            return Expr::Unary(UnOp::BitNot, Box::new(e));
        }
        self.parse_pow()
    }

    fn parse_pow(&mut self) -> Expr {
        // '**' binds tighter than unary minus on the left (-2**2 = -4) and is
        // right-associative (2**3**2 = 2**9). The right operand re-enters
        // parse_unary so 2**-3 parses.
        let left = self.parse_postfix();
        if matches!(self.peek(), Tok::StarStar) {
            self.next();
            let right = self.parse_unary();
            return Expr::Binary(BinOp::Pow, Box::new(left), Box::new(right));
        }
        left
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut e = self.parse_primary();
        loop {
            match self.peek().clone() {
                Tok::LParen => {
                    self.next();
                    let mut args = Vec::new();
                    loop {
                        self.eat_newlines_inline();
                        if matches!(self.peek(), Tok::RParen) {
                            self.next();
                            break;
                        }
                        if matches!(self.peek(), Tok::Eof) {
                            let line = self.line();
                            self.note(line, 4, "call arguments auto-closed");
                            break;
                        }
                        args.push(self.parse_expr());
                        if matches!(self.peek(), Tok::Comma) {
                            self.next();
                        }
                    }
                    e = Expr::Call(Box::new(e), args);
                }
                Tok::LBrack => {
                    self.next();
                    let idx = self.parse_expr();
                    if matches!(self.peek(), Tok::RBrack) {
                        self.next();
                    } else {
                        let line = self.line();
                        self.note(line, 4, "index bracket auto-closed");
                    }
                    e = Expr::Index(Box::new(e), Box::new(idx));
                }
                Tok::Dot => {
                    self.next();
                    match self.peek().clone() {
                        Tok::Ident(m) => {
                            self.next();
                            if matches!(self.peek(), Tok::LParen) {
                                self.next();
                                let mut args = Vec::new();
                                loop {
                                    self.eat_newlines_inline();
                                    if matches!(self.peek(), Tok::RParen) {
                                        self.next();
                                        break;
                                    }
                                    if matches!(self.peek(), Tok::Eof) {
                                        break;
                                    }
                                    args.push(self.parse_expr());
                                    if matches!(self.peek(), Tok::Comma) {
                                        self.next();
                                    }
                                }
                                e = Expr::Method(Box::new(e), m, args);
                            } else {
                                e = Expr::Member(Box::new(e), m);
                            }
                        }
                        other => {
                            let line = self.line();
                            self.note(line, 4, format!("'.' followed by '{:?}'; member skipped", other));
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        e
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek().clone() {
            Tok::Int(i) => {
                self.next();
                Expr::Int(i)
            }
            Tok::Float(f) => {
                self.next();
                Expr::Float(f)
            }
            Tok::Str(s) => {
                self.next();
                Expr::Str(s)
            }
            Tok::Interp(raw) => {
                let line = self.line();
                self.next();
                self.build_interp(raw, line)
            }
            Tok::LParen => {
                self.next();
                let e = self.parse_expr();
                if matches!(self.peek(), Tok::RParen) {
                    self.next();
                } else {
                    let line = self.line();
                    self.note(line, 4, "parenthesis auto-closed");
                }
                e
            }
            Tok::LBrack => {
                self.next();
                let mut items = Vec::new();
                loop {
                    self.eat_newlines_inline();
                    if matches!(self.peek(), Tok::RBrack) {
                        self.next();
                        break;
                    }
                    if matches!(self.peek(), Tok::Eof) {
                        let line = self.line();
                        self.note(line, 4, "list auto-closed at end of file");
                        break;
                    }
                    items.push(self.parse_expr());
                    if matches!(self.peek(), Tok::Comma) {
                        self.next();
                    } else if !matches!(self.peek(), Tok::RBrack) {
                        let line = self.line();
                        self.note(line, 4, "list items separated automatically");
                    }
                }
                Expr::List(items)
            }
            Tok::LBrace => self.parse_map_literal(),
            Tok::Ident(w) => {
                match w.as_str() {
                    "true" | "yes" | "on" => {
                        if w != "true" {
                            let line = self.line();
                            self.note(line, 2, format!("synonym '{}' repaired to 'true'", w));
                        }
                        self.next();
                        Expr::Bool(true)
                    }
                    "false" | "no" | "off" => {
                        if w != "false" {
                            let line = self.line();
                            self.note(line, 2, format!("synonym '{}' repaired to 'false'", w));
                        }
                        self.next();
                        Expr::Bool(false)
                    }
                    "null" | "nil" | "none" | "nothing" => {
                        if w != "null" {
                            let line = self.line();
                            self.note(line, 2, format!("synonym '{}' repaired to 'null'", w));
                        }
                        self.next();
                        Expr::Null
                    }
                    "gene" | "fn" | "func" | "def" | "lambda" => {
                        if w != "gene" {
                            let line = self.line();
                            self.note(line, 2, format!("synonym '{}' repaired to 'gene'", w));
                        }
                        self.next();
                        // anonymous lambda in expression position
                        match self.parse_gene_def(vec![]) {
                            Stmt::Gene(def) => Expr::Lambda(def),
                            _ => Expr::Null,
                        }
                    }
                    "new" => {
                        self.next();
                        let name = self.expect_ident().unwrap_or_default();
                        let mut args = Vec::new();
                        if matches!(self.peek(), Tok::LParen) {
                            self.next();
                            loop {
                                self.eat_newlines_inline();
                                if matches!(self.peek(), Tok::RParen) {
                                    self.next();
                                    break;
                                }
                                if matches!(self.peek(), Tok::Eof) {
                                    break;
                                }
                                let before = self.pos;
                                args.push(self.parse_expr());
                                if matches!(self.peek(), Tok::Comma) {
                                    self.next();
                                }
                                if self.pos == before {
                                    self.note(self.line(), 4, "unclosed constructor argument list; auto-closed");
                                    break;
                                }
                            }
                        }
                        Expr::New(name, args)
                    }
                    "for" => {
                        // collect expression: for x in iter (if cond)? collect body
                        self.next();
                        let var = self.expect_ident().unwrap_or_default();
                        self.expect_kw("in");
                        let iter = self.parse_expr();
                        let mut filter = None;
                        if self.at_kw("if") {
                            self.next();
                            filter = Some(Box::new(self.parse_expr()));
                        }
                        self.expect_kw("collect");
                        let body = self.parse_expr();
                        Expr::Collect { var, iter: Box::new(iter), filter, body: Box::new(body) }
                    }
                    _ => {
                        self.next();
                        Expr::Ident(w)
                    }
                }
            }
            other => {
                let line = self.line();
                self.note(
                    line,
                    4,
                    format!("unexpected token '{:?}' in expression; null substituted", other),
                );
                self.next();
                Expr::Null
            }
        }
    }

    fn parse_map_literal(&mut self) -> Expr {
        self.next(); // {
        let mut pairs: Vec<(Expr, Expr)> = Vec::new();
        loop {
            self.eat_newlines_inline();
            match self.peek().clone() {
                Tok::RBrace => {
                    self.next();
                    break;
                }
                Tok::Eof => {
                    let line = self.line();
                    self.note(line, 4, "map literal auto-closed at end of file");
                    break;
                }
                _ => {
                    let key = match self.peek().clone() {
                        Tok::Ident(k) => {
                            self.next();
                            Expr::Str(k)
                        }
                        Tok::Str(s) => {
                            self.next();
                            Expr::Str(s)
                        }
                        Tok::Int(i) => {
                            self.next();
                            Expr::Int(i)
                        }
                        _ => {
                            let line = self.line();
                            self.note(line, 4, "map key must be a name or string; key null");
                            self.next();
                            Expr::Null
                        }
                    };
                    if matches!(self.peek(), Tok::Colon) {
                        self.next();
                    } else {
                        let line = self.line();
                        self.note(line, 4, "map entry missing ':'; value null");
                    }
                    let val = self.parse_expr();
                    pairs.push((key, val));
                    if matches!(self.peek(), Tok::Comma) {
                        self.next();
                    }
                }
            }
        }
        Expr::Map(pairs)
    }

    fn build_interp(&mut self, raw: String, line: usize) -> Expr {
        // split "a{x}b{y}" into lit/expr parts; sub-parse each expression
        let mut parts: Vec<InterpPart> = Vec::new();
        let mut lit = String::new();
        let chars: Vec<char> = raw.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] == '{' {
                if !lit.is_empty() {
                    parts.push(InterpPart::Lit(std::mem::take(&mut lit)));
                }
                let mut depth = 1usize;
                let mut expr_txt = String::new();
                i += 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '{' {
                        depth += 1;
                    } else if chars[i] == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    expr_txt.push(chars[i]);
                    i += 1;
                }
                i += 1; // skip '}'
                let mut sub = parse_snippet(&expr_txt);
                let e = sub.parse_expr();
                self.notes.extend(sub.notes.drain(..).map(|mut n| {
                    n.line = line;
                    n
                }));
                parts.push(InterpPart::Expr(e));
            } else {
                lit.push(chars[i]);
                i += 1;
            }
        }
        if !lit.is_empty() {
            parts.push(InterpPart::Lit(lit));
        }
        Expr::Interp(parts)
    }

    fn parse_pattern(&mut self) -> MatchPat {
        let mut lits: Vec<Expr> = Vec::new();
        loop {
            // negative literals are legal patterns (case 1, -1 =>)
            let neg = matches!(self.peek(), Tok::Minus);
            if neg {
                self.next();
            }
            match self.peek().clone() {
                Tok::Int(i) => {
                    self.next();
                    let e = Expr::Int(if neg { -i } else { i });
                    lits.push(e);
                }
                Tok::Float(f) => {
                    self.next();
                    let e = Expr::Float(if neg { -f } else { f });
                    lits.push(e);
                }
                Tok::Str(s) => {
                    if neg {
                        let line = self.line();
                        self.note(line, 4, "'-' before a string pattern ignored");
                    }
                    self.next();
                    lits.push(Expr::Str(s));
                }
                Tok::Ident(w) if !neg && w == "true" => {
                    self.next();
                    lits.push(Expr::Bool(true));
                }
                Tok::Ident(w) if !neg && w == "false" => {
                    self.next();
                    lits.push(Expr::Bool(false));
                }
                Tok::Ident(w) if !neg && w == "null" => {
                    self.next();
                    lits.push(Expr::Null);
                }
                Tok::Ident(w) if !neg && w == "_" => {
                    self.next();
                    return MatchPat::Wild;
                }
                Tok::Ident(w) if !neg => {
                    self.next();
                    return MatchPat::Bind(w);
                }
                other => {
                    if neg {
                        // '-' consumed but no literal followed
                        let line = self.line();
                        self.note(line, 4, "dangling '-' in pattern treated as wildcard");
                    }
                    let line = self.line();
                    self.note(line, 4, format!("pattern '{:?}' treated as wildcard", other));
                    self.next();
                    return MatchPat::Wild;
                }
            }
            if matches!(self.peek(), Tok::Comma) {
                self.next();
            } else {
                break;
            }
        }
        if lits.len() == 1 {
            MatchPat::Lit(lits.remove(0))
        } else {
            MatchPat::Multi(lits)
        }
    }
}

fn parse_snippet(src: &str) -> Parser {
    let lexed = lex(src);
    Parser { toks: lexed.toks, pos: 0, notes: lexed.notes }
}
