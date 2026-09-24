#!/usr/bin/env bash
# build.sh — Operon toolchain build (Cargo drives Rust + C++ kernel).
# Produces: bin/operon + bin/operon-ls, build/codon_kernel.o for the
# native smoke test (tests/smoke_codon.cpp). The Rust side compiles the
# same kernel itself via build.rs (cc), so cargo build is self-contained.
# (sec-r2, audit A15: the C runtime kernel was deleted — its interning
# table now lives in Rust; see src/ffi.rs.)
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p build bin

echo "[1/2] Cargo build (Rust core; C++ kernel via build.rs)"
cargo build --release
cp target/release/operon bin/operon

cp target/release/operon-ls bin/operon-ls

echo "[2/2] C++ codon kernel objects (for tests/smoke_codon.cpp)"
g++ -O3 -Wall -Wextra -std=c++17 -fPIC -fno-exceptions -c runtime/codon_kernel.cpp -o build/codon_kernel.o

echo "OK: bin/operon, bin/operon-ls"
./bin/operon version
