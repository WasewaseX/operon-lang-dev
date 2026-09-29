#!/usr/bin/env python3
# check_docs_sync.py — W53 companion: fail (CI-red) whenever README/SPEC quote a
# count that contradicts the generated truth in docs/stats.json.
#
#   python3 scripts/check_docs_sync.py                 # check against committed stats.json
#   python3 scripts/check_docs_sync.py --regen         # regenerate stats live, then check
#   python3 scripts/check_docs_sync.py --selftest      # fixture: a deliberately-wrong edit must fail
#
# The number rules live in gen_doc_stats.py (ANCHORS) — single source for both
# the generator and this checker, so they can never disagree about the contract.
import sys, os, subprocess, tempfile, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(ROOT)
sys.path.insert(0, os.path.join(ROOT, "scripts"))

def load_stats(regen=False):
    if regen:
        r = subprocess.run(["python3", "scripts/gen_doc_stats.py"],
                           capture_output=True, text=True)
        if r.returncode != 0:
            print(r.stdout + r.stderr)
            sys.exit("FATAL: regeneration failed")
    import json
    if not os.path.exists("docs/stats.json"):
        sys.exit("docs/stats.json missing — run scripts/gen_doc_stats.py first")
    return json.load(open("docs/stats.json"))

def run_check(stats, root="."):
    import gen_doc_stats as g
    return g.check(stats, root=root)

def selftest():
    # The W53 done-when fixture: a deliberately-wrong number MUST fail the checker.
    tmp = tempfile.mkdtemp(prefix="w53-selftest-")
    try:
        shutil.copy("README.md", os.path.join(tmp, "README.md"))
        shutil.copy("SPEC.md", os.path.join(tmp, "SPEC.md"))
        stats = load_stats()
        # inject a wrong proof-file count into the README anchor
        txt = open(os.path.join(tmp, "README.md"), encoding="utf-8").read()
        bad = txt.replace("proof-frame runner (", "proof-frame runner (", 1)
        import re
        m = re.search(r"proof-frame runner \((\d+) files", txt)
        wrong = str(int(m.group(1)) + 7)
        bad = re.sub(r"proof-frame runner \(\d+ files", f"proof-frame runner ({wrong} files", txt, count=1)
        open(os.path.join(tmp, "README.md"), "w", encoding="utf-8").write(bad)
        drifts = run_check(stats, root=tmp)
        if not any(d[0] == "README.md" for d in drifts):
            print("SELFTEST FAILED: deliberately-wrong edit was NOT caught")
            sys.exit(1)
        print("selftest ok: deliberately-wrong edit caught ->", drifts[0][1][:60])
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

def main():
    if "--selftest" in sys.argv:
        selftest(); return
    stats = load_stats(regen="--regen" in sys.argv)
    drifts = run_check(stats)
    if drifts:
        print("DOC DRIFT — docs contradict docs/stats.json. Fix mechanically:")
        print("  python3 scripts/gen_doc_stats.py --patch && git add README.md SPEC.md docs/")
        for p, got, want in drifts:
            print(f"  {p}: {got!r} -> {want}")
        sys.exit(1)
    print(f"check_docs_sync: green ({len(load_rules())} anchored rules, version {stats['version']})")

def load_rules():
    import gen_doc_stats as g
    return g.ANCHORS

if __name__ == "__main__":
    main()
