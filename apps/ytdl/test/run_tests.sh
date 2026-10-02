#!/usr/bin/env bash
# run_tests.sh — ytdl test battery: e2e against deterministic mocks +
# cross-language differential (operon vs python vs deno) on the decision
# matrix, sort order, and info header lines.
#
#   usage: apps/ytdl/test/run_tests.sh [path-to-operon-binary]
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="$HERE/.."
REPO="$APP/../.."
OP="${1:-$REPO/target/release/operon}"
PY="$(command -v python3 || true)"
DENO="$(command -v deno || true)"
FIXTURE="$HERE/mock/fixtures/video1.json"

PASS=0
FAIL=0
ok()   { PASS=$((PASS+1)); echo "  ok  - $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL- $1"; }
check() { # check NAME CONDITION(0=pass)
  if [ "$2" = 0 ]; then ok "$1"; else bad "$1"; fi
}

# mock PATH: yt-dlp/ffmpeg/aria2c are the deterministic stubs
export PATH="$HERE/mock:$PATH"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP" "$REPO/downloads"' EXIT

RUN="$OP run $APP/ytdl.op --cell $APP/ytdl.cell \
  --allow-run yt-dlp --allow-run ffmpeg --allow-run aria2c \
  --allow-read $PWD --allow-read $APP --allow-read $TMP \
  --allow-write $TMP \
  --fuel 20000000000"

PYAPP="$REPO/apps/ytdl-compare/python/ytdl.py"
BSHAPP="$REPO/apps/ytdl-compare/bash/ytdl.sh"

echo "== ytdl: operon e2e (mock engines) =="

# 1. doctor
out="$($RUN -- doctor 2>&1)"; rc=$?
check "doctor exits 0" $rc
echo "$out" | grep -q "2026.08.19-mock"; check "doctor finds mock yt-dlp" $?
echo "$out" | grep -q "ffmpeg version 4.4.2-mock"; check "doctor finds mock ffmpeg" $?
echo "$out" | grep -q "aria2 1.37.0-mock"; check "doctor finds mock aria2c" $?

# 2. info
out="$($RUN -- info mock://video1 2>&1)"; rc=$?
check "info exits 0" $rc
echo "$out" | grep -q "title: Mock Video One"; check "info title" $?
echo "$out" | grep -q "duration: 1:23:41"; check "info duration fmt" $?
echo "$out" | grep -q "views: 1,234,567"; check "info views comma" $?
echo "$out" | grep -q "3840x2160"; check "info has 4k row" $?
echo "$out" | grep -q "accel: aria2c"; check "info accel line" $?

# 3. get (happy path)
rm -rf "$TMP/dl1"
out="$($RUN -- get mock://video1 --out "$TMP/dl1" 2>&1)"; rc=$?
check "get exits 0" $rc
[ -f "$TMP/dl1/video1.mp4" ]; check "get produced file" $?
echo "$out" | grep -q "attempts: 1"; check "get single attempt" $?
echo "$out" | grep -q "out=$TMP/dl1"; check "get honored --out" $?

# 4. get --audio (extract path)
rm -rf "$TMP/dl2"
out="$($RUN -- get mock://video1 --out "$TMP/dl2" --audio mp3 2>&1)"; rc=$?
check "get --audio exits 0" $rc
[ -f "$TMP/dl2/video1.mp3" ]; check "get --audio produced mp3" $?
echo "$out" | grep -q -- "--audio-format mp3"; check "get --audio selector shown" $?

# 5. checkpoint-resume (stall: attempt 1 leaves .part + exit 0)
rm -rf "$TMP/dl3"
out="$($RUN -- get mock://stall --out "$TMP/dl3" --max-attempts 3 2>&1)"; rc=$?
check "stall get exits 0" $rc
[ -f "$TMP/dl3/stall.mp4" ]; check "stall produced final file" $?
[ ! -f "$TMP/dl3/stall.mp4.part" ]; check "no .part left after resume" $?
echo "$out" | grep -q "attempts: 2"; check "resume took 2 attempts" $?

# 6. failure path
rm -rf "$TMP/dl4"
out="$($RUN -- get mock://fail --out "$TMP/dl4" --max-attempts 2 2>&1)"; rc=$?
[ "$rc" = 1 ]; check "fail exit code is 1" $?
echo "$out" | grep -q "result: FAIL"; check "fail reported FAIL" $?

# 7. queue with concurrency
rm -rf "$TMP/dl5"
printf '# comment line\nmock://video1\nmock://video2\nmock://video3\n' > "$TMP/q.txt"
out="$($RUN -- queue "$TMP/q.txt" --jobs 3 --out "$TMP/dl5" 2>&1)"; rc=$?
check "queue exits 0" $rc
[ "$(echo "$out" | grep -c '^ok')" = 3 ]; check "queue 3 items ok" $?
[ -f "$TMP/dl5/video3.mp4" ]; check "queue produced 3rd file" $?
echo "$out" | grep -q "workers: 3"; check "queue used 3 workers" $?

# 8. queue with a failing item -> exit 1
rm -rf "$TMP/dl6"
printf 'mock://video1\nmock://fail\n' > "$TMP/q2.txt"
out="$($RUN -- queue "$TMP/q2.txt" --jobs 2 --out "$TMP/dl6" 2>&1)"; rc=$?
[ "$rc" = 1 ]; check "queue with failure exits 1" $?

# 9. bad quality tier -> usage error exit 2
$RUN -- get mock://video1 --quality 999 >/dev/null 2>&1
[ "$?" = 2 ]; check "bad --quality exits 2" $?

# 10. unknown subcommand
$RUN -- frobnicate >/dev/null 2>&1
[ "$?" = 2 ]; check "unknown subcommand exits 2" $?

echo "== ytdl: differential (decision matrix byte-identical) =="

# operon selfcheck
$RUN -- selfcheck "$FIXTURE" > "$TMP/self.op" 2>"$TMP/self.op.err"
rc=$?
[ "$rc" = 0 ]; check "operon selfcheck runs" $?

# python selfcheck
if [ -n "$PY" ]; then
  "$PY" "$PYAPP" selfcheck "$FIXTURE" > "$TMP/self.py" 2>/dev/null
  rc=$?
  [ "$rc" = 0 ]; check "python selfcheck runs" $?
  diff -u "$TMP/self.op" "$TMP/self.py" > "$TMP/diff.op-py" 2>&1
  check "operon == python (byte-identical decisions)" $?
  if [ "$?" != 0 ] && false; then cat "$TMP/diff.op-py"; fi
else
  bad "python3 not found (skipping python differential)"
fi

# deno selfcheck (if deno exists on this machine)
if [ -n "$DENO" ]; then
  "$DENO" run --allow-read "$REPO/apps/ytdl-compare/deno/ytdl.ts" selfcheck "$FIXTURE" > "$TMP/self.ts" 2>/dev/null
  rc=$?
  [ "$rc" = 0 ]; check "deno selfcheck runs" $?
  diff -u "$TMP/self.op" "$TMP/self.ts" >/dev/null 2>&1
  check "operon == deno (byte-identical decisions)" $?
else
  echo "  (deno not installed here — deno differential runs on any deno machine)"
fi

# bash selfcheck must refuse honestly
bash "$BSHAPP" selfcheck "$FIXTURE" >/dev/null 2>&1
[ "$?" = 2 ]; check "bash selfcheck refuses (no JSON parser)" $?

# bash get against mock still works
rm -rf "$TMP/dl7"
bash "$BSHAPP" get mock://video1 --out "$TMP/dl7" >/dev/null 2>&1
rc=$?
[ "$rc" = 0 ] && [ -f "$TMP/dl7/video1.mp4" ]; check "bash get e2e (mock)" $?

echo ""
echo "== ytdl summary: $PASS passed, $FAIL failed =="
[ "$FAIL" = 0 ]
