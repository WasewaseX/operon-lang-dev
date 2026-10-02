#!/usr/bin/env bash
# release_smoke.sh — W60 (ROADMAP-100): per-target release smoke validation.
# Runs the shipped artifact itself: version, a real program, a check pass,
# and the std/ presence contract (dx-r3 lesson: clean installs silently
# broken when std/ didn't ship).
# Usage: scripts/release_smoke.sh <dir-or-archive> [expected-version]
# Callers: .github/workflows/release.yml (per-asset smoke, W060 done-when:
# failed smoke = failed release) and scripts/test.sh (dir-mode anti-rot).
# Archives are smoked in pkg mode: extracted, run from the extracted layout
# (binary + std/ + packaged examples), so the repo checkout cannot mask a
# broken package. Dir mode stays lenient for dev trees (repo std/ counts).
set -euo pipefail
cd "$(dirname "$0")/.."

ART="${1:?usage: release_smoke.sh <dir-or-archive> [expected-version]}"
EXPECT="${2:-}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

MODE=dir
if [ -d "$ART" ]; then
  DIR="$ART"
elif [ -f "$ART" ]; then
  MODE=pkg
  case "$ART" in
    *.tar.gz|*.tgz) tar -xzf "$ART" -C "$TMP" ;;
    *.zip)
      # windows leg ships .zip; extractor availability differs by runner —
      # unzip, then bsdtar (reads zip), then PowerShell Expand-Archive.
      if command -v unzip >/dev/null 2>&1; then
        unzip -q "$ART" -d "$TMP"
      elif tar -tf "$ART" >/dev/null 2>&1; then
        tar -xf "$ART" -C "$TMP"
      elif command -v powershell.exe >/dev/null 2>&1 && command -v cygpath >/dev/null 2>&1; then
        W="$(cygpath -w "$(cd "$(dirname "$ART")" && pwd)")/$(basename "$ART")"
        powershell.exe -NoProfile -Command "Expand-Archive -LiteralPath '$W' -DestinationPath '$(cygpath -w "$TMP")'"
      else
        echo "smoke: no zip extractor (need unzip, bsdtar or PowerShell)"; exit 2
      fi ;;
    *) echo "smoke: unsupported archive format: $ART"; exit 2 ;;
  esac
  # exact-name match: the operon* glob also caught operon-ls and could smoke
  # the language server instead of the compiler
  FIRST="$(find "$TMP" -type f \( -name operon -o -name operon.exe \) | head -1)"
  [ -n "$FIRST" ] || { echo "smoke FAIL: archive has no operon binary"; exit 1; }
  DIR="$(dirname "$FIRST")"
else
  echo "smoke: no such artifact: $ART"; exit 2
fi
case "$DIR" in /*) ;; *) DIR="$PWD/$DIR" ;; esac

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
# S5: the language server ships in every asset since lsp-r1 — smoke it like
# the compiler. pkg mode: it MUST be there (asset contract). dir mode: dev
# trees may not have built it, presence check only.
LSBIN="$(find "$DIR" -maxdepth 2 -type f \( -name operon-ls -o -name operon-ls.exe \) | head -1)"
if [ -n "$LSBIN" ]; then
  chmod +x "$LSBIN" 2>/dev/null || true
  step "operon-ls --version" "$LSBIN" --version
elif [ "$MODE" = pkg ]; then
  echo "FAIL  operon-ls missing inside the artifact (lsp-r1: the editor story ships with the binary)"
  FAIL=1
else
  echo "ok    operon-ls absent (dev tree without operon-ls build, tolerated in dir mode)"
fi
# pkg mode simulates the user's clean install: run from the extracted
# artifact itself, against the examples/std that were actually shipped —
# not the repo checkout (a repo checkout can mask a broken package).
if [ "$MODE" = pkg ]; then cd "$DIR"; fi
step "run hello"  "$BIN" run examples/cookbook/bank_account.op
step "check pass" "$BIN" check examples/cookbook/caesar.op
if [ "$MODE" = pkg ]; then
  # dx-r3: the SHIPPED tree must carry std/ next to the binary. The repo
  # checkout also has std/ and must NOT be allowed to satisfy this check —
  # that masking is exactly the silent breakage dx-r3 shipped once.
  if [ -d "$DIR/std" ]; then
    echo "ok    std/ present (in artifact)"
  else
    echo "FAIL  std/ missing inside the artifact (dx-r3: clean installs break without it)"
    FAIL=1
  fi
  # S5: the packaging contract also ships README / LICENSE / TUTORIAL —
  # they are what the release notes and the offline tutorial promise.
  for f in README.md LICENSE TUTORIAL.md; do
    if [ -f "$DIR/$f" ]; then
      echo "ok    $f present (in artifact)"
    else
      echo "FAIL  $f missing inside the artifact (packaging contract)"
      FAIL=1
    fi
  done
else
  if [ -d "$(dirname "$BIN")/../std" ] || [ -d "$DIR/std" ] || [ -d "std" ]; then
    echo "ok    std/ present"
  else
    echo "FAIL  std/ missing next to binary (dx-r3: clean installs break without it)"
    FAIL=1
  fi
fi

[ "$FAIL" = 0 ] && echo "smoke: PASS ($ART)" || { echo "smoke: FAIL ($ART)"; exit 1; }
