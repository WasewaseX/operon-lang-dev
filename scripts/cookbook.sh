#!/usr/bin/env bash
# cookbook.sh — Operon cookbook verifier (B3, builder-B).
# Every examples/cookbook/*.op is a small real program with a frozen,
# deterministic expected output (examples/cookbook/expected/<name>.out).
# This script runs each program and diffs the output — on the Rust core
# AND, when available, the Python oracle (bootstrap/oracle.py), so an
# example can never rot silently and can never silently diverge between
# the two cores. Exit 1 on any mismatch or failure.
#
# Exception: the packages chapter (#20) is a shell transcript
# (examples/cookbook/packages.sh) that drives the package CLI against a
# throwaway sandbox; it verifies on the Rust core only (the oracle is a
# language interpreter, not a package manager) and is skipped entirely
# under --core python.
#
# Usage:
#   bash scripts/cookbook.sh                 # verify on both cores
#   bash scripts/cookbook.sh --core rust     # Rust core only
#   bash scripts/cookbook.sh --core python   # oracle only
#   bash scripts/cookbook.sh --update        # re-freeze expected outputs
#                                            # (audit the diff before committing!)
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

CORE="both"
UPDATE=0
while [ $# -gt 0 ]; do
    case "$1" in
        --core)  CORE="$2"; shift 2 ;;
        --update) UPDATE=1; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

if [ -x target/release/operon ]; then
    OPERON=target/release/operon
elif [ -x bin/operon ]; then
    OPERON=bin/operon
else
    bash scripts/build.sh > /dev/null
    OPERON=target/release/operon
fi
ORACLE=bootstrap/oracle.py

echo "operon: $($OPERON version 2>/dev/null || $OPERON --version 2>/dev/null | head -1)"
echo "python: $(python3 --version)"
echo ""

pass=0
fail=0
mkdir -p examples/cookbook/expected

for prog in examples/cookbook/*.op; do
    name="$(basename "$prog" .op)"
    expected="examples/cookbook/expected/${name}.out"

    run_rust()   { timeout 120 "$OPERON" run "$prog" 2>/dev/null; }
    run_python() { timeout 120 python3 "$ORACLE" run "$prog" 2>/dev/null; }

    ok=1
    if [ "$CORE" = "rust" ] || [ "$CORE" = "both" ]; then
        rust_out="$(run_rust || true)"
        if [ "$UPDATE" = "1" ]; then
            printf '%s\n' "$rust_out" > "$expected"
        elif [ ! -f "$expected" ]; then
            echo "MISSING  $name (no expected output at $expected)"
            ok=0
        elif [ "$rust_out" != "$(cat "$expected")" ]; then
            echo "FAIL     $name (rust core output != expected)"
            ok=0
        fi
    fi
    if [ "$CORE" = "python" ] || [ "$CORE" = "both" ]; then
        if [ -f "$ORACLE" ]; then
            py_out="$(run_python || true)"
            if [ "$UPDATE" = "1" ]; then
                : # freeze step above used the rust output; oracle must match it
            fi
            if [ "$py_out" != "$(cat "$expected" 2>/dev/null || echo __none__)" ]; then
                echo "FAIL     $name (python oracle output != expected)"
                ok=0
            fi
        fi
    fi

    if [ "$ok" = "1" ]; then
        echo "PASS     $name"
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
    fi
done

# ---- chapter #20: packages (shell transcript, Rust core only)
name=packages
prog=examples/cookbook/packages.sh
expected="examples/cookbook/expected/${name}.out"
ok=1
if [ "$CORE" = "rust" ] || [ "$CORE" = "both" ]; then
    if [ "$UPDATE" = "1" ]; then
        bash "$prog" > "$expected"
    fi
    pkg_out="$(bash "$prog" 2>/dev/null || true)"
    if [ ! -f "$expected" ]; then
        echo "MISSING  $name (no expected output at $expected)"
        ok=0
    elif [ "$pkg_out" != "$(cat "$expected")" ]; then
        echo "FAIL     $name (rust core output != expected)"
        ok=0
    fi
    if [ "$ok" = "1" ]; then
        echo "PASS     $name"
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
    fi
fi

echo ""
echo "cookbook: $pass passed, $fail failed"
[ "$fail" = "0" ]
