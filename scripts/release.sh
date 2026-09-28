#!/usr/bin/env bash
# release.sh : reproducible source release archives + checksums (B1-U3).
#
# What it makes (under dist/):
#   operon-<version>.tar.gz            git archive of HEAD, tracked files only
#   operon-<version>.tar.gz.sha256     sha256sum-format line for the archive
#   SHA256SUMS                         manifest containing the archive hash
#
# Source-only by construction: git archive packs tracked files, so target/,
# bin/, build/ and dist/ itself can never leak into a release archive.
#
# Usage:
#   release.sh                 build archive + checksums into dist/
#   release.sh --dry-run       print the plan, write nothing
#   release.sh --verify        re-check an existing archive against its
#                              .sha256 file (and dist/SHA256SUMS if present)
#   release.sh --version X     override the version (default: Cargo.toml)
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."

die() { echo "release: error: $*" >&2; exit 1; }

MODE="release"
VERSION_OVERRIDE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --verify) MODE="verify"; shift ;;
    --dry-run) MODE="dry-run"; shift ;;
    --version)
      [ $# -ge 2 ] || die "--version needs a value"
      VERSION_OVERRIDE="$2"; shift 2 ;;
    --version=*)
      VERSION_OVERRIDE="${1#*=}"; shift ;;
    -h|--help)
      sed -n '2,18p' "$0" | sed 's/^#\{1,\} \{0,1\}//'; exit 0 ;;
    *) die "unknown argument '$1' (try --help)" ;;
  esac
done

# ---- version resolution ------------------------------------------------------
read_version_from_cargo() {
  sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -n1
}

if [ -n "$VERSION_OVERRIDE" ]; then
  VERSION="$VERSION_OVERRIDE"
  VERSION_SRC="command line"
else
  VERSION="$(read_version_from_cargo)"
  VERSION_SRC="Cargo.toml"
fi
[ -n "$VERSION" ] || die "could not read package version from Cargo.toml"
# the version lands in paths and tar prefixes; refuse traversal tricks
printf '%s' "$VERSION" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9._+-]*$' \
  || die "invalid version '$VERSION' (allowed: alphanumerics, dot, dash, plus, underscore)"

ARCHIVE="dist/operon-${VERSION}.tar.gz"
SUMFILE="dist/operon-${VERSION}.tar.gz.sha256"
MANIFEST="dist/SHA256SUMS"

# ---- checksum tooling (no deps beyond coreutils; macOS fallback) -------------
if command -v sha256sum >/dev/null 2>&1; then
  sha_of() { sha256sum "$1" | cut -d' ' -f1; }
  check_in() { ( cd "$2" && sha256sum -c "$1" ); }
elif command -v shasum >/dev/null 2>&1; then
  sha_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
  check_in() { ( cd "$2" && shasum -a 256 -c "$1" ); }
else
  die "no sha256sum or shasum found; cannot produce or check checksums"
fi

# ---- tar determinism detection ------------------------------------------------
# GNU tar can pin entry order, mtimes and ownership; BSD tar (macOS) cannot.
# The fallback still yields a valid archive: git archive output is ordered and
# uid/gid-0 already, only the mtime pin differs (commit time instead of epoch).
TAR_NORMALIZE=()
TAR_MODE="git archive output as-is (no GNU tar normalization available)"
TARID="$(tar --version 2>/dev/null || true)"
case "$TARID" in
  *"GNU tar"*)
    TAR_NORMALIZE=(--sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner)
    TAR_MODE="GNU tar normalization (--sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner)"
    ;;
esac

# ---- --verify mode ------------------------------------------------------------
if [ "$MODE" = "verify" ]; then
  [ -f "$ARCHIVE" ] || die "missing $ARCHIVE (run scripts/release.sh without --verify first)"
  [ -f "$SUMFILE" ] || die "missing $SUMFILE"
  echo "==> verifying $ARCHIVE against $SUMFILE"
  check_in "operon-${VERSION}.tar.gz.sha256" dist \
    || die "checksum mismatch: $ARCHIVE does not match $SUMFILE"
  if [ -f "$MANIFEST" ]; then
    # match only the archive line (anchored), never a same-prefix name
    MLINE="$(grep -E "^[0-9a-fA-F]{64}[[:space:]]+operon-${VERSION}\.tar\.gz$" "$MANIFEST" | head -n1 || true)"
    [ -n "$MLINE" ] || die "manifest $MANIFEST does not list operon-${VERSION}.tar.gz"
    MHASH="$(printf '%s\n' "$MLINE" | cut -d' ' -f1)"
    [ "$MHASH" = "$(sha_of "$ARCHIVE")" ] \
      || die "checksum mismatch: $ARCHIVE does not match $MANIFEST"
    echo "==> manifest $MANIFEST ok"
  fi
  echo "release: verify ok for operon-${VERSION}.tar.gz"
  exit 0
fi

# ---- preflight -----------------------------------------------------------------
command -v git >/dev/null 2>&1 || die "git not found"
git rev-parse --verify HEAD >/dev/null 2>&1 || die "not inside a git repository (or no commits)"

if [ "$MODE" = "dry-run" ]; then
  echo "release: dry-run plan (nothing written)"
  echo "  version:  ${VERSION} (from ${VERSION_SRC})"
  echo "  source:   git archive --prefix=operon-${VERSION}/ HEAD (tracked files only)"
  echo "  tar:      ${TAR_MODE}"
  echo "  archive:  ${ARCHIVE} [would create, gzip -n]"
  echo "  checksum: ${SUMFILE} [would create, sha256sum format]"
  echo "  manifest: ${MANIFEST} [would create/overwrite]"
  echo "  hash:     <sha256 computed on the real run>"
  if [ -f "$ARCHIVE" ]; then
    echo "  note:     ${ARCHIVE} already exists and will be overwritten"
  fi
  exit 0
fi

# git archive packs HEAD, so uncommitted changes never leak, but say so loudly
if [ -n "$(git status --porcelain)" ]; then
  echo "release: warning: working tree is dirty; packing HEAD (version from ${VERSION_SRC}), not the dirty files" >&2
fi

# ---- build the archive ----------------------------------------------------------
mkdir -p dist
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

git archive --format=tar --prefix="operon-${VERSION}/" HEAD > "$STAGE/src.tar"

if [ "${#TAR_NORMALIZE[@]}" -gt 0 ]; then
  # re-tar through GNU tar with pinned metadata; gzip -n drops name+mtime
  # from the gzip header so the bytes are reproducible for a given commit
  mkdir "$STAGE/root"
  tar -xf "$STAGE/src.tar" -C "$STAGE/root"
  ( cd "$STAGE/root" && tar "${TAR_NORMALIZE[@]}" -cf - "operon-${VERSION}" ) | gzip -n > "$ARCHIVE"
else
  gzip -n < "$STAGE/src.tar" > "$ARCHIVE"
fi

# ---- checksums + summary ---------------------------------------------------------
HASH="$(sha_of "$ARCHIVE")"
SIZE="$(wc -c < "$ARCHIVE" | tr -d ' ')"
printf '%s  %s\n' "$HASH" "operon-${VERSION}.tar.gz" > "$SUMFILE"
printf '%s  %s\n' "$HASH" "operon-${VERSION}.tar.gz" > "$MANIFEST"

echo "release: operon-${VERSION}.tar.gz (${SIZE} bytes) sha256 ${HASH:0:16}..."
echo "release: wrote $SUMFILE and $MANIFEST (fill packaging/homebrew/operon.rb sha256 from these)"
