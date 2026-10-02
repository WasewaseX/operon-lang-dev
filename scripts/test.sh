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
# W08r: the async corpus runs on the fiber lane (io.pool cell): the virtual
# clock makes fuel deterministic. The thread lane charges parked workers per
# REAL millisecond — on slow CI runners that burned the pool on identical
# files that pass locally (the [burned] failures on 6e7b7b7) — so both test
# walkers skip tests/async (the timing-lane precedent) and the corpus is
# exercised here, cell-gated; substrate equivalence stays pinned by
# tests/async_parity.rs (8 two-lane programs byte-identical) and the
# FIFO-wake determinism repeat.
./bin/operon test tests/async/ --cell tests/async/async.cell
./bin/operon test tests/async/timing/async_sleeps.op --cell tests/async/async.cell
./bin/operon test tests/async/timing/async_wake_order.op --cell tests/async/async.cell --repeat 3
# W18: cancellation inheritance — the child's observation lands in a file
# under an explicit write grant (granted-lane pattern).
./bin/operon test tests/granted/cancel_inherit.op --cell tests/granted/cancel_inherit.cell
# W06 wave 2: try_env's Err/Ok lookup outcomes need exact-name env grants
# (zero-grant suite pins only the interference contract). The SET probe is
# exported here so the Ok payload is byte-pinned; the UNSET probe is
# granted but never exported, so the Err payload is byte-pinned.
OPERON_TRY_WAVE2_SET=w2ok ./bin/operon test tests/granted/try_env_wave2.op --cell tests/granted/try_env_wave2.cell
# W006-B: wave-3 IO try_ granted lane — resolved-prefix read grant over the
# fixture dir (ok payloads + missing payloads with raw path echo), run grant
# for true, net grant for the closed-loopback missing probe. The zero-grant
# suite (tests/differential/try_io.op) pins interference + arity + the
# capless str_from_bytes shape.
./bin/operon test tests/granted/try_io_wave3.op --cell tests/granted/try_io_wave3.cell
echo "[3/4] Differential harness (Rust core vs Python oracle)"
python3 bootstrap/harness.py
echo "[4.5/4] W101 diagnostic golden gate"
bash scripts/diag_golden.sh
# W08r: the interactive debugger e2e — piped-session contract (break fires,
# vars/p/s/c work, EOF resumes, piped stdin never wedges). Runs in the main
# gate so the debug surface can never silently rot again (it once did: the
# VM-default change killed every trap and no gate noticed).
bash scripts/debug_e2e.sh
# W08r stage 2: the machine protocol e2e — a Python client drives
# `debug --protocol=json` (NDJSON purity, stop reasons, stack/vars/eval,
# all stepping verbs, runtime breakpoint management, print rerouting).
python3 scripts/debug_protocol_e2e.py
# W08r stage 3: the DAP adapter e2e — a Python DAP client drives `operon dap`
# over Content-Length framing (lifecycle, stopped events, stackTrace/scopes/
# variables/evaluate, stepping verbs, output events, exited/terminated).
python3 scripts/dap_e2e.py
# W001 stage 2: the static checker e2e — the typeck module (T01..T10) landed
# via the ai/type-system merge with NO gate (the debug_e2e rot class). This
# pins the flagship catch, alias resolution, Total-Graceful unknown names,
# the prime directive (plain check/run untouched by typed findings), and
# the stage-1 runtime soft contract as the compatibility fallback.
bash scripts/typeck_e2e.sh
# W060: the release smoke script joins the standing gate (the rot lesson —
# debug_e2e once sat outside every gate and silently died). Dir-mode smoke
# against the bin/ artifact this gate already uses: version, a real run, a
# check pass, std/ presence. The release.yml wiring is the W060 done-when
# (per-asset smoke on the exact uploaded bytes, failed smoke = failed
# release); this stanza keeps the script itself honest between releases.
bash scripts/release_smoke.sh bin
echo "[4/4] Oracle proof suite (the same frames on the second implementation)"
python3 bootstrap/oracle.py test tests/
echo "ALL GREEN"
