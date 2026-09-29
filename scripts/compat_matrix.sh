#!/usr/bin/env bash
# compat_matrix.sh — the compatibility matrix (docs/COMPAT.md).
#
# Runs every corpus program (committed tests/compat/ + fresh generated
# tests/compat_fresh/) through every engine axis and requires byte-identical
# stdout+stderr+exit-code across ALL of them:
#
#   A1  tree-walk      (Rust release)
#   A2  bytecode VM    (Rust release, --vm)
#   A3  optimized VM   (Rust release, --vm-opt)   <- rule R1: optimizations
#                                                      must preserve semantics
#   A4  tree-walk      (Rust DEBUG build)          <- build profile axis
#   A5  Python oracle                              <- implementation axis
#
# The debug binary is built on demand into target/debug/operon-compat.
# FAST=1 restricts to a deterministic 10% sample (for quick loops); the
# full matrix runs every program.
set -uo pipefail
cd "$(dirname "$0")/.."

GEN="${GEN:-0}"
FRESH="${FRESH:-800}"
SEED_FRESH="${SEED_FRESH:-$(date +%Y%m%d)}"
MODE="${MODE:-full}"
DEBUG_BIN="${DEBUG_BIN:-1}"

echo "== compat matrix (mode=$MODE) =="

# ---- fresh randomized programs (regenerated every run, never committed) --
if [ "$GEN" == "1" ]; then
  rm -rf tests/compat_fresh
  python3 scripts/gen_corpus.py --seed "$SEED_FRESH" --count "$FRESH" \
    --out tests/compat_fresh >/dev/null
  echo "fresh programs: $FRESH (seed $SEED_FRESH)"
fi

# ---- debug build (A4) -----------------------------------------------------
if [ "$DEBUG_BIN" == "1" ]; then
  if [ ! -x target/debug/operon ]; then
    echo "building debug binary..."
    cargo build >/dev/null 2>&1 || { echo "debug build FAILED"; exit 1; }
  fi
fi

mapfile -t FILES < <(find tests/compat tests/compat_fresh -name '*.op' 2>/dev/null | sort)
if [ "$MODE" == "fast" ]; then
  n=${#FILES[@]}
  step=$(( n / 120 )); [ "$step" -lt 1 ] && step=1
  sampled=()
  for ((i=0; i<n; i+=step)); do sampled+=("${FILES[i]}"); done
  FILES=("${sampled[@]}")
fi
echo "programs under test: ${#FILES[@]}"

pass=0; fail=0; failed_files=()
tmpa=$(mktemp); tmpb=$(mktemp); tmpc=$(mktemp); tmpd=$(mktemp); tmpe=$(mktemp)
trap 'rm -f "$tmpa" "$tmpb" "$tmpc" "$tmpd" "$tmpe"' EXIT

for f in "${FILES[@]}"; do
  ./bin/operon run "$f" >"$tmpa" 2>"$tmpa.err"; arc=$?
  ./bin/operon run --vm "$f" >"$tmpb" 2>"$tmpb.err"; brc=$?
  ./bin/operon run --vm-opt "$f" >"$tmpc" 2>"$tmpc.err"; crc=$?
  if [ "$DEBUG_BIN" == "1" ]; then
    ./target/debug/operon run "$f" >"$tmpd" 2>"$tmpd.err"; drc=$?
  fi
  python3 bootstrap/oracle.py run "$f" >"$tmpe" 2>"$tmpe.err"; erc=$?

  ok=1
  cmp -s "$tmpa" "$tmpb" || ok=0
  cmp -s "$tmpa" "$tmpc" || ok=0
  cmp -s "$tmpa.err" "$tmpb.err" || ok=0
  cmp -s "$tmpa.err" "$tmpc.err" || ok=0
  [ "$arc" == "$brc" ] && [ "$arc" == "$crc" ] || ok=0
  if [ "$DEBUG_BIN" == "1" ]; then
    cmp -s "$tmpa" "$tmpd" || ok=0
    cmp -s "$tmpa.err" "$tmpd.err" || ok=0
    [ "$arc" == "$drc" ] || ok=0
  fi
  cmp -s "$tmpa" "$tmpe" || ok=0
  cmp -s "$tmpa.err" "$tmpe.err" || ok=0
  [ "$arc" == "$erc" ] || ok=0

  if [ "$ok" == "1" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed_files+=("$f")
    if [ "${VERBOSE:-0}" == "1" ] && [ "$fail" -le 5 ]; then
      echo "DIVERGE: $f (rc $arc/$brc/$crc/$drc/$erc)"
      diff "$tmpa" "$tmpc" | head -4
      diff "$tmpa" "$tmpe" | head -4
    fi
  fi
done

echo "compat matrix: $pass identical, $fail divergent (${#FILES[@]} programs x $([ "$DEBUG_BIN" == "1" ] && echo 5 || echo 4) engines)"
if [ "$fail" -gt 0 ]; then
  printf '  divergent: %s\n' "${failed_files[@]}"
  exit 1
fi
echo "COMPAT MATRIX GREEN"
