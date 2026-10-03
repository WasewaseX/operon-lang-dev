#!/usr/bin/env bash
# pkg_e2e.sh — W20/W21-r1 package-system end-to-end gate.
# Exercises the full developer workflow against a THROWAWAY registry home
# and deps cache (OPERON_REGISTRY_HOME / OPERON_DEPS / OPERON_HOME), so the
# seed materialization path runs from zero exactly like a fresh install:
#
#   operon new myapp          scaffold
#   operon test               smoke proof green
#   operon add http           seed registry chain (no flags, offline)
#   use http in a program     vendored module resolution
#   operon add web            TRANSITIVE dep (web -> http) through the closure
#   vendored package tests    http/json/web proof frames from the cache
#   operon tree/verify        lockfile tree + checksum verification
#   operon install            offline materialization from operon.lock
#   lock-aware use hint       "pinned but not installed" message
#   operon remove             manifest + lock stay in sync
#   operon registry init/serve + curl client    read-only HTTP surface
#   remote dir-source refusal security probe
#   fresh caches -> same lock  byte-identical operon.lock from two fully
#                              independent HOMEs + deps caches (the seed
#                              registry re-materializes under each HOME)
#   cold offline install       wipe the deps cache; install re-materializes
#                              every dep from the lock pins; verify + run
#   lock is the contract       manifest rev drift = --locked hard failure
set -uo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-./bin/operon}"
# absolute: the gate cds into throwaway project dirs, a relative $BIN dies
case "$BIN" in /*) ;; *) BIN="$PWD/${BIN#./}" ;; esac
WORK=$(mktemp -d)
export OPERON_REGISTRY_HOME="$WORK/registry-home"
export OPERON_DEPS="$WORK/deps"
export HOME="$WORK/home"
mkdir -p "$HOME" "$WORK/proj"
pass=0; fail=0
ok()   { pass=$((pass+1)); echo "ok    $1"; }
bad()  { fail=$((fail+1)); echo "FAIL  $1"; [ "${VERBOSE:-0}" = "1" ] && [ -n "${2:-}" ] && echo "      $2"; return 0; }
check(){ if [ "$1" = "0" ]; then ok "$2"; else bad "$2" "${3:-}"; fi; }

# ---- 1. scaffold
(cd "$WORK/proj" && "$BIN" new myapp >/dev/null 2>&1)
check $? "operon new myapp"
[ -f "$WORK/proj/myapp/operon.toml" ] && [ -f "$WORK/proj/myapp/src/main.op" ] && [ -f "$WORK/proj/myapp/tests/smoke.op" ]
check $? "scaffold layout (operon.toml, src/main.op, tests/smoke.op)"
(cd "$WORK/proj/myapp" && "$BIN" test >/dev/null 2>&1)
check $? "scaffold smoke test green"
(cd "$WORK/proj/myapp" && "$BIN" run src/main.op 2>/dev/null | grep -q "hello, operon")
check $? "scaffold program runs"

# ---- 2. seed registry chain: add-by-name with zero flags
(cd "$WORK/proj/myapp" && "$BIN" add http 2>&1 | grep -F "via registry $OPERON_REGISTRY_HOME/index.jsonl" >/dev/null)
check $? "operon add http resolves via the materialized seed registry"
[ -f "$OPERON_REGISTRY_HOME/index.jsonl" ] && [ -d "$OPERON_REGISTRY_HOME/packages/http" ]
check $? "seed registry materialized (index + packages)"
grep -q 'git = "registry:http"' "$WORK/proj/myapp/operon.lock"
check $? "lock records the registry source (registry:http)"

# ---- 3. vendored use
cat > "$WORK/proj/myapp/src/app.op" <<'EOF'
use http
gene main() {
    let req = http.http_request_parse("GET /s?q=x HTTP/1.1\nHost: h\n\n")
    promote("q=" + req.query_map["q"])
}
EOF
(cd "$WORK/proj/myapp" && "$BIN" run src/app.op 2>/dev/null | grep -q "q=x")
check $? "vendored use-http resolves and runs"

# ---- 4. transitive dep: web -> http
(cd "$WORK/proj/myapp" && "$BIN" add web >/dev/null 2>&1)
check $? "operon add web"
grep -q 'name = "http"' "$WORK/proj/myapp/operon.lock" && grep -q 'name = "web"' "$WORK/proj/myapp/operon.lock"
check $? "web's http dep joined the lock closure"
(cd "$WORK/proj/myapp" && "$BIN" tree 2>/dev/null | grep -q "web content registry:web")
check $? "operon tree renders the resolved tree"

# ---- 5. vendored package proof frames
WEBREV=$("$BIN" run --version >/dev/null 2>&1; grep -A2 'name = "web"' "$WORK/proj/myapp/operon.lock" | grep '^rev' | cut -d'"' -f2 | cut -c1-12)
HTTPREV=$(grep -A2 'name = "http"' "$WORK/proj/myapp/operon.lock" | grep '^rev' | cut -d'"' -f2 | cut -c1-12)
(cd "$WORK/proj/myapp" && "$BIN" test "$OPERON_DEPS/web-$WEBREV/test_web.op" >/dev/null 2>&1)
check $? "vendored web proof frame green"
(cd "$WORK/proj/myapp" && "$BIN" test "$OPERON_DEPS/http-$HTTPREV/test_http.op" >/dev/null 2>&1)
check $? "vendored http proof frame green"

# ---- 6. verify + install + the lock-aware hint
(cd "$WORK/proj/myapp" && "$BIN" verify >/dev/null 2>&1)
check $? "operon verify: every vendored tree matches its checksum"
rm -rf "$OPERON_DEPS/http-$HTTPREV"
HINT=$(cd "$WORK/proj/myapp" && "$BIN" run src/app.op 2>&1 | grep -c "pinned in operon.lock but not installed")
[ "$HINT" -ge 1 ]
check $? "missing-dep use error carries the install hint"
(cd "$WORK/proj/myapp" && "$BIN" install >/dev/null 2>&1)
check $? "operon install materializes from the lockfile offline"
(cd "$WORK/proj/myapp" && "$BIN" run src/app.op 2>/dev/null | grep -q "q=x")
check $? "program runs again after install"

# ---- 7. remove keeps manifest + lock in sync
(cd "$WORK/proj/myapp" && "$BIN" remove web >/dev/null 2>&1)
check $? "operon remove web"
grep -q 'name = "web"' "$WORK/proj/myapp/operon.lock" && fail=1 || true
if grep -q 'name = "web"' "$WORK/proj/myapp/operon.lock"; then bad "remove purges the lock line" ; else ok "remove purges the lock line"; fi

# ---- 8. registry init/serve + client + security refusal
"$BIN" registry init "$WORK/reg" >/dev/null 2>&1
check $? "operon registry init"
"$BIN" registry serve "$WORK/reg" --port "$((RANDOM % 20000 + 30000))" >/dev/null 2>&1 &
SRV=$!
sleep 0.5
kill $SRV 2>/dev/null
PORT=7731
"$BIN" registry serve "$WORK/reg" --port $PORT >/dev/null 2>&1 &
SRV=$!
sleep 0.5
[ "$(curl -s http://127.0.0.1:$PORT/health)" = "ok" ]
check $? "registry serve /health"
curl -s http://127.0.0.1:$PORT/index.jsonl | head -1 | grep -q "^#"
check $? "registry serve /index.jsonl"
[ "$(curl -s --path-as-is http://127.0.0.1:$PORT/pkg/../index.jsonl)" = "not found" ]
check $? "path traversal refused (dot segment)"
[ "$(curl -s --path-as-is http://127.0.0.1:$PORT/pkg/%2e%2e/index.jsonl)" = "not found" ]
check $? "path traversal refused (percent-encoded)"
[ "$(curl -s -X POST http://127.0.0.1:$PORT/index.jsonl)" = "method not allowed" ]
check $? "non-GET refused (405 body)"

# serve the SEED registry over HTTP; its dir-sourced entries must be
# REFUSED for remote clients (the deny-by-default rule for remote index)
"$BIN" registry serve "$OPERON_REGISTRY_HOME" --port $((PORT + 1)) >/dev/null 2>&1 &
SRV2=$!
sleep 0.5
OUT=$(cd "$WORK/proj/myapp" && OPERON_REGISTRY=http://127.0.0.1:$((PORT + 1))/index.jsonl "$BIN" add json 2>&1 | tail -1)
echo "$OUT" | grep -q "remote but its entry for"
check $? "remote registry with dir-sourced entries refused (security)"
kill $SRV $SRV2 2>/dev/null

# ---- 9. REPRODUCIBILITY: byte-identical lockfile across completely fresh caches
# Two fully independent states: each sandbox gets its own HOME (the seed
# registry re-materializes under ~/.operon/registry), its own OPERON_DEPS,
# and no OPERON_REGISTRY_HOME. The same `new demo` + `add web` (which pulls
# transitive http) must end at the SAME operon.lock BYTES — W23's contract:
# same inputs, same revs, same checksums, zero locality in the lock.
iso_env() { env -u OPERON_REGISTRY_HOME HOME="$1" OPERON_DEPS="$2" "${@:3}"; }
fresh_demo() { # $1 = sandbox root; prints the lockfile sha256 on success
  ( cd "$1/proj" \
    && iso_env "$1/home" "$1/deps" "$BIN" new demo >/dev/null 2>&1 \
    && cd demo \
    && iso_env "$1/home" "$1/deps" "$BIN" add web >/dev/null 2>&1 \
    && sha256sum operon.lock | cut -d' ' -f1 )
}
LOCKA="$WORK/lockA"; LOCKB="$WORK/lockB"
mkdir -p "$LOCKA/proj" "$LOCKB/proj"
LOCKA_SHA=$(fresh_demo "$LOCKA"); LOCKB_SHA=$(fresh_demo "$LOCKB")
[ -n "$LOCKA_SHA" ] && [ -n "$LOCKB_SHA" ]
check $? "both fresh sandboxes resolved demo + web (locks captured)"
[ -f "$LOCKA/home/.operon/registry/index.jsonl" ] && [ -f "$LOCKB/home/.operon/registry/index.jsonl" ]
check $? "seed registry re-materialized under EACH fresh HOME"
if [ "$LOCKA_SHA" = "$LOCKB_SHA" ]; then
  ok "operon.lock byte-identical across completely fresh caches (sha ${LOCKA_SHA:0:16})"
else
  bad "operon.lock byte-identical across completely fresh caches" "sha A=$LOCKA_SHA sha B=$LOCKB_SHA"
  diff "$LOCKA/proj/demo/operon.lock" "$LOCKB/proj/demo/operon.lock" | head -20 || true
fi

# ---- 10. OFFLINE INSTALL FROM LOCK ONLY
# The lockfile is the contract: wipe the deps cache ENTIRELY, then `operon
# install` must re-materialize every dep from the lock pins alone
# (registry:NAME + rev, resolved against the seed registry, no network),
# verify must go green, and the app must run — while the lock bytes stay put.
DEMO="$LOCKA/proj/demo"
cat > "$DEMO/src/app.op" <<'EOF'
use web
gene main() {
    let router = web.web_router_new()
    let router = web.web_route_add(router, "GET", "/greet/:name", 0)
    let hit = web.web_match(router, "GET", "/greet/lock")
    promote("matched=" + hit.params["name"])
}
EOF
(cd "$DEMO" && iso_env "$LOCKA/home" "$LOCKA/deps" "$BIN" run src/app.op 2>/dev/null | grep -q "matched=lock")
check $? "app main runs against the vendored web dep (pre-install)"
rm -rf "$LOCKA/deps"
(cd "$DEMO" && iso_env "$LOCKA/home" "$LOCKA/deps" "$BIN" install >/dev/null 2>&1)
check $? "operon install re-materializes from lockfile pins (cold cache, offline)"
LWEBREV=$(grep -A2 'name = "web"' "$DEMO/operon.lock" | grep '^rev' | cut -d'"' -f2 | cut -c1-12)
LHTTPREV=$(grep -A2 'name = "http"' "$DEMO/operon.lock" | grep '^rev' | cut -d'"' -f2 | cut -c1-12)
[ -d "$LOCKA/deps/web-$LWEBREV" ] && [ -d "$LOCKA/deps/http-$LHTTPREV" ]
check $? "cache dirs re-materialized at the locked revs (web+$LWEBREV, http+$LHTTPREV)"
(cd "$DEMO" && iso_env "$LOCKA/home" "$LOCKA/deps" "$BIN" verify >/dev/null 2>&1)
check $? "operon verify green after the cold install"
[ "$(sha256sum "$DEMO/operon.lock" | cut -d' ' -f1)" = "$LOCKA_SHA" ]
check $? "install did not move the lock bytes"
(cd "$DEMO" && iso_env "$LOCKA/home" "$LOCKA/deps" "$BIN" run src/app.op 2>/dev/null | grep -q "matched=lock")
check $? "app main runs after the cold offline install"

# ---- 11. LOCK IS THE CONTRACT: manifest drift is a hard --locked failure
# Hand-edit operon.toml to drift from the lock (bogus rev), then use the
# real --locked check (wired to pkg::check_locked_manifest via `operon run
# --locked`): it must refuse to execute with a rev-drift error. Restore.
cp "$DEMO/operon.toml" "$DEMO/operon.toml.keep"
sed -i 's/rev = "content-[^"]*"/rev = "content-0000000000000000"/' "$DEMO/operon.toml"
(cd "$DEMO" && iso_env "$LOCKA/home" "$LOCKA/deps" "$BIN" run --locked src/app.op >/dev/null 2>"$WORK/drift.err")
DRIFT_RC=$?
[ "$DRIFT_RC" != "0" ] && grep -q "rev drift" "$WORK/drift.err"
check $? "--locked fails on manifest/lock rev drift (rc=$DRIFT_RC)" "$(cat "$WORK/drift.err")"
mv "$DEMO/operon.toml.keep" "$DEMO/operon.toml"
(cd "$DEMO" && iso_env "$LOCKA/home" "$LOCKA/deps" "$BIN" run --locked src/app.op 2>/dev/null | grep -q "matched=lock")
check $? "restored manifest passes --locked again"

# ---- 12. SEMANTIC VERSION REQUIREMENTS (item 4, ai/ecosystem-r3)
# A multi-version registry built in the sandbox: same package name, four
# index lines over one dir-sourced package tree. The dir rev IS the content
# checksum, so first an unconditional add resolves it (last-wins = 1.0.0)
# and the TRUE rev lands in the lock; the index lines are then patched to
# that truth — exactly what `operon publish` would have written. After that:
# the requirement must pick the HIGHEST satisfying line, refuse
# unsatisfiable requirements honestly, record req+version in manifest and
# lock, and make --locked catch version drift.
SEM="$WORK/semver"; mkdir -p "$SEM/reg/packages/beta" "$SEM/proj"
printf '[package]\nname = "beta"\nversion = "0.1.0"\ndescription = "test pkg"\nentry = "beta.op"\n' > "$SEM/reg/packages/beta/operon.toml"
printf 'value = 1;\n' > "$SEM/reg/packages/beta/beta.op"
for V in 0.1.0 0.1.5 0.2.0 1.0.0; do
  printf '{"name": "beta", "version": "%s", "git": "https://example.com/beta.git", "dir": "%s/reg/packages/beta", "rev": "placeholder-%s", "sha256": "", "description": "test pkg"}\n' "$V" "$SEM" "$V" >> "$SEM/reg/index.jsonl"
done
(cd "$SEM/proj" && iso_env "$SEM/home" "$SEM/deps" "$BIN" new reqapp >/dev/null 2>&1 && cd reqapp \
  && iso_env "$SEM/home" "$SEM/deps" "$BIN" add beta --registry "$SEM/reg/index.jsonl" >/dev/null 2>&1)
TRUE_REV=$(grep '^rev' "$SEM/proj/reqapp/operon.lock" | cut -d'"' -f2)
sed -i "s/\"rev\": \"placeholder-[^\"]*\"/\"rev\": \"$TRUE_REV\"/g" "$SEM/reg/index.jsonl"
(cd "$SEM/proj/reqapp" && iso_env "$SEM/home" "$SEM/deps" "$BIN" remove beta >/dev/null 2>&1)
# capture (not pipe): a grep -q on a live pipe exits early and can SIGPIPE
# the producer — the captured form tests the output, not the timing
ADD_OUT=$(cd "$SEM/proj/reqapp" && iso_env "$SEM/home" "$SEM/deps" "$BIN" add "beta@^0.1" --registry "$SEM/reg/index.jsonl" 2>&1); ADD_RC=$?
[ "$ADD_RC" = "0" ] && printf '%s\n' "$ADD_OUT" | grep -q "resolved 'beta' 0.1.5"
check $? "add beta@^0.1 picks the HIGHEST satisfying version (0.1.5)"
grep -q 'version = "\^0.1"' "$SEM/proj/reqapp/operon.toml"
check $? "manifest records the requirement"
grep -q '^version = "0.1.5"' "$SEM/proj/reqapp/operon.lock" && grep -q '^req = "\^0.1"' "$SEM/proj/reqapp/operon.lock"
check $? "lock records resolved version + req"
(cd "$SEM/proj/reqapp" && iso_env "$SEM/home" "$SEM/deps" "$BIN" run --locked src/main.op >/dev/null 2>&1)
check $? "program runs with the req-pinned dep"
# --locked catches version drift: lock says 0.1.5, hand-bump past the req
cp "$SEM/proj/reqapp/operon.lock" "$SEM/proj/reqapp/operon.lock.keep"
sed -i 's/^version = "0.1.5"/version = "0.2.0"/' "$SEM/proj/reqapp/operon.lock"
(cd "$SEM/proj/reqapp" && iso_env "$SEM/home" "$SEM/deps" "$BIN" run --locked src/main.op >/dev/null 2>"$SEM/vdrift.err")
[ "$?" != "0" ] && grep -q "does not satisfy" "$SEM/vdrift.err"
check $? "--locked fails when the lock version leaves the requirement"
mv "$SEM/proj/reqapp/operon.lock.keep" "$SEM/proj/reqapp/operon.lock"
# unsatisfiable requirement = honest error listing availability
(cd "$SEM/proj/reqapp" && iso_env "$SEM/home" "$SEM/deps" "$BIN" add "json@^9.0" --registry "$SEM/reg/index.jsonl" >/dev/null 2>"$SEM/unsat.err")
[ "$?" != "0" ] && grep -q "satisfies" "$SEM/unsat.err"
check $? "unsatisfiable requirement fails loudly (error names it)"
# malformed requirement = loud parse error, never a silent URL add
(cd "$SEM/proj/reqapp" && iso_env "$SEM/home" "$SEM/deps" "$BIN" add "beta@^abc" --registry "$SEM/reg/index.jsonl" >/dev/null 2>"$SEM/badreq.err")
[ "$?" != "0" ] && grep -q "version requirement" "$SEM/badreq.err"
check $? "malformed requirement fails loudly"
# no-req add keeps last-wins (zero behavior change for existing projects)
(cd "$SEM/proj/reqapp" && iso_env "$SEM/home" "$SEM/deps" "$BIN" remove beta >/dev/null 2>&1)
ADD_OUT=$(cd "$SEM/proj/reqapp" && iso_env "$SEM/home" "$SEM/deps" "$BIN" add beta --registry "$SEM/reg/index.jsonl" 2>&1); ADD_RC=$?
[ "$ADD_RC" = "0" ] && printf '%s\n' "$ADD_OUT" | grep -q "resolved 'beta' 1.0.0"
check $? "add without @req keeps the last-wins rule (1.0.0)"

echo "pkg_e2e: $pass passed, $fail failed"
[ "$fail" = "0" ] && echo "PKG E2E GREEN"
rm -rf "$WORK"
exit $([ "$fail" = "0" ] && echo 0 || echo 1)
