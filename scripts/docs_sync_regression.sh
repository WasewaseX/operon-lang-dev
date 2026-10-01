#!/usr/bin/env bash
# docs_sync_regression.sh — F3/#47 (reliab lane, 2026-10-02): the docs-sync
# gate is fast AND has teeth, pinned end-to-end.
#
# Hermetic (no Rust, no network; the real checker on the committed tree):
#   1. GATE WINDOW — `timeout 120 python3 scripts/check_docs_sync.py` exits 0
#      with ambient GEN_DOC_STATS_FAST unset (the explicit compute(fast=True)
#      path must win over the environment), inside the 120s window.
#   2. NO SUBPROCESS RECOUNT — compute(fast=True) reports both informational
#      recounts as skipped (the proof-suite run and the ~10 min differential
#      harness are never paid inside the gate; they run as their own gates in
#      scripts/test.sh and CI), while every drift-checked field is a real walk.
#   3. TEETH — a deliberately-wrong drift-checked number (stats.json
#      keyword_count, byte-restored afterwards) fails the gate with the drift
#      message; the tree is restored before any assertion can fail wide.
set -euo pipefail
cd "$(dirname "$0")/.."

WORK="$(mktemp -d)"
STATS_BAK="$WORK/stats.json.bak"
cp docs/stats.json "$STATS_BAK"
restore() { cp "$STATS_BAK" docs/stats.json; rm -rf "$WORK"; }
trap restore EXIT

fail() { echo "DOCS-SYNC REGRESSION FAIL: $1"; exit 1; }

# ---- 1+2. the gate window, with the environment NOT helping ----------------
unset GEN_DOC_STATS_FAST
T0=$(date +%s)
set +e
timeout 120 python3 scripts/check_docs_sync.py > "$WORK/gate.txt" 2>&1
RC=$?
set -e
T1=$(date +%s)
ELAPSED=$((T1 - T0))
[ "$RC" = 0 ] || fail "check_docs_sync must exit 0 on the committed tree inside 120s (got rc=$RC):
$(cat "$WORK/gate.txt")"
[ "$ELAPSED" -lt 120 ] || fail "gate took ${ELAPSED}s wall clock — the fast path is not fast"
grep -q "docs sync OK" "$WORK/gate.txt" || fail "gate output lacks the OK line:
$(cat "$WORK/gate.txt")"
echo "ok   gate window: docs-sync green in ${ELAPSED}s (< 120s), ambient GEN_DOC_STATS_FAST unset"

python3 - <<'PYEOF'
import os, sys
sys.path.insert(0, "scripts")
os.environ.pop("GEN_DOC_STATS_FAST", None)  # the explicit argument must win
from gen_doc_stats import compute
s = compute(fast=True)
assert s["proof_totals"].get("source") == "fast-mode skip", s["proof_totals"]
assert s["harness"] is None, s["harness"]
for k in ("version", "keyword_count", "std_module_count", "std_function_count",
          "redteam_files", "proof_files", "test_op_files"):
    assert s[k] is not None, f"drift-checked field {k} must be a real walk"
assert isinstance(s["keywords"], list) and s["keywords"], "keywords must be a real walk"
assert s["std_modules"], "std module inventory must be a real walk"
# legacy switch preserved: the bare compute() still honors the env var
os.environ["GEN_DOC_STATS_FAST"] = "1"
s2 = compute()
assert s2["proof_totals"].get("source") == "fast-mode skip", s2["proof_totals"]
print("ok   compute(fast=True): recounts skipped, drift fields walked; legacy env switch intact")
PYEOF

# ---- 3. teeth: a wrong number must fail the gate ---------------------------
python3 - <<'PYEOF'
import json
p = "docs/stats.json"
d = json.load(open(p, encoding="utf-8"))
d["keyword_count"] = int(d["keyword_count"]) + 1  # deliberately wrong
json.dump(d, open(p, "w", encoding="utf-8"), indent=1, sort_keys=True)
PYEOF
set +e
timeout 60 python3 scripts/check_docs_sync.py > "$WORK/neg.txt" 2>&1
NRC=$?
set -e
cp "$STATS_BAK" docs/stats.json   # restore BEFORE asserting: never leave the tree dirty
[ "$NRC" = 1 ] || fail "a deliberately-wrong doc number must fail the gate (got rc=$NRC):
$(cat "$WORK/neg.txt")"
grep -q "drift on 'keyword_count'" "$WORK/neg.txt" || fail "expected the keyword_count drift message:
$(cat "$WORK/neg.txt")"
cmp -s docs/stats.json "$STATS_BAK" || fail "stats.json not byte-identical after the negative test"
echo "ok   teeth: wrong keyword_count -> exit 1 with the drift message; tree byte-restored"

echo "ok   docs-sync regression: gate window pinned (<120s), fast path explicit, teeth pinned"
