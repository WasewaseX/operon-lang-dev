#!/usr/bin/env bash
# bench_opt_passes.sh — W011 per-pass benchmark rows (the toggle matrix's
# measured answer). Runs each named workload across the optimizer
# configurations and reports min-of-5 wall time per config, plus the
# differential requirement: every config must print byte-identical
# output (the same R1 rule scripts/vm_parity.sh enforces over the whole
# corpus, applied to the bench fixtures per pass set).
#
# Usage: bash scripts/bench_opt_passes.sh [--quick]
#   --quick: min-of-2 instead of min-of-5 (CI smoke)
set -uo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-./bin/operon}"
RUNS=5
[ "${1:-}" == "--quick" ] && RUNS=2

CONFIGS=(
  "--no-vm|tree-walk"
  "|vm (default)"
  "--opt 1|vm + stage1 (fold,thread,dce)"
  "--opt 2|vm + all (stage1+prop)"
  "--opt-passes fold|vm + fold only"
  "--opt-passes fold,prop|vm + fold,prop"
  "--opt-passes thread|vm + thread only"
  "--opt-passes dce|vm + dce only"
)
WORKLOADS=(scripts/bench/fib25.op scripts/bench/lists.op scripts/bench/loops.op)

printf '%-28s' "workload"
for c in "${CONFIGS[@]}"; do
  printf ' %-22s' "${c#*|}"
done
echo
deltas_seen=0
for w in "${WORKLOADS[@]}"; do
  ref_out=""
  ref_rc=""
  printf '%-28s' "$(basename "$w")"
  for c in "${CONFIGS[@]}"; do
    flags="${c%%|*}"
    best=""
    for _ in $(seq 1 "$RUNS"); do
      t0=$(date +%s%N)
      out=$("$BIN" run "$w" $flags 2>/dev/null); rc=$?
      t1=$(date +%s%N)
      ms=$(( (t1 - t0) / 1000000 ))
      if [ -z "$best" ] || [ "$ms" -lt "$best" ]; then best=$ms; fi
    done
    # differential requirement: identical stdout AND rc across configs
    if [ -z "$ref_out" ]; then
      ref_out="$out"; ref_rc="$rc"
    elif [ "$out" != "$ref_out" ] || [ "$rc" != "$ref_rc" ]; then
      echo "PARITY FAIL on $w [$c]: rc $rc vs $ref_rc, output differs"
      exit 1
    fi
    printf ' %-22s' "${best}ms"
  done
  echo
done
echo "per-pass differential: all configs byte-identical (stdout+rc) on every workload"
echo "BENCH_OPT_PASSES_GREEN"
