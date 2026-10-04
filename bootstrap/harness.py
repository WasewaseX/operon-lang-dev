#!/usr/bin/env python3
"""harness.py, differential test harness.

Runs every .op program through BOTH implementations (Rust core binary and the
Python oracle) and fails on any stdout divergence. This is the evidence engine
that keeps the two implementations honest.

Usage: python3 bootstrap/harness.py [--bin ../bin/operon]
"""
import subprocess, sys, os, argparse

# compat-r2 (windows leg, 2026-10-03): the harness's own stdout crashed on
# Windows runners. The W59 fix decodes the two cores as UTF-8, but the
# harness's own print() still encoded through the platform locale (cp1252),
# so the first diagnostic line containing a non-cp1252 character (U+2713 in
# caps_policy.op) raised UnicodeEncodeError and killed the leg mid-report,
# masking every divergence behind it. Reconfigure once, at startup.
# compat-r3 (2026-10-04): newline="\n" added — with the locale fix alone,
# Windows text-mode writes still translated \n to \r\n through pipes, and
# the Rust core emits LF, so every multi-line golden still diverged.
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace", newline="\n")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace", newline="\n")

# S3 exclusion policy (sz, 2026-09-24), builtins with NO exact-output golden,
# by nature, each accounted for:
#   exit              , control-flow terminator; its exit-code contract is the
#                        harness itself (rust_code == py_code on every program)
#   serve/recv_request/send_response, network server trio; timing-dependent,
#                        containment covered by redteam suite instead
#   repressi_start    , wall-clock thread ticker; manual rings (deterministic)
#                        are covered via repressi_next/repressi_state
# Every other builtin in src/interp.rs call_builtin has >= 1 differential
# golden or a shape contract under tests/differential/.
def run(cmd, timeout=120):
    # W59: decode both cores as UTF-8 explicitly. With bare text=True the
    # decoding uses the platform locale (cp1252 on Windows runners), which
    # both hides real output behind decode failures and can crash the
    # harness itself on any non-ASCII byte. errors="replace" keeps a broken
    # byte comparable instead of fatal.
    # compat-r2 follow-up (windows leg, run 37146021360): decoding was only
    # half the fix — the ORACLE's own stdout still ENCODED through the
    # platform locale, so its em-dash left the process as cp1252 0x97 and
    # the harness (decoding UTF-8) saw a replacement char where the Rust
    # core correctly emitted UTF-8: 9 phantom divergences, every one a
    # non-ASCII golden. Force the oracle's side of the pipe to UTF-8 too.
    env = dict(os.environ)
    env["PYTHONIOENCODING"] = "utf-8"
    env["PYTHONUTF8"] = "1"
    p = subprocess.run(cmd, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=timeout,
                       env=env)
    return p.stdout, p.returncode, p.stderr

def collect_op(root):
    out = []
    for dirpath, _, files in os.walk(root):
        # red-team payloads are adversarial by design (hangs, bombs,
        # escapes): they are exercised by scripts/redteam.sh, never by the
        # differential harness (both implementations would just time out)
        if "redteam" in dirpath:
            continue
        # substrate-r1: capability-granted proofs need an operator cell,
        # the differential harness runs zero-grant by design
        if "granted" in dirpath:
            continue
        for f in sorted(files):
            if f.endswith(".op"):
                out.append(os.path.join(dirpath, f))
    return sorted(out)

# loop-10 (F-7/F-8): granted-with-cell differential targets, the opt-in
# Rho/queue proofs run under an explicit operator cell on BOTH cores.
GRANTED_CELL_TARGETS = [
    ("tests/granted/rho_termination.op", "tests/granted/rho_termination.cell"),
    ("tests/granted/rho_readthrough.op", "tests/granted/rho_readthrough.cell"),
    ("tests/granted/rho_prob.op", "tests/granted/rho_prob.cell"),
    ("tests/granted/rho_queue_shield.op", "tests/granted/rho_queue_shield.cell"),
    ("tests/granted/rho_worker.op", "tests/granted/rho_worker.cell"),
    # W006-B: wave-3 IO try_ family — the granted ok/missing payload pins
    # run 3-lane (rust VM, tree-walk, oracle) under the same operator cell
    ("tests/granted/try_io_wave3.op", "tests/granted/try_io_wave3.cell"),
    # S3: file-bytes write-family ok side (audit_builtins coverage gaps) —
    # real granted IO, self-cleaning pin, same operator-cell discipline
    ("tests/granted/file_bytes_gaps.op", "tests/granted/file_bytes_gaps.cell"),
    # W24: strict visibility, the fixture module exports ONLY pub-marked
    # names; both engines must agree on what is exported and how private
    # reads contain (soft tier).
    ("tests/granted/visibility_strict.op", "tests/granted/visibility_strict.cell"),
]

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(os.path.dirname(__file__), "..", "bin", "operon"))
    ap.add_argument("--root", default=os.path.join(os.path.dirname(__file__), ".."))
    # compat-r2: run one lane alone (main = VM vs oracle, novm = tree-walk vs
    # oracle). Default "both" is unchanged — the full differential contract.
    ap.add_argument("--lane", choices=["both", "main", "novm"], default="both")
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    binpath = os.path.abspath(args.bin)
    oracle = os.path.join(root, "bootstrap", "oracle.py")

    targets = []
    for d in ("tests", "examples", "apps"):
        full = os.path.join(root, d)
        if os.path.isdir(full):
            targets += collect_op(full)

    passed, failed, skipped = 0, 0, 0
    print(f"differential harness, {len(targets)} program(s) × 2 implementations\n")
    if args.lane in ("both", "main"):
        for t in targets:
            rel = os.path.relpath(t, root)
            rust_out, rust_code, _ = run([binpath, "run", t])
            try:
                py_out, py_code, py_err = run([sys.executable, oracle, "run", t])
            except subprocess.TimeoutExpired:
                print(f"  TIMEOUT  {rel} (oracle)")
                failed += 1
                continue
            if rust_out == py_out and rust_code == py_code:
                print(f"  MATCH    {rel}")
                passed += 1
            else:
                print(f"  DIVERGE  {rel}")
                ro = rust_out.strip().splitlines()
                po = py_out.strip().splitlines()
                for i in range(max(len(ro), len(po))):
                    r = ro[i] if i < len(ro) else "<missing>"
                    p_ = po[i] if i < len(po) else "<missing>"
                    if r != p_:
                        print(f"    rust  : {r}")
                        print(f"    oracle: {p_}")
                if rust_code != py_code:
                    print(f"    exit codes: rust={rust_code} oracle={py_code}")
                # W59 diagnostics: an oracle that dies before producing output
                # used to surface as a wall of anonymous DIVERGEs (the oracle's
                # stderr was discarded). If the oracle side failed or went
                # quiet, show the first lines of its stderr, the crash is
                # there, not in the semantics.
                # W59 windows truth (2026-09-29): the old condition also
                # required EMPTY oracle stdout, so a crash AFTER partial output
                # (weak_refs.op: 5 lines, then a dead unix-only import died
                # inside memory()) hid the traceback and stayed anonymous.
                # Surface stderr on ANY oracle failure, not just silent ones.
                if py_code != 0 and py_err.strip():
                    for line in py_err.strip().splitlines()[:6]:
                        print(f"    oracle stderr: {line}")
                failed += 1
        # loop-10 (F-7/F-8): granted-with-cell targets, both implementations
        # under the SAME explicit --cell; stdout must match byte-for-byte like
        # every other target. This pins the Rho layer's ENTROPY-STREAM parity
        # (the opt-in draws are the riskiest divergence surface, trap #4 of
        # the reg-bio-4 kinetics design).
        for rel_op, rel_cell in GRANTED_CELL_TARGETS:
            gop = os.path.join(root, rel_op)
            gcell = os.path.join(root, rel_cell)
            if not (os.path.isfile(gop) and os.path.isfile(gcell)):
                print(f"  FAIL     {rel_op} (missing op or cell, granted targets are checked in, a missing one is a broken tree)")
                failed += 1
                continue
            rust_out, rust_code, _ = run([binpath, "run", gop, "--cell", gcell])
            try:
                py_out, py_code, py_err = run([sys.executable, oracle, "run", gop, "--cell", gcell])
            except subprocess.TimeoutExpired:
                print(f"  TIMEOUT  {rel_op} (oracle)")
                failed += 1
                continue
            if rust_out == py_out and rust_code == py_code:
                print(f"  MATCH    {rel_op} (granted)")
                passed += 1
            else:
                print(f"  DIVERGE  {rel_op} (granted)")
                # W59 windows truth (2026-09-29): the old condition also
                # required EMPTY oracle stdout, so a crash AFTER partial output
                # (weak_refs.op: 5 lines, then a dead unix-only import died
                # inside memory()) hid the traceback and stayed anonymous.
                # Surface stderr on ANY oracle failure, not just silent ones.
                if py_code != 0 and py_err.strip():
                    for line in py_err.strip().splitlines()[:6]:
                        print(f"    oracle stderr: {line}")
                failed += 1
    print(f"\nresult: {passed} match, {failed} diverge, {skipped} skipped")
    # W09 A6 (post-flip): the DEFAULT run IS the bytecode machine, and the
    # main lane above pins it against the oracle. This lane keeps the OTHER
    # engine (the tree-walk, --no-vm) pinned against the same oracle
    # outputs, so both engines stay differentially covered on every step.
    # Pre-flip this lane ran --vm explicitly; the labels inverted at the
    # A6 boundary, the coverage contract (both engines, whole corpus) did
    # not change.
    vm_pass, vm_fail = 0, 0
    print(f"\n--no-vm lane (tree-walk engine vs the same oracle output)")
    if args.lane in ("both", "novm"):
        for t in targets:
            rel = os.path.relpath(t, root)
            vm_out, vm_code, _ = run([binpath, "run", t, "--no-vm"])
            try:
                py_out, py_code, py_err = run([sys.executable, oracle, "run", t])
            except subprocess.TimeoutExpired:
                print(f"  TIMEOUT  {rel} (oracle, tree-walk lane)")
                vm_fail += 1
                continue
            if vm_out == py_out and vm_code == py_code:
                vm_pass += 1
            else:
                print(f"  VM-DIVERGE  {rel}")
                vo = vm_out.strip().splitlines()
                po = py_out.strip().splitlines()
                for i in range(max(len(vo), len(po))):
                    v = vo[i] if i < len(vo) else "<missing>"
                    p_ = po[i] if i < len(po) else "<missing>"
                    if v != p_:
                        print(f"    tree  : {v}")
                        print(f"    oracle: {p_}")
                if vm_code != py_code:
                    print(f"    exit codes: tree={vm_code} oracle={py_code}")
                vm_fail += 1
    print(f"tree-walk lane result: {vm_pass} match, {vm_fail} diverge")
    sys.exit(1 if (failed or vm_fail) else 0)

if __name__ == "__main__":
    main()
