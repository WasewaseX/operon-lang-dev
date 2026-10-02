## Verify before you run

Every binary asset ships with a SHA256 sidecar, and this release carries a
whole-release `SHA256SUMS` manifest covering every binary asset:

```sh
# per-asset check
sha256sum -c operon-<version>-<target>.tar.gz.sha256
# whole-release check (every asset in one file)
sha256sum -c SHA256SUMS
```

Or let the installer do it — verification is fail-closed there (a missing or
mismatched checksum refuses the install, never skips it):

```sh
curl -fsSL https://raw.githubusercontent.com/WasewaseX/operon-lang-dev/main/scripts/install.sh | sh
# strict mode: additionally requires the whole-release SHA256SUMS manifest
curl -fsSL https://raw.githubusercontent.com/WasewaseX/operon-lang-dev/main/scripts/install.sh | sh -s -- --verify
```

## What is inside

Each package ships the `operon` binary, the `operon-ls` language server, the
self-hosted `std/` library (required next to the binary — the module loader
resolves it exe-relative), `examples/`, and README / LICENSE / TUTORIAL.
Smoke-gated on the exact uploaded bytes per target before attach: version,
a real program run, a check pass, `std/` presence, `operon-ls --version`.
