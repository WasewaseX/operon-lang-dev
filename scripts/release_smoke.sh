#!/usr/bin/env bash
# release_smoke.sh — W60 (ROADMAP-100): per-target release smoke validation.
# Runs the shipped artifact itself: version, a real program, a check pass,
# and the std/ presence contract (dx-r3 lesson: clean installs silently
# broken when std/ didn't ship).
# Usage: scripts/release_smoke.sh <dir-or-archive> [expected-version]
set -euo pipefail
cd "$(dirname "$0")/.."

ART="${1:?usage: release_smoke.sh <dir-or-archive> [expected-version]}"
EXPECT="${2:-}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [ -d "$ART" ]; then
  DIR="$ART"
elif [ -f "$ART" ]; then
  case "$ART" in
    *.tar.gz|*.tgz) tar -xzf "$ART" -C "$TMP" ;;
    *.zip) unzip -q "$ART" -d "$TMP" ;;
    *) echo "smoke: unsupported archive format: $ART"; exit 2 ;;
  esac
  DIR="$(dirname "$(find "$TMP" -name operon* -type f | head -1)")"
else
  echo "smoke: no such artifact: $ART"; exit 2
fi

BIN="$(find "$DIR" -maxdepth 2 -type f \( -name operon -o -name operon.exe \) | head -1)"
[ -n "$BIN" ] || { echo "smoke FAIL: no operon binary in $ART"; exit 1; }
chmod +x "$BIN" 2>/dev/null || true

FAIL=0
step() { # step <name> <cmd...>
  local name="$1"; shift
  if "$@" >/dev/null 2>"$TMP/err"; then
    echo "ok    $name"
  else
    echo "FAIL  $name"; sed 's/^/      /' "$TMP/err" | head -3
    FAIL=1
  fi
}

V="$("$BIN" --version 2>/dev/null || true)"
echo "smoke: $ART -> $BIN ($V)"
if [ -n "$EXPECT" ] && [ "$V" != "${V//"$EXPECT"/}" ]; then :; else
  [ -n "$EXPECT" ] && { echo "FAIL  version ($V != expected $EXPECT)"; FAIL=1; }
fi
step "version"    "$BIN" --version
step "run hello"  "$BIN" run examples/cookbook/bank_account.op
step "check pass" "$BIN" check examples/cookbook/caesar.op
if [ -d "$(dirname "$BIN")/../std" ] || [ -d "$DIR/std" ] || [ -d "std" ]; then
  echo "ok    std/ present"
else
  echo "FAIL  std/ missing next to binary (dx-r3: clean installs break without it)"
  FAIL=1
fi

[ "$FAIL" = 0 ] && echo "smoke: PASS ($ART)" || { echo "smoke: FAIL ($ART)"; exit 1; }
