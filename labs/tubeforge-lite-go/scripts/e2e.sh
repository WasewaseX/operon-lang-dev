#!/usr/bin/env bash
# TubeForge Lite (Go build) — hermetic end-to-end test (mock yt-dlp/ffmpeg,
# bundled aria2, mocked operon for the policy lane).
# Covers: health/bundle extraction → probe → video job → audio job → cancel →
#         SSE → settings roundtrip → files → CLI mode → doctor → policy
#         (kind denial + size-cap abort + fail-closed when operon missing).
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${BIN:-$ROOT/dist/tubeforge-lite-linux}"
MOCKS="$ROOT/../tubeforge-lite/scripts/mocks"
PORT="${PORT:-8497}"
DATA="$(mktemp -d /tmp/tflite-go.XXXXXX)"
LOG="$DATA/server.log"
FAILS=0

if [ ! -x "$BIN" ]; then echo "e2e: build first (bash build.sh) — no binary at $BIN"; exit 1; fi

ck() { # ck <got> <want> <name>
  if [ "$1" != "$2" ]; then echo "  FAIL $3: got [$1] want [$2]"; FAILS=$((FAILS+1));
  else echo "  ok   $3"; fi
}

jsonget() { python3 -c "import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1], {'d': d}))" "$1" 2>/dev/null; }

# ---------- server phase (mocked tools via settings overrides) ----------
TUBEFORGE_DATA="$DATA" TUBEFORGE_DOWNLOADS="$DATA/dl" \
  "$BIN" serve --port "$PORT" --no-open >"$LOG" 2>&1 &
SRV=$!
trap 'kill $SRV 2>/dev/null' EXIT

for _ in $(seq 1 60); do
  curl -sf -o /dev/null "http://localhost:$PORT/api/tf/health" && break
  sleep 0.2
done

B="http://localhost:$PORT"
echo "== health + aria2 bundle"
H=$(curl -s "$B/api/tf/health")
ck "$(echo "$H" | jsonget 'd["aria2Bundle"]["bundled"]')" "True" "aria2 bundled inside the exe"
ck "$(echo "$H" | jsonget 'd["aria2Bundle"]["version"]')" "1.37.0" "bundle version pinned"
ARIA_PATH=$(echo "$H" | jsonget 'd["tools"][[t["name"]=="aria2c" for t in d["tools"]].index(True)]["path"]' 2>/dev/null)
case "$ARIA_PATH" in *"$DATA/bin/aria2c") ck "yes" "yes" "aria2c resolves to the self-extracted bundle ($ARIA_PATH)";; *) ck "$ARIA_PATH" "$DATA/bin/aria2c" "aria2c resolves to the self-extracted bundle";; esac

echo "== force tool paths to the mocks"
curl -s -XPOST "$B/api/tf/settings" -d "{\"toolPaths\":{\"ytdlp\":\"$MOCKS/yt-dlp\",\"ffmpeg\":\"$MOCKS/ffmpeg\"}}" >/dev/null

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
curl -s -XPOST "$B/api/tf/settings" -d '{"concurrentDownloads":3}' >/dev/null
ck "$(curl -s "$B/api/tf/settings" | jsonget 'd["settings"]["concurrentDownloads"]')" "3" "settings persisted"

echo "== files + raw serving"
FN=$(ls "$DATA/dl" | head -1)
FN_ENC=$(python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))" "$FN")
ck "$(curl -s "$B/api/tf/files" | jsonget 'len(d["files"])')" "2" "files listed"
ck "$(curl -s "$B/api/tf/files/raw?name=$FN_ENC" | wc -c | tr -d ' ')" "200000" "raw file serving"
ck "$(curl -s "$B/api/tf/files/raw?name=$FN_ENC" -H "Range: bytes=0-99" -o /dev/null -w '%{http_code}')" "206" "range request"
ck "$(curl -s "$B/api/tf/files/raw?name=../etc/passwd" -o /dev/null -w '%{http_code}')" "404" "traversal rejected"
ck "$(curl -s -XPOST "$B/api/tf/files" -d "{\"action\":\"delete\",\"name\":\"$FN\"}" | jsonget 'd["ok"]')" "True" "file delete"

kill $SRV 2>/dev/null; wait $SRV 2>/dev/null
trap - EXIT

# ---------- CLI phase ----------
echo "== CLI mode"
DATA2="$(mktemp -d /tmp/tflite-go-cli.XXXXXX)"
mkdir -p "$DATA2"
printf '{"toolPaths":{"ytdlp":"%s","ffmpeg":"%s"}}' "$MOCKS/yt-dlp" "$MOCKS/ffmpeg" > "$DATA2/settings.json"
TUBEFORGE_DATA="$DATA2" "$BIN" \
  "https://example.com/watch?v=mock001" --out "$DATA2/dl" -q >"$DATA2/cli.log" 2>&1
ck "$( [ -s "$DATA2/dl/Mock Video [mock001].mp4" ] && echo yes )" "yes" "CLI downloads file"
grep -q "✓" "$DATA2/cli.log" && ck "yes" "yes" "CLI prints done line" || ck "no" "yes" "CLI prints done line"

echo "== doctor"
DOC=$(TUBEFORGE_DATA="$DATA2" "$BIN" doctor)
ck "$(echo "$DOC" | jsonget 'd["aria2Bundle"]["bundled"]')" "True" "doctor reports bundle"
ck "$(echo "$DOC" | jsonget 'd["policy"]["enabled"]')" "False" "doctor reports policy off by default"

# ---------- policy lane (mocked operon emitting the promote() protocol) ----------
# The REAL operon path (deny-by-default VM, gene-computed cap) was pinned by the
# v1.0.0 Deno e2e; here we pin the Go adapter side of the same protocol.
MOCKBIN="$DATA/fake-operon"
cat > "$MOCKBIN" <<'EOS'
#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "operon 2.9.9-tfpolicy-mock"; exit 0; fi
file="$2"
grep -o 'promote("[^"]*")' "$file" | sed 's/^promote("//; s/")$//'
EOS
chmod +x "$MOCKBIN"

POL_DATA="$(mktemp -d /tmp/tflite-go-pol.XXXXXX)"
POL_PORT=8498
PB="http://localhost:$POL_PORT"
pol_server() { # <policy-file> <port> <operon-bin> → starts server, waits ready
  TUBEFORGE_DATA="$POL_DATA" TUBEFORGE_DOWNLOADS="$POL_DATA/dl" \
  TF_OPERON="$3" TUBEFORGE_POLICY="$1" \
    "$BIN" serve --port "$2" --no-open >>"$POL_DATA/server.log" 2>&1 &
  POL_SRV=$!
  for _ in $(seq 1 60); do
    curl -sf -o /dev/null "http://localhost:$2/api/tf/health" && return 0
    sleep 0.2
  done
}
trap 'kill $SRV $POL_SRV 2>/dev/null' EXIT

echo "== policy: audio-only (kind denial + cap)"
cat > "$POL_DATA/audio-only.op" <<'EOF'
promote("name = audio-only")
promote("version = 1")
promote("allow_video = 0")
promote("allow_audio = 1")
promote("allow_playlist = 1")
promote("max_bytes = 125829120")
promote("reason_video = video downloads are disabled by policy audio-only.op (audio only)")
EOF
pol_server "$POL_DATA/audio-only.op" $POL_PORT "$MOCKBIN"
curl -s -XPOST "$PB/api/tf/settings" -d "{\"toolPaths\":{\"ytdlp\":\"$MOCKS/yt-dlp\",\"ffmpeg\":\"$MOCKS/ffmpeg\"}}" >/dev/null
PH=$(curl -s "$PB/api/tf/health")
ck "$(echo "$PH" | jsonget 'd["policy"]["enabled"]')" "True" "health: policy enabled"
ck "$(echo "$PH" | jsonget 'd["policy"]["name"]')" "audio-only" "health: policy name from .op"
ck "$(echo "$PH" | jsonget 'd["policy"]["maxBytes"]')" "125829120" "health: computed max_bytes"
RV=$(curl -s -o /dev/null -w '%{http_code}' -XPOST "$PB/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"video"}')
ck "$RV" "403" "video denied at enqueue (HTTP 403)"
J=$(curl -s -XPOST "$PB/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"audio","audioFormat":"mp3"}')
ck "$(echo "$J" | jsonget 'd["ok"]')" "True" "audio allowed by policy"
PID2=$(echo "$J" | jsonget 'd["jobs"][0]["id"]')
ck "$(wait_status "$PB" "$PID2" 30)" "completed" "audio job completes under policy"
kill $POL_SRV 2>/dev/null; wait $POL_SRV 2>/dev/null

echo "== policy: size cap (reactive abort)"
printf 'promote("name = tiny-cap")\npromote("max_bytes = 500000")\n' > "$POL_DATA/cap.op"
pol_server "$POL_DATA/cap.op" $POL_PORT "$MOCKBIN"
curl -s -XPOST "$PB/api/tf/settings" -d "{\"toolPaths\":{\"ytdlp\":\"$MOCKS/yt-dlp\",\"ffmpeg\":\"$MOCKS/ffmpeg\"}}" >/dev/null
J=$(curl -s -XPOST "$PB/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"video"}')
ck "$(echo "$J" | jsonget 'd["ok"]')" "True" "video enqueued under cap policy"
PID3=$(echo "$J" | jsonget 'd["jobs"][0]["id"]')
ck "$(wait_status "$PB" "$PID3" 30)" "error" "transfer aborted when total size known"
PE=$(curl -s "$PB/api/tf/queue" | jsonget '[j["error"] for j in d["jobs"] if j["id"]=="'"$PID3"'"][0]')
case "$PE" in *"size cap"*) ck yes yes "error names the size cap";; *) ck "$PE" "size cap msg" "error names the size cap";; esac
kill $POL_SRV 2>/dev/null; wait $POL_SRV 2>/dev/null

echo "== policy: operon missing → fail-closed"
# A policy is configured but the operon binary is gone: everything is refused.
pol_server "$ROOT/../tubeforge-lite/operon/policies/default.op" $POL_PORT "/nonexistent/operon"
PH=$(curl -s "$PB/api/tf/health")
ck "$(echo "$PH" | jsonget 'd["policy"]["failClosed"]')" "True" "health: fail-closed flagged"
RV=$(curl -s -o /dev/null -w '%{http_code}' -XPOST "$PB/api/tf/download" -d '{"url":"https://example.com/watch?v=mock001","kind":"video"}')
ck "$RV" "403" "missing operon denies everything (fail-closed)"
kill $POL_SRV 2>/dev/null; wait $POL_SRV 2>/dev/null
trap 'kill $SRV 2>/dev/null' EXIT
rm -rf "$POL_DATA"

rm -rf "$DATA" "$DATA2"
echo
if [ "$FAILS" -eq 0 ]; then echo "E2E PASS — all checks green"; else echo "E2E FAIL — $FAILS failing checks"; exit 1; fi
