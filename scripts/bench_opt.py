#!/usr/bin/env python3
"""bench_opt.py — W011 per-configuration timing (see scripts/bench_opt.sh).

Median-of-N wall clock per (fixture, config). Each fixture prints a
deterministic checksum line; the runner asserts all configs produce
IDENTICAL output before timing (a correctness gate inside the benchmark).
"""
import subprocess
import statistics
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "bin" / "operon"
CORPUS = ROOT / "scripts" / "bench" / "opt"

CONFIGS = [
    ("tree", []),
    ("O0+fast0", ["--vm", "--opt=0", "--no-fast"]),
    ("O0", ["--vm", "--opt=0"]),
    ("O1+fast0", ["--vm", "--opt=1", "--no-fast"]),
    ("O1", ["--vm", "--opt=1"]),
    ("O2", ["--vm", "--opt=2"]),
]

N = 7  # runs per (fixture, config); median reported


def run(cfg_args: list[str], f: Path) -> tuple[str, float]:
    t0 = time.perf_counter()
    p = subprocess.run(
        [str(BIN), "run", *cfg_args, str(f)],
        capture_output=True, text=True, timeout=120,
    )
    dt = time.perf_counter() - t0
    if p.returncode != 0:
        raise RuntimeError(f"{f.name} rc={p.returncode}: {p.stderr[:200]}")
    return p.stdout, dt


def main() -> None:
    quick = "--quick" in sys.argv
    n = 3 if quick else N
    fixtures = sorted(CORPUS.glob("*.op"))
    if not fixtures:
        print(f"no fixtures in {CORPUS}", file=sys.stderr)
        sys.exit(1)
    print(f"operon optimizer benchmarks — median of {n} runs, N={n}")
    print(f"{'fixture':<22}{'tree':>9}{'O0+f0':>9}{'O0':>9}{'O1+f0':>9}{'O1':>9}{'O2':>9}   speedup O1 vs O0+f0")
    for f in fixtures:
        # correctness gate: every config must print the same bytes
        outputs = {}
        times: dict[str, list[float]] = {}
        for name, args in CONFIGS:
            outs, ts = [], []
            for _ in range(n):
                out, dt = run(args, f)
                outs.append(out)
                ts.append(dt)
            outputs[name] = outs[0]
            times[name] = ts
        if len(set(outputs.values())) != 1:
            print(f"{f.name}: OUTPUT MISMATCH across configs — SKIP (bug!)")
            for k, v in outputs.items():
                print(f"   {k}: {v[:80]!r}")
            sys.exit(2)
        row = f"{f.name:<22}"
        for name, _ in CONFIGS:
            row += f"{statistics.median(times[name]) * 1000:>8.1f}m"
        speed = statistics.median(times["O0+fast0"]) / statistics.median(times["O1"])
        row += f"   {speed:.2f}x"
        print(row)


if __name__ == "__main__":
    main()
