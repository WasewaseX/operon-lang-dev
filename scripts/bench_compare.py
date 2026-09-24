#!/usr/bin/env python3
"""bench_compare.py — measured Rust-core vs Python-oracle timings."""
import subprocess, time, statistics, os, sys
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
def bench(cmd, iters=5):
    ts = []
    for _ in range(iters):
        t0 = time.perf_counter()
        subprocess.run(cmd, capture_output=True, cwd=ROOT)
        ts.append(time.perf_counter() - t0)
    return min(ts), statistics.mean(ts)
print(f"{'bench':<10} {'rust min':>10} {'oracle min':>11} {'speedup':>8}")
for b in ("fib25", "loops", "strings"):
    p = os.path.join(ROOT, "scripts", "bench", f"{b}.op")
    if not os.path.exists(p):
        continue
    rmin, _ = bench([os.path.join(ROOT, "bin", "operon"), "run", p])
    omin, _ = bench([sys.executable, os.path.join(ROOT, "bootstrap", "oracle.py"), "run", p], iters=3)
    print(f"{b:<10} {rmin*1000:>8.1f}ms {omin*1000:>9.1f}ms {omin/rmin:>7.1f}x")
