#!/usr/bin/env bash
# sec_regression.sh — W099: one-command containment sweep (dev-3, M100).
#
# Runs the complete security regression surface in the order the
# THREAT-MODEL rows are proven:
#   1. cargo test --release   — unit + sec_regression.rs (TOCTOU, ghost-path
#                               unification, caps) + REPL contracts
#   2. scripts/redteam.sh     — the full containment payload corpus
#   3. caps probes            — default-deny sanity on a fresh binary
#
# CI runs (1)+(2) on every PR via ci.yml; this script exists for the weekly
# fresh-toolchain sweep (security.yml) and for humans: ONE command that must
# be green before any security-adjacent claim.
set -uo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
FAIL=0

echo "== sec_regression sweep =="
echo "[1/3] cargo test --release"
if ! cargo test --release 2>&1 | tail -20; then
    echo "FAIL: cargo test"; FAIL=1
fi

[ -x bin/operon ] || cp target/release/operon bin/operon

echo "[2/3] redteam containment corpus"
if bash scripts/redteam.sh 2>&1 | tail -3; then
    :
else
    echo "FAIL: redteam"; FAIL=1
fi

echo "[3/3] default-deny sanity probes"
denied=$(bin/operon run --quiet tests/security_caps.op 2>&1 && echo OK || echo FAIL)
echo "security_caps.op: ${denied}"
[ "$denied" = "OK" ] || FAIL=1
denied2=$(bin/operon run --quiet tests/caps_policy.op 2>&1 && echo OK || echo FAIL)
echo "caps_policy.op: ${denied2}"
[ "$denied2" = "OK" ] || FAIL=1

if [ "$FAIL" -eq 0 ]; then
    echo "== sec_regression: ALL GREEN =="
else
    echo "== sec_regression: FAILURES PRESENT =="
fi
exit "$FAIL"
