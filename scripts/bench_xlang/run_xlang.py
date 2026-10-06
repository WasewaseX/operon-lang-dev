#!/usr/bin/env python3
"""run_xlang.py — cross-language benchmark: operon vs Python vs Rust vs Node.js.

Extends the B1 suite methodology (scripts/bench.sh + bench_compare.py) to two
additional runners:

  operon  — the Rust-core interpreter, `bin/operon run <fixture>` (same fixtures)
  python  — native CPython mirror, run as a process (scripts/bench_xlang/native.py)
  rust    — native Rust mirror, rustc -O build (scripts/bench_xlang/native.rs)
  node    — Node.js LTS (the "random mainstream language" pick)

Methodology (identical to BENCH.md):
  * end-to-end process wall time, min over N runs (1 warmup)
  * startup floor measured per runner (m_empty / empty subcommand)
  * correctness anchor: every runner must print the SAME canonical string as
    the operon fixture — a mismatch is a hard FAIL, not a timing row.

Usage:
  python3 scripts/bench_xlang/run_xlang.py [--iters N] [--quick] [--json PATH]
                                           [--filter NAME] [--micro]
"""
import argparse
import json
import os
import platform
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OPERON = os.path.join(ROOT, "bin", "operon")
RS_BIN = os.path.join(HERE, "native_rs")
RS_SRC = os.path.join(HERE, "native.rs")
JS_SRC = os.path.join(HERE, "native.js")
PY_SRC = os.path.join(HERE, "native.py")

NAMED = ["fib25", "loops", "strings", "collections", "recursion", "grn"]
MICROS = ["m_empty", "m_call", "m_forrange", "m_while", "m_varread",
          "m_intadd", "m_listpush", "m_listidx", "m_mapset", "m_mapget", "m_strcat"]

# documented ops per iteration for micro ns/op (from bench_compare.py MICROS)
MICRO_OPS = {
    "m_empty": 0, "m_call": 300000, "m_forrange": 600000, "m_while": 600000,
    "m_varread": 600000, "m_intadd": 900000, "m_listpush": 150000,
    "m_listidx": 300000, "m_mapset": 80000, "m_mapget": 154000, "m_strcat": 24000,
}


def build_rust():
    if os.path.exists(RS_BIN) and os.path.getmtime(RS_BIN) > os.path.getmtime(RS_SRC):
        return
    rustc = os.path.expanduser("~/.cargo/bin/rustc")
    if not os.path.exists(rustc):
        rustc = "rustc"
    subprocess.run([rustc, "-O", "--edition", "2021", "-C", "codegen-units=1",
                    RS_SRC, "-o", RS_BIN], check=True)


def runner_cmd(which, name):
    fixture = os.path.join("scripts", "bench", f"{name}.op")
    if not os.path.exists(os.path.join(ROOT, fixture)):
        micro = os.path.join("scripts", "bench", "micro", f"{name}.op")
        if os.path.exists(os.path.join(ROOT, micro)):
            fixture = micro
    if which == "operon":
        return [OPERON, "run", fixture]
    if which == "python":
        return [sys.executable, PY_SRC, name]
    if which == "rust":
        return [RS_BIN, name]
    if which == "node":
        return ["node", JS_SRC, name]
    raise ValueError(which)


def time_cmd(cmd, cwd, iters):
    for _ in range(1):
        subprocess.run(cmd, capture_output=True, cwd=cwd)
    ts, outs = [], []
    for _ in range(iters):
        t0 = time.perf_counter()
        r = subprocess.run(cmd, capture_output=True, cwd=cwd)
        ts.append(time.perf_counter() - t0)
        outs.append(r.stdout.decode(errors="replace").strip())
    return min(ts), statistics.median(ts), outs[-1]


def cpu_model():
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def main():
    ap = argparse.ArgumentParser(description="operon vs Python vs Rust vs Node")
    ap.add_argument("--iters", type=int, default=5)
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--json", metavar="PATH")
    ap.add_argument("--filter", metavar="NAME", help="run only workloads containing NAME")
    ap.add_argument("--micro", action="store_true", help="micro fixtures instead of named")
    args = ap.parse_args()
    iters = 3 if args.quick else args.iters

    build_rust()
    names = MICROS if args.micro else NAMED
    if args.filter:
        names = [n for n in names if args.filter in n]

    versions = {
        "operon": subprocess.run([OPERON, "version"], capture_output=True, cwd=ROOT,
                                 text=True).stdout.strip(),
        "python": platform.python_version(),
        "rust": subprocess.run([os.path.expanduser("~/.cargo/bin/rustc"), "--version"],
                               capture_output=True, text=True).stdout.strip(),
        "node": subprocess.run(["node", "--version"], capture_output=True,
                               text=True).stdout.strip(),
    }
    meta = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "host": platform.node(),
        "os": platform.platform(),
        "cpu": cpu_model(),
        "versions": versions,
        "iters": iters,
        "timing": "end-to-end process wall time, min over iters (1 warmup)",
    }
    print(f"operon: {versions['operon']}")
    print(f"python: {versions['python']}   rust: {versions['rust']}")
    print(f"node:   {versions['node']}   iters: {iters} (min)")
    print()

    # startup floors
    floors = {}
    for which, probe in [("operon", "m_empty"), ("python", "m_empty"),
                         ("rust", "m_empty"), ("node", "m_empty")]:
        fmin, _, _ = time_cmd(runner_cmd(which, probe), ROOT, iters)
        floors[which] = fmin

    # workload run + correctness cross-check
    results, failures = {}, []
    hdr = [("workload", 12), ("operon", 9), ("python", 9), ("rust", 9), ("node", 9),
           ("op/py", 8), ("op/rs", 8), ("op/js", 8)]
    print("  ".join(f"{c:>{w}}" for c, w in hdr))
    print("  ".join("-" * w for _, w in hdr))

    for name in names:
        row, outs = {}, {}
        for which in ("operon", "python", "rust", "node"):
            mn, md, out = time_cmd(runner_cmd(which, name), ROOT, iters)
            row[which] = {"min": mn, "median": md}
            outs[which] = out
        ref = outs["operon"]
        for which in ("python", "rust", "node"):
            if outs[which] != ref:
                failures.append((name, which, ref, outs[which]))
        op = row["operon"]["min"] * 1000
        cells = [(name, 12)]
        for which in ("operon", "python", "rust", "node"):
            cells.append((f"{row[which]['min'] * 1000:.1f}", 9))
        for which in ("python", "rust", "node"):
            t = row[which]["min"] * 1000
            cells.append((f"{op / t:.1f}x" if t > 0 else "-", 8))
        print("  ".join(f"{c:>{w}}" for c, w in cells))
        results[name] = row

    print()
    print("startup floors (ms): " +
          ", ".join(f"{k} {v * 1000:.1f}" for k, v in floors.items()))
    if failures:
        print(f"\nCORRECTNESS FAILURES: {len(failures)}")
        for name, which, ref, got in failures:
            print(f"  {name} [{which}]: expected {ref!r}, got {got!r}")
        sys.exit(1)
    print("correctness: all runners agree with the operon fixture output")

    if args.json:
        with open(args.json, "w") as f:
            json.dump({"meta": meta, "floors": floors, "results": results}, f, indent=2)
        print(f"JSON written to {args.json}")


if __name__ == "__main__":
    main()
