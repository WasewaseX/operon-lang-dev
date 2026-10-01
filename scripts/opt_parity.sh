#!/usr/bin/env bash
# opt_parity.sh — W011 gate: the OPTIMIZER must preserve semantics on every
# corpus program (owner rule: "Every optimization must preserve semantics").
#
# Matrix (each against the tree-walk, byte-identical stdout+stderr+rc):
#   --vm --opt=0 --no-fast   pure W09 baseline (no passes, no fast paths)
#   --vm --opt=0             passes off, engine fast paths on
#   --vm --opt=1 --no-fast   compile passes only
#   --vm --opt=1             DEFAULT configuration
#   --vm --opt=2             propagation added
#   --vm --opt-passes=prop,fold,thread,dce --no-fast  every pass explicit
# Reuses the timing-case contract from vm_parity.sh (see that file for why
# rt_p4b_threadbomb_join is containment-checked rather than byte-checked).
set -uo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-./bin/operon}"
CONFIGS=(
  "--vm --opt=0 --no-fast"
  "--vm --opt=0"
  "--vm --opt=1 --no-fast"
  "--vm --opt=1"
  "--vm --opt=2"
  "--vm --opt-passes=prop,fold,thread,dce --no-fast"
)
VM_PARITY_TIMING=(
  tests/redteam/rt_p4b_threadbomb_join.op
)
is_timing_case() {
  local f="$1"
  for t in "${VM_PARITY_TIMING[@]}"; do
    [ "$f" == "$t" ] && return 0
  done
  return 1
}
total_fail=0
for cfg in "${CONFIGS[@]}"; do
  pass=0; fail=0; failed_files=()
  for f in $(find tests apps -name '*.op' 2>/dev/null | sort); do
    # byte-exact via files + cmp (command substitution drops NULs — a
    # redteam payload prints them; cmp keeps the gate honest)
    "$BIN" run "$f" >/tmp/optp_a.out 2>/tmp/optp_a.err; a_rc=$?
    "$BIN" run $cfg "$f" >/tmp/optp_b.out 2>/tmp/optp_b.err; b_rc=$?
    if is_timing_case "$f"; then
      if grep -q "\[contained\]" /tmp/optp_a.err && grep -q "\[contained\]" /tmp/optp_b.err && [ "$a_rc" -ne 0 ] && [ "$b_rc" -ne 0 ]; then
        pass=$((pass+1))
      else
        fail=$((fail+1)); failed_files+=("$f (timing case lost containment)")
      fi
      continue
    fi
    if cmp -s /tmp/optp_a.out /tmp/optp_b.out && cmp -s /tmp/optp_a.err /tmp/optp_b.err && [ "$a_rc" == "$b_rc" ]; then
      pass=$((pass+1))
    else
      fail=$((fail+1)); failed_files+=("$f")
      if [ "${VERBOSE:-0}" == "1" ]; then
        echo "FAIL [$cfg]: $f (rc $a_rc vs $b_rc)"
        diff /tmp/optp_a.out /tmp/optp_b.out | head -6
        diff /tmp/optp_a.err /tmp/optp_b.err | head -6
      fi
    fi
  done
  echo "[$cfg] parity: $pass identical, $fail divergent"
  if [ "$fail" -gt 0 ]; then
    printf '  divergent: %s\n' "${failed_files[@]}"
  fi
  total_fail=$((total_fail+fail))
done
if [ "$total_fail" -gt 0 ]; then
  echo "OPT PARITY RED ($total_fail divergences)"
  exit 1
fi
echo "OPT PARITY GREEN — optimizer preserves semantics across the corpus"
