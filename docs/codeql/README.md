# CodeQL, static security & quality analysis (W099 lane support)

Toolchain: CodeQL CLI v2.27.1 (github/codeql-cli-binaries), suite
`codeql/rust-queries:codeql-suites/rust-security-and-quality.qls`, the
GitHub security-and-quality suite (security-severity-weighted queries +
quality queries) over the full Rust core (28 source files, 0 extraction
errors).

Run (2026-09-27, main @ CodeQL fix commit):

1. First run: **1 finding**, `rust/unused-variable` (`closure` binding,
   src/interp.rs call_value sequence arm). Cosmetic, not a security
   finding; the closure env is consumed by the sequence worker path.
2. Fix: binding renamed `_closure` (semantics unchanged).
3. Re-run: **0 findings**, the security-and-quality suite is clean.

Reproduce:

    codeql database create /tmp/operon-db --language=rust --source-root=.
    codeql database analyze /tmp/operon-db \
      codeql/rust-queries:codeql-suites/rust-security-and-quality.qls \
      --format=sarifv2.1.0 --output=out.sarif

`rust-security-and-quality.sarif` is the clean re-run; `.before.sarif`
preserves the one pre-fix finding for the audit trail.

## Re-run after the M100 dev1 waves (2026-09-27, main 44c37fe)

Fresh database over the current tree (vm.rs, pkg.rs registry, debugger,
cancellation, scope, task groups included): rust-security-and-quality suite
returns 0 findings. Evidence: rust-security-and-quality.sarif in this folder.

## Re-run after the PR #28 merge (2026-09-27, merged tree: d9b1034 + main f0527e5)

Fresh database over the merged tree (30 source files, 0 extraction errors): rust-security-and-quality suite returns 0 findings. Evidence: rust-security-and-quality.sarif in this folder.

Run (2026-10-01, main @ c4d258f, v2.7.0 merge): **0 findings** —
`2026-10-01-v270-result-migration.sarif` (45 Rust files extracted, 30 rules,
security-and-quality suite, CLI 2.27.1). Fresh-sandbox re-install of the CLI
from the official bundle; query pack codeql/rust-queries@0.1.43 downloaded on
first resolve. Covers the W006 wave-2 Result builtins merged in PR #43.

- 2026-10-02 — `2026-10-02-w010a-disasm.sarif` — **0 findings** (45 Rust files
extracted, 30 rules, security-and-quality suite, CLI 2.27.1). Builder-D session
snapshot run on the W010-A head f19a86f (PR #53): the disasm line-annotation,
all_mnemonics() table and stability-suite growth touch no execution path; the
run re-establishes the 0-finding baseline for the debugger/tooling lane before
the W008 polish task starts from this head.
