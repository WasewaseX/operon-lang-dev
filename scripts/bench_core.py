#!/usr/bin/env python3
"""bench_core.py — W011-r2 standing benchmark: interleaved A/B measurement.

Interleaves the baseline binary and the patched binary run-by-run (and the
native CPython mirror) so box contention hits both sides equally; reports
min/median per side. This is the "check benchmarks every time" harness for
the optimization loop. Baseline = git HEAD binary (built once into /tmp by
build.sh -- the caller passes both paths explicitly).
"""
import subprocess
import sys
import time
import statistics

def build(rev, out):
    subprocess.run(["git", "stash", "create", rev], check=False, capture_output=True)
    return out

def timeit(cmd, n):
    ts = []
    out = None
    for _ in range(n):
        t0 = time.perf_counter()
        r = subprocess.run(cmd, capture_output=True, text=True)
        ts.append((time.perf_counter() - t0) * 1000)
        out = r.stdout.strip()
    return ts, out

def interleaved(cmd_a, cmd_b, n):
    a, b = [], []
    for _ in range(n):
        t0 = time.perf_counter()
        ra = subprocess.run(cmd_a, capture_output=True, text=True)
        a.append((time.perf_counter() - t0) * 1000)
        t0 = time.perf_counter()
        rb = subprocess.run(cmd_b, capture_output=True, text=True)
        b.append((time.perf_counter() - t0) * 1000)
        if ra.stdout != rb.stdout:
            print("OUTPUT MISMATCH!", ra.stdout, rb.stdout)
            sys.exit(2)
    return a, b

def report(name, a, b, py=None):
    ma, mb = statistics.median(a), statistics.median(b)
    line = f"{name:22s} base {ma:8.1f} ms  new {mb:8.1f} ms  speedup {ma/mb:5.2f}x"
    if py is not None:
        mp = statistics.median(py)
        line += f"   (cpython {mp:.1f} ms, base {ma/mp:4.1f}x py, new {mb/mp:4.1f}x py)"
    print(line)
    return ma / mb

def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 7
    base_bin = sys.argv[2] if len(sys.argv) > 2 else "/tmp/operon_base"
    new_bin = "./bin/operon"
    fixtures = sys.argv[3:] if len(sys.argv) > 3 else ["/tmp/fib27.op"]

    for fx in fixtures:
        ca = [base_bin, "run", fx]
        cb = [new_bin, "run", fx]
        # warmup 2 each
        for c in (ca, cb):
            subprocess.run(c, capture_output=True)
        a, b = interleaved(ca, cb, n)
        name = fx.split("/")[-1]
        report(name, a, b)

if __name__ == "__main__":
    main()
