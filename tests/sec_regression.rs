//! sec_regression.rs — regression tests for the sec-r1 audit fixes.
//!
//! Each test drives the RELEASE binary (like redteam.sh) but asserts the
//! CONTAINED DIAGNOSTIC, not just "no crash" — redteam payloads prove
//! containment; these prove the specific fix is live.
//!
//! Audit cross-refs: C-1 (similar DP freeze), C-2 (regex parser stack
//! overflow), C-3 (uncharged push), C-4 (LSP framing panic), C-7 (module
//! existence oracle), C-10 (join ceiling).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_op(src: &str) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!("operon_sec_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join(format!(
        "t{}_{}.op",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos(),
        std::process::id()
    ));
    std::fs::write(&f, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("run")
        .arg(&f)
        .output()
        .expect("run operon");
    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_dir(&dir);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn combined(rc: i32, out: &str, err: &str) -> String {
    format!("rc={} {} {}", rc, out, err)
}

#[test]
fn similar_dp_ceiling_contained() {
    // C-1: pre-fix this froze the process in a fuel-blind kernel call.
    let (rc, out, err) = run_op(
        r#"
let a = "a" * 40000
let b = "b" * 40000
stress {
    let close = similar(a, b, 2)
} rescue (e) {
    promote("contained:" + e.kind)
}
"#,
    );
    let all = combined(rc, &out, &err);
    assert!(
        out.contains("contained:overflow"),
        "rescue must observe the DP ceiling stress; got: {}",
        all
    );
    assert!(!all.contains("panicked"), "no panic allowed: {}", all);
}

#[test]
fn regex_parser_nesting_contained() {
    // C-2: pre-fix this aborted the process (rc=134, stack overflow).
    let (rc, out, err) = run_op(
        r#"
let p = "(" * 2000000 + ")" * 2000000
stress {
    let hit = re_match(p, "x")
} rescue (e) {
    promote("contained:" + e.kind)
}
"#,
    );
    let all = combined(rc, &out, &err);
    assert!(
        !all.contains("stack overflow"),
        "regex parser must not blow the native stack: {}",
        all
    );
    assert!(
        out.contains("contained:"),
        "nesting bomb must be caught by the rescue: {}",
        all
    );
    assert!(rc < 130, "no signal death: {}", all);
}

#[test]
fn push_builtin_is_memory_charged() {
    // C-3: pre-fix push() bypassed the aggregate allocator ceiling entirely
    // (allocator abort under pressure). Post-fix the 2 GiB ceiling trips
    // catchably; RSS stays tiny because the list is dropped each iteration
    // while the charge accumulates.
    let (rc, out, err) = run_op(
        r#"
let s = "A" * 16000000
let i = 0
while i < 500 {
    let l = []
    push(l, s)
    i = i + 1
}
"#,
    );
    let all = combined(rc, &out, &err);
    assert!(
        all.contains("aggregate allocation ceiling"),
        "uncharged push must trip the ceiling stress: {}",
        all
    );
    assert!(
        !all.contains("panicked") && !all.contains("memory allocation"),
        "no allocator abort: {}",
        all
    );
    assert!(rc < 130, "no signal death: {}", all);
}

#[test]
fn join_result_ceiling_contained() {
    // C-10: join minted unbounded result strings outside every ceiling.
    let (rc, out, err) = run_op(
        r#"
let s = "A" * 60000000
let l = [s, s, s, s, s, s, s, s, s, s]
stress {
    let joined = l.join("")
} rescue (e) {
    promote("contained:" + e.kind)
}
"#,
    );
    let all = combined(rc, &out, &err);
    assert!(
        out.contains("contained:overflow"),
        "join must hit the 512 MiB ceiling: {}",
        all
    );
    assert!(!all.contains("panicked"), "no panic: {}", all);
}

#[test]
fn module_oracle_closed() {
    // C-7: real vs ghost outside paths must be indistinguishable — ONE
    // unified failure string for the traversal name class.
    let dir = std::env::temp_dir().join(format!("operon_oracle_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("prog.op"),
        "use \"../sec_oracle_real\"\nuse \"../sec_oracle_ghost\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.parent().unwrap().join("sec_oracle_real.op"),
        "let x = 1\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("run")
        .arg(dir.join("prog.op"))
        .output()
        .expect("run operon");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(dir.parent().unwrap().join("sec_oracle_real.op"));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let real_denied = text.contains("sec_oracle_real") && text.contains("load failed");
    assert!(real_denied, "real outside module must be denied: {}", text);
    assert!(
        !text.contains("not found") || !text.contains("sec_oracle_ghost"),
        "ghost must not produce a distinct 'not found' for traversal names: {}",
        text
    );
}

#[test]
fn lsp_framing_never_panics() {
    // C-4: an attacker-controlled Content-Length panicked the language
    // server (capacity overflow, rc=101). Post-fix: graceful close.
    let mut child = Command::new(env!("CARGO_BIN_EXE_operon-ls"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn operon-ls");
    {
        let stdin = child.stdin.as_mut().unwrap();
        let _ = stdin.write_all(b"Content-Length: 18446744073709551615\r\n\r\n{");
    }
    let out = child.wait_with_output().expect("wait operon-ls");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let rc = out.status.code().unwrap_or(-1);
    assert!(rc < 130, "LSP must not die by signal: rc={} {}", rc, err);
    assert!(
        !err.contains("panicked"),
        "LSP must not panic on a hostile frame: {}",
        err
    );
    assert!(
        !err.contains("capacity overflow"),
        "no unbounded frame allocation: {}",
        err
    );
}
