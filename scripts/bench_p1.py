#!/usr/bin/env python3
"""P1 (map representation) measurement runner.

Runs the scripts/bench/p1/ probe family through `operon bench` (in-process
min/avg per run; startup excluded by the engine), R repeats per probe, takes
the min of mins. Emits JSON + a scaling table for the build family.

Method notes:
- operon bench times main() IN-PROCESS, so the subprocess-startup dilution
  lesson (BENCH.md: quote in-process mins) does not apply here; the reported
  min is engine-internal wall time.
- Every probe's stdout must match the expected promote() line (output
  mismatch = hard fail, the interleaved-A/B discipline).

Usage: python3 scripts/bench_p1.py [--bin bin/operon] [--repeats 3] [--iters 15]
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
P1 = ROOT / "scripts" / "bench" / "p1"

# probe file -> expected stdout (promote) regex
PROBES = {
    "p1_maplit.op": r"^n = 180000$",
    "p1_mapmiss.op": r"^a = -50000$",
    "p1_mapiter.op": r"^a = 159960000$",
    "p1_mapdel.op": r"^n = 2000$",
    "p1_countmap.op": r"^h = 800 u = 2000 s = 8$",
}
BUILD_SIZES = [1000, 2000, 4000, 8000]
BUILD_RE = re.compile(r"^n = (\d+)$")


def gen_build(size: int) -> Path:
    tpl = (P1 / "p1_mapbuild.op").read_text()
    f = P1 / f"p1_mapbuild_{size}.op"
    f.write_text(tpl.replace("__SIZE__", str(size)))
    return f


def run_probe(binary: str, path: Path, iters: int, expect: re.Pattern) -> float:
    best = None
    for _ in range(args.repeats):
        r = subprocess.run(
            [binary, "bench", str(path), "--iters", str(iters)],
            capture_output=True, text=True, cwd=ROOT,
        )
        if r.returncode != 0:
            sys.exit(f"FAIL {path.name}: rc={r.returncode}\n{r.stderr[-2000:]}")
        lines = r.stdout.strip().splitlines()
        if not lines or "operon bench:" not in lines[-1]:
            sys.exit(f"FAIL {path.name}: no bench line in {r.stdout!r}")
        m = re.search(r"min ([0-9.]+) ms", lines[-1])
        if not m:
            sys.exit(f"FAIL {path.name}: no min in {lines[-1]!r}")
        # every iter's promote output must match (output mismatch = hard fail)
        promote = [l for l in lines[:-1] if l.strip()]
        if len(promote) != iters or any(not expect.match(l) for l in promote):
            sys.exit(f"FAIL {path.name}: output mismatch: {promote[:3]!r}...")
        v = float(m.group(1))
        best = v if best is None else min(best, v)
    return best


def main() -> None:
    global args
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=str(ROOT / "bin" / "operon"))
    ap.add_argument("--repeats", type=int, default=3)
    ap.add_argument("--iters", type=int, default=15)
    ap.add_argument("--out", default="")
    args = ap.parse_args()

    results = {}
    for probe, expect in PROBES.items():
        pat = re.compile(expect)
        ms = run_probe(args.bin, P1 / probe, args.iters, pat)
        results[probe.replace("p1_", "").replace(".op", "")] = ms

    build = {}
    tpl_expect = re.compile(r"^n = (\d+)$")
    for size in BUILD_SIZES:
        f = gen_build(size)
        ms = run_probe(args.bin, f, args.iters, tpl_expect)
        build[str(size)] = ms
        f.unlink()
    results["build"] = build

    ratios = {}
    bs = BUILD_SIZES
    for a, b in zip(bs, bs[1:]):
        ratios[f"{b}/{a}"] = round(build[str(b)] / build[str(a)], 2)
    results["build_scaling_ratio"] = ratios

    print(json.dumps(results, indent=2))
    print("\n# build scaling t(2N)/t(N): " + ", ".join(
        f"{k}={v}" for k, v in ratios.items()), file=sys.stderr)
    if args.out:
        Path(args.out).write_text(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
