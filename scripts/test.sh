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
# dx-r6 (loop-5-a audit): apps/ joins the CI gate — the flagship app's
# proof frame was failing silently (stale ORF assert) because nothing ran it
./bin/operon test apps/
# substrate-r1: the Python bridge granted suite — explicit operator cell
# (grants cannot come from auto-detected cells, so this is a separate step)
./bin/operon test tests/granted/ --cell tests/granted/pybridge.cell
# loop-9 (P0-4): the standalone m6A decay cadence — its own explicit cell
# (m6a.decay 0.5); a silent no-op before this round, now pinned here.
./bin/operon test tests/granted/m6a_decay_cadence.op --cell tests/granted/m6a_decay_cadence.cell
echo "[3/4] Differential harness (Rust core vs Python oracle)"
python3 bootstrap/harness.py
echo "[4/4] Oracle proof suite (the same frames on the second implementation)"
python3 bootstrap/oracle.py test tests/
echo "ALL GREEN"
