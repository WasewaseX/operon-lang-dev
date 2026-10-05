#!/usr/bin/env bash
# TubeForge Lite (Go) — build both native binaries from source.
#   dist/tubeforge-lite-linux   (runs the e2e suite locally)
#   dist/TubeForge-Lite.exe     (cross-compiled for Windows x64, no Wine needed)
# v1.1.0: same app, Go runtime — 82.5 MB (deno compile) drops to ~10-12 MB.
set -euo pipefail
cd "$(dirname "$0")"
GO="${GO:-go}"
if ! command -v "$GO" >/dev/null 2>&1; then
  echo "[lite-go] 'go' not on PATH — set GO=/path/to/go (needs go >= 1.22)" >&2
  exit 1
fi

echo "[lite-go] 1/4 aria2 bundles (both targets)…"
bash scripts/gen-bundle.sh all
echo "[lite-go] 2/4 vet…"
"$GO" vet ./...
echo "[lite-go] 3/4 build linux…"
mkdir -p dist
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 "$GO" build -trimpath -ldflags "-s -w" -o dist/tubeforge-lite-linux .
echo "[lite-go] 4/4 build windows…"
CGO_ENABLED=0 GOOS=windows GOARCH=amd64 "$GO" build -trimpath -ldflags "-s -w" -o dist/TubeForge-Lite.exe .
ls -lh dist/
