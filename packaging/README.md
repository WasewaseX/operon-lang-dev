# Operon distribution

How Operon reaches users: source archives with checksums, binary releases,
and package manager formulas. User quickstart: README.md, "Build from source".

## Layout

| path | purpose |
|------|---------|
| ../scripts/release.sh | reproducible source archive + checksums into dist/ |
| ../scripts/install.sh | installer for release binaries, supports --verify |
| homebrew/operon.rb | Homebrew formula, builds the tag tarball from source |
| aur/PKGBUILD | AUR -git package, builds the main branch from source |
| build_exe.bat, genomelab.spec | Windows PyInstaller packaging (separate lane) |

## Source release (scripts/release.sh)

Packs HEAD via `git archive` (tracked files only, so target/, bin/, build/
and dist/ itself can never leak) into:

- `dist/operon-<version>.tar.gz`
- `dist/operon-<version>.tar.gz.sha256` (sha256sum format)
- `dist/SHA256SUMS` (manifest, one line per published artifact)

Version comes from Cargo.toml; override with `--version X.Y.Z`.
`--dry-run` prints the plan and writes nothing. `--verify` re-checks an
existing archive against its .sha256 file and (if present) the manifest,
exiting nonzero on any mismatch. GNU tar is detected for byte-stable
packing (`--sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner`,
plus `gzip -n`); without it the script falls back to plain git archive
output, which is still deterministic for a given commit.

## Checksum workflow

Maintainer:
1. tag vX.Y.Z, run `scripts/release.sh`
2. attach `dist/operon-<version>.tar.gz`, its `.sha256`, and `SHA256SUMS`
   to the release page
3. binary assets built by CI (release.yml) ship per-asset `.sha256` files;
   append those lines into `SHA256SUMS` too, so one manifest covers every
   artifact of the release

User:
1. download the archive and `SHA256SUMS` from the release page
2. `sha256sum -c SHA256SUMS` (or the single `.sha256` file)
3. only then untar; a mismatch means do not use the archive, report it

`scripts/install.sh --verify` applies the same rule to binary installs:
the release must publish a `SHA256SUMS` manifest listing the asset, and
the installer fails closed when it is missing or the hash differs. Without
the flag, behavior is unchanged: the per-asset `.sha256` check stays
mandatory and fail-closed.

## Package managers

Homebrew (`homebrew/operon.rb`): builds from the GitHub tag tarball
(`/archive/refs/tags/v<version>.tar.gz`), `depends_on "rust" => :build`
and `gcc` (C++ kernels), installs `bin/operon` and `bin/operon-ls`, and
puts `std/` and `bootstrap/` into `libexec` with a `bin/std` symlink so
exe-relative module resolution (src/genes.rs) works. The `sha256` stanza
is filled from the release.sh output after tagging; the placeholder is
zeroed so `brew audit` catches an unfilled formula.

AUR (`aur/PKGBUILD`): `-git` package, `pkgver` from `git describe` (with
a Cargo.toml fallback when no tags exist), `makedepends=('rust' 'gcc'
'git')`, runs `scripts/build.sh`, installs `bin/operon` and
`bin/operon-ls` to `/usr/bin`, `std/` and `bootstrap/` to
`/usr/lib/operon`, and links `/usr/bin/std` to `/usr/lib/operon/std` for
the same exe-relative resolution. `sha256sums=('SKIP')` is correct for a
VCS source: makepkg verifies the git checkout instead.

## What archives contain (read this twice)

Release archives contain NO build artifacts: source only. No binaries, no
`target/`, no `bin/`, no `build/`, no `dist/`. Binaries reach users
through the package managers above, through `scripts/install.sh` (which
downloads CI-built release assets and verifies checksums before
installing anything), or by building from source with `scripts/build.sh`.
