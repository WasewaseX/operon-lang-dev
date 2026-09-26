# PACKAGING.md — distribution channels, honest validation ledger (W61)

Normative for `packaging/`. The rule this file enforces: **every channel
carries a validation mark, and the marks are honest.** A channel is either
`validated` (a CI job or a smoke script on this repo exercises it) or
`community` (a draft a human must finish and verify at publish time). Nothing
in between — "should work" is not a state we write down.

## Channel ledger

| channel | artifact / file | mark | what validated / what the human must do |
|---|---|---|---|
| GitHub release archives | `operon-<v>-<target>.tar.gz` / `.zip` + companion `.sha256` (5 targets) | **validated** | `scripts/release_smoke.sh` runs per-artifact smoke in CI; download → verify sha256 → run |
| install script | `scripts/install.sh` | community | curl-to-sh runs on YOUR machine (sandbox CI cannot grant network); review before piping |
| from source | `cargo install --path .` / `./scripts/build.sh` | **validated** | the CI cargo gate builds this exact path on every push |
| cargo-binstall | `[package.metadata.binstall]` in Cargo.toml | staged | metadata was validated against real v2.2.0 assets on the B5 branch (`b2/b5-packaging`, PR #14); it merges with the stack rebase — see the stack note below |
| Scoop (Windows) | `packaging/scoop/operon.json` | community | maintainer pins `hash` from the companion `.sha256`, then publishes a bucket |
| AUR (release) | `packaging/aur/PKGBUILD` | community | maintainer replaces `REPLACE_WITH_COMPANION_SHA256`, runs `makepkg -si`, publishes |
| AUR (git) | `packaging/aur/PKGBUILD.git` | community | same; `pkgver()` is generated at build time |
| Nix | `packaging/nix/default.nix` | community | maintainer replaces `lib.fakeSha256`, extends the arch map as targets ship |
| deb | `[package.metadata.deb]` in Cargo.toml | community | `cargo deb` in a clean env; CI packaging job = the validation upgrade path |
| rpm | `[package.metadata.generate-rpm]` in Cargo.toml | community | `cargo generate-rpm` in a clean env; same upgrade path |
| Homebrew | formula draft | staged | draft rides the B5 branch; the tap itself is owner-published |
| winget | submission note | staged | rides the B5 branch; winget needs a signed/published stable URL |

Marks are re-checked by `scripts/check_docs_sync.py` (W61 guard): every
`packaging/` path the README install matrix names must exist, and every
channel file under `packaging/` must be represented in the matrix — the two
documents cannot drift apart.

## The B5 stack note (why binstall + brew say "staged")

The B-track stack (`b2/b1-bench-suite` → `b2/b5-packaging`, PRs #12–#14)
merged into its own stack tip, which has **not** reached `main` yet. Its
content is based on an older tree, so the stack needs a REBASE onto current
main before merging — a plain merge would regress the README statistics
(15 std modules → 11, stale line counts) that W53–W58 made canonical. Until
that lands, this branch ships the NEW channels only and leaves the
`[package.metadata.binstall]` table to B5's rebase, so the same table never
exists twice.

## Publish checklist (owner/maintainer, per release)

1. Pin every `REPLACE_WITH_COMPANION_SHA256` / `fakeSha256` / empty `hash`
   from the release's `.sha256` companions.
2. Bump the version literals in `packaging/scoop/operon.json` and
   `packaging/aur/PKGBUILD*` (or script this once it hurts — a version-
   literal checker in check_docs_sync is the natural next step).
3. `makepkg -si` (AUR) / `nix-build` (Nix) / `cargo deb` / `cargo generate-rpm`
   locally; move a channel to `validated` ONLY when a CI job reproduces it.
4. Update this ledger in the same commit as any publish.
