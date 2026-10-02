#!/usr/bin/env bash
# install_e2e.sh — S5 (release hardening): hermetic installer e2e.
# Exercises scripts/install.sh against a SYNTHETIC release directory
# (OPERON_INSTALL_ASSET_DIR): no network, no real release needed. This is
# the surface install.sh never had — until now the installer's fail-closed
# law (sec-r1 / B1-U3) was only ever exercised by production traffic.
#
# Covered:
#   1. happy path: sidecar verified, SHA256SUMS cross-checked, binary +
#      operon-ls + std/ land in DEST, `operon version` runs
#   2. tampered sidecar -> refusal (exit 1), nothing installed
#   3. --verify with NO manifest -> refusal (fail closed, B1-U3)
#   4. --verify with asset NOT LISTED in the manifest -> refusal
#   5. offline mode refuses OPERON_VERSION=latest (would need the network)
#
# Callers: scripts/test.sh (standing stanza, after the build).
set -euo pipefail
cd "$(dirname "$0")/.."

[ -x bin/operon ] || { echo "install_e2e: bin/operon missing (run scripts/build.sh first)"; exit 2; }

# mirror install.sh's uname->target mapping (asset name must match what the
# installer computes for this host; a mapping drift fails loudly here)
os=$(uname -s); arch=$(uname -m)
case "$os" in
  Linux) os_part="unknown-linux-gnu" ;;
  Darwin) os_part="apple-darwin" ;;
  *) echo "install_e2e: unsupported host OS '$os' (windows uses the .zip path)"; exit 0 ;;
esac
case "$arch" in
  x86_64|amd64) arch_part="x86_64" ;;
  aarch64|arm64) arch_part="aarch64" ;;
  *) echo "install_e2e: unsupported host arch '$arch'"; exit 0 ;;
esac
TARGET="${arch_part}-${os_part}"

VER="0.0.0-e2e"
ASSET="operon-${VER}-${TARGET}.tar.gz"
WORK="$(mktemp -d)"
DEST="$(mktemp -d)"
trap 'rm -rf "$WORK" "$DEST"' EXIT

FAIL=0
check() { # check <name> <expected-output-substring> <cmd...>
  local name="$1" want="$2"; shift 2
  local out
  if out="$("$@" 2>&1)"; then
    if [ -n "$want" ] && [ "${out#*"$want"}" = "$out" ]; then
      echo "FAIL  $name (output missing '$want')"; FAIL=1
    else
      echo "ok    $name"
    fi
  else
    echo "FAIL  $name (exit $?)"; FAIL=1
  fi
}
refuse() { # refuse <name> <expected-error-substring> <cmd...>
  local name="$1" want="$2"; shift 2
  local out rc=0
  out="$("$@" 2>&1)" || rc=$?
  if [ "$rc" -ne 1 ]; then
    echo "FAIL  $name (expected refusal exit 1, got $rc)"; FAIL=1
  elif [ "${out#*"$want"}" = "$out" ]; then
    echo "FAIL  $name (error output missing '$want')"; FAIL=1
  else
    echo "ok    $name"
  fi
}

# --- synthetic release directory, laid out exactly like a real release ---
ST="operon-${VER}-${TARGET}"
mkdir -p "$WORK/$ST"
cp bin/operon "$WORK/$ST/"
if [ -f bin/operon-ls ]; then cp bin/operon-ls "$WORK/$ST/"; fi
if [ -d std ]; then cp -r std "$WORK/$ST/std"; fi
if [ -f README.md ]; then cp README.md "$WORK/$ST/"; fi
if [ -f LICENSE ]; then cp LICENSE "$WORK/$ST/"; fi
if [ -f TUTORIAL.md ]; then cp TUTORIAL.md "$WORK/$ST/"; fi
tar czf "$WORK/$ASSET" -C "$WORK" "$ST"
(cd "$WORK" && sha256sum "$ASSET" > "$ASSET.sha256")
(cd "$WORK" && sha256sum "$ASSET" > SHA256SUMS)

export OPERON_INSTALL_ASSET_DIR="$WORK"
export OPERON_VERSION="v$VER"
export OPERON_INSTALL_DIR="$DEST"

# 1. happy path (manifest present, so the SHA256SUMS cross-check runs too)
check "install (offline mode, manifest ok)" "SHA256SUMS manifest ok" sh scripts/install.sh
[ -x "$DEST/operon" ] || { echo "FAIL  binary not installed to DEST"; FAIL=1; }
[ -x "$DEST/operon-ls" ] || { echo "FAIL  operon-ls not installed to DEST"; FAIL=1; }
[ -d "$DEST/std" ] || { echo "FAIL  std/ not installed next to the binary (dx-r3)"; FAIL=1; }
# the installed binary reports the COMPILED version, not the asset's
# synthetic name — compare against the local build's own string
EXPECT_V="$(bin/operon --version)"
check "installed binary runs" "$EXPECT_V" "$DEST/operon" --version

# 2. tampered sidecar -> refusal; and the refusal must leave NOTHING behind
rm -rf "$DEST"; mkdir -p "$DEST"
GOOD=$(cat "$WORK/$ASSET.sha256")
echo "0000000000000000000000000000000000000000000000000000000000000000  $ASSET" > "$WORK/$ASSET.sha256"
refuse "tampered sidecar refuses" "checksum mismatch" sh scripts/install.sh
[ -e "$DEST/operon" ] && { echo "FAIL  tampered sidecar still installed the binary"; FAIL=1; }
echo "$GOOD" > "$WORK/$ASSET.sha256"

# 3. --verify with NO manifest -> fail closed
rm -rf "$DEST"; mkdir -p "$DEST"
mv "$WORK/SHA256SUMS" "$WORK/SHA256SUMS.bak"
refuse "--verify without manifest refuses" "no SHA256SUMS manifest" sh scripts/install.sh --verify
# non-verify mode stays usable (per-asset sidecar is the mandatory floor)
check "no manifest, non-verify still installs" "no SHA256SUMS manifest for this release" sh scripts/install.sh

# 4. --verify with the asset NOT LISTED -> fail closed
rm -rf "$DEST"; mkdir -p "$DEST"
echo "$GOOD" > "$WORK/SHA256SUMS"
printf '%s  operon-%s-some-other-target.tar.gz\n' "$(cut -d' ' -f1 "$WORK/$ASSET.sha256")" "$VER" >> "$WORK/SHA256SUMS"
sed -i "/$ASSET/d" "$WORK/SHA256SUMS"
refuse "--verify unlisted asset refuses" "not listed in the SHA256SUMS manifest" sh scripts/install.sh --verify

# 5. offline mode refuses 'latest' (that resolution needs the network)
rm -rf "$DEST"; mkdir -p "$DEST"
refuse "offline mode refuses OPERON_VERSION=latest" "explicit OPERON_VERSION" env OPERON_VERSION= sh scripts/install.sh

# restore manifest; uninstall leftovers already cleaned by the trap
if [ "$FAIL" = 0 ]; then
  echo "install_e2e: ALL GREEN (5 contracts)"
else
  echo "install_e2e: FAIL"; exit 1
fi
