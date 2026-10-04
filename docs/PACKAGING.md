# PACKAGING.md, distribution channels, honest validation ledger (W61)

Normative for `packaging/`. The rule this file enforces: **every channel
carries a validation mark, and the marks are honest.** A channel is either
`validated` (a CI job or a smoke script on this repo exercises it) or
`community` (a draft a human must finish and verify at publish time). Nothing
in between, "should work" is not a state we write down.

## Channel ledger

| channel | artifact / file | mark | what validated / what the human must do |
|---|---|---|---|
| GitHub release archives | `operon-<v>-<target>.tar.gz` / `.zip` + companion `.sha256` (5 targets) | **validated** | `scripts/release_smoke.sh` runs per-artifact smoke in CI; download → verify sha256 → run |
| install script | `scripts/install.sh` | community | curl-to-sh runs on YOUR machine (sandbox CI cannot grant network); review before piping |
| from source | `cargo install --path .` / `./scripts/build.sh` | **validated** | the CI cargo gate builds this exact path on every push |
| cargo-binstall | `[package.metadata.binstall]` in Cargo.toml | community | the template contract (asset naming, inner dir, binaries) is pinned by `scripts/pkg_meta_check.py` in the standing gate, so it cannot drift behind release.yml; a maintainer still runs `cargo binstall operon` once against a real release before calling it validated |
| Scoop (Windows) | `packaging/scoop/operon.json` | community | maintainer pins `hash` from the companion `.sha256`, then publishes a bucket |
| AUR (release) | `packaging/aur/PKGBUILD` | community | maintainer replaces `REPLACE_WITH_COMPANION_SHA256`, runs `makepkg -si`, publishes |
| AUR (git) | `packaging/aur/PKGBUILD.git` | community | same; `pkgver()` is generated at build time |
| Nix | `packaging/nix/default.nix` | community | maintainer replaces `lib.fakeSha256`, extends the arch map as targets ship |
| deb | `[package.metadata.deb]` in Cargo.toml | community | `cargo deb` in a clean env; CI packaging job = the validation upgrade path |
| rpm | `[package.metadata.generate-rpm]` in Cargo.toml | community | `cargo generate-rpm` in a clean env; same upgrade path |
| Homebrew | `packaging/homebrew/operon.rb` | community | maintainer pins the tag tarball's `.sha256` into the formula, then owner-publishes the tap |
| source release archive | `scripts/release.sh` → `dist/operon-<v>.tar.gz` + `SHA256SUMS` | validated locally | deterministic `git archive` re-tar (pinned mtime/uid/gid), `--verify` fail-closed; run it before tagging so SHA256SUMS is fresh |
| whole-release `SHA256SUMS` (binary assets) | `.github/workflows/release.yml` `sha256sums` job | validated | hashes the exact published bytes after sidecar re-verification; partial releases refused; pinned to the build matrix by `scripts/pkg_meta_check.py`; consumed by `install.sh --verify` |
| installer (curl \| sh) | `scripts/install.sh` + `scripts/install_e2e.sh` | validated | fail-closed verification (sidecar mandatory, manifest cross-checked, `--verify` strict); hermetic offline mode (`OPERON_INSTALL_ASSET_DIR`) exercised by `install_e2e.sh` in the standing gate AND by the `Hermetic installer gate (W061-J)` step in every `release.yml` build job — the exact packaged bytes are install-verified (sidecar fail-closed path, installed binary runs and reports the tree version) in every release run, dry-run rehearsals included |
| hosted registry server (W19-r2) | `packaging/registry/app.py` · `packaging/registry/requirements.txt` · `packaging/registry/render.yaml` | community | self-hosted tier (docs/specs/REGISTRY.md); the HTTP + WSGI surfaces and the render.yaml wiring are pinned by `scripts/pkg_meta_check.py` + `scripts/pkg_hosted_e2e.sh` in the standing gate; an actual deployment (Render or your own host) is a maintainer action |
| winget | submission note | staged | winget needs a signed/published stable URL; submission is owner-gated (the 2026-10-02 owner triage) |

Marks are re-checked by `scripts/check_docs_sync.py` (W61 guard): every
`packaging/` path the README install matrix names must exist, and every
channel file under `packaging/` must be represented in the matrix, the two
documents cannot drift apart.

## The B5 stack note (what actually happened)

The B-track stack (`b2/b1-bench-suite` → `b2/b5-packaging`, PRs #12–#14)
merged through its own stack tip; PR #14 shows merged on GitHub, but its
`Cargo.toml` delta (the `[package.metadata.binstall]` table) **never reached
`main`** — the table lived on the stack tip only, and the branch is gone from
the remote. Every doc that said the table "lands with the B5 stack merge" was
waiting for a merge that had already happened without it: the cargo-binstall
channel had silently died (found by the W061-A audit, 2026-10-02). The table
is restored directly on `main` now, with its template contract pinned by
`scripts/pkg_meta_check.py` so it can never silently drift behind release.yml
again. The old hazard that kept the stack unmerged (its README diff was based
on an older tree and would have regressed the W53–W58 canonical statistics)
never applied to the metadata table itself — the table is inert to the build.

## Source release flow (scripts/release.sh)

`scripts/release.sh` packs HEAD with `git archive` (tracked files only, so
`target/`, `bin/`, `build/` and `dist/` itself cannot leak) into
`dist/operon-<version>.tar.gz`, writes the archive's `.sha256` companion and
a `SHA256SUMS` manifest, and re-tars with pinned metadata (mtime epoch 0,
uid/gid 0) when GNU tar is available, so two runs of the same commit produce
byte-identical archives. `--verify` checks an existing archive fail-closed;
`--dry-run` prints the plan. The manifest is what `install.sh --verify`
consumes. S5 (2026-10-02) closed the gap the old wording papered over: the
release workflow now has a `sha256sums` job that publishes a whole-release
`SHA256SUMS` (every binary asset, hashed from the exact published bytes
downloaded back from the release, each sidecar re-verified first, a partial
asset set refused), so strict mode is enforceable against real releases —
previously only the per-asset `.sha256` sidecars existed and `--verify`
could only ever fail closed. `scripts/pkg_meta_check.py` pins the job to
the build matrix: a target added to the matrix without a matching manifest
entry fails the standing gate, not a live release.

## Release notes template (S5)

Every release body starts from `.github/release-notes-template.md` — the
verification preamble (per-asset check, whole-release check, installer
strict mode) — with GitHub's auto-generated notes appended
(`generate_release_notes: true`). The template is pinned by
`scripts/pkg_meta_check.py`; edit it when the verification story changes,
not per release.

## Pipeline rehearsal (#51, S5)

The smoke gate's first live run must not be a real release (v2.7.0 shipped
unsmoked — the W060 wiring had zero executions against uploaded bytes).
`release.yml` therefore takes a `workflow_dispatch` input, `dry_run`:
it builds and smokes every matrix target exactly like a tag push — per-asset
smoke rows in the run log are the acceptance evidence — but attaches nothing
to any release and skips the manifest job. Dispatch refs are sanitised
(`/` → `-` in the version slot; tags unaffected). One rehearsal per pipeline
change is the honest cadence; the runs are free of repo side effects.

The smoke's *expected version* is ref-aware, per the first live rehearsal's
catch (run 36992837397): on a tag push the expectation is the tag minus `v`
(the release-version contract); on any non-tag ref the expectation falls
back to the built tree's `Cargo.toml` version — a branch name is not a
version a dispatch-built binary could ever carry, and the first rehearsal
failed all four smoked targets on exactly that mismatch before the fix.
The same ref-aware split drives the hermetic installer gate (W061-J): the
version it LOOKS UP comes from the asset name (the filename contract
`install.sh` resolves), while the run-check compares the installed binary
against the `Cargo.toml` version — both derivations are exercised on every
dispatch rehearsal, so the installer contract never first-runs on a real
tag (the sha256sums-job lesson, run 37146021197).
The wiring is pinned by `scripts/pkg_meta_check.py`; a rehearsal that
cannot derive an expectation refuses to run.

The second rehearsal (run 37045410736) caught the windows sidecar producer
writing CRLF (`Out-File`), which makes every `sha256sum -c` consumer read
the trailing CR into the filename and fail — a latent bug the first real
tag release would have hit at the manifest job. The producer now writes LF
(`[System.IO.File]::WriteAllText`), and both in-workflow consumers (smoke
step, SHA256SUMS job) CR-normalize each sidecar before `-c`; the hash
bytes verified are still the exact published ones.

## Installer gate (S5)

`scripts/install.sh` had zero gate coverage — its fail-closed verification
law (sec-r1 / B1-U3) was only ever exercised by production traffic.
`OPERON_INSTALL_ASSET_DIR` gives it a hermetic offline mode (a directory
laid out like a release: assets, sidecars, optional manifest; every
verification law unchanged), and `scripts/install_e2e.sh` exercises the
full contract on every gate: happy path (sidecar + manifest cross-check,
binary + `operon-ls` + `std/` installed), tampered sidecar refusal,
`--verify` without a manifest, `--verify` with the asset unlisted, and the
offline mode's refusal to resolve `latest` without a network.

## Publish checklist (owner/maintainer, per release)

1. Pin every `REPLACE_WITH_COMPANION_SHA256` / `fakeSha256` / empty `hash`
   from the release's `.sha256` companions.
2. Bump the version literals in `packaging/scoop/operon.json`,
   `packaging/aur/PKGBUILD*`, `packaging/homebrew/operon.rb` and
   `packaging/nix/default.nix`. The checker now enforces this (W061-A):
   the W61 guard in `scripts/check_docs_sync.py` fails when any manifest's
   version literal drifts behind the Cargo version — two releases (2.6.0,
   2.7.0) shipped with the drafts still pinned to 2.2.0 before it existed.
3. `makepkg -si` (AUR) / `nix-build` (Nix) / `cargo deb` / `cargo generate-rpm`
   locally; move a channel to `validated` ONLY when a CI job reproduces it.
4. Update this ledger in the same commit as any publish.
