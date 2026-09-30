#!/usr/bin/env bash
# pkg_tls_e2e.sh — https transport gate for the package system
# (ai/ecosystem lane, item 10: release distribution).
#
# The default build has no TLS (zero-external-crates policy), so this gate
# compiles the opt-in `tls` variant (rustls) into its own target dir and
# proves the full https package lifecycle against the REAL registry service:
#
#   cargo build --features tls
#     -> openssl self-signed cert (the internal-CA shape)
#     -> packaging/registry/app.py booted with OPERON_TLS_CERT/KEY
#     -> operon publish / search / add / run over https://127.0.0.1
#     -> negative checks: no-CA untrusted rejection, no-token rejection,
#        version immutability
#     -> real-internet TLS sanity against a public https host (offline-safe)
#
# The default (no-tls) binary is additionally checked to refuse https with
# the actionable rebuild message.
#
# Usage: scripts/pkg_tls_e2e.sh
set -u

REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
PORT="${TLS_E2E_PORT:-8793}"
WORK="$(mktemp -d)"
PASS=0
FAIL=0
SRV=""
TLSOP=""
DEFAULTOP=""

cleanup() {
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  [ -n "$SRV" ] && wait "$SRV" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

ok()  { PASS=$((PASS + 1)); echo "  ok: $1"; }
bad() { FAIL=$((FAIL + 1)); echo "  FAIL: $1"; }
check() { if [ "$2" = "0" ]; then ok "$1"; else bad "$1"; fi; }
die() { echo "fatal: $*" >&2; exit 2; }

export PATH="$HOME/.cargo/bin:$PATH"
command -v openssl >/dev/null 2>&1 || die "openssl not found (required for the self-signed internal-CA shape)"
command -v cargo >/dev/null 2>&1 || die "cargo not found (install the Rust toolchain first)"

echo "== operon package TLS e2e =="
echo "== workdir: $WORK =="

echo "[1/8] cargo build --features tls (own target dir; default build untouched)"
cargo build --release --features tls --target-dir target/tls 2>"$WORK/build.log" || {
  cat "$WORK/build.log"; die "tls build failed"; }
TLSOP="$WORK/operon-tls"
cp target/tls/release/operon "$TLSOP"
[ -x "$TLSOP" ]; check "tls binary built" $?
[ -f bin/operon ] && DEFAULTOP="$(pwd)/bin/operon"

echo "[2/8] internal-CA shape: CA cert + CA-signed server cert"
# The real self-hosted shape: an internal CA (trust anchor handed to the
# client via OPERON_CA_FILE) signs the server's end-entity cert. A bare
# self-signed cert would be CA:TRUE and rustls rightly rejects
# "CaUsedAsEndEntity" — exactly the discipline the feature should enforce.
openssl req -x509 -newkey rsa:2048 -keyout "$WORK/ca.key.pem" -out "$WORK/ca.pem" \
  -days 2 -nodes -subj "/CN=Operon Test CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign" 2>/dev/null
openssl req -newkey rsa:2048 -keyout "$WORK/key.pem" -out "$WORK/csr.pem" \
  -nodes -subj "/CN=127.0.0.1" \
  -addext "subjectAltName=IP:127.0.0.1,DNS:localhost" 2>/dev/null
openssl x509 -req -in "$WORK/csr.pem" -CA "$WORK/ca.pem" -CAkey "$WORK/ca.key.pem" \
  -CAcreateserial -out "$WORK/cert.pem" -days 2 -copy_extensions copyall 2>/dev/null
[ -f "$WORK/cert.pem" ] && [ -f "$WORK/ca.pem" ]; check "CA + signed server cert generated" $?

echo "[3/8] boot the registry with TLS"
env -u DATABASE_URL PORT="$PORT" OPERON_TOKENS="tlstoken" \
  OPERON_TLS_CERT="$WORK/cert.pem" OPERON_TLS_KEY="$WORK/key.pem" \
  OPERON_REGISTRY_DB="$WORK/tlsreg.db" \
  python3 packaging/registry/app.py >"$WORK/server.log" 2>&1 &
SRV=$!
UP=0
for _ in $(seq 1 50); do
  if openssl s_client -connect "127.0.0.1:$PORT" -servername localhost \
       </dev/null 2>/dev/null | grep -q "BEGIN CERTIFICATE"; then UP=1; break; fi
  sleep 0.2
done
[ "$UP" = "1" ] || { cat "$WORK/server.log"; die "TLS registry did not come up on :$PORT"; }
check "https registry is live (TLS handshake via openssl s_client)" 0

# --- 4. publish over https -----------------------------------------------------
echo "[4/8] publish over https"
cd "$WORK"
rm -rf tlsapp && "$TLSOP" new tlsapp lib >/dev/null 2>&1
(cd tlsapp && \
  OPERON_REGISTRY="https://127.0.0.1:$PORT" OPERON_CA_FILE="$WORK/ca.pem" \
  OPERON_TOKEN="tlstoken" "$TLSOP" publish >"$WORK/pub.out" 2>&1)
RC=$?
[ "$RC" = "0" ] || sed 's/^/    | /' "$WORK/pub.out"
check "publish with CA file + token" $RC
grep -q '"published": "tlsapp' "$WORK/pub.out"
RC=$?
[ "$RC" = "0" ] || sed 's/^/    | /' "$WORK/pub.out"
check "publish output confirms" $RC
(cd tlsapp && \
  OPERON_REGISTRY="https://127.0.0.1:$PORT" OPERON_CA_FILE="$WORK/ca.pem" \
  OPERON_TOKEN="tlstoken" "$TLSOP" publish >/dev/null 2>&1)
[ "$?" != "0" ]; check "republish is rejected (versions immutable)" $?
(cd tlsapp && \
  OPERON_REGISTRY="https://127.0.0.1:$PORT" OPERON_CA_FILE="$WORK/ca.pem" \
  "$TLSOP" publish >/dev/null 2>&1)
[ "$?" != "0" ]; check "publish without a token is rejected" $?
(cd tlsapp && \
  OPERON_REGISTRY="https://127.0.0.1:$PORT" \
  OPERON_TOKEN="tlstoken" "$TLSOP" publish >/dev/null 2>&1)
[ "$?" != "0" ]; check "self-signed registry is refused WITHOUT OPERON_CA_FILE (TLS verification is real)" $?

# --- 5. search + add + run over https -------------------------------------------
echo "[5/8] search + add + run over https"
OPERON_REGISTRY="https://127.0.0.1:$PORT" OPERON_CA_FILE="$WORK/ca.pem" \
  "$TLSOP" search tls >"$WORK/search.out" 2>&1
grep -q "tlsapp" "$WORK/search.out"; check "search finds tlsapp over https" $?
rm -rf tlsuse && "$TLSOP" new tlsuse >/dev/null 2>&1 && cd tlsuse
OPERON_REGISTRY="https://127.0.0.1:$PORT" OPERON_CA_FILE="$WORK/ca.pem" \
  "$TLSOP" add tlsapp >/dev/null 2>&1
check "add tlsapp over https (resolve + install)" $?
[ -f operon.lock ]; check "operon.lock written" $?
[ -f operon_modules/tlsapp/tlsapp.op ]; check "entry module installed" $?
printf 'use tlsapp\n\ngene main {\n    print(tlsapp.hello("tls"));\n    return 0;\n}\n' > src/main.op
OPERON_REGISTRY="https://127.0.0.1:$PORT" OPERON_CA_FILE="$WORK/ca.pem" \
  "$TLSOP" run >"$WORK/run.out" 2>"$WORK/run.err"
check "project run exits 0" $?
grep -q "hello, tls" "$WORK/run.out"; check "run output uses the https-installed package" $?
if grep -q "fallback" "$WORK/run.err"; then sed 's/^/    | /' "$WORK/run.err"; bad "run is note-clean"; else ok "run is note-clean"; fi

# --- 6. lock reproducibility over https ------------------------------------------
echo "[6/8] lock reproducibility over https"
cp operon.lock "$WORK/lock1"
rm -rf operon_modules
OPERON_REGISTRY="https://127.0.0.1:$PORT" OPERON_CA_FILE="$WORK/ca.pem" \
  "$TLSOP" update >/dev/null 2>&1
check "update re-installs from the https registry" $?
cmp -s operon.lock "$WORK/lock1"; check "re-resolution is byte-identical (reproducible)" $?

# --- 7. default build refuses https honestly --------------------------------------
echo "[7/8] default (no-tls) build refuses https with the actionable message"
if [ -n "$DEFAULTOP" ]; then
  OPERON_REGISTRY="https://127.0.0.1:$PORT" OPERON_CA_FILE="$WORK/ca.pem" \
    "$DEFAULTOP" search tls >"$WORK/refuse.out" 2>&1
  [ "$?" != "0" ]; check "no-tls build fails on https" $?
  grep -q "cargo build --features tls" "$WORK/refuse.out"
  check "refusal names the exact rebuild command" $?
else
  echo "  (bin/operon not built yet — skipped; scripts/test.sh covers the default build)"
fi

# --- 8. real-internet TLS sanity (offline-safe) ------------------------------------
echo "[8/8] real-internet TLS sanity"
if curl -fsS --max-time 8 https://example.com >/dev/null 2>&1; then
  OPERON_REGISTRY="https://example.com" "$TLSOP" search x >"$WORK/net.out" 2>&1
  [ "$?" != "0" ]; check "https to a public host completes the TLS+HTTP roundtrip" $?
  grep -q "HTTP 404" "$WORK/net.out"
  check "public host answered through the https client (404 on /api/search)" $?
else
  echo "  (offline — skipped)"
fi

echo ""
echo "== pkg TLS e2e: $PASS passed, $FAIL failed =="
[ "$FAIL" = "0" ] || exit 1
exit 0
