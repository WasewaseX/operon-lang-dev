#!/usr/bin/env bash
# apps/csvstat e2e — hermetic: runs the CLI through the launcher (sandbox
# grants included) against the committed fixtures and pins exact output.
# Mirrors the apps/ytdl hermetic-e2e pattern (release-gate discipline:
# the app must work as installed, not just in proof frames).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
SMALL="$HERE/data/small.csv"
BIG="$HERE/data/big.csv"
OP="${OPERON_BIN:-$HERE/../../../target/release/operon}"
export OPERON_BIN="$OP"

pass=0
fail=0
expect_ok() { # desc, rc
  if [ "$2" -eq 0 ]; then pass=$((pass+1)); else fail=$((fail+1)); echo "FAIL: $1 (rc=$2)"; fi
}
expect_eq() { # desc, expected, actual
  if [ "$2" = "$3" ]; then pass=$((pass+1)); else fail=$((fail+1)); echo "FAIL: $1"; echo "  want: $2"; echo "  got:  $3"; fi
}

# ---- launcher-based runs (sandbox path) ----
out="$(bash "$HERE/../csvstat.sh" stats "$SMALL" 2>/dev/null)"; rc=$?
expect_ok "launcher stats small" $rc
expect_eq "stats header line" "column,count,nulls,min,max,sum,mean" "$(printf '%s\n' "$out" | sed -n 3p)"

out="$(bash "$HERE/../csvstat.sh" info "$BIG" 2>/dev/null)"; rc=$?
expect_ok "launcher info big" $rc
expect_eq "big rows" "rows,5000" "$(printf '%s\n' "$out" | sed -n 2p)"
expect_eq "big columns" "columns,6" "$(printf '%s\n' "$out" | sed -n 3p)"

out="$(bash "$HERE/../csvstat.sh" top "$SMALL" product 3 2>/dev/null)"; rc=$?
expect_ok "launcher top small" $rc
expect_eq "top header" "rank,value,count,share" "$(printf '%s\n' "$out" | sed -n 2p)"

out="$(bash "$HERE/../csvstat.sh" table "$SMALL" 2 2>/dev/null)"; rc=$?
expect_ok "launcher table small" $rc
expect_eq "table line1" "| 2028     | north  | beta    | 8   | 3782       | ok          |" "$(printf '%s\n' "$out" | sed -n 5p)"

# ---- failure paths ----
bash "$HERE/../csvstat.sh" stats /nonexistent/file.csv > /dev/null 2>&1; rc=$?
[ $rc -ne 0 ] && pass=$((pass+1)) || { fail=$((fail+1)); echo "FAIL: missing file must exit nonzero"; }
out="$(bash "$HERE/../csvstat.sh" stats /nonexistent/file.csv 2>/dev/null)"
expect_eq "missing file message" "csvstat: cannot read /nonexistent/file.csv" "$out"
out="$(bash "$HERE/../csvstat.sh" 2>/dev/null)"
expect_eq "bare usage line1" "usage: csvstat info FILE" "$(printf '%s\n' "$out" | sed -n 1p)"
out="$(bash "$HERE/../csvstat.sh" bogus "$SMALL" 2>/dev/null)"
expect_eq "unknown subcommand = usage" "usage: csvstat info FILE" "$(printf '%s\n' "$out" | sed -n 1p)"

# ---- cross-language byte identity (the app-level differential) ----
for w in "stats" "info" "table" "table 7"; do
  bash "$HERE/../csvstat.sh" $w "$BIG" > /tmp/csvstat_op_big.txt 2>/dev/null
  python3 "$HERE/../csvstat.py" $w "$BIG" > /tmp/csvstat_py_big.txt
  if cmp -s /tmp/csvstat_op_big.txt /tmp/csvstat_py_big.txt; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); echo "FAIL: byte identity big.csv [$w]"
  fi
done
for w in "stats" "info" "top product 3" "top note 10" "table" "table 3"; do
  bash "$HERE/../csvstat.sh" $w "$SMALL" > /tmp/csvstat_op_small.txt 2>/dev/null
  python3 "$HERE/../csvstat.py" $w "$SMALL" > /tmp/csvstat_py_small.txt
  if cmp -s /tmp/csvstat_op_small.txt /tmp/csvstat_py_small.txt; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); echo "FAIL: byte identity small.csv [$w]"
  fi
done

# ---- fixture reproducibility: regenerating must be byte-identical ----
sha_before="$(sha256sum "$SMALL" "$BIG" | awk '{print $1}' | tr '\n' ' ')"
python3 "$HERE/gen_fixture.py" > /dev/null 2>&1
sha_after="$(sha256sum "$SMALL" "$BIG" | awk '{print $1}' | tr '\n' ' ')"
expect_eq "fixture regeneration byte-identical" "$sha_before" "$sha_after"

echo
echo "csvstat e2e: $pass passed, $fail failed"
[ $fail -eq 0 ]
