#!/usr/bin/env bash
# ============================================================
# bench.sh — cross-language benchmark for the ytdl downloader app
# Hosts: operon (gene app on the Operon VM), python, rust (native),
#        node. All four implement the identical CLI contract.
# Offline determinism: PATH is prefixed with bench/shims (fake
# yt-dlp/ffprobe/ffmpeg with fixed latencies + fixed payloads), so
# every measurement isolates HOST + ORCHESTRATION cost, not network.
#
# Metrics:
#   M1  LOC               (total / code-only / per-language %)
#   M2  artifact size     (binary or script + runtime identity)
#   M3  cold start        (startup mode, 25 reps -> mean/min/stdev)
#   M4  spawn overhead    (spawnbench N sequential child spawns)
#   M5  queue throughput  (32 shim jobs x workers 1/4/8, 3 reps)
#   M6  host RSS          (/usr/bin/time -v peak during queue w=8)
#   M7  error handling    (failing jobs: latency, exit code)
#   M8  host compute      (recursive fib(27) inside the runtime)
# Output: bench/results/*.txt + bench/results/summary.tsv
# ============================================================
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$(cd "$HERE/.." && pwd)"
REPO="$(cd "$APP/../.." && pwd)"
RES="$HERE/results"
export PATH="$HERE/shims:$HOME/.local/bin:$PATH"

OP_BIN="$REPO/target/release/operon"
PY_BIN="$(command -v python3)"
NODE_BIN="$(command -v node)"
RS_BIN="$APP/rust/target/release/ytdl-rs"

mkdir -p "$RES"
: > "$RES/summary.tsv"

# ---------- host commands (script dir for jobs paths) ----------
host_op()   { "$APP/ytdl" "$@"; }
host_py()   { "$PY_BIN" "$APP/python/ytdl.py" "$@"; }
host_rs()   { "$RS_BIN" "$@"; }
host_node() { "$NODE_BIN" "$APP/node/ytdl.mjs" "$@"; }

HOSTS=(op py rs node)
NAMES=(operon python rust node)

stats() { # mean min stdev over stdin (seconds), python-free awk
    awk '{s+=$1; if(min==""||$1<min)min=$1; ss+=$1*$1; n++} END{
        m=s/n; v=ss/n-m*m; if(v<0)v=0;
        printf "%.4f %.4f %.4f", m, min, sqrt(v)}'
}

run_timed() { # prints wall seconds of "$@"; child exit code ignored
    local t0 t1
    t0=$(date +%s.%N)
    "$@" > /dev/null 2>&1 || true
    t1=$(date +%s.%N)
    awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.4f", b-a}'
}

run_timed_rc() { # prints "<seconds> <rc>" (survives command substitution)
    local t0 t1 rc
    t0=$(date +%s.%N)
    "$@" > /dev/null 2>&1
    rc=$?
    t1=$(date +%s.%N)
    awk -v a="$t0" -v b="$t1" -v rc="$rc" 'BEGIN{printf "%.4f %d", b-a, rc}'
}

cd "$APP"   # download artifacts land in $APP/downloads; cleaned per rep

echo "== ytdl cross-language benchmark ==" | tee "$RES/run.log"
echo "host: $(uname -srm) / $(nproc) cores / $(free -h | awk '/Mem/{print $2}') RAM" | tee -a "$RES/run.log"
echo "date: $(date -u +%FT%TZ)" | tee -a "$RES/run.log"

# ---------- M1 LOC ----------
echo "[M1] LOC" | tee -a "$RES/run.log"
{
    echo "metric,lang,total,code_only"
    # operon: comments are '#'
    tot=$(wc -l < "$APP/operon/ytdl.op")
    cod=$(grep -cvE '^\s*(#|$)' "$APP/operon/ytdl.op")
    echo "loc,operon,$tot,$cod"
    tot=$(wc -l < "$APP/python/ytdl.py")
    cod=$(grep -cvE '^\s*(#|$)' "$APP/python/ytdl.py")
    echo "loc,python,$tot,$cod"
    tot=$(cat "$APP/rust/src/main.rs" "$APP/rust/src/mini_json.rs" | wc -l)
    cod=$(cat "$APP/rust/src/main.rs" "$APP/rust/src/mini_json.rs" | grep -cvE '^\s*(//|$)')
    echo "loc,rust,$tot,$cod"
    tot=$(wc -l < "$APP/node/ytdl.mjs")
    cod=$(grep -cvE '^\s*(//|$)' "$APP/node/ytdl.mjs")
    echo "loc,node,$tot,$cod"
} | tee "$RES/m1_loc.csv"

# ---------- M2 artifact size ----------
echo "[M2] artifact size" | tee -a "$RES/run.log"
{
    echo "metric,lang,artifact,bytes"
    echo "artifact,operon,VM-binary-$(basename "$OP_BIN"),$(stat -c%s "$OP_BIN")"
    echo "artifact,operon,app-script-ytdl.op,$(stat -c%s "$APP/operon/ytdl.op")"
    echo "artifact,python,app-script-ytdl.py,$(stat -c%s "$APP/python/ytdl.py")"
    echo "artifact,rust,standalone-binary,$(stat -c%s "$RS_BIN")"
    echo "artifact,node,app-script-ytdl.mjs,$(stat -c%s "$APP/node/ytdl.mjs")"
} | tee "$RES/m2_artifact.csv"

# ---------- M3 cold start (startup mode) ----------
echo "[M3] cold start (startup mode, 25 reps)" | tee -a "$RES/run.log"
for i in "${!HOSTS[@]}"; do
    h="${HOSTS[$i]}"; n="${NAMES[$i]}"
    case $h in
        op)   c=(host_op startup) ;;
        py)   c=(host_py startup) ;;
        rs)   c=(host_rs startup) ;;
        node) c=(host_node startup) ;;
    esac
    # warmup
    "${c[@]}" > /dev/null 2>&1 || true
    times=()
    for r in $(seq 1 25); do times+=("$(run_timed "${c[@]}")"); done
    s=$(printf '%s\n' "${times[@]}" | stats)
    echo -e "coldstart\t$n\t$s" | tee -a "$RES/summary.tsv"
done

# ---------- M4 spawn overhead ----------
echo "[M4] spawn overhead (spawnbench 100, shim yt-dlp --version)" | tee -a "$RES/run.log"
for i in "${!HOSTS[@]}"; do
    h="${HOSTS[$i]}"; n="${NAMES[$i]}"
    case $h in
        op)   out=$(host_op spawnbench 100) ;;
        py)   out=$(host_py spawnbench 100) ;;
        rs)   out=$(host_rs spawnbench 100) ;;
        node) out=$(host_node spawnbench 100) ;;
    esac
    secs=$(echo "$out" | sed -n 's/.*secs=\([0-9.]*\).*/\1/p')
    echo -e "spawn100\t$n\t$secs" | tee -a "$RES/summary.tsv"
done

# ---------- M5 queue throughput ----------
echo "[M5] queue throughput (32 jobs, workers x {1,4,8}, 3 reps)" | tee -a "$RES/run.log"
for w in 1 4 8; do
    for i in "${!HOSTS[@]}"; do
        h="${HOSTS[$i]}"; n="${NAMES[$i]}"
        times=()
        for r in $(seq 1 3); do
            case $h in
                op)   times+=("$(run_timed host_op queue "$HERE/jobs32.txt" --workers "$w")") ;;
                py)   times+=("$(run_timed host_py queue "$HERE/jobs32.txt" --workers "$w")") ;;
                rs)   times+=("$(run_timed host_rs queue "$HERE/jobs32.txt" --workers "$w")") ;;
                node) times+=("$(run_timed host_node queue "$HERE/jobs32.txt" --workers "$w")") ;;
            esac
            rm -rf "$APP/downloads"   # 32 MB/rep otherwise accumulates
        done
        s=$(printf '%s\n' "${times[@]}" | stats)
        echo -e "queue32_w$w\t$n\t$s" | tee -a "$RES/summary.tsv"
    done
done

# ---------- M6 host RSS ----------
echo "[M6] host peak RSS (queue 32 jobs, workers=8; VmHWM sampled)" | tee -a "$RES/run.log"
rss_of() { # VmHWM (KB) of the host process itself, sampled from /proc
    # NOTE: takes the host command DIRECTLY (no bash function wrapper), so
    # the sampled pid is the real host (operon VM / python / rust / node).
    "$@" > /dev/null 2>&1 &
    local pid=$! peak=0 v
    while kill -0 "$pid" 2>/dev/null; do
        v=$(awk '/VmHWM/{print $2}' "/proc/$pid/status" 2>/dev/null)
        if [ -n "$v" ] && [ "$v" -gt "$peak" ]; then peak=$v; fi
        sleep 0.02
    done
    wait "$pid" || true
    echo "$peak"
}
for i in "${!HOSTS[@]}"; do
    h="${HOSTS[$i]}"; n="${NAMES[$i]}"
    case $h in
        op)   kb=$(rss_of "$APP/ytdl" queue "$HERE/jobs32.txt" --workers 8) ;;
        py)   kb=$(rss_of "$PY_BIN" "$APP/python/ytdl.py" queue "$HERE/jobs32.txt" --workers 8) ;;
        rs)   kb=$(rss_of "$RS_BIN" queue "$HERE/jobs32.txt" --workers 8) ;;
        node) kb=$(rss_of "$NODE_BIN" "$APP/node/ytdl.mjs" queue "$HERE/jobs32.txt" --workers 8) ;;
    esac
    rm -rf "$APP/downloads"
    echo -e "rss_kb_w8\t$n\t$kb" | tee -a "$RES/summary.tsv"
done

# ---------- M7 error handling ----------
echo "[M7] error handling (SHIM_FAIL=1, 2-job failing queue)" | tee -a "$RES/run.log"
for i in "${!HOSTS[@]}"; do
    h="${HOSTS[$i]}"; n="${NAMES[$i]}"
    SHIM_FAIL=1
    export SHIM_FAIL
    case $h in
        op)   out=$(run_timed_rc host_op queue "$HERE/jobs_fail.txt" --workers 2) ;;
        py)   out=$(run_timed_rc host_py queue "$HERE/jobs_fail.txt" --workers 2) ;;
        rs)   out=$(run_timed_rc host_rs queue "$HERE/jobs_fail.txt" --workers 2) ;;
        node) out=$(run_timed_rc host_node queue "$HERE/jobs_fail.txt" --workers 2) ;;
    esac
    unset SHIM_FAIL
    t=$(echo "$out" | awk '{print $1}')
    rc=$(echo "$out" | awk '{print $2}')
    rm -rf "$APP/downloads"
    echo -e "failqueue\t$n\t$t\trc=$rc" | tee -a "$RES/summary.tsv"
done

# ---------- M8 host compute ----------
echo "[M8] host compute (fib 27, in-runtime recursion)" | tee -a "$RES/run.log"
for i in "${!HOSTS[@]}"; do
    h="${HOSTS[$i]}"; n="${NAMES[$i]}"
    case $h in
        op)   out=$(host_op cpu 27) ;;
        py)   out=$(host_py cpu 27) ;;
        rs)   out=$(host_rs cpu 27) ;;
        node) out=$(host_node cpu 27) ;;
    esac
    secs=$(echo "$out" | sed -n 's/.*secs=\([0-9.]*\).*/\1/p')
    echo -e "fib27\t$n\t$secs" | tee -a "$RES/summary.tsv"
done

echo "done -> $RES/summary.tsv" | tee -a "$RES/run.log"
