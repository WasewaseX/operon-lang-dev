#!/usr/bin/env bash
# bench_opt.sh — W011 per-pass / per-level benchmarks (builder-A, dev-1).
#
# Answers "what did each optimization buy?" with wall-clock numbers on a
# fixed corpus, run at every optimization configuration. Complements
# bench.sh (which benchmarks the LANGUAGE workloads) — this one isolates
# the OPTIMIZER's effect:
#   tree                     — the tree-walk baseline
#   O0+fast0                 — pure W09 bytecode (no passes, no fast paths)
#   O0                       — engine fast paths only (trivial-body return,
#                              lazy traceback frames, builtin dispatch tables)
#   O1+fast0                 — compile passes only (fold, thread, dce)
#   O1 (default)             — passes + fast paths
#   O2                       — + block-local constant propagation
#
# The corpus lives in scripts/bench/opt/ — each fixture names the pass it
# exercises in its header. Timing: median of N subprocess runs (python3),
# warm page cache, same binary. Deterministic programs (a checksum prints).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
[ -x bin/operon ] || bash scripts/build.sh > /dev/null
python3 scripts/bench_opt.py "$@"
