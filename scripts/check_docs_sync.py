#!/usr/bin/env python3
"""check_docs_sync.py, W53/W54/W55/W56/W57/W58: docs can never lie again.

Recomputes the generated statistics (same pure function as gen_doc_stats.py)
and fails on:
  1. docs/stats.json drift vs recomputed reality (stale committed stats)
  2. SPEC Status version != Cargo.toml version (W54)
  3. SPEC H1 title carrying a version number (W54)
  4. hand-typed counts the audit caught: "N proof files", "N programs MATCH",
     "N attacks contained", "canonical, N" keyword counts (W53/W55)
  5. stale architecture claims: "C runtime" anywhere in md/html docs (W56)
  6. dependency overclaim "No crates, no network" (W58)
  7. README/STDLIB.md stdlib inventory missing a real std module (W57)
  7b. keyword without a D-008 analogy, or docs/KEYWORDS.md drift (W55)
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
    (r"No crates, no network", "W58: overclaim, Cargo.toml build-depends on cc; network is reachable via py()/run()",
     ["README.md"]),
    (r"canonical,\s*\d+", "W55: keyword count is generated (docs/KEYWORDS.md), never hand-typed",
     ["SPEC.md"]),
    (r"\d+ proof files", "W53: proof counts are generated (docs/STATS.md), never hand-typed",
     ["README.md", "BENCH.md"]),
    (r"\d+/\d+\s*(programs|differential).*MATCH", "W53: differential counts move every session, link docs/STATS.md",
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
    (r"\d+ programs, all MATCH", "W53: differential counts move every session, link docs/STATS.md",
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
        fails.append("docs/stats.json missing, run scripts/gen_doc_stats.py")
    else:
        committed = json.load(open(sp, encoding="utf-8"))
        for k in ("version", "keyword_count", "std_module_count", "std_function_count",
                  "redteam_files", "proof_files", "test_op_files"):
            if committed.get(k) != truth[k]:
                fails.append(f"stats.json drift on '{k}': committed={committed.get(k)} "
                             f"actual={truth[k]}, regenerate (gen_doc_stats.py)")
        if committed.get("keywords") != truth["keywords"]:
            fails.append("stats.json drift on 'keywords', regenerate")

    # 2/3. SPEC version honesty (W54), D-009 semantics:
    #   tagged state:   SPEC Status == Cargo version (exact)
    #   mid-milestone:  SPEC Status = vX.Y.Z-dev while Cargo carries the last tag
    spec_txt = open(os.path.join(ROOT, "SPEC.md"), encoding="utf-8").read()
    st = truth["spec"]["status"]
    ok_dev = re.fullmatch(r"v\d+\.\d+\.\d+-dev", st)
    # normalize the optional v/V prefix so a tagged Status ("v2.2.0") matches
    # Cargo's unprefixed "2.2.0", the exact-match form could never pass,
    # leaving the tagged-state branch dead code (D-009 needs both forms live)
    ok_tag = (st.lstrip("vV") == truth["version"])
    if not (ok_dev or ok_tag):
        fails.append(f"W54: SPEC Status ('{st}') must be the Cargo version ('{truth['version']}') "
                     f"or a vX.Y.Z-dev milestone label (D-009)")
    if re.search(r"v\d+\.\d+", truth["spec"]["h1"]):
        fails.append(f"W54: SPEC H1 carries a version ('{truth['spec']['h1']}'), "
                     f"version lives ONLY in the Status line")

    # W091 enforcement (sz follow-up): the two-track boundary is load-bearing
    # (CONTRIBUTING §8b). These guards fail when the §11a contract header, its
    # pointers, or the recorded crossing markers go missing silently. Removing
    # a boundary marker is a language-track change and needs a DECISIONS.md
    # entry plus a same-PR update of the minimum counts recorded here.
    m11a = re.search(r"^### 11a\. .*(W091)", spec_txt, re.M)
    if not m11a:
        fails.append("W091: SPEC §11a contract header missing (the biology-layer "
                     "boundary map must stay a first-class section)")
    else:
        end11a = re.search(r"^### ", spec_txt[m11a.end():], re.M)
        region = spec_txt[m11a.start():m11a.end() + (end11a.start() if end11a else 0)]
        if "docs/spec/BIO-CONTRACT.md" not in region:
            fails.append("W091: SPEC §11a no longer points at docs/spec/BIO-CONTRACT.md")
        if "DETERMINISM" not in region:
            fails.append("W091: SPEC §11a no longer carries its DETERMINISM pointer")
    crossings = len(re.findall(r"Crossing \(§11a\)", spec_txt))
    if crossings < 3:
        fails.append(f"W091: SPEC has {crossings} 'Crossing (§11a)' marker(s), "
                     f"minimum 3 (the live count this guard was born with; "
                     f"additions are fine, silent regressions are not)")
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
                fails.append(f"{os.path.relpath(p, ROOT)}:{line}: /{pat}/, {why}")

    # W55. the keyword table is generated and every keyword carries a
    # D-008 analogy; a pending entry means the table can silently lie
    if truth["keyword_analogy_pending"]:
        fails.append("W55: %d keyword(s) lack a D-008 analogy (%s), "
                     "fill KEYWORD_ANALOGY in gen_doc_stats.py and regenerate"
                     % (len(truth["keyword_analogy_pending"]),
                        ", ".join(truth["keyword_analogy_pending"])))
    kp = os.path.join(ROOT, "docs", "KEYWORDS.md")
    if not os.path.exists(kp):
        fails.append("W55: docs/KEYWORDS.md missing, run scripts/gen_doc_stats.py")
    elif f"Count: **{truth['keyword_count']}**" not in open(kp, encoding="utf-8").read():
        fails.append("W55: docs/KEYWORDS.md count line does not match the parser's "
                     "reserved set, regenerate (gen_doc_stats.py)")

    # 7. README stdlib inventory completeness (W57)
    readme = open(os.path.join(ROOT, "README.md"), encoding="utf-8").read()
    for m in truth["std_modules"]:
        if not re.search(rf"`{re.escape(m['module'])}`", readme):
            fails.append(f"W57: README stdlib inventory missing module '{m['module']}'")
    if re.search(r"\b(six|Six)\s+std", readme):
        fails.append("W57: README still says 'six std' modules")

    # 7b. STDLIB.md must name every real std module (W57)
    stdlib = open(os.path.join(ROOT, "STDLIB.md"), encoding="utf-8").read()
    for m in truth["std_modules"]:
        if f"std/{m['module']}" not in stdlib:
            fails.append(f"W57: STDLIB.md inventory missing module 'std/{m['module']}'")

    # 8. packaging channel ledger ↔ README install matrix ↔ packaging/ dir
    #    (W61): the matrix must name every channel file that exists, and
    #    every packaging/ file it names must exist, in both directions.
    pkg_dir = os.path.join(ROOT, "packaging")
    channel_files = []
    if os.path.isdir(pkg_dir):
        for base, _dirs, fnames in os.walk(pkg_dir):
            for f in fnames:
                rel = os.path.relpath(os.path.join(base, f), ROOT).replace(os.sep, "/")
                if rel != "packaging/genomelab.spec" and rel != "packaging/build_exe.bat":
                    channel_files.append(rel)
    packaging_md = open(os.path.join(ROOT, "docs", "PACKAGING.md"), encoding="utf-8").read()
    for rel in channel_files:
        if rel not in readme:
            fails.append(f"W61: packaging channel '{rel}' missing from README install matrix")
        # W061-A: the README calls docs/PACKAGING.md "the full ledger", so every
        # channel the matrix names must also have a ledger row — the hosted
        # registry drifted exactly this way (README row existed, ledger row
        # did not, and the guard could not see it).
        if rel not in packaging_md:
            fails.append(f"W61: packaging channel '{rel}' missing from docs/PACKAGING.md ledger")
    for rel in ["packaging/scoop/operon.json", "packaging/aur/PKGBUILD",
                "packaging/aur/PKGBUILD.git", "packaging/nix/default.nix"]:
        if rel in readme and not os.path.exists(os.path.join(ROOT, rel)):
            fails.append(f"W61: README install matrix names '{rel}' but the file is missing")
    if "docs/PACKAGING.md" in readme and not os.path.exists(os.path.join(ROOT, "docs", "PACKAGING.md")):
        fails.append("W61: README links docs/PACKAGING.md but the ledger is missing")
    if not os.path.exists(os.path.join(ROOT, "docs", "PACKAGING.md")):
        fails.append("W61: docs/PACKAGING.md channel ledger missing")

    # 8b. W061-A: the version literals in the community-draft manifests must
    # match the Cargo version. Two releases (2.6.0, 2.7.0) shipped while every
    # manifest still pinned 2.2.0 — the publish checklist called this checker
    # "the natural next step"; here it is.
    mver = re.search(r'^version = "([^"]+)"', open(os.path.join(ROOT, "Cargo.toml"),
                                                    encoding="utf-8").read(), re.M)
    if not mver:
        fails.append("W61: cannot read the version from Cargo.toml [package]")
    else:
        ver = mver.group(1)
        scoop = open(os.path.join(ROOT, "packaging", "scoop", "operon.json"),
                     encoding="utf-8").read()
        if f'"version": "{ver}"' not in scoop:
            fails.append(f"W61: packaging/scoop/operon.json 'version' != {ver} "
                         "(bump the manifest in the same commit as the release)")
        if f"/v{ver}/operon-{ver}-" not in scoop:
            fails.append(f"W61: packaging/scoop/operon.json asset URL does not pin {ver} "
                         "(url + extract_dir carry the version too)")
        if f'"extract_dir": "operon-{ver}"' not in scoop:
            fails.append(f"W61: packaging/scoop/operon.json extract_dir != operon-{ver}")
        pkgbuild = open(os.path.join(ROOT, "packaging", "aur", "PKGBUILD"),
                        encoding="utf-8").read()
        if not re.search(rf"^pkgver={re.escape(ver)}$", pkgbuild, re.M):
            fails.append(f"W61: packaging/aur/PKGBUILD pkgver != {ver}")
        pkgbuild_git = open(os.path.join(ROOT, "packaging", "aur", "PKGBUILD.git"),
                            encoding="utf-8").read()
        # the -git draft's pkgver is a git-describe placeholder regenerated at
        # publish time; only the base version prefix is pinned here
        if not re.search(rf"^pkgver={re.escape(ver)}\.r", pkgbuild_git, re.M):
            fails.append(f"W61: packaging/aur/PKGBUILD.git pkgver base != {ver}")
        homebrew = open(os.path.join(ROOT, "packaging", "homebrew", "operon.rb"),
                        encoding="utf-8").read()
        if f"/archive/refs/tags/v{ver}.tar.gz" not in homebrew:
            fails.append(f"W61: packaging/homebrew/operon.rb url does not pin v{ver}")
        nix = open(os.path.join(ROOT, "packaging", "nix", "default.nix"),
                   encoding="utf-8").read()
        if f'version = "{ver}"' not in nix:
            fails.append(f"W61: packaging/nix/default.nix version != {ver}")

    if fails:
        print("DOCS OUT OF SYNC, fix the source, never hand-patch generated files:")
        for f in fails:
            print("  ✗", f)
        return 1
    print(f"docs sync OK: v{truth['version']} · {truth['keyword_count']} keywords · "
          f"{truth['std_module_count']} std modules / {truth['std_function_count']} funcs · "
          f"{truth['proof_files']} proof files · {truth['redteam_files']} red-team payloads")
    return 0

if __name__ == "__main__":
    sys.exit(main())
