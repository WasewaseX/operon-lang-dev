//! operon-ls — Operon language server (G5 seed → lsp-r1 v2): stdio LSP.
//!
//! Features: initialize / shutdown / exit, full-text document sync (ranged
//! edits are ignored, not mis-applied), publishDiagnostics (Total Grammar
//! parse notes + `tools check` phantoms — resolved CWD-independently since
//! lsp-r1), textDocument/hover with gene signatures, textDocument/definition,
//! textDocument/documentSymbol, textDocument/completion, and
//! textDocument/formatting (the canonical `operon fmt` engine).
//!
//! Protocol framing: Content-Length headers over stdio (LSP standard).
//! The dependency surface stays at zero — request JSON is parsed with the
//! language's own `json_parse`, responses are built as `Value` maps and
//! serialized with `json_stringify` (the core eats its own food).
//!
//! A malformed frame or stream end closes the session.

use operon::interp::{json_parse, json_stringify};
use operon::ls::{
    analyze_doc, completions, definition, document_symbols, format_text, hover, mapv,
    publish_params, range_value, LsDoc,
};
use operon::value::Value;
use std::collections::HashMap;
use std::io::{Read, Write};

/// lsp-r1: per-document state — source text AND its analysis, so hover /
/// definition / symbols / completion answer from the cached LsDoc instead of
/// re-parsing twice per request (the seed re-analyzed on every hover).
struct DocEntry {
    src: String,
    analyzed: LsDoc,
}

fn main() {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut docs: HashMap<String, DocEntry> = HashMap::new();

    while let Ok(body) = read_message(&mut input) {
        let msg = match json_parse(&body) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = get_str(&msg, "method").unwrap_or_default();
        let id = get(&msg, "id");
        let params = get(&msg, "params");
        match method.as_str() {
            "initialize" => {
                let result = mapv(vec![
                    (
                        "capabilities",
                        mapv(vec![
                            ("textDocumentSync", Value::Int(1)), // full sync
                            ("hoverProvider", Value::Bool(true)),
                            ("definitionProvider", Value::Bool(true)),
                            ("documentSymbolProvider", Value::Bool(true)),
                            ("documentFormattingProvider", Value::Bool(true)),
                            (
                                "completionProvider",
                                mapv(vec![("resolveProvider", Value::Bool(false))]),
                            ),
                        ]),
                    ),
                    (
                        "serverInfo",
                        mapv(vec![
                            ("name", Value::Str("operon-ls".into())),
                            ("version", Value::Str(env!("CARGO_PKG_VERSION").into())),
                        ]),
                    ),
                ]);
                send(&mut stdout, id.unwrap_or(Value::Null), Some(result), &None);
            }
            "initialized" => {} // notification, nothing to do
            "shutdown" => {
                send(&mut stdout, id.unwrap_or(Value::Null), None, &None);
            }
            "exit" => break,
            "textDocument/didOpen" | "textDocument/didChange" => {
                if let Some(p) = &params {
                    if let Some(td) = get(p, "textDocument") {
                        if let Some(uri) = get_str(&td, "uri") {
                            let text = get_str(p, "text")
                                .or_else(|| {
                                    // didChange carries contentChanges[] with .text.
                                    // lsp-r1: a RANGED change (sync=1 means none
                                    // should arrive) used to silently replace the
                                    // WHOLE document with the fragment — corrupting
                                    // editor state. Ranged changes are ignored;
                                    // the last full-text change wins.
                                    get(p, "contentChanges").and_then(|c| match c {
                                        Value::List(items) => items
                                            .borrow()
                                            .iter()
                                            .rev()
                                            .find(|v| get(v, "range").is_none())
                                            .and_then(|v| get_str(v, "text")),
                                        _ => None,
                                    })
                                })
                                .unwrap_or_default();
                            let base = doc_base_dir(&uri);
                            let analyzed = analyze_doc(&text, base.as_deref());
                            let diags = analyzed.diagnostics.clone();
                            docs.insert(
                                uri.clone(),
                                DocEntry {
                                    src: text,
                                    analyzed,
                                },
                            );
                            notify(
                                &mut stdout,
                                "textDocument/publishDiagnostics",
                                publish_params(&uri, &diags),
                            );
                        }
                    }
                }
            }
            "textDocument/didClose" => {
                // lsp-r1: the seed never dropped the doc — stale state and
                // unbounded memory. Per convention: forget it, clear diagnostics.
                if let Some(p) = &params {
                    if let Some(uri) = get(p, "textDocument").and_then(|td| get_str(&td, "uri")) {
                        docs.remove(&uri);
                        notify(
                            &mut stdout,
                            "textDocument/publishDiagnostics",
                            publish_params(&uri, &[]),
                        );
                    }
                }
            }
            "textDocument/hover" => {
                let response = with_doc_position(&docs, &params, |src, doc, line, ch| {
                    hover(src, doc, line, ch).map(|md| {
                        mapv(vec![
                            (
                                "contents",
                                mapv(vec![
                                    ("kind", Value::Str("markdown".into())),
                                    ("value", Value::Str(md)),
                                ]),
                            ),
                            ("range", Value::Null),
                        ])
                    })
                });
                send(&mut stdout, id.unwrap_or(Value::Null), response, &None);
            }
            "textDocument/definition" => {
                let uri = doc_uri(&params);
                let response = with_doc_position(&docs, &params, |src, doc, line, ch| {
                    definition(src, doc, line, ch).map(|(l, c, len)| {
                        mapv(vec![
                            ("uri", Value::Str(uri.clone())),
                            ("range", range_value(l, c, len)),
                        ])
                    })
                });
                send(&mut stdout, id.unwrap_or(Value::Null), response, &None);
            }
            "textDocument/documentSymbol" => {
                let response = docs
                    .get(&doc_uri(&params))
                    .map(|e| document_symbols(&e.src, &e.analyzed));
                send(&mut stdout, id.unwrap_or(Value::Null), response, &None);
            }
            "textDocument/completion" => {
                let response = docs.get(&doc_uri(&params)).map(|e| {
                    Value::List(std::rc::Rc::new(std::cell::RefCell::new(completions(
                        &e.src,
                        &e.analyzed,
                    ))))
                });
                send(&mut stdout, id.unwrap_or(Value::Null), response, &None);
            }
            "textDocument/formatting" => {
                let response = docs.get(&doc_uri(&params)).and_then(|e| {
                    format_text(&e.src).map(|new_text| {
                        // one full-document TextEdit: (0,0) → end of last line
                        let last_line = e.src.lines().count().saturating_sub(1);
                        let end_col = e.src.lines().last().map(|l| l.chars().count()).unwrap_or(0);
                        full_doc_edit(last_line, end_col, new_text)
                    })
                });
                send(&mut stdout, id.unwrap_or(Value::Null), response, &None);
            }
            _ => {
                // unknown request (has id) → method not found; unknown
                // notifications are dropped silently per the spec
                if let Some(id) = id {
                    let err = mapv(vec![
                        ("code", Value::Int(-32601)),
                        (
                            "message",
                            Value::Str(format!("method not found: {}", method)),
                        ),
                    ]);
                    send(&mut stdout, id, None, &Some(err));
                }
            }
        }
    }
}

/// lsp-r1: full-document TextEdit — range (0,0)..(last_line, last_line_len)
/// plus the replacement text.
fn full_doc_edit(last_line: usize, last_col: usize, new_text: String) -> Value {
    Value::List(std::rc::Rc::new(std::cell::RefCell::new(vec![mapv(vec![
        ("range", end_range(last_line, last_col)),
        ("newText", Value::Str(new_text)),
    ])])))
}

fn end_range(line: usize, col: usize) -> Value {
    let pos = |l: usize, c: usize| {
        mapv(vec![
            ("line", Value::Int(l as i64)),
            ("character", Value::Int(c as i64)),
        ])
    };
    mapv(vec![("start", pos(0, 0)), ("end", pos(line, col))])
}

/// The document URI from request params.
fn doc_uri(params: &Option<Value>) -> String {
    params
        .as_ref()
        .and_then(|p| get(p, "textDocument"))
        .and_then(|td| get_str(&td, "uri"))
        .unwrap_or_default()
}

/// Run `f` against the cached (src, analyzed) doc at the request position.
fn with_doc_position(
    docs: &HashMap<String, DocEntry>,
    params: &Option<Value>,
    f: impl Fn(&str, &LsDoc, usize, usize) -> Option<Value>,
) -> Option<Value> {
    let uri = doc_uri(params);
    let p = params.as_ref()?;
    let pos = get(p, "position")?;
    let line = get(&pos, "line").and_then(|v| as_int(&v))? as usize;
    let ch = get(&pos, "character").and_then(|v| as_int(&v))? as usize;
    let e = docs.get(&uri)?;
    f(&e.src, &e.analyzed, line, ch)
}

/// lsp-r1 (P0): the directory of the document on disk, for document-relative
/// `use`-module resolution. `file:///a/b/c.op` → `/a/b` (best effort).
fn doc_base_dir(uri: &str) -> Option<String> {
    let path = uri
        .strip_prefix("file://")
        .map(|p| p.to_string())
        .unwrap_or_else(|| uri.to_string());
    let decoded = percent_decode(&path);
    std::path::PathBuf::from(decoded)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
}

/// Minimal %XX decoding for file URIs (spaces especially).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 && i + 2 < b.len() + 1 {
            let hex = |c: u8| -> Option<u8> {
                match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    _ => None,
                }
            };
            if i + 2 < b.len() {
                if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                    out.push(h * 16 + l);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

// ---- framing ------------------------------------------------------------

fn read_message(input: &mut impl Read) -> Result<String, ()> {
    // sec-r1 (audit C-4): Content-Length is attacker-controlled from the
    // editor side. `vec![0u8; len]` with an unbounded len panicked on
    // capacity overflow (rc=101, killing the session) and reserved gigabytes
    // for large-but-valid values before one body byte arrived. Cap the frame.
    const MAX_FRAME: usize = 64 * 1024 * 1024;
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    // read until \r\n\r\n
    loop {
        match input.read(&mut byte) {
            Ok(0) => return Err(()),
            Ok(_) => {
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
                if header.len() > 16 * 1024 {
                    return Err(());
                }
            }
            Err(_) => return Err(()),
        }
    }
    let text = String::from_utf8_lossy(&header);
    let len = text
        .lines()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("Content-Length:")
                .map(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(None)
        })
        .ok_or(())?;
    if len > MAX_FRAME {
        return Err(());
    }
    let mut body = vec![0u8; len];
    input.read_exact(&mut body).map_err(|_| ())?;
    String::from_utf8(body).map_err(|_| ())
}

fn send(out: &mut impl Write, id: Value, result: Option<Value>, error: &Option<Value>) {
    let mut pairs = vec![("jsonrpc", Value::Str("2.0".into())), ("id", id)];
    // LSP: a successful response always carries `result` — null on purpose
    pairs.push(("result", result.unwrap_or(Value::Null)));
    if let Some(e) = error {
        pairs.push(("error", e.clone()));
    }
    write_frame(out, mapv(pairs));
}

fn notify(out: &mut impl Write, method: &str, params: Value) {
    write_frame(
        out,
        mapv(vec![
            ("jsonrpc", Value::Str("2.0".into())),
            ("method", Value::Str(method.into())),
            ("params", params),
        ]),
    );
}

fn write_frame(out: &mut impl Write, v: Value) {
    let body = json_stringify(&v);
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

// ---- Value digging ------------------------------------------------------

fn get(v: &Value, key: &str) -> Option<Value> {
    match v {
        Value::Map(m) => m
            .borrow()
            .iter()
            .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
            .map(|(_, val)| val.clone()),
        _ => None,
    }
}

fn get_str(v: &Value, key: &str) -> Option<String> {
    get(v, key).and_then(|x| match x {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    })
}

fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Float(f) => Some(*f as i64),
        _ => None,
    }
}
