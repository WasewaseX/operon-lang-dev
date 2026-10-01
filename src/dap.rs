//! dap.rs — W08r stage 3: the Debug Adapter Protocol adapter.
//!
//! `operon dap f.op` speaks the industry-standard Debug Adapter Protocol
//! (DAP) on stdio using the Content-Length base protocol, so editors and
//! IDEs (VS Code and anything else with a DAP client) can debug Operon
//! programs natively. The adapter drives the same interpreter debug hooks
//! as the human REPL and the NDJSON machine protocol: the trap in
//! exec_block calls [`serve_at_trap`], which emits a stopped event and
//! serves requests until a resume request (continue/next/stepIn/stepOut)
//! arrives.
//!
//! Supported lifecycle: initialize → (initialized event) → launch →
//! setBreakpoints → configurationDone → run → stopped events →
//! stackTrace/scopes/variables/evaluate → continue/next/stepIn/stepOut →
//! terminated/exited. Program print output is delivered as DAP output
//! events (category "stdout") — the adapter's stdout carries protocol
//! frames only.
//!
//! v1 scope notes (honest): stackTrace reports accurate lines for the
//! innermost frame (outer frames carry their name but line 1 — call-site
//! lines live in the stress traceback machinery, not in call_stack yet);
//! variables are rendered strings (no child references, no setVariable).

use crate::interp::Env;
use crate::interp::{json_quote, Interp};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::rc::Rc;

/// Per-session adapter state. The host interpreter is single-threaded
/// (workers/fibers never trap: the debug hooks are host-only), so a
/// thread_local is the safe home for the state the trap hook shares with
/// the launch path in main.rs.
struct DapState {
    seq: i64,
    next_var_ref: i64,
    var_tables: HashMap<i64, Vec<(String, String)>>,
    /// (name, line) innermost-first snapshot of the call stack at the stop
    frames: Vec<(String, usize)>,
    stop_line: usize,
}

fn with_state<T>(f: impl FnOnce(&mut DapState) -> T) -> T {
    DAP.with(|c| {
        let mut st = c.borrow_mut();
        f(&mut st)
    })
}

thread_local! {
    static DAP: RefCell<DapState> = RefCell::new(DapState {
        seq: 0,
        next_var_ref: 1000,
        var_tables: HashMap::new(),
        frames: Vec::new(),
        stop_line: 0,
    });
}

// ------------------------------------------------------------- framing

/// Read one base-protocol message (Content-Length framing). None = EOF.
fn read_message() -> Option<String> {
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut headers = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        match lock.read(&mut buf) {
            Ok(0) => return None,
            Ok(_) => headers.push(buf[0]),
            Err(_) => return None,
        }
        if headers.ends_with(b"\r\n\r\n") {
            break;
        }
        // tolerate bare-\n framing from test clients
        if headers.ends_with(b"\n\n") {
            break;
        }
        if headers.len() > 16 * 1024 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&headers);
    let len: usize = head
        .lines()
        .find_map(|l| {
            l.strip_prefix("Content-Length:")
                .or_else(|| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
        })
        .or_else(|| {
            head.lines().find_map(|l| {
                l.strip_prefix("Content-Length:")
                    .and_then(|v| v.trim().parse().ok())
            })
        })?;
    let mut body = vec![0u8; len];
    lock.read_exact(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// Write one base-protocol message. The byte count (not the char count) is
/// what the framing demands: multibyte payloads (CJK identifiers) are legal.
fn send_raw(s: &str) {
    let out = std::io::stdout();
    let mut lock = out.lock();
    // s.len() is the BYTE count (what the framing demands): multibyte
    // payloads (CJK identifiers) are legal and counted correctly
    let _ = write!(lock, "Content-Length: {}\r\n\r\n{}", s.len(), s);
    let _ = lock.flush();
}

fn next_seq() -> i64 {
    with_state(|st| {
        st.seq += 1;
        st.seq
    })
}

fn send_event(event: &str, body: Option<String>) {
    let seq = next_seq();
    let b = body.map(|x| format!(",\"body\":{}", x)).unwrap_or_default();
    send_raw(&format!(
        "{{\"seq\":{},\"type\":\"event\",\"event\":\"{}\"{}}}",
        seq, event, b
    ));
}

fn send_response(req_seq: i64, success: bool, command: &str, body: Option<String>) {
    let seq = next_seq();
    let b = body.map(|x| format!(",\"body\":{}", x)).unwrap_or_default();
    send_raw(&format!(
        "{{\"seq\":{},\"type\":\"response\",\"request_seq\":{},\"success\":{},\"command\":\"{}\"{}}}",
        seq, req_seq, success, command, b
    ));
}

fn req_int(args: &Option<crate::value::Value>, key: &str) -> Option<i64> {
    match args {
        Some(crate::value::Value::Map(m)) => {
            for (k, v) in m.borrow().items.iter() {
                if let crate::value::Value::Str(s) = k {
                    if s == key {
                        if let crate::value::Value::Int(i) = v {
                            return Some(*i);
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn req_str(args: &Option<crate::value::Value>, key: &str) -> Option<String> {
    match args {
        Some(crate::value::Value::Map(m)) => {
            for (k, v) in m.borrow().items.iter() {
                if let crate::value::Value::Str(s) = k {
                    if s == key {
                        if let crate::value::Value::Str(t) = v {
                            return Some(t.clone());
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

// ------------------------------------------------------------- lifecycle

/// The pre-run configuration phase: serve initialize/launch/setBreakpoints/
/// configurationDone until the client says go. Breakpoints land directly in
/// the interpreter's set.
pub fn configure(interp: &mut Interp) {
    while let Some(msg) = read_message() {
        let req = match crate::interp::json_parse(&msg) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let seq = req_int(&Some(req.clone()), "seq").unwrap_or(0);
        let command = req_str(&Some(req.clone()), "command").unwrap_or_default();
        let args = crate::interp::Interp::jfield_pub(&req, "arguments");
        match command.as_str() {
            "initialize" => {
                send_response(
                    seq,
                    true,
                    "initialize",
                    Some(
                        "{\"capabilities\":{\"supportsConfigurationDoneRequest\":true,\
                         \"supportsEvaluateForHovers\":true,\"supportsTerminateRequest\":true,\
                         \"supportsSetVariable\":false,\"supportsConditionalBreakpoints\":false}}"
                            .to_string(),
                    ),
                );
                send_event("initialized", None);
            }
            "launch" => {
                send_response(seq, true, "launch", None);
            }
            "setBreakpoints" => {
                let mut lines: Vec<usize> = Vec::new();
                if let Some(crate::value::Value::Map(m)) = &args {
                    for (k, v) in m.borrow().items.iter() {
                        if let crate::value::Value::Str(s) = k {
                            if s == "breakpoints" {
                                if let crate::value::Value::List(list) = v {
                                    for bp in list.borrow().iter() {
                                        if let crate::value::Value::Map(bm) = bp {
                                            for (bk, bv) in bm.borrow().items.iter() {
                                                if let crate::value::Value::Str(bks) = bk {
                                                    if bks == "line" {
                                                        if let crate::value::Value::Int(n) = bv {
                                                            lines.push(*n as usize);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                interp.debug_breaks = lines.iter().copied().collect();
                let rows: Vec<String> = lines
                    .iter()
                    .map(|l| format!("{{\"verified\":true,\"line\":{}}}", l))
                    .collect();
                send_response(
                    seq,
                    true,
                    "setBreakpoints",
                    Some(format!("{{\"breakpoints\":[{}]}}", rows.join(","))),
                );
            }
            "configurationDone" => {
                send_response(seq, true, "configurationDone", None);
                return;
            }
            "disconnect" | "terminate" => {
                send_response(seq, true, &command, None);
                send_event("terminated", None);
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(0);
            }
            other => {
                send_response(
                    seq,
                    false,
                    other,
                    Some("{\"error\":\"notSupportedDuringConfiguration\"}".into()),
                );
            }
        }
    }
}

// ------------------------------------------------------------- trap

/// The trap hook: called from exec_block at every stop while a DAP session
/// is running. Emits the stopped event, then serves requests until a resume
/// request arrives.
pub fn serve_at_trap(interp: &mut Interp, env: &Rc<Env>, stmt_line: Option<usize>, reason: &str) {
    let shown = stmt_line.unwrap_or(interp.cur_line);
    // snapshot the call stack for stackTrace/scopes/variables
    let frames: Vec<(String, usize)> = interp
        .call_stack
        .iter()
        .rev()
        .enumerate()
        .map(|(i, (name, _, _))| (name.clone(), if i == 0 { shown } else { 1 }))
        .collect();
    with_state(|st| {
        st.frames = frames;
        st.stop_line = shown;
        st.var_tables.clear();
    });
    send_event(
        "stopped",
        Some(format!(
            "{{\"reason\":\"{}\",\"threadId\":1,\"allThreadsStopped\":true}}",
            reason
        )),
    );
    // program print output captured since the last stop becomes output events
    drain_sink_to_events(interp);

    loop {
        let msg = match read_message() {
            Some(m) => m,
            None => {
                // client went away: resume to completion, never wedge
                interp.debug_step = false;
                interp.debug_step_depth = None;
                interp.debug_until = None;
                interp.debug_breaks.clear();
                return;
            }
        };
        let req = match crate::interp::json_parse(&msg) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let seq = req_int(&Some(req.clone()), "seq").unwrap_or(0);
        let command = req_str(&Some(req.clone()), "command").unwrap_or_default();
        let args = Interp::jfield_pub(&req, "arguments");
        match command.as_str() {
            "threads" => {
                send_response(
                    seq,
                    true,
                    "threads",
                    Some("{\"threads\":[{\"id\":1,\"name\":\"main\"}]}".to_string()),
                );
            }
            "stackTrace" => {
                let frames_json = with_state(|st| {
                    st.frames
                        .iter()
                        .enumerate()
                        .map(|(i, (name, line))| {
                            format!(
                                "{{\"id\":{},\"name\":{},\"line\":{},\"column\":1}}",
                                i,
                                json_quote(name),
                                line
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                });
                let total = with_state(|st| st.frames.len());
                send_response(
                    seq,
                    true,
                    "stackTrace",
                    Some(format!(
                        "{{\"stackFrames\":[{}],\"totalFrames\":{}}}",
                        frames_json, total
                    )),
                );
            }
            "scopes" => {
                let frame = req_int(&args, "frameId").unwrap_or(0);
                let rows = with_state(|st| {
                    let mut rows = Vec::new();
                    if let Some((name, _)) = st.frames.get(frame as usize) {
                        let mut vars = Vec::new();
                        // the frame env chain (innermost first, capped like the REPL)
                        let mut cur = Some(env.clone());
                        let mut depth = 0usize;
                        while let Some(e) = cur {
                            let vars_borrow = e.vars.borrow();
                            let mut names: Vec<String> = vars_borrow.keys().cloned().collect();
                            names.sort();
                            for nm in names {
                                let v = vars_borrow
                                    .get(&nm)
                                    .cloned()
                                    .unwrap_or(crate::value::Value::Null);
                                vars.push((nm, v.display()));
                            }
                            drop(vars_borrow);
                            depth += 1;
                            if depth >= 8 {
                                break;
                            }
                            cur = e.parent.clone();
                        }
                        st.next_var_ref += 1;
                        let r = st.next_var_ref;
                        st.var_tables.insert(r, vars);
                        rows.push(format!(
                            "{{\"name\":{},\"variablesReference\":{},\"expensive\":false}}",
                            json_quote(&format!("Locals — {}", name)),
                            r
                        ));
                    }
                    rows
                });
                send_response(
                    seq,
                    true,
                    "scopes",
                    Some(format!("{{\"scopes\":[{}]}}", rows.join(","))),
                );
            }
            "variables" => {
                let r = req_int(&args, "variablesReference").unwrap_or(0);
                let vars_json = with_state(|st| {
                    st.var_tables
                        .get(&r)
                        .map(|vars| {
                            vars.iter()
                                .map(|(n, v)| {
                                    format!(
                                        "{{\"name\":{},\"value\":{},\"variablesReference\":0}}",
                                        json_quote(n),
                                        json_quote(v)
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default()
                });
                send_response(
                    seq,
                    true,
                    "variables",
                    Some(format!("{{\"variables\":[{}]}}", vars_json)),
                );
            }
            "evaluate" => {
                let expr = req_str(&args, "expression").unwrap_or_default();
                match interp.debug_eval_display(env, &expr) {
                    Ok(v) => send_response(
                        seq,
                        true,
                        "evaluate",
                        Some(format!(
                            "{{\"result\":{},\"variablesReference\":0}}",
                            json_quote(&v)
                        )),
                    ),
                    Err(e) => send_response(
                        seq,
                        false,
                        "evaluate",
                        Some(format!("{{\"error\":{}}}", json_quote(&e))),
                    ),
                }
            }
            "continue" => {
                interp.debug_step = false;
                interp.debug_step_depth = None;
                send_response(
                    seq,
                    true,
                    "continue",
                    Some("{\"allThreadsContinued\":true}".to_string()),
                );
                return;
            }
            "next" => {
                interp.debug_step = true;
                interp.debug_step_depth = Some(interp.call_stack.len());
                send_response(seq, true, "next", None);
                return;
            }
            "stepIn" => {
                interp.debug_step = true;
                interp.debug_step_depth = None;
                send_response(seq, true, "stepIn", None);
                return;
            }
            "stepOut" => {
                interp.debug_step = true;
                interp.debug_step_depth = Some(interp.call_stack.len().saturating_sub(1));
                send_response(seq, true, "stepOut", None);
                return;
            }
            "disconnect" | "terminate" => {
                send_response(seq, true, &command, None);
                send_event("terminated", None);
                // ast-grep-ignore: no-std-process-exit-in-core
                std::process::exit(0);
            }
            other => {
                send_response(
                    seq,
                    false,
                    other,
                    Some("{\"error\":\"notSupported\"}".to_string()),
                );
            }
        }
    }
}

/// Program print output captured in stdout_sink becomes DAP output events
/// (the adapter's stdout is the protocol transport, never program output).
pub fn drain_sink_to_events(interp: &mut Interp) {
    if let Some(sink) = &interp.stdout_sink {
        let lines = std::mem::take(&mut *sink.borrow_mut());
        for l in lines {
            send_event(
                "output",
                Some(format!(
                    "{{\"category\":\"stdout\",\"output\":{}}}",
                    json_quote(&format!("{}\n", l))
                )),
            );
        }
    }
}

/// The lifecycle close after the program finishes: the exited event (with
/// the process exit code) and then terminated.
pub fn session_end(code: i64) {
    send_event("exited", Some(format!("{{\"exitCode\":{}}}", code)));
    send_event("terminated", None);
}
