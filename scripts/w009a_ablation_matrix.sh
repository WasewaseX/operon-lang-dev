#!/usr/bin/env bash
# W009-A ablation matrix (builder-A, 2026-10-02) — evidence-only harness.
# Requires the W009-A ablation commits (src/w009a.rs + call-site gates);
# with them absent the env vars are simply ignored and every row measures
# the same binary. Rebuilds nothing; run scripts/build.sh first.
#
# Usage: bash scripts/w009a_ablation_matrix.sh
# Writes: docs/bench/2026-10-02-w009a-ablation.txt (gitignored locally? no —
# overwrite deliberately to re-derive the evidence; the committed copy is the
# recorded 2026-10-02 sandbox run).
set -u
cd "$(dirname "$0")/.."
BIN=./bin/operon
OUT=docs/bench/2026-10-02-w009a-ablation.txt

echo "=== machine ===" | tee "$OUT"
rg "model name" /proc/cpuinfo | head -1 | tee -a "$OUT"
nproc | tee -a "$OUT"
echo "=== matrix (operon bench --iters 30, min/avg ms, in-process) ===" | tee -a "$OUT"

run_cfg() {
  local label="$1"; shift
  local out res
  out=$(env "$@" "$BIN" run scripts/bench/fib25.op 2>/dev/null)
  if [[ "$out" != "fib(25) = 75025" ]]; then
    echo "$label | WRONG OUTPUT: $out" | tee -a "$OUT"
    return
  fi
  res=$(env "$@" "$BIN" bench scripts/bench/fib25.op --iters 30 2>/dev/null | tail -1)
  echo "$label | $res" | tee -a "$OUT"
}

run_cfg "base  (scratch, flags off)"
run_cfg "tick  (no fuel tick in dispatch)"        OPERON_W009A_ABLATE=tick
run_cfg "tb    (lazy traceback frame)"            OPERON_W009A_ABLATE=tb
run_cfg "promo (promoter clone after check)"      OPERON_W009A_ABLATE=promo
run_cfg "decay (no m6a/grn decay tickers)"        OPERON_W009A_ABLATE=decay
run_cfg "bk    (no call bookkeeping at all)"      OPERON_W009A_ABLATE=bk
run_cfg "gates (whole regulatory block skipped)"  OPERON_W009A_ABLATE=gates
run_cfg "pool  (Env map/consts free-list)"        OPERON_W009A_ABLATE=pool
run_cfg "SAFE  (gates,tb,promo — fix-shaped)"     OPERON_W009A_ABLATE=gates,tb,promo
run_cfg "ALL   (everything above)"                OPERON_W009A_ABLATE=all

echo "=== optimizer irrelevance (end-to-end run, min of 5, incl startup) ===" | tee -a "$OUT"
python3 - <<'EOF' 2>&1 | tee -a "$OUT"
import subprocess, time
def t(args):
    ts = []
    for _ in range(6):
        s = time.perf_counter()
        subprocess.run(args, capture_output=True)
        ts.append(time.perf_counter() - s)
    ts.pop(0)
    return min(ts) * 1000
print(f"run default (opt 0): {t(['./bin/operon','run','scripts/bench/fib25.op']):7.1f} ms")
print(f"run --opt 2        : {t(['./bin/operon','run','scripts/bench/fib25.op','--opt','2']):7.1f} ms")
print(f"run --opt-passes all: {t(['./bin/operon','run','scripts/bench/fib25.op','--opt-passes','all']):6.1f} ms")
print(f"run --no-vm        : {t(['./bin/operon','run','--no-vm','scripts/bench/fib25.op']):7.1f} ms")
EOF

echo "=== counters (OPERON_W009A_COUNTS=1, flags off) ===" | tee -a "$OUT"
OPERON_W009A_COUNTS=1 "$BIN" run scripts/bench/fib25.op 2>&1 >/dev/null | rg "w009a" | tee -a "$OUT"
