#!/usr/bin/env bash
# install.sh — Operon installer (T3).
# Usage:  curl -fsSL https://raw.githubusercontent.com/WasewaseX/operon-lang-dev/main/scripts/install.sh | sh
# Env:    OPERON_VERSION (default: latest release)  OPERON_INSTALL_DIR (default: ~/.local/bin)
set -eu

REPO="WasewaseX/operon-lang-dev"
VER="${OPERON_VERSION:-latest}"
DEST="${OPERON_INSTALL_DIR:-$HOME/.local/bin}"

os=$(uname -s); arch=$(uname -m)
case "$os" in
  Linux) os_part="unknown-linux-gnu" ;;
  Darwin) os_part="apple-darwin" ;;
  *) echo "error: unsupported OS '$os' (windows: download the .zip from the releases page)" >&2; exit 1 ;;
esac
case "$arch" in
  x86_64|amd64) arch_part="x86_64" ;;
  aarch64|arm64) arch_part="aarch64" ;;
  *) echo "error: unsupported architecture '$arch'" >&2; exit 1 ;;
esac
TARGET="${arch_part}-${os_part}"

if [ "$VER" = "latest" ]; then
  # follow the releases/latest redirect; no API token, no rate-limit JSON needed
  VER=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed 's#.*/tag/##')
  [ -n "$VER" ] || { echo "error: could not determine latest release" >&2; exit 1; }
fi

ASSET="operon-${VER#v}-${TARGET}.tar.gz"
URL="https://github.com/$REPO/releases/download/${VER}/${ASSET}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "==> downloading $ASSET"
curl -fsSL "$URL" -o "$TMP/$ASSET" || { echo "error: download failed ($URL)" >&2; exit 1; }

if command -v sha256sum >/dev/null 2>&1 && curl -fsSL "$URL.sha256" -o "$TMP/$ASSET.sha256" 2>/dev/null; then
  (cd "$TMP" && sha256sum -c "$ASSET.sha256" >/dev/null) && echo "==> checksum ok" \
    || { echo "error: checksum mismatch" >&2; exit 1; }
fi

mkdir -p "$DEST"
tar xzf "$TMP/$ASSET" -C "$TMP"
mv "$TMP/operon-${VER#v}-${TARGET}/operon" "$DEST/operon"
chmod +x "$DEST/operon"

case ":$PATH:" in
  *":$DEST:"*) ;;
  *) echo "==> note: $DEST is not on your PATH; add:  export PATH=\"$DEST:\$PATH\"" ;;
esac

"$DEST/operon" version
echo "==> installed. try: operon repl   (or read the tutorial: operon-lang/TUTORIAL.md)"
