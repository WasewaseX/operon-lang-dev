#!/usr/bin/env bash
# install.sh — Operon installer (T3).
# Usage:  curl -fsSL https://raw.githubusercontent.com/WasewaseX/operon-lang-dev/main/scripts/install.sh | sh
# Env:    OPERON_VERSION (default: latest release)  OPERON_INSTALL_DIR (default: ~/.local/bin)
# Flags:  --verify (B1-U3): require a SHA256SUMS manifest for the release and
#         fail closed when it is missing or does not list the asset. Without
#         the flag, behavior is unchanged: the per-asset .sha256 check stays
#         mandatory and fail-closed, the manifest is checked when present.
set -eu

# flag parsing (B1-U3): --verify only; every pre-existing env-driven
# behavior is untouched
INSTALL_MODE="install"
for arg in "$@"; do
  case "$arg" in
    --verify) INSTALL_MODE="verify" ;;
    -h|--help)
      echo "usage: install.sh [--verify]"
      echo "  --verify  require the release SHA256SUMS manifest and fail closed"
      echo "            if it is missing or does not list the asset"
      exit 0 ;;
    *) echo "error: unknown argument '$arg' (supported: --verify)" >&2; exit 1 ;;
  esac
done

REPO="WasewaseX/operon-lang-dev"
VER="${OPERON_VERSION:-latest}"
DEST="${OPERON_INSTALL_DIR:-$HOME/.local/bin}"

# S5 (release hardening): hermetic mode — OPERON_INSTALL_ASSET_DIR points at
# a directory laid out like a release (the .tar.gz assets, their .sha256
# sidecars, an optional SHA256SUMS manifest). When set, NOTHING touches the
# network: the asset, sidecar and manifest are read from the directory and
# every verification law below is unchanged (the per-asset sidecar stays
# mandatory and fail-closed; SHA256SUMS is cross-checked when present;
# --verify still fails closed without it). This is the test surface that
# scripts/install_e2e.sh exercises on every gate, and an air-gapped install
# path for machines that pre-stage release artifacts.
SRC_DIR="${OPERON_INSTALL_ASSET_DIR:-}"

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

if [ -n "$SRC_DIR" ]; then
  [ -d "$SRC_DIR" ] || { echo "error: OPERON_INSTALL_ASSET_DIR is not a directory: $SRC_DIR" >&2; exit 1; }
  if [ "$VER" = "latest" ]; then
    echo "error: OPERON_INSTALL_ASSET_DIR needs an explicit OPERON_VERSION (offline mode cannot resolve 'latest')" >&2
    exit 1
  fi
elif [ "$VER" = "latest" ]; then
  # follow the releases/latest redirect; no API token, no rate-limit JSON needed
  VER=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed 's#.*/tag/##')
  [ -n "$VER" ] || { echo "error: could not determine latest release" >&2; exit 1; }
fi

ASSET="operon-${VER#v}-${TARGET}.tar.gz"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ -n "$SRC_DIR" ]; then
  [ -f "$SRC_DIR/$ASSET" ] || { echo "error: $ASSET not found in $SRC_DIR" >&2; exit 1; }
  cp "$SRC_DIR/$ASSET" "$TMP/$ASSET"
else
  URL="https://github.com/$REPO/releases/download/${VER}/${ASSET}"
  echo "==> downloading $ASSET"
  curl -fsSL "$URL" -o "$TMP/$ASSET" || { echo "error: download failed ($URL)" >&2; exit 1; }
fi

# sec-r1 (audit SC-1): verification must FAIL CLOSED — the old version
# silently skipped verification on macOS (no sha256sum) or when the .sha256
# asset 404'd, which is exactly the downgrade/substitution hole curl|sh
# must never have.
if [ "$(uname -s)" = "Darwin" ]; then SHASUM="shasum -a 256"; else SHASUM="sha256sum"; fi
if ! command -v ${SHASUM%% *} >/dev/null 2>&1; then
  echo "error: no checksum tool found; refusing to install unverified" >&2
  exit 1
fi
if [ -n "$SRC_DIR" ]; then
  [ -f "$SRC_DIR/$ASSET.sha256" ] || { echo "error: checksum file unavailable ($ASSET.sha256); refusing to install unverified" >&2; exit 1; }
  cp "$SRC_DIR/$ASSET.sha256" "$TMP/$ASSET.sha256"
else
  if ! curl -fsSL "$URL.sha256" -o "$TMP/$ASSET.sha256"; then
    echo "error: checksum file unavailable ($URL.sha256); refusing to install unverified" >&2
    exit 1
  fi
fi
(cd "$TMP" && $SHASUM -c "$ASSET.sha256" >/dev/null) && echo "==> checksum ok" \
  || { echo "error: checksum mismatch; refusing to install" >&2; exit 1; }

# B1-U3 (supply chain): SHA256SUMS manifest cross-check. The release
# workflow's sha256sums job publishes a SHA256SUMS file next to the release
# assets (every binary asset, hashed from the exact published bytes after
# its sidecar re-verifies); scripts/release.sh emits the source-archive
# manifest. When the manifest exists, the downloaded asset is also
# verified against it before anything is installed. --verify makes the
# manifest mandatory and fail-closed.
SUMS_URL="https://github.com/$REPO/releases/download/${VER}/SHA256SUMS"
SUMS_GOT=0
if [ -n "$SRC_DIR" ]; then
  # offline mode: the manifest, when published, sits next to the assets
  if [ -f "$SRC_DIR/SHA256SUMS" ]; then
    cp "$SRC_DIR/SHA256SUMS" "$TMP/SHA256SUMS"
    SUMS_GOT=1
  fi
elif curl -fsSL "$SUMS_URL" -o "$TMP/SHA256SUMS" 2>/dev/null; then
  SUMS_GOT=1
fi
if [ "$SUMS_GOT" = "1" ]; then
  WANT=$(tr -s ' \t' ' ' < "$TMP/SHA256SUMS" | while IFS=' ' read -r h n; do
    [ "$n" = "$ASSET" ] && printf '%s\n' "$h"
  done | head -n1)
  WANT=$(printf '%s' "$WANT" | tr 'A-Z' 'a-z')
  GOT=$($SHASUM "$TMP/$ASSET" | cut -d' ' -f1)
  if [ -z "$WANT" ]; then
    if [ "$INSTALL_MODE" = "verify" ]; then
      echo "error: $ASSET is not listed in the SHA256SUMS manifest; refusing to install (fail closed, --verify)" >&2
      exit 1
    fi
    echo "==> note: $ASSET not listed in SHA256SUMS; per-asset checksum already verified"
  elif [ "$WANT" != "$GOT" ]; then
    echo "error: SHA256SUMS manifest mismatch for $ASSET; refusing to install" >&2
    exit 1
  else
    echo "==> SHA256SUMS manifest ok"
  fi
else
  if [ "$INSTALL_MODE" = "verify" ]; then
    echo "error: no SHA256SUMS manifest published for release $VER; refusing to install (fail closed, --verify)" >&2
    exit 1
  fi
  echo "==> note: no SHA256SUMS manifest for this release; per-asset checksum still enforced"
fi

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
