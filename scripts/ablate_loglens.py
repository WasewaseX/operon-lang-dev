#!/usr/bin/env python3
# ablate.sh — time each ablate.op stage (medians of 3) and print deltas.
import statistics
import subprocess
import sys
import time

ROOT = "/home/z/my-project/operon-lang-dev"
OP = ROOT + "/target/release/operon"
STAGES = ["read", "lines", "quote", "reqrest", "digits", "records",
          "counts", "sorts"]
CMD = [OP, "run", ROOT + "/scripts/ablate_loglens.op",
       "--cell", ROOT + "/apps/loglens/loglens.cell",
       "--allow-read", ROOT, "--fuel", "20000000000", "--"]


def time_stage(stage, runs=3):
    ts = []
    for _ in range(runs):
        t0 = time.perf_counter()
        p = subprocess.run(CMD + [stage], capture_output=True, cwd=ROOT)
        dt = (time.perf_counter() - t0) * 1000.0
        if p.returncode != 0:
            sys.stderr.write(p.stderr.decode()[:500])
            return None
        ts.append(dt)
    return statistics.median(ts)


def main():
    prev = 0.0
    for st in STAGES:
        ms = time_stage(st)
        if ms is None:
            print(f"{st:8s} ERROR")
            return 1
        print(f"{st:8s} {ms:9.1f} ms   (+{ms - prev:8.1f})")
        prev = ms
    return 0


if __name__ == "__main__":
    sys.exit(main())
