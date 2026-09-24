#!/usr/bin/env bash
# bench.sh — measure Rust core vs Python oracle on the fixture set.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
[ -x bin/operon ] || bash scripts/build.sh > /dev/null
python3 scripts/bench_compare.py "$@"
