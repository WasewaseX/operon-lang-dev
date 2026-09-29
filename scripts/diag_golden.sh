#!/usr/bin/env bash
# diag_golden.sh — W101 (excellent errors) golden-file gate.
# Pins the rendered fatal diagnostics (SPEC 9a) byte-for-byte so the error
# surface cannot drift silently. The fixtures use entry genes because
# top-level statements are per-statement contained by design (the stress
# becomes a note, not a fatal); a fatal needs the entry-call path.
# rc contract: every case must still exit 1 (dx-r1, a failing program must
# not report success).
set -uo pipefail
cd "$(dirname "$0")/.."
fails=0

check() {
    local fixture="$1" mode="$2" expected="$3"
    local err rc
    if [ "$mode" = "json" ]; then
        err=$(./bin/operon run "$fixture" --json-errors 2>&1 >/dev/null); rc=$?
    else
        err=$(./bin/operon run "$fixture" 2>&1 >/dev/null); rc=$?
    fi
    # notes flush AFTER the block; only the block lines are pinned here.
    # Blank lines are stripped on both sides: the line-0 path renders a
    # separator blank that carries no information.
    err=$(printf '%s\n' "$err" | sed '/^$/d')
    if [ $rc -ne 1 ]; then
        echo "FAIL rc   $fixture ($mode): rc=$rc, want 1"
        fails=$((fails + 1))
        return
    fi
    if [ "$err" != "$(sed '/^$/d' "$expected")" ]; then
        echo "FAIL diff $fixture ($mode):"
        diff <(printf '%s\n' "$err") "$expected" | head -10
        fails=$((fails + 1))
        return
    fi
    echo "ok   $fixture ($mode)"
}

check tests/diagnostics/raise_overflow.op text tests/diagnostics/expected/raise_overflow.txt
check tests/diagnostics/raise_overflow.op json tests/diagnostics/expected/raise_overflow.json
check tests/diagnostics/cap_denied.op     text tests/diagnostics/expected/cap_denied.txt

if [ $fails -gt 0 ]; then
    echo "diag_golden: $fails failure(s); re-derive expected files from real runs, never hand-patch them"
    exit 1
fi
echo "diag_golden: ALL GREEN"
