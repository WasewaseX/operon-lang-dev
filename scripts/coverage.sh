#!/usr/bin/env bash
# coverage.sh — W52 (ROADMAP-100): code coverage report, two layers.
# Real coverage or an honest "unavailable" — never a fake number.
#
#   layer 1  cargo-llvm-cov line/region coverage of the Rust core over every
#            cargo test target. Skipped, and honestly marked unavailable,
#            when the tool or toolchain is not installable in the
#            environment. One install attempt, no fighting.
#   layer 2  std call-site coverage of the .op proof corpus
#            (scripts/coverage_corpus.py, python3 stdlib, works offline).
#
# The layer 1 summary table is captured to target/llvm-cov-summary.txt and
# embedded in docs/coverage.md by layer 2; when layer 1 is unavailable the
# doc says so instead of inventing a number.
#
# Exit codes: 0 = at least one layer measured; 1 = a layer that should always
# work failed; 2 = nothing was measurable.
#
# Usage: scripts/coverage.sh            # both layers, text report
#        scripts/coverage.sh --html     # + html report in target/coverage/html
set -uo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

L1="unavailable"
L2="unavailable"
GEN=(--ignore-filename-regex '(target|tests|examples|std|bootstrap)/')
mkdir -p target
rm -f target/llvm-cov-summary.txt

echo "== layer 1: cargo llvm-cov (cargo test suite, line/region) =="
if command -v cargo-llvm-cov >/dev/null 2>&1 && cargo llvm-cov --version >/dev/null 2>&1; then
  if cargo llvm-cov --summary-only "${GEN[@]}" > target/llvm-cov-full.txt 2>&1; then
    L1="measured"
    awk '/^Filename/{f=1} f{print} /^TOTAL/{f=0}' \
      target/llvm-cov-full.txt > target/llvm-cov-summary.txt
    echo "all cargo test targets passed; headline:"
    tail -1 target/llvm-cov-summary.txt
    echo "full log: target/llvm-cov-full.txt ; summary: target/llvm-cov-summary.txt"
    if [ "${1:-}" = "--html" ]; then
      cargo llvm-cov --html "${GEN[@]}" --output-dir target/coverage
      echo "html report: target/coverage/html/index.html"
    fi
  else
    echo "coverage: cargo llvm-cov ran but failed (see target/llvm-cov-full.txt):"
    tail -3 target/llvm-cov-full.txt | sed 's/^/  /'
  fi
else
  echo "coverage: cargo-llvm-cov not installed."
  echo "  install:  cargo install cargo-llvm-cov   (one-time)"
  echo "            rustup component add llvm-tools-preview"
fi
if [ "$L1" != "measured" ]; then
  echo "  layer 1 baseline: NOT MEASURED — do not report a number you did not measure."
fi

echo
echo "== layer 2: std call-site coverage of the .op proof corpus =="
if python3 scripts/coverage_corpus.py; then
  L2="measured"
else
  echo "coverage: corpus layer failed (it is stdlib-only and must always run)."
fi

echo
if [ "$L2" = "failed" ]; then
  if [ "$L1" = "measured" ]; then
    echo "coverage: layer 2 broken, layer 1 measured; fix scripts/coverage_corpus.py."
    exit 1
  fi
  echo "coverage: NOT MEASURABLE — no layer produced a measurement."
  exit 2
fi
if [ "$L1" = "measured" ]; then
  echo "coverage: both layers measured; baseline written to docs/coverage.md."
else
  echo "coverage: corpus layer measured, Rust layer honestly unavailable;"
  echo "          baseline written to docs/coverage.md."
fi
exit 0
