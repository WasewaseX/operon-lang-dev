#!/usr/bin/env bash
# bench_startup.sh — W084: startup-time benchmark (dev-3, M100).
#
# Measures two floors, N runs each, reports min/median/p90 in ms:
#   exec-only : `operon version`  — process spawn + binary init + arg parse
#   run-hello : `operon run examples/hello.op` — the full parse+exec path
#
# Why: for a scripting language, startup cost IS a feature. The bytecode VM
# (W009) inherits this as a non-regression gate: a VM that wins loops but
# loses 50 ms of startup is a net loss for CLI usage.
#
# Usage: scripts/bench_startup.sh [N] [BIN]
# Machine-noise honesty: run on an idle box; the CI perf job (perf.yml)
# compares same-runner pairs, never absolute cross-machine numbers.
set -euo pipefail
cd "$(dirname "$0")/.."
N="${1:-30}"
BIN="${2:-bin/operon}"
PROG="examples/hello.op"
[ -x "$BIN" ] || { echo "bench_startup: no executable at $BIN — build first" >&2; exit 1; }
[ -f "$PROG" ] || { echo "bench_startup: missing $PROG" >&2; exit 1; }

measure() {
    # emits one duration in microseconds per line, N lines
    local i s e
    for i in $(seq 1 "$N"); do
        s=$(date +%s%N)
        "$@" > /dev/null 2>&1 || true
        e=$(date +%s%N)
        echo $(( (e - s) / 1000 ))
    done | python3 -c '
import sys, statistics
xs = sorted(int(l) for l in sys.stdin if l.strip())
if not xs:
    print("no samples"); sys.exit(1)
p90 = xs[min(len(xs) - 1, int(round(0.9 * (len(xs) - 1))))]
print(f"min {xs[0]/1000:.2f}ms  median {statistics.median(xs)/1000:.2f}ms  p90 {p90/1000:.2f}ms  (n={len(xs)})")
'
}

echo "operon startup benchmark (N=$N)"
echo "exec-only: $(measure "$BIN" version)"
echo "run-hello: $(measure "$BIN" run "$PROG")"
echo ""
echo "BENCH.md row (fill from above):"
echo "| startup (exec-only / run-hello) | $(measure "$BIN" version) / $(measure "$BIN" run "$PROG") |"
