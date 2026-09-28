#!/usr/bin/env python3
"""tg_sweep.py — Total Grammar semantic contract sweep (W037, opt-in tool).

The parse half of the Total Grammar contract ("nothing you write is ever
rejected") is pinned by the differential suite. This tool proves the SEMANTIC
half over the whole corpus: every input that PARSES must either run to
completion or fail ONLY with a catchable Stress — a panic, a stack overflow,
a signal death or a hang is a contract violation, and exit codes must
distinguish the fates:

    clean      exit 0, no runtime panic markers anywhere
    contained  exit 1 (uncaught top-level Stress: `[contained]` diagnostic),
               exit 3 (strict escalation), or exit 2 (CLI-level refusal);
               the runtime stayed in control and told the operator
    panic      exit 101, a Rust panic/stack-overflow message, or death by
               signal (rc >= 128) — THE contract line, must be empty
    hang       exceeded the per-file wall-clock timeout

Corpus: every .op under tests/, examples/, std/, apps/ (redteam/ is skipped:
those payloads are adversarial by design and are covered by
scripts/redteam.sh with containment expectations) plus
scripts/tg_sweep_corpus/ — hand-written nasty-but-parseable programs that must
classify as clean or contained, never panic.

Usage:
    python3 scripts/tg_sweep.py [--bin PATH] [--timeout SECS] [--json OUT]
                                [--gate]

Opt-in, NOT a CI gate (same rule as coverage: baseline first, gate later).
Without --gate the script always exits 0 after printing the table; --gate
flips it to exit 1 on any panic/hang (for when the sweep earns a CI slot).
"""
import argparse
import os
import subprocess
import sys

PANIC_MARKERS = ("panicked at", "stack overflow", "fatal runtime error")
SKIP_DIR = "redteam"
CORPUS_DIR = os.path.join("scripts", "tg_sweep_corpus")


def collect_op(root):
    out = []
    for dirpath, dirnames, files in os.walk(root):
        rel = os.path.relpath(dirpath, root)
        parts = rel.split(os.sep)
        if SKIP_DIR in parts:
            continue
        for f in sorted(files):
            if f.endswith(".op"):
                out.append(os.path.relpath(os.path.join(dirpath, f), root))
    return sorted(out)


def classify(binpath, rel, timeout):
    """Run one program and classify from REAL behavior only."""
    try:
        p = subprocess.run(
            [binpath, "run", rel],
            capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return "hang", None, ""
    blob = (p.stdout or "") + (p.stderr or "")
    if p.returncode < 0 or p.returncode >= 128:
        return "panic", p.returncode, blob
    if any(m in blob for m in PANIC_MARKERS):
        # exit-code spoofing: a runtime that died must not report success
        return "panic", p.returncode, blob
    if p.returncode == 101:
        return "panic", p.returncode, blob
    if p.returncode in (1, 2, 3):
        return "contained", p.returncode, blob
    return "clean", p.returncode, blob


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bin", default=None, help="operon binary (default: bin/operon, then target/debug/operon)")
    ap.add_argument("--timeout", type=int, default=20, help="per-file wall-clock seconds")
    ap.add_argument("--json", default=None, help="also write the table as JSON to this path")
    ap.add_argument("--gate", action="store_true", help="exit 1 on any panic/hang (future CI wiring)")
    args = ap.parse_args()

    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    binpath = args.bin
    if binpath is None:
        for cand in ("bin/operon", "target/debug/operon"):
            if os.path.isfile(os.path.join(root, cand)):
                binpath = os.path.join(root, cand)
                break
    if binpath is None:
        print("no operon binary found (build with scripts/build.sh or cargo build)", file=sys.stderr)
        return 2
    binpath = os.path.abspath(binpath)

    targets = collect_op(root)
    corpus = [t for t in targets if t.replace(os.sep, "/").startswith(CORPUS_DIR)]
    print(f"total grammar sweep: {len(targets)} program(s) "
          f"({len(corpus)} corpus) under {os.path.relpath(binpath, root)}, "
          f"timeout {args.timeout}s\n")

    counts = {"clean": 0, "contained": 0, "panic": 0, "hang": 0}
    rows, panics = [], []
    width = max(len(t) for t in targets) if targets else 10
    for rel in targets:
        cls, rc, blob = classify(binpath, rel, args.timeout)
        counts[cls] += 1
        rows.append((cls, rc, rel, blob))
        marker = ""
        if cls == "panic":
            first = next((ln for ln in blob.splitlines()
                          if any(m in ln for m in PANIC_MARKERS)), "")
            panics.append((rel, rc, first.strip()[:160]))
            marker = "  <-- CONTRACT VIOLATION"
        if cls in ("panic", "hang") or os.environ.get("TG_SWEEP_VERBOSE"):
            print(f"  {cls:<9} rc={rc!s:<4} {rel}{marker}")

    print()
    print(f"{'class':<10} {'count':>5}   {'%':>5}")
    total = len(rows) or 1
    for cls in ("clean", "contained", "panic", "hang"):
        print(f"{cls:<10} {counts[cls]:>5}   {100.0 * counts[cls] / total:>4.1f}%")
    print(f"{'TOTAL':<10} {len(rows):>5}")

    # corpus strip: the added corpus must be clean-or-contained line by line
    if corpus:
        print(f"\ncorpus strip ({len(corpus)} hand-written nasty-but-parseable programs):")
        for rel in corpus:
            cls, rc = next((r[0], r[1]) for r in rows if r[2] == rel)
            flag = "ok " if cls in ("clean", "contained") else "FAIL"
            print(f"  [{flag}] {cls:<9} rc={rc!s:<4} {rel}")

    if panics:
        print("\npanic findings (full detail, each one is a sweep SUCCESS):")
        for rel, rc, first in panics:
            print(f"  {rel} rc={rc}\n    {first}")

    verdict = (
        f"verdict: {len(rows)} parseable programs; "
        f"{counts['clean']} clean, {counts['contained']} contained, "
        f"{counts['panic']} panic, {counts['hang']} hang. "
        + (
            "The semantic contract holds: every input that parses either ran "
            "to completion or failed only with a catchable Stress; no panic, "
            "no hang, exit codes distinguish the fates."
            if not counts["panic"] and not counts["hang"]
            else "CONTRACT VIOLATIONS PRESENT (see panic/hang rows above)."
        )
    )
    print(f"\n{verdict}")

    if args.json:
        import json
        with open(args.json, "w", encoding="utf-8") as fh:
            json.dump(
                {
                    "bin": binpath,
                    "timeout": args.timeout,
                    "counts": counts,
                    "rows": [{"file": r, "class": c, "rc": rc} for c, rc, r, _ in rows],
                },
                fh, indent=1,
            )
        print(f"json table: {args.json}")

    if args.gate and (counts["panic"] or counts["hang"]):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
