#!/usr/bin/env bash
# W073: docs/api regen-check — the committed markdown must equal a fresh
# `operon doc std -o` render. Usage: scripts/doc_api_check.sh [bin-path]
# Exit 1 with a diff on drift (CI-wirable; run scripts/gen then commit).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${1:-bin/operon}"
if [ ! -x "$BIN" ]; then echo "doc_api_check: $BIN not found (build first)" >&2; exit 2; fi
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
"$BIN" doc std -o "$TMP" > /dev/null
if diff -ru docs/api "$TMP" > /tmp/doc_api_diff.txt; then
  echo "doc_api_check: docs/api is up to date"
else
  echo "doc_api_check: DRIFT — docs/api does not match 'operon doc std':" >&2
  head -40 /tmp/doc_api_diff.txt >&2
  echo "... (full diff in /tmp/doc_api_diff.txt) — rerun: operon doc std -o docs/api" >&2
  exit 1
fi
