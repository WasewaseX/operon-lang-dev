#!/usr/bin/env bash
# proof_rules_e2e.sh — the §12 proof-runner integrity gate (S4 audit, 2026-10-02).
# SPEC §12: a proof must RUN TO COMPLETION (early return/break fails it,
# "exited early") and must exercise ≥1 assertion (zero asserts fails it,
# "vacuous proof"). Both rules are implemented in the Rust runner
# (src/tools.rs run_tests) since W12-era, but NOTHING exercised them
# negatively — the debug_e2e rot class: a contract no gate probes can rot
# silently. This gate pins, against the real binary:
#   * positive control: a good proof file passes (exit 0),
#   * a zero-assertion proof FAILS with the exact runner message,
#   * an early `return` inside a proof FAILS with the exact runner message,
#   * an early `break` (frame-level, NOT consumed by an enclosing loop —
#     a break inside a for-loop is normal completion) FAILS with the
#     exact runner message,
#   * the failing file's own program output is still captured/normal (the
#     run continues; Total Grammar holds).
# Fixtures are built in a temp dir, NEVER under tests/ (the walker must not
# ingest deliberately-failing files). Oracle scope note: the Python oracle's
# test runner does not enforce the two integrity rules today (REVIEWS
# S4-6b, core lane); this gate pins the canonical Rust `operon test`.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin/operon"
[ -x "$BIN" ] || { echo "SKIP: bin/operon not built"; exit 0; }
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "proof_rules_e2e: FAIL: $1"; exit 1; }

# --- fixtures ----------------------------------------------------------------
cat > "$TMP/good.op" <<'EOF'
frame proof {
    assert(1 + 1 == 2, "arithmetic holds")
}
main {
    promote("body-ok")
}
EOF

cat > "$TMP/vacuous.op" <<'EOF'
frame proof {
    let x = 1
    promote("no assertion ran here")
}
EOF

cat > "$TMP/early_return.op" <<'EOF'
frame proof {
    assert(1 == 1, "first assert runs")
    return 0
    assert(2 == 3, "never reached")
}
EOF

cat > "$TMP/early_break.op" <<'EOF'
frame proof {
    assert(1 == 1, "first assert runs")
    break
    assert(2 == 3, "never reached")
}
EOF

# --- 1. positive control -----------------------------------------------------
out="$("$BIN" test "$TMP/good.op" 2>&1)"; rc=$?
[ $rc -eq 0 ] || fail "positive control failed (rc=$rc): $out"
echo "$out" | grep -q "1 passed" || fail "positive control report shape: $out"

# --- 2. vacuous proof fails with the exact message ---------------------------
out="$("$BIN" test "$TMP/vacuous.op" 2>&1)"; rc=$?
[ $rc -eq 1 ] || fail "vacuous proof must fail the suite (rc=$rc): $out"
echo "$out" | grep -q "no assertion exercised (vacuous proof)" \
  || fail "vacuous message missing: $out"

# --- 3. early return fails with the exact message ----------------------------
out="$("$BIN" test "$TMP/early_return.op" 2>&1)"; rc=$?
[ $rc -eq 1 ] || fail "early return proof must fail the suite (rc=$rc): $out"
echo "$out" | grep -q "exited early" || fail "early-return message missing: $out"

# --- 4. early break fails with the exact message -----------------------------
out="$("$BIN" test "$TMP/early_break.op" 2>&1)"; rc=$?
[ $rc -eq 1 ] || fail "early break proof must fail the suite (rc=$rc): $out"
echo "$out" | grep -q "exited early" || fail "early-break message missing: $out"

# --- 5. the failing file's program output still surfaces ---------------------
out="$("$BIN" test "$TMP/vacuous.op" 2>&1)"
echo "$out" | grep -q "no assertion ran here" \
  || fail "failing file's captured program output not shown: $out"

echo "proof_rules_e2e: ALL GREEN (positive control + 3 negative shapes + output capture)"
