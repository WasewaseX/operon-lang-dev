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

# --- W008 polish: conditional breakpoints fire only when truthy
OUT10=$(printf 'b 3 if n > 5\nc\nc\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null)
echo "$OUT10" | grep -q "breakpoint at line 3 if n > 5" || { echo "FAIL: conditional bp not added"; exit 1; }
echo "$OUT10" | grep -q "(dbg) line 3" || { echo "FAIL: conditional bp (true cond) did not fire"; exit 1; }
OUT11=$(printf 'b 3 if n > 50\nc\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null)
if echo "$OUT11" | grep -q "(dbg) line 3"; then echo "FAIL: conditional bp (false cond) fired"; exit 1; fi
echo "$OUT11" | grep -q "21" || { echo "FAIL: conditional-false session did not finish"; exit 1; }
# a condition that cannot evaluate counts as NOT firing (never a surprise stop)
OUT12=$(printf 'b 3 if definitely_not_a_binding > 1\nc\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null)
if echo "$OUT12" | grep -q "(dbg) line 3"; then echo "FAIL: broken condition fired"; exit 1; fi
echo "$OUT12" | grep -q "21" || { echo "FAIL: broken-condition session did not finish"; exit 1; }
# b list renders the condition
OUT13=$(printf 'b 3 if n > 5\nb list\nq\n' | "$OP" debug "$SB/dbg.op" --break 7 2>/dev/null || true)
echo "$OUT13" | grep -q "line 3 if n > 5" || { echo "FAIL: b list lost the condition"; exit 1; }

# --- W008 polish: set rebinds a live variable (same reach as assignment)
OUT14=$(printf 'set x 99\np x\nc\n' | "$OP" debug "$SB/dbg.op" --break 2 2>/dev/null)
echo "$OUT14" | grep -q "x = 99" || { echo "FAIL: set did not report the new value"; exit 1; }
echo "$OUT14" | grep -q "99" || { echo "FAIL: p after set does not show the new value"; exit 1; }
echo "$OUT14" | grep -q "line 2 > 100" || { echo "FAIL: the program did not run with the set value (y = 99+1 should print 100)"; exit 1; }
# set on a const is refused (frozen is frozen); set of an unknown name is refused
cat > "$SB/dbgc.op" <<'DBGEOF2'
main {
    const k = 5
    let v = k + 1
    print(v)
}
DBGEOF2
# the refusal messages are errors: they live on stderr
OUT15=$(printf 'set k 9\nset nosuch 1\nq\n' | "$OP" debug "$SB/dbgc.op" --break 3 2>&1 || true)
echo "$OUT15" | grep -q "frozen" || { echo "FAIL: set on const not refused"; exit 1; }
echo "$OUT15" | grep -q "no such binding" || { echo "FAIL: set of unknown name not refused"; exit 1; }

# --- W008 polish: bt shows call-site lines
OUT16=$(printf 'bt\nc\n' | "$OP" debug "$SB/dbg.op" --break 2 2>/dev/null)
echo "$OUT16" | grep -q "at work line 2" || { echo "FAIL: bt missing the inner frame stop line"; exit 1; }
echo "$OUT16" | grep -q "at main line 8" || { echo "FAIL: bt missing the outer frame call-site line"; exit 1; }

echo "DEBUG E2E OK"
