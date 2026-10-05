#!/usr/bin/env bash
# bench: call_args driver — runs the arity ladder twice (slot path default,
# env path via OPERON_VM_SLOTS=0), min-of-REPS inside the fixture itself
# (the fixture loops reps and prints only the last leg; the shell runs the
# whole fixture REPS times and keeps the min per arity/leg).
set -u
cd "$(dirname "$0")/../.."

BIN=${OPERON_BIN:-}
if [ -z "$BIN" ]; then
  if [ -x bin/operon ]; then BIN=bin/operon; else BIN=target/release/operon; fi
fi

N=${CALL_ARGS_N:-200000}
REPS=${CALL_ARGS_REPS:-3}
OPS="--allow-env CALL_ARGS_N --allow-env CALL_ARGS_REPS"

for lanes in slot env; do
  if [ "$lanes" = "slot" ]; then VMSLOTS=""; else VMSLOTS="0"; fi
  for rep in $(seq 1 "$REPS"); do
    CALL_ARGS_N="$N" CALL_ARGS_REPS="1" OPERON_VM_SLOTS="$VMSLOTS" \
      "$BIN" run scripts/bench/call_args.op $OPS 2>/dev/null \
      | grep '^ARGS ' | sed "s/^/LANE=$lanes /"
  done
done | awk -F'[= ]' '
  /LANE=slot/ { for (i = 1; i <= NF; i++) { if ($i == "arity") ar = $(i+1); if ($i == "ns_per_call") ns = $(i+1) }
                k = "slot," ar; if (!(k in best) || ns < best[k]) best[k] = ns }
  /LANE=env/  { for (i = 1; i <= NF; i++) { if ($i == "arity") ar = $(i+1); if ($i == "ns_per_call") ns = $(i+1) }
                k = "env," ar; if (!(k in best) || ns < best[k]) best[k] = ns }
  END { for (k in best) print "ARGSBEST lanes=" k " ns_per_call=" best[k] }' \
  | sort
