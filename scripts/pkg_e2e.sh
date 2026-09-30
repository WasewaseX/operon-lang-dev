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

echo "pkg_e2e: $pass passed, $fail failed"
[ "$fail" = "0" ] && echo "PKG E2E GREEN"
rm -rf "$WORK"
exit $([ "$fail" = "0" ] && echo 0 || echo 1)
