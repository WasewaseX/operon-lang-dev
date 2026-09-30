#!/usr/bin/env bash
# diag_golden.sh — W101 (excellent errors) golden-file gate.
# Pins the rendered diagnostics (SPEC 9a.1) byte-for-byte so the error
# surface cannot drift silently. The fatal fixtures use entry genes because
# top-level statements are per-statement contained by design (the stress
# becomes a note, not a fatal); a fatal needs the entry-call path.
# rc contract: fatal `run` cases exit 1 (dx-r1, a failing program must
# not report success); lint findings exit 0 (advisory); check hard errors
# exit 3 (W48 escalation). W101 slice 2: lint/check finding blocks are
# pinned by the same gate.
set -uo pipefail
cd "$(dirname "$0")/.."
fails=0

# check <fixture> <mode> <expected> [subcommand] [want_rc] [extra args]
check() {
    local fixture="$1" mode="$2" expected="$3"
    local sub="${4:-run}" want_rc="${5:-1}" extra="${6:-}"
    local err rc flag=""
    if [ "$mode" = "json" ]; then
        if [ "$sub" = "run" ]; then flag="--json-errors"; else flag="--json"; fi
    fi
    if [ "$sub" = "run" ]; then
        # fatal blocks print to stderr; stdout may carry program output
        err=$(./bin/operon run "$fixture" $flag $extra 2>&1 >/dev/null); rc=$?
    else
        # lint/check print to stdout; keep stderr out of the golden
        err=$(./bin/operon "$sub" "$fixture" $flag $extra 2>/dev/null); rc=$?
    fi
    # notes flush AFTER the block; only the block lines are pinned here.
    # Blank lines are stripped on both sides: the line-0 path renders a
    # separator blank that carries no information.
    err=$(printf '%s\n' "$err" | sed '/^$/d')
    if [ $rc -ne "$want_rc" ]; then
        echo "FAIL rc   $fixture ($mode): rc=$rc, want $want_rc"
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
check tests/diagnostics/unused_binding.op text tests/diagnostics/expected/unused_binding.txt lint 0
check tests/diagnostics/wrong_arity.op    text tests/diagnostics/expected/wrong_arity.txt    check 3
# W101 slice 6: the typo'd entry is a fatal with did-you-mean (rc 1, dx-r1);
# the phantom is a located, suggested, machine-applicable-fix warning.
check tests/diagnostics/entry_typo.op      text tests/diagnostics/expected/entry_typo.txt      run 1 "--entry maiin"
check tests/diagnostics/entry_typo.op      json tests/diagnostics/expected/entry_typo.json    run 1 "--entry maiin"
check tests/diagnostics/phantom_suggest.op text tests/diagnostics/expected/phantom_suggest.txt check 0
# W101 slice 7: parse/repair notes carry derived E2xxx codes in the flush;
# the program itself SUCCEEDS (repair, never reject → rc 0).
check tests/diagnostics/repair_notes.op    text tests/diagnostics/expected/repair_notes.txt run 0

if [ $fails -gt 0 ]; then
    echo "diag_golden: $fails failure(s); re-derive expected files from real runs, never hand-patch them"
    exit 1
fi
echo "diag_golden: ALL GREEN"
