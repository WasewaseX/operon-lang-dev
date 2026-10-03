#!/usr/bin/env bash
# typeck_e2e.sh — the W001 stage-2 checker gate (TYPE-SYSTEM.md is normative).
# The static checker (src/typeck.rs, rules T01..T10) landed on main via the
# ai/type-system merge with NO standing gate — the debug_e2e rot class: a
# 56KB module nothing exercised, so any regression would have been silent.
# This gate pins:
#   * the flagship catch (T02 unknown-member) and the core mismatches
#     (T01 binding/return, T03 argument),
#   * type aliases RESOLVE (a violation names the resolved type, not the alias),
#   * unions and optionals accept their families,
#   * Total Grammar (SPEC §4): an unknown type name is a T04 note, never a
#     rejection and never a nonzero exit,
#   * the prime directive (TYPE-SYSTEM.md §1): plain check and plain run are
#     untouched by every finding the checker raises — dynamic semantics stay
#     byte-for-byte frozen, and
#   * the stage-1 runtime soft contract still arms (contained [unfolded]
#     stress, catchable, rc=0 — the compatibility fallback).
# Known open (tracked in TODO-100 W001 REMAIN, deliberately NOT pinned here):
# typed collections sugar (`List<T>`) — list-literal inference degrades
# conservatively today (`list[any]` + null-inference T01 on a VALID
# assignment); flip that here when the inference lands.
set -euo pipefail
cd "$(dirname "$0")/.."
OP="$(pwd)/bin/operon"
SB=$(mktemp -d)
trap 'rm -rf "$SB"' EXIT

expect_code() { # expect_code <file> <code> — typed finding present, rc=3
  local file="$1" code="$2" out rc
  out=$("$OP" check "$file" --typed 2>&1) && rc=0 || rc=$?
  echo "$out" | grep -qF "(TYPED-MODE)[$code]" \
    || { echo "FAIL: $file: expected $code, got:"; echo "$out" | head -4; exit 1; }
  [ "$rc" = 3 ] || { echo "FAIL: $file: rc $rc, wanted 3"; echo "$out" | tail -1; exit 1; }
}
expect_clean() { # expect_clean <file> — typed mode, zero findings, rc=0
  local out rc
  out=$("$OP" check "$1" --typed 2>&1) && rc=0 || rc=$?
  { [ "$rc" = 0 ] && ! echo "$out" | grep -q "TYPED-MODE"; } \
    || { echo "FAIL: $1: expected clean, got:"; echo "$out" | head -4; exit 1; }
}

cat > "$SB/let_mismatch.op" <<'EOF'
let n: int = "s"
EOF
cat > "$SB/return_mismatch.op" <<'EOF'
gene f() -> int {
    return "s"
}
print(f())
EOF
cat > "$SB/unknown_member.op" <<'EOF'
x = 10
print(x.name())
print("after")
EOF
cat > "$SB/arg_mismatch.op" <<'EOF'
gene f(a: int) {
    print(a)
}
f("s")
EOF
cat > "$SB/alias_neg.op" <<'EOF'
type UserId = int
let u: UserId = "s"
EOF
cat > "$SB/alias_pos.op" <<'EOF'
type UserId = int
let u: UserId = 5
print(u)
EOF
cat > "$SB/union_pos.op" <<'EOF'
gene h(v: int | str) {
    print(v)
}
h(1)
h("x")
EOF
cat > "$SB/optional_pos.op" <<'EOF'
let n: int? = null
print(n)
EOF
cat > "$SB/unknown_name.op" <<'EOF'
let z: Foo = 1
print(z)
EOF

# --- core mismatches (typed findings + nonzero exit)
expect_code "$SB/let_mismatch.op"    T01
expect_code "$SB/return_mismatch.op" T01
expect_code "$SB/unknown_member.op"  T02
expect_code "$SB/arg_mismatch.op"    T03

# --- aliases resolve: the violation names the RESOLVED type (int), not the alias
OUT=$("$OP" check "$SB/alias_neg.op" --typed 2>&1) || true
echo "$OUT" | grep -qF "annotated 'int'" \
  || { echo "FAIL: alias did not resolve in the finding text:"; echo "$OUT" | head -4; exit 1; }

# --- families accept their members
expect_clean "$SB/alias_pos.op"
expect_clean "$SB/union_pos.op"
expect_clean "$SB/optional_pos.op"

# --- Total Grammar: unknown type name = T04 note, 0 errors, rc=0
OUT=$("$OP" check "$SB/unknown_name.op" --typed 2>&1) || true
echo "$OUT" | grep -qF "(TYPED-MODE)[T04]" \
  || { echo "FAIL: unknown type name did not produce a T04 note:"; echo "$OUT" | head -4; exit 1; }
echo "$OUT" | grep -qF "0 error(s)" \
  || { echo "FAIL: unknown type name was REJECTED (Total Grammar breach):"; echo "$OUT" | head -4; exit 1; }

# --- the prime directive: plain check stays clean on every checker fixture
for f in let_mismatch return_mismatch unknown_member arg_mismatch alias_neg; do
  OUT=$("$OP" check "$SB/$f.op" 2>&1) && rc=0 || rc=$?
  { [ "$rc" = 0 ] && ! echo "$OUT" | grep -q "TYPED-MODE"; } \
    || { echo "FAIL: prime directive: plain check of $f.op is not clean (rc=$rc):"; echo "$OUT" | head -4; exit 1; }
done

# --- the prime directive, runtime half: plain run of the T02 fixture flows
#     through the dynamic law (unknown member -> null) and completes
OUT=$("$OP" run "$SB/unknown_member.op" 2>/dev/null)
echo "$OUT" | grep -q "after" \
  || { echo "FAIL: plain run did not complete past the unknown member"; exit 1; }

# --- the stage-1 fallback still arms: the let violation is a CONTAINED
#     [unfolded] stress at runtime, catchable, rc=0 (compat contract)
OUT=$("$OP" run "$SB/let_mismatch.op" 2>&1) && rc=0 || rc=$?
echo "$OUT" | grep -qF "type annotation violated" \
  || { echo "FAIL: stage-1 runtime soft contract did not fire:"; echo "$OUT" | head -4; exit 1; }
[ "$rc" = 0 ] || { echo "FAIL: contained stress must not fail the run (rc=$rc)"; exit 1; }

echo "typeck e2e: ALL GREEN"
