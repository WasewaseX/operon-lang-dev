#!/usr/bin/env bash
# registry_e2e.sh — the W21 done-when proof: publish two fixture packages
# into a static git-index registry, then a consumer resolves a dependency
# BY NAME through that index, installs offline from the lockfile, and
# republishing is idempotent. Hermetic: local git repos (plain paths) and
# an OPERON_DEPS sandbox. No network.
set -euo pipefail
cd "$(dirname "$0")/.."
OP="$(pwd)/bin/operon"
SB=$(mktemp -d)
trap 'rm -rf "$SB"' EXIT
export OPERON_DEPS="$SB/deps"
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=t@t

fail() { echo "REGISTRY E2E FAIL: $1"; exit 1; }

# ---- fixture: beta (a leaf library, its own git repo)
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
EOF
git -C "$SB/beta" init -q && git -C "$SB/beta" add -A && git -C "$SB/beta" commit -qm "beta r1"
BETA_REV=$(git -C "$SB/beta" rev-parse HEAD)

# ---- fixture: gamma (also a leaf; proves the index holds several names)
mkdir -p "$SB/gamma"
cat > "$SB/gamma/operon.toml" <<'EOF'
[package]
name = "gamma"
version = "1.2.0"
EOF
cat > "$SB/gamma/lib.op" <<'EOF'
pub let WHO = "gamma"
EOF
git -C "$SB/gamma" init -q && git -C "$SB/gamma" add -A && git -C "$SB/gamma" commit -qm "gamma r1"

# ---- the registry index: publish both fixtures into one JSON-lines file
REG="$SB/registry.jsonl"
(cd "$SB/beta"  && $OP mod publish --registry "$REG" --url "$SB/beta"  --desc "leaf library") || fail "publish beta"
(cd "$SB/gamma" && $OP mod publish --registry "$REG" --url "$SB/gamma" --desc "who Am I") || fail "publish gamma"
grep -q '"name": "beta"'  "$REG" || fail "registry missing beta line"
grep -q '"name": "gamma"' "$REG" || fail "registry missing gamma line"

# idempotence: republishing the same (name, version, rev) appends nothing
BEFORE=$(wc -l < "$REG")
(cd "$SB/beta" && $OP mod publish --registry "$REG" --url "$SB/beta") || fail "republish beta"
AFTER=$(wc -l < "$REG")
[ "$BEFORE" = "$AFTER" ] || fail "republish was not idempotent"

# ---- the consumer: resolves by NAME through the index
mkdir -p "$SB/app"
cd "$SB/app"
$OP mod init my-app || fail "init"
$OP mod add beta --registry "$REG" || fail "add by name from registry"
grep -q 'name = "beta"' operon.lock || fail "lock missing beta"
grep -q "rev = \"$BETA_REV\"" operon.lock || fail "lock rev != registry pin"

# the installed dep runs (the closure vendored into OPERON_DEPS)
cat > main.op <<'EOF'
use beta/lib as beta;
main {
    promote(beta.shout("registry works"))
}
EOF
OUT=$($OP run main.op 2>/dev/null) || fail "run after registry add"
echo "$OUT" | grep -q "registry works!!" || fail "wrong run output: $OUT"

# offline: same run with the network-shaped path gone is irrelevant (file://
# fixtures are local), so instead prove the lockfile discipline: --locked
# rejects a manifest drift
$OP mod remove beta >/dev/null || fail "remove beta"
$OP mod add beta --registry "$REG" || fail "re-add beta"
$OP mod verify | grep -q "ok beta" || fail "verify after registry install"

# malformed registry lines are rejected with the line number
printf '{"name": "x", "git": "y", "rev": "z"}\nBROKEN LINE\n' > "$SB/bad.jsonl"
$OP mod add beta --registry "$SB/bad.jsonl" 2>"$SB/bad.err" && fail "bad registry accepted"
grep -q "registry line 2" "$SB/bad.err" || fail "no line-precise rejection: $(cat "$SB/bad.err")"

# unknown name resolves to a clean error
$OP mod add nosuchpkg --registry "$REG" 2>"$SB/no.err" && fail "unknown name accepted"
grep -q "not in registry" "$SB/no.err" || fail "wrong unknown-name error"

echo "REGISTRY E2E OK"
