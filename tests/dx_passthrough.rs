//! dx-r6 STRICT regression (ytdl-app audit, 2026-10):
//! after the POSIX `--` separator, EVERY remaining argument belongs to the
//! program verbatim — including flag names the host CLI also defines.
//! Before the fix, `operon run app.op -- get U --out D` silently lost
//! `--out D` to the host's build-output flag (the host arms kept matching
//! past the separator). A second `--` must arrive as a literal program arg.

use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_operon");

fn write_probe(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn passthrough_survives_host_known_flags() {
    let tmp = std::env::temp_dir().join(format!("dx_passthrough_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let script = write_probe(
        &tmp,
        "probe.op",
        r#"
gene main() {
    let a = argv()
    promote("argc={len(a)}")
    promote("a1={a[0]}")
    promote("a2={a[1]}")
    promote("a3={a[2]}")
}
"#,
    );

    // --out and --json are host-known names; after `--` they must survive.
    let out = Command::new(EXE)
        .args([
            "run",
            script.to_str().unwrap(),
            "--",
            "get",
            "--out",
            "/tmp/x",
            "--json",
        ])
        .output()
        .expect("spawn operon");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("argc=4"), "argc line missing: {s}");
    assert!(s.contains("a1=get"), "first arg lost: {s}");
    assert!(s.contains("a2=--out"), "host-known --out was eaten: {s}");
    assert!(s.contains("a3=/tmp/x"), "value of --out was eaten: {s}");

    // a second `--` is a literal program argument (POSIX).
    let out = Command::new(EXE)
        .args(["run", script.to_str().unwrap(), "--", "--", "x"])
        .output()
        .expect("spawn operon");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("a1=--"), "second -- must be literal: {s}");
    assert!(s.contains("a2=x"), "arg after second -- lost: {s}");

    // no `--` at all: positional still works, unknown --flags still die loudly
    // (dx-r1 typo guard is unchanged).
    let out = Command::new(EXE)
        .args(["run", script.to_str().unwrap(), "--strick"])
        .output()
        .expect("spawn operon");
    let s = String::from_utf8_lossy(&out.stderr);
    assert!(
        s.contains("unknown flag '--strick'"),
        "typo guard must stay loud: {s}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
