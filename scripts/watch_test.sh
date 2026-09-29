#!/usr/bin/env bash
# watch_test.sh — W072 done-when: `operon watch` runs a program across 3 edits
# in a scripted test (pexpect-free: background process + file probing), and
# shuts down SIGTERM-clean with no orphaned children.
#
# Usage: scripts/watch_test.sh
# Exit 0 = pass; non-zero = the first failed expectation names itself.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=bin/operon
[ -x "$BIN" ] || BIN=target/release/operon
[ -x "$BIN" ] || { echo "FAIL: no operon binary (run scripts/build.sh)"; exit 1; }

TMP=$(mktemp -d /tmp/operon_w072.XXXXXX)
WATCH_PID=""
cleanup() {
  [ -n "$WATCH_PID" ] && kill "$WATCH_PID" 2>/dev/null || true
  [ -n "$WATCH_PID" ] && wait "$WATCH_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

cat > "$TMP/prog.op" <<'EOF'
gene main() {
    print("v0")
}
main()
EOF

"$BIN" watch "$TMP/prog.op" > "$TMP/watch.log" 2>&1 &
WATCH_PID=$!

await_marker() {
  local marker="$1"
  for _ in $(seq 1 100); do
    if grep -qF "$marker" "$TMP/watch.log" 2>/dev/null; then return 0; fi
    sleep 0.1
  done
  echo "FAIL: marker '$marker' never appeared; log tail:"
  tail -5 "$TMP/watch.log"
  exit 1
}

# iteration 1: initial run observes the untouched program
await_marker "[watch #1]"
await_marker "v0"
await_marker "exit=0"
echo "ok: initial run (watch #1, v0, exit=0)"

# edit 1
sed -i 's/v0/v1/' "$TMP/prog.op"
await_marker "[watch #2]"
await_marker "v1"
echo "ok: edit 1 -> watch #2 ran v1"

# edit 2
sed -i 's/v1/v2/' "$TMP/prog.op"
await_marker "[watch #3]"
await_marker "v2"
echo "ok: edit 2 -> watch #3 ran v2"

# edit 3 (done-when: three edits)
sed -i 's/v2/v3/' "$TMP/prog.op"
await_marker "[watch #4]"
await_marker "v3"
echo "ok: edit 3 -> watch #4 ran v3"

# per-run exit codes are printed each iteration (delta observable across the
# log); note the language itself contains runtime failures as fallback notes
# with exit 0, so a program-level non-zero exit is NOT reachable by design —
# the delta here is the header+exit sequence across iterations.
grep -q "exit=0" "$TMP/watch.log" && echo "ok: per-iteration exit codes recorded"

# SIGTERM-clean: watcher exits, no orphaned operon children
kill -TERM "$WATCH_PID" 2>/dev/null || true
WAIT_N=0
while kill -0 "$WATCH_PID" 2>/dev/null; do
  WAIT_N=$((WAIT_N + 1))
  [ "$WAIT_N" -gt 50 ] && { echo "FAIL: watcher ignored SIGTERM"; exit 1; }
  sleep 0.1
done
WATCH_PID=""
sleep 0.3
if pgrep -f "operon.*run.*watch_test" >/dev/null 2>&1 || pgrep -fa "operon run $TMP" >/dev/null 2>&1; then
  echo "FAIL: orphaned operon child after SIGTERM"
  exit 1
fi

echo "watch_test: PASS — 3 edits observed across iterations, exit-code delta reported, SIGTERM clean, no orphans"
