#!/usr/bin/env bash
# debug_e2e.sh — the W08 phase-1 done-when proof: a scripted session breaks
# at a line, inspects the frame, evaluates an expression, steps, and
# resumes to completion. Piped stdin must never wedge.
set -euo pipefail
cd "$(dirname "$0")/.."
OP="$(pwd)/bin/operon"
SB=$(mktemp -d)
trap 'rm -rf "$SB"' EXIT

cat > "$SB/dbg.op" <<'DBGEOF'
gene work(n) {
    let x = n * 2
    let y = x + 1
    return y
}
main {
    let a = work(10)
    print(a)
}
DBGEOF

OUT=$(printf 'vars\np x * 3\ns\nc\nc\n' | "$OP" debug "$SB/dbg.op" --break 3 2>/dev/null)
echo "$OUT" | grep -q "x = 20" || { echo "FAIL: vars missing x"; exit 1; }
echo "$OUT" | grep -q "60" || { echo "FAIL: p x * 3 != 60"; exit 1; }
echo "$OUT" | grep -q "21" || { echo "FAIL: program output missing"; exit 1; }

# step mode then quit
OUT2=$(printf 's\nq\n' | "$OP" debug "$SB/dbg.op" --break 3 2>/dev/null || true)
echo "$OUT2" | grep -q "(dbg)" || { echo "FAIL: no second trap"; exit 1; }

# EOF resumes to completion
OUT3=$(printf '' | "$OP" debug "$SB/dbg.op" --break 3 2>/dev/null)
echo "$OUT3" | grep -q "21" || { echo "FAIL: EOF did not resume"; exit 1; }

echo "DEBUG E2E OK"
