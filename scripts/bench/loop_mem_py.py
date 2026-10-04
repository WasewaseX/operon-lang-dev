#!/usr/bin/env python3
"""loop_mem_py.py — CPython mirror for the P3 loop-memory audit (builder-E).

One leg per invocation (env LOOP_MEM_SHAPE / LOOP_MEM_N / LOOP_MEM_OUTER,
same contract as loop_mem.op). Reports in-process elapsed time, self peak
RSS (resource.ru_maxrss, KB), and the deterministic checksum (sum 0..N-1)
so the cross-engine differential holds. The accumulate leg retains a list
of Python ints — the closest CPython analog to operon's boxed numerics.
"""
import os
import resource
import sys
import time


def main():
    shape = os.environ.get("LOOP_MEM_SHAPE", "")
    n = int(os.environ.get("LOOP_MEM_N", "0"))
    outer = int(os.environ.get("LOOP_MEM_OUTER", "1000") or 1000)
    if n < 1 or not shape:
        print("error: set LOOP_MEM_N and LOOP_MEM_SHAPE")
        return 2

    m0 = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    t0 = time.perf_counter()
    s = 0

    if shape == "acc_list":
        xs = []
        i = 0
        while i < n:
            xs.append(i)
            i += 1
        for v in xs:
            s += v
    elif shape == "transient_while":
        i = 0
        while i < n:
            s += i
            i += 1
    elif shape == "transient_forrange":
        for i in range(n):
            s += i
    elif shape == "transient_while_let":
        i = 0
        while i < n:
            t = i * 2
            s += t
            i += 1
    elif shape == "nested":
        for _ in range(outer):
            j = 0
            while j < n:
                s += j
                j += 1
    else:
        print(f"error: unknown shape {shape}")
        return 2

    dt = (time.perf_counter() - t0) * 1000.0
    m1 = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    hwm = 0
    for line in open("/proc/self/status"):
        if line.startswith("VmHWM"):
            hwm = int(line.split()[1])
    print(f"LOOP engine=cpython shape={shape} n={n} outer={outer} time_ms={dt:.3f} hwm_kb={hwm} rss0_kb={m0} rss1_kb={m1} cksum={s}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
