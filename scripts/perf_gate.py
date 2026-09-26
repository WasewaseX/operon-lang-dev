#!/usr/bin/env python3
"""perf_gate.py — W082: performance regression gate (dev-3, M100).

Compares two bench_compare.py --json outputs (base vs head, SAME runner)
and fails on any tracked-workload regression beyond the threshold.

Design constraints:
  * consumes builder-B's bench_compare.py JSON (B1 artifact) — zero edits to
    that script, this is a separate gate layer
  * compares the `operon` runner only (the oracle/native-py runners are
    reference bars, not regression targets)
  * median-of-runs is assumed from bench_compare; this tool adds a ±5%
    noise band below which differences are reported but never fail
  * threshold default 20% per the M100 W082 acceptance

Exit codes: 0 = within threshold, 1 = regression, 2 = usage/data error.

Usage:
  python3 scripts/perf_gate.py base.json head.json [--threshold 20] [--noise 5]
"""
import argparse
import json
import sys


def load_rows(path):
    """bench_compare.py --json shape:
    {"suites": {name: {"operon": {"min": s, "median": s, ...}, "oracle": ...}},
     "micros": {...}} — times in SECONDS. Convert to ms, prefer median."""
    with open(path) as f:
        data = json.load(f)
    rows = {}
    for group in ("suites", "micros"):
        for name, runners in (data.get(group) or {}).items():
            op = runners.get("operon") if isinstance(runners, dict) else None
            if isinstance(op, dict):
                v = op.get("median", op.get("min"))
                if isinstance(v, (int, float)):
                    rows[name] = v * 1000.0
            elif isinstance(op, (int, float)):
                rows[name] = op * 1000.0
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base")
    ap.add_argument("head")
    ap.add_argument("--threshold", type=float, default=20.0)
    ap.add_argument("--noise", type=float, default=5.0)
    args = ap.parse_args()

    base = load_rows(args.base)
    head = load_rows(args.head)
    if not base or not head:
        print(f"perf_gate: could not parse workload rows from {args.base} / {args.head}", file=sys.stderr)
        return 2

    common = sorted(set(base) & set(head))
    if not common:
        print("perf_gate: no common workloads between base and head", file=sys.stderr)
        return 2

    print(f"perf_gate: threshold {args.threshold:.0f}%  noise band ±{args.noise:.0f}%  ({len(common)} workloads)")
    print(f"{'workload':<16} {'base ms':>10} {'head ms':>10} {'delta':>8}  verdict")
    failed = []
    for name in common:
        b, h = float(base[name]), float(head[name])
        if b <= 0:
            print(f"{name:<16} {b:>10.2f} {h:>10.2f} {'n/a':>8}  skip (base=0)")
            continue
        delta_pct = (h - b) / b * 100.0
        if delta_pct > args.threshold:
            verdict = "REGRESSION"
            failed.append(name)
        elif delta_pct > args.noise:
            verdict = "worse (within noise policy)"
        elif delta_pct < -args.noise:
            verdict = "improved"
        else:
            verdict = "flat"
        print(f"{name:<16} {b:>10.2f} {h:>10.2f} {delta_pct:>+7.1f}%  {verdict}")

    missing = sorted(set(base) - set(head))
    if missing:
        print(f"note: workloads missing from head (not failed): {', '.join(missing)}")

    if failed:
        print(f"perf_gate: FAIL — regression > {args.threshold:.0f}%: {', '.join(failed)}")
        return 1
    print("perf_gate: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
