#!/usr/bin/env python3
"""bars_check.py — R0.8 reproducibility half (L-039): the pointer audit for
docs/perf/BARS.md.

The pattern is check_composition_pin.py (W061-D): a measured report that
nothing checks WILL drift silently — so this script re-derives every number
in BARS.md from the saved bars.json and refuses to pass on any mismatch.

Checks (all static, seconds — no benchmark re-run):
  1. Numbers: every cell of BARS.md's two tables must match the JSON value
     at the printed precision (ms cells :.1f, ratio cells :.2f).
  2. Pins: the header pins (HEAD sha, Bar A tag, Bar A tarball sha256) must
     equal the JSON meta; the tarball sha256 must additionally match the
     release's own SHA256SUMS when the cached copy is present.
  3. Refusals: the documented Bar C refusals (regex/json/modules) must be
     refused in the JSON too, and vice versa.
  4. Recipe: every path the recipe references must exist in the tree.

Exit 1 on drift. `--print` emits the exact expected table rows (copy-paste
re-pin, the composition-pin pattern). `--self-test` mutates a copy and
checks the checker notices.

Run:  python3 scripts/bench/bars_check.py
      python3 scripts/bench/bars_check.py --print
      python3 scripts/bench/bars_check.py --self-test
"""
import argparse
import hashlib
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DOC = os.path.join(ROOT, "docs", "perf", "BARS.md")
JSN = os.path.join(ROOT, "docs", "perf", "bars.json")

REFUSALS = {"regex", "json", "modules"}
NO_SCALE = {"fib25", "recursion"}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def expected_rows(data):
    """The rows BARS.md must carry, derived from the JSON."""
    suites = []
    for name, r in data["suites"].items():
        barc = "refused" if r.get("barc", {}).get("refused") \
            else (f"{r['barc']['min']*1000:.1f}" if "barc" in r else "-")
        suites.append("| {} | {} | {} | {} | {} | {}x | {} |".format(
            name, f"{r['head']['min']*1000:.1f}", f"{r['bara']['min']*1000:.1f}",
            f"{r['barb']['min']*1000:.1f}", barc,
            f"{r['head']['min']/r['barb']['min']:.1f}",
            f"{r['head']['min']/r['bara']['min']:.2f}"))
    scaling = []
    for name, s in data["scaling"].items():
        if name in NO_SCALE:
            continue
        scaling.append("| {} | {} | {} | {} |".format(
            name, f"{s['head']['ratio']:.2f}", f"{s['bara']['ratio']:.2f}",
            f"{s['barc']['ratio']:.2f}" if s.get("barc") else "-"))
    return suites, scaling


def parse_doc_rows(text, header_marker):
    """Parse the pipe-table rows under a header marker."""
    rows = {}
    in_table = False
    for line in text.splitlines():
        if header_marker in line:
            in_table = True
            continue
        if in_table:
            m = re.match(r"^\|\s*([a-z0-9_]+)\s*\|(.+)\|\s*$", line)
            if m:
                rows[m.group(1)] = [c.strip() for c in m.group(2).split("|")]
            elif line.startswith("|") is False and rows:
                break
    return rows


def check(fail):
    if not os.path.exists(DOC):
        return fail("BARS.md missing")
    if not os.path.exists(JSN):
        return fail("bars.json missing")
    doc = open(DOC).read()
    data = json.load(open(JSN))

    # 1. numbers
    exp_suites, exp_scaling = expected_rows(data)
    got_suites = parse_doc_rows(doc, "workload | HEAD ms")
    got_scaling = parse_doc_rows(doc, "workload | HEAD t(2N)")
    for row in exp_suites:
        name = row.split("|")[1].strip()
        if got_suites.get(name) != [c.strip() for c in row.split("|")[2:-1]]:
            fail(f"suite row drift for {name}:\n  doc: {got_suites.get(name)}\n"
                 f"  json: {row}")
    for row in exp_scaling:
        name = row.split("|")[1].strip()
        if got_scaling.get(name) != [c.strip() for c in row.split("|")[2:-1]]:
            fail(f"scaling row drift for {name}:\n  doc: {got_scaling.get(name)}\n"
                 f"  json: {row}")

    # 2. pins
    for label, want in (
            ("HEAD pin", data["meta"]["head"]["source"]),
            ("Bar A pin", data["meta"]["bara"].get("source", "")),
            ("Bar A tarball sha256", data["meta"]["bara"].get("tarball_sha256", ""))):
        if want and want not in doc:
            fail(f"{label} `{want}` not pinned in BARS.md")
    tarball = data["meta"]["bara"].get("asset")
    tar_sha = data["meta"]["bara"].get("tarball_sha256")
    cache = os.path.join(os.environ.get("OPERON_BARS_CACHE", "/tmp/operon-bars-cache"),
                         tarball or "")
    sums = os.path.join(os.path.dirname(cache), "SHA256SUMS")
    if tarball and os.path.exists(cache) and os.path.exists(sums):
        line = [l.split()[0] for l in open(sums) if l.split()[1:2] == [tarball]]
        if line and line[0] != tar_sha:
            fail(f"cached tarball sha256 {sha256(cache)} != release SHA256SUMS "
                 f"{line[0]} — the Bar A chain is broken")

    # 3. refusals agree
    for name in REFUSALS:
        r = data["suites"].get(name, {})
        doc_refused = re.search(rf"\|\s*{name}\s*\|.*\|\s*refused\s*\|", doc)
        json_refused = r.get("barc", {}).get("refused", False)
        if bool(doc_refused) != json_refused:
            fail(f"refusal mismatch on {name}: doc={bool(doc_refused)} json={json_refused}")

    # 4. recipe paths exist
    for p in ("scripts/bench/bars.py", "scripts/bench/bars_check.py",
              "docs/perf/bars.json", "scripts/build.sh"):
        if not os.path.exists(os.path.join(ROOT, p)):
            fail(f"recipe path missing: {p}")
    for name in data["suites"]:
        if name in REFUSALS:
            continue
        twin = {"fib25": "call_fib_rs.rs"}.get(name, f"{name}_rs.rs")
        if not os.path.exists(os.path.join(ROOT, "scripts", "bench", twin)):
            fail(f"Bar C twin missing for {name}: {twin}")


def main():
    ap = argparse.ArgumentParser(description="BARS.md pointer audit")
    ap.add_argument("--print", action="store_true", help="emit expected rows")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.print:
        data = json.load(open(JSN))
        exp_suites, exp_scaling = expected_rows(data)
        print("\n".join(exp_suites))
        print("--")
        print("\n".join(exp_scaling))
        return

    fails = []
    if args.self_test:
        # mutate the JSON copy -> the checker must notice
        import shutil, tempfile
        tmp = tempfile.mkdtemp()
        shutil.copy(DOC, tmp)
        data = json.load(open(JSN))
        data["suites"]["loops"]["head"]["min"] *= 1.5
        bad = os.path.join(tmp, "bars.json")
        json.dump(data, open(bad, "w"))
        orig = JSN
        globals()["JSN"] = bad
        check(lambda msg: fails.append(msg))
        globals()["JSN"] = orig
        print("self-test:", "PASS (drift caught)" if fails else
              "FAIL (checker missed the mutation)")
        sys.exit(0 if fails else 1)

    check(lambda msg: fails.append(msg))
    if fails:
        print(f"bars_check: {len(fails)} finding(s)")
        for f in fails:
            print(" -", f)
        sys.exit(1)
    print("bars_check: OK — every BARS.md number re-derives from bars.json, "
          "pins intact, refusals agree, recipe paths exist")


if __name__ == "__main__":
    main()
