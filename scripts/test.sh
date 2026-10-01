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
./bin/operon test tests/granted/pybridge.op --cell tests/granted/pybridge.cell
# loop-9 (P0-4): the standalone m6A decay cadence — its own explicit cell
# (m6a.decay 0.5); a silent no-op before this round, now pinned here.
./bin/operon test tests/granted/m6a_decay_cadence.op --cell tests/granted/m6a_decay_cadence.cell
# loop-10 (F-7/F-8): Rho termination + ribosome-queue shield — explicit
# operator cells (granted-lane pattern); the ENTROPY-STREAM parity of the
# opt-in draws is differentially pinned in harness.py (granted targets).
./bin/operon test tests/granted/rho_termination.op --cell tests/granted/rho_termination.cell
./bin/operon test tests/granted/rho_readthrough.op --cell tests/granted/rho_readthrough.cell
./bin/operon test tests/granted/rho_prob.op --cell tests/granted/rho_prob.cell
./bin/operon test tests/granted/rho_queue_shield.op --cell tests/granted/rho_queue_shield.cell
./bin/operon test tests/granted/rho_worker.op --cell tests/granted/rho_worker.cell
echo "[3/4] Differential harness (Rust core vs Python oracle)"
python3 bootstrap/harness.py
echo "[4/4] Oracle proof suite (the same frames on the second implementation)"
python3 bootstrap/oracle.py test tests/
# ai/ecosystem (W19): package CLI end-to-end — new/add/run/lock/search/publish/
# remove/update over the dir + HTTP registries, sha256 pinning, immutability.
echo "[5/5] Package system e2e (tests/package/pkg_e2e.sh)"
bash tests/package/pkg_e2e.sh bin/operon
# W011: the optimizer must preserve semantics on the whole corpus at every
# configuration (6 configs × full corpus, byte-identical vs tree-walk).
echo "[6/6] Optimizer parity matrix (scripts/opt_parity.sh)"
bash scripts/opt_parity.sh
echo "ALL GREEN"
