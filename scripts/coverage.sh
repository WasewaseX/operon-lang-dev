#!/usr/bin/env bash
# coverage.sh — W52 (ROADMAP-100): code coverage report.
# Real coverage or an honest "unavailable" — never a fake number.
# Usage: scripts/coverage.sh            # llvm-cov via cargo-llvm-cov
#        scripts/coverage.sh --html     # + html report in target/coverage/html
set -uo pipefail
cd "$(dirname "$0")/.."

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  if cargo llvm-cov --version >/dev/null 2>&1; then :; else
    echo "coverage: cargo-llvm-cov not installed."
    echo "  install:  cargo install cargo-llvm-cov   (one-time, ~2 min)"
    echo "  baseline: NOT MEASURED — do not report a number you did not measure."
    exit 2
  fi
fi

GEN=(--ignore-filename-regex '(target|tests|examples|std|bootstrap)/')
echo "== cargo llvm-cov (unit + lib tests) =="
cargo llvm-cov --summary-only "${GEN[@]}" || exit 1
if [ "${1:-}" = "--html" ]; then
  cargo llvm-cov --html "${GEN[@]}" --output-dir target/coverage
  echo "html report: target/coverage/html/index.html"
fi
echo "note: proof-corpus coverage mapping rides \`operon test --list\` (W49);"
echo "      link uncovered hot spots into BENCH.md profiling section."
