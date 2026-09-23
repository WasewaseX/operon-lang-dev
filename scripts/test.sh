#!/usr/bin/env bash
# test.sh — full verification: C kernel smoke, proof suite, differential harness.
set -euo pipefail
cd "$(dirname "$0")/.."
echo "[1/3] C/C++ kernel smoke test"
gcc -O2 -std=c17 tests/smoke_runtime.c build/operon_rt.o build/codon_kernel.o -lstdc++ -lm -o /tmp/operon_smoke
/tmp/operon_smoke
echo "[2/3] Operon proof suite (Rust core)"
./bin/operon test tests/
echo "[3/3] Differential harness (Rust core vs Python oracle)"
python3 bootstrap/harness.py
echo "ALL GREEN"
