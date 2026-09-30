#!/usr/bin/env bash
# pkg_hosted_e2e.sh — W19 items 5/7/10 (ai/ecosystem-r2): the HOSTED
# registry tier, end to end with the stock CLI and zero compiler changes.
#
#   Operon CLI -> Registry API (packaging/registry/app.py) -> SQLite/PG
#                                                       -> git/source artifacts
#
# Proven here, against the REAL service and the REAL client:
#   - /healthz, /index.jsonl (the exact NDJSON contract the client's curl
#     fetch consumes), /api/search
#   - `operon publish` mints a real index line from a real git package
#     (rev + checkout_checksum), the API accepts it with a token
#   - (name, version) immutability: republish is 409, fix forward
#   - auth: no token = 403, bad token = 403; dir-sourced line = 400
#     (remote registries publish git URLs only — the client's own rule)
#   - `operon search` over the hosted registry (W19 item 3 on this tier)
#   - `operon add NAME` against OPERON_REGISTRY=.../index.jsonl: clone via
#     the published git URL, sha256 checksum verify, vendor, lock
#   - `use <pkg>` + run through the hosted-installed module, note-clean
#   - `mod verify` checksum contract holds through the hosted tier
set -uo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-./bin/operon}"
case "$BIN" in /*) ;; *) BIN="$PWD/${BIN#./}" ;; esac
PORT="${HOSTED_E2E_PORT:-8799}"
WORK=$(mktemp -d)
SRV=""
export OPERON_REGISTRY_HOME="$WORK/registry-home"
export OPERON_DEPS="$WORK/deps"
export HOME="$WORK/home"
mkdir -p "$HOME" "$WORK/proj"
pass=0; fail=0
ok()   { pass=$((pass+1)); echo "ok    $1"; }
bad()  { fail=$((fail+1)); echo "FAIL  $1"; [ "${VERBOSE:-0}" = "1" ] && [ -n "${2:-}" ] && echo "      $2"; return 0; }
check(){ if [ "$1" = "0" ]; then ok "$2"; else bad "$2" "${3:-}"; fi; }

cleanup() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; [ -n "$SRV" ] && wait "$SRV" 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT

[ -x "$BIN" ] || { echo "fatal: operon binary not found at $BIN (build first)"; exit 2; }

echo "== operon HOSTED registry e2e (API: packaging/registry/app.py) =="

# ---- 1. boot the hosted API -------------------------------------------------
env -u DATABASE_URL PORT="$PORT" OPERON_TOKENS="hostedtoken" \
  OPERON_REGISTRY_DB="$WORK/hosted.db" \
  python3 packaging/registry/app.py >"$WORK/server.log" 2>&1 &
SRV=$!
UP=0
for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && { UP=1; break; }
  sleep 0.2
done
[ "$UP" = "1" ] || { cat "$WORK/server.log"; echo "fatal: hosted API did not come up"; exit 2; }
check 0 "hosted API is live on :$PORT"

curl -fsS "http://127.0.0.1:$PORT/healthz" | grep -q '"operon-registry"'
check $? "healthz identifies the service"

# empty index served byte-clean
[ "$(curl -fsS "http://127.0.0.1:$PORT/index.jsonl" | wc -c)" = "0" ]
check $? "fresh index.jsonl is empty (no phantom bytes)"

# ---- 2. a real git package, published through the real CLI -------------------
mkdir -p "$WORK/hostdemo"
cat > "$WORK/hostdemo/operon.toml" <<'EOF'
[package]
name = "hostdemo"
version = "0.2.0"
operon-version = "2.2"
description = "hosted tier e2e demo package"
[deps]
EOF
cat > "$WORK/hostdemo/hostdemo.op" <<'EOF'
# hostdemo — published through the hosted tier in the e2e
gene ping(s) {
    return "pong: " + s;
}
EOF
(cd "$WORK/hostdemo" && git init -q . && git -c user.email=e2e@operon -c user.name=e2e add -A && git -c user.email=e2e@operon -c user.name=e2e commit -qm "hostdemo 0.2.0")
(cd "$WORK/hostdemo" && "$BIN" publish --registry "$WORK/local.jsonl" --url "file://$WORK/hostdemo" --desc "hosted tier e2e demo package" >"$WORK/pub.out" 2>&1)
check $? "operon publish mints an index line (git URL + rev + checksum)"
grep -q '"name": "hostdemo"' "$WORK/local.jsonl"
check $? "local index line carries the package"
grep -q '"sha256": "' "$WORK/local.jsonl"
check $? "index line carries the checkout checksum"

# ---- 3. hosted publish: POST the line (token, immutability, auth, validation)
curl -fsS -X POST -H "Authorization: Bearer hostedtoken" \
  -H "Content-Type: application/json" \
  --data-binary @"$WORK/local.jsonl" \
  "http://127.0.0.1:$PORT/api/publish" >"$WORK/post.json" 2>&1
check $? "POST /api/publish with a token accepts the line"
grep -q '"published": "hostdemo 0.2.0"' "$WORK/post.json"
check $? "publish response confirms"
curl -s -o /dev/null -w "%{http_code}" -X POST -H "Authorization: Bearer hostedtoken" \
  --data-binary @"$WORK/local.jsonl" "http://127.0.0.1:$PORT/api/publish" | grep -q 409
check $? "republish is 409 — (name, version) is immutable"
curl -s -o /dev/null -w "%{http_code}" -X POST --data-binary @"$WORK/local.jsonl" \
  "http://127.0.0.1:$PORT/api/publish" | grep -q 403
check $? "publish without a token is 403"
curl -s -o /dev/null -w "%{http_code}" -X POST -H "Authorization: Bearer wrongtoken" \
  --data-binary @"$WORK/local.jsonl" "http://127.0.0.1:$PORT/api/publish" | grep -q 403
check $? "publish with a wrong token is 403"
printf '{"name": "evil", "version": "1.0", "git": "x", "rev": "y", "dir": "/etc"}\n' > "$WORK/dirline.jsonl"
curl -s -o /dev/null -w "%{http_code}" -X POST -H "Authorization: Bearer hostedtoken" \
  --data-binary @"$WORK/dirline.jsonl" "http://127.0.0.1:$PORT/api/publish" | grep -q 400
check $? "dir-sourced line is 400 (remote registries publish git URLs only)"
printf '{"name": "EVIL", "version": "1.0", "git": "x", "rev": "y"}\n' > "$WORK/badname.jsonl"
curl -s -o /dev/null -w "%{http_code}" -X POST -H "Authorization: Bearer hostedtoken" \
  --data-binary @"$WORK/badname.jsonl" "http://127.0.0.1:$PORT/api/publish" | grep -q 400
check $? "invalid package name is 400"

# ---- 4. the stock client consumes the hosted index --------------------------
HOSTED="http://127.0.0.1:$PORT/index.jsonl"
(cd "$WORK/proj" && "$BIN" new myapp >/dev/null 2>&1 && cd myapp \
  && OPERON_REGISTRY="$HOSTED" "$BIN" search host 2>&1 | grep -q "hostdemo 0.2.0")
check $? "operon search finds hostdemo through the hosted API"
(cd "$WORK/proj" && "$BIN" new emptyproj >/dev/null 2>&1 && cd emptyproj \
  && OPERON_REGISTRY="$HOSTED" "$BIN" search zzz-nothing 2>&1 | grep -q "no packages matching")
check $? "search reports misses honestly (no matches message)"
(cd "$WORK/proj/myapp" && OPERON_REGISTRY="$HOSTED" "$BIN" add hostdemo >/dev/null 2>&1)
check $? "operon add hostdemo resolves through the hosted API (clone + checksum + vendor)"
grep -q 'git = "registry:hostdemo"' "$WORK/proj/myapp/operon.lock"
check $? "lock records the hosted registry source"
(cd "$WORK/proj/myapp" && "$BIN" mod verify >/dev/null 2>&1)
check $? "mod verify: checksum contract holds through the hosted tier"

# ---- 5. use + run through the hosted-installed module ------------------------
cat > "$WORK/proj/myapp/src/app.op" <<'EOF'
use hostdemo

gene main {
    print(hostdemo.ping("hosted"));
    return 0;
}
EOF
(cd "$WORK/proj/myapp" && "$BIN" run src/app.op 2>"$WORK/run.err" | grep -q "pong: hosted")
check $? "program runs against the hosted-installed package"
if grep -q "fallback" "$WORK/run.err"; then bad "run is note-clean"; else ok "run is note-clean"; fi

# ---- 6. served index is byte-exact; /api/search answers ---------------------
curl -fsS "$HOSTED" > "$WORK/served.jsonl"
tr -d '\n' < "$WORK/served.jsonl" > "$WORK/served.flat"
tr -d '\n' < "$WORK/local.jsonl" > "$WORK/local.flat"
cmp -s "$WORK/served.flat" "$WORK/local.flat"
check $? "index.jsonl serves the published line byte-exact"
curl -fsS "http://127.0.0.1:$PORT/api/search?q=hosted" | grep -q "hostdemo"
check $? "/api/search?q= answers with the package"

echo ""
echo "== hosted registry e2e: $pass passed, $fail failed =="
[ "$fail" = "0" ] || exit 1
exit 0
