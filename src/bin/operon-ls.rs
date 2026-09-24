//! operon-ls — Operon language server (G5 seed): stdio LSP.
//!
//! Minimal but real: initialize / shutdown / exit, full-text document sync,
//! publishDiagnostics (Total Grammar parse notes + `tools check` phantoms),
//! and textDocument/hover with gene signatures.
//!
//! Protocol framing: Content-Length headers over stdio (LSP standard).
//! The seed keeps the dependency surface at zero — request JSON is parsed
//! with the language's own `json_parse`, responses are built as `Value`
//! maps and serialized with `json_stringify` (the core eats its own food).
//!
//! A malformed frame or stream end closes the session.

use operon::interp::{json_parse, json_stringify};
use operon::ls::{analyze, hover, mapv, publish_params};
use operon::value::Value;
use std::collections::HashMap;
use std::io::{Read, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut docs: HashMap<String, String> = HashMap::new();

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
                                    // didChange carries contentChanges[] with .text
                                    get(p, "contentChanges").and_then(|c| match c {
                                        Value::List(items) => {
                                            items.borrow().last().and_then(|v| get_str(v, "text"))
                                        }
                                        _ => None,
                                    })
                                })
                                .unwrap_or_default();
                            docs.insert(uri.clone(), text);
                            let src = docs.get(&uri).unwrap().clone();
                            let doc = analyze(&src);
                            notify(
                                &mut stdout,
                                "textDocument/publishDiagnostics",
                                publish_params(&uri, &doc.diagnostics),
                            );
                        }
                    }
                }
            }
            "textDocument/hover" => {
                let mut response: Option<Value> = None;
                if let Some(p) = &params {
                    let uri = get(p, "textDocument")
                        .and_then(|td| get(&td, "uri"))
                        .and_then(|v| get_str_s(&v));
                    let pos = get(p, "position");
                    let line = pos
                        .as_ref()
                        .and_then(|p| get(p, "line"))
                        .and_then(|v| as_int(&v));
                    let ch = pos
                        .as_ref()
                        .and_then(|p| get(p, "character"))
                        .and_then(|v| as_int(&v));
                    if let (Some(uri), Some(line), Some(ch)) = (uri, line, ch) {
                        if let Some(src) = docs.get(&uri) {
                            let doc = analyze(src);
                            if let Some(md) = hover(src, &doc, line as usize, ch as usize) {
                                response = Some(mapv(vec![
                                    (
                                        "contents",
                                        mapv(vec![
                                            ("kind", Value::Str("markdown".into())),
                                            ("value", Value::Str(md)),
                                        ]),
                                    ),
                                    ("range", Value::Null),
                                ]));
                            }
                        }
                    }
                }
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

// ---- framing ------------------------------------------------------------

fn read_message(input: &mut impl Read) -> Result<String, ()> {
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

fn get_str_s(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Float(f) => Some(*f as i64),
        _ => None,
    }
}
