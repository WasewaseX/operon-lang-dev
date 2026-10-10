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
# redteam.sh owns these payloads' contracts. Verified empirically 2026-09-30
# (each engine serially x3: identical 'thread cap' line every time).
# Load-sensitive redteam payloads: cap-trip / cancel order is scheduler-
# dependent (containment itself is deterministic; verified x3 x3 engines
# serially). Same documented class as rt_p4a/rt_p4b; rt_p20a joined after
# a 5-run flake measurement (1/5 divergent note ordering, 5/5 contained).
LOAD_SENSITIVE="tests/redteam/rt_p4a_threadbomb.op tests/redteam/rt_p4b_threadbomb_join.op tests/redteam/rt_p20a_cancel_storm.op tests/redteam/rt_p17b_scope_cancel.op tests/redteam/rt_p15b_quorum_unjoined.op"

# rt_p17b_scope_cancel (P2P3P5 wave, 2026-10-06): joins the storm class
# above (rt_p4a/b, rt_p20a). A RACY_NOTE normalization first attempted here
# did NOT survive CI (run 37451404462): that payload's stderr is
# timing-dependent in COUNT and SPLIT, not just note text — its busy loop
# rebinds a name every iteration (0..N "rebinding 's'" notes by when the
# cancel lands; the P3 lazy range shifted the race window) and each of the
# 8 tasks resolves as cancel-requested OR already-finished by race, so the
# normalized line count itself varies. The variant set is not finite —
# normalization cannot honestly pin it — so the payload rides
# rt_p15b_quorum_unjoined (2026-10-10, CI run on the ls-bytechar PR): the
# UNJOINED quorum read is documented in the payload header itself as
# outside the deterministic contract (the read VALUE is explicitly not
# pinned; differential coverage stops at the join boundary). The spawned
# secretor races the host read: 12/12 local runs byte-identical on both
# engines, but the CI debug-build axis hit the opposite ordering once.
# Containment stays redteam.sh's contract (109/0). Same class as the
# storm siblings above.
# LOAD_SENSITIVE like its storm siblings: stdout is deterministic across
# modes (9/9 serial runs byte-identical locally) and containment is
# redteam.sh's contract, which passes (109/0).
#
# #59 (2026-10-11, CLOSED): the scope-reap containment note race is fixed
# at the ROOT — the reap cancel is mechanical containment and narrates
# NOTHING on either engine (cancel_task_quiet in genes.rs; the oracle's
# scope arm mirrors it), so the two variants collapsed to zero lines and
# the RACY_NOTE normalization is RETIRED. scope.op's stderr is now
# byte-compared strictly like every other corpus row. Explicit cancel()
# keeps its documented notes (tests/differential/cancel.op pins them).
pass=0; fail=0; lskip=0; failed_files=()
for f in $(find tests apps -name '*.op' 2>/dev/null | sort); do
  skip=""
  for ls in $LOAD_SENSITIVE; do
    [ "$f" == "$ls" ] && skip=1
  done
  if [ -n "$skip" ]; then
    lskip=$((lskip+1))
    continue
  fi
  a_out=$("$BIN" run --no-vm "$f" 2>/tmp/vmp_a_err); a_rc=$?
  a_err=$(cat /tmp/vmp_a_err)
  b_out=$("$BIN" run "$f" 2>/tmp/vmp_b_err); b_rc=$?
  b_err=$(cat /tmp/vmp_b_err)
  c_out=$("$BIN" run --opt 1 "$f" 2>/tmp/vmp_c_err); c_rc=$?
  c_err=$(cat /tmp/vmp_c_err)
  # W011: the FULL pipeline level rides the same gate (new passes must
  # stay byte-identical over the whole corpus the day they land)
  d_out=$("$BIN" run --opt 2 "$f" 2>/tmp/vmp_d_err); d_rc=$?
  d_err=$(cat /tmp/vmp_d_err)
  if [ "$a_out" == "$b_out" ] && [ "$a_err" == "$b_err" ] && [ "$a_rc" == "$b_rc" ] \
     && [ "$a_out" == "$c_out" ] && [ "$a_err" == "$c_err" ] && [ "$a_rc" == "$c_rc" ] \
     && [ "$a_out" == "$d_out" ] && [ "$a_err" == "$d_err" ] && [ "$a_rc" == "$d_rc" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed_files+=("$f")
    if [ "${VERBOSE:-0}" == "1" ]; then
      echo "PARITY FAIL: $f (rc $a_rc vs $b_rc vs $c_rc vs $d_rc)"
      diff <(printf '%s' "$a_out") <(printf '%s' "$b_out") | head -10
      diff <(printf '%s' "$b_out") <(printf '%s' "$c_out") | head -10
      diff <(printf '%s' "$b_out") <(printf '%s' "$d_out") | head -10
      diff <(printf '%s' "$a_err") <(printf '%s' "$d_err") | head -10
    fi
  fi
done
echo "vm parity (tree-walk/vm/opt1/opt2): $pass identical, $fail divergent, $lskip load-sensitive (documented)"
if [ "$fail" -gt 0 ]; then
  printf '  divergent: %s\n' "${failed_files[@]}"
  exit 1
fi
echo "VM PARITY GREEN"
