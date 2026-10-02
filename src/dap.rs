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
//! stackTrace/scopes/variables/evaluate/setVariable →
//! continue/next/stepIn/stepOut → terminated/exited. Program print output
//! is delivered as DAP output events (category "stdout") — the adapter's
//! stdout carries protocol frames only.
//!
//! W008 polish: stopOnEntry launch argument (one-shot stopped event with
//! reason "entry" at the program's first statement); per-breakpoint
//! `condition` fields (conditional breakpoints, evaluated in the stopped
//! frame); setVariable (assignment with the language's own reach — rebinds
//! where the name lives on the frame chain, consts stay frozen); stackTrace
//! reports real CALL-SITE lines for outer frames (stamped into call_stack
//! at push time), not placeholder line 1.
//!
//! Remaining honest limits: variables are rendered strings (no child
//! references — setVariable targets top-level frame bindings); no
//! logPoints/hitConditions.

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
    /// W008 polish: the live env each rendered scope came from, so
    /// setVariable can rebind the real frame state (cleared at every stop).
    scope_envs: HashMap<i64, Rc<Env>>,
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
        scope_envs: HashMap::new(),
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

fn req_bool(args: &Option<crate::value::Value>, key: &str) -> Option<bool> {
    match args {
        Some(crate::value::Value::Map(m)) => {
            for (k, v) in m.borrow().items.iter() {
                if let crate::value::Value::Str(s) = k {
                    if s == key {
                        if let crate::value::Value::Bool(b) = v {
                            return Some(*b);
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
                         \"supportsSetVariable\":true,\"supportsConditionalBreakpoints\":true}}"
                            .to_string(),
                    ),
                );
                send_event("initialized", None);
            }
            "launch" => {
                // W008 polish: stopOnEntry arms a one-shot entry stop at the
                // program's first statement (reason "entry")
                if req_bool(&args, "stopOnEntry").unwrap_or(false) {
                    interp.debug_stop_entry = true;
                }
                send_response(seq, true, "launch", None);
            }
            "setBreakpoints" => {
                let mut bps: Vec<(usize, Option<String>)> = Vec::new();
                if let Some(crate::value::Value::Map(m)) = &args {
                    for (k, v) in m.borrow().items.iter() {
                        if let crate::value::Value::Str(s) = k {
                            if s == "breakpoints" {
                                if let crate::value::Value::List(list) = v {
                                    for bp in list.borrow().iter() {
                                        if let crate::value::Value::Map(bm) = bp {
                                            let mut ln: Option<usize> = None;
                                            let mut cond: Option<String> = None;
                                            for (bk, bv) in bm.borrow().items.iter() {
                                                if let crate::value::Value::Str(bks) = bk {
                                                    match (bks.as_str(), bv) {
                                                        ("line", crate::value::Value::Int(n)) => {
                                                            ln = Some(*n as usize)
                                                        }
                                                        (
                                                            "condition",
                                                            crate::value::Value::Str(c),
                                                        ) => cond = Some(c.clone()),
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            if let Some(n) = ln {
                                                bps.push((n, cond));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                interp.debug_breaks = bps.iter().map(|(l, c)| (*l, c.clone())).collect();
                let rows: Vec<String> = bps
                    .iter()
                    .map(|(l, c)| match c {
                        Some(c) => format!(
                            "{{\"verified\":true,\"line\":{},\"condition\":{}}}",
                            l,
                            json_quote(c)
                        ),
                        None => format!("{{\"verified\":true,\"line\":{}}}", l),
                    })
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
    // snapshot the call stack for stackTrace/scopes/variables. W008 polish:
    // every frame gets a real line — the innermost frame is at the stop
    // line; an outer frame is parked at the call site that invoked the
    // frame one level deeper (the deeper frame's stamped call-site line).
    let n = interp.call_stack.len();
    let frames: Vec<(String, usize)> = interp
        .call_stack
        .iter()
        .rev()
        .enumerate()
        .map(|(i, (name, _, _, _))| {
            let line = if i == 0 {
                shown
            } else {
                interp.call_stack[n - i].3
            };
            (name.clone(), line)
        })
        .collect();
    with_state(|st| {
        st.frames = frames;
        st.stop_line = shown;
        st.var_tables.clear();
        st.scope_envs.clear();
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
                        // W008 polish: remember the live env so setVariable
                        // rebinds the real frame state
                        st.scope_envs.insert(r, env.clone());
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
            "setVariable" => {
                // W008 polish: assignment with the language's own reach —
                // the name rebinds where it is found on the frame chain;
                // consts are refused (frozen) and unknown names are refused
                // (no silent creation). The rendered table is refreshed so a
                // following `variables` shows the new value.
                let r = req_int(&args, "variablesReference").unwrap_or(0);
                let name = req_str(&args, "name").unwrap_or_default();
                let value = req_str(&args, "value").unwrap_or_default();
                let env_for_scope = with_state(|st| st.scope_envs.get(&r).cloned());
                match env_for_scope {
                    Some(scope_env) => match interp.debug_assign(&scope_env, &name, &value) {
                        Ok(d) => {
                            with_state(|st| {
                                if let Some(rows) = st.var_tables.get_mut(&r) {
                                    for row in rows.iter_mut() {
                                        if row.0 == name {
                                            row.1 = d.clone();
                                        }
                                    }
                                }
                            });
                            send_response(
                                seq,
                                true,
                                "setVariable",
                                Some(format!("{{\"value\":{}}}", json_quote(&d))),
                            );
                        }
                        Err(e) => send_response(
                            seq,
                            false,
                            "setVariable",
                            Some(format!("{{\"error\":{}}}", json_quote(&e))),
                        ),
                    },
                    None => send_response(
                        seq,
                        false,
                        "setVariable",
                        Some("{\"error\":\"unknown variablesReference\"}".to_string()),
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
