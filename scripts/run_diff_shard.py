#!/usr/bin/env python3
# run_diff_shard.py — sharded driver for bootstrap/harness.py.
#
# The sandbox kills any process that outlives a single tool call, and the
# full corpus (3477 programs x 2 implementations x 2 lanes) exceeds the
# 10-minute tool ceiling on the 2-core bench box. This driver imports the
# harness module and runs ONE shard (same collect, same compare logic,
# same byte-equality contract); summing all shards == the full corpus.
#
# Usage: python3 scripts/run_diff_shard.py --shard k --of n [--lane both]
import argparse
import importlib.util
import os
import subprocess
import sys

spec = importlib.util.spec_from_file_location(
    "harness", os.path.join(os.path.dirname(__file__), "..", "bootstrap", "harness.py"))
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--shard", type=int, required=True)
    ap.add_argument("--of", type=int, required=True)
    ap.add_argument("--root", default=os.path.join(os.path.dirname(__file__), ".."))
    ap.add_argument("--bin", default=os.path.join(os.path.dirname(__file__), "..", "bin", "operon"))
    ap.add_argument("--lane", choices=["both", "main", "novm"], default="both")
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    binpath = os.path.abspath(args.bin)
    oracle = os.path.join(root, "bootstrap", "oracle.py")

    targets = []
    for d in ("tests", "examples", "apps"):
        full = os.path.join(root, d)
        if os.path.isdir(full):
            targets += harness.collect_op(full)
    targets = sorted(targets)
    targets = [t for i, t in enumerate(targets) if i % args.of == args.shard]

    passed, failed = 0, 0
    print(f"differential shard {args.shard}/{args.of}: {len(targets)} program(s)")
    if args.lane in ("both", "main"):
        for t in targets:
            rel = os.path.relpath(t, root)
            rust_out, rust_code, _ = harness.run([binpath, "run", t])
            try:
                py_out, py_code, py_err = harness.run([sys.executable, oracle, "run", t])
            except subprocess.TimeoutExpired:
                print(f"  TIMEOUT  {rel} (oracle)")
                failed += 1
                continue
            if rust_out == py_out and rust_code == py_code:
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
                failed += 1
    vm_pass, vm_fail = 0, 0
    if args.lane in ("both", "novm"):
        for t in targets:
            rel = os.path.relpath(t, root)
            vm_out, vm_code, _ = harness.run([binpath, "run", t, "--no-vm"])
            try:
                py_out, py_code, py_err = harness.run([sys.executable, oracle, "run", t])
            except subprocess.TimeoutExpired:
                print(f"  TIMEOUT  {rel} (oracle, tree-walk lane)")
                vm_fail += 1
                continue
            if vm_out == py_out and vm_code == py_code:
                vm_pass += 1
            else:
                print(f"  VM-DIVERGE  {rel}")
                vm_fail += 1
    print(f"shard {args.shard}/{args.of}: main lane {passed} match / {failed} diverge; "
          f"tree-walk {vm_pass} match / {vm_fail} diverge")
    sys.exit(1 if (failed or vm_fail) else 0)


if __name__ == "__main__":
    main()
