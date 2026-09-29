#!/usr/bin/env bash
# vm_parity.sh — W09/W11 gate: every corpus program must produce
# byte-identical stdout AND stderr + exit code across all three execution
# modes: tree-walk, bytecode VM (--vm), and optimized VM (--vm-opt).
# (docs/VM.md §7, rule R1 in docs/COMPAT.md: every optimization must
# preserve semantics — enforced here over the whole corpus.)
# The differential oracle is untouched; this pins VM == tree-walk == VM-opt,
# all of which are pinned == Python oracle by bootstrap/harness.py.
set -uo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-./bin/operon}"
# Load-sensitive payloads: the CONTAINMENT is deterministic (exit code 1,
# '[contained]' marker) but WHICH cap trips first is scheduler-dependent:
# under a parallel sweep the task-count cap can beat the OS thread cap,
# serially the reverse. Both messages are legitimate containment outcomes;
# redteam.sh owns this payload's contract. Verified empirically 2026-09-30
# (each engine serially x3: identical 'thread cap' line every time).
LOAD_SENSITIVE="tests/redteam/rt_p4b_threadbomb_join.op"
pass=0; fail=0; lskip=0; failed_files=()
for f in $(find tests apps -name '*.op' 2>/dev/null | sort); do
  if [ "$f" == "$LOAD_SENSITIVE" ]; then
    lskip=$((lskip+1))
    continue
  fi
  a_out=$("$BIN" run "$f" 2>/tmp/vmp_a_err); a_rc=$?
  a_err=$(cat /tmp/vmp_a_err)
  b_out=$("$BIN" run --vm "$f" 2>/tmp/vmp_b_err); b_rc=$?
  b_err=$(cat /tmp/vmp_b_err)
  c_out=$("$BIN" run --vm-opt "$f" 2>/tmp/vmp_c_err); c_rc=$?
  c_err=$(cat /tmp/vmp_c_err)
  if [ "$a_out" == "$b_out" ] && [ "$a_err" == "$b_err" ] && [ "$a_rc" == "$b_rc" ] \
     && [ "$a_out" == "$c_out" ] && [ "$a_err" == "$c_err" ] && [ "$a_rc" == "$c_rc" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed_files+=("$f")
    if [ "${VERBOSE:-0}" == "1" ]; then
      echo "PARITY FAIL: $f (rc $a_rc vs $b_rc vs $c_rc)"
      diff <(printf '%s' "$a_out") <(printf '%s' "$b_out") | head -10
      diff <(printf '%s' "$b_out") <(printf '%s' "$c_out") | head -10
      diff <(printf '%s' "$a_err") <(printf '%s' "$c_err") | head -10
    fi
  fi
done
echo "vm parity (tree-walk/vm/vm-opt): $pass identical, $fail divergent, $lskip load-sensitive (documented)"
if [ "$fail" -gt 0 ]; then
  printf '  divergent: %s\n' "${failed_files[@]}"
  exit 1
fi
echo "VM PARITY GREEN"
