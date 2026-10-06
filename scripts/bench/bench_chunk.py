#!/usr/bin/env python3
"""bench_chunk.py — run a SUBSET of the bench suite in one foreground call.

The sandbox reaps background processes, so long bench passes must run as
several foreground calls. This driver reuses bench_compare's runners verbatim
(same fixtures, same min-over-iters methodology) for a caller-chosen subset,
writing a partial JSON that bench_merge.py stitches together.

Usage:
  python3 scripts/bench/bench_chunk.py --named --names fib25,loops --json /tmp/p1.json
  python3 scripts/bench/bench_chunk.py --micro --names m_call,m_intadd --json /tmp/p2.json
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import bench_compare as bc  # noqa: E402


def main():
    ap = argparse.ArgumentParser(description="chunked subset of the Operon bench suite")
    ap.add_argument("--named", action="store_true", help="run named workloads")
    ap.add_argument("--micro", action="store_true", help="run micro fixtures")
    ap.add_argument("--names", default="", help="comma list to keep (empty = all in scope)")
    ap.add_argument("--iters", type=int, default=5)
    ap.add_argument("--json", required=True)
    args = ap.parse_args()

    iters = args.iters
    oiters = max(3, iters // 2)
    keep = set(n for n in args.names.split(",") if n)
    out = {"meta": bc.meta(iters)}

    if args.named:
        suites = {}
        for name, path, native, _calls, extra in bc.SUITES:
            if keep and name not in keep:
                continue
            full = os.path.join(bc.ROOT, path)
            if not os.path.exists(full):
                print(f"skip {name}: {path} missing", file=sys.stderr)
                continue
            omin, omed = bc.run_operon(path, iters, extra)
            rmin, rmed = bc.run_oracle(path, oiters, extra)
            nmin, nmed = bc.time_fn(native, iters)
            suites[name] = {
                "operon": {"min": omin, "median": omed, "iters": iters},
                "oracle": {"min": rmin, "median": rmed, "iters": oiters},
                "native": {"min": nmin, "median": nmed, "iters": iters},
            }
            print(f"done {name}: op {omin*1000:.1f} ms, oracle {rmin*1000:.1f} ms, "
                  f"native {nmin*1000:.1f} ms", flush=True)
        out["suites"] = suites

    if args.micro:
        micros = {}
        for name, _ops, native in bc.MICROS:
            if keep and name not in keep:
                continue
            path = os.path.join(bc.ROOT, "scripts/bench/micro", f"{name}.op")
            if not os.path.exists(path):
                print(f"skip {name}: missing", file=sys.stderr)
                continue
            omin, omed = bc.run_operon(path, iters)
            rmin, rmed = bc.run_oracle(path, oiters)
            nmin, nmed = bc.time_fn(native, iters)
            micros[name] = {
                "operon": {"min": omin, "median": omed, "iters": iters},
                "oracle": {"min": rmin, "median": rmed, "iters": oiters},
                "native": {"min": nmin, "median": nmed, "iters": iters},
            }
            print(f"done {name}: op {omin*1000:.1f} ms, oracle {rmin*1000:.1f} ms, "
                  f"native {nmin*1000:.1f} ms", flush=True)
        out["micros"] = micros

    with open(args.json, "w") as f:
        json.dump(out, f, indent=2)
    print(f"JSON written to {args.json}", flush=True)


if __name__ == "__main__":
    main()
