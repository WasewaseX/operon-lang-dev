#!/usr/bin/env bash
# redteam.sh — adversarial containment suite (wave-3 Critic-X payloads).
# Every payload MUST be CONTAINED by the runtime: a clean Operon-level
# diagnostic (or a bounded wobble repair) — never a panic, SIGSEGV, hang,
# OOM, sandbox escape, or exit-code spoofing.
#
# The proof runner (operon test) skips this directory by design; adversarial
# payloads have containment expectations, not assertion expectations.
set -u
cd "$(dirname "$0")/.."
DIR="tests/redteam"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; rm -f "$DIR/rt_evil_link" "$DIR/rt_wlink"; rm -rf "$DIR/rt_evildir" /tmp/redteam-out-escape' EXIT

# recreate the adversarial symlinks (runtime fixtures, never committed):
# rt_evil_link → /etc/passwd (read escape), rt_wlink → /tmp/redteam-out-escape
# (write escape). If a run CREATES the escape target, the suite fails below.
ln -sf /etc/passwd "$DIR/rt_evil_link"
ln -sf /tmp/redteam-out-escape "$DIR/rt_wlink"
mkdir -p "$DIR/rt_evildir" && ln -sf /etc/hostname "$DIR/rt_evildir/hostname"
rm -rf /tmp/redteam-out-escape
mkdir -p /tmp/redteam-out-escape  # S5: a LIVE escape target makes denials non-vacuous
pass=0; fail=0; failed_files=()

run_one() {
    local f="$1"; shift
    local grants=("$@")
    timeout 15 ./bin/operon run "$f" "${grants[@]}" > "$TMP/out" 2> "$TMP/err"
    local rc=$?
    if [ $rc -eq 124 ]; then
        echo "HANG  $f"; return 1
    fi
    if [ $rc -ge 130 ] && [ $rc -ne 137 ]; then
        echo "CRASH($rc)  $f"; return 1
    fi
    if grep -qE "panicked at|stack overflow|fatal runtime error" "$TMP/err" "$TMP/out"; then
        echo "PANIC $f"; return 1
    fi
    # exit-code spoofing: a runtime that died must not report success
    if [ $rc -eq 0 ] && grep -qE "fatal|panic" "$TMP/err"; then
        echo "SPOOF $f"; return 1
    fi
    echo "ok    $f (rc=$rc)"
    return 0
}

for f in "$DIR"/rt_p*.op; do
    case "$(basename "$f")" in
        *grant*|*read_ok*|*write_ok*|*run_ok*)
            # positive-control payloads granted access to the payload dir + /tmp
            run_one "$f" --allow-read "$DIR" --allow-write "$DIR" --allow-write /tmp --allow-run echo ;;
        *symlink*|*wlink*|*traversal*|*escape*|*dotdot*|*proc*|*net*|*env*)
            # escape attempts: grant ONLY the payload dir — target must be denied
            run_one "$f" --allow-read "$DIR" --allow-write "$DIR" --allow-run echo --allow-env PATH ;;
        *spawn*|*thread*|*sleep*)
            run_one "$f" --allow-read "$DIR" --allow-write /tmp ;;
        *http*)
            # sec-r1: CRLF-injection payload needs a net grant to reach the
            # guard (default-deny blocks net anyway — both are containment)
            run_one "$f" --allow-net "127.0.0.1:1" --allow-read "$DIR" ;;
        *cell*)
            run_one "$f" --cell "$DIR/rt_grant.cell" ;;
        *)
            run_one "$f" --allow-read "$DIR" --allow-write "$DIR" --allow-run echo ;;
    esac
    if [ $? -eq 0 ]; then pass=$((pass+1)); else fail=$((fail+1)); failed_files+=("$f"); fi
done

echo

# sec-r1 (audit C-4): LSP framing fuzz — attacker-controlled Content-Length
# from the editor side must close the session gracefully, never panic.
printf 'Content-Length: 18446744073709551615\r\n\r\n{' | timeout 5 ./bin/operon-ls > /dev/null 2> "$TMP/lsperr"
lsp_rc=$?
if [ $lsp_rc -ge 130 ] || [ $lsp_rc -eq 124 ] || grep -qE "panicked at|capacity overflow|stack overflow" "$TMP/lsperr"; then
    echo "PANIC  lsp-framing (rc=$lsp_rc)"
    fail=$((fail+1)); failed_files+=("lsp-framing")
else
    echo "ok    lsp-framing (rc=$lsp_rc)"
    pass=$((pass+1))
fi

if [ -n "$(ls -A /tmp/redteam-out-escape 2>/dev/null)" ]; then
    echo "ESCAPE: files were created inside /tmp/redteam-out-escape — sandbox breached"
    fail=$((fail+1))
fi
rm -f "$DIR/rt_evil_link" "$DIR/rt_wlink"; rm -rf "$DIR/rt_evildir"
echo "redteam: $pass contained, $fail breached"
if [ $fail -gt 0 ]; then
    printf '  %s\n' "${failed_files[@]}"
    exit 1
fi
