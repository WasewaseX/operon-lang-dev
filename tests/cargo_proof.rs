//! cargo_proof.rs — cargo-level integration gate for T1 (G2).
//!
//! `cargo test` must prove more than "it compiles": the shipped release
//! binary runs the full `.op` proof suite (the same files CI's proof step
//! exercises), and the CLI answers `version`. If any proof frame fails, the
//! binary exits non-zero (main.rs exits 1 when TestReport.failed > 0).

use std::process::Command;

fn operon_bin() -> &'static str {
    env!("CARGO_BIN_EXE_operon")
}

#[test]
fn version_reports_toolchain() {
    let out = Command::new(operon_bin())
        .arg("version")
        .output()
        .expect("operon binary runs");
    assert!(out.status.success(), "operon version exited non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.to_lowercase().contains("operon"),
        "`operon version` should identify itself, got: {stdout}"
    );
}

#[test]
fn proof_suite_is_green() {
    // cargo runs test binaries with CWD = CARGO_MANIFEST_DIR, so "tests"
    // and the std/ imports inside the .op files resolve exactly like CI.
    let out = Command::new(operon_bin())
        .args(["test", "tests"])
        .output()
        .expect("operon test runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "proof suite failed (exit {:?}):\n{}\n{}",
        out.status.code(),
        stdout,
        stderr
    );
    // The suite must not be vacuous: it has to actually run files and proofs.
    assert!(
        !stdout.contains("0 proof(s)"),
        "proof suite ran zero proofs — the gate is vacuous:\n{stdout}"
    );
}
