#!/usr/bin/env bash
# dedupe_regression.sh — F4/#48 (reliab lane, 2026-10-02): the cross-run
# crash-signature dedupe DB, pinned end-to-end.
#
# Hermetic (no real crasher, no Rust, no network): a stub "binary" that
# exits 101 on every surface stands in for a live crash. Two deterministic
# fuzz runs (same --seed, same single seed file) prove the contract:
#   run 1 (empty DB)  -> findings are NEW, saved, signatures recorded
#   run 2 (same DB)   -> the identical crash stream is KNOWN-DEDUPE:
#                        zero new findings, nothing re-saved
# and the DB file itself is schema-valid with sorted, deterministic keys.
set -euo pipefail
cd "$(dirname "$0")/../.."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

STUB="$WORK/stub-crash"
printf '#!/bin/sh\nexit 101\n' > "$STUB"
chmod +x "$STUB"
mkdir -p "$WORK/seeds" "$WORK/run1" "$WORK/run2"
printf 'gene f(x) { return x + 1 }\npromote(f(41))\n' > "$WORK/seeds/seed.op"
SIG="$WORK/signatures.json"

FUZZ="python3 scripts/fuzz/fuzz.py --bin $STUB --seed-dir $WORK/seeds \
 --execs 15 --time-budget 60 --per-input-timeout 5 --no-minimize"

set +e
$FUZZ --seed 1 --signatures "$SIG" --corpus-dir "$WORK/run1" > "$WORK/out1.txt" 2>&1
RC1=$?
$FUZZ --seed 1 --signatures "$SIG" --corpus-dir "$WORK/run2" > "$WORK/out2.txt" 2>&1
RC2=$?
set -e

fail() { echo "DEDUPE REGRESSION FAIL: $1"; echo "--- run1 ---"; cat "$WORK/out1.txt"; echo "--- run2 ---"; cat "$WORK/out2.txt"; exit 1; }

[ "$RC1" = 1 ] || fail "run 1 must exit 1 (findings), got $RC1"
grep -q "FINDING #" "$WORK/out1.txt" || fail "run 1 reported no FINDING lines"
[ "$RC2" = 1 ] || fail "run 2 must exit 1 (a live crash matching a triaged signature still fails the run), got $RC2"
grep -q "KNOWN-DEDUPE" "$WORK/out2.txt" || fail "run 2 reported no KNOWN-DEDUPE lines (cross-run dedupe did not fire)"
grep -q "NEW findings saved" "$WORK/out2.txt" && fail "run 2 saved NEW findings (dedupe did not dedupe)"
[ ! -f "$WORK/run2/MANIFEST.jsonl" ] || fail "run 2 wrote a MANIFEST (known re-hits must not be re-saved)"

python3 - "$SIG" <<'PYEOF'
import json, sys
db = json.load(open(sys.argv[1], encoding="utf-8"))
assert db.get("schema") == 1, "schema must be 1"
sigs = db.get("signatures")
assert isinstance(sigs, dict) and len(sigs) >= 1, "run 1 must record >=1 signature"
assert list(sigs.keys()) == sorted(sigs.keys()), "DB keys must be sorted (deterministic bytes)"
for k, v in sigs.items():
    parts = k.split("|")
    assert len(parts) == 4, f"signature must be kind|surface|detail|shape: {k}"
    assert all(p for p in parts), f"signature has an empty component: {k}"
    assert isinstance(v, dict) and "first_seen" in v, f"signature record needs first_seen: {k}"
print(f"ok   signature DB valid: {len(sigs)} triaged signature(s), sorted, 4-part")
PYEOF

echo "ok   dedupe regression: run1 saved new findings; identical run2 = KNOWN-DEDUPE, 0 new, nothing re-saved; DB deterministic"
