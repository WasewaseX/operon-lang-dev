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
    Diagnostic {
        line: line0,
        col,
        len,
        severity: note_severity(n.rung),
        message: format!("{} ({} rung)", n.message, rung_label(n.rung)),
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

/// Hover text (markdown) for the word under the cursor, per D-008:
/// programmer-first, gene vocabulary as intuition aid, never a prerequisite.
pub fn hover(src: &str, doc: &LsDoc, line0: usize, col0: usize) -> Option<String> {
    let line = src.lines().nth(line0)?;
    let (_, word) = word_at(line, col0)?;

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
Calls may be gated by GRN thresholds, methylation, or toggle state (SPEC §11).{}",
            sig, def
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
            "```operon\n{}()\n```\n\nbuilt-in gene (SPEC §10).",
            word
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
pub fn publish_params(uri: &str, diags: &[Diagnostic]) -> Value {
    let items: Vec<Value> = diags
        .iter()
        .map(|d| {
            mapv(vec![
                ("range", range_value(d.line, d.col, d.len)),
                ("severity", Value::Int(d.severity as i64)),
                ("source", Value::Str("operon-check".into())),
                ("message", Value::Str(d.message.clone())),
            ])
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
}
