//! ls.rs — operon-ls language-server core (G5 seed).
//!
//! Pure analysis logic, unit-testable: `analyze` parses an editor buffer and
//! produces diagnostics (Total Grammar parse notes + phantom calls from the
//! `tools check` engine) plus the gene/splice tables; `hover` answers
//! textDocument/hover with a gene signature. The stdio JSON-RPC loop lives in
//! `src/bin/operon-ls.rs`.
//!
//! Positions are LSP-style 0-based (line, UTF-16-free character columns —
//! the seed counts chars, which is exact for ASCII sources and close enough
//! for the seed elsewhere).

use crate::ast::{Note, Stmt};
use crate::tools::check_source;
use crate::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// 0-based line.
    pub line: usize,
    /// 0-based column (start of the flagged span).
    pub col: usize,
    /// Length of the flagged span (0 = to end of line, editor's choice).
    pub len: usize,
    /// LSP severity: 1 error, 2 warning, 3 information, 4 hint.
    pub severity: u8,
    pub message: String,
    /// W46: repair provenance — what the interpreter decided the token meant
    /// (canonical form + rung). Empty when nothing was repaired (phantoms,
    /// canonical teaching notes). Serialized as LSP relatedInformation.
    pub related: Vec<Related>,
}

/// W46: one repair-provenance entry — the span it applies to (same range as
/// the diagnostic in practice) and the canonical interpretation.
#[derive(Debug, Clone, PartialEq)]
pub struct Related {
    pub line: usize,
    pub col: usize,
    pub len: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct GeneInfo {
    pub name: String,
    pub params: Vec<String>,
    pub acetylate: bool,
    pub methylate: bool,
    pub m6a: bool,
    pub enhanced: bool,
    pub seq: bool,
}

#[derive(Debug, Clone)]
pub struct SpliceInfo {
    pub root: String,
    pub variants: Vec<String>,
}

#[derive(Debug, Default)]
pub struct LsDoc {
    pub genes: Vec<GeneInfo>,
    pub splices: Vec<SpliceInfo>,
    pub diagnostics: Vec<Diagnostic>,
}

fn note_severity(rung: u8) -> u8 {
    match rung {
        1 => 4, // hint: canonical, noted for teaching
        2 => 3, // information: synonym accepted
        3 => 2, // warning: wobble, program still runs
        _ => 1, // fallback: parsed under fault tolerance, likely wrong
    }
}

fn rung_label(rung: u8) -> &'static str {
    match rung {
        1 => "canonical",
        2 => "synonym",
        3 => "wobble",
        _ => "fallback",
    }
}

/// Word under the cursor: the identifier span containing `col`, if any.
/// Returns (start_col, word).
pub fn word_at(line: &str, col: usize) -> Option<(usize, String)> {
    let b: Vec<char> = line.chars().collect();
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    if col >= b.len() || !is_id(b[col]) {
        // allow hovering just past the end of a word (editors do this)
        if col == 0 || col > b.len() || col > 0 && !is_id(b[col - 1]) {
            return None;
        }
    }
    let mut start = col.min(b.len().saturating_sub(1));
    while start > 0 && is_id(b[start - 1]) {
        start -= 1;
    }
    let mut end = start;
    while end < b.len() && is_id(b[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some((start, b[start..end].iter().collect()))
}

/// First 0-based line containing `gene NAME` / `seq NAME` as a definition.
fn def_line(src: &str, kind: &str, name: &str) -> Option<usize> {
    let needle = format!("{} {}", kind, name);
    for (i, l) in src.lines().enumerate() {
        // simple scan: the needle, with the char before it not being an id char
        if let Some(pos) = l.find(&needle) {
            let before_ok = pos == 0 || !line_char_is_id(l, pos - 1);
            let after = pos + needle.len();
            let after_ok = after >= l.len() || !line_char_is_id(l, after);
            if before_ok && after_ok {
                return Some(i);
            }
        }
    }
    None
}

fn line_char_is_id(l: &str, idx: usize) -> bool {
    l.chars()
        .nth(idx)
        .map(|c| c.is_ascii_alphanumeric() || c == '_')
        .unwrap_or(false)
}

/// W46: extract the canonical token a repair note names. Covers the three
/// repair phrasings the parser emits today:
///   rung 2 — `synonym 'x' repaired to 'y'`
///   rung 3 — `wobble: 'x' repaired to keyword 'y'` / `wobble: '@x' repaired to '@y'`
/// Rung-4 fallbacks name no canonical form ("unmatched '}' skipped") → None.
fn canonical_from(msg: &str) -> Option<String> {
    for (pat, strip_at) in [
        ("repaired to keyword '", false),
        ("repaired to '@", true),
        ("repaired to '", false),
    ] {
        if let Some(a) = msg.find(pat) {
            let rest = &msg[a + pat.len()..];
            if let Some(b) = rest.find('\'') {
                let t = if strip_at {
                    rest[..b].trim_start_matches('@')
                } else {
                    &rest[..b]
                };
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

fn note_to_diagnostic(src: &str, n: &Note) -> Diagnostic {
    let line0 = n.line.saturating_sub(1); // parser lines are 1-based
    let line_text = src.lines().nth(line0).unwrap_or("");
    // underline the offending token: notes quote it ('x'), so the first
    // single-quoted segment of the message is the best span candidate;
    // fall back to the line's first identifier
    let mut target: Option<String> = None;
    if let Some(a) = n.message.find('\'') {
        if let Some(b) = n.message[a + 1..].find('\'') {
            let tok = n.message[a + 1..a + 1 + b]
                .trim_start_matches('@')
                .to_string();
            if !tok.is_empty() {
                target = Some(tok);
            }
        }
    }
    let (col, len) = pick_span(line_text, target.as_deref());
    // W46: repair rungs (2 synonym, 3 wobble, 4+ fallback) carry provenance;
    // rung-1 notes are canonical teaching notes — nothing was repaired.
    let related = if n.rung >= 2 {
        match canonical_from(&n.message) {
            Some(canon) => vec![Related {
                line: line0,
                col,
                len,
                message: format!("interpreted as '{}' ({} rung)", canon, rung_label(n.rung)),
            }],
            None => vec![Related {
                line: line0,
                col,
                len,
                message: format!(
                    "parsed under fault tolerance ({} rung) — no canonical form",
                    rung_label(n.rung)
                ),
            }],
        }
    } else {
        Vec::new()
    };
    Diagnostic {
        line: line0,
        col,
        len,
        severity: note_severity(n.rung),
        message: format!("{} ({} rung)", n.message, rung_label(n.rung)),
        related,
    }
}

/// Column + length of `target` as a word on the line; else the first
/// identifier; else (0, 0).
fn pick_span(line: &str, target: Option<&str>) -> (usize, usize) {
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    if let Some(t) = target {
        if let Some(pos) = line.find(t) {
            let before_ok = pos == 0 || !is_id(line[..pos].chars().last().unwrap_or(' '));
            let after = pos + t.len();
            let after_ok =
                after >= line.len() || !is_id(line[after..].chars().next().unwrap_or(' '));
            if before_ok && after_ok {
                return (line[..pos].chars().count(), t.chars().count());
            }
        }
    }
    let chars: Vec<char> = line.chars().collect();
    let start = match chars
        .iter()
        .position(|c| c.is_ascii_alphabetic() || *c == '_')
    {
        Some(i) => i,
        None => return (0, 0),
    };
    let len = chars[start..].iter().take_while(|c| is_id(**c)).count();
    (start, len)
}

/// Analyze an editor buffer: parse notes + check-engine phantoms as
/// diagnostics; collect the gene/splice tables for hover.
pub fn analyze(src: &str) -> LsDoc {
    analyze_doc(src, None)
}

/// lsp-r1 (P0): analyze with the document's own directory as the first
/// `use`-module search base — editors launch servers from arbitrary CWDs,
/// and module resolution must not depend on the process CWD.
pub fn analyze_doc(src: &str, base_dir: Option<&str>) -> LsDoc {
    let mut doc = LsDoc::default();
    let prog = crate::parser::parse(src);
    doc.diagnostics = prog
        .notes
        .iter()
        .map(|n| note_to_diagnostic(src, n))
        .collect();

    let enhanced: Vec<String> = prog
        .stmts
        .iter()
        .filter_map(|st| match st {
            Stmt::Enhance(names) => Some(names.clone()),
            _ => None,
        })
        .flatten()
        .collect();

    for st in &prog.stmts {
        match st {
            Stmt::Gene(g) | Stmt::Seq(g) => {
                if let Some(n) = &g.name {
                    doc.genes.push(GeneInfo {
                        name: n.clone(),
                        params: g.params.iter().map(|(p, _)| p.clone()).collect(),
                        acetylate: g.acetylate,
                        methylate: g.methylate,
                        m6a: g.m6a,
                        enhanced: enhanced.iter().any(|e| e == n),
                        seq: matches!(st, Stmt::Seq(_)),
                    });
                }
            }
            Stmt::Splice(sp) => {
                doc.splices.push(SpliceInfo {
                    root: sp.root.clone(),
                    variants: sp.variants.iter().map(|(v, _)| v.clone()).collect(),
                });
            }
            _ => {}
        }
    }

    // phantoms from the same engine `operon check` uses (defined-vs-called)
    let (rep, _) = check_source(src, false, base_dir);
    for p in &rep.phantoms {
        // underline the first bare occurrence of the name
        let mut diag = Diagnostic {
            line: 0,
            col: 0,
            len: p.chars().count(),
            severity: 1,
            message: format!(
                "phantom call: '{}' is called but not defined in this file (check)",
                p
            ),
            related: Vec::new(), // a phantom is not a repair — no provenance
        };
        for (i, l) in src.lines().enumerate() {
            if let Some(pos) = l.find(p.as_str()) {
                if pos == 0 || !line_char_is_id(l, pos - 1) {
                    let after = pos + p.len();
                    if after >= l.len() || !line_char_is_id(l, after) {
                        let col = l[..pos].chars().count();
                        diag.line = i;
                        diag.col = col;
                        break;
                    }
                }
            }
        }
        doc.diagnostics.push(diag);
    }
    doc.diagnostics.sort_by_key(|d| (d.line, d.col));
    doc
}

/// W46: the repair provenance covering (line0, col0), if any — a diagnostic
/// whose span contains the position AND carries related entries.
fn repair_at(doc: &LsDoc, line0: usize, col0: usize) -> Option<&Related> {
    doc.diagnostics
        .iter()
        .find(|d| {
            d.line == line0 && !d.related.is_empty() && col0 >= d.col && col0 < d.col + d.len.max(1)
        })
        .and_then(|d| d.related.first())
}

/// Hover text (markdown) for the word under the cursor, per D-008:
/// programmer-first, gene vocabulary as intuition aid, never a prerequisite.
/// W46: hovering a repaired token shows what the interpreter decided it
/// meant — provenance first, so the editor and the parser never disagree
/// silently.
pub fn hover(src: &str, doc: &LsDoc, line0: usize, col0: usize) -> Option<String> {
    let line = src.lines().nth(line0)?;
    let (_, word) = word_at(line, col0)?;
    let provenance = repair_at(doc, line0, col0)
        .map(|r| format!("\n\n---\n*Total Grammar repair: {}*", r.message))
        .unwrap_or_default();

    if let Some(g) = doc.genes.iter().find(|g| g.name == word) {
        let kind = if g.seq { "seq" } else { "gene" };
        let mut sig = format!("{} {}({})", kind, g.name, g.params.join(", "));
        let mut marks = Vec::new();
        if g.acetylate {
            marks.push("@acetylate");
        }
        if g.methylate {
            marks.push("@methylate");
        }
        if g.m6a {
            marks.push("@m6a");
        }
        if g.enhanced {
            marks.push("enhance");
        }
        if !marks.is_empty() {
            sig.push(' ');
            sig.push_str(&marks.join(" "));
        }
        let def = def_line(src, kind, &g.name)
            .map(|l| format!("\n\ndeclared at line {}", l + 1))
            .unwrap_or_default();
        return Some(format!(
            "```operon\n{}\n```\n\ngene — Operon's named function (a callable unit of expression). \
Calls may be gated by GRN thresholds, methylation, or toggle state (SPEC §11).{}{}",
            sig, def, provenance
        ));
    }

    if let Some(sp) = doc.splices.iter().find(|s| s.root == word) {
        let variants = sp
            .variants
            .iter()
            .map(|v| format!("variant {}", v))
            .collect::<Vec<_>>()
            .join(", ");
        let def = def_line(src, "splice", &sp.root)
            .map(|l| format!("\n\ndeclared at line {}", l + 1))
            .unwrap_or_default();
        return Some(format!(
            "```operon\nsplice {} {{ {} }}\n```\n\nsplice — a dispatch group: the call `{}(...)` runs the \
active variant (selection: .cell > CLI > @m6a > first declared).{}",
            sp.root, variants, sp.root, def
        ));
    }

    if crate::interp::BUILTIN_NAMES.contains(&word.as_str()) {
        return Some(format!(
            "```operon\n{}()\n```\n\nbuilt-in gene (SPEC §10).{}",
            word, provenance
        ));
    }

    // W46: not a gene/splice/builtin — but if the parser repaired this token
    // (e.g. `retrn` → `return`, a synonym like `var`), hover answers with the
    // provenance instead of an empty null. The editor and the interpreter
    // then agree in one view.
    if let Some(r) = repair_at(doc, line0, col0) {
        return Some(format!(
            "```operon\n{}\n```\n\n*Total Grammar repair: {}*",
            word, r.message
        ));
    }

    None
}

/// lsp-r1: textDocument/definition — the location of the definition of the
/// word under the cursor. Returns (line, col, len), 0-based line, of the
/// `gene NAME` / `seq NAME` / `splice NAME` site. Builtins and unknown
/// words resolve to None (the editor keeps the cursor).
pub fn definition(
    src: &str,
    doc: &LsDoc,
    line0: usize,
    col0: usize,
) -> Option<(usize, usize, usize)> {
    let line = src.lines().nth(line0)?;
    let (_, word) = word_at(line, col0)?;
    let kind = if doc
        .genes
        .iter()
        .find(|g| g.name == word)
        .map(|g| g.seq)
        .unwrap_or(false)
    {
        "seq"
    } else if doc.genes.iter().any(|g| g.name == word) {
        "gene"
    } else if doc.splices.iter().any(|s| s.root == word) {
        "splice"
    } else {
        return None;
    };
    let dl = def_line(src, kind, &word)?;
    let text = src.lines().nth(dl)?;
    let col = text
        .find(&word)
        .map(|p| text[..p].chars().count())
        .unwrap_or(0);
    Some((dl, col, word.chars().count()))
}

/// W45: textDocument/references — every word-boundary occurrence of the
/// identifier under the cursor, declaration included (includeDeclaration:
/// true by LSP default). Grep-class by design (the ROADMAP-100 spec): span-
/// accurate, single-file, no second parse. Mentions inside comments and
/// quoted strings are excluded. Returns (line, col, len).
pub fn references(
    src: &str,
    doc: &LsDoc,
    line0: usize,
    col0: usize,
) -> Option<Vec<(usize, usize, usize)>> {
    // doc is unused today (definition kinds come from the same scan) but
    // stays in the signature so callers keep passing the cached analysis.
    let _ = doc;
    let line = src.lines().nth(line0)?;
    let (_, word) = word_at(line, col0)?;
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    for (i, l) in src.lines().enumerate() {
        let chars: Vec<char> = l.chars().collect();
        let mut j = 0usize;
        let mut in_str = false;
        while j < chars.len() {
            let c = chars[j];
            if in_str {
                if c == '"' {
                    in_str = false;
                }
                j += 1;
                continue;
            }
            if c == '"' {
                in_str = true;
                j += 1;
                continue;
            }
            if c == '#' {
                break; // comment: nothing after this counts as a reference
            }
            if is_id(c) {
                let start = j;
                while j < chars.len() && is_id(chars[j]) {
                    j += 1;
                }
                let w: String = chars[start..j].iter().collect();
                if w == word {
                    out.push((i, start, j - start));
                }
            } else {
                j += 1;
            }
        }
    }
    Some(out)
}

/// W45: the fixed semantic-token legend. Index into this array is the
/// tokenType integer on the wire. Must stay in lockstep with the legend
/// advertised in the initialize capabilities (asserted by lsp_smoke).
pub const SEMANTIC_TOKEN_TYPES: [&str; 6] = [
    "keyword",  // 0 — parse-level words + @marks
    "function", // 1 — genes, seqs, splice roots, builtins
    "variable", // 2 — every other identifier
    "string",   // 3 — quoted spans (opening to closing quote)
    "number",   // 4 — numeric literals
    "comment",  // 5 — `#` to end of line (incl. `##` doc comments)
];

/// One classified span from the line scanner: (col, len, tokenType).
type ScanTok = (usize, usize, usize);

/// W45: classify one source line into semantic-token spans. String-aware
/// (a `#` inside quotes does not start a comment) and comment-terminating.
fn scan_line_tokens(line: &str, doc: &LsDoc) -> Vec<ScanTok> {
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<ScanTok> = Vec::new();
    let mut j = 0usize;
    while j < chars.len() {
        let c = chars[j];
        if c == '#' {
            // comment to end of line (## doc comments included)
            out.push((j, chars.len() - j, 5));
            break;
        }
        if c == '"' {
            // quoted span: opening quote through the closing quote (or EOL
            // for an unterminated string — the editor still gets the span)
            let start = j;
            j += 1;
            while j < chars.len() && chars[j] != '"' {
                j += 1;
            }
            if j < chars.len() {
                j += 1; // include the closing quote
            }
            out.push((start, j - start, 3));
            continue;
        }
        if c == '@' && j + 1 < chars.len() && is_id(chars[j + 1]) {
            // marks: @methylate & friends are parse-level directives
            let start = j;
            j += 1;
            while j < chars.len() && is_id(chars[j]) {
                j += 1;
            }
            out.push((start, j - start, 0));
            continue;
        }
        if is_id(c) {
            let start = j;
            while j < chars.len() && is_id(chars[j]) {
                j += 1;
            }
            let w: String = chars[start..j].iter().collect();
            let ty = if w
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                4 // number
            } else if crate::parser::KEYWORDS.contains(&w.as_str()) {
                0 // keyword
            } else if doc.genes.iter().any(|g| g.name == w)
                || doc.splices.iter().any(|s| s.root == w)
                || crate::interp::BUILTIN_NAMES.contains(&w.as_str())
            {
                1 // function-class: gene / seq / splice root / builtin
            } else {
                2 // variable
            };
            out.push((start, j - start, ty));
            continue;
        }
        j += 1; // operators and punctuation: no semantic token (editor themes cover them)
    }
    out
}

/// W45: textDocument/semanticTokens (full) — the whole document delta-encoded
/// per the LSP wire format: a flat `data` array of
/// [deltaLine, deltaStartCol, length, tokenType, tokenModifiers=0] quints,
/// tokens ordered by position. Editors that skip the capability degrade to
/// TextMate coloring with no behavioral change.
pub fn semantic_tokens(src: &str, doc: &LsDoc) -> Value {
    let mut data: Vec<Value> = Vec::new();
    let mut prev_line: i64 = 0;
    for (i, l) in src.lines().enumerate() {
        for (col, len, ty) in scan_line_tokens(l, doc) {
            let dl = i as i64 - prev_line;
            prev_line = i as i64;
            data.push(Value::Int(dl));
            data.push(Value::Int(col as i64)); // deltaStart resets per line
            data.push(Value::Int(len as i64));
            data.push(Value::Int(ty as i64));
            data.push(Value::Int(0)); // tokenModifiers
        }
    }
    Value::List(Rc::new(RefCell::new(data)))
}

/// lsp-r1: textDocument/documentSymbol — the file's callable inventory
/// (genes, sequences, splices) as LSP DocumentSymbol values. Data the
/// analyze() pass already collects; no second parse.
pub fn document_symbols(src: &str, doc: &LsDoc) -> Value {
    let sym = |name: String, detail: &str, line: usize| {
        let line_text = src.lines().nth(line).unwrap_or("");
        let col = line_text
            .find(&name)
            .map(|p| line_text[..p].chars().count())
            .unwrap_or(0);
        let len = name.chars().count();
        mapv(vec![
            ("name", Value::Str(name)),
            ("kind", Value::Int(12)), // LSP SymbolKind.Function
            ("range", range_value(line, 0, line_text.chars().count())),
            ("selectionRange", range_value(line, col, len)),
            ("detail", Value::Str(detail.into())),
        ])
    };
    let mut items: Vec<Value> = Vec::new();
    for g in &doc.genes {
        let kind = if g.seq { "seq" } else { "gene" };
        let line = def_line(src, kind, &g.name).unwrap_or(0);
        items.push(sym(
            format!("{}({})", g.name, g.params.join(", ")),
            kind,
            line,
        ));
    }
    for sp in &doc.splices {
        let line = def_line(src, "splice", &sp.root).unwrap_or(0);
        items.push(sym(
            format!("splice {} {{ {} }}", sp.root, sp.variants.join(", ")),
            "splice",
            line,
        ));
    }
    Value::List(Rc::new(RefCell::new(items)))
}

/// lsp-r1: textDocument/completion — a flat, editor-filtered inventory:
/// in-file genes/splices (with signatures), builtins, keywords, and
/// top-level bindings. Kind codes: 3 = Function, 14 = Keyword, 6 = Variable.
pub fn completions(src: &str, doc: &LsDoc) -> Vec<Value> {
    let mut items: Vec<Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push = |label: String,
                kind: i64,
                detail: String,
                seen: &mut std::collections::HashSet<String>,
                items: &mut Vec<Value>| {
        if seen.insert(label.clone()) {
            items.push(mapv(vec![
                ("label", Value::Str(label)),
                ("kind", Value::Int(kind)),
                ("detail", Value::Str(detail)),
            ]));
        }
    };
    for g in &doc.genes {
        let kind = if g.seq { "seq" } else { "gene" };
        let mut marks = String::new();
        if g.acetylate {
            marks.push_str(" @acetylate");
        }
        if g.methylate {
            marks.push_str(" @methylate");
        }
        if g.m6a {
            marks.push_str(" @m6a");
        }
        push(
            g.name.clone(),
            3,
            format!("{} {}({}){}", kind, g.name, g.params.join(", "), marks),
            &mut seen,
            &mut items,
        );
    }
    for sp in &doc.splices {
        push(
            sp.root.clone(),
            3,
            format!("splice {} {{ {} }}", sp.root, sp.variants.join(", ")),
            &mut seen,
            &mut items,
        );
    }
    // top-level bindings (v1: file-level lets — gene bodies come with the
    // AST-indexed completion pass; the editor filters client-side anyway)
    let prog = crate::parser::parse(src);
    for st in &prog.stmts {
        if let Stmt::Let(name, _) = st {
            push(name.clone(), 6, "let binding".into(), &mut seen, &mut items);
        }
    }
    for b in crate::interp::BUILTIN_NAMES {
        push(
            (*b).to_string(),
            3,
            "built-in gene (SPEC §10)".into(),
            &mut seen,
            &mut items,
        );
    }
    for k in crate::parser::KEYWORDS {
        push(
            (*k).to_string(),
            14,
            "keyword".into(),
            &mut seen,
            &mut items,
        );
    }
    items
}

/// lsp-r1: textDocument/formatting — the same canonical formatter `operon fmt`
/// uses, applied to the buffer. Returns the full-document replacement text.
pub fn format_text(src: &str) -> Option<String> {
    let prog = crate::parser::parse(src);
    Some(crate::tools::format_program(&prog))
}

/// Serialize diagnostics as an LSP publishDiagnostics params Value.
/// W46: diagnostics carrying repair provenance embed relatedInformation
/// pointing at the same range with the canonical interpretation.
pub fn publish_params(uri: &str, diags: &[Diagnostic]) -> Value {
    let items: Vec<Value> = diags
        .iter()
        .map(|d| {
            let mut pairs = vec![
                ("range", range_value(d.line, d.col, d.len)),
                ("severity", Value::Int(d.severity as i64)),
                ("source", Value::Str("operon-check".into())),
                ("message", Value::Str(d.message.clone())),
            ];
            if !d.related.is_empty() {
                let related: Vec<Value> = d
                    .related
                    .iter()
                    .map(|r| {
                        mapv(vec![
                            (
                                "location",
                                mapv(vec![
                                    ("uri", Value::Str(uri.to_string())),
                                    ("range", range_value(r.line, r.col, r.len)),
                                ]),
                            ),
                            ("message", Value::Str(r.message.clone())),
                        ])
                    })
                    .collect();
                pairs.push((
                    "relatedInformation",
                    Value::List(Rc::new(RefCell::new(related))),
                ));
            }
            mapv(pairs)
        })
        .collect();
    mapv(vec![
        ("uri", Value::Str(uri.into())),
        (
            "diagnostics",
            Value::List(std::rc::Rc::new(std::cell::RefCell::new(items))),
        ),
    ])
}

pub fn range_value(line: usize, col: usize, len: usize) -> Value {
    let pos = |l: Value, c: Value| mapv(vec![("line", l), ("character", c)]);
    mapv(vec![
        (
            "start",
            pos(Value::Int(line as i64), Value::Int(col as i64)),
        ),
        (
            "end",
            pos(Value::Int(line as i64), Value::Int((col + len) as i64)),
        ),
    ])
}

pub fn mapv(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(std::rc::Rc::new(std::cell::RefCell::new(
        pairs
            .into_iter()
            .map(|(k, v)| (Value::Str(k.into()), v))
            .collect(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = "\
gene boost(x) { return x }
enhance boost;
gene caller() { return boost(1) + missing(2) }
splice alt { variant a { return 1 } variant b { return 2 } }
main { let wobbles = caller() }
";

    #[test]
    fn analyze_collects_genes_splices_and_diagnostics() {
        let doc = analyze(DEMO);
        let names: Vec<&str> = doc.genes.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"boost") && names.contains(&"caller"));
        assert_eq!(doc.splices.len(), 1);
        assert_eq!(doc.splices[0].root, "alt");
        assert_eq!(doc.splices[0].variants, vec!["a", "b"]);
        // 'missing' is a phantom (called, never defined)
        assert!(
            doc.diagnostics
                .iter()
                .any(|d| d.severity == 1 && d.message.contains("missing")),
            "phantom diagnostic expected: {:?}",
            doc.diagnostics
        );
        // every diagnostic is either a labeled parser note or a phantom call
        assert!(doc
            .diagnostics
            .iter()
            .all(|d| d.message.contains("rung") || d.message.contains("phantom")));
    }

    #[test]
    fn hover_gives_gene_signature() {
        let doc = analyze(DEMO);
        // line 0: "gene boost(x) { return x }" — hover the name at col 6
        let h = hover(DEMO, &doc, 0, 6).expect("hover on gene def");
        assert!(h.contains("gene boost(x)"), "{}", h);
        assert!(h.contains("enhance"), "marks should be visible: {}", h);
        // hover the splice root on its line
        let sp = hover(DEMO, &doc, 3, 7).expect("hover on splice");
        assert!(sp.contains("splice alt"), "{}", sp);
        // hover a builtin
        let b = hover(
            "main { let q = random() }",
            &analyze("main { let q = random() }"),
            0,
            17,
        );
        assert!(
            b.as_deref()
                .map(|s| s.contains("built-in"))
                .unwrap_or(false),
            "{:?}",
            b
        );
    }

    #[test]
    fn hover_misses_are_none() {
        let doc = analyze(DEMO);
        assert_eq!(hover(DEMO, &doc, 0, 0), None); // on the `gene` keyword
        assert_eq!(hover(DEMO, &doc, 100, 0), None); // out of range
    }

    #[test]
    fn diagnostics_line_is_zero_based_with_span() {
        let src = "gene g() { retrn 1 }\n";
        let doc = analyze(src);
        // `retrn` is a wobble for `return` (rung 3) → warning severity 2
        let d = doc
            .diagnostics
            .iter()
            .find(|d| d.message.contains("return"))
            .expect("wobble diagnostic expected");
        assert_eq!(d.line, 0);
        assert_eq!(d.severity, 2);
        assert_eq!(
            src.lines().next().unwrap()[d.col..d.col + d.len].to_string(),
            "retrn"
        );
    }

    // ---- W45: references ---------------------------------------------------

    #[test]
    fn references_find_all_word_boundaries() {
        let src = "\
gene boost(x) { return x }
main { let y = boost(1) + boost(2) }
# boost mentioned in a comment
main { let z = \"boost in a string\" }
";
        let doc = analyze(src);
        // cursor on the declaration (line 0, col 6)
        let refs = references(src, &doc, 0, 6).expect("references for boost");
        // 2 on line 1 + 1 declaration on line 0; comment + string mentions excluded
        assert_eq!(refs.len(), 3, "{:?}", refs);
        assert!(refs.contains(&(0, 5, 5))); // declaration (`gene boost`)
        assert!(refs.contains(&(1, 15, 5))); // first call
        assert!(refs.contains(&(1, 26, 5))); // second call
                                             // unknown word → empty list, not None (None = cursor not on a word)
        let none = references(src, &doc, 0, 0).expect("cursor on `gene` keyword");
        assert!(none.iter().all(|&(l, _, _)| l == 0)); // only the keyword itself
    }

    #[test]
    fn references_none_off_word() {
        let doc = analyze(DEMO);
        assert_eq!(references(DEMO, &doc, 0, 300), None); // past EOL
    }

    // ---- W45: semantic tokens ----------------------------------------------

    #[test]
    fn semantic_tokens_classify_and_delta_encode() {
        let src = "gene boost(x) {\n  let y = \"hi\" + 42\n}\n";
        let doc = analyze(src);
        let v = semantic_tokens(src, &doc);
        let items: Vec<i64> = match &v {
            Value::List(l) => l
                .borrow()
                .iter()
                .map(|x| match x {
                    Value::Int(i) => *i,
                    _ => panic!("non-int in data"),
                })
                .collect(),
            _ => panic!("not a list"),
        };
        // quints: [dl, dc, len, ty, 0] — line 0 starts with keyword `gene`
        assert_eq!(&items[0..5], &[0, 0, 4, 0, 0]); // `gene` keyword, len 4
        assert_eq!(&items[5..10], &[0, 5, 5, 1, 0]); // `boost` function
                                                     // line 1: `let y = "hi" + 42` → dl=1 keyword, variable, string, number
        let line1 = &items[15..]; // after 3 tokens on line 0 (gene/boost/x)
        assert_eq!(&line1[0..5], &[1, 2, 3, 0, 0]); // `let`
        assert!(line1.chunks(5).any(|q| q[3] == 3), "string token expected");
        assert!(line1.chunks(5).any(|q| q[3] == 4), "number token expected");
    }

    #[test]
    fn semantic_tokens_comment_and_mark() {
        let src = "## doc comment\n@methylate gene g() {}\n";
        let doc = analyze(src);
        let v = semantic_tokens(src, &doc);
        let items: Vec<i64> = match &v {
            Value::List(l) => l
                .borrow()
                .iter()
                .map(|x| match x {
                    Value::Int(i) => *i,
                    _ => panic!("non-int"),
                })
                .collect(),
            _ => panic!("not a list"),
        };
        // first token: the whole doc comment, type 5
        assert_eq!(&items[0..5], &[0, 0, 14, 5, 0]);
        // `@methylate` is a keyword-class mark on line 1
        assert_eq!(&items[5..10], &[1, 0, 10, 0, 0]);
    }

    // ---- W46: repair provenance ---------------------------------------------

    #[test]
    fn wobble_diagnostic_carries_provenance() {
        let src = "gene g() { retrn 1 }\n";
        let doc = analyze(src);
        let d = doc
            .diagnostics
            .iter()
            .find(|d| !d.related.is_empty())
            .expect("wobble carries related");
        assert_eq!(
            d.related[0].message,
            "interpreted as 'return' (wobble rung)"
        );
        assert_eq!(d.related[0].line, 0);
    }

    #[test]
    fn hover_on_repaired_token_shows_provenance() {
        let src = "gene g() { retrn 1 }\n";
        let doc = analyze(src);
        // hover the `retrn` token (cols 11..16)
        let h = hover(src, &doc, 0, 13).expect("hover on repaired token");
        assert!(h.contains("retrn"), "{}", h);
        assert!(h.contains("repair"), "{}", h);
        assert!(h.contains("return"), "{}", h);
    }

    #[test]
    fn phantom_has_no_provenance() {
        let doc = analyze(DEMO);
        let ph = doc
            .diagnostics
            .iter()
            .find(|d| d.message.contains("phantom"))
            .expect("phantom present");
        assert!(ph.related.is_empty());
    }

    #[test]
    fn canonical_from_covers_all_repair_phrasings() {
        assert_eq!(
            canonical_from("synonym 'var' repaired to 'let'").as_deref(),
            Some("let")
        );
        assert_eq!(
            canonical_from("wobble: 'retrn' repaired to keyword 'return'").as_deref(),
            Some("return")
        );
        assert_eq!(
            canonical_from("wobble: '@methlated' repaired to '@methylate'").as_deref(),
            Some("methylate")
        );
        assert_eq!(canonical_from("unmatched '}' skipped"), None);
    }
}
