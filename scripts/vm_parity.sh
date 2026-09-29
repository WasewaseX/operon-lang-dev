#!/usr/bin/env bash
# vm_parity.sh — W09 gate: every corpus program under --vm must produce
# byte-identical stdout AND stderr vs the tree-walk run (docs/VM.md §7).
# The differential oracle is untouched; this pins VM == tree-walk, which is
# itself pinned == Python oracle by bootstrap/harness.py.
set -uo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-./bin/operon}"
pass=0; fail=0; failed_files=()
for f in $(find tests apps -name '*.op' 2>/dev/null | sort); do
  a_out=$("$BIN" run "$f" 2>/tmp/vmp_a_err); a_rc=$?
  a_err=$(cat /tmp/vmp_a_err)
  b_out=$("$BIN" run --vm "$f" 2>/tmp/vmp_b_err); b_rc=$?
  b_err=$(cat /tmp/vmp_b_err)
  if [ "$a_out" == "$b_out" ] && [ "$a_err" == "$b_err" ] && [ "$a_rc" == "$b_rc" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed_files+=("$f")
    if [ "${VERBOSE:-0}" == "1" ]; then
      echo "PARITY FAIL: $f (rc $a_rc vs $b_rc)"
      diff <(printf '%s' "$a_out") <(printf '%s' "$b_out") | head -10
      diff <(printf '%s' "$a_err") <(printf '%s' "$b_err") | head -10
    fi
  fi
done
echo "vm parity: $pass identical, $fail divergent"
if [ "$fail" -gt 0 ]; then
  printf '  divergent: %s\n' "${failed_files[@]}"
  exit 1
fi
echo "VM PARITY GREEN"
