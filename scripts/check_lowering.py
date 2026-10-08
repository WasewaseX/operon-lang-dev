#!/usr/bin/env python3
"""check_lowering.py — R0.9 teeth: the lowering contract cannot drift.

docs/spec/LOWERING.md (R0.9) is normative for every biological surface the
language carries. This checker enforces its coverage obligations against the
generated/live truth in the repo:

  K1  keyword-class coverage (bidirectional, grounded):
      (a) every keyword in docs/KEYWORDS.md (GENERATED from src/parser.rs)
          that is not claimed by CORE-BIO-BOUNDARY.md's frozen CORE table
          must appear as a classified surface in LOWERING.md §2 — the 25
          grandfathered bio keywords cannot go unlowered;
      (b) every row in LOWERING.md §2.1–2.4 (classes C1/C2) must be
          grounded in a real reserved surface: a keyword from KEYWORDS.md
          or an @mark in its surface cell — no invented grammar.
  K2  MN-key 1:1 (bidirectional): every MN-* key cited in LOWERING.md §2
      resolves to a MODELING-NOTES.md §2 audit heading, and every
      MODELING-NOTES.md §2 heading is cited in LOWERING.md — the contract
      and the term audits cannot drift apart.
  K3  label vocabulary: every label in LOWERING.md §2 rows uses only the
      BIO-CONTRACT.md vocabulary {REAL, APPROX, ABSTRACTION, SIMPLIFICATION}
      (compound "REAL (x) + APPROX (y)" forms allowed).
  K4  template presence: the nine mandatory per-feature template section
      names appear in LOWERING.md §4.
  K5  laws present: the L1/L2 law statements and the precedence line exist
      in LOWERING.md §0.

Usage:
  python3 scripts/check_lowering.py            # from repo root (or --repo)
  python3 scripts/check_lowering.py --selftest # negative suite: every
                                               # mutation below MUST fail

Exit 0 = all checks green. Exit 1 = drift, with the failing check ids.
House note: this checker is self-contained (stdlib only), mirrors the
check_docs_sync.py style, and never writes to the tree outside --selftest's
temporary directory.
"""

import argparse
import re
import shutil
import sys
import tempfile
from pathlib import Path

VOCAB = {"REAL", "APPROX", "ABSTRACTION", "SIMPLIFICATION"}

TEMPLATE_SECTIONS = [
    "**`[MN-<key>]`**",
    "**Surface**",
    "**Lowering (L1)**",
    "**Engine locus + parity plan (L2)**",
    "**Determinism class**",
    "**BIO-CONTRACT label**",
    "**Not-modeled list**",
    "**Cell keys + grants**",
    "**Freeze verdict**",
]

L1_LAW = "**Law L1 — one lowering, declared.**"
L2_LAW = "**Law L2 — engines implement the lowered form, never the biology.**"
PRECEDENCE = "SPEC.md wins, then BIO-LAYER-POLICY.md"


def fail(msg):
    print(f"LOWERING CHECK FAIL: {msg}")
    return False


def read(p: Path):
    try:
        return p.read_text(encoding="utf-8")
    except OSError as e:
        print(f"LOWERING CHECK FAIL: cannot read {p}: {e}")
        sys.exit(1)


def keywords_from_keydoc(text):
    """docs/KEYWORDS.md: rows '| `kw` | analogy |' (skip the header)."""
    kws = []
    for line in text.splitlines():
        m = re.match(r"^\|\s*`([^`]+)`\s*\|", line)
        if m and not line.startswith("| keyword"):
            kws.append(m.group(1))
    return kws


def core_set_from_boundary(text):
    """CORE-BIO-BOUNDARY.md: backticked tokens inside the frozen CORE table
    section ('## What is core grammar today'). Includes the non-keyword
    literal words — harmless, they are absent from KEYWORDS.md."""
    i = text.find("## What is core grammar today")
    if i < 0:
        return set()
    j = text.find("\n## ", i + 1)  # next H2 ends the section
    if j < 0:
        j = len(text)
    blob = text[i:j].replace("\n", " ")  # join wrapped backtick blocks
    out = set()
    for tok in re.findall(r"`([^`]+)`", blob):
        out.update(tok.split())  # multi-word blocks: `true false null and or not`
    return out


def contextual_set_from_boundary(text):
    """CORE-BIO-BOUNDARY.md: the contextual-spellings bullet — recognized
    positionally but NOT reserved (`translates` `attenuates` `secrete`
    `quorum` `quench` `sum` `any` `occupy` `hill` `rbs`)."""
    out = set()
    lines = text.splitlines()
    in_ctx = False
    for line in lines:
        if line.startswith("- **Contextual spellings**"):
            in_ctx = True
        elif in_ctx and (line.startswith("- ") or line.startswith("## ")):
            break
        if in_ctx:
            for tok in re.findall(r"`([^`]+)`", line):
                out.update(tok.split())
    return out


def section(text, start_marker, end_markers):
    """Return the text between start_marker and the first end_marker."""
    i = text.find(start_marker)
    if i < 0:
        return None
    j = len(text)
    for em in end_markers:
        k = text.find(em, i + len(start_marker))
        if 0 <= k < j:
            j = k
    return text[i:j]


def lowering_rows(low_text):
    """Parse §2 rows: (surface_cell, class, label_cell, mn_cell)."""
    s2 = section(low_text, "## 2. Lowering rules", ["## 3. Reserved-word policy"])
    if s2 is None:
        return None
    rows = []
    for line in s2.splitlines():
        if not line.startswith("|") or line.startswith("| surface") or set(line) <= set("|- "):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) != 6:
            continue  # table headers etc. handled above
        rows.append(cells)
    return rows


def check(repo: Path):
    ok = True
    keydoc = read(repo / "docs/KEYWORDS.md")
    boundary = read(repo / "docs/specs/CORE-BIO-BOUNDARY.md")
    low = read(repo / "docs/spec/LOWERING.md")
    model = read(repo / "docs/spec/MODELING-NOTES.md")

    kws = set(keywords_from_keydoc(keydoc))
    if not kws:
        return fail("K1: parsed zero keywords from docs/KEYWORDS.md — parser output shape changed?")
    core = core_set_from_boundary(boundary)
    rows = lowering_rows(low)
    if rows is None:
        return fail("K1: LOWERING.md §2 not found")
    if not rows:
        return fail("K1: LOWERING.md §2 parsed zero rows")

    # ---- K1a: every reserved keyword outside the frozen core is classified
    # normalize surface cells to identifier-like tokens (handles `@copies n`,
    # `@burst(kon, koff)`, `decoy site / bind site`, multi-word blocks)
    def ident_tokens(s):
        toks = set()
        for block in re.findall(r"`([^`]+)`", s):
            toks.update(re.findall(r"@[A-Za-z0-9_]+|[A-Za-z_][A-Za-z0-9_]*", block))
        return toks

    surface_tokens = set()
    for r in rows:
        surface_tokens |= ident_tokens(r[0])
    def classified(k):
        return k in surface_tokens or ("@" + k) in surface_tokens

    unclassified = sorted(k for k in kws if k not in core and not classified(k))
    if unclassified:
        ok = fail(f"K1a: keywords with NO lowering row in LOWERING.md §2: {unclassified}")

    # ---- K1b: every C1/C2 row is grounded in a real reserved surface
    # (reserved keyword, @mark, or a boundary-doc CONTEXTUAL spelling —
    # occupy/attenuates/translates/etc. are recognized positionally,
    # not reserved, per CORE-BIO-BOUNDARY.md)
    ctx = contextual_set_from_boundary(boundary)
    grounded_bad = []
    for r in rows:
        surf, klass = r[0], r[1]
        if klass not in ("C1", "C2"):
            continue
        toks = ident_tokens(surf)
        if not any(t in kws or t in ctx or t.startswith("@") for t in toks):
            grounded_bad.append(surf[:60])
    if grounded_bad:
        ok = fail(f"K1b: C1/C2 rows not grounded in a reserved surface: {grounded_bad}")

    # ---- K2: MN keys bidirectional 1:1 (hyphen-aware: MN-operon-unit,
    # MN-m6a-write, MN-@m6a)
    cited = set(re.findall(r"MN-[A-Za-z0-9@_-]+", " ".join(r[5] for r in rows if len(r) >= 6)))
    audited = set(re.findall(r"^### (MN-[^\s(]+)", model, re.M))
    phantom = sorted(cited - audited)
    uncovered = sorted(audited - cited)
    if phantom:
        ok = fail(f"K2: LOWERING.md cites MN keys with no MODELING-NOTES §2 audit: {phantom}")
    if uncovered:
        ok = fail(f"K2: MODELING-NOTES §2 audits never cited in LOWERING.md §2: {uncovered}")

    # ---- K3: label vocabulary
    bad_labels = []
    for r in rows:
        surf, label = r[0], r[4]
        stripped = re.sub(r"\([^)]*\)", "", label)
        for part in stripped.split("+"):
            part = part.strip()
            if part and part not in VOCAB:
                bad_labels.append(f"{label!r} (row {surf[:40]!r})")
    if bad_labels:
        ok = fail(f"K3: labels outside BIO-CONTRACT vocabulary {sorted(VOCAB)}: {bad_labels}")

    # ---- K4: template section presence
    missing = [s for s in TEMPLATE_SECTIONS if s not in low]
    if missing:
        ok = fail(f"K4: template section names missing from LOWERING.md §4: {missing}")

    # ---- K5: laws + precedence
    if L1_LAW not in low:
        ok = fail("K5: Law L1 statement missing from LOWERING.md §0")
    if L2_LAW not in low:
        ok = fail("K5: Law L2 statement missing from LOWERING.md §0")
    if PRECEDENCE not in low:
        ok = fail("K5: precedence line missing from LOWERING.md header")

    if ok:
        n_kws = len([k for k in kws if k not in core])
        print(
            f"LOWERING CHECKS GREEN: K1 {n_kws} bio keywords classified, "
            f"{len(rows)} rows grounded | K2 {len(cited)} MN keys 1:1 | "
            f"K3 vocabulary clean | K4 template 9/9 | K5 laws present"
        )
    return ok


# --- negative suite (selftest): each mutation MUST flip the named check ---

NEGATIVES = [
    ("K1a", lambda t: t.replace("| `toggle` | C1 |", "| `toggle_renamed` | C1 |")),
    ("K2", lambda t: t.replace("MN-toggle", "MN-nonexistent-key")),
    ("K3", lambda t: t.replace("| APPROX | MN-occupy |", "| APPROXX | MN-occupy |")),
    ("K4", lambda t: t.replace("**Freeze verdict**", "**Freeze ruling**")),
]


def selftest(repo: Path):
    """Build a minimal repo skeleton per mutation (only the four files the
    checker reads), mutate LOWERING.md, and require the named check to fire."""
    original = read(repo / "docs/spec/LOWERING.md")
    skeleton = {
        Path("docs/KEYWORDS.md"): read(repo / "docs/KEYWORDS.md"),
        Path("docs/specs/CORE-BIO-BOUNDARY.md"): read(repo / "docs/specs/CORE-BIO-BOUNDARY.md"),
        Path("docs/spec/MODELING-NOTES.md"): read(repo / "docs/spec/MODELING-NOTES.md"),
    }
    all_ok = True
    with tempfile.TemporaryDirectory() as td:
        for expect, mutate in NEGATIVES:
            tmp = Path(td) / f"repo_{expect}"
            for rel, content in skeleton.items():
                dest = tmp / rel
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_text(content, encoding="utf-8")
            specdir = tmp / "docs/spec"
            specdir.mkdir(parents=True, exist_ok=True)
            (specdir / "LOWERING.md").write_text(mutate(original), encoding="utf-8")
            import io
            import contextlib
            buf = io.StringIO()
            try:
                with contextlib.redirect_stdout(buf):
                    res = check(tmp)
            except SystemExit:
                res = False
            caught = (not res) and expect in buf.getvalue()
            if caught:
                print(f"  selftest {expect}: mutation caught (as required)")
            else:
                all_ok = False
                print(f"  selftest {expect}: MUTATION NOT CAUGHT — checker toothless")
                print("    output:", buf.getvalue().strip()[:300])
    if all_ok:
        print("SELFTEST GREEN: all mutations caught")
    return all_ok


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent,
                    help="repo root (default: parent of scripts/)")
    ap.add_argument("--selftest", action="store_true",
                    help="run the negative suite on temp copies; never writes the tree")
    args = ap.parse_args()
    if args.selftest:
        sys.exit(0 if selftest(args.repo) else 1)
    sys.exit(0 if check(args.repo) else 1)


if __name__ == "__main__":
    main()
