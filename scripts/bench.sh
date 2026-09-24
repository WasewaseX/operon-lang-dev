#!/usr/bin/env bash
# bench.sh — Operon benchmark suite (B1, builder-B).
# Runs the six named workloads (fib/loops/strings/collections/recursion/grn)
# on three runners: the Rust core, the Python oracle (identical .op), and a
# native CPython mirror of each algorithm. Pass --micro for per-construct
# micro fixtures, --json PATH for machine-readable results, --quick for a
# fast pass. Full methodology + current numbers: BENCH.md.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
[ -x bin/operon ] || bash scripts/build.sh > /dev/null
echo "operon: $(bin/operon version 2>/dev/null || bin/operon --version 2>/dev/null | head -1)"
echo "python: $(python3 --version)"
echo ""
python3 scripts/bench_compare.py "$@"
