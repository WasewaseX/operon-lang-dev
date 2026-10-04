#!/usr/bin/env bash
# loglens — access-log analytics: Operon runtime (~2.5 MB), pure stdlib.
# This launcher is the security boundary: it grants the Operon sandbox
# read access to the resolved log path's directory and the app directory
# — nothing else. Run it from anywhere; it relocates to the repo root
# first because std/ module resolution is CWD-relative (known quirk,
# see BENCH.md cross-lane note — app-level workaround, not a runtime fix).
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OP="${OPERON_BIN:-$ROOT/target/release/operon}"
if [ ! -x "$OP" ]; then
  OP="$(command -v operon || true)"
fi
if [ -z "${OP:-}" ]; then
  echo "loglens: operon runtime not found (set OPERON_BIN or put operon on PATH)" >&2
  exit 2
fi

cd "$ROOT"
exec "$OP" run "$HERE/loglens.op" \
  --cell "$HERE/loglens.cell" \
  --allow-read "$ROOT" \
  --fuel 20000000000 \
  -- "$@"
