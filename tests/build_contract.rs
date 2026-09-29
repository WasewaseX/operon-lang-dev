//! W086 — `operon build` contract honesty. The CLI must not silently ignore
//! modes it does not implement: `--bundle` (W87, deferred design) and
//! `--native` (W85) refuse with exit 2 and a pointer to their design state;
//! the plain source-bake build is untouched.

use std::process::Command;

fn run_build(dir: &std::path::Path, extra: &[&str]) -> (i32, String, String) {
    let prog = dir.join("prog.op");
    std::fs::write(&prog, "gene main() {\n    print(\"x\")\n}\nmain()\n").expect("write program");
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("build")
        .arg(&prog)
        .args(extra)
        .output()
        .expect("run operon build");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("operon_w086_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn native_refuses_honestly() {
    let dir = tempdir("native");
    let (code, stdout, stderr) = run_build(&dir, &["--native"]);
    assert_eq!(code, 2, "honest refusal exits 2");
    let combined = format!("{}{}", stdout, stderr);
    assert!(combined.contains("--native"), "names the flag");
    assert!(combined.contains("not implemented"), "says what is true");
    assert!(combined.contains("W85"), "points at the owning item");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn bundle_refuses_honestly() {
    let dir = tempdir("bundle");
    let (code, stdout, stderr) = run_build(&dir, &["--bundle"]);
    assert_eq!(code, 2, "honest refusal exits 2");
    let combined = format!("{}{}", stdout, stderr);
    assert!(combined.contains("--bundle"), "names the flag");
    assert!(combined.contains("not implemented"), "says what is true");
    assert!(
        combined.contains("docs/design/BUNDLE.md"),
        "points at the deferred design"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn plain_bake_unaffected() {
    let dir = tempdir("plain");
    let (code, _stdout, stderr) = run_build(&dir, &["-o"]);
    // `-o` consumes the next positional, so re-run properly with a real out path
    let out_path = dir.join("baked.op");
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("build")
        .arg(dir.join("prog.op"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .expect("run operon build");
    let _ = (code, stderr);
    assert!(out.status.success(), "plain build succeeds");
    assert!(out_path.exists(), "baked file written");
    let baked = std::fs::read_to_string(&out_path).unwrap();
    assert!(
        baked.contains("gene main()"),
        "baked source carries the program"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn usage_lists_the_honest_flags() {
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .output()
        .expect("run operon bare");
    let usage = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        usage.contains("[--bundle] [--native]"),
        "usage line documents the refusal flags"
    );
}
