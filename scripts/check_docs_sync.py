#!/usr/bin/env python3
"""check_docs_sync.py — W53/W54/W55/W56/W57/W58: docs can never lie again.

Recomputes the generated statistics (same pure function as gen_doc_stats.py)
and fails on:
  1. docs/stats.json drift vs recomputed reality (stale committed stats)
  2. SPEC Status version != Cargo.toml version (W54)
  3. SPEC H1 title carrying a version number (W54)
  4. hand-typed counts the audit caught: "N proof files", "N programs MATCH",
     "N attacks contained", "canonical, N" keyword counts (W53/W55)
  5. stale architecture claims: "C runtime" anywhere in md/html docs (W56)
  6. dependency overclaim "No crates, no network" (W58)
  7. README stdlib inventory missing a real std module (W57)
Run:  python3 scripts/check_docs_sync.py   (from repo root; exit 1 on drift)
"""
import json, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
from gen_doc_stats import compute  # noqa: E402

DOC_FILES = ["README.md", "SPEC.md", "BENCH.md", "STDLIB.md", "TUTORIAL.md", "CONTRIBUTING.md"]
HTML_DIR = "docs"

FORBIDDEN = [
    # (pattern, why, files)
    (r"\| Mechanism \(real molecular biology\) \| Operon feature \|", "W091: the biology ↔ feature map lives in docs/spec/MODELING-NOTES.md (modeling track), not SPEC",
     ["SPEC.md"]),
    # (pattern, why, files)
    (r"C runtime", "W56: the C runtime kernel was ported to Rust (A15); only the C++ codon kernel exists",
     ["README.md", "TUTORIAL.md", "CONTRIBUTING.md", "*.html"]),
    (r"No crates, no network", "W58: overclaim — Cargo.toml build-depends on cc; network is reachable via py()/run()",
     ["README.md"]),
    (r"canonical,\s*\d+", "W55: keyword count is generated (docs/KEYWORDS.md), never hand-typed",
     ["SPEC.md"]),
    (r"\d+ proof files", "W53: proof counts are generated (docs/STATS.md), never hand-typed",
     ["README.md", "BENCH.md"]),
    (r"\d+/\d+\s*(programs|differential).*MATCH", "W53: differential counts move every session — link docs/STATS.md",
     ["README.md", "BENCH.md"]),
    (r"\d+\s+attacks contained", "W53: red-team counts are generated (docs/STATS.md), never hand-typed",
     ["README.md", "BENCH.md"]),
    (r"proof-frame runner \(\d+ files", "W53: suite counts are generated (docs/STATS.md), never hand-typed",
     ["README.md"]),
    (r"Current suite: \d+ files", "W53: suite counts are generated (docs/STATS.md), never hand-typed",
     ["SPEC.md"]),
    (r"\d+ files / \d+ proofs", "W53: suite counts are generated (docs/STATS.md), never hand-typed",
     ["SPEC.md"]),
    (r"\d+ byte-exact targets", "W53: differential counts are generated (docs/STATS.md), never hand-typed",
     ["SPEC.md"]),
    (r"\d+ programs, all MATCH", "W53: differential counts move every session — link docs/STATS.md",
     ["SPEC.md"]),
    (r"\d+ payloads, \d+ breaches", "W53: red-team counts are generated (docs/STATS.md), never hand-typed",
     ["SPEC.md"]),
]

def files_for(spec):
    out = []
    for s in spec:
        if s == "*.html":
            d = os.path.join(ROOT, HTML_DIR)
            if os.path.isdir(d):
                out += [os.path.join(HTML_DIR, f) for f in sorted(os.listdir(d)) if f.endswith(".html")]
        else:
            p = os.path.join(ROOT, s)
            if os.path.exists(p):
                out.append(p)
    return out

def main():
    fails = []
    truth = compute()

    # 1. committed stats.json must match recomputed truth
    sp = os.path.join(ROOT, "docs", "stats.json")
    if not os.path.exists(sp):
        fails.append("docs/stats.json missing — run scripts/gen_doc_stats.py")
    else:
        committed = json.load(open(sp, encoding="utf-8"))
        for k in ("version", "keyword_count", "std_module_count", "std_function_count",
                  "redteam_files", "proof_files", "test_op_files"):
            if committed.get(k) != truth[k]:
                fails.append(f"stats.json drift on '{k}': committed={committed.get(k)} "
                             f"actual={truth[k]} — regenerate (gen_doc_stats.py)")
        if committed.get("keywords") != truth["keywords"]:
            fails.append("stats.json drift on 'keywords' — regenerate")

    # 2/3. SPEC version honesty (W54) — D-009 semantics:
    #   tagged state:   SPEC Status == Cargo version (exact)
    #   mid-milestone:  SPEC Status = vX.Y.Z-dev while Cargo carries the last tag
    spec_txt = open(os.path.join(ROOT, "SPEC.md"), encoding="utf-8").read()
    st = truth["spec"]["status"]
    ok_dev = re.fullmatch(r"v\d+\.\d+\.\d+-dev", st)
    # normalize the optional v/V prefix so a tagged Status ("v2.2.0") matches
    # Cargo's unprefixed "2.2.0" — the exact-match form could never pass,
    # leaving the tagged-state branch dead code (D-009 needs both forms live)
    ok_tag = (st.lstrip("vV") == truth["version"])
    if not (ok_dev or ok_tag):
        fails.append(f"W54: SPEC Status ('{st}') must be the Cargo version ('{truth['version']}') "
                     f"or a vX.Y.Z-dev milestone label (D-009)")
    if re.search(r"v\d+\.\d+", truth["spec"]["h1"]):
        fails.append(f"W54: SPEC H1 carries a version ('{truth['spec']['h1']}') — "
                     f"version lives ONLY in the Status line")

    # W091: the SPEC <-> modeling-track split is load-bearing.
    if "docs/spec/MODELING-NOTES.md" not in spec_txt:
        fails.append("W091: SPEC.md no longer links the modeling track "
                     "(docs/spec/MODELING-NOTES.md) — the §11 pointer and §16 stub must survive")
    for marker in set(re.findall(r"\[MN-[A-Za-z@_-]+\]", spec_txt)):
        key = marker[1:-1]
        if f"### {key}" not in open(os.path.join(ROOT, "docs", "spec", "MODELING-NOTES.md"),
                                    encoding="utf-8").read():
            fails.append(f"W091: SPEC marker {marker} has no matching '### {key}' heading "
                         "in docs/spec/MODELING-NOTES.md")

    # 4/5/6. forbidden hand-typed/stale patterns
    for pat, why, scope in FORBIDDEN:
        for p in files_for(scope):
            txt = open(p, encoding="utf-8", errors="replace").read()
            for m in re.finditer(pat, txt):
                line = txt[:m.start()].count("\n") + 1
                fails.append(f"{os.path.relpath(p, ROOT)}:{line}: /{pat}/ — {why}")

    # 7. README stdlib inventory completeness (W57)
    readme = open(os.path.join(ROOT, "README.md"), encoding="utf-8").read()
    for m in truth["std_modules"]:
        if not re.search(rf"`{re.escape(m['module'])}`", readme):
            fails.append(f"W57: README stdlib inventory missing module '{m['module']}'")
    if re.search(r"\b(six|Six)\s+std", readme):
        fails.append("W57: README still says 'six std' modules")

    # 8. packaging channel ledger ↔ README install matrix ↔ packaging/ dir
    #    (W61): the matrix must name every channel file that exists, and
    #    every packaging/ file it names must exist — in both directions.
    pkg_dir = os.path.join(ROOT, "packaging")
    channel_files = []
    if os.path.isdir(pkg_dir):
        for base, _dirs, fnames in os.walk(pkg_dir):
            for f in fnames:
                rel = os.path.relpath(os.path.join(base, f), ROOT).replace(os.sep, "/")
                if rel != "packaging/genomelab.spec" and rel != "packaging/build_exe.bat":
                    channel_files.append(rel)
    for rel in channel_files:
        if rel not in readme:
            fails.append(f"W61: packaging channel '{rel}' missing from README install matrix")
    for rel in ["packaging/scoop/operon.json", "packaging/aur/PKGBUILD",
                "packaging/aur/PKGBUILD.git", "packaging/nix/default.nix"]:
        if rel in readme and not os.path.exists(os.path.join(ROOT, rel)):
            fails.append(f"W61: README install matrix names '{rel}' but the file is missing")
    if "docs/PACKAGING.md" in readme and not os.path.exists(os.path.join(ROOT, "docs", "PACKAGING.md")):
        fails.append("W61: README links docs/PACKAGING.md but the ledger is missing")
    if not os.path.exists(os.path.join(ROOT, "docs", "PACKAGING.md")):
        fails.append("W61: docs/PACKAGING.md channel ledger missing")

    if fails:
        print("DOCS OUT OF SYNC — fix the source, never hand-patch generated files:")
        for f in fails:
            print("  ✗", f)
        return 1
    print(f"docs sync OK: v{truth['version']} · {truth['keyword_count']} keywords · "
          f"{truth['std_module_count']} std modules / {truth['std_function_count']} funcs · "
          f"{truth['proof_files']} proof files · {truth['redteam_files']} red-team payloads")
    return 0

if __name__ == "__main__":
    sys.exit(main())
