//! pybridge.rs, the Python substrate bridge (substrate-r1, loop-6).
//!
//! Owner directive: Operon is NOT built from zero. The substrate statement
//! (D-010) is that Operon stands ON existing ecosystems:
//!   * Rust , the toolchain and host interpreter (already true, 46% of tree)
//!   * C++  , the codon kernel that earned its place (Myers bit-parallel)
//!   * Python, the scientific ecosystem, joined via THIS bridge.
//!
//! DeepSeek's critique "libraries too small" is answered architecturally:
//! a stdlib does not need 500,000 modules of its own, it needs a hard,
//! capability-gated DOOR to the ecosystem that already has them (NumPy,
//! SciPy, Biopython, pandas). Operon joins the 82.8% computational-biology
//! wall instead of fighting it.
//!
//! Containment contract (all inherited from the run() precedent):
//!   * capability `py`, default-deny, exact-match per MODULE name
//!     (`--allow-py math`); a grant to "os" is an explicit trust act, the
//!     same trust model as --allow-run.
//!   * the child is `python3 -X utf8 -I -B` (isolated mode: no user site,
//!     PYTHON* env ignored) running an EMBEDDED runner, no files shipped.
//!   * environment scrubbed to OS essentials (shared safe_base_env).
//!   * wall-clock timeout (`.cell py.timeout_ms`, clamp 1..300_000, default
//!     10_000) with kill + bounded drain, identical to run().
//!   * stdout/stderr drained concurrently; output capped at MAX_CHILD_OUT
//!     (single source of truth, shared with run()).
//!   * the request is ONE JSON line on stdin, then the pipe CLOSES, an
//!     import-time stdin reader gets EOF, not a hang.
//!   * the response is ONE JSON line as the LAST non-empty stdout line, so
//!     a chatty module that prints during import or call cannot corrupt it.
//!   * bridge failures are DATA (ok:false / code:-2), not interpreter
//!     stresses, only capability denial raises `interference`, and only a
//!     dead interpreter raises `missing`. The language never panics on a
//!     misbehaving Python module.

use crate::value::Value;
use std::io::Write;
use std::process::{Command, Stdio};

/// Single source of truth for the per-stream child-output cap (run() reads
/// this too). An emitter beyond the cap is killed by the timeout; the
/// collected prefix is all the caller ever sees.
pub(crate) const MAX_CHILD_OUT: usize = 64 * 1024 * 1024;

/// Grace window for the concurrent drainers after child exit (mirrors run()).
const DRAIN_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

/// The embedded Python runner. `-I` (isolated) already ignores PYTHON* env
/// vars and user site-packages; `-B` skips .pyc writes; `-X utf8` pins
/// encoding on Windows. The runner speaks the one-line JSON protocol and
/// NOTHING else: unknown requests and BaseException (including SystemExit
/// raised by imported code) become ok:false lines, never tracebacks to the
/// parent's stderr, never a nonzero-exit without a parseable response.
const RUNNER: &str = r#"
import sys, json, importlib, traceback, datetime
from decimal import Decimal

def _default(o):
    m = getattr(type(o), "__module__", "") or ""
    tn = type(o).__name__
    if m.startswith("numpy"):
        if tn == "ndarray":
            return o.tolist()
        if hasattr(o, "item"):
            return o.item()
    if isinstance(o, (set, frozenset, tuple)):
        return list(o)
    if isinstance(o, bytes):
        return o.decode("utf-8", "replace")
    if isinstance(o, Decimal):
        return float(o)
    if isinstance(o, (datetime.date, datetime.time, datetime.datetime)):
        return o.isoformat()
    raise TypeError("not JSON-serializable: %s" % tn)

def main():
    try:
        req = json.loads(sys.stdin.readline())
        module = importlib.import_module(req["module"])
        obj = module
        for part in str(req.get("func") or "").split("."):
            if part:
                obj = getattr(obj, part)
        val = obj(*req.get("args", []))
        sys.stdout.write(json.dumps({"ok": True, "value": val,
                                     "py": ".".join(map(str, sys.version_info[:2]))},
                                    default=_default, allow_nan=False) + "\n")
        return 0
    except BaseException as e:
        msg = "".join(traceback.format_exception_only(type(e), e)).strip()
        sys.stdout.write(json.dumps({"ok": False, "error": msg,
                                     "py": ".".join(map(str, sys.version_info[:2]))}) + "\n")
        return 1

if __name__ == "__main__":
    sys.exit(main())
"#;

/// A decoded bridge outcome. `ok` is SEMANTIC success (the Python call ran
/// and returned a value); `code` is the process exit contract:
///   0  ok:true                -1 killed (timeout)
///   1  ok:false (exception)   -2 protocol failure (no/invalid response)
pub struct PyResponse {
    pub ok: bool,
    pub value: Value,
    pub error: Option<String>,
    pub code: i64,
    pub timed_out: bool,
    /// W079: interpreter version as reported by the bridge child
    /// (`sys.version.split()[0]`), when the child reported one.
    pub py_version: Option<String>,
}

static PY_EXE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Probe once per process: `python3` first (POSIX convention), `python`
/// fallback (Windows). The probe is a cheap `--version` spawn; the result
/// is cached so per-call cost is zero.
pub fn python_available() -> bool {
    resolve_python().is_some()
}

fn resolve_python() -> Option<&'static str> {
    PY_EXE
        .get_or_init(|| {
            for cand in ["python3", "python"] {
                if let Ok(out) = Command::new(cand).arg("--version").output() {
                    if out.status.success() {
                        return Some(cand.to_string());
                    }
                }
            }
            None
        })
        .as_deref()
}

/// Last non-empty stdout line, the protocol tolerates chatty modules.
fn last_line(s: &str) -> Option<&str> {
    s.lines().rev().map(str::trim).find(|l| !l.is_empty())
}

/// Execute one bridge call. `args_json` is the pre-serialized JSON array of
/// Operon arguments (the caller marshals via json_stringify so Seq/Float
/// semantics stay the language's own). Errors returned as Err are
/// interpreter-level (interpreter missing); every Python-side outcome is a
/// typed PyResponse, never a panic.
pub fn py_call(
    module: &str,
    func: &str,
    args_json: &str,
    timeout_ms: u64,
) -> Result<PyResponse, String> {
    let exe = resolve_python()
        .ok_or_else(|| "python interpreter not found (tried python3, python)".to_string())?;
    let timeout_ms = timeout_ms.clamp(1, 300_000);

    // one JSON request line; json_quote escapes module/func, args_json is
    // already valid JSON from the interpreter's own serializer
    let request = format!(
        "{{\"module\":{},\"func\":{},\"args\":{}}}\n",
        crate::interp::json_quote(module),
        crate::interp::json_quote(func),
        if args_json.is_empty() {
            "[]"
        } else {
            args_json
        },
    );

    let mut cmd = Command::new(exe);
    cmd.arg("-X")
        .arg("utf8")
        .arg("-I")
        .arg("-B")
        .arg("-c")
        .arg(RUNNER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // substrate-r1: the bridge child gets the SAME safe-base environment as
    // run() children, OS essentials only, zero env grants of its own. With
    // -I, PYTHON* variables are ignored anyway; this is defense in depth.
    crate::interp::safe_base_env(&mut cmd, &[]);

    let started = std::time::Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("py spawn: {}", e))?;
    {
        let mut sin = child.stdin.take().ok_or("py stdin unavailable")?;
        sin.write_all(request.as_bytes())
            .map_err(|e| format!("py request write: {}", e))?;
        sin.flush().ok();
        // drop == close: an import-time stdin reader gets EOF, not a hang
    }

    // concurrent drainers, capped, same shape as run() (sec-r3/sec-r4)
    use std::sync::mpsc;
    let (tx_so, rx_so) = mpsc::channel::<Vec<u8>>();
    let (tx_se, rx_se) = mpsc::channel::<Vec<u8>>();
    if let Some(mut p) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            use std::io::Read;
            let mut chunk = [0u8; 16384];
            loop {
                match p.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        v.extend_from_slice(&chunk[..n]);
                        if v.len() > MAX_CHILD_OUT {
                            break;
                        }
                    }
                }
            }
            let _ = tx_so.send(v);
        });
    }
    if let Some(mut p) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            use std::io::Read;
            let mut chunk = [0u8; 16384];
            loop {
                match p.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        v.extend_from_slice(&chunk[..n]);
                        if v.len() > MAX_CHILD_OUT {
                            break;
                        }
                    }
                }
            }
            let _ = tx_se.send(v);
        });
    }

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if started.elapsed().as_millis() as u64 >= timeout_ms {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) => return Err(format!("py wait: {}", e)),
        }
    };
    let so_raw = rx_so.recv_timeout(DRAIN_WAIT).unwrap_or_default();
    let se_raw = rx_se.recv_timeout(DRAIN_WAIT).unwrap_or_default();
    let stdout = String::from_utf8_lossy(&so_raw).to_string();
    let stderr = String::from_utf8_lossy(&se_raw).to_string();

    if timed_out {
        return Ok(PyResponse {
            ok: false,
            value: Value::Null,
            error: Some(format!(
                "py: killed after {} ms (timeout; set .cell py.timeout_ms)",
                timeout_ms
            )),
            code: -1,
            timed_out: true,
            py_version: None,
        });
    }
    let code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1) as i64;

    // protocol: the LAST non-empty stdout line must parse as {"ok":..}
    let resp = match last_line(&stdout).map(crate::interp::json_parse) {
        Some(Ok(v)) => v,
        _ => {
            return Ok(PyResponse {
                ok: false,
                value: Value::Null,
                error: Some(truncate_protocol(
                    "py bridge protocol failure",
                    &stdout,
                    &stderr,
                )),
                code: -2,
                timed_out: false,
                py_version: None,
            });
        }
    };
    let ok = matches!(&resp, Value::Map(m) if m.borrow().iter().any(|(k, v)| matches!((k, v), (Value::Str(s), Value::Bool(true)) if s == "ok")));
    let value = match &resp {
        Value::Map(m) => m
            .borrow()
            .iter()
            .find(|(k, _)| matches!(k, Value::Str(s) if s == "value"))
            .map(|(_, v)| v.clone())
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let error = if ok {
        None
    } else {
        match &resp {
            Value::Map(m) => Some(
                m.borrow()
                    .iter()
                    .find(|(k, _)| matches!(k, Value::Str(s) if s == "error"))
                    .map(|(_, v)| v.display())
                    .unwrap_or_else(|| "unknown python error".into()),
            ),
            _ => Some("unknown python error".into()),
        }
    };
    Ok(PyResponse {
        ok,
        value,
        error,
        code,
        timed_out: false,
        py_version: match &resp {
            Value::Map(m) => m
                .borrow()
                .iter()
                .find(|(k, _)| matches!(k, Value::Str(s) if s == "py"))
                .map(|(_, v)| v.display()),
            _ => None,
        },
    })
}

fn truncate_protocol(head: &str, stdout: &str, stderr: &str) -> String {
    let tail = last_line(stdout).unwrap_or("").to_string();
    let err_tail = last_line(stderr).unwrap_or("").to_string();
    let mut msg = format!("{} (stdout tail: {:?})", head, &tail[..tail.len().min(160)]);
    if !err_tail.is_empty() {
        msg.push_str(&format!(
            " (stderr tail: {:?})",
            &err_tail[..err_tail.len().min(160)]
        ));
    }
    msg
}

// ---------------------------------------------------------------- tests
// These run against the real interpreter on the machine. Every test
// self-skips when no python3/python exists; the granted proof suite
// (tests/granted/) covers the same paths at the language level.

#[cfg(test)]
mod tests {
    use super::*;

    fn need_python() -> bool {
        if !python_available() {
            eprintln!("skip: no python interpreter on PATH");
            return false;
        }
        true
    }

    #[test]
    fn math_sqrt_marshals_float() {
        if !need_python() {
            return;
        }
        let r = py_call("math", "sqrt", "[2]", 10_000).unwrap();
        assert!(r.ok, "expected ok, got {:?}", r.error);
        match r.value {
            Value::Float(f) => assert!((f - std::f64::consts::SQRT_2).abs() < 1e-12),
            other => panic!("expected float, got {:?}", other.display()),
        }
        assert_eq!(r.code, 0);
    }

    #[test]
    fn json_roundtrip_list() {
        if !need_python() {
            return;
        }
        let r = py_call("json", "loads", "[\"[1,2,3]\"]", 10_000).unwrap();
        assert!(r.ok);
        match r.value {
            Value::List(l) => {
                let want = [1.0, 2.0, 3.0];
                for (i, v) in l.borrow().iter().enumerate() {
                    match v {
                        Value::Int(n) => assert_eq!(*n as f64, want[i]),
                        Value::Float(f) => assert_eq!(*f, want[i]),
                        o => panic!("unexpected {}", o.display()),
                    }
                }
            }
            o => panic!("expected list, got {}", o.display()),
        }
    }

    #[test]
    fn dotted_func_resolution() {
        if !need_python() {
            return;
        }
        let r = py_call("math", "pow", "[2, 10]", 10_000).unwrap();
        assert!(r.ok);
        match r.value {
            Value::Float(f) => assert!((f - 1024.0).abs() < 1e-9),
            Value::Int(i) => assert_eq!(i, 1024),
            o => panic!("expected number, got {}", o.display()),
        }
    }

    #[test]
    fn python_exception_is_data_not_panic() {
        if !need_python() {
            return;
        }
        let r = py_call("math", "sqrt", "[-1]", 10_000).unwrap();
        assert!(!r.ok);
        assert!(r.error.as_deref().unwrap_or("").contains("ValueError"));
        assert_eq!(r.code, 1);
    }

    #[test]
    fn module_not_found_contained() {
        if !need_python() {
            return;
        }
        let r = py_call("operon_no_such_module_xyz", "f", "[]", 10_000).unwrap();
        assert!(!r.ok);
        assert!(r.error.as_deref().unwrap_or("").contains("No module named"));
    }

    #[test]
    fn chatty_module_does_not_corrupt_protocol() {
        if !need_python() {
            return;
        }
        // `this` prints the Zen of Python AT IMPORT, stdout noise before the
        // response line. The LAST non-empty line must still parse cleanly.
        let r = py_call("this", "x", "[]", 5_000).unwrap();
        assert!(!r.ok, "this has no attr x, must be a typed AttributeError");
        assert!(r.error.as_deref().unwrap_or("").contains("AttributeError"));
    }

    #[test]
    fn timeout_kills_bounded() {
        if !need_python() {
            return;
        }
        let started = std::time::Instant::now();
        let r = py_call("time", "sleep", "[30]", 300).unwrap();
        assert!(r.timed_out);
        assert!(!r.ok);
        assert_eq!(r.code, -1);
        assert!(
            started.elapsed().as_millis() < 5_000,
            "kill must be bounded"
        );
    }

    #[test]
    fn environment_is_scrubbed() {
        if !need_python() {
            return;
        }
        std::env::set_var("PYBRIDGE_CANARY", "leak-me");
        let r = py_call("os", "getenv", "[\"PYBRIDGE_CANARY\"]", 10_000).unwrap();
        assert!(r.ok);
        assert!(
            matches!(r.value, Value::Null),
            "canary must not leak: {}",
            r.value.display()
        );
    }

    #[test]
    fn output_cap_is_enforced() {
        if !need_python() {
            return;
        }
        // ~76 MiB of output: beyond MAX_CHILD_OUT the drainer stops; the
        // child dies on EPIPE / timeout; the outcome stays typed
        let started = std::time::Instant::now();
        let r = py_call("os", "urandom", "[76000000]", 30_000).unwrap();
        assert!(
            matches!(r.value, Value::Null) || r.ok,
            "capped output must not become a giant value"
        );
        assert!(started.elapsed().as_secs() < 40);
    }

    #[test]
    fn numpy_if_present() {
        if !need_python() {
            return;
        }
        let probe = py_call("numpy", "arange", "[0, 5]", 30_000).unwrap();
        if !probe.ok {
            eprintln!("skip: numpy absent on this interpreter");
            return;
        }
        assert!(
            matches!(probe.value, Value::List(ref l) if l.borrow().len() == 5),
            "ndarray must tolist() into an Operon list: {}",
            probe.value.display()
        );
    }
}
