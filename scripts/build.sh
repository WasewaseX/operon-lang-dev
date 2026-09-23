#!/usr/bin/env bash
# build.sh — Operon toolchain build (Rust + C + C++).
# Produces: build/liboperon_rt.a, bin/operon (the native toolchain binary).
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p build bin

echo "[1/3] C runtime kernel"
gcc -O2 -Wall -Wextra -std=c17 -fPIC -c runtime/operon_rt.c -o build/operon_rt.o

echo "[2/3] C++ codon kernel"
g++ -O3 -Wall -Wextra -std=c++17 -fPIC -fno-exceptions -c runtime/codon_kernel.cpp -o build/codon_kernel.o

echo "[3/3] Rust core (linking native kernels)"
ar rcs build/liboperon_rt.a build/operon_rt.o build/codon_kernel.o
rustc -O --edition 2021 \
  -L build -l static=operon_rt -l dylib=stdc++ \
  src/main.rs -o bin/operon

echo "OK: bin/operon"
./bin/operon version
