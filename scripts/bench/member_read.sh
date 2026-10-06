#!/usr/bin/env bash
# bench: member_read driver — P4-safe member-access before/after evidence.
# Same-binary interleaved discipline (the PR #101 house pattern): the shell
# alternates BASE and NEW binaries REPS times, keeps the min per (bin, leg).
#   OPERON_BASE  — pre-fix binary (default: /tmp/p4safe/operon-base)
#   OPERON_NEW   — post-fix binary (default: bin/operon)
# Output: MEMBERBEST bin=<base|new> leg=<name> ns_per_read=<min>
set -u
cd "$(dirname "$0")/../.."

BASE=${OPERON_BASE:-/tmp/p4safe/operon-base}
NEW=${OPERON_NEW:-bin/operon}
REPS=${MEMBER_REPS:-3}

[ -x "$BASE" ] || { echo "member_read.sh: base binary $BASE not found" >&2; exit 1; }
[ -x "$NEW" ] || { echo "member_read.sh: new binary $NEW not found" >&2; exit 1; }

for rep in $(seq 1 "$REPS"); do
  "$BASE" run scripts/bench/member_read.op 2>/dev/null | grep '^MEMBER ' | sed 's/^/BIN=base /'
  "$NEW" run scripts/bench/member_read.op 2>/dev/null | grep '^MEMBER ' | sed 's/^/BIN=new /'
done | awk '
  /BIN=/ {
    bin = ""; leg = ""; ns = "";
    for (i = 1; i <= NF; i++) {
      if ($i ~ /^BIN=/)      { bin = substr($i, 5) }
      if ($i ~ /^leg=/)      { leg = substr($i, 5) }
      if ($i ~ /^ns_per_read=/) { ns = substr($i, 13) }
    }
    if (bin != "" && leg != "" && ns != "") {
      k = bin "," leg
      if (!(k in best) || ns + 0 < best[k]) best[k] = ns + 0
    }
  }
  END {
    for (k in best) print "MEMBERBEST bin=" substr(k, 1, index(k, ",") - 1) \
      " leg=" substr(k, index(k, ",") + 1) " ns_per_read=" best[k]
  }' | sort
