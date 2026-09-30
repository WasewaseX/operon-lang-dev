#!/usr/bin/env bash
# pkg_e2e.sh - end-to-end package-system gate (ai/ecosystem lane, W19).
#
# Exercises the full developer surface against BOTH registry transports:
#   1. a seeded local directory registry   (OPERON_REGISTRY=<dir>)
#   2. the hosted-shape HTTP registry API  (OPERON_REGISTRY=http://127.0.0.1:<port>)
#
# Covered: new / add (explicit + caret-of-latest) / run against installed
# packages / lock reproducibility (byte-identical re-resolution) / frozen
# installs / search / publish (dir + HTTP, immutability, auth) / remove /
# update / transitive dependencies / sha256 pinning.
#
# Usage: tests/package/pkg_e2e.sh [path-to-operon-binary]
set -u

OP="${1:-$(dirname "$0")/../../target/release/operon}"
OP="$(cd "$(dirname "$OP")" && pwd)/$(basename "$OP")"
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
PASS=0
FAIL=0
REGISTRY_PORT="${REGISTRY_PORT:-8791}"
HTTP_PID=""

cleanup() {
  if [ -n "$HTTP_PID" ]; then kill "$HTTP_PID" 2>/dev/null; fi
  rm -rf "$WORK"
}
trap cleanup EXIT

ok()   { PASS=$((PASS + 1)); echo "  ok: $1"; }
bad()  { FAIL=$((FAIL + 1)); echo "  FAIL: $1"; }
check() { # check <desc> <condition-result>
  if [ "$2" = "0" ]; then ok "$1"; else bad "$1"; fi
}

die() { echo "fatal: $*" >&2; exit 2; }
[ -x "$OP" ] || die "operon binary not found at $OP (build first: scripts/build.sh)"

echo "== operon package e2e (binary: $OP) =="
echo "== workdir: $WORK =="

# --- 0. seed the directory registry from packaging/packages -------------------
echo "[0] seed directory registry"
python3 "$REPO/scripts/pkg_seed_registry.py" "$WORK/registry" >/dev/null || die "seed failed"
[ -f "$WORK/registry/index/http.json" ]; check "registry seeded (http index exists)" $?
DIRREG="$WORK/registry"
export OPERON_REGISTRY="$DIRREG"

# --- 1. operon new ------------------------------------------------------------
echo "[1] operon new"
cd "$WORK"
"$OP" new demo >/dev/null 2>&1; check "new: project scaffolded" $?
[ -f demo/operon.toml ] && [ -f demo/src/main.op ] && [ -f demo/tests/main_test.op ]
check "new: manifest + entry + tests present" $?
"$OP" new demo 2>/dev/null; [ "$?" != "0" ]; check "new: refuses non-empty dir" $?
"$OP" new libdemo lib >/dev/null 2>&1; check "new: lib template scaffolds" $?
(cd libdemo && "$OP" test >/dev/null 2>&1); check "new: lib template tests pass on fresh checkout" $?

# --- 2. operon add + operon run -----------------------------------------------
echo "[2] add + run against installed package"
cd "$WORK/demo"
"$OP" add http >/dev/null 2>&1; check "add http" $?
[ -f operon.lock ]; check "add wrote operon.lock" $?
[ -f operon_modules/http/http.op ]; check "add installed the entry module" $?
printf 'use http\n\ngene main {\n    print("escaped: " + http.escape("x y/z"));\n    return 0;\n}\n' > src/main.op
"$OP" run > "$WORK/run.out" 2>"$WORK/run.err"
check "project run exits 0" $?
grep -q "escaped: x%20y%2Fz" "$WORK/run.out"; check "run output uses the installed package" $?
if grep -q "fallback" "$WORK/run.err"; then bad "run is note-clean"; else ok "run is note-clean"; fi

# --- 3. lock reproducibility ---------------------------------------------------
echo "[3] lock reproducibility"
cp operon.lock "$WORK/lock1"
rm -rf operon_modules
"$OP" update >/dev/null 2>&1; check "update re-resolves + reinstalls" $?
cmp -s operon.lock "$WORK/lock1"; check "re-resolution is byte-identical (deterministic)" $?
rm -rf operon_modules
"$OP" run >/dev/null 2>&1; check "run auto-installs from a fresh lock (lockfile-first)" $?
sha_before="$(grep sha256 operon.lock)"
# frozen: adding an already-satisfied dep must not bump anything
"$OP" add "http@^1.0" >/dev/null 2>&1
sha_after="$(grep sha256 operon.lock)"
[ "$sha_before" = "$sha_after" ]; check "add of a satisfied dep keeps the lock frozen" $?

# --- 4. transitive dependencies -------------------------------------------------
echo "[4] transitive deps"
cd "$WORK/demo"
"$OP" add web >/dev/null 2>&1
grep -q 'name = "http"' operon.lock; check "web pulls http transitively into the lock" $?
[ -f operon_modules/web/web.op ]; check "web installed" $?

# --- 5. search -------------------------------------------------------------------
echo "[5] search"
"$OP" search json > "$WORK/search.out" 2>&1
grep -q "json" "$WORK/search.out"; check "search finds json" $?
"$OP" search "sql" > "$WORK/search2.out" 2>&1
grep -q "postgres" "$WORK/search2.out"; check "search matches description text" $?

# --- 6. publish round-trip (directory registry) ----------------------------------
echo "[6] publish -> add round-trip (dir registry)"
cd "$WORK/libdemo"
"$OP" publish -o "$WORK/outbox.opkg" >/dev/null 2>&1; check "publish outbox writes an artifact" $?
"$OP" publish >/dev/null 2>&1; check "publish to dir registry" $?
"$OP" publish >/dev/null 2>&1; [ "$?" != "0" ]; check "publish rejects duplicate versions (immutability)" $?
cd "$WORK/demo"
"$OP" add "libdemo@^0.1" >/dev/null 2>&1; check "add of a freshly published package" $?
printf 'use libdemo\n\ngene main {\n    print("lib says: " + libdemo.hello("world"));\n    return 0;\n}\n' > src/main.op
"$OP" run > "$WORK/rt.out" 2>&1
grep -q "lib says: hello, world" "$WORK/rt.out"; check "published package imports and runs" $?

# --- 7. remove --------------------------------------------------------------------
echo "[7] remove"
cd "$WORK/demo"
"$OP" remove libdemo >/dev/null 2>&1; check "remove libdemo" $?
[ ! -d operon_modules/libdemo ]; check "remove pruned the module dir" $?
"$OP" remove nosuchpkg 2>/dev/null; [ "$?" != "0" ]; check "remove of a non-dep fails loudly" $?

# --- 8. HTTP registry (the hosted shape) -------------------------------------------
echo "[8] HTTP registry API"
cd "$WORK"
# explicit env: the sandbox may carry its own PORT/DATABASE_URL; the test
# registry must bind OUR port and use OUR sqlite file, never the host's.
env -u DATABASE_URL PORT="$REGISTRY_PORT" OPERON_REGISTRY_DB="$WORK/httpreg.db" \
  OPERON_TOKENS="testtoken" python3 "$REPO/packaging/registry/app.py" &
HTTP_PID=$!
for i in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$REGISTRY_PORT/healthz" >/dev/null 2>&1 && break
  sleep 0.2
done
curl -fsS "http://127.0.0.1:$REGISTRY_PORT/healthz" >/dev/null 2>&1
check "registry /healthz" $?
# publish the seed packages to the (empty) HTTP registry through the CLI —
# this IS the hosted publish workflow
for p in http json postgres web; do
  (cd "$REPO/packaging/packages/$p" && \
   OPERON_REGISTRY="http://127.0.0.1:$REGISTRY_PORT" OPERON_TOKEN="testtoken" \
   "$OP" publish >/dev/null 2>&1) || true
done
curl -fsS "http://127.0.0.1:$REGISTRY_PORT/api/packages/http" | grep -q '"http"'
check "HTTP publish seeded the API (package readable)" $?
OPERON_REGISTRY="http://127.0.0.1:$REGISTRY_PORT" "$OP" search http > "$WORK/hsearch.out" 2>&1
grep -q "http" "$WORK/hsearch.out"; check "HTTP search works from the CLI" $?
rm -rf htdemo && "$OP" new htdemo >/dev/null 2>&1 && cd htdemo
OPERON_REGISTRY="http://127.0.0.1:$REGISTRY_PORT" "$OP" add http >/dev/null 2>&1
check "HTTP add resolves + installs" $?
printf 'use http\n\ngene main {\n    print("ok: " + http.escape("a b"));\n    return 0;\n}\n' > src/main.op
OPERON_REGISTRY="http://127.0.0.1:$REGISTRY_PORT" "$OP" run > "$WORK/ht.out" 2>&1
grep -q "ok: a%20b" "$WORK/ht.out"; check "HTTP-installed package runs" $?
# publish over HTTP: auth required
OPERON_REGISTRY="http://127.0.0.1:$REGISTRY_PORT" "$OP" publish 2>/dev/null
[ "$?" != "0" ]; check "HTTP publish without a token is rejected" $?
OPERON_REGISTRY="http://127.0.0.1:$REGISTRY_PORT" OPERON_TOKEN="testtoken" "$OP" publish >/dev/null 2>&1
check "HTTP publish with a token" $?
# download the published artifact back through HTTP
(cd "$WORK/libdemo" && \
 OPERON_REGISTRY="http://127.0.0.1:$REGISTRY_PORT" OPERON_TOKEN="testtoken" \
 "$OP" publish >/dev/null 2>&1)
cd "$WORK" && rm -rf htdemo2 && "$OP" new htdemo2 >/dev/null 2>&1 && cd htdemo2
OPERON_REGISTRY="http://127.0.0.1:$REGISTRY_PORT" "$OP" add libdemo >/dev/null 2>&1
check "HTTP add of the just-published package (round-trip)" $?

# --- 9. sha256 pinning rejects corrupted artifacts ---------------------------------
echo "[9] content pinning"
cd "$WORK/demo"
export OPERON_REGISTRY="$DIRREG"
"$OP" update >/dev/null 2>&1
CORRUPT="$DIRREG/artifacts/web/1.0.0.opkg"
cp "$CORRUPT" "$WORK/web.bak"
printf 'X' >> "$CORRUPT"
rm -rf operon_modules
"$OP" update >/dev/null 2>&1; [ "$?" != "0" ]; check "corrupted artifact fails sha256 verification" $?
mv "$WORK/web.bak" "$CORRUPT"
"$OP" update >/dev/null 2>&1; check "restored artifact installs again" $?

echo ""
echo "== pkg e2e: $PASS passed, $FAIL failed =="
[ "$FAIL" = "0" ] || exit 1
exit 0
