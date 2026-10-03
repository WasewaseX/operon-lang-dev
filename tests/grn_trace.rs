//! W095 — GRN tick-stream (`--trace-grn`).
//!
//! Every engine update point (a `grn_fire` pulse or a decay-clock tick —
//! both funnel through `trans_integrate`) snapshots the current level map
//! as one JSONL frame. Frames carry a monotonic `tick`, the `phase`
//! ("fire" | "decay"), and the SORTED level map (deterministic output per
//! the W089 determinism contract — HashMap iteration order never crosses).
//! The interpreter performs no I/O; the CLI drains the buffer to the
//! operator's file after the run (success or contained failure).
//!
//! Each test drives the release binary (the sec_regression pattern) — the
//! flag is an operator diagnostic surface, so the pins are end-to-end.

use std::process::Command;

fn run_trace(src: &str) -> (i32, String, String, String) {
    // Per-CALL unique dir (nanos+pid): parallel tests in one binary shared a
    // per-pid dir before, and a finishing test's remove_dir could delete
    // another test's just-created EMPTY dir mid-flight (Windows CI failure,
    // 2026-09-27; also a rare sandbox flake). Unique dirs make the race
    // structurally impossible.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let dir = std::env::temp_dir().join(format!("operon_trace_{}_{}", std::process::id(), nanos));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join(format!("t{}_{}.op", nanos, std::process::id()));
    let tf = dir.join(format!("tr{}_{}.jsonl", nanos, std::process::id()));
    std::fs::write(&f, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("run")
        .arg(&f)
        .arg("--trace-grn")
        .arg(&tf)
        .output()
        .expect("run operon");
    let trace = std::fs::read_to_string(&tf).unwrap_or_default();
    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_file(&tf);
    let _ = std::fs::remove_dir(&dir);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        trace,
    )
}

const GRN_SRC: &str = r#"regulate {
  x activates z strength 1.0 threshold 0.3
  z translates zp rate 0.5 decay 0.1
}
gene x() { return "x-tx" }
gene z() { return "z-OUT" }
gene main() {
  decay_clock(2, 0.05)
  grn_fire("x")
  x()
  grn_fire("x")
  print(grn_get("zp"))
}
"#;

#[test]
fn frames_fire_decay_and_monotonic_ticks() {
    let (rc, _out, _err, trace) = run_trace(GRN_SRC);
    assert_eq!(rc, 0, "run succeeds");
    let lines: Vec<&str> = trace.lines().filter(|l| l.starts_with('{')).collect();
    assert!(!lines.is_empty(), "frames emitted:\n{trace}");
    // ticks are sequential 0..N and phases stay within the documented set
    for (i, l) in lines.iter().enumerate() {
        assert!(
            l.contains(&format!("\"tick\":{}", i)),
            "tick {} in place: {}",
            i,
            l
        );
        let phase = if l.contains("\"phase\":\"fire\"") {
            "fire"
        } else if l.contains("\"phase\":\"decay\"") {
            "decay"
        } else {
            ""
        };
        assert!(!phase.is_empty(), "phase present and known: {}", l);
    }
    // both phases actually occur in this program
    assert!(
        trace.contains("\"phase\":\"fire\""),
        "fire frames:\n{trace}"
    );
    assert!(
        trace.contains("\"phase\":\"decay\""),
        "decay frames:\n{trace}"
    );
}

#[test]
fn level_map_is_sorted_and_valid_jsonl_shape() {
    let (_rc, _out, _err, trace) = run_trace(GRN_SRC);
    for l in trace.lines() {
        assert!(
            l.starts_with('{') && l.ends_with('}'),
            "one object per line: {}",
            l
        );
        assert!(l.contains("\"levels\":{"), "levels object present: {}", l);
        // keys inside one frame are in byte order — parse them back out
        if let Some(pos) = l.find("\"levels\":{\"") {
            let rest = &l[pos + "\"levels\":{\"".len()..];
            let inner = rest.trim_end_matches('}');
            let mut keys: Vec<String> = Vec::new();
            for pair in inner.split(',') {
                let k = pair.split('"').nth(1).unwrap_or_default();
                if !k.is_empty() {
                    keys.push(k.to_string());
                }
            }
            let mut sorted = keys.clone();
            sorted.sort();
            assert_eq!(
                keys, sorted,
                "level keys byte-sorted inside the frame: {}",
                l
            );
        }
    }
}

#[test]
fn tracing_off_is_a_no_op_and_run_output_unchanged() {
    // without the flag the same program runs identically (no trace side
    // effects) — the OFF path must stay byte-identical
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let dir = std::env::temp_dir().join(format!("operon_notrace_{}_{}", std::process::id(), nanos));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("plain.op");
    std::fs::write(&f, GRN_SRC).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .arg("run")
        .arg(&f)
        .output()
        .expect("run operon");
    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_dir(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.contains("0"),
        "program still prints the level: {stdout}"
    );

    // and the traced run's PROGRAM output matches the untraced one — the
    // diagnostic must not perturb semantics
    let (_rc, out_traced, _err, _trace) = run_trace(GRN_SRC);
    let plain_lines: Vec<&str> = stdout.lines().collect();
    let traced_lines: Vec<&str> = out_traced.lines().collect();
    assert_eq!(plain_lines, traced_lines, "identical program output");
}

#[test]
fn trace_survives_a_contained_failure() {
    // frames collected before an uncaught stress still drain (exit 1 path)
    let src = r#"regulate {
  x activates z strength 1.0 threshold 0.3
}
gene x() { return "x-tx" }
gene z() { return "z-OUT" }
gene main() {
  grn_fire("x")
  raise "boom"
}
"#;
    let (rc, out, err, trace) = run_trace(src);
    // harness lesson (W100 session-2): a failing child's stderr must not
    // anonymize — surface everything or the failure is undebuggable on
    // runners we cannot reproduce locally (macos arm64 exit-code lie,
    // 2026-09-29 release-matrix run).
    assert_eq!(
        rc,
        1,
        "uncaught stress exits 1 | stdout={:?} | stderr={:?} | trace_len={}",
        out,
        err,
        trace.len()
    );
    assert!(err.contains("boom"), "the failure is reported");
    assert!(
        trace.contains("\"phase\":\"fire\""),
        "pre-failure frames drained:\n{trace}"
    );
}
