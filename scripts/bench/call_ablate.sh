#!/usr/bin/env bash
# bench: call_ablate — the P4 same-binary call-path ablation ladder (builder-E)
# Roadmap P4 charter (lane E, AMENDED 2026-10-04): profile gene call frames,
# environment lookup, argument binding, telemetry counters, fuel charging,
# stack allocation/reuse, bridged calls. §34 amendment: the deeper call-frame
# redesign is "gated on fresh same-binary interleaved evidence" — THIS script
# produces exactly that: every W009-A ablation config runs on the SAME release
# binary over the IDENTICAL workload (scripts/bench/call_fib.op, n=25/30),
# interleaved round-robin (rep-major loop) so thermal/CPU drift hits all
# configs equally; per-config min-of-3.
#
# Ablation directions (honesty note): tick/tb/promo/bk/decay/gates REMOVE work
# on the call path (deltas are costs). pool REMOVES the frame-Env recycling
# OPTIMIZATION (delta is the recycling's win — expect SLOWER). workless =
# gates,bk,decay,tick,tb,promo together = the call-core floor (all removable
# work gone, optimizations kept). all = workless + pool-off (not a floor —
# mixed direction, shown for completeness).
#
# The W009-A harness is read via std::env at process start (main.rs), so it
# needs no --allow-env and costs one AtomicBool load per site when off.
# Counters (OPERON_W009A_COUNTS=1) print one stderr line per run; the
# counts-on config doubles as the telemetry-cost leg (atomic adds per call).
#
# Cross-engine differential anchor: call counts are engine-independent
# (C(25)=242,785, C(30)=2,745,165, computed by call_fib.op mode=count and the
# py/rs mirrors); the ladder checks the count-mode output against the analytic
# constant and the counters against it.
set -u
cd "$(dirname "$0")/../.."   # repo root

BIN=${OPERON_BIN:-}
if [ -z "$BIN" ]; then
  if [ -x bin/operon ]; then BIN=bin/operon; else BIN=target/release/operon; fi
fi

OPS="--allow-env CALL_FIB_N --allow-env CALL_FIB_MODE"
REPS=${CALL_ABLATE_REPS:-3}
N25=25
N30=30
CONFIGS=(base tick tb promo bk decay gates pool workless all)

run_ms() {  # n mode cfg -> prints "<n> <cfg> <min_ms>" progressively per rep
  local n=$1 mode=$2 cfg=$3
  local spec=""
  case "$cfg" in
    base)    spec="" ;;
    workless) spec="gates,bk,decay,tick,tb,promo" ;;
    all)     spec="gates,bk,decay,tick,tb,promo,pool" ;;
    *)       spec="$cfg" ;;
  esac
  local ms
  ms=$(CALL_FIB_N="$n" CALL_FIB_MODE="$mode" OPERON_W009A_ABLATE="$spec" \
       "$BIN" run scripts/bench/call_fib.op $OPS 2>/dev/null \
       | sed -n 's/^CALLFIB n=[0-9]* \(r=[0-9]* \|calls_analytic=[0-9]*\)ms=\(.*\)$/\2/p')
  echo "$ms"
}

echo "# call_ablate ladder — bin=$BIN reps=$REPS (min-of-N, in-process clock)"
for n in $N25 $N30; do
  # analytic count anchor (mode=count, one run, no timing)
  cnt=$(CALL_FIB_N="$n" CALL_FIB_MODE=count "$BIN" run scripts/bench/call_fib.op $OPS 2>/dev/null \
        | sed -n 's/^CALLFIB n=[0-9]* calls_analytic=\([0-9]*\)$/\1/p')
  echo "ANALYTIC n=$n calls=$cnt"
  # counters line (base config, counts on — also the telemetry-cost config)
  counters=$(CALL_FIB_N="$n" CALL_FIB_MODE=time OPERON_W009A_COUNTS=1 \
             "$BIN" run scripts/bench/call_fib.op $OPS 2>&1 >/dev/null \
             | grep 'w009a counters:')
  echo "COUNTERS n=$n $counters"
  # interleaved ladder
  declare -A best
  for cfg in "${CONFIGS[@]}"; do best[$cfg]="" ; done
  for rep in $(seq 1 "$REPS"); do
    for cfg in "${CONFIGS[@]}"; do
      ms=$(run_ms "$n" time "$cfg")
      [ -z "$ms" ] && { echo "ERR n=$n cfg=$cfg run failed"; continue; }
      b=${best[$cfg]}
      if [ -z "$b" ] || awk "BEGIN{exit !($ms < $b)}"; then best[$cfg]=$ms; fi
    done
  done
  for cfg in "${CONFIGS[@]}"; do
    echo "LADDER n=$n cfg=$cfg min_ms=${best[$cfg]}"
  done
done
