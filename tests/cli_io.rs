//! #113 CLI I/O contract family — the read/write error paths die CLEANLY
//! (rc 2, no data loss, no panic) instead of truncating files to zero bytes
//! with exit 0, panicking with exit 101, or reporting success on unreadable
//! input. Every leg mirrors a falsified behavior from INTEGRITY-SWEEP-2.

use std::process::Command;

fn operon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_operon"))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("operon_113_{}_{}", std::process::id(), name));
    p
}

#[test]
fn fmt_write_never_truncates_unreadable_files() {
    let f = scratch("bad_utf8.op");
    std::fs::write(&f, b"let bad = \"\xff\xfe\"\n").unwrap();
    let before = std::fs::read(&f).unwrap();
    let out = operon()
        .arg("fmt")
        .arg(&f)
        .arg("--write")
        .output()
        .expect("run fmt");
    assert_eq!(
        out.status.code(),
        Some(2),
        "fmt --write on a non-UTF-8 file must die cleanly rc 2"
    );
    assert_eq!(
        std::fs::read(&f).unwrap(),
        before,
        "fmt --write must NEVER truncate an unreadable file (data loss class)"
    );
    std::fs::remove_file(&f).ok();
}

#[test]
fn fmt_write_missing_file_dies_and_creates_nothing() {
    let f = scratch("does_not_exist.op");
    std::fs::remove_file(&f).ok();
    let out = operon()
        .arg("fmt")
        .arg(&f)
        .arg("--write")
        .output()
        .expect("run fmt");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        !f.exists(),
        "fmt --write on a missing file must not create an empty file"
    );
}

#[test]
fn check_unreadable_file_is_not_a_false_green() {
    let out = operon()
        .arg("check")
        .arg(scratch("nope_for_check.op"))
        .output()
        .expect("run check");
    assert_ne!(
        out.status.code(),
        Some(0),
        "check on an unreadable file displayed error[E00] with exit 0 — the dx-r1 false-green class"
    );
}

#[test]
fn build_bad_output_path_dies_cleanly_not_panic() {
    let src = scratch("build_src_ok.op");
    std::fs::write(&src, "let x = 1\n").unwrap();
    let out = operon()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg("/nope/dir/x.op")
        .output()
        .expect("run build");
    assert_eq!(
        out.status.code(),
        Some(2),
        "build -o into an unwritable path died with exit 101 (panic) — must die cleanly rc 2"
    );
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("internal toolchain panic"),
        "write failures must not surface as toolchain panics"
    );
    std::fs::remove_file(&src).ok();
}

#[test]
fn graph_and_doc_die_on_unreadable_input() {
    for sub in ["graph", "doc", "disasm"] {
        let out = operon()
            .arg(sub)
            .arg(scratch("nope_for_doc.op"))
            .output()
            .expect("run subcommand");
        assert_ne!(
            out.status.code(),
            Some(0),
            "{} reported success on unreadable input",
            sub
        );
    }
}

// ---- #122: CLI false-green family ----

#[test]
fn frame_body_error_is_not_swallowed() {
    let f = scratch("frame_boom.op");
    std::fs::write(&f, "frame boom { assert(1 == 2, \"boom fails\") }\n").unwrap();
    let out = operon()
        .arg("run")
        .arg(&f)
        .arg("--frame")
        .arg("boom")
        .output()
        .expect("run --frame");
    assert_eq!(
        out.status.code(),
        Some(1),
        "a failing frame body was DROPPED (let _) — --frame smoke gates were false-green machines"
    );
    std::fs::remove_file(&f).ok();
}

#[test]
fn fmt_write_missing_file_dies_and_creates_nothing() {
    let f = scratch("does_not_exist.op");
    std::fs::remove_file(&f).ok();
    let out = operon()
        .arg("fmt")
        .arg(&f)
        .arg("--write")
        .output()
        .expect("run fmt");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        !f.exists(),
        "fmt --write on a missing file must not create an empty file"
    );
}

#[test]
fn check_unreadable_file_is_not_a_false_green() {
    let out = operon()
        .arg("check")
        .arg(scratch("nope_for_check.op"))
        .output()
        .expect("run check");
    assert_ne!(
        out.status.code(),
        Some(0),
        "check on an unreadable file displayed error[E00] with exit 0 — the dx-r1 false-green class"
    );
}

#[test]
fn build_bad_output_path_dies_cleanly_not_panic() {
    let src = scratch("build_src_ok.op");
    std::fs::write(&src, "let x = 1\n").unwrap();
    let out = operon()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg("/nope/dir/x.op")
        .output()
        .expect("run build");
    assert_eq!(
        out.status.code(),
        Some(2),
        "build -o into an unwritable path died with exit 101 (panic) — must die cleanly rc 2"
    );
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("internal toolchain panic"),
        "write failures must not surface as toolchain panics"
    );
    std::fs::remove_file(&src).ok();
}

#[test]
fn graph_and_doc_die_on_unreadable_input() {
    for sub in ["graph", "doc", "disasm"] {
        let out = operon()
            .arg(sub)
            .arg(scratch("nope_for_doc.op"))
            .output()
            .expect("run subcommand");
        assert_ne!(
            out.status.code(),
            Some(0),
            "{} reported success on unreadable input",
            sub
        );
    }
fn missing_frame_is_not_a_false_green() {
    let f = scratch("frame_ok.op");
    std::fs::write(&f, "frame loud { print(1) }\n").unwrap();
    let out = operon()
        .arg("run")
        .arg(&f)
        .arg("--frame")
        .arg("nope")
        .output()
        .expect("run --frame");
    assert_eq!(out.status.code(), Some(1), "a missing frame was a rung-4 note with exit 0");
    std::fs::remove_file(&f).ok();
}

#[test]
fn entry_validation_applies_under_frame() {
    let f = scratch("entry_typo.op");
    std::fs::write(&f, "gene main() { print(1) }\n").unwrap();
    let out = operon()
        .arg("run")
        .arg(&f)
        .arg("--entry")
        .arg("mian")
        .arg("--frame")
        .arg("main")
        .output()
        .expect("run");
    assert_eq!(
        out.status.code(),
        Some(1),
        "--entry mian under --frame silently ran NOTHING with exit 0 (W101 reopened)"
    );
    std::fs::remove_file(&f).ok();
}

#[test]
fn help_exits_zero() {
    for flag in ["--help", "-h"] {
        let out = operon().arg(flag).output().expect("help");
        assert_eq!(out.status.code(), Some(0), "{} must exit 0 (dx-r5)", flag);
    }
}

#[test]
fn value_flags_reject_flag_shaped_values() {
    let f = scratch("flag_eat.op");
    std::fs::write(&f, "gene main() { print(1) }\n").unwrap();
    let out = operon()
        .arg("run")
        .arg(&f)
        .arg("--variant")
        .arg("--quiet")
        .output()
        .expect("run");
    assert_ne!(
        out.status.code(),
        Some(0),
        "--variant --quiet silently lost --quiet (value flags ate flags)"
    );
    std::fs::remove_file(&f).ok();
}
