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

# sec-r1 (audit SC-1): verification must FAIL CLOSED — the old version
# silently skipped verification on macOS (no sha256sum) or when the .sha256
# asset 404'd, which is exactly the downgrade/substitution hole curl|sh
# must never have.
if [ "$(uname -s)" = "Darwin" ]; then SHASUM="shasum -a 256"; else SHASUM="sha256sum"; fi
if ! command -v ${SHASUM%% *} >/dev/null 2>&1; then
  echo "error: no checksum tool found; refusing to install unverified" >&2
  exit 1
fi
if ! curl -fsSL "$URL.sha256" -o "$TMP/$ASSET.sha256"; then
  echo "error: checksum file unavailable ($URL.sha256); refusing to install unverified" >&2
  exit 1
fi
(cd "$TMP" && $SHASUM -c "$ASSET.sha256" >/dev/null) && echo "==> checksum ok" \
  || { echo "error: checksum mismatch; refusing to install" >&2; exit 1; }

mkdir -p "$DEST"
tar xzf "$TMP/$ASSET" -C "$TMP"
mv "$TMP/operon-${VER#v}-${TARGET}/operon" "$DEST/operon"
chmod +x "$DEST/operon"
# dx-r3 (re-audit): the release tarball ships the self-hosted stdlib and
# the LSP server — the old installer discarded BOTH, so `use std/strings`
# silently resolved to null on every clean install. std/ must sit next to
# the binary (exe-relative resolution, genes.rs module loader).
if [ -d "$TMP/operon-${VER#v}-${TARGET}/std" ]; then
  rm -rf "$DEST/std"
  cp -r "$TMP/operon-${VER#v}-${TARGET}/std" "$DEST/std"
fi
if [ -f "$TMP/operon-${VER#v}-${TARGET}/operon-ls" ]; then
  mv "$TMP/operon-${VER#v}-${TARGET}/operon-ls" "$DEST/operon-ls"
  chmod +x "$DEST/operon-ls"
fi

case ":$PATH:" in
  *":$DEST:"*) ;;
  *) echo "==> note: $DEST is not on your PATH; add:  export PATH=\"$DEST:\$PATH\"" ;;
esac

"$DEST/operon" version
echo "==> installed. try: operon repl   (or read the tutorial: https://github.com/WasewaseX/operon-lang-dev/blob/main/TUTORIAL.md)"
