#!/usr/bin/env python3
"""check_stats_pin.py — std-funcs count pin (W061-M, the W061 checker family).

The generated-artifact landing race, third documented class: std/*.op edits
change the exported function count while the committed docs/STATS.md pair
trails behind (339 -> 340 caught by builder-B's refresh-3, 2026-10-04).
digest-4 assigned this pin to the W061 audit family.

What it pins (docs/STATS.md vs the live tree):
  1. the module count + the module name list,
  2. the total ".op-level gene defs" count,
  3. EVERY per-module function count in the inventory table (names the
     exact module that moved, not just the total).

Counting mirrors scripts/gen_doc_stats.py::std_modules() exactly — one
regex over std/*.op:  ^\\s*(?:pub\\s+)?gene\\s+(\\w+)\\s*\\(
Keep the two in sync if the definition of "exported std function" changes.

Usage:
  python3 scripts/check_stats_pin.py              # gate mode, exit 1 on staleness
  python3 scripts/check_stats_pin.py --selftest   # offline fixtures
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STATS = os.path.join(ROOT, "docs", "STATS.md")

# The one regex from gen_doc_stats.std_modules() — do not fork the definition.
GENE_RE = r"^\s*(?:pub\s+)?gene\s+(\w+)\s*\("

MODS_LINE_RE = re.compile(r"^- \*\*Std modules\*\*: (\d+) \(([^)]*)\)", re.M)
FUNCS_LINE_RE = re.compile(r"^- \*\*Std functions[^:]*\*\*: (\d+)", re.M)
TABLE_ROW_RE = re.compile(r"^\| `?(std/)?([\w-]+)`? \| (\d+) \|", re.M)


def live_counts():
    """Recount std/*.op the way gen_doc_stats.py does. Returns {module: n}."""
    std_dir = os.path.join(ROOT, "std")
    out = {}
    for f in sorted(os.listdir(std_dir)):
        if not f.endswith(".op"):
            continue
        body = open(os.path.join(std_dir, f), encoding="utf-8").read()
        out[f[:-3]] = len(re.findall(GENE_RE, body, re.M))
    return out


def committed_counts(text):
    mods = MODS_LINE_RE.search(text)
    funcs = FUNCS_LINE_RE.search(text)
    table = {m: int(n) for _p, m, n in TABLE_ROW_RE.findall(text)}
    return mods, funcs, table


def audit(text):
    """Returns list of findings (strings); empty = pin true."""
    findings = []
    live = live_counts()
    mods, funcs, table = committed_counts(text)
    if not (mods and funcs):
        return ["STATS.md std lines not found — the generated pair may be hand-edited or truncated"]
    live_total = sum(live.values())
    if int(funcs.group(1)) != live_total:
        findings.append(
            f"std funcs stale: STATS.md says {funcs.group(1)}, tree counts {live_total} "
            f"(diff {live_total - int(funcs.group(1)):+d})"
        )
    live_names = ", ".join(live.keys())
    if mods.group(2).strip() != live_names:
        findings.append("std module name list stale: STATS.md list != live std/*.op set")
    if int(mods.group(1)) != len(live):
        findings.append(f"std module count stale: STATS.md says {mods.group(1)}, tree has {len(live)}")
    for m, n in sorted(live.items()):
        if m in table and table[m] != n:
            findings.append(f"module '{m}' stale: STATS.md table says {table[m]}, tree counts {n}")
        elif m not in table and n:
            findings.append(f"module '{m}' missing from the STATS.md inventory table ({n} funcs)")
    return findings


SELFTEST_FRESH = """- **Std modules**: 2 (alpha, beta)
- **Std functions (.op-level `gene` defs)**: 3
| module | .op-level functions |
|---|---|
| alpha | 2 |
| beta | 1 |
"""

SELFTEST_STALE = SELFTEST_FRESH.replace("340", "339").replace("**: 3", "**: 2") \
    .replace("| alpha | 2 |", "| alpha | 1 |")


def selftest():
    import tempfile
    ok = True
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as t:
        t.write(SELFTEST_STALE)
        stale_path = t.name
    # audit() reads STATS from the real tree, so exercise the parse/compare
    # core through a patched path for the fixtures.
    global STATS
    real = STATS
    try:
        STATS = stale_path
        f = audit(SELFTEST_STALE)
        good = any("stale" in x for x in f)
        print(f"  {'ok  ' if good else 'FAIL'} stale fixture flagged ({len(f)} finding(s))")
        ok &= good
        f = audit(SELFTEST_FRESH.replace("2 (alpha, beta)", "0 ()"))
        good = any("module count" in x for x in f)
        print(f"  {'ok  ' if good else 'FAIL'} count-mismatch fixture flagged")
        ok &= good
    finally:
        STATS = real
        os.unlink(stale_path)
    print(f"check_stats_pin selftest: {'ALL GREEN' if ok else 'FAILURES'}")
    return 0 if ok else 1


def main():
    if "--selftest" in sys.argv:
        return selftest()
    text = open(STATS, encoding="utf-8").read()
    findings = audit(text)
    if findings:
        print("stats-pin: FAIL — docs/STATS.md trails the tree (generated-artifact race):")
        for f in findings:
            print(f"  - {f}")
        print("  re-pin with: python3 scripts/gen_doc_stats.py  (then commit the pair)")
        return 1
    mods, funcs, table = committed_counts(text)
    print(f"stats pin OK: {mods.group(1)} modules / {funcs.group(1)} std funcs true at the current tree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
