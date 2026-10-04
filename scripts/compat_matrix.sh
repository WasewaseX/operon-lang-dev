#!/usr/bin/env bash
# compat_matrix.sh — the compatibility matrix (docs/COMPAT.md).
#
# Runs every corpus program (committed tests/compat/ + fresh generated
# tests/compat_fresh/) through every engine axis and requires byte-identical
# stdout+stderr+exit-code across ALL of them:
#
#   A1  tree-walk      (Rust release, --no-vm; the VM is the default)
#   A2  bytecode VM    (Rust release, default)
#   A3  optimized VM   (Rust release, --opt 1)    <- rule R1: optimizations
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

# compat-r3 (environment-pair repair, F-assigned per digest-4, 2026-10-04;
# C veto-at-review): three boundary fixes so the matrix measures ENGINES,
# not the runner's locale. (1) Force the Python side to UTF-8/LF for every
# oracle invocation here — mirrors harness.py's W59 env, and belt-and-
# suspenders to the compat-r3 self-guard now living in oracle.py itself.
export PYTHONUTF8=1
export PYTHONIOENCODING=utf-8

# Run fingerprint (the triage post's "log run IDs"): one line naming the
# platform facts every comparison depends on, so a red leg is attributable
# from its log alone.
echo "run env: os=$(uname -s 2>/dev/null || echo windows) bash=$BASH_VERSION python=$(python3 -V 2>&1) gen=$GEN fresh=$FRESH seed=$SEED_FRESH mode=$MODE debug=$DEBUG_BIN"

# ---- fresh randomized programs (regenerated every run, never committed) --
if [ "$GEN" == "1" ]; then
  rm -rf tests/compat_fresh
  python3 scripts/gen_corpus.py --seed "$SEED_FRESH" --count "$FRESH" \
    --out tests/compat_fresh >/dev/null
  echo "fresh programs: $FRESH (seed $SEED_FRESH)"
fi

# compat-r3: loud canary — one tiny program (multi-line, UTF-8: U+2713 and
# an em-dash) through ALL engines BEFORE the corpus loop. If the canary
# itself diverges, the comparison boundary is broken on this runner
# (encoding/newline class) and every per-program "divergence" below would
# be phantom noise from the runner, not the engines. Fail fast naming the
# class instead of reporting 100% uniform divergence.
CANARY_DIR=$(mktemp -d)
CANARY="$CANARY_DIR/canary.op"
cna="$CANARY_DIR/a.out"; cnb="$CANARY_DIR/b.out"; cne="$CANARY_DIR/e.out"
printf 'print("canary: \xe2\x9c\x93 utf-8 \xe2\x80\x94 em-dash")\nprint("canary: line two")\n' > "$CANARY"
./bin/operon run --no-vm "$CANARY" >"$cna" 2>"$CANARY_DIR/a.err"; carc=$?
./bin/operon run "$CANARY" >"$cnb" 2>"$CANARY_DIR/b.err"; cbrc=$?
python3 bootstrap/oracle.py run "$CANARY" >"$cne" 2>"$CANARY_DIR/e.err"; cerc=$?
canary_ok=1
cmp -s "$cna" "$cnb" || canary_ok=0
cmp -s "$cna" "$cne" || canary_ok=0
[ "$carc" == "$cbrc" ] && [ "$carc" == "$cerc" ] || canary_ok=0
if [ "$canary_ok" != "1" ]; then
  echo "CANARY DIVERGED — comparison boundary broken on this runner (encoding/newline class), not the engines."
  echo "  rust(tree-walk) rc=$carc bytes:"; od -c "$cna" | head -3
  echo "  rust(vm)        rc=$cbrc bytes:"; od -c "$cnb" | head -3
  echo "  python(oracle)  rc=$cerc bytes:"; od -c "$cne" | head -3
  echo "  oracle stderr:"; head -3 "$CANARY_DIR/e.err"
  echo "  Aborting before the corpus loop: fix the boundary (PYTHONUTF8/PYTHONIOENCODING/newline normalization) and re-run."
  exit 2
fi
echo "canary: byte-identical across 3 engines (UTF-8/LF boundary healthy)"

# ---- debug build (A4) -----------------------------------------------------
# Always build (incremental, a no-op when current): a STALE debug binary is
# worse than none — it silently tests last week's engine (found the hard way
# when the debug binary predated the --no-vm flag and every program "failed").
if [ "$DEBUG_BIN" == "1" ]; then
  echo "building/refreshing debug binary..."
  cargo build >/dev/null 2>&1 || { echo "debug build FAILED"; exit 1; }
fi

# compat-r2 (macos leg, 2026-10-03): mapfile is a bash-4 builtin and the
# macOS runner's /bin/bash is 3.2 — the whole leg died here with
# "mapfile: command not found" then cascaded into unbound-variable errors.
# A while-read loop fills the same array on every bash this matrix runs.
FILES=()
while IFS= read -r f; do FILES+=("$f"); done < <(find tests/compat tests/compat_fresh -name '*.op' 2>/dev/null | sort)
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
trap 'rm -rf "$tmpa" "$tmpb" "$tmpc" "$tmpd" "$tmpe" "$tmpa.err" "$tmpb.err" "$tmpc.err" "$tmpd.err" "$tmpe.err" "$CANARY_DIR"' EXIT

for f in "${FILES[@]}"; do
  ./bin/operon run --no-vm "$f" >"$tmpa" 2>"$tmpa.err"; arc=$?
  ./bin/operon run "$f" >"$tmpb" 2>"$tmpb.err"; brc=$?
  ./bin/operon run --opt 1 "$f" >"$tmpc" 2>"$tmpc.err"; crc=$?
  if [ "$DEBUG_BIN" == "1" ]; then
    ./target/debug/operon run --no-vm "$f" >"$tmpd" 2>"$tmpd.err"; drc=$?
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
