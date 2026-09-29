# TOOLCHAIN — what it takes to build a language like Operon

Canonical toolchain statement (adopted from the owner, 2026-09):

> Rust + Cargo, a C/C++ compiler, Python 3, shell/CI, and your own
> parser/evaluator/verification harness. Everything else is design,
> discipline, and iteration.

This file is the one-view inventory. The long-form narrative lives in the
owner's report (Operon Build and Toolchain Report); this file is the version
that lives with the code.

## Layers

| Layer | Choice | Where it lives here |
|---|---|---|
| Host language | Rust: lexer, parser, AST, evaluator, runtime, CLI, LSP | `src/` (~22k lines) |
| Performance kernel | C / C++, optional, only with a benchmark in hand | small kernels built by `scripts/build.sh` |
| Bootstrap / testing | Python 3: oracle, differential harness, glue. Never shipped in the binary | `bootstrap/` |
| Parser approach | Hand-written recursive descent (Total Grammar). Tree-sitter / Langium / K Framework evaluated and rejected | `src/parser.rs` |
| Build tools | cargo, gcc/g++ or clang, shell scripts, CI | `Cargo.toml`, `scripts/*.sh`, CI workflows |
| Verification | Proof frames, differential harness, red-team corpus, fuzz stress, benchmarks, expectation pins | `tests/`, `bootstrap/harness.py`, `scripts/redteam.sh`, `scripts/bench.sh` |
| Developer tooling | REPL, formatter, operon-ls, watch, profiler, embedding guide | `src/bin/`, `docs/` |
| Distribution | Release archives, checksums, install script, Homebrew/AUR metadata | queued for the ecosystem wave |
| Security model | Capability sandbox, fuel, caps, audits — a design requirement, not a tool | enforced in `src/interp.rs`, audited by `scripts/redteam.sh` |

## Parser approach note

The Total Grammar invariant requires that malformed input degrades into a
usable tree plus notes, never a dead-end syntax error. Generated parsers
optimize the happy path; this project needs total control of every failure
path and byte-parity with the mirrored oracle, so the parser is hand-written.

## Verification layers

1. Proof frames: 112 files / 98 proofs / 1,298 assertions, green on both cores.
2. Differential harness: 143/143 programs MATCH byte-for-byte (5 granted-lane
   targets under explicit operator cells).
3. Red-team corpus: 100 payloads, 0 breaches.
4. Fuzz-style stress: deep nesting, pathological programs.
5. Benchmarks: timing regression gates.
6. Expectation pins: statistical sanity checks. Lesson of loop-10:
   byte-parity is blind to mirrored bugs; expectation-based pins are the
   missing sense.

## Explicitly NOT needed

**CodeQL is not part of this toolchain.** CodeQL is a static
security-analysis tool for existing languages. It is not used to create a
language. It may optionally be pointed at the Rust/C++/Python implementation
code later to scan that code for vulnerabilities, but it does not belong in
the language-building checklist and has been removed from it.
