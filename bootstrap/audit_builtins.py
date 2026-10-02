#!/usr/bin/env python3
"""audit_builtins.py — every src/ builtin must have >=1 differential golden (S3).

Single source of truth for the builtin surface: `pub const BUILTIN_NAMES`
in src/interp.rs (the same table the resolution cache and the wobble
suggester read). A builtin "has a differential golden" when at least one
tracked .op file under tests/ or apps/ calls it (or calls a SYNONYM that
canonicalizes to it — print/echo/say/show all cover promote, and vice
versa: the synonym path exercises the same builtin body through
call_builtin's canonicalization).

Call detection is deliberately conservative-but-textual: comments and
every string-literal form are stripped, then a call is `NAME(` at a word
boundary. Over-approximation risk (a same-spelled user gene shadows the
builtin) is accepted and documented: the audit's job is to catch a NEW
builtin landing with zero corpus contact, not to prove semantic
exercision — the differential harness itself proves behavior per file.

Redteam payloads are excluded: adversarial BYTES whose identity is the
test (fix_corpus skips them for the same reason); a payload that happens
to spell `sqrt(` says nothing about golden coverage.

Exit codes: 0 = every builtin covered; 1 = gap list (also printed);
2 = self-check failed (registry not found / corpus walk failed).
"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INTERP = os.path.join(ROOT, "src", "interp.rs")

REGISTRY_RE = re.compile(
    r"pub\s+const\s+BUILTIN_NAMES\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\];", re.S)
SYNONYM_RE = re.compile(
    r"pub\s+const\s+BUILTIN_SYNONYMS\s*:\s*&\[\(&str,\s*&str\)\]\s*=\s*&\[(.*?)\];", re.S)
ENTRY_RE = re.compile(r'"((?:[^"\\]|\\.)*)"')
RUST_COMMENT_RE = re.compile(r'//[^\n]*|/\*.*?\*/', re.S)


def extract_table(src, regex, what):
    m = regex.search(src)
    if not m:
        print(f"audit_builtins: cannot locate {what} in src/interp.rs", file=sys.stderr)
        sys.exit(2)
    # strip Rust comments FIRST: the registry block carries doc comments and
    # some of them contain quoted names (the "#phenotype" identity key in the
    # W34 stage-2 note) — quoting inside a comment is not a registry entry.
    return ENTRY_RE.findall(RUST_COMMENT_RE.sub("", m.group(1)))


def tracked_op_files():
    """Tracked .op files under tests/ and apps/, redteam excluded. Falls
    back to a filesystem walk when git is unavailable (same corpus the
    harness/fix_corpus walkers see, minus the adversarial-bytes lane)."""
    try:
        out = subprocess.run(
            ["git", "ls-files", "tests", "apps"], cwd=ROOT,
            capture_output=True, text=True, timeout=30, check=True).stdout
        files = [l for l in out.splitlines() if l.endswith(".op")]
        if files:
            return sorted(f for f in files if os.sep + "redteam" + os.sep not in f)
    except Exception:
        pass
    out = []
    for base in ("tests", "apps"):
        d = os.path.join(ROOT, base)
        for dirpath, dirnames, filenames in os.walk(d):
            dirnames[:] = [x for x in dirnames if x != "redteam"]
            for fn in filenames:
                if fn.endswith(".op"):
                    out.append(os.path.relpath(os.path.join(dirpath, fn), ROOT))
    return sorted(out)


STRIP_RE = re.compile(
    r'"""(?:[^\\]|\\.)*?"""'      # triple-quoted
    r"|'''(?:[^\\]|\\.)*?'''"
    r'|"(?:[^"\\\n]|\\.)*"'       # single-line double-quoted
    r"|'(?:[^'\\\n]|\\.)*'"       # single-quoted
    r'|#[^\n]*'                   # comment to end of line
    , re.S)

CALL_RE_TMPL = r"(?<![A-Za-z0-9_?.])(%s)\s*\("


def calls_in(path):
    with open(os.path.join(ROOT, path), encoding="utf-8", errors="replace") as f:
        text = f.read()
    return STRIP.sub("", text)


STRIP = STRIP_RE  # alias


def main():
    with open(INTERP, encoding="utf-8") as f:
        src = f.read()
    builtins = extract_table(src, REGISTRY_RE, "BUILTIN_NAMES")
    synonyms = extract_table(src, SYNONYM_RE, "BUILTIN_SYNONYMS")

    # registry hygiene while we are here (S3 audit = surface truth):
    # duplicate entries are dead weight in the linear-scan era narrative
    # and hash-irrelevant today, but a duplicate name in the source table
    # is exactly the kind of drift a new-builtin merge can introduce.
    seen, dups = set(), []
    for n in builtins:
        if n in seen and n not in dups:
            dups.append(n)
        seen.add(n)

    canon_of = {}
    for syn, target in zip(synonyms[0::2], synonyms[1::2]):
        canon_of[syn] = target
    # NOTE: BUILTIN_SYNONYMS entries are ("syn", "canonical") string pairs;
    # ENTRY_RE flattens them in order, so pair them up.

    files = tracked_op_files()
    if not files:
        print("audit_builtins: no tracked .op corpus found", file=sys.stderr)
        sys.exit(2)

    # one stripped pass per file, then per-name regex over it
    texts = {f: calls_in(f) for f in files}
    coverage = {n: [] for n in builtins}
    alt_names = {n: [n] + [s for s, t in canon_of.items() if t == n] for n in builtins}
    patterns = {
        n: re.compile(CALL_RE_TMPL % "|".join(re.escape(a) for a in alts))
        for n, alts in alt_names.items()
    }
    for f, text in texts.items():
        for n, pat in patterns.items():
            if pat.search(text):
                coverage[n].append(f)

    gaps = sorted(n for n, fs in coverage.items() if not fs)
    covered = sum(1 for fs in coverage.values() if fs)

    print(f"audit_builtins: {len(builtins)} registry builtins "
          f"({len(synonyms)//2} synonyms) · corpus {len(files)} tracked .op files")
    if dups:
        print(f"audit_builtins: REGISTRY DUPLICATES: {', '.join(dups)}")
    if gaps:
        print(f"audit_builtins: FAIL — {len(gaps)} builtin(s) with NO differential "
              f"golden ({covered}/{len(builtins)} covered):")
        for n in gaps:
            where = sorted(coverage.get(canon_of.get(n, n), []))[:3]
            hint = f"  (synonym '{n}' -> '{canon_of[n]}')" if n in canon_of else ""
            print(f"  - {n}{hint}")
        sys.exit(1)
    print(f"audit_builtins: OK — every builtin has >=1 differential golden "
          f"({covered}/{len(builtins)})")
    sys.exit(0)


if __name__ == "__main__":
    main()
