#!/usr/bin/env bash
# Fresh-clone helper: fetch aria2 static binaries into vendor/ for bundling.
# (The dev sandbox already carries verified copies — build.sh falls back to
# the canonical locations if vendor/ is empty.)
set -uo pipefail
cd "$(dirname "$0")/.."
VER="1.37.0"
mkdir -p vendor

if [ ! -f vendor/aria2c-linux ]; then
  echo "[fetch-aria2] linux static build…"
  for URL in \
    "https://github.com/abcfy2/aria2-static-build/releases/download/${VER}/aria2-${VER}-linux-gnu-64bit-build1.tar.xz" \
    "https://github.com/abcfy2/aria2-static-build/releases/download/${VER}/aria2-${VER}-linux-musl-64bit-build1.tar.xz"; do
    if curl -fL --max-time 180 -o /tmp/aria2-linux.tar.xz "$URL" 2>/dev/null; then
      tar -xJf /tmp/aria2-linux.tar.xz -C /tmp
      BIN=$(find /tmp -type f -name aria2c | head -1)
      if [ -n "$BIN" ]; then cp "$BIN" vendor/aria2c-linux; chmod +x vendor/aria2c-linux; break; fi
    fi
  done
  [ -f vendor/aria2c-linux ] || echo "[fetch-aria2] WARNING: linux aria2c missing — build with --platform windows/none only"
fi

if [ ! -f vendor/aria2c.exe ]; then
  echo "[fetch-aria2] official windows build…"
  curl -fL --max-time 180 -o /tmp/aria2-win.zip \
    "https://github.com/aria2/aria2/releases/download/release-${VER}/aria2-${VER}-win-64bit-build1.zip" \
    && unzip -o -j /tmp/aria2-win.zip "*aria2c.exe" -d vendor/ >/dev/null
  [ -f vendor/aria2c.exe ] || echo "[fetch-aria2] WARNING: windows aria2c.exe missing"
fi

ls -la vendor/ 2>/dev/null
