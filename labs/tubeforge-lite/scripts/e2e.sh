#!/usr/bin/env bash
# TubeForge Lite — hermetic end-to-end test (mock yt-dlp/ffmpeg, bundled aria2).
# Covers: health/bundle extraction → probe → video job → audio job → cancel →
#         SSE → settings roundtrip → files → CLI mode → doctor.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DENO="${DENO:-$HOME/.deno/bin/deno}"
PORT="${PORT:-8497}"
DATA="$(mktemp -d /tmp/tflite.XXXXXX)"
LOG="$DATA/server.log"
FAILS=0

# dev-run needs a real bundle in the module graph (compiled binaries carry their own)
"$DENO" run -A "$ROOT/scripts/gen-aria2-bundle.ts" --platform linux >/dev/null

ck() { # ck <got> <want> <name>
  if [ "$1" != "$2" ]; then echo "  FAIL $3: got [$1] want [$2]"; FAILS=$((FAILS+1));
  else echo "  ok   $3"; fi
}

jsonget() { python3 -c "import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1], {'d': d}))" "$1" 2>/dev/null; }

# ---------- server phase (mocked tools on PATH) ----------
export PATH="$ROOT/scripts/mocks:$PATH"
TUBEFORGE_DATA="$DATA" TUBEFORGE_DOWNLOADS="$DATA/dl" \
  "$DENO" run -A "$ROOT/src/main.ts" serve --port "$PORT" --no-open >"$LOG" 2>&1 &
SRV=$!
trap 'kill $SRV 2>/dev/null' EXIT

for _ in $(seq 1 60); do
  curl -sf -o /dev/null "http://localhost:$PORT/api/tf/health" && break
  sleep 0.2
done

B="http://localhost:$PORT"
echo "== health + aria2 bundle"
H=$(curl -s "$B/api/tf/health")
ck "$(echo "$H" | jsonget 'all(t["ok"] for t in d["tools"])')" "True" "all tools ok (yt-dlp/ffmpeg on PATH-ish dirs, aria2 from bundle)"

echo "== force tool paths to the mocks (resolveTool prefers ~/.venv/bin over PATH)"
curl -s -XPOST "$B/api/tf/settings" -d "{\"toolPaths\":{\"ytdlp\":\"$ROOT/scripts/mocks/yt-dlp\",\"ffmpeg\":\"$ROOT/scripts/mocks/ffmpeg\"}}" >/dev/null

echo "== probe"
P=$(curl -s -XPOST "$B/api/tf/probe" -d '{"url":"https://example.com/watch?v=mock001"}')
ck "$(echo "$P" | jsonget 'd["result"]["title"]')" "Mock Video" "probe title"
ck "$(echo "$P" | jsonget 'len(d["result"]["formats"])')" "5" "probe formats"
ck "$(echo "$P" | jsonget 'sorted(d["result"]["subtitles"])')" "['de', 'en']" "probe subtitles"

echo "== video job"
J=$(curl -s -XPOST "$B/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"video","quality":720,"title":"Mock Video"}')
ID=$(echo "$J" | jsonget 'd["jobs"][0]["id"]')
wait_status() { # <base> id timeout → echoes final status
  local base="$1" id="$2" t="${3:-30}" st=""
  for _ in $(seq 1 $((t * 5))); do
    st=$(curl -s "$base/api/tf/queue" | jsonget '[j["status"] for j in d["jobs"] if j["id"]=="'"$id"'"][0]' 2>/dev/null)
    case "$st" in completed|error|canceled) break;; esac
    sleep 0.2
  done
  echo "$st"
}
ST=$(wait_status "$B" "$ID" 30)
ck "$ST" "completed" "video job completes"
VP=$(curl -s "$B/api/tf/queue" | jsonget '[j["filePath"] for j in d["jobs"] if j["id"]=="'"$ID"'"][0]')
ck "$( [ -s "$VP" ] && echo yes )" "yes" "video file exists and non-empty ($VP)"

echo "== audio job"
J=$(curl -s -XPOST "$B/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"audio","audioFormat":"mp3","title":"Mock Video"}')
ID=$(echo "$J" | jsonget 'd["jobs"][0]["id"]')
ST=$(wait_status "$B" "$ID" 30)
ck "$ST" "completed" "audio job completes"
AP=$(curl -s "$B/api/tf/queue" | jsonget '[j["filePath"] for j in d["jobs"] if j["id"]=="'"$ID"'"][0]')
ck "$( [ -s "$AP" ] && [ "${AP##*.}" = "mp3" ] && echo yes )" "yes" "audio mp3 file exists ($AP)"

echo "== cancel"
J=$(curl -s -XPOST "$B/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001#slow","kind":"video","title":"Slow Mock"}')
ID=$(echo "$J" | jsonget 'd["jobs"][0]["id"]')
for _ in $(seq 1 25); do
  st=$(curl -s "$B/api/tf/queue" | jsonget '[j["status"] for j in d["jobs"] if j["id"]=="'"$ID"'"][0]' 2>/dev/null)
  [ "$st" = "downloading" ] && break
  sleep 0.2
done
curl -s -XPOST "$B/api/tf/cancel" -d "{\"id\":\"$ID\"}" >/dev/null
ST=$(wait_status "$B" "$ID" 10)
ck "$ST" "canceled" "cancel during download"

echo "== SSE stream"
S=$(curl -sN --max-time 2 "$B/api/tf/events" | head -c 40)
ck "$( echo "$S" | cut -c1-6 )" "data: " "SSE emits snapshots"

echo "== settings roundtrip"
curl -s -XPOST "$B/api/tf/settings" -d '{"concurrentDownloads":3,"toolPaths":{"ytdlp":"'"$ROOT"'/scripts/mocks/yt-dlp","ffmpeg":"'"$ROOT"'/scripts/mocks/ffmpeg"}}' >/dev/null
ck "$(curl -s "$B/api/tf/settings" | jsonget 'd["settings"]["concurrentDownloads"]')" "3" "settings persisted"

echo "== files + raw serving"
FN=$(ls "$DATA/dl" | head -1)
ck "$(curl -s "$B/api/tf/files" | jsonget 'len(d["files"])' | grep -Eo '[0-9]+')" "2" "files listed"
ck "$(curl -s "$B/api/tf/files/raw?name=$(python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))" "$FN")" | wc -c | tr -d ' ')" "200000" "raw file serving"
ck "$(curl -s -XPOST "$B/api/tf/files" -d "{\"action\":\"delete\",\"name\":\"$FN\"}" | jsonget 'd["ok"]')" "True" "file delete"

kill $SRV 2>/dev/null; wait $SRV 2>/dev/null
trap - EXIT

# ---------- CLI phase ----------
echo "== CLI mode"
DATA2="$(mktemp -d /tmp/tflite-cli.XXXXXX)"
mkdir -p "$DATA2"
printf '{"toolPaths":{"ytdlp":"%s","ffmpeg":"%s"}}' "$ROOT/scripts/mocks/yt-dlp" "$ROOT/scripts/mocks/ffmpeg" > "$DATA2/settings.json"
TUBEFORGE_DATA="$DATA2" "$DENO" run -A "$ROOT/src/main.ts" \
  "https://example.com/watch?v=mock001" --out "$DATA2/dl" -q >"$DATA2/cli.log" 2>&1
ck "$( [ -s "$DATA2/dl/Mock Video [mock001].mp4" ] && echo yes )" "yes" "CLI downloads file"
grep -q "✓" "$DATA2/cli.log" && ck "yes" "yes" "CLI prints done line" || ck "no" "yes" "CLI prints done line"

echo "== doctor"
DOC=$(TUBEFORGE_DATA="$DATA2" "$DENO" run -A "$ROOT/src/main.ts" doctor)
ck "$(echo "$DOC" | jsonget 'd["aria2Bundle"]["bundled"]')" "True" "doctor reports bundle"
ck "$(echo "$DOC" | jsonget 'd["policy"]["enabled"]')" "False" "doctor reports policy off by default"

# ---------- operon policy phase (real operon binary in the loop) ----------
# The policy is an .op script executed by Operon's deny-by-default VM;
# these checks pin: kind denial, allow, size-cap abort, fail-closed.
OPERON_BIN="${OPERON_BIN:-$ROOT/../../target/release/operon}"
if [ -x "$OPERON_BIN" ]; then
  pol_server() { # <policy-file> <port> <operon-bin> → starts server, waits ready
    TUBEFORGE_DATA="$POL_DATA" TUBEFORGE_DOWNLOADS="$POL_DATA/dl" \
    TF_OPERON="$3" TUBEFORGE_POLICY="$1" \
      "$DENO" run -A "$ROOT/src/main.ts" serve --port "$2" --no-open >>"$POL_DATA/server.log" 2>&1 &
    POL_SRV=$!
    for _ in $(seq 1 60); do
      curl -sf -o /dev/null "http://localhost:$2/api/tf/health" && return 0
      sleep 0.2
    done
  }
  POL_DATA="$(mktemp -d /tmp/tflite-pol.XXXXXX)"
  POL_PORT=8498
  PB="http://localhost:$POL_PORT"
  trap 'kill $SRV $POL_SRV 2>/dev/null' EXIT

  echo "== policy: audio-only.op"
  pol_server "$ROOT/operon/policies/audio-only.op" $POL_PORT "$OPERON_BIN"
  curl -s -XPOST "$PB/api/tf/settings" -d "{\"toolPaths\":{\"ytdlp\":\"$ROOT/scripts/mocks/yt-dlp\",\"ffmpeg\":\"$ROOT/scripts/mocks/ffmpeg\"}}" >/dev/null
  PH=$(curl -s "$PB/api/tf/health")
  ck "$(echo "$PH" | jsonget 'd["policy"]["enabled"]')" "True" "health: policy enabled"
  ck "$(echo "$PH" | jsonget 'd["policy"]["name"]')" "audio-only" "health: policy name from .op"
  ck "$(echo "$PH" | jsonget 'd["policy"]["maxBytes"]')" "125829120" "health: computed max_bytes (gene mib(120))"
  RV=$(curl -s -o /dev/null -w '%{http_code}' -XPOST "$PB/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"video"}')
  ck "$RV" "403" "video denied at enqueue (HTTP 403)"
  J=$(curl -s -XPOST "$PB/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"audio","audioFormat":"mp3"}')
  ck "$(echo "$J" | jsonget 'd["ok"]')" "True" "audio allowed by policy"
  PID2=$(echo "$J" | jsonget 'd["jobs"][0]["id"]')
  ck "$(wait_status "$PB" "$PID2" 30)" "completed" "audio job completes under policy"
  kill $POL_SRV 2>/dev/null; wait $POL_SRV 2>/dev/null

  echo "== policy: size cap (reactive abort)"
  printf 'promote("name = tiny-cap")\npromote("max_bytes = 500000")\n' > "$POL_DATA/cap.op"
  pol_server "$POL_DATA/cap.op" $POL_PORT "$OPERON_BIN"
  curl -s -XPOST "$PB/api/tf/settings" -d "{\"toolPaths\":{\"ytdlp\":\"$ROOT/scripts/mocks/yt-dlp\",\"ffmpeg\":\"$ROOT/scripts/mocks/ffmpeg\"}}" >/dev/null
  J=$(curl -s -XPOST "$PB/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"video"}')
  ck "$(echo "$J" | jsonget 'd["ok"]')" "True" "video enqueued under cap policy"
  PID3=$(echo "$J" | jsonget 'd["jobs"][0]["id"]')
  ck "$(wait_status "$PB" "$PID3" 30)" "error" "transfer aborted when total size known"
  PE=$(curl -s "$PB/api/tf/queue" | jsonget '[j["error"] for j in d["jobs"] if j["id"]=="'"$PID3"'"][0]')
  case "$PE" in *"size cap"*) ck yes yes "error names the size cap";; *) ck "$PE" "size cap msg" "error names the size cap";; esac
  kill $POL_SRV 2>/dev/null; wait $POL_SRV 2>/dev/null

  echo "== policy: operon missing → fail-closed"
  # Script content cannot make operon exit non-zero (total grammar + stress
  # containment + capability-gated exit), so the fail-closed path guards the
  # INFRASTRUCTURE: a policy is configured but the operon binary is gone.
  pol_server "$ROOT/operon/policies/default.op" $POL_PORT "/nonexistent/operon"
  PH=$(curl -s "$PB/api/tf/health")
  ck "$(echo "$PH" | jsonget 'd["policy"]["failClosed"]')" "True" "health: fail-closed flagged"
  RV=$(curl -s -o /dev/null -w '%{http_code}' -XPOST "$PB/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"video"}')
  ck "$RV" "403" "missing operon denies everything (fail-closed)"
  kill $POL_SRV 2>/dev/null; wait $POL_SRV 2>/dev/null
  trap 'kill $SRV 2>/dev/null' EXIT
  rm -rf "$POL_DATA"
else
  echo "  skip operon policy phase (no operon binary at $OPERON_BIN)"
fi

rm -rf "$DATA" "$DATA2"
echo
if [ "$FAILS" -eq 0 ]; then echo "E2E PASS — all checks green"; else echo "E2E FAIL — $FAILS failing checks"; exit 1; fi
