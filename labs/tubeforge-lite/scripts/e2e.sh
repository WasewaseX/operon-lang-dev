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
wait_status() { # id timeout → echoes final status
  local id="$1" t="${2:-30}" st=""
  for _ in $(seq 1 $((t * 5))); do
    st=$(curl -s "$B/api/tf/queue" | jsonget '[j["status"] for j in d["jobs"] if j["id"]=="'"$id"'"][0]' 2>/dev/null)
    case "$st" in completed|error|canceled) break;; esac
    sleep 0.2
  done
  echo "$st"
}
ST=$(wait_status "$ID" 30)
ck "$ST" "completed" "video job completes"
VP=$(curl -s "$B/api/tf/queue" | jsonget '[j["filePath"] for j in d["jobs"] if j["id"]=="'"$ID"'"][0]')
ck "$( [ -s "$VP" ] && echo yes )" "yes" "video file exists and non-empty ($VP)"

echo "== audio job"
J=$(curl -s -XPOST "$B/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"audio","audioFormat":"mp3","title":"Mock Video"}')
ID=$(echo "$J" | jsonget 'd["jobs"][0]["id"]')
ST=$(wait_status "$ID" 30)
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
ST=$(wait_status "$ID" 10)
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

rm -rf "$DATA" "$DATA2"
echo
if [ "$FAILS" -eq 0 ]; then echo "E2E PASS — all checks green"; else echo "E2E FAIL — $FAILS failing checks"; exit 1; fi
