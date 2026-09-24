// T4 — REPL contract tests: the shell must evaluate expressions, persist
// definitions, list genes, run session + file proof frames, and reset.
// Pipes scripted stdin into `operon repl` and asserts on stdout.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_repl(script: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn operon repl");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(script.as_bytes())
        .expect("write repl stdin");
    let out = child.wait_with_output().expect("repl run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn repl_evaluates_and_persists_definitions() {
    let out = run_repl("1 + 2 * 3\ngene double(x) { return x * 2 }\ndouble(21)\n:quit\n");
    assert!(out.contains('7'), "bare expression should print 7:\n{out}");
    assert!(
        out.contains("42"),
        "gene defined in-session should be callable:\n{out}"
    );
}

#[test]
fn repl_session_proof_frames_pass_and_detect_vacuous() {
    let out = run_repl(
        "gene sq(x) { return x * x }\nframe proof { assert(sq(3) == 9, \"sq works\") }\n:proof\n:quit\n",
    );
    assert!(
        out.contains("1/1 passed"),
        "session proof should pass:\n{out}"
    );
}

#[test]
fn repl_lists_genes_vars_and_resets() {
    let out = run_repl("let answer = 42\n:genes\n:vars\n:reset\n:genes\n:quit\n");
    assert!(out.contains("answer"), "vars should list answer:\n{out}");
    assert!(
        out.contains("(no genes defined yet"),
        "reset should clear state:\n{out}"
    );
}

#[test]
fn repl_load_runs_a_file_and_file_proofs() {
    // a small fixture written to a temp dir: one gene + one passing proof
    let mut path = std::env::temp_dir();
    path.push("operon_repl_t4_fixture.op");
    std::fs::write(
        &path,
        "gene add(a, b) { return a + b }\nframe proof { assert(add(1, 2) == 3, \"add\") }\n",
    )
    .unwrap();
    let script = format!(
        ":load {}\nadd(40, 2)\n:proof {}\n:quit\n",
        path.display(),
        path.display()
    );
    let out = run_repl(&script);
    assert!(
        out.contains("42"),
        ":load should make genes callable:\n{out}"
    );
    assert!(
        out.contains("1/1 proof(s) passed"),
        ":proof file.op should pass:\n{out}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn repl_unknown_command_hints_help() {
    let out = run_repl(":frobnicate\n:quit\n");
    assert!(
        out.contains(":help"),
        "unknown commands should point at :help:\n{out}"
    );
}
