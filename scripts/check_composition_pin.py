#!/usr/bin/env python3
"""check_composition_pin.py — W061-D: the README measured-composition table
can never silently contradict the tree it claims to measure.

Twice in one day (W061-B follow-up @ 0f8615b, then W061-C @ fa88e20) the
hand-pinned "Measured composition" table in README.md went stale because a
merge landed between the measurement and the merge — the second occurrence
came within hours of the first re-pin. The table header names the tool
(`bash scripts/stack_report.sh`) and pins a commit, but nothing CHECKED the
numbers. This script is that check: it re-derives stack_report.sh at the
current HEAD and diffs it against the table, so the next drift is a one-line
command, not a re-discovered audit finding.

Checks (all static, seconds — stack_report.sh is find+wc):
  1. Numbers: every language row's line count must match the tool output at
     the current tree (the Operon row = the tool's printed total .op line).
  2. Rank order: rows must be sorted by lines descending (rank 1..N).
  3. Modules count: the Operon role text's "(N modules" must equal the
     std/*.op file count (the W061-B R2 class — role text said 30, truth 31).
  4. Share bands: each row's stated share ("~85%", "~0.5%", "<1%") must
     match the recomputed share at the stated precision.
  5. Header pin: `main @ <sha>` — FAIL if the sha is unknown to git or not
     an ancestor of HEAD; WARN (exit 0) if it trails HEAD while every
     number is checked against the current tree anyway (the benign
     post-docs-only-landing state; the W061-B lesson keeps the refresh at
     the next merge step).

Deliberately NOT done: wiring into scripts/test.sh. A hard gate would turn
every other lane's code landing red until someone re-pins README — that is
a merge-flow policy change, which belongs to the owner (the S3
flagged-not-forced precedent). Until ratified, run this per W061 audit and
before merging anything that moves the measured trees. There is also no
--fix: the table sits inside hand-curated prose, so repairs stay manual —
but `--print` emits the exact expected rows, making a re-pin a copy-paste.

Run:  python3 scripts/check_composition_pin.py            (exit 1 on drift)
      python3 scripts/check_composition_pin.py --print    (expected rows)
      python3 scripts/check_composition_pin.py --self-test
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# stack_report.sh language key -> the README row name it feeds. The tool
# splits .op by directory (std/tests/examples/apps/tools); the README's
# single Operon row carries the tool's printed total line.
README_ROW_FOR_LANG = {
    "Rust": "Rust",
    "C++": "C++",
    "Python": "Python",
    "Shell": "Shell",
    "JS": "JavaScript",
    "HTML": "HTML",
    "CSS": "CSS",
    "TS": "TypeScript",
}
LANG_FOR_README_ROW = {v: k for k, v in README_ROW_FOR_LANG.items()}

# The tool splits .op by directory; every OperonN key is already included in
# its printed total — the grand total must count .op exactly once.
OP_KEYS = {"Operon", "Operon2", "Operon3", "Operon4", "Operon5"}


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def stack_report():
    """Run the header-named tool; return ({lang: lines}, total_op_lines)."""
    p = run(["bash", os.path.join("scripts", "stack_report.sh")], cwd=ROOT)
    if p.returncode != 0:
        raise RuntimeError("stack_report.sh failed: " + p.stderr.strip())
    counts, total = {}, None
    for line in p.stdout.splitlines():
        m = re.match(r"^total Operon \(\.op\) lines: (\d+)$", line.strip())
        if m:
            total = int(m.group(1))
            continue
        parts = line.split()
        if len(parts) == 3 and parts[0] != "language" and parts[1].isdigit():
            counts[parts[0]] = int(parts[1])
    if total is None:
        raise RuntimeError("stack_report.sh output missing the total line")
    return counts, total


def std_module_count():
    p = run(["bash", "-c", "find std -name '*.op' -type f | wc -l"], cwd=ROOT)
    return int(p.stdout.strip())


def git_sha_known(sha):
    return run(["git", "cat-file", "-e", sha + "^{commit}"],
               cwd=ROOT).returncode == 0


def git_sha_ancestor_of_head(sha):
    return run(["git", "merge-base", "--is-ancestor", sha, "HEAD"],
               cwd=ROOT).returncode == 0


def git_commits_behind(sha):
    p = run(["git", "rev-list", "--count", sha + "..HEAD"], cwd=ROOT)
    return int(p.stdout.strip()) if p.returncode == 0 and p.stdout.strip() else 0


def parse_readme(readme_path):
    """Extract (pin_sha, [(rank, lang, lines, share)], modules_in_role_text).

    Returns None if the section is missing or structurally drifted.
    """
    text = open(readme_path, encoding="utf-8").read()
    header = re.search(r"^### Measured composition \(main @ ([0-9a-f]{7,40}),",
                       text, re.M)
    if not header:
        return None
    tbl = re.search(
        r"^\| rank \| language \| lines \| share \| role \|\n"
        r"\|[-| ]+\n((?:\|.*\n)+)", text, re.M)
    if not tbl:
        return None
    rows, modules = [], None
    for line in tbl.group(1).splitlines():
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) < 5:
            continue
        rank = int(cells[0])
        lang = cells[1].replace("**", "")
        lines = int(cells[2].replace(",", ""))
        share = cells[3]
        mm = re.search(r"\((\d+) modules", cells[4])
        if lang == "Operon" and mm:
            modules = int(mm.group(1))
        rows.append((rank, lang, lines, share))
    return header.group(1), rows, modules


def share_matches(stated, actual_pct):
    """True if the stated share band matches the recomputed percent."""
    s = stated.strip()
    if s.startswith("<"):
        return actual_pct < float(s[1:].rstrip("%"))
    if s.startswith("~"):
        body = s[1:].rstrip("%")
        if "." in body:                    # one-decimal band, e.g. ~0.5%
            return round(actual_pct, len(body.split(".")[1])) == float(body)
        return round(actual_pct) == int(body)
    return False


def check(readme_path, out=sys.stderr):
    """Run every check; return 0 green, 1 drift."""
    counts, total_op = stack_report()
    std_mods = std_module_count()
    parsed = parse_readme(readme_path)
    failures, warns = [], []

    if parsed is None:
        print("composition-pin: FAIL — 'Measured composition' section not "
              "found or structurally drifted in %s" % readme_path, file=out)
        return 1
    pin, rows, modules = parsed

    # Grand total: non-.op languages summed once + .op total (once).
    grand = sum(n for l, n in counts.items() if l not in OP_KEYS) + total_op

    prev_lines = None
    for rank, lang, lines, share in rows:
        if lang == "Operon":
            expected = total_op
        else:
            key = LANG_FOR_README_ROW.get(lang)
            expected = counts.get(key) if key else None
        if expected is None:
            failures.append("row %d (%s): language not produced by "
                            "stack_report.sh" % (rank, lang))
            continue
        if lines != expected:
            failures.append("row %d (%s): README says %s lines, tree "
                            "measures %d" % (rank, lang,
                                             format(lines, ","), expected))
        actual_share = 100.0 * expected / grand
        if not share_matches(share, actual_share):
            failures.append("row %d (%s): share '%s' vs recomputed %.2f%%"
                            % (rank, lang, share, actual_share))
        if prev_lines is not None and lines > prev_lines:
            failures.append("rank order broken at row %d (%s): %s after %s"
                            % (rank, lang, format(lines, ","),
                               format(prev_lines, ",")))
        prev_lines = lines

    if modules is None:
        failures.append("Operon role text: '(N modules' count not found")
    elif modules != std_mods:
        failures.append("Operon role text says %d modules, std/ holds %d "
                        ".op files" % (modules, std_mods))

    if not git_sha_known(pin):
        failures.append("header pin @ %s is not a commit known to git" % pin)
    elif not git_sha_ancestor_of_head(pin):
        failures.append("header pin @ %s is not an ancestor of HEAD "
                        "(rebased away?)" % pin)
    else:
        behind = git_commits_behind(pin)
        if behind > 0:
            warns.append("header pin @ %s trails HEAD by %d commit(s); all "
                         "numbers were checked against the CURRENT tree — "
                         "refresh the pin at the next merge step (W061-B/C "
                         "lesson)" % (pin, behind))

    for w in warns:
        print("composition-pin: WARN — " + w, file=out)
    if failures:
        for f in failures:
            print("composition-pin: FAIL — " + f, file=out)
        print("composition-pin: %d finding(s). Re-pin with the header-named "
              "tool (`bash scripts/stack_report.sh`) and update the README "
              "table — `--print` emits the expected rows." % len(failures),
              file=out)
        return 1
    print("composition pin OK: %d rows true at the current tree (.op total "
          "%s, %d std modules)" % (len(rows), format(total_op, ","),
                                   std_mods), file=out)
    return 0


def print_expected(out=sys.stdout):
    """Emit the rows the README table should carry at the current tree."""
    counts, total_op = stack_report()
    std_mods = std_module_count()
    grand = sum(n for l, n in counts.items() if l not in OP_KEYS) + total_op
    entries = [("Operon", total_op)] + sorted(
        ((README_ROW_FOR_LANG[l], n) for l, n in counts.items()
         if l in README_ROW_FOR_LANG),
        key=lambda e: -e[1])
    for rank, (lang, lines) in enumerate(entries, 1):
        pct = 100.0 * lines / grand
        if pct < 1.0:
            share = "<1%"
        elif pct < 10.0:
            share = "~%.1f%%" % round(pct, 1)
        else:
            share = "~%d%%" % round(pct)
        print("| %d | **%s** | %s | %s |" %
              (rank, lang, format(lines, ","), share))
    print("# Operon role text modules count: %d" % std_mods, file=out)


def self_test(out=sys.stderr):
    """Hermetic negatives: one real drift at a time on a README copy.

    Anchors derive from the LIVE table (parse_readme), never hardcoded, so
    the probes survive legitimate re-pins of the true values.
    """
    real = os.path.join(ROOT, "README.md")
    text = open(real, encoding="utf-8").read()
    parsed = parse_readme(real)
    if parsed is None:
        print("self-test FAIL: could not parse the live README table",
              file=out)
        return 1
    pin, rows, modules = parsed
    counts, total_op = stack_report()
    std_mods = std_module_count()
    tmp = tempfile.mkdtemp()

    def write_variant(name, mutate):
        path = os.path.join(tmp, "README-%s.md" % name)
        open(path, "w", encoding="utf-8").write(mutate(text))
        return path

    rank1, lang1, lines1, share1 = rows[0]
    row1_pat = re.compile(
        r"^\| 1 \| \*\*%s\*\* \| [\d,]+ \| (?:~[\d.]+%%?|<1%%) \|" % lang1,
        re.M)

    probes = []

    # 1. number drift: rank-1 line count +1 (also breaks the share + rank).
    probes.append(("number drift", lambda t: row1_pat.sub(
        "| 1 | **%s** | %s | %s |" %
        (lang1, format(lines1 + 1, ","), share1), t, count=1)))

    # 2. share drift: stated band +1 when integer ~N%, else <1% -> ~5%.
    if share1.startswith("~") and "." not in share1:
        wrong = "~%d%%" % (int(share1.strip("~%")) + 1)
    elif share1 == "<1%":
        wrong = "~5%"
    else:
        wrong = None
    if wrong:
        probes.append(("share drift", lambda t: t.replace(
            "| %s |" % share1, "| %s |" % wrong, 1)))

    # 3. module drift: role-text count +1 (the W061-B R2 class).
    if modules is not None:
        probes.append(("module drift", lambda t: t.replace(
            "(%d modules" % modules, "(%d modules" % (modules + 1), 1)))

    # 4. garbage pin: a sha no git knows.
    probes.append(("garbage pin", lambda t: re.sub(
        r"### Measured composition \(main @ [0-9a-f]{7,40},",
        "### Measured composition (main @ deadbeef00,", t, count=1)))

    passed = 0
    for name, mutate in probes:
        rc = check(write_variant(name, mutate), out=out)
        if rc == 1:
            passed += 1
            print("self-test ok: %s detected" % name, file=out)
        else:
            print("self-test FAIL: %s NOT detected (exit %d)" % (name, rc),
                  file=out)
            return 1
    print("self-test: %d/%d negatives caught" % (passed, len(probes)),
          file=out)
    return 0


def main():
    ap = argparse.ArgumentParser(
        description="Check the README measured-composition table against "
                    "stack_report.sh at the current tree (W061-D).")
    ap.add_argument("--readme", default=os.path.join(ROOT, "README.md"),
                    help="README path to check (default: repo README.md)")
    ap.add_argument("--print", dest="print_rows", action="store_true",
                    help="print the expected table rows at the current tree")
    ap.add_argument("--self-test", action="store_true",
                    help="run the hermetic negative probes and exit")
    args = ap.parse_args()
    if args.self_test:
        sys.exit(self_test())
    if args.print_rows:
        print_expected()
        sys.exit(0)
    sys.exit(check(args.readme))


if __name__ == "__main__":
    main()
