#!/usr/bin/env python3
# gen_doc_stats.py — W53: the ONE generated source of truth for every count
# that docs quote. Humans never hand-type these numbers again.
#
#   python3 scripts/gen_doc_stats.py            # write docs/stats.json + docs/STATS.md
#   python3 scripts/gen_doc_stats.py --patch    # also mechanically patch README/SPEC anchors
#
# Sources (all measured live from the tree, never from docs):
#   proofs      -> ./bin/operon test tests/   (files / proofs / asserts / passed / failed)
#   apps        -> ./bin/operon test apps/
#   granted     -> tests/granted/*.op count
#   differential-> bootstrap/harness.py       (match / diverge / skipped)
#   redteam     -> tests/redteam/*.op count
#   std         -> std/*.op stems + gene declaration count
#   keywords    -> src/parser.rs KEYWORDS const
#   lsp         -> textDocument/* + lifecycle methods in src/ls.rs + src/bin/operon-ls.rs
#   version     -> Cargo.toml
import json, re, subprocess, sys, glob, os, datetime

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(ROOT)

def run(cmd, timeout=600):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout)
    return r.stdout + r.stderr

def parse_test_suite():
    out = run("./bin/operon test tests/")
    m = re.search(r"(\d+) file\(s\), (\d+) proof\(s\): (\d+) passed, (\d+) failed \((\d+) assertion", out)
    if not m:
        sys.exit("FATAL: cannot parse operon test output:\n" + out[-800:])
    return {"files": int(m.group(1)), "proofs": int(m.group(2)),
            "passed": int(m.group(3)), "failed": int(m.group(4)),
            "asserts": int(m.group(5))}

def parse_harness():
    out = run("python3 bootstrap/harness.py")
    m = re.search(r"result: (\d+) match, (\d+) diverge, (\d+) skipped", out)
    if not m:
        sys.exit("FATAL: cannot parse harness output:\n" + out[-800:])
    granted = len(re.findall(r"^\s*MATCH\s+\S*granted", out, re.M))
    return {"match": int(m.group(1)), "diverge": int(m.group(2)),
            "skipped": int(m.group(3)), "granted_targets": granted}

def collect():
    stats = {}
    stats["generated_utc"] = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")
    stats["version"] = re.search(r'^version = "([^"]+)"', open("Cargo.toml").read(), re.M).group(1)
    stats["proofs"] = parse_test_suite()
    out = run("./bin/operon test apps/")
    m = re.search(r"(\d+) file\(s\), (\d+) proof\(s\): (\d+) passed, (\d+) failed \((\d+) assertion", out)
    stats["apps"] = {"files": int(m.group(1)), "proofs": int(m.group(2)),
                     "passed": int(m.group(3)), "failed": int(m.group(4)),
                     "asserts": int(m.group(5))} if m else None
    stats["granted"] = {"files": len(glob.glob("tests/granted/*.op"))}
    stats["differential"] = parse_harness()
    rt = sorted(os.path.basename(p) for p in glob.glob("tests/redteam/*.op"))
    # the number docs quote is the SUITE result (committed payloads + runtime
    # fixtures like the TOCTOU in-grant file and adversarial symlinks), so run it.
    out = run("bash scripts/redteam.sh")
    m = re.search(r"redteam: (\d+) contained, (\d+) breached", out)
    if not m:
        sys.exit("FATAL: cannot parse redteam output:\n" + out[-800:])
    stats["redteam"] = {"files": len(rt), "contained": int(m.group(1)),
                        "breached": int(m.group(2))}
    mods = sorted(os.path.splitext(os.path.basename(p))[0] for p in glob.glob("std/*.op"))
    gene = 0
    for p in glob.glob("std/*.op"):
        gene += len(re.findall(r"^(@\w+(?:\([^)]*\))?\s+)*(pub\s+)?gene\s+[a-z_0-9]+", open(p).read(), re.M))
    stats["std"] = {"modules": len(mods), "module_list": mods, "genes": gene}
    kw = re.search(r"KEYWORDS:\s*&\[&str\]\s*=\s*&\[(.*?)\];", open("src/parser.rs").read(), re.S)
    words = re.findall(r'"([^"]+)"', kw.group(1)) if kw else []
    stats["keywords"] = {"count": len(words), "list": words}
    lsp_src = open("src/ls.rs").read() + open("src/bin/operon-ls.rs").read()
    methods = sorted(set(re.findall(r'"((?:textDocument|workspace)/[A-Za-z]+)"', lsp_src))
                     | set(re.findall(r'"(initialize|shutdown|exit)"', lsp_src)))
    stats["lsp"] = {"methods": methods}
    return stats

WORDS = {1:"One",2:"Two",3:"Three",4:"Four",5:"Five",6:"Six",7:"Seven",8:"Eight",
         9:"Nine",10:"Ten",11:"Eleven",12:"Twelve",13:"Thirteen",14:"Fourteen",
         15:"Fifteen",16:"Sixteen",17:"Seventeen",18:"Eighteen",19:"Nineteen",20:"Twenty"}

def fmt(n):  # 1298 -> "1,298"
    return f"{n:,}"

def render_md(s):
    L = []
    L.append("# STATS — generated doc counts (W53)")
    L.append("")
    L.append(f"Generated {s['generated_utc']} by `scripts/gen_doc_stats.py`. **Do not hand-edit** —")
    L.append("regenerate with `python3 scripts/gen_doc_stats.py` and commit together with any")
    L.append("change that moves a count. `scripts/check_docs_sync.py` fails when README/SPEC")
    L.append("quote a number that contradicts `docs/stats.json`.")
    L.append("")
    L.append("| metric | value |")
    L.append("|---|---|")
    L.append(f"| implementation version | {s['version']} |")
    L.append(f"| proof suite (tests/) | {s['proofs']['files']} files / {s['proofs']['proofs']} proofs / {fmt(s['proofs']['asserts'])} assertions ({s['proofs']['passed']} passed, {s['proofs']['failed']} failed) |")
    if s["apps"]:
        L.append(f"| proof suite (apps/) | {s['apps']['files']} files / {s['apps']['proofs']} proofs / {fmt(s['apps']['asserts'])} assertions |")
    L.append(f"| granted-lane proofs | {s['granted']['files']} files under explicit operator cells |")
    L.append(f"| differential harness | {s['differential']['match']} match / {s['differential']['diverge']} diverge ({s['differential']['granted_targets']} granted targets) |")
    L.append(f"| red-team suite | {s['redteam']['contained']} attacks contained, {s['redteam']['breached']} breached ({s['redteam']['files']} committed payloads + runtime fixtures) |")
    L.append(f"| stdlib | {s['std']['modules']} modules, {s['std']['genes']} genes |")
    L.append(f"| keywords | {s['keywords']['count']} |")
    L.append(f"| LSP methods | {len(s['lsp']['methods'])} |")
    L.append("")
    L.append("## std modules")
    L.append("")
    L.append(", ".join("`std/%s`" % m for m in s["std"]["module_list"]))
    L.append("")
    L.append("## keyword inventory")
    L.append("")
    L.append(", ".join("`%s`" % k for k in s["keywords"]["list"]))
    L.append("")
    L.append("## LSP methods")
    L.append("")
    L.append(", ".join("`%s`" % m for m in s["lsp"]["methods"]))
    L.append("")
    return "\n".join(L)

ANCHORS = [
    # (file, pattern with (?P<...>) groups, builder(stats)->replacement groups)
    ("README.md",
     r"proof-frame runner \((\d+) files / (\d+) proofs / ([\d,]+) assertions green\)",
     lambda s: (str(s["proofs"]["files"]), str(s["proofs"]["proofs"]), fmt(s["proofs"]["asserts"]))),
    ("README.md",
     r"# (\d+) proof files \(([\d,]+) assertions\), C\+\+ kernel smoke",
     lambda s: (str(s["proofs"]["files"]), fmt(s["proofs"]["asserts"]))),
    ("SPEC.md",
     r"Current suite: (\d+) files / (\d+) proofs / ([\d,]+) assertions",
     lambda s: (str(s["proofs"]["files"]), str(s["proofs"]["proofs"]), fmt(s["proofs"]["asserts"]))),
    ("SPEC.md",
     r"plus (\d+) granted-lane proofs",
     lambda s: (str(s["granted"]["files"]),)),
    ("SPEC.md",
     r"harness verifies (\d+) byte-exact targets \((\d+) zero-grant sweep \+ (\d+) granted-with-cell\)",
     lambda s: (str(s["differential"]["match"]),
                str(s["differential"]["match"] - s["differential"]["granted_targets"]),
                str(s["differential"]["granted_targets"]))),
    ("SPEC.md",
     r"Proof frames: \*\*(\d+) files / (\d+) proofs / ([\d,]+) assertions\*\*",
     lambda s: (str(s["proofs"]["files"]), str(s["proofs"]["proofs"]), fmt(s["proofs"]["asserts"]))),
    ("README.md",
     r"adversarial containment: (\d+) attacks contained, (\d+) breached",
     lambda s: (str(s["redteam"]["contained"]), str(s["redteam"]["breached"]))),
]

def check(stats, root="."):
    drifts = []
    for path, pat, build in ANCHORS:
        txt = open(os.path.join(root, path), encoding="utf-8").read()
        want = build(stats)
        for m in re.finditer(pat, txt):
            got = tuple(g.replace(",", "") for g in m.groups())
            if got != tuple(w.replace(",", "") for w in want):
                drifts.append((path, m.group(0)[:90], "live: " + " / ".join(want)))
    # std module list rule (README "Fifteen stdlib modules today: `a`, `b`, ...")
    for path in ("README.md",):
        txt = open(os.path.join(root, path), encoding="utf-8").read()
        m = re.search(r"([A-Z][a-z]+) stdlib modules today: ((?:`[a-z_0-9]+`(?:, )?)+)", txt)
        if m:
            word, listing = m.group(1), m.group(2)
            if WORDS.get(stats["std"]["modules"]) != word:
                drifts.append((path, f"{word} stdlib modules", f"live: {stats['std']['modules']} ({WORDS.get(stats['std']['modules'],'?')})"))
            listed = re.findall(r"`([a-z_0-9]+)`", listing)
            if sorted(listed) != stats["std"]["module_list"]:
                drifts.append((path, "stdlib module list", "live: " + ", ".join(stats["std"]["module_list"])))
    return drifts

def patch(stats):
    n = 0
    for path, pat, build in ANCHORS:
        txt = open(path, encoding="utf-8").read()
        want = build(stats)
        out = ""
        last = 0
        changed = False
        for m in re.finditer(pat, txt):
            # splice: replace each capture group's text with the live value,
            # keeping all literal text between groups (spans are txt-absolute).
            out += txt[last:m.start()]
            seg = ""
            seglast = m.start()
            for i in range(1, len(m.groups()) + 1):
                g0, g1 = m.span(i)
                seg += txt[seglast:g0] + want[i - 1]
                seglast = g1
            seg += txt[seglast:m.end()]
            out += seg
            last = m.end()
            if seg != txt[m.start():m.end()]:
                changed = True
                n += 1
        out += txt[last:]
        if changed:
            open(path, "w", encoding="utf-8").write(out)
    return n

def main():
    write = "--patch" not in sys.argv
    stats = collect()
    os.makedirs("docs", exist_ok=True)
    json.dump(stats, open("docs/stats.json", "w"), indent=2)
    open("docs/STATS.md", "w").write(render_md(stats))
    print(f"stats.json + STATS.md written (proofs {stats['proofs']['files']}f/{stats['proofs']['proofs']}p/{stats['proofs']['asserts']}a, "
          f"diff {stats['differential']['match']}, redteam {stats['redteam']['contained']}c/{stats['redteam']['breached']}b, "
          f"std {stats['std']['modules']}m/{stats['std']['genes']}g, kw {stats['keywords']['count']})")
    if not write:
        n = patch(stats)
        print(f"patched {n} anchor occurrence(s) in README/SPEC")
    drifts = check(stats)
    if drifts:
        print("DOC DRIFT (run: python3 scripts/gen_doc_stats.py --patch):")
        for p, got, want in drifts:
            print(f"  {p}: {got!r} -> {want}")
        sys.exit(1)
    print("check_docs_sync: docs match generated truth")

if __name__ == "__main__":
    main()
