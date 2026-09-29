#!/usr/bin/env bash
# M100 W076 — CI-verified embedding example gate.
#
# Builds examples/embed (an external-style crate consuming `operon` as a path
# dependency, its own workspace + lockfile) and runs it, asserting the
# tool-level run produces the expected captured promote() output.
#
# This keeps docs/EMBEDDING.md honest: if the public embedding surface
# drifts, this crate stops compiling or the assertion fails and CI goes red.
#
# Linux-only by design (the parent crate's Linux CI job already has the C++
# toolchain the operon build.rs needs); Windows/macOS embedders follow the
# same steps locally.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== W076: building examples/embed (path dependency on operon) =="
cargo build --manifest-path examples/embed/Cargo.toml --quiet

echo "== W076: running the embedded program =="
out=$(cargo run --manifest-path examples/embed/Cargo.toml --quiet 2>/dev/null)

echo "$out"
case "$out" in
  *"embedded ok: tool-level run through the operon library"*)
    echo "== W076 EMBED OK =="
    ;;
  *)
    echo "FAIL: expected the captured promote() line from examples/embed/demo.op" >&2
    exit 1
    ;;
esac
