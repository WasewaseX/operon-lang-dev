#!/usr/bin/env bash
# run_tests.sh — hermetic e2e for apps/loglens through the launcher.
# Covers: 4-way byte parity on the small fixture (all subcommands),
# the missing-file exit-2 path on the Operon launcher, and usage output.
set -u
HERE="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

PASS=0
FAIL=0

chk() { # chk NAME EXPECTED_RC ACTUAL_RC
  if [ "$2" = "$3" ]; then
    PASS=$((PASS+1))
  else
    FAIL=$((FAIL+1))
    echo "FAIL $1: rc=$3 want $2"
  fi
}

same() { # same NAME FILE_A FILE_B
  if cmp -s "$2" "$3"; then
    PASS=$((PASS+1))
  else
    FAIL=$((FAIL+1))
    echo "FAIL $1: outputs differ"
  fi
}

SMALL=apps/loglens/test/data/small.log
D=apps/loglens/test/data

for cmd in "stats $SMALL" "top $SMALL status 5" "top $SMALL host 8" \
           "top $SMALL url 8" "errors $SMALL 3" "errors $SMALL" "table $SMALL 4"; do
  ./apps/loglens/loglens.sh $cmd > /tmp/ll_op.out 2>/dev/null
  chk "operon rc [$cmd]" 0 $?
  python3 apps/loglens/loglens.py $cmd > /tmp/ll_py.out 2>/dev/null
  same "parity py [$cmd]" /tmp/ll_op.out /tmp/ll_py.out
  if command -v node >/dev/null 2>&1; then
    node apps/loglens/loglens.js $cmd > /tmp/ll_js.out 2>/dev/null
    same "parity node [$cmd]" /tmp/ll_op.out /tmp/ll_js.out
  fi
  RS=/tmp/loglens_rs_e2e
  if [ ! -x "$RS" ] && [ -x "$HOME/.cargo/bin/rustc" ]; then
    "$HOME/.cargo/bin/rustc" -O apps/loglens/loglens.rs -o "$RS" 2>/dev/null
  fi
  if [ -x "$RS" ]; then
    "$RS" $cmd > /tmp/ll_rs.out 2>/dev/null
    same "parity rust [$cmd]" /tmp/ll_op.out /tmp/ll_rs.out
  fi
done

# missing file -> exit 2 on BOTH engines (byte-identical message)
./apps/loglens/loglens.sh stats /nonexistent/nope.log > /tmp/ll_miss.out 2>/dev/null
chk "operon missing-file rc" 2 $?
python3 apps/loglens/loglens.py stats /nonexistent/nope.log > /tmp/ll_miss_py.out 2>/dev/null
chk "python missing-file rc" 2 $?
same "missing-file message" /tmp/ll_miss.out /tmp/ll_miss_py.out

# unknown field -> exit 2, byte-identical
./apps/loglens/loglens.sh top $SMALL bogus 5 > /tmp/ll_bf.out 2>/dev/null
chk "operon bad-field rc" 2 $?
python3 apps/loglens/loglens.py top $SMALL bogus 5 > /tmp/ll_bf_py.out 2>/dev/null
chk "python bad-field rc" 2 $?
same "bad-field message" /tmp/ll_bf.out /tmp/ll_bf_py.out

# usage (no args) -> rc 0, byte-identical
./apps/loglens/loglens.sh > /tmp/ll_use.out 2>/dev/null
chk "operon usage rc" 0 $?
python3 apps/loglens/loglens.py > /tmp/ll_use_py.out 2>/dev/null
chk "python usage rc" 0 $?
same "usage message" /tmp/ll_use.out /tmp/ll_use_py.out

echo "loglens e2e: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
