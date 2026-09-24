#!/usr/bin/env bash
# test.sh — full verification: native kernel smoke (ASan-capable), proof
# suite, differential harness.
set -euo pipefail
cd "$(dirname "$0")/.."
echo "[1/3] C++ codon kernel smoke test"
g++ -O2 -std=c++17 tests/smoke_codon.cpp build/codon_kernel.o -o /tmp/operon_smoke
/tmp/operon_smoke
echo "[2/3] Operon proof suite (Rust core)"
./bin/operon test tests/
echo "[3/3] Differential harness (Rust core vs Python oracle)"
python3 bootstrap/harness.py
echo "ALL GREEN"
