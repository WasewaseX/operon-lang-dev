//! #105 — the REPL runs permanently bookkeeping-gated: `load_file("/dev/null")`
//! classifies the empty boot program clean, `repl_eval` never re-runs the
//! analysis, and `fingerprint()`/`regulate` telemetry reads all-zeros for the
//! whole session while the same program as a FILE reports real counts.
//! Contract after the fix: each REPL line re-derives the consumer gate
//! MONOTONELY (a line can only forfeit the fast path, never re-arm it — a
//! gene defined three lines ago may carry the `fingerprint()` call this
//! line's AST cannot see).

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn repl_fingerprint_sees_real_call_counts() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn operon repl");
    {
        let stdin = child.stdin.as_mut().expect("pipe repl stdin");
        stdin
            .write_all(b"gene work(n) { if n < 2 { return n } return work(n-1) + work(n-2) }\n")
            .unwrap();
        stdin.write_all(b"work(8)\n").unwrap();
        stdin.write_all(b"fingerprint()\n").unwrap();
        stdin.write_all(b":quit\n").unwrap();
    }
    let out = child.wait_with_output().expect("repl run");
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let fp = s
        .lines()
        .filter(|l| l.contains("calls"))
        .next_back()
        .unwrap_or("");
    assert!(
        fp.contains("work"),
        "REPL fingerprint must report real per-gene call counts, got: {fp}"
    );
    assert!(
        !fp.contains("calls: {}"),
        "the all-zeros fingerprint is the #105 bug (bookkeeping silently dead all session), got: {fp}"
    );
}
