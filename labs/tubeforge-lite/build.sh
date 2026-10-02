#!/usr/bin/env bash
# TubeForge Lite — build both native binaries from source.
#   dist/tubeforge-lite-linux   (runs the e2e suite locally)
#   dist/TubeForge-Lite.exe     (cross-compiled for Windows x64, no Wine needed)
# Each build embeds the aria2c FOR ITS TARGET via scripts/gen-aria2-bundle.ts.
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.deno/bin:$PATH"

echo "[lite] 1/5 aria2 bundle (linux)…"
deno run -A scripts/gen-aria2-bundle.ts --platform linux
echo "[lite] 2/5 typecheck…"
deno check src/main.ts
echo "[lite] 3/5 compile linux…"
mkdir -p dist
deno compile -A --output dist/tubeforge-lite-linux src/main.ts
echo "[lite] 4/5 aria2 bundle (windows)…"
deno run -A scripts/gen-aria2-bundle.ts --platform windows
echo "[lite] 5/5 cross-compile windows .exe…"
deno compile -A --target x86_64-pc-windows-msvc --output dist/TubeForge-Lite.exe src/main.ts
# restore the neutral placeholder so the working tree stays clean
deno run -A scripts/gen-aria2-bundle.ts --platform none >/dev/null
ls -lh dist/
