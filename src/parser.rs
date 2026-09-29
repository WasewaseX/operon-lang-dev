//! parser.rs, the Total Grammar parser.
//!
//! Law: no token stream is rejected. The ladder:
//!   rung 1, canonical match
//!   rung 2, synonym keywords (noted, repaired)
//!   rung 3, wobble: edit-distance repair when a keyword is REQUIRED
//!   rung 4, semantic fallback: skip/auto-close/stringify, always noted
//!
//! Keyword repair only happens at positions where a keyword is syntactically
//! required, identifiers in binding positions are never touched.

use crate::ast::*;
use crate::lexer::{lex, Tok};

pub(crate) const KEYWORDS: &[&str] = &[
    "gene",
    "let",
    "trait",
    "if",
    "elif",
    "else",
    "while",
    "loop",
    "scope",
    "for",
    "in",
    "return",
    "break",
    "continue",
    "match",
    "case",
    "use",
    "tad",
    "anchor",
    "export",
    "import",
    "enhance",
    "silence",
    "stress",
    "rescue",
    "raise",
    "fate",
    "state",
    "regulate",
    "activates",
    "inhibits",
    "strength",
    "toggle",
    "repressilator",
    "period",
    "frame",
    "proof",
    "guard",
    "splice",
    "variant",
    "edit",
    "replace",
    "ires",
    "as",
    "collect",
    "enter",
    "phenotype",
    "sequence",
    "yield",
    "new",
    "threshold",
    "from",
    "self",
    "decoy",
    "ligand",
    "autoinducer",
    "bind",
    "inducer",
    "cofactor",
    "operon",
];

const MARKS: &[&str] = &[
    "acetylate",
    "methylate",
    "m6a",
    "copies",
    "riboswitch",
    "burst",
    // W64: metadata-only deprecation mark, `@deprecated("text", since="2.4")`.
    // Known here so the wobble repair never mangles it; the payload parse
    // lives in the mark arm, the data rides GeneDef, the runtime never reads it.
    "deprecated",
];

/// Words that end a `use` path, the alias introducer and statement enders.
fn use_path_boundary(w: &str) -> bool {
    w == "as" || w == "from"
}

/// W55 (ROADMAP-100): public read-only accessor for the reserved keyword set.
/// Source of truth stays the `KEYWORDS` table above; generated docs
/// (docs/KEYWORDS.md) mirror it, never the other way around.
pub fn keyword_list() -> &'static [&'static str] {
    KEYWORDS
}

/// W45-v2 (LSP rename): true when `w` cannot be used as a plain identifier.
/// Composition of the reserved-word surfaces, all read-only:
///   - canonical keywords (`KEYWORDS` above),
///   - mark names (`MARKS`, the `@directive` class),
///   - the literal words the grammar reserves (`true`/`false`/`null` and the
///     canonicalized `ifnot`),
///   - the synonym table (`fn`, `var`, `on`, …, a name that repairs into a
///     keyword is not a stable identifier).
///
/// Read-only accessor in the W55 precedent; the tables above stay the single
/// source of truth. Mirrored in the oracle? No, rename is an editor-side
/// tool, no differential surface.
pub fn is_reserved_for_identifier(w: &str) -> bool {
    if KEYWORDS.contains(&w) || MARKS.contains(&w) {
        return true;
    }
    matches!(w, "true" | "false" | "null" | "ifnot") || synonym(w).is_some()
}

pub fn is_canonical(w: &str) -> bool {
    KEYWORDS.contains(&w)
}

fn synonym(w: &str) -> Option<&'static str> {
    Some(match w {
        "fn" | "func" | "fun" | "def" | "funct" | "sub" | "lambda" | "proc" => "gene",
        "var" | "val" => "let",
        "elseif" => "elif",
        "foreach" | "each" => "for",
        "import" | "include" | "require" => "use",
        "ret" => "return",
        "stop" => "break",
        "next" | "skip" => "continue",
        "yes" | "on" => "true",
        "no" | "off" => "false",
        "nil" | "nothing" => "null",
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
    depth: u32,
    /// W074: `##` doc lines (line, text) in source order + consumption cursor.
    docs: Vec<(usize, String)>,
    doc_cursor: usize,
    /// W074: line of the first non-newline token, a doc block entirely
    /// above it and not hugging any decl is the module doc.
    first_tok_line: usize,
    module_doc_assigned: bool,
    module_doc: Vec<String>,
    /// W24: top-level names introduced via the contextual `pub` marker
    /// (`pub gene` / `pub let` / `pub const` / `pub phenotype`). `pub` is
    /// NOT a keyword, an ordinary identifier named `pub` is untouched.
    pub_names: Vec<String>,
}

pub fn parse(src: &str) -> Program {
    let lexed = lex(src);
    let mut notes = lexed.notes;
    let first_tok_line = lexed
        .toks
        .iter()
        .find(|(t, _)| !matches!(t, Tok::Newline | Tok::Semi))
        .map(|(_, l)| *l)
        .unwrap_or(1);
    let mut p = Parser {
        toks: lexed.toks,
        pos: 0,
        notes: Vec::new(),
        depth: 0,
        // W074: doc lines ride alongside the token stream; the parser
        // attaches them to declarations by source order (see take_doc).
        docs: lexed.docs,
        doc_cursor: 0,
        first_tok_line,
        module_doc_assigned: false,
        module_doc: Vec::new(),
        pub_names: Vec::new(),
    };
    let stmts = p.parse_program();
    notes.append(&mut p.notes);
    let mut prog = Program {
        proofs: Vec::new(),
        named_frames: Vec::new(),
        anchor_exports: Vec::new(),
        tad_exports: Vec::new(),
        tad_members: Vec::new(),
        ires: Vec::new(),
        module_doc: p.module_doc.clone(),
        pub_exports: p.pub_names.clone(),
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
                Stmt::Frame {
                    name,
                    is_proof,
                    body,
                } => {
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
                            Stmt::Trait(tr) => members.push(tr.name.clone()),
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
        // clamp: token walk must never index past the trailing Eof
        let i = self.pos.min(self.toks.len() - 1);
        &self.toks[i].0
    }
    fn line(&self) -> usize {
        let i = self.pos.min(self.toks.len() - 1);
        self.toks[i].1
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].0.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn note(&mut self, line: usize, rung: u8, msg: impl Into<String>) {
        // sec-r4 (F-3): parse-time notes were uncapped, a 3 MB file of
        // syntax errors grew 1.5 M notes (646 MB RSS, 258 MB stderr). Same
        // 10k contract as Interp::note; past the cap, notes are suppressed.
        const PARSE_NOTE_CAP: usize = 10_000;
        if self.notes.len() >= PARSE_NOTE_CAP {
            if self.notes.len() == PARSE_NOTE_CAP {
                self.notes.push(Note {
                    line,
                    rung: 4,
                    message: "parse note cap (10000) reached, further notes suppressed".into(),
                });
            }
            return;
        }
        self.notes.push(Note {
            line,
            rung,
            message: msg.into(),
        });
    }
    fn eat_newlines(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.next();
        }
    }

    // ---- W074: doc-comment attachment ---------------------------------
    /// Consume every doc line above `decl_line`, group them into blocks
    /// (a blank-line gap of 2+ source lines starts a new block), and attach
    /// the LAST block, but only if it HUGS the declaration (doc line
    /// immediately above the `gene`/`phenotype`/... keyword line). A top
    /// block separated from everything by a blank line and positioned before
    /// the first real token becomes the module doc. Docs are pure metadata:
    /// dropping one can never change program behavior.
    fn take_doc(&mut self, decl_line: usize) -> Vec<String> {
        let mut consumed: Vec<(usize, String)> = Vec::new();
        while self.doc_cursor < self.docs.len() && self.docs[self.doc_cursor].0 < decl_line {
            consumed.push(self.docs[self.doc_cursor].clone());
            self.doc_cursor += 1;
        }
        if consumed.is_empty() {
            return Vec::new();
        }
        // group into blocks: gap > 1 line (i.e. 1+ blank line) splits.
        let mut blocks: Vec<Vec<String>> = vec![Vec::new()];
        let mut prev_line = consumed[0].0;
        for (l, t) in &consumed {
            if l.saturating_sub(prev_line) > 1 {
                blocks.push(Vec::new());
            }
            // ast-grep-ignore: no-unwrap-in-src
            blocks.last_mut().unwrap().push(t.clone());
            prev_line = *l;
        }
        // ast-grep-ignore: no-unwrap-in-src
        let last = blocks.pop().unwrap();
        let last_line = prev_line; // line of the final consumed doc line
        let hugs = decl_line.saturating_sub(last_line) == 1;
        if !hugs {
            // earlier blocks (and a non-hugging last block) that sit before
            // the first real token line become the module doc (last wins).
            if !self.module_doc_assigned {
                let start_line = consumed[0].0;
                if start_line < self.first_tok_line {
                    self.module_doc = last;
                    self.module_doc_assigned = true;
                }
            }
            return Vec::new();
        }
        // non-attached earlier blocks before the first token: module doc.
        if !self.module_doc_assigned && blocks.len() == 1 && !blocks[0].is_empty() {
            self.module_doc = blocks.remove(0);
            self.module_doc_assigned = true;
        }
        last
    }
    /// Skip tokens to end of line (rung-4 recovery).
    fn skip_line(&mut self) {
        while !matches!(
            self.peek(),
            Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof
        ) {
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
            // W24: contextual `pub` visibility marker, `pub gene` / `pub let`
            // / `pub const` / `pub phenotype` at TOP LEVEL records the name
            // for strict-mode export filtering. `pub` is not a keyword: an
            // identifier named `pub` (or `pub` in any other position) parses
            // exactly as before, so this is invisible unless used as a marker.
            if let Tok::Ident(w) = self.peek().clone() {
                if w == "pub" {
                    let nxt = self.toks.get(self.pos + 1).map(|t| &t.0);
                    if let Some(Tok::Ident(k)) = nxt {
                        if matches!(k.as_str(), "gene" | "let" | "const" | "phenotype") {
                            self.next(); // consume `pub`
                            let s = self.parse_stmt();
                            if let Some(st) = s {
                                let name = match &st {
                                    Stmt::Gene(g) => g.name.clone(),
                                    Stmt::Let(n, _) | Stmt::LetConst(n, _) => Some(n.clone()),
                                    Stmt::LetAnn(n, _, _) => Some(n.clone()),
                                    Stmt::Pheno(pd) => Some(pd.name.clone()),
                                    _ => None,
                                };
                                if let Some(n) = name {
                                    self.pub_names.push(n);
                                }
                                out.push(st);
                            }
                            continue;
                        }
                    }
                }
            }
            if let Some(s) = self.parse_stmt() {
                out.push(s);
            }
            if self.pos == before {
                // no progress: hard fallback
                let line = self.line();
                self.note(
                    line,
                    4,
                    format!("token '{}' skipped", self.peek().describe()),
                );
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
                // W074: docs hug the whole declaration, above the marks.
                let doc = self.take_doc(line);
                self.next();
                let mark = self.repair_mark(m, line)?;
                let mut marks = vec![mark];
                // allow stacked marks, with newlines/semis between them
                // (loop-9: the dx-r6 newline bridge now applies BETWEEN
                // marks too, so `@acetylate\n@riboswitch … gene` stacks)
                loop {
                    self.eat_newlines();
                    if let Tok::Mark(m2) = self.peek().clone() {
                        let l2 = self.line();
                        self.next();
                        if let Some(r) = self.repair_mark(m2, l2) {
                            marks.push(r);
                        }
                    } else {
                        break;
                    }
                }
                // reg-bio-3 (C10): `@copies n` carries its dosage argument.
                let mut copies: u32 = 1;
                if marks.iter().any(|m| m == "copies") {
                    match self.peek().clone() {
                        Tok::Int(i) => {
                            self.next();
                            if !(1..=64).contains(&i) {
                                let line = self.line();
                                self.note(
                                    line,
                                    4,
                                    "gene dosage clamped to 1..=64 copies (a level is a concentration, not an amplifier)",
                                );
                            }
                            copies = i.clamp(1, 64) as u32;
                        }
                        _ => {
                            let line = self.line();
                            self.note(line, 4, "@copies needs an integer 1..=64; default 1");
                        }
                    }
                }
                // loop-9 (F-5): `@riboswitch ligand off|on threshold t`, a
                // cis aptamer on this gene's own transcript. `off` class:
                // bound => terminator hairpin => OFF. `on` class: bound =>
                // RBS exposed => ON. threshold optional (default 0.5).
                let mut riboswitch: Option<(String, bool, f64)> = None;
                if marks.iter().any(|m| m == "riboswitch") {
                    let lig = match self.peek().clone() {
                        Tok::Ident(w) => {
                            self.next();
                            w
                        }
                        _ => String::new(),
                    };
                    let mut on = false;
                    let mut ok = !lig.is_empty();
                    if !ok {
                        let line = self.line();
                        self.note(line, 4, "@riboswitch needs a ligand name; mark ignored");
                    } else {
                        match self.peek().clone() {
                            Tok::Ident(w) if w == "on" => {
                                self.next();
                                on = true;
                            }
                            Tok::Ident(w) if w == "off" => {
                                self.next();
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "@riboswitch needs 'on' or 'off'; mark ignored");
                                ok = false;
                            }
                        }
                    }
                    let mut threshold = 0.5;
                    if ok {
                        if matches!(self.peek().clone(), Tok::Ident(w) if w == "threshold") {
                            self.next();
                            match self.peek().clone() {
                                Tok::Float(f) => {
                                    self.next();
                                    threshold = f.clamp(0.0, 1.0);
                                }
                                Tok::Int(i) => {
                                    self.next();
                                    threshold = (i as f64).clamp(0.0, 1.0);
                                }
                                _ => {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        4,
                                        "riboswitch threshold needs a number 0..=1; default 0.5",
                                    );
                                }
                            }
                        }
                        riboswitch = Some((lig, on, threshold));
                    }
                }
                // loop-9 (F-2): `@burst kon koff`, per-gene promoter
                // identity (two numbers, clamped 0..=1).
                let mut burst: Option<(f64, f64)> = None;
                if marks.iter().any(|m| m == "burst") {
                    let mut vals: [f64; 2] = [0.3, 0.1];
                    let mut got = 0;
                    while got < 2 {
                        match self.peek().clone() {
                            Tok::Float(f) => {
                                self.next();
                                vals[got] = f.clamp(0.0, 1.0);
                                got += 1;
                            }
                            Tok::Int(i) => {
                                self.next();
                                vals[got] = (i as f64).clamp(0.0, 1.0);
                                got += 1;
                            }
                            _ => break,
                        }
                    }
                    if got < 2 {
                        let line = self.line();
                        self.note(
                            line,
                            4,
                            "@burst needs kon and koff (0..=1); defaults 0.3/0.1",
                        );
                    }
                    burst = Some((vals[0], vals[1]));
                }
                // W64: `@deprecated("migration text", since="2.4")` payload.
                // Metadata only: parse it leniently, attach it to the gene,
                // never evaluate it (SPEC §3 marks table). Any malformed
                // shape degrades to a rung-4 note, the mark is dropped, the
                // gene keeps parsing (Total Grammar).
                let mut deprecated: Option<crate::ast::Deprecation> = None;
                if marks.iter().any(|m| m == "deprecated") {
                    match self.peek().clone() {
                        Tok::LParen => {
                            self.next();
                            let msg = match self.peek().clone() {
                                Tok::Str(s) => {
                                    self.next();
                                    s
                                }
                                other => {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        4,
                                        format!(
                                            "@deprecated needs a string message; got {}, mark skipped",
                                            other.describe()
                                        ),
                                    );
                                    String::new()
                                }
                            };
                            let mut since: Option<String> = None;
                            // optional `, since = "x.y"` (the bare comma form
                            // `, "x.y"` is accepted as since, lenient by design)
                            if matches!(self.peek(), Tok::Comma) {
                                self.next();
                                self.eat_newlines_inline();
                                match self.peek().clone() {
                                    Tok::Ident(w) if w == "since" => {
                                        self.next();
                                        self.eat_newlines_inline();
                                        if matches!(self.peek(), Tok::Eq) {
                                            self.next();
                                            self.eat_newlines_inline();
                                        }
                                        if let Tok::Str(s) = self.peek().clone() {
                                            self.next();
                                            since = Some(s);
                                        }
                                    }
                                    Tok::Str(s) => {
                                        self.next();
                                        since = Some(s);
                                    }
                                    _ => {
                                        let line = self.line();
                                        self.note(
                                            line,
                                            4,
                                            "@deprecated since needs a string; ignored",
                                        );
                                    }
                                }
                            }
                            self.eat_newlines_inline();
                            if matches!(self.peek(), Tok::RParen) {
                                self.next();
                            } else {
                                // recover to the closing paren (or the line end)
                                let mut depth = 0usize;
                                loop {
                                    match self.peek().clone() {
                                        Tok::LParen => {
                                            depth += 1;
                                            self.next();
                                        }
                                        Tok::RParen => {
                                            self.next();
                                            if depth == 0 {
                                                break;
                                            }
                                            depth -= 1;
                                        }
                                        Tok::Newline | Tok::Eof => break,
                                        _ => {
                                            self.next();
                                        }
                                    }
                                }
                                let line = self.line();
                                self.note(line, 4, "@deprecated payload auto-closed");
                            }
                            if !msg.is_empty() {
                                deprecated = Some(crate::ast::Deprecation {
                                    message: msg,
                                    since,
                                });
                            }
                        }
                        _ => {
                            let line = self.line();
                            self.note(
                                line,
                                4,
                                "@deprecated needs a migration string; mark skipped",
                            );
                        }
                    }
                }
                // dx-r6 (loop-5-a audit MED): an own-line mark,
                //   @acetylate\ngene foo(),
                // never reached `gene`: the newline between mark and keyword
                // wasn't eaten, the mark was dropped with a rung-4 note, and
                // the same code silently changed regulation semantics vs its
                // same-line spelling. Marks and `gene` may now be separated
                // by newlines/semicolons like any other statement pair.
                self.eat_newlines();
                if !self.expect_kw("gene") {
                    let line = self.line();
                    self.note(line, 4, "mark must precede 'gene'; skipped line");
                    self.skip_line();
                    return None;
                }
                Some(self.parse_gene_def(marks, copies, riboswitch, burst, doc, deprecated))
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
                self.note(
                    line,
                    4,
                    format!(
                        "unexpected token '{}' at statement position",
                        other.describe()
                    ),
                );
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
        // keeps its original spelling, binding positions are never repaired.
        const ARMS: &[&str] = &[
            "gene",
            "let",
            "trait",
            "if",
            "ifnot",
            "elif",
            "else",
            "while",
            "loop",
            "scope",
            "for",
            "return",
            "break",
            "continue",
            "match",
            "use",
            "tad",
            "anchor",
            "enhance",
            "silence",
            "stress",
            "rescue",
            "raise",
            "fate",
            "regulate",
            "toggle",
            "repressilator",
            "frame",
            "splice",
            "edit",
            "ires",
            "phenotype",
            "sequence",
            "yield",
            "decoy",
            "ligand",
            "autoinducer",
            "operon",
        ];
        let mut word = w.to_string();
        // Expression-head detection: `i += 1`, `x = 2`, `f(...)`, `a[0]`,
        // `s.trim()`, a word followed by these cannot be a keyword position,
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
                    self.note(
                        line,
                        2,
                        format!("synonym '{}' repaired to '{}'", word, canon),
                    );
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
                let l = self.line();
                let doc = self.take_doc(l);
                self.next();
                Some(self.parse_gene_def(vec![], 1, None, None, doc, None))
            }
            "trait" => {
                // W04 (SPEC §8b): `trait Name { gene m(); gene n() { ... } }`.
                // A method with a body is a DEFAULT (parsed by the normal
                // gene parser); one without is REQUIRED (semicolon or
                // newline terminates it).
                let l = self.line();
                let doc = self.take_doc(l);
                self.next();
                let name = self.expect_ident()?;
                let mut methods: Vec<TraitMethod> = Vec::new();
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
                                let ln = self.line();
                                self.note(ln, 4, "trait body auto-closed at EOF");
                                break;
                            }
                            Tok::Ident(w) if w == "gene" || synonym(&w) == Some("gene") => {
                                let mline = self.line();
                                self.next();
                                let mname = self.expect_ident().unwrap_or_default();
                                let mut params: Vec<String> = Vec::new();
                                if matches!(self.peek(), Tok::LParen) {
                                    self.next();
                                    loop {
                                        self.eat_newlines_inline();
                                        match self.peek().clone() {
                                            Tok::RParen => {
                                                self.next();
                                                break;
                                            }
                                            Tok::Ident(p) => {
                                                self.next();
                                                params.push(p);
                                                // optional default value (doc-only
                                                // in a signature; ignored with a
                                                // note if present)
                                                if matches!(self.peek(), Tok::Eq) {
                                                    self.next();
                                                    let _ = self.parse_expr();
                                                }
                                                if matches!(self.peek(), Tok::Comma) {
                                                    self.next();
                                                }
                                            }
                                            Tok::Comma => {
                                                self.next();
                                            }
                                            other => {
                                                let ln = self.line();
                                                self.note(
                                                    ln,
                                                    4,
                                                    format!(
                                                        "unexpected {:?} in trait method params; skipped",
                                                        other
                                                    ),
                                                );
                                                self.next();
                                            }
                                        }
                                    }
                                }
                                if matches!(self.peek(), Tok::LBrace) {
                                    // default method: body parsed HERE, the
                                    // head (name + params) was already
                                    // consumed manually above, so parse_gene_def
                                    // would mis-parse from the brace.
                                    let body = self.parse_block().unwrap_or_default();
                                    methods.push(TraitMethod {
                                        name: mname.clone(),
                                        line: mline,
                                        required: false,
                                        default: Some(std::sync::Arc::new(GeneDef {
                                            name: Some(mname),
                                            line: mline,
                                            doc: vec![],
                                            params: params
                                                .iter()
                                                .map(|p| (p.clone(), None))
                                                .collect(),
                                            guard: None,
                                            body,
                                            acetylate: false,
                                            methylate: false,
                                            m6a: false,
                                            copies: 1,
                                            seq: false,
                                            riboswitch: None,
                                            burst: None,
                                            deprecated: None,
                                            param_anns: vec![],
                                            ret_ann: None,
                                            type_params: vec![],
                                        })),
                                    });
                                } else {
                                    // required method: `;` or newline
                                    self.end_stmt();
                                    methods.push(TraitMethod {
                                        name: mname,
                                        line: mline,
                                        required: true,
                                        default: None,
                                    });
                                }
                            }
                            _ => {
                                let ln = self.line();
                                self.note(ln, 4, "unexpected token in trait body; skipped");
                                self.next();
                            }
                        }
                    }
                } else {
                    let ln = self.line();
                    self.note(ln, 4, "trait without body declares no methods");
                }
                Some(Stmt::Trait(std::sync::Arc::new(TraitDef {
                    name,
                    line: l,
                    doc,
                    methods,
                })))
            }
            "const" => {
                // W05: `const NAME = expr`, immutable binding with deep-freeze
                // semantics (SPEC §7d). Destructuring degrades to a plain let
                // (Total Grammar: degrade, never reject); a type annotation is
                // parsed and dropped with a note (v1 scope).
                self.next();
                if matches!(self.peek(), Tok::LBrack | Tok::LBrace) {
                    let line = self.line();
                    let pat = self.parse_destructure_pat();
                    self.note(
                        line,
                        4,
                        "const destructuring not supported; bound as mutable let",
                    );
                    if matches!(self.peek(), Tok::Eq) {
                        self.next();
                        let e = self.parse_expr();
                        self.end_stmt();
                        return Some(Stmt::LetPat(pat, e));
                    }
                    self.note(line, 4, "destructured 'let' without value binds nulls");
                    self.end_stmt();
                    return Some(Stmt::LetPat(pat, Expr::Null));
                }
                let name = self.expect_ident()?;
                if matches!(self.peek(), Tok::Colon) {
                    let line = self.line();
                    self.next();
                    let _ = self.parse_type_ann();
                    self.note(
                        line,
                        4,
                        "const type annotation is not checked in v1; dropped",
                    );
                }
                if matches!(self.peek(), Tok::Eq) {
                    self.next();
                    let e = self.parse_expr();
                    self.end_stmt();
                    return Some(Stmt::LetConst(name, e));
                }
                let line = self.line();
                self.note(line, 4, "const without '=' binds null");
                self.end_stmt();
                Some(Stmt::LetConst(name, Expr::Null))
            }
            "let" => {
                self.next();
                // W05: contextual `mut` annotation, `let mut x = ...` is
                // documentation-only in v2.x; consumed silently when `mut` is
                // followed by the real name (so a variable literally named
                // `mut` keeps working: `let mut = 5`).
                if matches!(self.peek(), Tok::Ident(w) if w == "mut") {
                    if let Some(Tok::Ident(_)) = self.toks.get(self.pos + 1).map(|t| &t.0) {
                        self.next();
                    }
                }
                // L1a: destructuring definitions, `let [a, b] = e`,
                // `let {x, y} = e` (patterns nest; `*rest` captures the tail).
                if matches!(self.peek(), Tok::LBrack | Tok::LBrace) {
                    let pat = self.parse_destructure_pat();
                    if matches!(self.peek(), Tok::Eq) {
                        self.next();
                        let e = self.parse_expr();
                        self.end_stmt();
                        return Some(Stmt::LetPat(pat, e));
                    }
                    let line = self.line();
                    self.note(line, 4, "destructured 'let' without value binds nulls");
                    self.end_stmt();
                    return Some(Stmt::LetPat(pat, Expr::Null));
                }
                let name = self.expect_ident()?;
                // W01 (L2c): soft type annotation, `let n: int = 3`
                if matches!(self.peek(), Tok::Colon) {
                    self.next();
                    let ann = self.parse_type_ann();
                    if matches!(self.peek(), Tok::Eq) {
                        self.next();
                        let e = self.parse_expr();
                        self.end_stmt();
                        return Some(Stmt::LetAnn(name, ann, e));
                    }
                    let line = self.line();
                    self.note(
                        line,
                        4,
                        format!("'let {name}: {}' without value binds null", ann.render()),
                    );
                    self.end_stmt();
                    return Some(Stmt::LetAnn(name, ann, Expr::Null));
                }
                // L1a: `let a, b = 1, 2`, multi-define (all values evaluated
                // before any name binds).
                if matches!(self.peek(), Tok::Comma) {
                    let mut names = vec![name];
                    while matches!(self.peek(), Tok::Comma) {
                        self.next();
                        names.push(self.expect_ident()?);
                    }
                    if matches!(self.peek(), Tok::Eq) {
                        self.next();
                        let mut values = vec![self.parse_expr()];
                        while matches!(self.peek(), Tok::Comma) {
                            self.next();
                            values.push(self.parse_expr());
                        }
                        self.end_stmt();
                        let targets: Vec<Expr> = names.into_iter().map(Expr::Ident).collect();
                        return Some(Stmt::MultiAssign(targets, values, true));
                    }
                    let line = self.line();
                    self.note(line, 4, "multi 'let' without value binds nulls");
                    self.end_stmt();
                    let targets: Vec<Expr> = names.into_iter().map(Expr::Ident).collect();
                    let values: Vec<Expr> = targets.iter().map(|_| Expr::Null).collect();
                    return Some(Stmt::MultiAssign(targets, values, true));
                }
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
            // W17: structured-concurrency block (see SPEC §13)
            "scope" => {
                self.next();
                let body = self.parse_block().unwrap_or_default();
                Some(Stmt::Scope(body))
            }
            "for" => {
                self.next();
                // L1a: destructuring loop target, `for [k, v] in pairs { }`.
                if matches!(self.peek(), Tok::LBrack | Tok::LBrace) {
                    let pat = self.parse_destructure_pat();
                    if !self.expect_kw("in") {
                        let line = self.line();
                        self.note(line, 4, "'for <pattern>' missing 'in'; iterating null");
                    }
                    let iter = self.parse_expr();
                    let body = self.parse_block().unwrap_or_default();
                    return Some(Stmt::ForPat(pat, iter, body));
                }
                let name = self.expect_ident()?;
                if !self.expect_kw("in") {
                    let line = self.line();
                    self.note(
                        line,
                        4,
                        format!("'for {}' missing 'in'; iterating null", name),
                    );
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
                let match_line = self.line();
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
                Some(Stmt::Match(subject, cases, match_line))
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
            "module" => {
                // W025 stage 2: nested sub-module declaration. CONTEXTUAL:
                // only `module NAME {` (newlines tolerated between the words)
                // is the declaration; every other use of the word `module`
                // stays an ordinary identifier (it is not in KEYWORDS, so
                // pre-existing programs that bind `module` never change).
                let is_decl = {
                    let mut j = self.pos + 1;
                    while matches!(
                        self.toks.get(j).map(|t| &t.0),
                        Some(Tok::Newline) | Some(Tok::Semi)
                    ) {
                        j += 1;
                    }
                    if !matches!(self.toks.get(j).map(|t| &t.0), Some(Tok::Ident(_))) {
                        false
                    } else {
                        j += 1;
                        while matches!(
                            self.toks.get(j).map(|t| &t.0),
                            Some(Tok::Newline) | Some(Tok::Semi)
                        ) {
                            j += 1;
                        }
                        matches!(self.toks.get(j).map(|t| &t.0), Some(Tok::LBrace))
                    }
                };
                if !is_decl {
                    return self.parse_assign_or_expr(w);
                }
                self.next();
                let name = self.expect_ident()?;
                self.eat_newlines();
                let body = self.parse_block().unwrap_or_default();
                Some(Stmt::Module(name, body))
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
                // reg-bio-3 (C9): stoichiometric RISC, `strength s` is the
                // per-site capture probability; `sites n` composes
                // multiplicatively. Omitted = legacy binary silence.
                let mut strength = 1.0;
                if matches!(self.peek().clone(), Tok::Ident(w) if w == "strength") {
                    self.next();
                    match self.peek().clone() {
                        Tok::Float(f) => {
                            self.next();
                            strength = f.clamp(0.0, 1.0);
                        }
                        Tok::Int(i) => {
                            self.next();
                            strength = (i as f64).clamp(0.0, 1.0);
                        }
                        _ => {
                            let line = self.line();
                            self.note(line, 4, "strength needs a number 0..=1; ignored");
                        }
                    }
                }
                let mut sites: u32 = 1;
                if matches!(self.peek().clone(), Tok::Ident(w) if w == "sites") {
                    self.next();
                    match self.peek().clone() {
                        Tok::Int(i) => {
                            self.next();
                            if !(1..=64).contains(&i) {
                                let line = self.line();
                                self.note(line, 4, "sites clamped to 1..=64");
                            }
                            sites = i.clamp(1, 64) as u32;
                        }
                        _ => {
                            let line = self.line();
                            self.note(line, 4, "sites needs an integer 1..=64; ignored");
                        }
                    }
                }
                self.end_stmt();
                Some(Stmt::Silence(from, to, strength, sites))
            }
            "stress" => {
                self.next();
                let kind = if let Tok::Ident(w) = self.peek().clone() {
                    if !is_canonical(&w)
                        || w == "missing"
                        || w == "unfolded"
                        || w == "overflow"
                        || w == "burned"
                        || w == "any"
                    {
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
                let raise_line = self.line(); // W007: the raise statement's own line
                                              // raise kind, msg   |   raise msg
                let save = self.pos;
                if let Tok::Ident(k) = self.peek().clone() {
                    if matches!(k.as_str(), "unfolded" | "missing" | "overflow" | "burned") {
                        self.next();
                        if matches!(self.peek(), Tok::Comma) {
                            self.next();
                            let msg = self.parse_expr();
                            self.end_stmt();
                            return Some(Stmt::Raise(Some(k), msg, raise_line));
                        }
                        self.pos = save;
                    }
                }
                let msg = self.parse_expr();
                self.end_stmt();
                Some(Stmt::Raise(None, msg, raise_line))
            }
            "fate" => {
                self.next();
                let fate_line = self.line();
                let fate_doc = self.take_doc(fate_line);
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
                            Tok::Ident(w)
                                if w == "state"
                                    || synonym(&w) == Some("state")
                                    || crate::ffi::edit_distance(&w, "state") <= 1 =>
                            {
                                if w != "state" {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        3,
                                        format!("wobble: '{}' repaired to 'state'", w),
                                    );
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
                            Tok::Ident(w)
                                if w == "enter" || crate::ffi::edit_distance(&w, "enter") <= 1 =>
                            {
                                if w != "enter" {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        3,
                                        format!("wobble: '{}' repaired to 'enter'", w),
                                    );
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
                Some(Stmt::Fate(std::sync::Arc::new(FateDef {
                    name: fname,
                    line: fate_line,
                    doc: fate_doc,
                    states,
                    enter,
                })))
            }
            "regulate" => {
                self.next();
                let mut edges = Vec::new();
                let mut trans = Vec::new();
                let mut binds = Vec::new();
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
                                // reg-bio-2 (C1): `a translates b rate r decay d;`
                                //, the translation layer. Not a cis-gate: a
                                // production relation (mRNA -> protein).
                                if self.expect_kw("translates") {
                                    let to = self.expect_ident().unwrap_or_default();
                                    let mut rate = None;
                                    if self.expect_kw("rate") {
                                        match self.peek().clone() {
                                            Tok::Float(f) => {
                                                self.next();
                                                rate = Some(f);
                                            }
                                            Tok::Int(i) => {
                                                self.next();
                                                rate = Some(i as f64);
                                            }
                                            _ => {
                                                let line = self.line();
                                                self.note(
                                                    line,
                                                    4,
                                                    "rate needs a number; using 1.0",
                                                );
                                            }
                                        }
                                    }
                                    let mut pdecay = None;
                                    if self.expect_kw("decay") {
                                        match self.peek().clone() {
                                            Tok::Float(f) => {
                                                self.next();
                                                pdecay = Some(f.clamp(0.0, 1.0));
                                            }
                                            Tok::Int(i) => {
                                                self.next();
                                                pdecay = Some((i as f64).clamp(0.0, 1.0));
                                            }
                                            _ => {
                                                let line = self.line();
                                                self.note(line, 4, "decay needs a number; ignored");
                                            }
                                        }
                                    }
                                    trans.push(crate::ast::TransEdge {
                                        from,
                                        to,
                                        rate,
                                        decay: pdecay,
                                    });
                                    self.end_stmt();
                                    continue;
                                }
                                // reg-bio-2 (A4): `bind tf inducer lg k v;` /
                                // `bind tf cofactor lg k v;`, allostery. The
                                // binding modulates the TF's DNA-available
                                // fraction at every regulation read. `bind` is
                                // a HEAD keyword here (no edge source): the
                                // head ident was consumed as `from` above.
                                if from == "bind" {
                                    let tf = self.expect_ident().unwrap_or_default();
                                    let inducer = if self.expect_kw("inducer") {
                                        true
                                    } else if self.expect_kw("cofactor") {
                                        false
                                    } else {
                                        let line = self.line();
                                        self.note(
                                            line,
                                            4,
                                            "bind needs 'inducer' or 'cofactor'; binding dropped",
                                        );
                                        self.skip_line();
                                        continue;
                                    };
                                    let lg = self.expect_ident().unwrap_or_default();
                                    let mut k = 0.1;
                                    if self.expect_kw("k") {
                                        match self.peek().clone() {
                                            Tok::Float(f) => {
                                                self.next();
                                                k = f;
                                            }
                                            Tok::Int(i) => {
                                                self.next();
                                                k = i as f64;
                                            }
                                            _ => {
                                                let line = self.line();
                                                self.note(line, 4, "k needs a number; using 0.1");
                                            }
                                        }
                                        if k <= 0.0 {
                                            let line = self.line();
                                            self.note(line, 4, "k must be > 0; using 0.1");
                                            k = 0.1;
                                        }
                                    }
                                    binds.push(crate::ast::BindDef {
                                        tf,
                                        ligand: lg,
                                        inducer,
                                        k,
                                    });
                                    self.end_stmt();
                                    continue;
                                }
                                // reg-bio-2 (A5): per-edge attenuator flag,
                                // taken by the next edge build (std::mem::take).
                                let mut attenuating = false;
                                let inhibit = if self.expect_kw("activates") {
                                    false
                                } else if self.expect_kw("inhibits") {
                                    true
                                } else if self.expect_kw("attenuates") {
                                    // reg-bio-2 (A5/C7): RNA-level attenuation,
                                    // veto mechanics of an inhibitor, RNA-level
                                    // report, fire-phase inhibition.
                                    attenuating = true;
                                    true
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "regulate edge missing 'activates'/'inhibits'/'translates'; edge dropped");
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
                                            self.note(
                                                line,
                                                4,
                                                "strength needs a number; using 1.0",
                                            );
                                        }
                                    }
                                    // reg-bio-2 (D2c): a strength is a binding
                                    // weight, not an amplifier, clamped to the
                                    // physical range and the propagation write
                                    // clamps influence at 1.0. Legacy programs
                                    // (0..=1) are untouched; out-of-range gets
                                    // a note, not an error (Total Grammar).
                                    if strength > 1.0 {
                                        let line = self.line();
                                        self.note(
                                            line,
                                            4,
                                            "strength > 1.0 clamped to 1.0 (levels are concentration fractions)",
                                        );
                                        strength = 1.0;
                                    } else if strength < 0.0 {
                                        let line = self.line();
                                        self.note(
                                            line,
                                            4,
                                            "negative strength clamped to 0.0 (a negative repressor is not a booster)",
                                        );
                                        strength = 0.0;
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
                                // reg-bio (F-2): optional per-edge Hill exponent.
                                // Canonical edge order: strength -> threshold -> hill -> any.
                                let mut hill = None;
                                if self.expect_kw("hill") {
                                    match self.peek().clone() {
                                        Tok::Int(i) => {
                                            self.next();
                                            if (1..=8).contains(&i) {
                                                hill = Some(i as u32);
                                            } else {
                                                let line = self.line();
                                                self.note(
                                                    line,
                                                    4,
                                                    "hill needs an integer 1..=8; edge keeps the default n=2 shape",
                                                );
                                            }
                                        }
                                        _ => {
                                            let line = self.line();
                                            self.note(
                                                line,
                                                4,
                                                "hill needs an integer 1..=8; edge keeps the default n=2 shape",
                                            );
                                        }
                                    }
                                }
                                // reg-bio (F-3): optional cis-regulatory OR membership.
                                let mut any_edge = false;
                                if self.expect_kw("any") {
                                    any_edge = true;
                                }
                                // reg-bio-2 (D2b): optional occupancy repression,
                                // the multiplicative Kⁿ/(Kⁿ+Rⁿ) survival form.
                                // Canonical edge order:
                                //   strength -> threshold -> hill -> any -> occupy -> sum
                                let mut occupy_edge = false;
                                if self.expect_kw("occupy") {
                                    occupy_edge = true;
                                }
                                // reg-bio-2 (B7): optional synergistic pooling.
                                let mut sum_edge = false;
                                if self.expect_kw("sum") {
                                    sum_edge = true;
                                }
                                // hill/any shape the dose-response gate; without a
                                // threshold the edge is declarative, so both are noise.
                                if (hill.is_some() || any_edge) && threshold.is_none() {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        4,
                                        "hill/any apply to thresholded edges; ignored (edge stays declarative)",
                                    );
                                    hill = None;
                                    any_edge = false;
                                }
                                // `any` on an inhibitor is noise: inhibitors already
                                // veto with OR semantics (any above-threshold veto fires).
                                if any_edge && inhibit {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        4,
                                        "any on an inhibiting edge is ignored (inhibitors already veto independently)",
                                    );
                                    any_edge = false;
                                }
                                // `occupy` reshapes INHIBITION (multiplicative
                                // survival); on an activator it is noise.
                                if occupy_edge && !inhibit {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        4,
                                        "occupy applies to inhibiting edges; ignored (activators cannot occupy a promoter they activate)",
                                    );
                                    occupy_edge = false;
                                }
                                // `sum` pools ACTIVATING inputs; an inhibitor already
                                // composes multiplicatively with `occupy`.
                                if sum_edge && (inhibit || threshold.is_none()) {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        4,
                                        "sum applies to thresholded activating edges; ignored",
                                    );
                                    sum_edge = false;
                                }
                                edges.push(RegEdge {
                                    from,
                                    to,
                                    strength,
                                    inhibit,
                                    threshold,
                                    hill,
                                    any: any_edge,
                                    occupy: occupy_edge,
                                    sum: sum_edge,
                                    attenuates: std::mem::take(&mut attenuating),
                                });
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
                Some(Stmt::Regulate(edges, trans, binds))
            }
            // reg-bio-2 (A4): `ligand iptg;`, a small-molecule pool. The
            // pool's level is set with ligand_set(name, v) or the `.cell
            // [ligand.<name>]` bath config, and a ligand named as an edge
            // source gates calls directly (riboswitch-style, protein-free).
            "ligand" => {
                self.next();
                let name = self.expect_ident().unwrap_or_default();
                self.end_stmt();
                Some(Stmt::Ligand(name))
            }
            // loop-9 (C8): `autoinducer ahl;`, register a quorum-sensing
            // signal species into the process-global shared medium.
            // Idempotent; secrete() auto-registers too (ligand_set precedent).
            "autoinducer" => {
                self.next();
                let name = self.expect_ident().unwrap_or_default();
                self.end_stmt();
                Some(Stmt::Autoinducer(name))
            }
            // reg-bio-2 (C11): `decoy d for tf capacity 0.5;`, a decoy
            // binding site that titrates its regulator (competitive
            // sequestration: free TF = total − capacity × decoy level).
            "decoy" => {
                self.next();
                let d = self.expect_ident().unwrap_or_default();
                let mut tf = String::new();
                let mut cap = 0.0;
                if self.expect_kw("for") {
                    tf = self.expect_ident().unwrap_or_default();
                    if matches!(self.peek().clone(), Tok::Ident(w) if w == "capacity") {
                        self.next();
                        match self.peek().clone() {
                            Tok::Float(f) => {
                                self.next();
                                cap = f.clamp(0.0, 1.0);
                            }
                            Tok::Int(i) => {
                                self.next();
                                cap = (i as f64).clamp(0.0, 1.0);
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "capacity needs a number 0..=1; ignored");
                                cap = 0.0;
                            }
                        }
                    } else {
                        let line = self.line();
                        self.note(line, 4, "decoy needs 'capacity <num>'; declared inert");
                    }
                } else {
                    let line = self.line();
                    self.note(line, 4, "decoy needs 'for <tf> capacity <num>'; skipped");
                }
                self.end_stmt();
                Some(Stmt::Decoy(d, tf, cap))
            }
            // reg-bio-3 (A1/A7): the polycistronic transcription unit,
            // `operon lac { lacZ rbs 1.0; lacY rbs 0.6; lacA; }`. ONE
            // promoter drives N cistrons on ONE transcript; member ORDER is
            // load-bearing (RBS gradient + polarity exposure). `rbs` is the
            // per-cistron translation efficiency (Shine-Dalgarno strength,
            // clamped 0..=1, default 1.0), distinct from edge `strength`,
            // which is a binding weight.
            "operon" => {
                self.next();
                let name = self.expect_ident().unwrap_or_default();
                let mut members: Vec<(String, f64)> = Vec::new();
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
                                self.note(line, 4, "operon block auto-closed");
                                break;
                            }
                            Tok::Ident(g) => {
                                self.next();
                                let mut rbs = 1.0;
                                if matches!(self.peek().clone(), Tok::Ident(w) if w == "rbs") {
                                    self.next();
                                    match self.peek().clone() {
                                        Tok::Float(f) => {
                                            self.next();
                                            rbs = f.clamp(0.0, 1.0);
                                        }
                                        Tok::Int(i) => {
                                            self.next();
                                            rbs = (i as f64).clamp(0.0, 1.0);
                                        }
                                        _ => {
                                            let line = self.line();
                                            self.note(
                                                line,
                                                4,
                                                "rbs needs a number 0..=1; using 1.0",
                                            );
                                        }
                                    }
                                }
                                members.push((g, rbs));
                                self.end_stmt();
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "unexpected token in operon block; skipped");
                                self.next();
                            }
                        }
                    }
                } else {
                    let line = self.line();
                    self.note(
                        line,
                        4,
                        "operon needs a block '{ cistron rbs r; ... }'; skipped",
                    );
                }
                Some(Stmt::Operon(name, members))
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
                // reg-bio (F-5): inline kinetics, plasmid engineering in
                // source. Canonical order: alpha, gamma, hill, basal, noise,
                // seed; each keyword optional, each layers onto the current
                // params (last declaration wins per-field).
                let mut ov = crate::ast::RepressiOverrides::default();
                loop {
                    if self.expect_kw("alpha") {
                        match self.peek().clone() {
                            Tok::Float(f) => {
                                self.next();
                                if f > 0.0 {
                                    ov.alpha = Some(f);
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "alpha needs a number > 0; ignored");
                                }
                            }
                            Tok::Int(i) => {
                                self.next();
                                if i > 0 {
                                    ov.alpha = Some(i as f64);
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "alpha needs a number > 0; ignored");
                                }
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "alpha needs a number > 0; ignored");
                            }
                        }
                    } else if self.expect_kw("gamma") {
                        match self.peek().clone() {
                            Tok::Float(f) => {
                                self.next();
                                if f >= 0.0 {
                                    ov.gamma = Some(f);
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "gamma needs a number >= 0; ignored");
                                }
                            }
                            Tok::Int(i) => {
                                self.next();
                                ov.gamma = Some(i as f64);
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "gamma needs a number >= 0; ignored");
                            }
                        }
                    } else if self.expect_kw("hill") {
                        if let Tok::Int(i) = self.peek().clone() {
                            self.next();
                            if (1..=8).contains(&i) {
                                ov.hill = Some(i as u32);
                            } else {
                                let line = self.line();
                                self.note(line, 4, "hill needs an integer 1..=8; ignored");
                            }
                        } else {
                            let line = self.line();
                            self.note(line, 4, "hill needs an integer 1..=8; ignored");
                        }
                    } else if self.expect_kw("basal") {
                        match self.peek().clone() {
                            Tok::Float(f) => {
                                self.next();
                                if f >= 0.0 {
                                    ov.basal = Some(f);
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "basal needs a number >= 0; ignored");
                                }
                            }
                            Tok::Int(i) => {
                                self.next();
                                ov.basal = Some(i as f64);
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "basal needs a number >= 0; ignored");
                            }
                        }
                    } else if self.expect_kw("noise") {
                        match self.peek().clone() {
                            Tok::Float(f) => {
                                self.next();
                                if (0.0..=1.0).contains(&f) {
                                    ov.noise = Some(f);
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "noise needs a number in 0..=1; ignored");
                                }
                            }
                            Tok::Int(i) => {
                                self.next();
                                let f = i as f64;
                                if f <= 1.0 {
                                    ov.noise = Some(f);
                                } else {
                                    let line = self.line();
                                    self.note(line, 4, "noise needs a number in 0..=1; ignored");
                                }
                            }
                            _ => {
                                let line = self.line();
                                self.note(line, 4, "noise needs a number in 0..=1; ignored");
                            }
                        }
                    } else if self.expect_kw("seed") {
                        if let Tok::Int(i) = self.peek().clone() {
                            self.next();
                            ov.seed = Some(if i == 0 { 0x9E3779B97F4A7C15 } else { i as u64 });
                        } else {
                            let line = self.line();
                            self.note(line, 4, "seed needs an integer; ignored");
                        }
                    } else {
                        break;
                    }
                }
                self.end_stmt();
                Some(Stmt::Repressilator(ring, period, ov))
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
                Some(Stmt::Frame {
                    name,
                    is_proof,
                    body,
                })
            }
            "splice" => {
                self.next();
                let splice_line = self.line();
                // W074: capture at arm start, variant genes' take_doc calls
                // must not steal the splice's own doc block.
                let splice_doc = self.take_doc(splice_line);
                let root = self.expect_ident()?;
                let mut variants = Vec::new();
                // T2c: marks collected before a 'variant' keyword apply to
                // that variant (same grammar as gene definitions).
                let mut pending_marks: Vec<String> = Vec::new();
                if matches!(self.peek(), Tok::LBrace) {
                    self.next();
                    loop {
                        self.eat_newlines();
                        match self.peek().clone() {
                            Tok::RBrace => {
                                self.next();
                                if !pending_marks.is_empty() {
                                    self.note(
                                        self.line(),
                                        4,
                                        "mark must precede 'variant' in splice block; skipped",
                                    );
                                }
                                break;
                            }
                            Tok::Eof => {
                                let line = self.line();
                                self.note(line, 4, "splice block auto-closed");
                                break;
                            }
                            Tok::Mark(m) => {
                                let line = self.line();
                                self.next();
                                if let Some(r) = self.repair_mark(m, line) {
                                    pending_marks.push(r);
                                }
                            }
                            Tok::Ident(w)
                                if w == "variant"
                                    || synonym(&w) == Some("variant")
                                    || crate::ffi::edit_distance(&w, "variant") <= 1 =>
                            {
                                if w != "variant" {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        3,
                                        format!("wobble: '{}' repaired to 'variant'", w),
                                    );
                                }
                                self.next();
                                let vname = self.expect_ident().unwrap_or_else(|| "v".into());
                                // T2c: drain pending marks into the variant's
                                // GeneDef, @m6a gives the variant selection
                                // priority (choose_variant step 3); @acetylate
                                // and @methylate ride on the resolved binding.
                                let vmarks = std::mem::take(&mut pending_marks);
                                let v_ac = vmarks.iter().any(|m| m == "acetylate");
                                let v_me = vmarks.iter().any(|m| m == "methylate");
                                let v_m6 = vmarks.iter().any(|m| m == "m6a");
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
                                            self.note(
                                                self.line(),
                                                4,
                                                "unclosed variant parameter list; auto-closed",
                                            );
                                            break;
                                        }
                                    }
                                }
                                let body = self.parse_block().unwrap_or_default();
                                variants.push((
                                    vname,
                                    std::sync::Arc::new(GeneDef {
                                        name: Some(root.clone()),
                                        line: self.line(),
                                        params,
                                        guard: None,
                                        body,
                                        acetylate: v_ac,
                                        methylate: v_me,
                                        m6a: v_m6,
                                        copies: 1,
                                        seq: false,
                                        riboswitch: None,
                                        burst: None,
                                        ..GeneDef::default()
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
                    self.note(
                        line,
                        4,
                        format!("splice '{}' has no variants; binds null", root),
                    );
                }
                Some(Stmt::Splice(std::sync::Arc::new(SpliceDef {
                    root,
                    line: splice_line,
                    doc: splice_doc,
                    variants,
                })))
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
                                            other.describe()
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
                                        other => other.describe(),
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
                let pheno_line = self.line();
                // W074: capture at arm start, inner method genes' take_doc
                // calls must not steal the phenotype's own doc block.
                let pheno_doc = self.take_doc(pheno_line);
                let name = self.expect_ident()?;
                let mut parent = None;
                if self.at_kw("from") {
                    self.next();
                    parent = Some(self.expect_ident().unwrap_or_default());
                }
                // W04: optional `implements A, B, C`, the trait contract.
                // Contextual word (not a keyword): an ident named implements
                // elsewhere is untouched.
                let mut implements = Vec::new();
                if self.at_kw("implements") {
                    self.next();
                    loop {
                        if let Some(t) = self.expect_ident() {
                            implements.push(t);
                        }
                        if matches!(self.peek(), Tok::Comma) {
                            self.next();
                            continue;
                        }
                        break;
                    }
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
                                let doc = self.take_doc(line);
                                self.next();
                                if let Some(mark) = self.repair_mark(m, line) {
                                    let marks = vec![mark];
                                    if self.expect_kw("gene") {
                                        if let Some(Stmt::Gene(g)) = Some(
                                            self.parse_gene_def(marks, 1, None, None, doc, None),
                                        ) {
                                            methods.push(g);
                                        }
                                    } else {
                                        self.note(
                                            line,
                                            4,
                                            "mark inside phenotype must precede 'gene'; skipped",
                                        );
                                        self.skip_line();
                                    }
                                }
                            }
                            Tok::Ident(w) if w == "gene" || synonym(&w) == Some("gene") => {
                                let mline = self.line();
                                let doc = self.take_doc(mline);
                                if w != "gene" {
                                    self.note(
                                        mline,
                                        2,
                                        format!("synonym '{}' repaired to 'gene'", w),
                                    );
                                }
                                self.next();
                                if let Some(Stmt::Gene(g)) =
                                    Some(self.parse_gene_def(vec![], 1, None, None, doc, None))
                                {
                                    methods.push(g);
                                }
                            }
                            Tok::Ident(w) if w == "let" || synonym(&w) == Some("let") => {
                                if w != "let" {
                                    let line = self.line();
                                    self.note(
                                        line,
                                        2,
                                        format!("synonym '{}' repaired to 'let'", w),
                                    );
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
                Some(Stmt::Pheno(std::sync::Arc::new(PhenoDef {
                    name: pname,
                    line: pheno_line,
                    doc: pheno_doc,
                    parent,
                    implements,
                    fields,
                    methods,
                })))
            }
            "sequence" => {
                let l = self.line();
                let doc = self.take_doc(l);
                self.next();
                let def = self.parse_gene_def(vec![], 1, None, None, doc, None);
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
                // are gene definitions, the C-like entry/route idiom.
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
                                        while j < n && matches!(toks[j].0, Tok::Newline | Tok::Semi)
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
                    let gene_line = self.line();
                    let line = self.line();
                    // dx-r3 (re-audit): bare `main { }` is the community's
                    // most common top-level form and the formatter's own
                    // output style, it is canonical sugar (rung 1), not a
                    // repair. Other bare-name blocks stay rung 4.
                    let (rung, msg) = if word == "main" {
                        (
                            1,
                            "bare 'main' block accepted as the entry gene (canonical form: gene main())".to_string(),
                        )
                    } else {
                        (
                            4,
                            format!("bare name block '{word}' treated as gene definition"),
                        )
                    };
                    self.note(line, rung, msg);
                    // W074: bare-name blocks can carry docs too (docs hug
                    // the name line).
                    let doc = self.take_doc(line);
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
                        line: gene_line,
                        doc,
                        params,
                        guard: None,
                        body,
                        acetylate: false,
                        methylate: false,
                        m6a: false,
                        copies: 1,
                        seq: false,
                        riboswitch: None,
                        burst: None,
                        ..GeneDef::default()
                    })));
                }
                self.parse_assign_or_expr(w)
            }
        }
    }

    fn parse_use_path(&mut self) -> String {
        // path := segment (('/' | '.' | '-' | '::') segment)* ; segments are
        // plain identifiers. A boundary word ('as', statement keyword, etc.)
        // ends the path, it is never glued into it. W25: `::` is the
        // Rust-style separator, sugar for '/' (`use bio::sequence` =
        // `use bio/sequence`) so namespaces read like mainstream module paths.
        // W025 stage 2: a trailing `*` after a separator is the wildcard
        // form (`use mymod::seq::*`), carried as a literal `/*` suffix the
        // interpreter strips before resolution.
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
                    if !matches!(self.peek(), Tok::Slash | Tok::Dot | Tok::Minus | Tok::Colon) {
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
                Tok::Colon => {
                    // `::` only; a lone ':' ends the path (never consumed)
                    let nxt = self.toks.get(self.pos + 1).map(|t| &t.0);
                    if !matches!(nxt, Some(Tok::Colon)) {
                        break;
                    }
                    cur.push('/');
                    self.next();
                    self.next();
                }
                Tok::Str(s) => {
                    cur.push_str(&s);
                    self.next();
                    if !matches!(self.peek(), Tok::Slash | Tok::Dot | Tok::Minus | Tok::Colon) {
                        break;
                    }
                }
                Tok::Star => {
                    // W025 stage 2: wildcard tail, `use a::b::*` carries as
                    // "a/b/*"; the interpreter strips the `/*` and flat-binds
                    // the target table's exports.
                    cur.push('*');
                    self.next();
                    break;
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
                self.note(
                    line,
                    4,
                    format!("expected a name, found '{}'; used '?'", other.describe()),
                );
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
                            Expr::Index(t, i, _) => Some(Stmt::IndexAssign(*t, *i, None, val)),
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
                    Tok::Comma if Self::is_assign_target(&e) => {
                        // L1a: multiple assignment / swap, `a, b = b, a`,
                        // `m.k, l[0] = x, y`. Parse the rest of the target
                        // list, require '=', then the RHS list (RHS is fully
                        // evaluated before any target is assigned). If no '='
                        // follows, rewind and fall back to the old behavior.
                        let save = self.pos;
                        let mut targets = vec![e.clone()];
                        let mut valid = true;
                        while matches!(self.peek(), Tok::Comma) {
                            self.next();
                            let t = self.parse_expr();
                            if !Self::is_assign_target(&t) {
                                valid = false;
                            }
                            targets.push(t);
                        }
                        if valid && matches!(self.peek(), Tok::Eq) {
                            self.next();
                            let mut values = vec![self.parse_expr()];
                            while matches!(self.peek(), Tok::Comma) {
                                self.next();
                                values.push(self.parse_expr());
                            }
                            self.end_stmt();
                            return Some(Stmt::MultiAssign(targets, values, false));
                        }
                        self.pos = save;
                        let line = self.line();
                        self.note(
                            line,
                            4,
                            "expression statement not terminated; rest of line skipped",
                        );
                        self.skip_line();
                        self.end_stmt();
                        Some(Stmt::ExprStmt(e))
                    }
                    Tok::PlusEq
                    | Tok::MinusEq
                    | Tok::StarEq
                    | Tok::SlashEq
                    | Tok::DSlashEq
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
                            Expr::Index(t, i, _) => Some(Stmt::IndexAssign(*t, *i, Some(op), val)),
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
                            self.note(
                                line,
                                4,
                                "expression statement not terminated; rest of line skipped",
                            );
                            self.skip_line();
                        }
                        self.end_stmt();
                        Some(Stmt::ExprStmt(e))
                    }
                }
            }
        }
    }

    /// L1a: can this expression stand on the left of `=` in a multi-assign?
    /// Names, index expressions and member expressions only.
    fn is_assign_target(e: &Expr) -> bool {
        matches!(e, Expr::Ident(_) | Expr::Index(..) | Expr::Member(..))
    }

    /// W01 (L2c): type-annotation grammar, `name` (a type name), `name?`
    /// (optional), `a | b | ...` (union). Total Grammar: a malformed
    /// annotation degrades to `any` with a note, never a rejection.
    fn parse_type_ann(&mut self) -> TypeAnn {
        let first = self.parse_type_ann_atom();
        if matches!(self.peek(), Tok::Pipe) {
            let mut alts = vec![first];
            while matches!(self.peek(), Tok::Pipe) {
                self.next();
                alts.push(self.parse_type_ann_atom());
            }
            return TypeAnn::Union(alts);
        }
        first
    }

    fn parse_type_ann_atom(&mut self) -> TypeAnn {
        match self.peek().clone() {
            Tok::Ident(w) => {
                self.next();
                // TYPED-MODE: generic annotation, `list[int]`, `map[str, int]`,
                // `result[int, str]`. Bracketed type args are unambiguous in
                // annotation position (Total-Grammar soft: unclosed lists
                // auto-close with a note, never a reject).
                if matches!(self.peek(), Tok::LBrack) {
                    self.next();
                    let mut args: Vec<TypeAnn> = Vec::new();
                    loop {
                        self.eat_newlines_inline();
                        match self.peek().clone() {
                            Tok::RBrack => {
                                self.next();
                                break;
                            }
                            Tok::Eof => {
                                let line = self.line();
                                self.note(line, 4, "type-argument list auto-closed");
                                break;
                            }
                            _ => {
                                args.push(self.parse_type_ann());
                                self.eat_newlines_inline();
                                if matches!(self.peek(), Tok::Comma) {
                                    self.next();
                                }
                            }
                        }
                    }
                    return TypeAnn::Generic(w, args);
                }
                let base = TypeAnn::Named(w);
                if matches!(self.peek(), Tok::Question) {
                    self.next();
                    TypeAnn::Optional(Box::new(base))
                } else {
                    base
                }
            }
            other => {
                let line = self.line();
                self.note(
                    line,
                    4,
                    format!(
                        "'{}' is not a type name; annotation treated as any",
                        other.describe()
                    ),
                );
                self.next();
                TypeAnn::Named("any".to_string())
            }
        }
    }

    /// L1a: destructuring pattern, `[a, b]`, `[head, *rest]`, `{x, y}`,
    /// nested list patterns. Soft Total Grammar: unclosed brackets are
    /// auto-closed with a note; garbage elements bind wildcards.
    fn parse_destructure_pat(&mut self) -> Pat {
        match self.peek().clone() {
            Tok::LBrack => {
                self.next();
                let mut elems = Vec::new();
                let mut rest = None;
                loop {
                    self.eat_newlines_inline();
                    match self.peek().clone() {
                        Tok::RBrack => {
                            self.next();
                            break;
                        }
                        Tok::Eof => {
                            let line = self.line();
                            self.note(line, 4, "pattern bracket auto-closed");
                            break;
                        }
                        Tok::Star => {
                            self.next();
                            if let Tok::Ident(w) = self.peek().clone() {
                                self.next();
                                rest = Some(w);
                            } else {
                                let line = self.line();
                                self.note(line, 4, "'*' in pattern needs a name; tail skipped");
                            }
                        }
                        Tok::Comma => {
                            self.next();
                        }
                        _ => {
                            elems.push(self.parse_destructure_pat());
                        }
                    }
                }
                Pat::List { elems, rest }
            }
            Tok::LBrace => {
                self.next();
                let mut keys = Vec::new();
                loop {
                    self.eat_newlines_inline();
                    match self.peek().clone() {
                        Tok::RBrace => {
                            self.next();
                            break;
                        }
                        Tok::Eof => {
                            let line = self.line();
                            self.note(line, 4, "pattern brace auto-closed");
                            break;
                        }
                        Tok::Comma => {
                            self.next();
                        }
                        Tok::Ident(w) => {
                            self.next();
                            keys.push(w);
                        }
                        other => {
                            let line = self.line();
                            self.note(
                                line,
                                4,
                                format!("'{}' is not a map-pattern key; skipped", other.describe()),
                            );
                            self.next();
                        }
                    }
                }
                Pat::Map { keys }
            }
            Tok::Ident(w) => {
                self.next();
                Pat::Bind(w)
            }
            other => {
                let line = self.line();
                self.note(
                    line,
                    4,
                    format!(
                        "'{}' cannot start a pattern; binding null",
                        other.describe()
                    ),
                );
                self.next();
                Pat::Bind("_".to_string())
            }
        }
    }

    fn parse_gene_def(
        &mut self,
        marks: Vec<String>,
        copies: u32,
        riboswitch: Option<(String, bool, f64)>,
        burst: Option<(f64, f64)>,
        doc: Vec<String>,
        deprecated: Option<crate::ast::Deprecation>,
    ) -> Stmt {
        // A13 (dx-r2): the def keyword's line, every definition-borne
        // runtime note (gates, silencing) points here.
        let def_line = self.line();
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
        // TYPED-MODE: declared generic type parameters, `gene first<T>(...)`.
        // `<` directly after the gene name in signature position is
        // unambiguous (expressions never follow a gene NAME); bounds are
        // plain idents (`T: numeric`, `U: Drawable`). Soft: unclosed lists
        // auto-close with a note.
        let mut type_params: Vec<(String, Option<String>)> = Vec::new();
        if matches!(self.peek(), Tok::Lt) && name.is_some() {
            self.next();
            loop {
                self.eat_newlines_inline();
                match self.peek().clone() {
                    Tok::Gt => {
                        self.next();
                        break;
                    }
                    Tok::Eof => {
                        let line = self.line();
                        self.note(line, 4, "type-parameter list auto-closed");
                        break;
                    }
                    Tok::Ident(p) => {
                        self.next();
                        let bound = if matches!(self.peek(), Tok::Colon) {
                            self.next();
                            match self.peek().clone() {
                                Tok::Ident(b) => {
                                    self.next();
                                    Some(b)
                                }
                                _ => {
                                    let line = self.line();
                                    self.note(line, 4, "type bound is not a name; dropped");
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        type_params.push((p, bound));
                        self.eat_newlines_inline();
                        if matches!(self.peek(), Tok::Comma) {
                            self.next();
                        }
                    }
                    other => {
                        let line = self.line();
                        self.note(
                            line,
                            4,
                            format!(
                                "'{}' is not a type parameter name; list auto-closed",
                                other.describe()
                            ),
                        );
                        break;
                    }
                }
            }
        };
        let mut params = Vec::new();
        let mut param_anns: Vec<Option<TypeAnn>> = Vec::new();
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
                // W01 (L2c): parameter annotation, `gene f(x: int) { }`
                let ann = if matches!(self.peek(), Tok::Colon) {
                    self.next();
                    Some(self.parse_type_ann())
                } else {
                    None
                };
                let default = if matches!(self.peek(), Tok::Eq) {
                    self.next();
                    Some(self.parse_expr())
                } else {
                    None
                };
                params.push((pname, default));
                param_anns.push(ann);
                if matches!(self.peek(), Tok::Comma) {
                    self.next();
                }
                if self.pos == before {
                    // no progress: non-parameter token inside the signature,
                    // auto-close instead of spinning (Total Grammar recovery)
                    let line = self.line();
                    self.note(line, 4, "unclosed parameter list; auto-closed");
                    break;
                }
            }
        }
        // W01 (L2c): return annotation, `gene f(x) -> int { }` (Tok::Arrow,
        // parsed before the uORF guard clause).
        let ret_ann = if matches!(self.peek(), Tok::Arrow) {
            self.next();
            Some(self.parse_type_ann())
        } else {
            None
        };
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
                line: def_line,
                doc,
                params,
                param_anns,
                ret_ann,
                type_params,
                guard,
                body: vec![Stmt::Return(Some(e))],
                acetylate,
                methylate,
                m6a,
                copies,
                seq: false,
                riboswitch,
                burst,
                deprecated,
            };
            return Stmt::Gene(std::sync::Arc::new(def));
        }
        let body = self.parse_block().unwrap_or_default();
        let def = GeneDef {
            name,
            line: def_line,
            doc,
            params,
            param_anns,
            ret_ann,
            type_params,
            guard,
            body,
            acetylate,
            methylate,
            m6a,
            copies,
            seq: false,
            riboswitch,
            burst,
            deprecated,
        };
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
                    if let Some(s) = self.parse_stmt() {
                        out.push(s);
                    }
                    if self.pos == before {
                        let line = self.line();
                        self.note(
                            line,
                            4,
                            format!("token '{}' skipped", self.peek().describe()),
                        );
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
        let mut left = self.parse_nullish();
        loop {
            let is_or = self.at_kw("or") || matches!(self.peek(), Tok::PipePipe);
            if is_or {
                self.next();
                let right = self.parse_nullish();
                left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right), self.line());
            } else {
                break;
            }
        }
        left
    }

    /// L1a: `a ?? b`, sits between `or` and `and` so `a or b ?? c` reads as
    /// `a or (b ?? c)`. Associative, so a left-assoc loop is fine.
    fn parse_nullish(&mut self) -> Expr {
        let mut left = self.parse_and();
        while matches!(self.peek(), Tok::QuestionQuestion) {
            let nullish_line = self.line();
            self.next();
            let right = self.parse_and();
            left = Expr::Binary(
                BinOp::Nullish,
                Box::new(left),
                Box::new(right),
                nullish_line,
            );
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
                left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right), self.line());
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
                let bin_line = self.line();
                self.next();
                let right = self.parse_bitor();
                left = Expr::Binary(op, Box::new(left), Box::new(right), bin_line);
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
            left = Expr::Binary(BinOp::BitOr, Box::new(left), Box::new(right), self.line());
        }
        left
    }

    fn parse_bitxor(&mut self) -> Expr {
        let mut left = self.parse_bitand();
        while matches!(self.peek(), Tok::Caret) {
            self.next();
            let right = self.parse_bitand();
            left = Expr::Binary(BinOp::BitXor, Box::new(left), Box::new(right), self.line());
        }
        left
    }

    fn parse_bitand(&mut self) -> Expr {
        let mut left = self.parse_shift();
        while matches!(self.peek(), Tok::Amp) {
            self.next();
            let right = self.parse_shift();
            left = Expr::Binary(BinOp::BitAnd, Box::new(left), Box::new(right), self.line());
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
                let bin_line = self.line();
                self.next();
                let right = self.parse_add();
                left = Expr::Binary(op, Box::new(left), Box::new(right), bin_line);
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
                let bin_line = self.line();
                self.next();
                let right = self.parse_mul();
                left = Expr::Binary(op, Box::new(left), Box::new(right), bin_line);
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
                let bin_line = self.line();
                self.next();
                let right = self.parse_unary();
                left = Expr::Binary(op, Box::new(left), Box::new(right), bin_line);
            } else {
                break;
            }
        }
        left
    }

    fn parse_unary(&mut self) -> Expr {
        // unary chains (`------x` ×1M) recurse natively, same depth cap as
        // paren/list nesting (S4 NEW-4)
        if self.depth >= 4096 {
            let line = self.line();
            self.note(line, 4, "expression nested deeper than 4096; truncated");
            self.skip_to_stmt_end();
            return Expr::Null;
        }
        if matches!(self.peek(), Tok::Minus) {
            self.next();
            self.depth += 1;
            let e = self.parse_unary();
            self.depth -= 1;
            return Expr::Unary(UnOp::Neg, Box::new(e));
        }
        if matches!(self.peek(), Tok::Tilde) {
            self.next();
            self.depth += 1;
            let e = self.parse_unary();
            self.depth -= 1;
            return Expr::Unary(UnOp::BitNot, Box::new(e));
        }
        self.parse_pow()
    }

    /// Consume tokens up to (and including) the next statement terminator,
    /// used when a depth cap truncates a pathological expression.
    fn skip_to_stmt_end(&mut self) {
        let mut depth = 0usize;
        while self.pos < self.toks.len() {
            match self.toks[self.pos].0 {
                Tok::LParen | Tok::LBrack | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBrack | Tok::RBrace => {
                    if depth == 0 {
                        return; // let the caller see the closing delimiter
                    }
                    depth -= 1;
                }
                Tok::Newline | Tok::Semi if depth == 0 => {
                    self.pos += 1;
                    return;
                }
                Tok::Eof => return,
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn parse_pow(&mut self) -> Expr {
        // '**' binds tighter than unary minus on the left (-2**2 = -4) and is
        // right-associative (2**3**2 = 2**9). The right operand re-enters
        // parse_unary so 2**-3 parses.
        let left = self.parse_postfix();
        if matches!(self.peek(), Tok::StarStar) {
            let pow_line = self.line();
            self.next();
            let right = self.parse_unary();
            return Expr::Binary(BinOp::Pow, Box::new(left), Box::new(right), pow_line);
        }
        left
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut e = self.parse_primary();
        loop {
            match self.peek().clone() {
                Tok::LParen => {
                    let call_line = self.line();
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
                    e = Expr::Call(Box::new(e), args, call_line);
                }
                Tok::LBrack => {
                    let index_line = self.line();
                    self.next();
                    let idx = self.parse_expr();
                    if matches!(self.peek(), Tok::RBrack) {
                        self.next();
                    } else {
                        let line = self.line();
                        self.note(line, 4, "index bracket auto-closed");
                    }
                    e = Expr::Index(Box::new(e), Box::new(idx), index_line);
                }
                Tok::Dot => {
                    self.next();
                    let method_line = self.line();
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
                                e = Expr::Method(Box::new(e), m, args, method_line);
                            } else {
                                e = Expr::Member(Box::new(e), m);
                            }
                        }
                        other => {
                            let line = self.line();
                            self.note(
                                line,
                                4,
                                format!("'.' followed by '{}'; member skipped", other.describe()),
                            );
                            break;
                        }
                    }
                }
                Tok::QuestionDot => {
                    // L1a: `a?.k` / `a?.k(args)`, null-safe member access.
                    self.next();
                    let method_line = self.line();
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
                                e = Expr::MethodSafe(Box::new(e), m, args, method_line);
                            } else {
                                e = Expr::MemberSafe(Box::new(e), m);
                            }
                        }
                        other => {
                            let line = self.line();
                            self.note(
                                line,
                                4,
                                format!(
                                    "'?.' followed by '{}'; chain resolves to null",
                                    other.describe()
                                ),
                            );
                            e = Expr::MemberSafe(Box::new(e), String::new());
                            break;
                        }
                    }
                }
                Tok::QuestionBang => {
                    // W06 (D-014): `e?!`, Option/Result propagation, a
                    // postfix operator (binds tighter than every binary op,
                    // repeats: Some(Some(x))?!?! unwraps twice).
                    let line = self.line();
                    self.next();
                    e = Expr::Propagate(Box::new(e), line);
                }
                Tok::Colon
                    if matches!(self.toks.get(self.pos + 1).map(|t| &t.0), Some(Tok::Colon)) =>
                {
                    // W025 stage 2: `e::name` is the namespace spelling of
                    // `e.name` (stage 1 made `::` a use-path separator; stage 2
                    // lets qualified reads and calls ride the same spelling
                    // through nested tables). Exactly sugar for the Dot arm
                    // below, the AST is identical.
                    self.next();
                    self.next();
                    let method_line = self.line();
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
                                e = Expr::Method(Box::new(e), m, args, method_line);
                            } else {
                                e = Expr::Member(Box::new(e), m);
                            }
                        }
                        other => {
                            let line = self.line();
                            self.note(
                                line,
                                4,
                                format!("'::' followed by '{}'; member skipped", other.describe()),
                            );
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
            Tok::Bytes(b) => {
                self.next();
                Expr::Bytes(b)
            }
            Tok::Interp(raw) => {
                let line = self.line();
                self.next();
                self.build_interp(raw, line)
            }
            Tok::LParen => {
                // nesting cap: a million-deep `((((…` must not exhaust the
                // native stack (Critic-X rt_p1c3), truncate as a rung-4 note
                if self.depth >= 4096 {
                    let line = self.line();
                    self.note(line, 4, "expression nested deeper than 4096; truncated");
                    let mut bal = 0usize;
                    while self.pos < self.toks.len() {
                        match self.toks[self.pos].0 {
                            Tok::LParen => bal += 1,
                            Tok::RParen => {
                                bal -= 1;
                                self.pos += 1;
                                if bal == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.pos += 1;
                    }
                    return Expr::Null;
                }
                self.next();
                self.depth += 1;
                let e = self.parse_expr();
                self.depth -= 1;
                if matches!(self.peek(), Tok::RParen) {
                    self.next();
                } else {
                    let line = self.line();
                    self.note(line, 4, "parenthesis auto-closed");
                }
                e
            }
            Tok::LBrack => {
                if self.depth >= 4096 {
                    let line = self.line();
                    self.note(line, 4, "expression nested deeper than 4096; truncated");
                    let mut bal = 0usize;
                    while self.pos < self.toks.len() {
                        match self.toks[self.pos].0 {
                            Tok::LBrack => bal += 1,
                            Tok::RBrack => {
                                bal -= 1;
                                self.pos += 1;
                                if bal == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.pos += 1;
                    }
                    return Expr::Null;
                }
                self.depth += 1;
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
                self.depth -= 1;
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
                    "null" | "nil" | "nothing" => {
                        // W06 (D-014): 'none' was RETIRED from the null-synonym
                        // set, it is now the Option constructor (none()). Bare
                        // 'none' degrades to an unbound ident (phantom note),
                        // never a silent null.
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
                        match self.parse_gene_def(vec![], 1, None, None, Vec::new(), None) {
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
                                    self.note(
                                        self.line(),
                                        4,
                                        "unclosed constructor argument list; auto-closed",
                                    );
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
                        Expr::Collect {
                            var,
                            iter: Box::new(iter),
                            filter,
                            body: Box::new(body),
                        }
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
                    format!(
                        "unexpected token '{}' in expression; null substituted",
                        other.describe()
                    ),
                );
                self.next();
                Expr::Null
            }
        }
    }

    fn parse_map_literal(&mut self) -> Expr {
        if self.depth >= 4096 {
            let line = self.line();
            self.note(line, 4, "expression nested deeper than 4096; truncated");
            let mut bal = 0usize;
            while self.pos < self.toks.len() {
                match self.toks[self.pos].0 {
                    Tok::LBrace => bal += 1,
                    Tok::RBrace => {
                        bal -= 1;
                        self.pos += 1;
                        if bal == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                self.pos += 1;
            }
            return Expr::Map(Vec::new());
        }
        self.depth += 1;
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
                        Tok::Bytes(b) => {
                            self.next();
                            Expr::Bytes(b)
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
        // sec-r4 (F-1): interpolation nesting recurses parser-in-parser; each
        // level used to start a fresh depth counter, so `"{ \"{ … }\" }"` was
        // unbounded (60 KB source -> 2.14 GB RSS -> SIGKILL). Cap nesting and
        // inherit the parent depth so the existing cap machinery applies.
        if self.depth >= 64 {
            self.note(
                line,
                4,
                "string interpolation nested deeper than 64; treated as literal text",
            );
            return Expr::Interp(vec![InterpPart::Lit(raw)]);
        }
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
                self.depth += 1;
                let mut sub = parse_snippet(&expr_txt, self.depth);
                let e = sub.parse_expr();
                self.depth -= 1;
                for mut n in sub.notes.drain(..) {
                    n.line = line;
                    self.note(n.line, n.rung, n.message);
                }
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

    /// W02 (match-v2) pattern grammar:
    ///   pattern  := atom ( '|' atom )*            (or-pattern, first hit binds)
    ///             | pattern 'if' expr             (guard, sees the bindings)
    ///   atom     := literal | '_' | ident        (legacy forms, unchanged)
    ///             | 'Some'/'None'/'Ok'/'Err' [ '(' pattern ')' ]
    ///             | '[' pattern,... [ '*' ident ] ']'
    ///             | '{' ident [':' pattern],... '}'
    /// Legacy literal comma-runs (`case 1, 2 =>`) keep their Multi shape and
    /// their legacy edge behavior byte-for-byte; everything new hangs off the
    /// atom/`|`/`if` rules. Total Grammar: a malformed pattern degrades to a
    /// wildcard/bind with a note, never a rejection.
    fn parse_pattern(&mut self) -> MatchPat {
        let first = self.parse_pat_atom();
        // Legacy literal comma-run, only for a leading literal, preserving
        // the pre-W02 shape (`case 1, 2 =>` → Multi) and edge behavior
        // (a non-literal in the run discards the collected literals).
        let leading_lit = match &first {
            MatchPat::Lit(e) => Some(e.clone()),
            _ => None,
        };
        if let Some(e0) = leading_lit {
            if matches!(self.peek(), Tok::Comma) {
                let mut lits: Vec<Expr> = vec![e0];
                loop {
                    if !matches!(self.peek(), Tok::Comma) {
                        break;
                    }
                    self.next();
                    let neg = matches!(self.peek(), Tok::Minus);
                    if neg {
                        self.next();
                    }
                    match self.peek().clone() {
                        Tok::Int(i) => {
                            self.next();
                            lits.push(Expr::Int(if neg { -i } else { i }));
                        }
                        Tok::Float(f) => {
                            self.next();
                            lits.push(Expr::Float(if neg { -f } else { f }));
                        }
                        Tok::Str(s) => {
                            if neg {
                                let line = self.line();
                                self.note(line, 4, "'-' before a string pattern ignored");
                            }
                            self.next();
                            lits.push(Expr::Str(s));
                        }
                        Tok::Bytes(b) => {
                            if neg {
                                let line = self.line();
                                self.note(line, 4, "'-' before a bytes pattern ignored");
                            }
                            self.next();
                            lits.push(Expr::Bytes(b));
                        }
                        // Legacy edge: a non-literal in the comma-run returns
                        // that pattern alone and discards prior literals
                        // (pre-W02 behavior kept verbatim).
                        _ => return self.parse_pat_atom(),
                    }
                }
                return if lits.len() == 1 {
                    MatchPat::Lit(lits.remove(0))
                } else {
                    MatchPat::Multi(lits)
                };
            }
        }
        // Or-pattern chain: `p1 | p2 | ...`, newlines are allowed before
        // any alternative (multi-line chains), before the guard check, and
        // nowhere else. Eating them here is safe: an arm body always starts
        // with `{` or `=>`, never with `|`/`if`.
        self.eat_newlines_inline();
        let pat = if matches!(self.peek(), Tok::Pipe) {
            let mut alts = vec![first];
            loop {
                self.eat_newlines_inline();
                if !matches!(self.peek(), Tok::Pipe) {
                    break;
                }
                self.next();
                alts.push(self.parse_pat_atom());
                self.eat_newlines_inline();
            }
            MatchPat::Or(alts)
        } else {
            first
        };
        // Guarded arm: `pat if cond`, applies to the WHOLE or-chain (the
        // condition sees whichever alternative won).
        if self.at_kw("if") {
            self.next();
            let cond = self.parse_expr();
            return MatchPat::Guard(Box::new(pat), cond);
        }
        pat
    }

    /// One pattern atom (no `|`, no `if` at this level).
    fn parse_pat_atom(&mut self) -> MatchPat {
        let neg = matches!(self.peek(), Tok::Minus);
        if neg {
            self.next();
        }
        match self.peek().clone() {
            Tok::Int(i) => {
                self.next();
                MatchPat::Lit(Expr::Int(if neg { -i } else { i }))
            }
            Tok::Float(f) => {
                self.next();
                MatchPat::Lit(Expr::Float(if neg { -f } else { f }))
            }
            Tok::Str(s) => {
                if neg {
                    let line = self.line();
                    self.note(line, 4, "'-' before a string pattern ignored");
                }
                self.next();
                MatchPat::Lit(Expr::Str(s))
            }
            Tok::Bytes(b) => {
                if neg {
                    let line = self.line();
                    self.note(line, 4, "'-' before a bytes pattern ignored");
                }
                self.next();
                MatchPat::Lit(Expr::Bytes(b))
            }
            Tok::Ident(w) if !neg && w == "true" => {
                self.next();
                MatchPat::Lit(Expr::Bool(true))
            }
            Tok::Ident(w) if !neg && w == "false" => {
                self.next();
                MatchPat::Lit(Expr::Bool(false))
            }
            Tok::Ident(w) if !neg && w == "null" => {
                self.next();
                MatchPat::Lit(Expr::Null)
            }
            Tok::Ident(w) if !neg && w == "_" => {
                self.next();
                MatchPat::Wild
            }
            Tok::Ident(w) if !neg && matches!(w.as_str(), "Some" | "None" | "Ok" | "Err") => {
                self.next();
                let payload = if matches!(self.peek(), Tok::LParen) {
                    self.next();
                    let line = self.line();
                    if matches!(self.peek(), Tok::RParen) {
                        self.next();
                        self.note(
                            line,
                            4,
                            "empty variant payload pattern, treated as tag-only",
                        );
                        None
                    } else {
                        let p = self.parse_pat_atom();
                        if matches!(self.peek(), Tok::Comma) {
                            self.note(
                                line,
                                4,
                                "variant payload is a single value; extra pattern elements ignored",
                            );
                            while !matches!(self.peek(), Tok::RParen | Tok::Eof) {
                                self.next();
                            }
                        }
                        if matches!(self.peek(), Tok::RParen) {
                            self.next();
                        } else {
                            let line = self.line();
                            self.note(line, 4, "variant payload pattern auto-closed");
                        }
                        Some(Box::new(p))
                    }
                } else {
                    None
                };
                MatchPat::Variant(w, payload)
            }
            Tok::Ident(w) if !neg && w.chars().next().is_some_and(|c| c.is_ascii_uppercase()) => {
                // Unknown capitalized tag: soft fallback to a binding (the
                // whole subject value binds under the name). A parenthesized
                // payload is consumed and ignored so the token stream stays
                // aligned (Total Grammar: degrade, never reject).
                let line = self.line();
                self.note(
                    line,
                    4,
                    format!("unknown variant tag '{w}', pattern treated as a binding"),
                );
                self.next();
                if matches!(self.peek(), Tok::LParen) {
                    self.note(line, 4, "unknown tag payload ignored (bound as a whole)");
                    self.next();
                    if !matches!(self.peek(), Tok::RParen) {
                        let _ = self.parse_pat_atom();
                        if matches!(self.peek(), Tok::Comma) {
                            while !matches!(self.peek(), Tok::RParen | Tok::Eof) {
                                self.next();
                            }
                        }
                    }
                    if matches!(self.peek(), Tok::RParen) {
                        self.next();
                    } else {
                        let line = self.line();
                        self.note(line, 4, "unknown tag payload auto-closed");
                    }
                }
                MatchPat::Bind(w)
            }
            Tok::Ident(w) if !neg => {
                self.next();
                MatchPat::Bind(w)
            }
            Tok::LBrack if !neg => {
                self.next();
                let mut elems: Vec<MatchPat> = Vec::new();
                let mut rest: Option<String> = None;
                loop {
                    self.eat_newlines_inline();
                    match self.peek().clone() {
                        Tok::RBrack => {
                            self.next();
                            break;
                        }
                        Tok::Eof => {
                            let line = self.line();
                            self.note(line, 4, "pattern bracket auto-closed");
                            break;
                        }
                        Tok::Star => {
                            self.next();
                            if let Tok::Ident(w) = self.peek().clone() {
                                self.next();
                                rest = Some(w);
                            } else {
                                let line = self.line();
                                self.note(line, 4, "'*' in pattern needs a name; tail skipped");
                            }
                        }
                        Tok::Comma => {
                            self.next();
                        }
                        _ => elems.push(self.parse_pat_atom()),
                    }
                }
                MatchPat::ListPat { elems, rest }
            }
            Tok::LBrace if !neg => {
                self.next();
                let mut keys: Vec<(String, Option<Box<MatchPat>>)> = Vec::new();
                loop {
                    self.eat_newlines_inline();
                    match self.peek().clone() {
                        Tok::RBrace => {
                            self.next();
                            break;
                        }
                        Tok::Eof => {
                            let line = self.line();
                            self.note(line, 4, "pattern brace auto-closed");
                            break;
                        }
                        Tok::Comma => {
                            self.next();
                        }
                        Tok::Ident(w) => {
                            self.next();
                            let sub = if matches!(self.peek(), Tok::Colon) {
                                self.next();
                                Some(Box::new(self.parse_pat_atom()))
                            } else {
                                None
                            };
                            keys.push((w, sub));
                        }
                        other => {
                            let line = self.line();
                            self.note(
                                line,
                                4,
                                format!("'{}' is not a map-pattern key; skipped", other.describe()),
                            );
                            self.next();
                        }
                    }
                }
                MatchPat::MapPat { keys }
            }
            other => {
                if neg {
                    let line = self.line();
                    self.note(line, 4, "dangling '-' in pattern treated as wildcard");
                }
                let line = self.line();
                self.note(
                    line,
                    4,
                    format!("pattern '{}' treated as wildcard", other.describe()),
                );
                self.next();
                MatchPat::Wild
            }
        }
    }
}

fn parse_snippet(src: &str, depth: u32) -> Parser {
    let lexed = lex(src);
    Parser {
        toks: lexed.toks,
        pos: 0,
        notes: lexed.notes,
        depth,
        // snippet parsing (REPL fragments) carries no docs
        docs: Vec::new(),
        doc_cursor: 0,
        first_tok_line: 1,
        module_doc_assigned: false,
        module_doc: Vec::new(),
        pub_names: Vec::new(),
    }
}
