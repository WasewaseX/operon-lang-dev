#!/bin/bash
# sort_scale.sh — P2 sort-audit driver (builder-E, lane E).
# Runs one operon process per size (the 200M run-wide step budget, SPEC §9b,
# does not fit a multi-size study in a single invocation), then the CPython
# and native-Rust mirrors. All three engines print the same SCALE lines.
# Usage: scripts/bench/sort_scale.sh [sizes-csv]   (default 1000,2000,4000,8000)
set -u
cd "$(dirname "$0")/../.."
SIZES="${1:-1000,2000,4000,8000}"
OPERON="./bin/operon"
[ -x "$OPERON" ] || OPERON="target/release/operon"

# build the native mirror if needed
RS_BIN="/tmp/sort_scale_rs"
if [ ! -x "$RS_BIN" ] || [ scripts/bench/sort_scale_rs.rs -nt "$RS_BIN" ]; then
    rustc -O scripts/bench/sort_scale_rs.rs -o "$RS_BIN"
fi

for n in ${SIZES//,/ }; do
    # reps: 3 everywhere except the largest size, where 2 keeps the run
    # comfortably inside the 200M step budget per process
    reps=3
    [ "$n" -ge 8000 ] && reps=2
    SORT_SCALE_N="$n" SORT_SCALE_REPS="$reps" "$OPERON" run \
        --allow-env SORT_SCALE_N --allow-env SORT_SCALE_REPS \
        scripts/bench/sort_scale.op 2>/dev/null
done

python3 scripts/bench/sort_scale_py.py --sizes "$SIZES" --reps 3
"$RS_BIN" "$SIZES"
