#!/usr/bin/env python3
"""harness.py — differential test harness.

Runs every .op program through BOTH implementations (Rust core binary and the
Python oracle) and fails on any stdout divergence. This is the evidence engine
that keeps the two implementations honest.

Usage: python3 bootstrap/harness.py [--bin ../bin/operon]
"""
import subprocess, sys, os, argparse

# S3 exclusion policy (sz, 2026-09-24) — builtins with NO exact-output golden,
# by nature, each accounted for:
#   exit               — control-flow terminator; its exit-code contract is the
#                        harness itself (rust_code == py_code on every program)
#   serve/recv_request/send_response — network server trio; timing-dependent,
#                        containment covered by redteam suite instead
#   repressi_start     — wall-clock thread ticker; manual rings (deterministic)
#                        are covered via repressi_next/repressi_state
# Every other builtin in src/interp.rs call_builtin has >= 1 differential
# golden or a shape contract under tests/differential/.
def run(cmd, timeout=120):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.stdout, p.returncode

def collect_op(root):
    out = []
    for dirpath, _, files in os.walk(root):
        # red-team payloads are adversarial by design (hangs, bombs,
        # escapes): they are exercised by scripts/redteam.sh, never by the
        # differential harness (both implementations would just time out)
        if "redteam" in dirpath:
            continue
        for f in sorted(files):
            if f.endswith(".op"):
                out.append(os.path.join(dirpath, f))
    return sorted(out)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(os.path.dirname(__file__), "..", "bin", "operon"))
    ap.add_argument("--root", default=os.path.join(os.path.dirname(__file__), ".."))
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
    print(f"differential harness — {len(targets)} program(s) × 2 implementations\n")
    for t in targets:
        rel = os.path.relpath(t, root)
        rust_out, rust_code = run([binpath, "run", t])
        try:
            py_out, py_code = run([sys.executable, oracle, "run", t])
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
            failed += 1
    print(f"\nresult: {passed} match, {failed} diverge, {skipped} skipped")
    sys.exit(1 if failed else 0)

if __name__ == "__main__":
    main()
