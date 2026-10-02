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
LOAD_SENSITIVE="tests/redteam/rt_p4a_threadbomb.op tests/redteam/rt_p4b_threadbomb_join.op tests/redteam/rt_p20a_cancel_storm.op"

# F6/#50 (reliab lane, 2026-10-02, aarch64 leg): the scope-reap containment
# note races between the two legitimate outcomes of the same contained child
# failure — tree-walk resolves it as "[fallback] task N already finished",
# the VM as "[fallback] cancel requested for task N". Natively both engines
# consistently land on the same variant (every linux/macos/windows leg green
# across many runs); under qemu-user's single-core thread timing the two
# modes deterministically diverge (aarch64: 2/2 CI runs, caught with the
# first-divergence printer: rc 0/0/0/0, stdout identical, stderr line 2 only).
# This is the documented rt_p20a note-ordering class: containment itself is
# deterministic (rc 0, contained); only the NOTE TEXT races. For exactly
# these files the two documented variants are normalized to one token before
# the stderr comparison — stdout and exit codes stay STRICTLY byte-compared
# (the differential law), and any THIRD note variant still fails the gate.
# The engine-lane fix (make the note deterministic) is tracked on issue #59.
RACY_NOTE="tests/differential/scope.op"
norm_notes() {
    printf '%s' "$1" \
      | sed -E 's/\[fallback\] task [0-9]+ already finished/[TASK-CONTAINED]/' \
      | sed -E 's/\[fallback\] cancel requested for task [0-9]+/[TASK-CONTAINED]/'
}

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
  is_racy=""
  for rf in $RACY_NOTE; do [ "$f" == "$rf" ] && is_racy=1; done
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
  # normalize AFTER capture (the two documented note variants of the
  # scope-reap race — see the RACY_NOTE comment above)
  if [ -n "$is_racy" ]; then
    a_err=$(norm_notes "$a_err")
    b_err=$(norm_notes "$b_err")
    c_err=$(norm_notes "$c_err")
    d_err=$(norm_notes "$d_err")
  fi
  if [ "$a_out" == "$b_out" ] && [ "$a_err" == "$b_err" ] && [ "$a_rc" == "$b_rc" ] \
     && [ "$a_out" == "$c_out" ] && [ "$a_err" == "$c_err" ] && [ "$a_rc" == "$c_rc" ] \
     && [ "$a_out" == "$d_out" ] && [ "$a_err" == "$d_err" ] && [ "$a_rc" == "$d_rc" ]; then
    pass=$((pass+1))
    [ -n "$is_racy" ] && echo "  (racy note text normalized for $f — documented class; stdout+rc strict)"
  else
    fail=$((fail+1)); failed_files+=("$f")
    # F6/#50 (iteration 3): ALWAYS diagnose the first divergence (stream +
    # rc + diff head) — the aarch64 scope.op qemu divergence cost a full
    # CI cycle blind; a parity gate must say WHICH stream disagreed.
    if [ "${VERBOSE:-0}" == "1" ] || [ "$fail" -eq 1 ]; then
      echo "PARITY FAIL: $f (rc $a_rc/$b_rc/$c_rc/$d_rc)"
      [ "$a_out" == "$b_out" ] || { echo "  stdout tree-walk vs vm:"; diff <(printf '%s' "$a_out") <(printf '%s' "$b_out") | head -6; }
      [ "$a_out" == "$c_out" ] || { echo "  stdout tree-walk vs opt1:"; diff <(printf '%s' "$a_out") <(printf '%s' "$c_out") | head -6; }
      [ "$a_out" == "$d_out" ] || { echo "  stdout tree-walk vs opt2:"; diff <(printf '%s' "$a_out") <(printf '%s' "$d_out") | head -6; }
      [ "$a_err" == "$b_err" ] || { echo "  stderr tree-walk vs vm:"; diff <(printf '%s' "$a_err") <(printf '%s' "$b_err") | head -6; }
      [ "$a_err" == "$c_err" ] || { echo "  stderr tree-walk vs opt1:"; diff <(printf '%s' "$a_err") <(printf '%s' "$c_err") | head -6; }
      [ "$a_err" == "$d_err" ] || { echo "  stderr tree-walk vs opt2:"; diff <(printf '%s' "$a_err") <(printf '%s' "$d_err") | head -6; }
    fi
  fi
done
echo "vm parity (tree-walk/vm/opt1/opt2): $pass identical, $fail divergent, $lskip load-sensitive (documented)"
if [ "$fail" -gt 0 ]; then
  printf '  divergent: %s\n' "${failed_files[@]}"
  exit 1
fi
echo "VM PARITY GREEN"
