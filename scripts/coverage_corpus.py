#!/usr/bin/env python3
"""coverage_corpus.py, W052: std call-site coverage of the .op proof corpus.

Layer 2 of scripts/coverage.sh. Stdlib only, works offline. Layer 1 (Rust
line/region coverage via cargo-llvm-cov) is driven by scripts/coverage.sh,
which stores its summary table in target/llvm-cov-summary.txt; when that
file is present it is embedded in docs/coverage.md, otherwise layer 1 is
recorded as unavailable. Nothing is ever invented.

What is measured here:

  static set   top-level `gene name(` definitions in std/*.op. This reuses
               the regex strategy of scripts/gen_doc_stats.py (copied, not
               imported, so both tools stay independently runnable).

  dynamic set  std call sites in the proof corpus: tests/*.op,
               tests/differential/*.op, tests/granted/*.op. Per file, the
               `use std/<module> [as <alias>]` lines say which std modules
               the file pulls; a module's gene name counts as referenced
               when it appears in a call position in that file's body.

This is CALL-SITE COVERAGE BY STATIC ANALYSIS of the corpus. It is not
execution coverage. Exact matching rules:

  1. comments and string-literal contents are stripped before matching;
  2. a qualified call `ns.name(` counts for the module bound to namespace
     `ns` by one of the file's own use lines;
  3. a bare call `name(` counts for an imported module that defines that
     name, unless the file itself defines a gene of the same name (the
     file's own top-level gene defs are subtracted);
  4. a method-style `.name(` never matches the bare rule.

Known limits, stated so nobody over-reads the number:

  - a call inside an untaken branch still counts (mentioned-not-called);
  - a dynamically dispatched call (call(), gene values, maps of genes) may
    not count;
  - a bare call can credit a same-named builtin to an imported std module;
  - std/seq exports `sequence` definitions, not genes, and the pinned
    gene-def regex reports 0 functions for it, matching the generated
    stdlib inventory in docs/stats.json.

Deterministic by construction: sorted inputs, sorted outputs, no timestamps,
no environment reads beyond the repo tree. Exit 0 on success.
"""

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
L1_SUMMARY = os.path.join(ROOT, "target", "llvm-cov-summary.txt")
DOC_PATH = os.path.join(ROOT, "docs", "coverage.md")

# Copied strategy from scripts/gen_doc_stats.py std_modules() so the static
# inventory here can be diffed against docs/stats.json line for line.
STD_GENE_RE = re.compile(r'^\s*(?:pub\s+)?gene\s+(\w+)\s*\(', re.M)
USE_RE = re.compile(
    r'^\s*use\s+std[/::]([A-Za-z_]\w*)(?:\s+as\s+([A-Za-z_]\w*))?\s*;?\s*$')
QUAL_CALL_RE = re.compile(
    r'(?<![\w.])([A-Za-z_]\w*)\s*\.\s*([A-Za-z_]\w*)\s*\(')
BARE_CALL_RE = re.compile(r'(?<![\w.])([A-Za-z_]\w*)\s*\(')

CORPUS_GLOBS = ("tests/*.op", "tests/differential/*.op", "tests/granted/*.op")
TOP_UNCOVERED = 20


def read(rel):
    with open(os.path.join(ROOT, rel), encoding="utf-8", errors="replace") as f:
        return f.read()


def strip_comments_and_strings(text):
    """Blank out comment tails and string-literal contents, keep code shape."""
    out = []
    for line in text.splitlines():
        buf = []
        quote = None
        i = 0
        while i < len(line):
            ch = line[i]
            if quote is not None:
                if ch == "\\" and i + 1 < len(line):
                    i += 2
                    continue
                if ch == quote:
                    quote = None
            elif ch in "\"'":
                quote = ch
            elif ch == "#":
                break
            else:
                buf.append(ch)
            i += 1
        out.append("".join(buf))
    return "\n".join(out)


def static_defs():
    """module -> sorted list of top-level gene names (gen_doc_stats pattern)."""
    std_dir = os.path.join(ROOT, "std")
    defs = {}
    for f in sorted(os.listdir(std_dir)):
        if not f.endswith(".op"):
            continue
        names = sorted(set(STD_GENE_RE.findall(read(os.path.join("std", f)))))
        defs[f[:-3]] = names
    return defs


def corpus_files():
    files = set()
    for pat in CORPUS_GLOBS:
        for p in glob.glob(os.path.join(ROOT, pat)):
            files.add(os.path.relpath(p, ROOT))
    return sorted(files)


def file_imports(text):
    """Ordered unique (namespace, module) pairs from the file's use lines."""
    imports = []
    seen = set()
    for line in text.splitlines():
        m = USE_RE.match(line)
        if not m:
            continue
        mod = m.group(1)
        ns = m.group(2) or mod
        if (ns, mod) not in seen:
            seen.add((ns, mod))
            imports.append((ns, mod))
    return imports


def scan_file(rel, defs):
    """(module -> referenced gene names, module -> qualified-only subset)."""
    text = strip_comments_and_strings(read(rel))
    imports = file_imports(text)
    if not imports:
        return {}, {}
    local_defs = set(STD_GENE_RE.findall(text))
    qual = set(QUAL_CALL_RE.findall(text))
    bare = set(BARE_CALL_RE.findall(text))
    ref = {}
    ref_q = {}
    for ns, mod in imports:
        names = defs.get(mod)
        if names is None:
            continue
        q_hits = {n for n in names if (ns, n) in qual}
        b_hits = {n for n in names
                  if n in bare and n not in local_defs} | q_hits
        if b_hits:
            ref[mod] = b_hits
        if q_hits:
            ref_q[mod] = q_hits
    return ref, ref_q


def compute():
    defs = static_defs()
    files = corpus_files()
    referenced = {m: set() for m in defs}
    qualified_only = {m: set() for m in defs}
    files_with_imports = 0
    for rel in files:
        ref, ref_q = scan_file(rel, defs)
        if ref:
            files_with_imports += 1
        for mod, names in ref.items():
            if mod in referenced:
                referenced[mod] |= names
        for mod, names in ref_q.items():
            if mod in qualified_only:
                qualified_only[mod] |= names
    rows = []
    for mod in sorted(defs):
        names = defs[mod]
        hit = sorted(referenced[mod])
        miss = sorted(set(names) - set(hit))
        rows.append({"module": mod, "defined": len(names),
                     "referenced": len(hit), "names": hit, "uncovered": miss})
    defined_total = sum(r["defined"] for r in rows)
    ref_total = sum(r["referenced"] for r in rows)
    pct = (100.0 * ref_total / defined_total) if defined_total else 0.0
    q_total = sum(len(qualified_only[m]) for m in qualified_only)
    q_pct = (100.0 * q_total / defined_total) if defined_total else 0.0
    return {"rows": rows, "files": files, "files_with_imports": files_with_imports,
            "modules": len(rows), "defined_total": defined_total,
            "ref_total": ref_total, "pct": pct, "q_total": q_total,
            "q_pct": q_pct}


def l1_summary_text():
    """Verbatim llvm-cov table if scripts/coverage.sh captured one, else None."""
    try:
        with open(L1_SUMMARY, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return None
    return text if text.strip() else None


def render_rows(rows, indent=""):
    """Shared table body for stdout and the doc."""
    L = []
    for r in rows:
        names = ", ".join(r["names"]) if r["names"] else "(none)"
        if not r["defined"]:
            names = "(no top-level gene defs; sequence exports)"
        L.append("%sstd/%-14s %3d  %4d  %s" % (indent, r["module"], r["defined"],
                                               r["referenced"], names))
    return L


def render_stdout(s):
    L = []
    L.append("== std call-site coverage of the .op proof corpus ==")
    L.append("scope: %s (%d files, %d importing std)"
             % (", ".join(CORPUS_GLOBS), len(s["files"]), s["files_with_imports"]))
    L.append("static set: top-level gene defs in std/*.op (gen_doc_stats pattern)")
    L.append("this is call-site coverage by static analysis, not execution coverage")
    L.append("")
    L.append("module             def   ref  referenced names")
    L.extend(render_rows(s["rows"]))
    L.append("")
    L.append("TOTAL              %3d  %4d  %.1f%% call-site coverage over %d modules"
             % (s["defined_total"], s["ref_total"], s["pct"], s["modules"]))
    uncovered = [(r["module"], n) for r in s["rows"] for n in r["uncovered"]]
    private = [(m, n) for (m, n) in uncovered if n.startswith("__")]
    L.append("uncovered: %d std functions (%d public, %d std-internal __ helpers)"
             % (len(uncovered), len(uncovered) - len(private), len(private)))
    if s["q_total"] < s["ref_total"]:
        L.append("qualified-namespace calls alone reach %d of %d (%.1f%%); the bare-name"
                 % (s["q_total"], s["defined_total"], s["q_pct"]))
        L.append("rule adds the other %d and is the disclosed over-count surface"
                 % (s["ref_total"] - s["q_total"]))
    else:
        L.append("qualified-namespace calls alone reach %d of %d (%.1f%%); the bare-name"
                 % (s["q_total"], s["defined_total"], s["q_pct"]))
        L.append("rule adds nothing today, it stays in as the disclosed over-count surface")
    return "\n".join(L)


def render_doc(s):
    L = []
    L.append("# Coverage baseline (GENERATED, do not hand-edit)")
    L.append("")
    L.append("Regenerated by `scripts/coverage.sh` (both layers) or by")
    L.append("`python3 scripts/coverage_corpus.py` (layer 2 and this file). These")
    L.append("numbers are a tracked baseline only: no gate depends on them, the")
    L.append("same rule the check score follows. The baseline exists to be beaten")
    L.append("honestly, not defended.")
    L.append("")
    L.append("## Layer 1: Rust line/region coverage (cargo llvm-cov)")
    L.append("")
    table = l1_summary_text()
    if table is not None:
        L.append("Status: measured. `scripts/coverage.sh` runs `cargo llvm-cov")
        L.append("--summary-only --ignore-filename-regex")
        L.append("'(target|tests|examples|std|bootstrap)/'` over every cargo test")
        L.append("target (lib unit tests plus the tests/*.rs integration tests).")
        L.append("Test harness sources and the interpreted .op tree are excluded")
        L.append("from instrumentation by the ignore filter; .op-level")
        L.append("reachability is layer 2. The TOTAL row is the headline.")
        L.append("")
        L.append("```")
        L.append(table.rstrip())
        L.append("```")
    else:
        L.append("Status: unavailable. `cargo-llvm-cov` was not installed or the")
        L.append("run failed in the environment that last generated this file, and")
        L.append("no number was invented in its place. Install with")
        L.append("`cargo install cargo-llvm-cov && rustup component add")
        L.append("llvm-tools-preview`, then re-run `scripts/coverage.sh`.")
    L.append("")
    L.append("## Layer 2: std call-site coverage of the .op proof corpus")
    L.append("")
    L.append("Status: measured. Static analysis of the corpus, not execution")
    L.append("coverage; the exact rules and limits are in the methodology below.")
    L.append("Scope: %s (%d files scanned, %d of them import std)."
             % (", ".join("`%s`" % g for g in CORPUS_GLOBS),
                len(s["files"]), s["files_with_imports"]))
    L.append("")
    L.append("| module | defined | referenced | referenced names |")
    L.append("|---|---|---|---|")
    for r in s["rows"]:
        if not r["defined"]:
            names = "(no top-level gene defs; sequence exports)"
        elif r["names"]:
            names = ", ".join("`%s`" % n for n in r["names"])
        else:
            names = "(none)"
        L.append("| std/%s | %d | %d | %s |" % (r["module"], r["defined"],
                                                r["referenced"], names))
    L.append("")
    uncovered = [(r["module"], n) for r in s["rows"] for n in r["uncovered"]]
    L.append("Total: **%d of %d** defined std functions referenced by the corpus "
             "(**%.1f%%** call-site coverage) across %d modules; %d functions "
             "uncovered. Qualified-namespace calls alone reach %d of %d "
             "(%.1f%%)%s"
             % (s["ref_total"], s["defined_total"], s["pct"], s["modules"],
                len(uncovered), s["q_total"], s["defined_total"], s["q_pct"],
                (", the bare-name rule adds the other %d and is the disclosed "
                 "over-count surface." % (s["ref_total"] - s["q_total"]))
                if s["q_total"] < s["ref_total"] else
                "; the bare-name rule adds nothing today, it stays in as the "
                "disclosed over-count surface."))
    L.append("")
    L.append("### Methodology (call-site coverage, static analysis of the corpus)")
    L.append("")
    L.append("- Static set: top-level `gene name(...)` definitions in std/*.op,")
    L.append("  found with the same regex strategy as the std module inventory in")
    L.append("  `scripts/gen_doc_stats.py` (pattern copied, not imported). The two")
    L.append("  inventories are expected to agree; docs/stats.json is the cross-check.")
    L.append("- Dynamic set: per corpus file, the `use std/<module> [as <alias>]`")
    L.append("  lines are read to learn which std modules that file pulls and under")
    L.append("  which namespace (`alias` if present, else the module name). A module's")
    L.append("  gene name is referenced when it appears in a call position in that")
    L.append("  file's body, as `ns.name(` or as a bare `name(`.")
    L.append("- Matching rules: comments and string-literal contents are stripped")
    L.append("  first; qualified calls count for the module bound to that namespace;")
    L.append("  bare calls count for an imported module defining the name, unless")
    L.append("  the file itself defines a gene of the same name; method-style")
    L.append("  `.name(` never matches the bare rule.")
    L.append("- Limits, stated so nobody over-reads the number: a call inside an")
    L.append("  untaken branch still counts (mentioned-not-called); a dynamically")
    L.append("  dispatched call (call(), gene values, maps of genes) may not count;")
    L.append("  a bare call can credit a same-named builtin to an imported std")
    L.append("  module; std/seq exports `sequence` definitions, not genes, and the")
    L.append("  pinned gene-def regex reports 0 functions for it, matching the")
    L.append("  generated inventory in docs/stats.json.")
    L.append("- Determinism: sorted inputs and outputs, no timestamps, no")
    L.append("  environment reads beyond the repo tree; two runs over the same")
    L.append("  tree produce byte-identical output.")
    L.append("- Reading the number: the qualified-namespace total is the hard floor,")
    L.append("  the combined total is the ceiling; the truth sits between them and")
    L.append("  this tool cannot narrow it further without executing the corpus.")
    L.append("")
    L.append("## Top-%d uncovered std functions (next targets, pick from evidence)"
             % TOP_UNCOVERED)
    L.append("")
    if not uncovered:
        L.append("Nothing is uncovered; the corpus reaches every measured std")
        L.append("function. Treat that as suspicious and re-check the matching rules")
        L.append("before celebrating.")
    else:
        public = [(m, n) for (m, n) in uncovered if not n.startswith("__")]
        private = [(m, n) for (m, n) in uncovered if n.startswith("__")]
        if not public:
            L.append("Every uncovered name is a std-internal __ helper; there is no")
            L.append("uncovered public surface to target.")
        else:
            if len(public) < TOP_UNCOVERED:
                L.append("The uncovered public surface is smaller than %d; all %d "
                         "are listed." % (TOP_UNCOVERED, len(public)))
                L.append("")
            shown = public[:TOP_UNCOVERED]
            L.append("Public names only, ranked by module, then name; the full "
                     "uncovered set is %d" % len(uncovered))
            L.append("functions.")
            L.append("")
            L.append("| # | module | function |")
            L.append("|---|---|---|")
            for i, (mod, name) in enumerate(shown, 1):
                L.append("| %d | std/%s | `%s` |" % (i, mod, name))
        if private:
            L.append("")
            L.append("Std-internal helpers (the __ prefix) are %d further names the "
                     "corpus never" % len(private))
            L.append("reaches; the corpus cannot call them directly by design, so "
                     "they are not")
            L.append("targets, and this is not a dead-code claim: %s."
                     % ", ".join("`%s`" % n for _m, n in private))
    return "\n".join(L) + "\n"


def main():
    s = compute()
    print(render_stdout(s))
    os.makedirs(os.path.dirname(DOC_PATH), exist_ok=True)
    with open(DOC_PATH, "w", encoding="utf-8") as f:
        f.write(render_doc(s))
    print("")
    print("doc: docs/coverage.md (layer 1: %s)"
          % ("measured" if l1_summary_text() is not None else "unavailable"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
