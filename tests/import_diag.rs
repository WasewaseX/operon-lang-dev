//! W070 — import diagnostics: a failed `use` reports the SEARCH, not just the
//! failure. The golden shape: `module '<path>' not found, tried: <candidate>
//! (<why>); ... ; OPERON_STD (<state>)` — every probed candidate with its
//! failure reason, capped list, and the resolved std root row.
//!
//! Done-when (ROADMAP-100 W70): "an import of `use missing_mod` prints the
//! full probe list."

use std::process::Command;

fn run_missing_import(dir: &std::path::Path, module: &str) -> (i32, String, String) {
    let prog = dir.join("miss.op");
    std::fs::write(
        &prog,
        format!(
            "gene main() {{\n    print(\"body ran\")\n}}\nmain()\nuse {}\n",
            module
        ),
    )
    .expect("write program");
    // a local decoy so the candidate list has a non-std row to enumerate
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("run")
        .arg(&prog)
        .output()
        .expect("run operon");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn missing_import_prints_full_probe_list() {
    let dir = std::env::temp_dir().join(format!("operon_w070_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let (code, stdout, stderr) = run_missing_import(&dir, "missing_mod");
    let combined = format!("{}{}", stdout, stderr);

    // the program still runs (Total Grammar: failed use is a repair note)
    assert!(stdout.contains("body ran"), "program body executed");
    let _ = code; // exit code is run's own; the note rides stderr

    // golden: the tried-list names the module and enumerates candidates
    assert!(
        combined.contains("module 'missing_mod' not found, tried:"),
        "tried-list header present, got: {}",
        combined
    );
    // relative candidate (sibling of the importer)
    assert!(
        combined.contains("missing_mod.op (missing)"),
        "relative candidate with reason, got: {}",
        combined
    );
    // std-root candidate
    assert!(
        combined.contains("std/missing_mod.op (missing)"),
        "std candidate with reason, got: {}",
        combined
    );
    // the OPERON_STD resolution row (resolved std root or its absence)
    assert!(
        combined.contains("OPERON_STD"),
        "std-root provenance row present, got: {}",
        combined
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hit_import_does_not_emit_tried_list() {
    let dir = std::env::temp_dir().join(format!("operon_w070b_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    // a module that EXISTS must not produce the diagnostic
    std::fs::write(
        dir.join("real_mod.op"),
        "gene helper() {\n    return 1\n}\n",
    )
    .unwrap();
    let prog = dir.join("hit.op");
    std::fs::write(
        &prog,
        "use real_mod\ngene main() {\n    print(\"ok\")\n}\nmain()\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("run")
        .arg(&prog)
        .output()
        .expect("run operon");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !combined.contains("not found, tried:"),
        "no probe list on hit"
    );
    assert!(combined.contains("ok"), "imported program ran");

    std::fs::remove_dir_all(&dir).ok();
}
