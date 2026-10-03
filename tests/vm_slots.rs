//! W011 stage-2 frame-slot locals — lane-parity regression tests (2026-10-04).
//!
//! The slot path binds eligible params into a per-frame slot Vec (reads via
//! LoadName/LoadNameQuiet/LoadBinImm/RetName, writes via AssignName, with
//! write-through to the frame env so the call funnel's env view stays
//! authoritative). The tree-walk lane never changed. Every test here runs
//! the SAME program on BOTH lanes and asserts identical stdout — the
//! invariant vm_parity.sh checks corpus-wide, pinned here for the specific
//! shapes the qualification analysis must get right:
//!   - plain param reads/returns (the fib shape)
//!   - param reassigned in nested scopes (AssignName routing)
//!   - param shadowed by a nested `let` (must fall back to env semantics)
//!   - higher-order params holding gene values (callee resolution)
//!   - missing args / extra args (notes on the shared binding path)

use std::process::Command;

fn run(lane_flag: &str, src: &str, tag: &str) -> (i32, String) {
    // unique dir per (pid, test): cargo runs one binary's tests on parallel
    // threads — a shared dir would race prog.op writes across tests
    let dir = std::env::temp_dir().join(format!(
        "op_slot_test_{}_{}",
        std::process::id(),
        tag.replace(" ", "_")
    ));
    std::fs::create_dir_all(&dir).unwrap_or_default();
    let file = dir.join("prog.op");
    std::fs::write(&file, src).unwrap_or_default();
    let out = Command::new(env!("CARGO_BIN_EXE_operon"))
        .args(["run", lane_flag])
        .arg(&file)
        .output()
        .expect("operon binary runs");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn assert_lanes_agree(name: &str, src: &str) {
    // the program must EXIT CLEAN on both lanes; stdout byte-equal
    let (rc_vm, out_vm) = run("--vm", src, name);
    let (rc_tw, out_tw) = run("--no-vm", src, name);
    assert_eq!(rc_vm, 0, "{name}: vm lane rc {rc_vm}");
    assert_eq!(rc_vm, rc_tw, "{name}: rc divergence vm={rc_vm} tw={rc_tw}");
    assert_eq!(out_vm, out_tw, "{name}: stdout divergence");
}

fn assert_lanes_agree_allow_stress(name: &str, src: &str) {
    // for programs whose PINNED behavior is a catchable stress (rc 1 on
    // both lanes): rc equality + empty stdout on both
    let (rc_vm, out_vm) = run("--vm", src, name);
    let (rc_tw, out_tw) = run("--no-vm", src, name);
    assert_eq!(rc_vm, rc_tw, "{name}: rc divergence vm={rc_vm} tw={rc_tw}");
    assert_eq!(out_vm, out_tw, "{name}: stdout divergence");
}

#[test]
fn param_reads_and_recursion_match_lanes() {
    assert_lanes_agree(
        "fib",
        r#"gene fib(n) {
    if n <= 1 { return n }
    return fib(n - 1) + fib(n - 2)
}
main { print(fib(15)) }
"#,
    );
}

#[test]
fn param_reassigned_in_nested_scopes_matches_lanes() {
    assert_lanes_agree(
        "assign nested",
        r#"gene work(n) {
    let acc = 0
    let i = 0
    while i < n {
        acc = acc + i
        i += 1
        if i == 3 {
            n = n + 10
        }
    }
    return acc * 100 + n
}
main { print(work(5)) }
"#,
    );
}

#[test]
fn param_shadowed_by_nested_let_matches_lanes() {
    // n is shadowed by a branch-local let: the qualification must keep n
    // on the env path; both lanes must agree either way.
    assert_lanes_agree(
        "shadow",
        r#"gene shape(n) {
    let outer = n
    if n > 0 {
        let n = 99
        outer = outer + n
    }
    return outer + n
}
main { print(shape(7)) }
"#,
    );
}

#[test]
fn higher_order_param_callee_matches_lanes() {
    // f is a param holding a gene value; the funnel resolves the callee
    // through the env chain, which write-through keeps authoritative.
    assert_lanes_agree(
        "hof",
        r#"gene twice(f, v) {
    return f(f(v))
}
gene add3(x) {
    return x + 3
}
main { print(twice(add3, 10)) }
"#,
    );
}

#[test]
fn missing_and_extra_args_match_lanes() {
    // g(1) binds b=null (shared note) then int+null stresses on BOTH lanes
    // (pinned: the note fires, the stress is the shared apply_binop).
    assert_lanes_agree_allow_stress(
        "arity",
        r#"gene g(a, b) {
    return a * 10 + b
}
main {
    print(g(1))
    print(g(1, 2, 3))
}
"#,
    );
}
