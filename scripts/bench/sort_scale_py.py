#!/usr/bin/env python3
"""sort_scale_py.py — CPython mirror for the P2 sort audit (builder-E).

Same LCG seed-42 stream, same insertion-sort algorithm with a Python-level
comparator callback (callback-bound analog), plus two reference floors:
  builtin  — CPython Timsort at C speed (algorithmic floor)
  cmptokey — Timsort with a Python callback per comparison via cmp_to_key

Output lines match sort_scale.op / sort_scale_rs.rs (SCALE key=value...).
The comparator-call counts must equal the operon and rust counts EXACTLY
for every n (differential law: same LCG -> same permutation -> same
comparison sequence for the same algorithm).
"""
import argparse
import sys
import time


def lcg_list(n):
    xs = []
    x = 42
    for _ in range(n):
        x = (x * 1103515245 + 12345) % 2147483648
        xs.append(x % 1000000)
    return xs


def insertion_sort_callback(xs):
    """Pure-Python insertion sort with a comparator callback — the exact
    algorithm operon's `sorted(xs, cmp)` runs (cmp(a,b) true when a belongs
    before b). Returns (sorted_list, comparator_call_count, checksum)."""
    v = list(xs)
    calls = 0
    for i in range(1, len(v)):
        j = i
        while j > 0:
            calls += 1
            if not (v[j - 1] < v[j]):
                v[j - 1], v[j] = v[j], v[j - 1]
                j -= 1
            else:
                break
    return v, calls, sum(v)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sizes", default="1000,2000,4000,8000")
    ap.add_argument("--reps", type=int, default=3)
    args = ap.parse_args()
    sizes = [int(s) for s in args.sizes.split(",")]

    for n in sizes:
        xs = lcg_list(n)

        # matched-algorithm insertion sort with callback
        best = None
        out = None
        for _ in range(args.reps):
            t0 = time.perf_counter()
            out = insertion_sort_callback(xs)[0]
            dt = (time.perf_counter() - t0) * 1000.0
            best = dt if best is None else min(best, dt)
        v, calls, ck = insertion_sort_callback(xs)
        print(f"SCALE engine=cpython n={n} mode=insertion min_ms={best:.3f} cksum={ck}", flush=True)
        print(f"SCALE engine=cpython n={n} mode=counted calls={calls} cksum={ck}", flush=True)

        # algorithmic floor: builtin Timsort
        best = None
        for _ in range(args.reps):
            t0 = time.perf_counter()
            out = sorted(xs)
            dt = (time.perf_counter() - t0) * 1000.0
            best = dt if best is None else min(best, dt)
        print(f"SCALE engine=cpython n={n} mode=builtin min_ms={best:.3f} cksum={sum(out)}", flush=True)

        # callback-bound floor: Timsort + python-level comparator
        from functools import cmp_to_key
        cmp_calls = 0

        def cmp(a, b):
            nonlocal cmp_calls
            cmp_calls += 1
            return -1 if a < b else (1 if a > b else 0)

        t0 = time.perf_counter()
        out = sorted(xs, key=cmp_to_key(cmp))
        dt = (time.perf_counter() - t0) * 1000.0
        print(f"SCALE engine=cpython n={n} mode=cmptokey min_ms={dt:.3f} calls={cmp_calls} cksum={sum(out)}", flush=True)

    return 0


if __name__ == "__main__":
    sys.exit(main())
