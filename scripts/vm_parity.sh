#!/usr/bin/env bash
# vm_parity.sh — W09 gate: every corpus program under --vm must produce
# byte-identical stdout AND stderr vs the tree-walk run (docs/VM.md §7).
# The differential oracle is untouched; this pins VM == tree-walk, which is
# itself pinned == Python oracle by bootstrap/harness.py.
#
# W011 addendum: VM_PARITY_TIMING — payloads whose OUTPUT MESSAGE depends
# on OS scheduler timing (which sandbox cap trips first under load) can
# never be byte-deterministic. rt_p4b_threadbomb_join: the spawn loop races
# its workers; the run flips between the 4096-task cap and the 256-thread
# cap ON THE SAME BINARY (verified run-to-run). For these, the gate still
# requires BOTH engines to end contained ([contained] [overflow]) — the
# sandbox invariant — but not byte-identical text. Everything else in the
# corpus stays byte-exact.
set -uo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-./bin/operon}"
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
pass=0; fail=0; failed_files=()
for f in $(find tests apps -name '*.op' 2>/dev/null | sort); do
  a_out=$("$BIN" run "$f" 2>/tmp/vmp_a_err); a_rc=$?
  a_err=$(cat /tmp/vmp_a_err)
  b_out=$("$BIN" run --vm "$f" 2>/tmp/vmp_b_err); b_rc=$?
  b_err=$(cat /tmp/vmp_b_err)
  if is_timing_case "$f"; then
    # containment must hold on BOTH engines; text may race
    if [[ "$a_err" == *"[contained]"* && "$b_err" == *"[contained]"* ]] && [ "$a_rc" -ne 0 ] && [ "$b_rc" -ne 0 ]; then
      pass=$((pass+1))
    else
      fail=$((fail+1)); failed_files+=("$f (timing case lost containment)")
      if [ "${VERBOSE:-0}" == "1" ]; then
        echo "PARITY FAIL: $f"
        echo "  a_err: $a_err"
        echo "  b_err: $b_err"
      fi
    fi
    continue
  fi
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
