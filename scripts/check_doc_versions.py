#!/usr/bin/env python3
"""check_doc_versions.py — S6: the docs/ HTML site can never drift from truth.

The four HTML pages (docs/index.html, tour.html, regulation.html, grammar.html)
predate v2.3 and had quietly fossilized: stale version banners, a build path
that no longer exists, a C-runtime row for the kernel that A15 ported to Rust,
a 51-keyword list against the generated 60-keyword truth, a v2.1-era builtins
table. Markdown docs have had W53/W54/W55 gates (check_docs_sync.py) since the
W5x wave — the HTML site had nothing. This script is that gate.

Checks (all static, seconds):
  1. Version strings: every version-like token in docs/*.html must be the
     current Cargo.toml version. A version the pages may legitimately cite
     lives in ALLOWED_OTHER_VERSIONS with a why (deny-by-default, D-005
     discipline for text).
  2. Keyword truth: grammar.html's "Canonical keywords (N)" count must equal
     the GENERATED docs/KEYWORDS.md count, and every generated keyword must
     appear in grammar.html as a standalone token (the list is kept true or
     the gate says which names are missing).
  3. Stale-claim denylist: (pattern, why) pairs — the same shape as
     check_docs_sync.py's FORBIDDEN table. Each entry here is a claim that
     was true once and rotted; the fix is the source, never the gate.

Run:  python3 scripts/check_doc_versions.py   (from repo root; exit 1 on drift)
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

HTML_DIR = os.path.join(ROOT, "docs")

def strip_tags(html):
    """Visible text only — so markup like <span class="p">./</span>build
    cannot hide a fossil claim from the denylist."""
    return re.sub(r"<[^>]+>", "", html)

def cargo_version():
    toml = open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read()
    m = re.search(r'^version\s*=\s*"([^"]+)"', toml, re.M)
    if not m:
        raise SystemExit("check_doc_versions: Cargo.toml version not found")
    return m.group(1)

# Version-like tokens the pages may legitimately carry that are NOT the
# current release version. Each with a why — deny-by-default.
ALLOWED_OTHER_VERSIONS = {
    # (none today — add with a reason if a page ever needs one)
}

# (pattern, why) — claims that rotted; fix the source, never hand-silence.
# The pattern set grows whenever a fossil is found; each row names the truth.
FORBIDDEN = [
    (r"\./build/operon",
     "the build produces bin/operon (scripts/build.sh: 'OK: bin/operon'); "
     "there is no build/operon"),
    (r"SPEC v2\.1\b|SPEC \(v2\.1\)",
     "SPEC is version-locked to Cargo.toml (W54); v2.1 references are fossils"),
    (r"200,000,000|200M",
     "the step budget is the 500M run-wide pool (SPEC §17 amendment 4 + "
     "main.rs); the 200M default is a pre-amendment-4 fossil"),
    (r"OS thread tasks",
     "spawn/join run fibers over the VM loop since W016 (deterministic FIFO "
     "scheduler, virtual clock); 'OS thread tasks' is the pre-W016 truth"),
    (r"arena_bytes, interns, allocs</code> from the runtime core",
     "memory() accounting is the symbol-table gauge pinned in SPEC §10; the "
     "'runtime core' wording predates the A15 port"),
    (r"\d+ files? / \d+ proofs? / \d+ assertions?",
     "hand-typed suite counts rot on every landing (the W53 lesson): link "
     "docs/STATS.md instead — counts are generated, never hand-typed"),
]

def html_files():
    out = []
    for f in sorted(os.listdir(HTML_DIR)):
        if f.endswith(".html"):
            out.append(os.path.join(HTML_DIR, f))
    return out

def keywords_truth():
    """(count, names) from the GENERATED docs/KEYWORDS.md — never hand-edit."""
    md = open(os.path.join(ROOT, "docs", "KEYWORDS.md"), encoding="utf-8").read()
    m = re.search(r"Count: \*\*(\d+)\*\*", md)
    names = re.findall(r"^\| `([a-z0-9]+)` \|", md, re.M)
    if not m or not names:
        raise SystemExit("check_doc_versions: docs/KEYWORDS.md unreadable — "
                         "regenerate with scripts/gen_doc_stats.py first")
    return int(m.group(1)), names

def builtins_truth():
    """BUILTIN_NAMES from src/interp.rs — same extraction contract as
    bootstrap/audit_builtins.py (S3). Duplicated here DELIBERATELY: scripts/
    must not import bootstrap/ (layer boundary), and the extraction is one
    regex over a published const. If the const moves, both gates say so."""
    src = open(os.path.join(ROOT, "src", "interp.rs"), encoding="utf-8").read()
    i = src.index("pub const BUILTIN_NAMES")
    j = src.index("];", i)
    return sorted(set(re.findall(r'"([a-z_0-9]+)"', src[i:j])))

def main():
    version = cargo_version()
    files = html_files()
    if not files:
        print("check_doc_versions: no docs/*.html found — nothing to guard")
        return

    problems = []

    # 1. version strings (per occurrence, not per token)
    ver_re = re.compile(r"\b(\d+\.\d+\.\d+)\b")
    for path in files:
        rel = os.path.relpath(path, ROOT)
        text = open(path, encoding="utf-8", errors="replace").read()
        for m in ver_re.finditer(text):
            tok = m.group(1)
            if tok == version or tok in ALLOWED_OTHER_VERSIONS:
                continue
            line = text[:m.start()].count("\n") + 1
            key = (rel, line, tok)
            if key not in [(p[0], p[1], p[2]) for p in problems]:
                problems.append(
                    "%s:%d: version token %s is not the current %s and is "
                    "not allow-listed (fix the page or add an explicit why "
                    "to ALLOWED_OTHER_VERSIONS)" % (rel, line, tok, version))

    # 2. keyword truth (grammar.html)
    count, names = keywords_truth()
    gram_path = os.path.join(HTML_DIR, "grammar.html")
    if os.path.isfile(gram_path):
        gram = open(gram_path, encoding="utf-8").read()
        m = re.search(r"Canonical keywords \((\d+)\)", gram)
        if not m:
            problems.append("docs/grammar.html: the 'Canonical keywords (N)' "
                            "line is missing — the keyword-count gate has "
                            "nothing to check")
        elif int(m.group(1)) != count:
            problems.append(
                "docs/grammar.html: claims %s canonical keywords, generated "
                "truth (docs/KEYWORDS.md) is %d — sync the list"
                % (m.group(1), count))
        missing = [k for k in names
                   if not re.search(r"\b%s\b" % re.escape(k), gram)]
        if missing:
            problems.append(
                "docs/grammar.html: missing keyword(s) from the generated "
                "set: %s" % ", ".join(missing))
        bn = builtins_truth()
        bmissing = [b for b in bn
                    if not re.search(r"\b%s\b" % re.escape(b), gram)]
        if bmissing:
            problems.append(
                "docs/grammar.html: builtin(s) missing from the page (the "
                "registry has %d, every one must be documented): %s"
                % (len(bn), ", ".join(bmissing)))

    # 3. stale-claim denylist (on VISIBLE text — tags stripped)
    for path in files:
        rel = os.path.relpath(path, ROOT)
        raw = open(path, encoding="utf-8", errors="replace").read()
        text = strip_tags(raw)
        for pat, why in FORBIDDEN:
            m = re.search(pat, text)
            if m:
                line = text[:m.start()].count("\n") + 1
                problems.append("%s:~%d: stale claim %r — %s"
                                % (rel, line, m.group(0)[:60], why))

    if problems:
        print("DOC VERSIONS OUT OF SYNC, fix the source, never hand-silence "
              "the gate:")
        for p in problems:
            print("  ✗ " + p)
        sys.exit(1)
    print("doc versions OK: %d html page(s) clean against %s, keyword set "
          "(%d) and claim denylist green" % (len(files), version, count))


if __name__ == "__main__":
    main()
