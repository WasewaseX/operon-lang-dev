#!/usr/bin/env bash
# build.sh — Operon toolchain build (Cargo drives Rust + C + C++ kernels).
# Produces: bin/operon (release binary), build/*.o + liboperon_rt.a for the
# C smoke test (tests/smoke_runtime.c). The Rust side compiles the same
# kernels itself via build.rs (cc), so cargo build is self-contained.
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p build bin

echo "[1/3] Cargo build (Rust core; C + C++ kernels via build.rs)"
cargo build --release
cp target/release/operon bin/operon

echo "[2/3] C runtime kernel objects (for tests/smoke_runtime.c)"
gcc -O2 -Wall -Wextra -std=c17 -fPIC -c runtime/operon_rt.c -o build/operon_rt.o

echo "[3/3] C++ codon kernel objects"
g++ -O3 -Wall -Wextra -std=c++17 -fPIC -fno-exceptions -c runtime/codon_kernel.cpp -o build/codon_kernel.o
ar rcs build/liboperon_rt.a build/operon_rt.o build/codon_kernel.o

echo "OK: bin/operon"
./bin/operon version
