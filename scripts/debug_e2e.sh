#!/usr/bin/env bash
# debug_e2e.sh — the W08 debugger done-when proof. W08 phase 1: a scripted
# session breaks at a line, inspects the frame, evaluates an expression,
# steps, and resumes to completion. Piped stdin must never wedge.
# W08r stage 1: next (step-over does NOT descend into the called gene),
# finish (step-out lands back in the caller), until N (one-shot
# continue-to-line), and live breakpoint management (b N / b del N / b list).
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
    let z = 1
    let a = work(10)
    print(a)
}
DBGEOF

# --- phase-1 contract: break fires, vars/p/s/c work, EOF resumes
OUT=$(printf 'vars\np x * 3\ns\nc\nc\n' | "$OP" debug "$SB/dbg.op" --break 3 2>/dev/null)
echo "$OUT" | grep -q "x = 20" || { echo "FAIL: vars missing x"; exit 1; }
echo "$OUT" | grep -q "60" || { echo "FAIL: p x * 3 != 60"; exit 1; }
echo "$OUT" | grep -q "21" || { echo "FAIL: program output missing"; exit 1; }

# --- step mode then quit
OUT2=$(printf 's\nq\n' | "$OP" debug "$SB/dbg.op" --break 3 2>/dev/null || true)
echo "$OUT2" | grep -q "(dbg)" || { echo "FAIL: no second trap"; exit 1; }

# --- EOF resumes to completion
OUT3=$(printf '' | "$OP" debug "$SB/dbg.op" --break 3 2>/dev/null)
echo "$OUT3" | grep -q "21" || { echo "FAIL: EOF did not resume"; exit 1; }

# --- W08r: next steps OVER the call (trap at line 8, never inside work)
OUT4=$(printf 'n\nc\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null)
echo "$OUT4" | grep -q "(dbg) line 8" || { echo "FAIL: next did not land on line 8"; exit 1; }
if echo "$OUT4" | grep -q "(dbg) line 3"; then echo "FAIL: next descended into work"; exit 1; fi
echo "$OUT4" | grep -q "21" || { echo "FAIL: next session did not finish"; exit 1; }

# --- W08r: finish steps OUT of a gene (break inside work, fin -> caller)
OUT5=$(printf 'fin\nc\n' | "$OP" debug "$SB/dbg.op" --break 2 2>/dev/null)
echo "$OUT5" | grep -q "(dbg) line 8" || { echo "FAIL: finish did not return to the caller"; exit 1; }
echo "$OUT5" | grep -q "21" || { echo "FAIL: finish session did not finish"; exit 1; }

# --- W08r: until N is a one-shot continue-to-line
OUT6=$(printf 'until 9\nc\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null)
echo "$OUT6" | grep -q "(dbg) line 9" || { echo "FAIL: until 9 did not stop at line 9"; exit 1; }
COUNT=$(echo "$OUT6" | grep -c "(dbg) line 9")
[ "$COUNT" = "1" ] || { echo "FAIL: until target re-fired ($COUNT traps at 9)"; exit 1; }

# --- W08r: live breakpoint management
OUT7=$(printf 'b 9\nb list\nc\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null)
echo "$OUT7" | grep -q "breakpoint at line 9" || { echo "FAIL: b 9 not added"; exit 1; }
echo "$OUT7" | grep -q "(dbg) line 9" || { echo "FAIL: runtime-added bp 9 did not fire"; exit 1; }
OUT8=$(printf 'b del 9\nb list\nq\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null || true)
echo "$OUT8" | grep -q "no breakpoint at line 9" || { echo "FAIL: b del of a missing bp not reported"; exit 1; }
echo "$OUT8" | grep -q "  line 7" || { echo "FAIL: b list lost the original --break 7"; exit 1; }
OUT9=$(printf 'b 9\nb del 9\nb del 7\nb list\nq\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null || true)
echo "$OUT9" | grep -q "deleted line 9" || { echo "FAIL: b del 9 not confirmed"; exit 1; }
echo "$OUT9" | grep -q "deleted line 7" || { echo "FAIL: b del 7 not confirmed"; exit 1; }
echo "$OUT9" | grep -q "(no breakpoints)" || { echo "FAIL: b list not empty after del all"; exit 1; }

echo "DEBUG E2E OK"
