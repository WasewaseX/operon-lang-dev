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
# W18: cancellation timing proofs need real OS threads (the sequential
# oracle cannot observe mid-flight ordering), so tests/timing/ is a
# Rust-only lane with its own explicit cell; both test walkers skip it.
./bin/operon test tests/timing/ --cell tests/timing/timing.cell
# W18: cancellation inheritance — the child's observation lands in a file
# under an explicit write grant (granted-lane pattern).
./bin/operon test tests/granted/cancel_inherit.op --cell tests/granted/cancel_inherit.cell
# W06 wave 2: try_env's Err/Ok lookup outcomes need exact-name env grants
# (zero-grant suite pins only the interference contract). The SET probe is
# exported here so the Ok payload is byte-pinned; the UNSET probe is
# granted but never exported, so the Err payload is byte-pinned.
OPERON_TRY_WAVE2_SET=w2ok ./bin/operon test tests/granted/try_env_wave2.op --cell tests/granted/try_env_wave2.cell
echo "[3/4] Differential harness (Rust core vs Python oracle)"
python3 bootstrap/harness.py
echo "[4.5/4] W101 diagnostic golden gate"
bash scripts/diag_golden.sh
# W08r: the interactive debugger e2e — piped-session contract (break fires,
# vars/p/s/c work, EOF resumes, piped stdin never wedges). Runs in the main
# gate so the debug surface can never silently rot again (it once did: the
# VM-default change killed every trap and no gate noticed).
bash scripts/debug_e2e.sh
echo "[4/4] Oracle proof suite (the same frames on the second implementation)"
python3 bootstrap/oracle.py test tests/
echo "ALL GREEN"
