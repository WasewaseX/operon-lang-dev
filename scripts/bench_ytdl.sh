#!/usr/bin/env bash
# bench_ytdl.sh — comparison harness for the ytdl app builds.
# Measures: runtime size, cold start, orchestration overhead (mock engines),
# decision-matrix cost, and LOC. Prints a markdown table on stdout.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$HERE/.."
OP="$REPO/target/release/operon"
PY="$(command -v python3 || true)"
FIXTURE="$REPO/apps/ytdl/test/mock/fixtures/video1.json"
MOCKS="$REPO/apps/ytdl/test/mock"
N=${N:-20}
M=${M:-10}

mb() { awk -v b="$1" 'BEGIN {printf "%.1f", b/1048576}'; }
sz() { stat -c%s "$1" 2>/dev/null || echo 0; }

# ---------- sizes
op_sz=$(sz "$OP")
py_bin="$(command -v python3)"
py_sz=$(stat -Lc%s "$py_bin" 2>/dev/null || echo 0)
sh_sz=$(sz /usr/bin/bash)
deno_bin="$(command -v deno || true)"
deno_sz=0
[ -n "$deno_bin" ] && deno_sz=$(sz "$deno_bin")

# ---------- timing helpers (python for the clock: best available timer)
time_ms() { # time_ms N -- cmd...
  local n="$1"; shift 2
  "$PY" - "$n" "$@" <<'EOF'
import subprocess, sys, time
n = int(sys.argv[1]); cmd = sys.argv[2:]
xs = []
for _ in range(n):
    t0 = time.perf_counter()
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    xs.append((time.perf_counter() - t0) * 1000)
xs.sort()
print(f"{xs[len(xs)//2]:.1f}")
EOF
}

# ---------- cold start
op_cold=$(time_ms "$N" -- "$OP" run "$REPO/examples/hello.op")
py_cold=$(time_ms "$N" -- "$PY" -c "pass")
sh_cold=$(time_ms "$N" -- /usr/bin/bash -c "exit 0")
deno_cold="n/a"
[ -n "$deno_bin" ] && deno_cold=$(time_ms "$N" -- "$deno_bin" eval "void 0;")

# ---------- orchestration overhead (doctor against mock engines)
export PATH="$MOCKS:$PATH"
CELL="$REPO/apps/ytdl/ytdl.cell"
op_doc=$(time_ms "$M" -- "$OP" run "$REPO/apps/ytdl/ytdl.op" --cell "$CELL" --allow-run yt-dlp --allow-run ffmpeg --allow-run aria2c --fuel 20000000000 -- doctor)
py_doc=$(time_ms "$M" -- "$PY" "$REPO/apps/ytdl-compare/python/ytdl.py" doctor)
sh_doc=$(time_ms "$M" -- /usr/bin/bash "$REPO/apps/ytdl-compare/bash/ytdl.sh" doctor)
deno_doc="n/a"
[ -n "$deno_bin" ] && deno_doc=$(time_ms "$M" -- "$deno_bin" run --allow-run --allow-read "$REPO/apps/ytdl-compare/deno/ytdl.ts" doctor)

# ---------- decision matrix (selfcheck: JSON parse + sort + format)
op_self=$(time_ms "$M" -- "$OP" run "$REPO/apps/ytdl/ytdl.op" --cell "$CELL" --allow-read "$REPO" --fuel 20000000000 -- selfcheck "$FIXTURE")
py_self=$(time_ms "$M" -- "$PY" "$REPO/apps/ytdl-compare/python/ytdl.py" selfcheck "$FIXTURE")
deno_self="n/a"
[ -n "$deno_bin" ] && deno_self=$(time_ms "$M" -- "$deno_bin" run --allow-read "$REPO/apps/ytdl-compare/deno/ytdl.ts" selfcheck "$FIXTURE")

# ---------- LOC
loc_op=$(wc -l < "$REPO/apps/ytdl/ytdl.op")
loc_py=$(wc -l < "$REPO/apps/ytdl-compare/python/ytdl.py")
loc_ts=$(wc -l < "$REPO/apps/ytdl-compare/deno/ytdl.ts")
loc_sh=$(wc -l < "$REPO/apps/ytdl-compare/bash/ytdl.sh")

echo "| runtime | size (MB) | cold start (ms, median) | doctor x$M mock (ms) | selfcheck x$M (ms) | app LOC |"
echo "|---|---|---|---|---|---|"
echo "| Operon | $(mb $op_sz) | $op_cold | $op_doc | $op_self | $loc_op |"
echo "| Python 3 | $(mb $py_sz)† | $py_cold | $py_doc | $py_self | $loc_py |"
if [ -n "$deno_bin" ]; then
  echo "| Deno | $(mb $deno_sz) | $deno_cold | $deno_doc | $deno_self | $loc_ts |"
else
  echo "| Deno | n/a (not installed here; upstream ~80-100 MB) | n/a | n/a | n/a | $loc_ts |"
fi
echo "| Bash | $(mb $sh_sz)‡ | $sh_cold | $sh_doc | n/a (no JSON) | $loc_sh |"
echo ""
echo "† python3 binary only — a working python needs its stdlib tree (~10-30 MB extra)."
echo "‡ bash alone is not a JSON-capable runtime; the bash build is a deliberate subset."
