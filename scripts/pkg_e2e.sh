#!/usr/bin/env bash
# pkg_e2e.sh — the W19/W20/W23 done-when proof: a two-package dependency
# tree builds and runs from a lockfile, offline, with --locked drift
# detection and checksum verification. Hermetic: local git repos (file://)
# and an OPERON_DEPS sandbox — no network, no ~/.operon pollution.
set -euo pipefail
cd "$(dirname "$0")/.."
OP="$(pwd)/bin/operon"
SB=$(mktemp -d)
trap 'rm -rf "$SB"' EXIT
export OPERON_DEPS="$SB/deps"
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=t@t

fail() { echo "E2E FAIL: $1"; exit 1; }

# ---- fixture package: beta (a leaf library)
mkdir -p "$SB/beta"
cat > "$SB/beta/operon.toml" <<'EOF'
[package]
name = "beta"
version = "0.1.0"
EOF
cat > "$SB/beta/lib.op" <<'EOF'
pub gene shout(s) {
    return s + "!!"
}
gene internal_norm(s) {
    return s
}
pub let STAMP = "beta-1.0"
EOF
git -C "$SB/beta" init -q && git -C "$SB/beta" add -A && git -C "$SB/beta" commit -qm "beta r1"
REV1=$(git -C "$SB/beta" rev-parse HEAD)

# ---- fixture package: alpha (depends on beta — the transitive edge)
mkdir -p "$SB/alpha"
cat > "$SB/alpha/operon.toml" <<'EOF'
[package]
name = "alpha"
version = "0.2.0"

[deps]
beta = { git = "FILE_URL" }
EOF
sed -i "s|FILE_URL|$SB/beta|" "$SB/alpha/operon.toml"
cat > "$SB/alpha/lib.op" <<'EOF'
use beta/lib as beta;
pub gene greet(s) {
    return "hello " + beta.shout(s)
}
EOF
git -C "$SB/alpha" init -q && git -C "$SB/alpha" add -A && git -C "$SB/alpha" commit -qm "alpha r1"

# ---- the app package
mkdir -p "$SB/app"
cd "$SB/app"
$OP mod init my-app || fail "init"
grep -q 'name = "my-app"' operon.toml || fail "manifest missing name"

$OP mod add "$SB/alpha" || fail "add alpha"
grep -q 'name = "alpha"' operon.lock || fail "lock missing alpha"
grep -q 'name = "beta"' operon.lock || fail "lock missing TRANSITIVE beta"
grep -q "sha256:" operon.lock || fail "lock entries lack checksums"

# tree shows the two-package shape
$OP mod tree | grep -q "alpha" || fail "tree missing alpha"
$OP mod tree | grep -q "beta" || fail "tree missing beta"

# the pinned rev matches the fixture's HEAD (deterministic resolution)
grep -q "rev = \"$REV1\"" operon.lock || fail "lock rev != fixture HEAD (transitive resolve not pinned)"

# ---- checksum honesty: verify catches tampering
$OP mod verify >/dev/null || fail "verify failed on a fresh cache"
TAMPER=$(find "$OPERON_DEPS" -maxdepth 2 -name 'lib.op' | head -1)
echo "tampered" >> "$TAMPER"
$OP mod verify >/dev/null 2>&1 && fail "verify missed tampering"
rm -rf "$OPERON_DEPS"

# ---- W23 workflow: a fresh checkout + a warm-less cache reproduces from
# the LOCKFILE ONLY (operon mod install re-vendors from the pinned revs;
# file:// remotes = offline). Then run.
$OP mod install || fail "install from lockfile"
$OP mod verify >/dev/null || fail "verify after install"
cat > app.op <<'EOF'
use alpha/lib as alpha;
print(alpha.greet("world"))
print(beta.STAMP)
EOF
$OP run app.op > out.txt 2>/dev/null || fail "run from lockfile"
grep -q "hello world!!" out.txt || fail "wrong output: $(cat out.txt)"
grep -q "beta-1.0" out.txt || fail "transitive data export missing"

# ---- --locked: clean tree passes, drift fails
$OP run app.op --locked >/dev/null 2>&1 || fail "--locked failed on a clean tree"
$OP mod add "$SB/beta" --as extra >/dev/null 2>&1 || true
# adding a dep rewrites the lock (consistent), so simulate drift by hand:
sed -i 's|name = "extra"|name = "ghost"|' operon.lock
$OP run app.op --locked >/dev/null 2>&1 && fail "--locked missed stale lock line"

echo "E2E OK: two-package tree resolved, locked, run offline, verified"
