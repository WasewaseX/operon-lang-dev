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
