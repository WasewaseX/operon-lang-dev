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
trap 'rm -rf "$TMP"; rm -f "$DIR/rt_evil_link" "$DIR/rt_wlink" "$DIR/rt_toctou_link" "$DIR/rt_fifo_fixture" "$DIR/rt_toctou_in"; rm -rf "$DIR/rt_evildir" /tmp/redteam-out-escape' EXIT

# recreate the adversarial symlinks (runtime fixtures, never committed):
# rt_evil_link → /etc/passwd (read escape), rt_wlink → /tmp/redteam-out-escape
# (write escape). If a run CREATES the escape target, the suite fails below.
ln -sf /etc/passwd "$DIR/rt_evil_link"
ln -sf /tmp/redteam-out-escape "$DIR/rt_wlink"
mkdir -p "$DIR/rt_evildir" && ln -sf /etc/hostname "$DIR/rt_evildir/hostname"
rm -rf /tmp/redteam-out-escape
mkdir -p /tmp/redteam-out-escape  # S5: a LIVE escape target makes denials non-vacuous

# sec-r5 (F-11): FIFO fixture — read_file must refuse non-regular files
# instead of blocking open() forever.
mkfifo "$DIR/rt_fifo_fixture" 2>/dev/null || true
# sec-r5 (F-8): TOCTOU fixtures — an in-grant plain file and a symlink
# that the flipper below keeps swapping between it and the outside canary.
# Link targets are ABSOLUTE (like a real attacker's); a relative target
# here would be a broken symlink and make the race vacuous.
printf 'canary-pristine' > /tmp/redteam-out-escape/toctou_canary
printf 'in-grant' > "$DIR/rt_toctou_in"
ln -sfn "$(pwd)/$DIR/rt_toctou_in" "$DIR/rt_toctou_link"
# reg-bio-2 (jury 12-score-invariants): fail fast if ./bin/operon is a stale
# copy — every gate that runs ./bin/operon must exercise the same binary the
# other gates built in target/release/.
if [ -f target/release/operon ]; then
    bin_md5=$(md5sum bin/operon 2>/dev/null | cut -d" " -f1)
    tgt_md5=$(md5sum target/release/operon 2>/dev/null | cut -d" " -f1)
    if [ "$bin_md5" != "$tgt_md5" ]; then
        echo "STALE bin/operon (md5 mismatch vs target/release) — run scripts/build.sh first" >&2
        exit 3
    fi
fi

pass=0; fail=0; failed_files=()

run_one() {
    local f="$1"; shift
    local grants=("$@")
    # sz hotfix: -k 5 — a payload that ignores SIGTERM must die by SIGKILL;
    # plain `timeout 15` wedged CI for ~131 s on rt_p4a (TERM ignored, timeout
    # blocked until the process died on its own). rc 137 = SIGKILLed (hang).
    # W09: OPERON_EXTRA_ARGS (e.g. --vm) re-runs the same containment gate
    # against the bytecode engine; default empty = the tree-walk baseline.
    timeout -k 5 15 ./bin/operon run ${OPERON_EXTRA_ARGS:-} "$f" "${grants[@]}" > "$TMP/out" 2> "$TMP/err"
    local rc=$?
    if [ $rc -eq 124 ] || [ $rc -eq 137 ]; then
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
        *p12a*)
            # substrate-r1: py bridge default-deny — NO py grant, the call
            # must be denied (granting nothing is the containment)
            run_one "$f" --allow-read "$DIR" --allow-write "$DIR" --allow-run echo ;;
        *p12b*)
            # substrate-r1: module fence — math granted, os must stay denied
            run_one "$f" --allow-py math --allow-read "$DIR" ;;
        *p12c*)
            # substrate-r1: timeout kill — time granted, 300 ms budget via
            # an explicit operator cell
            run_one "$f" --cell "$DIR/py_grant.cell" --allow-read "$DIR" ;;
        *p14e*)
            # loop-10 (F-9): Rho termination under megacistron load — the
            # opt-in layer armed via an explicit operator cell (catch 0.5,
            # queue_cap 0.0 = unshielded); containment = bounded runtime
            run_one "$f" --cell "$DIR/rt_p14e.cell" --allow-read "$DIR" ;;
        *p15a*)
            # M100 W007: uncaught deep chain via the exit-1 entry path —
            # rc must be 1 (dx-r1 honesty), render capped, no panic
            run_one "$f" --entry go ;;
        *p21a*)
            # W013/W015 closure: close-while-recv + parked-recv leak storm.
            # A tiny run-wide fuel pool makes the containment observable: a
            # recv parked on a channel nobody sends to drains 50k fuel per
            # 50 ms wake (and its own step budget), so every leak must END
            # on the catchable overflow stress, never hang.
            run_one "$f" --fuel 600000 ;;
        *p24a*|*p24b*)
            # W016: async containment — the fiber leak storm rides the
            # live-task cap (4096) and the cancel storm proves no parked
            # fiber outlives its join (the io.pool cell is required: without
            # it the payloads take the thread path and prove nothing about
            # fibers).
            run_one "$f" --cell "$DIR/async.cell" --entry go ;;
        *p24c*)
            # W016: the never-fed park drains the run-wide pool (operator-set
            # small --fuel) into the catchable overflow at the suspension
            # point — a parked fiber can never outlive the run's budget.
            run_one "$f" --cell "$DIR/async.cell" --entry go --fuel 600000 ;;
        *p15b*|*p15c*)
            # M100 W007: chain on rescue bindings — contained (rc=0),
            # frames leak nothing beyond gene names + in-file lines
            run_one "$f" ;;
        *cell*)
            run_one "$f" --cell "$DIR/rt_grant.cell" ;;
        *p11n*)
            # sec-r5 F-11: /dev/zero is a char device — the grant is real,
            # the refusal must come from the regular-file check, not the
            # sandbox (default-deny would make the test vacuous)
            run_one "$f" --allow-read /dev --allow-read "$DIR" ;;
        *p11h*|*toctou*)
            # the payload writes through rt_toctou_link INSIDE the granted
            # dir; the flipper escapes are caught by fd verification
            run_one "$f" --allow-write "$DIR" --allow-read "$DIR" ;;
        *p11f*|*fifo*)
            run_one "$f" --allow-read "$DIR" ;;
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

# sec-r2 (audit C-11): exit() is a capability, default-deny. The payload
# calls exit(99) inside stress/rescue; if the process dies with 99 the
# sandbox is breached — a contained run rescues, prints "survived", exits 0.
timeout 5 ./bin/operon run "$DIR/rt_x_exit_denied.op" --allow-read "$DIR" --allow-write "$DIR" > "$TMP/out" 2> "$TMP/err"
exit_rc=$?
if [ $exit_rc -eq 99 ] || [ $exit_rc -eq 124 ] || ! grep -q "survived" "$TMP/out"; then
    echo "BREACH exit-capability (rc=$exit_rc)"
    fail=$((fail+1)); failed_files+=("exit-capability")
else
    echo "ok    exit-capability (rc=$exit_rc)"
    pass=$((pass+1))
fi

# sec-r2 (audit A14): run() children die at the wall-clock timeout. The
# payload spawns `sleep 10` under run.timeout_ms=300; the interpreter must
# return in well under 8s with ok=false — not hang (rc=124) and not wait.
start=$SECONDS
timeout 15 ./bin/operon run "$DIR/rt_x_run_timeout.op" --allow-run sleep --cell "$DIR/rt_x_timeout.cell" > "$TMP/out" 2> "$TMP/err"
to_rc=$?
elapsed=$((SECONDS - start))
if [ $to_rc -eq 124 ] || [ $elapsed -ge 8 ] || ! grep -q "ok=false" "$TMP/out"; then
    echo "BREACH run-timeout (rc=$to_rc, ${elapsed}s)"
    fail=$((fail+1)); failed_files+=("run-timeout")
else
    echo "ok    run-timeout (rc=$to_rc, ${elapsed}s)"
    pass=$((pass+1))
fi

# sec-r4 (audit F-6): run() child output is capped at 64 MiB per stream.
# The payload's child emits ~80 MB; the interpreter must return a collected
# prefix <= 67108864 chars with rc 0 -- not OOM, not the full 80000000.
timeout 30 ./bin/operon run "$DIR/rt_p10e_run_output_cap.op" --allow-run sh > "$TMP/out" 2> "$TMP/err"
cap_rc=$?
cap_len=$(grep -oE "capped: [0-9]+" "$TMP/out" | grep -oE "[0-9]+" || echo 0)
if [ $cap_rc -eq 124 ] || [ $cap_rc -eq 137 ] || [ "$cap_len" -gt 67125248 ] || [ "$cap_len" -eq 0 ]; then
    echo "BREACH run-output-cap (rc=$cap_rc, len=$cap_len)"
    fail=$((fail+1)); failed_files+=("run-output-cap")
else
    echo "ok    run-output-cap (len=$cap_len)"
    pass=$((pass+1))
fi

# sec-r5 (F-8): TOCTOU symlink-swap orchestrator. A flipper process races
# the payload's 2000 writes through rt_toctou_link, swapping it between
# the in-grant file and the outside canary. CONTAINMENT = the canary's
# content is untouched after the race (post-open fd verification means
# no byte ever lands outside the grant).
( for i in $(seq 1 400); do
    ln -sfn /tmp/redteam-out-escape/toctou_canary "$DIR/rt_toctou_link" 2>/dev/null
    ln -sfn "$(pwd)/$DIR/rt_toctou_in" "$DIR/rt_toctou_link" 2>/dev/null
  done ) &
FLIPPER=$!
timeout 30 ./bin/operon run "$DIR/rt_p11h_toctou.op" --allow-write "$DIR" --allow-read "$DIR" > "$TMP/out" 2> "$TMP/err"
toctou_rc=$?
wait $FLIPPER
canary=$(cat /tmp/redteam-out-escape/toctou_canary 2>/dev/null || echo MISSING)
if [ $toctou_rc -ge 130 ] || [ $toctou_rc -eq 124 ] || [ "$canary" != "canary-pristine" ] || ! grep -q "p11h-contained" "$TMP/out"; then
    echo "BREACH toctou (rc=$toctou_rc, canary='$canary')"
    fail=$((fail+1)); failed_files+=("toctou")
else
    echo "ok    toctou (canary pristine, $(grep -oE 'denied: [0-9]+' "$TMP/out" | head -1))"
    pass=$((pass+1))
fi

if [ -n "$(ls -A /tmp/redteam-out-escape 2>/dev/null | grep -v toctou_canary)" ]; then
    echo "ESCAPE: files were created inside /tmp/redteam-out-escape — sandbox breached"
    fail=$((fail+1))
fi
rm -f "$DIR/rt_evil_link" "$DIR/rt_wlink" "$DIR/rt_toctou_link" "$DIR/rt_fifo_fixture" "$DIR/rt_toctou_in"; rm -rf "$DIR/rt_evildir"
echo "redteam: $pass contained, $fail breached"
if [ $fail -gt 0 ]; then
    printf '  %s\n' "${failed_files[@]}"
    exit 1
fi
