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
# W099 fix (builder-A finding, session-16): the probe programs PRINT text on
# success ("sandbox: denied as expected"), so capturing combined output and
# comparing it to the sentinel always failed on a green tree. rc-only now:
# the run's exit code is the signal; output is shown, never compared.
denied=OK
bin/operon run --quiet tests/security_caps.op > /tmp/sec_probe1.out 2>&1 || denied=FAIL
echo "security_caps.op: rc-signal ${denied}"
tail -n 1 /tmp/sec_probe1.out
[ "$denied" = "OK" ] || FAIL=1
denied2=OK
bin/operon run --quiet tests/caps_policy.op > /tmp/sec_probe2.out 2>&1 || denied2=FAIL
echo "caps_policy.op: rc-signal ${denied2}"
tail -n 1 /tmp/sec_probe2.out
[ "$denied2" = "OK" ] || FAIL=1

if [ "$FAIL" -eq 0 ]; then
    echo "== sec_regression: ALL GREEN =="
else
    echo "== sec_regression: FAILURES PRESENT =="
fi
exit "$FAIL"
