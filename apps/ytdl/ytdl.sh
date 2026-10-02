#!/usr/bin/env bash
# ytdl — lightweight YouTube downloader: Operon runtime (~2.5 MB) + PATH engines.
# This launcher is the security boundary: it grants the Operon sandbox exactly
# yt-dlp/ffmpeg/aria2c + the output directory — nothing else.
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
OP="${OPERON_BIN:-$HERE/../../target/release/operon}"
if [ ! -x "$OP" ]; then
  OP="$(command -v operon || true)"
fi
if [ -z "${OP:-}" ]; then
  echo "ytdl: operon runtime not found (set OPERON_BIN or put operon on PATH)" >&2
  exit 2
fi

# pre-scan --out (both --out DIR and --out=DIR) so the write grant stays narrow
OUT="downloads"
PREV=""
for a in "$@"; do
  case "$PREV" in
    --out) OUT="$a" ;;
  esac
  case "$a" in
    --out=*) OUT="${a#--out=}" ;;
  esac
  PREV="$a"
done
mkdir -p "$OUT"

# extra grants (comma-separated programs) for power users:
#   YTDL_EXTRA_RUN=python3 ./ytdl.sh get ...
EXTRA_RUN=""
if [ -n "${YTDL_EXTRA_RUN:-}" ]; then
  IFS=',' read -ra _parts <<< "$YTDL_EXTRA_RUN"
  for p in "${_parts[@]}"; do EXTRA_RUN="$EXTRA_RUN --allow-run $p"; done
fi

exec "$OP" run "$HERE/ytdl.op" \
  --cell "$HERE/ytdl.cell" \
  --allow-run yt-dlp --allow-run ffmpeg --allow-run aria2c $EXTRA_RUN \
  --allow-read "$PWD" --allow-read "$OUT" --allow-read "$HERE" \
  --allow-write "$OUT" \
  --fuel 20000000000 \
  -- "$@"
