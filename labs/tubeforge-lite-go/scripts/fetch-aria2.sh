#!/usr/bin/env bash
# TubeForge Lite (Go) — fetch aria2 static binaries into vendor/ for bundling.
# Sources are the same pinned builds the v1.0.0 Deno bundle used.
set -uo pipefail
cd "$(dirname "$0")/.."
VER="1.37.0"
mkdir -p vendor

if [ ! -f vendor/aria2c-linux ]; then
  echo "[fetch-aria2] linux static build…"
  for URL in \
    "https://github.com/abcfy2/aria2-static-build/releases/download/${VER}/aria2-x86_64-linux-musl_static.zip" \
    "https://github.com/abcfy2/aria2-static-build/releases/download/${VER}/aria2-x86_64-linux-gnu_static.zip"; do
    if curl -fL --max-time 180 -o /tmp/aria2-linux.zip "$URL" 2>/dev/null; then
      rm -rf /tmp/aria2lx && mkdir -p /tmp/aria2lx
      unzip -o -j /tmp/aria2-linux.zip "*aria2c" -d /tmp/aria2lx >/dev/null
      if [ -f /tmp/aria2lx/aria2c ]; then cp /tmp/aria2lx/aria2c vendor/aria2c-linux; chmod +x vendor/aria2c-linux; break; fi
    fi
  done
  [ -f vendor/aria2c-linux ] || echo "[fetch-aria2] WARNING: linux aria2c missing — bundle stays placeholder for linux"
fi

if [ ! -f vendor/aria2c.exe ]; then
  echo "[fetch-aria2] official windows build…"
  curl -fL --max-time 180 -o /tmp/aria2-win.zip \
    "https://github.com/aria2/aria2/releases/download/release-${VER}/aria2-${VER}-win-64bit-build1.zip" \
    && unzip -o -j /tmp/aria2-win.zip "*aria2c.exe" -d vendor/ >/dev/null
  [ -f vendor/aria2c.exe ] || echo "[fetch-aria2] WARNING: windows aria2c.exe missing — bundle stays placeholder for windows"
fi

ls -la vendor/ 2>/dev/null
