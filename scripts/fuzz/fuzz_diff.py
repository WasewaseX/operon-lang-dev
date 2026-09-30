#!/usr/bin/env python3
"""fuzz_diff.py — S7 stage 2: differential fuzzing AT SCALE.

The fixed differential corpus (bootstrap/harness.py, 190 programs) pins the
engines on inputs people WROTE. This lane holds the same promise against
programs nobody wrote YET: it generates grammar-aware, deterministic Operon
programs by reusing scripts/gen_corpus.py's bucket generators VERBATIM (same
(rng, tag) contract, so fuzz_diff(seed, i) == gen_corpus(seed, i) program
for program), then runs every program through every implementation and the
checker, and fails on any divergence.

Contracts under fuzz, per generated program P:

  D1  Rust VM (default run)      vs Python oracle: rc equal + stdout
      byte-identical (the harness.py differential contract, at scale)
  D2  Rust tree-walk (--no-vm)   vs Python oracle: rc equal + stdout
      byte-identical
  D3  Rust VM vs Rust tree-walk: rc + stdout + stderr identical (the
      fuzz_parser.py C4 engine-agreement contract, on runnable programs)
  D4  `operon check` rc contract (dx-r9, no --strict): rc 3 <=> the check
      --json findings array contains at least one severity=="error" item;
      rc 0 <=> none
  D5  JSON-vs-plain shape parity (grounded in print_diag's contract, SPEC
      9a.1: section headers and the summary: line are the parse surface
      scripts rely on): `check --json` parses to an object with the
      documented key set {file, score, letter, notes, wobbles, fallbacks,
      phantoms, nmd, findings}; the plain summary line's counters must
      equal the JSON document (errors == error-severity findings;
      warnings == warning findings + phantoms; repair note(s) == notes);
      the repair section quotes notes/wobbles/fallbacks verbatim and
      appears iff notes > 0; phantom rendering iff phantoms non-empty;
      the lint style pointer count equals the summary style count
  R1  no surface ever panics, aborts, signals, or hangs (rc in {0,1,2,3},
      no panic text, per-input wall-clock bound)

Every program is self-contained and deterministic by the gen_corpus
contract (no clock, rng, threads, spawns; bounded loops/recursion/
allocation; stresses only deliberately wrapped) — so a finding reproduces
from (seed, i) alone, and the offending program is saved alongside.

Findings are saved to fuzz_corpus/diff_<seed>_<i>_<kind>.op with one JSON
line each in fuzz_corpus/MANIFEST.jsonl. Exit 1 = findings exist (they are
BUGS to fix — land the fix with a regression .op; see TRIAGE.md).

Usage:
  python3 scripts/fuzz/fuzz_diff.py [--n 1200] [--seed 20260930]
      [--time-budget 600] [--per-input-timeout 10] [--bin bin/operon]
      [--corpus-dir fuzz_corpus] [--max-findings 20]
Exit: 0 no findings, 1 findings, 2 setup error.
"""
import argparse
import hashlib
import json
import os
import random
import re
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
import gen_corpus  # noqa: E402  (the grammar-aware bucket generators, reused verbatim)

ORACLE = os.path.join(ROOT, "bootstrap", "oracle.py")
TMPDIR = os.path.join(ROOT, "target", "fuzz_diff")
PANIC_TEXTS = ("panicked at", "stack overflow", "fatal runtime error", "[fatal]")
CONTAINED_RCS = {0, 1, 2, 3}
CHECK_KEYS = ["file", "score", "letter", "notes", "wobbles", "fallbacks",
              "phantoms", "nmd", "findings"]


def find_binary(explicit):
    cands = [
        explicit,
        os.environ.get("OPERON_BIN"),
        os.path.join(ROOT, "bin", "operon"),
        os.path.join(ROOT, "target", "release", "operon"),
        os.path.join(ROOT, "target", "debug", "operon"),
    ]
    for c in cands:
        if c and os.path.isfile(c) and os.access(c, os.X_OK):
            return c
    return None


def probe(argv, timeout):
    try:
        p = subprocess.run(argv, cwd=ROOT, capture_output=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired as e:
        return "TIMEOUT", e.stdout or b"", e.stderr or b""


def robustness(rc, err):
    """R1: contained-or-clean on every surface. Returns a finding string or None."""
    if rc == "TIMEOUT":
        return "hang (per-input timeout)"
    if not isinstance(rc, int):
        return f"non-integer rc {rc!r}"
    if rc not in CONTAINED_RCS:
        return f"exit {rc} outside the contained set"
    blob = (err or b"").decode("utf-8", errors="replace")
    for t in PANIC_TEXTS:
        if t in blob:
            return f"panic text {t!r} on stderr"
    return None


def gen_program(seed, i):
    """The exact gen_corpus program for (seed, i): same bucket rotation, same
    per-index rng derivation, same header discipline. Deterministic."""
    bucket = gen_corpus.BUCKETS[i % len(gen_corpus.BUCKETS)]
    idx = i // len(gen_corpus.BUCKETS)
    tag = f"{bucket[0]}{idx:03d}"
    rng = random.Random(seed * 1000003 + i * 7919)
    frags = gen_corpus.GEN[bucket](rng, tag)
    header = [
        "# differential fuzz corpus — generated by fuzz_diff.py, do not edit",
        f"# bucket={bucket} seed={seed} idx={idx} tag={tag}",
        "",
    ]
    text = "\n".join(header) + "\n" + "\n\n".join(frags) + "\n"
    return bucket, idx, text


def diff_finding(binpath, oracle, path, timeout):
    """Runs the full D1-D5 + R1 battery on one program.
    Returns (kind, detail) for the FIRST violation, or None if all hold.
    kind in: crash / hang / oracle-diverge-vm / oracle-diverge-tree /
    engine-diverge / check-rc-contract / json-shape."""
    execs = {
        "vm": [binpath, "run", path],
        "tree": [binpath, "run", path, "--no-vm"],
        "oracle": [sys.executable, oracle, "run", path],
        "check": [binpath, "check", path],
        "check_json": [binpath, "check", path, "--json"],
    }
    res = {}
    for name, argv in execs.items():
        rc, out, err = probe(argv, timeout)
        bad = robustness(rc, err)
        if bad:
            return ("hang" if bad.startswith("hang") else "crash",
                    f"{name}: {bad}")
        res[name] = (rc, out, err)

    vm_rc, vm_out, _ = res["vm"]
    tr_rc, tr_out, tr_err = res["tree"]
    or_rc, or_out, _ = res["oracle"]

    # D1 / D2: Rust vs Python oracle — the harness.py contract (rc + stdout;
    # stderr notes are a mirrored parity surface, not part of the byte
    # contract, same precedent as bootstrap/harness.py).
    if (vm_rc, vm_out) != (or_rc, or_out):
        return "oracle-diverge-vm", f"rc {vm_rc} vs {or_rc}"
    if (tr_rc, tr_out) != (or_rc, or_out):
        return "oracle-diverge-tree", f"rc {tr_rc} vs {or_rc}"

    # D3: engine-vs-engine on the SAME Rust binary — full triple, the
    # fuzz_parser.py C4 precedent.
    if (vm_rc, vm_out, res["vm"][2]) != (tr_rc, tr_out, tr_err):
        return "engine-diverge", f"rc {vm_rc} vs {tr_rc}"

    # D4: the check rc contract (dx-r9, no --strict in this lane).
    plain_rc, plain_out, _ = res["check"]
    try:
        doc = json.loads(res["check_json"][1].decode("utf-8"))
    except (ValueError, UnicodeDecodeError) as e:
        return "json-shape", f"check --json does not parse: {e}"
    if not isinstance(doc, dict) or any(k not in doc for k in CHECK_KEYS):
        missing = [k for k in CHECK_KEYS
                   if not isinstance(doc, dict) or k not in doc]
        return "json-shape", f"missing key(s) {missing}"
    err_findings = [f for f in doc["findings"]
                    if isinstance(f, dict) and f.get("severity") == "error"]
    expect_rc = 3 if err_findings else 0
    if plain_rc != expect_rc:
        return "check-rc-contract", (
            f"rc {plain_rc} but {len(err_findings)} error finding(s), "
            f"contract says {expect_rc}")

    # D5: JSON-vs-plain shape parity. The plain diag output's summary line
    # is the parse surface scripts rely on (SPEC 9a.1); every counter in it
    # must equal the JSON document's numbers. `check` never executes the
    # program, so plain stdout is exactly the diag rendering.
    plain_text = plain_out.decode("utf-8", errors="replace")
    lines = plain_text.splitlines()
    summary = [l for l in lines if l.startswith("summary: ")]
    if len(summary) != 1:
        return "json-shape", (
            f"expected exactly one summary: line, got {len(summary)}")
    m = re.fullmatch(
        r"summary: (\d+) error\(s\), (\d+) warning\(s\), (\d+) style, "
        r"(\d+) repair note\(s\)",
        summary[0])
    if not m:
        return "json-shape", f"summary line not in contract shape: {summary[0]!r}"
    s_err, s_warn, s_style, s_notes = (int(g) for g in m.groups())
    if s_err != len(err_findings):
        return "json-shape", (
            f"summary errors {s_err} vs json {len(err_findings)}")
    warn_findings = [f for f in doc["findings"]
                     if isinstance(f, dict) and f.get("severity") == "warning"]
    if s_warn != len(warn_findings) + len(doc["phantoms"]):
        return "json-shape", (
            f"summary warnings {s_warn} vs json {len(warn_findings)} "
            f"warning finding(s) + {len(doc['phantoms'])} phantoms")
    if s_notes != doc["notes"]:
        return "json-shape", (
            f"summary repair notes {s_notes} vs json notes {doc['notes']}")
    rep_line = [l for l in lines if l.startswith("  ") and "wobble(s)" in l]
    if (doc["notes"] > 0) != bool(rep_line):
        return "json-shape", (
            "repair section presence disagrees with json notes")
    if rep_line:
        m2 = re.search(r"(\d+) note\(s\), (\d+) wobble\(s\), (\d+) fallback\(s\)",
                       rep_line[0])
        if not m2 or (int(m2.group(1)), int(m2.group(2)), int(m2.group(3))) != \
                (doc["notes"], doc["wobbles"], doc["fallbacks"]):
            return "json-shape", (
                f"repair line counters disagree with json: {rep_line[0]!r}")
    if (len(doc["phantoms"]) > 0) != ("phantom call" in plain_text):
        return "json-shape", ("phantom rendering disagrees with phantoms "
                              "array")
    style_ptr = [l for l in lines
                 if l.startswith("style (owned by") and "style finding(s)" in l]
    if style_ptr:
        m3 = re.search(r": (\d+) style finding\(s\)", style_ptr[0])
        if not m3 or int(m3.group(1)) != s_style:
            return "json-shape", (
                "style pointer count disagrees with summary style count")
    elif s_style != 0:
        return "json-shape", (
            f"summary claims {s_style} style finding(s) but no pointer line")
    return None


def main():
    ap = argparse.ArgumentParser(description="S7 stage 2 differential fuzzer")
    ap.add_argument("--n", type=int, default=1200,
                    help="generated programs (default 1200; round multiples "
                         "of 12 to cover every bucket evenly)")
    ap.add_argument("--seed", type=int, default=20260930)
    ap.add_argument("--time-budget", type=float, default=600.0)
    ap.add_argument("--per-input-timeout", type=float, default=10.0)
    ap.add_argument("--bin", default=None)
    ap.add_argument("--oracle", default=ORACLE)
    ap.add_argument("--corpus-dir", default=os.path.join(ROOT, "fuzz_corpus"))
    ap.add_argument("--max-findings", type=int, default=20)
    args = ap.parse_args()

    binpath = find_binary(args.bin)
    if binpath is None:
        print("no operon binary found — cargo build first", file=sys.stderr)
        return 2
    if not os.path.isfile(args.oracle):
        print(f"oracle not found: {args.oracle}", file=sys.stderr)
        return 2
    os.makedirs(TMPDIR, exist_ok=True)
    os.makedirs(args.corpus_dir, exist_ok=True)
    manifest = os.path.join(args.corpus_dir, "MANIFEST.jsonl")

    n_buckets = len(gen_corpus.BUCKETS)
    print(f"fuzz_diff — S7 stage 2: differential at scale")
    print(f"  binary: {binpath}")
    print(f"  oracle: {args.oracle}")
    print(f"  seed {args.seed}, {args.n} program(s) across {n_buckets} buckets, "
          f"budget {args.time_budget:.0f}s")
    print(f"  contracts: D1/D2 rust==oracle (rc+stdout), D3 vm==tree "
          f"(rc+stdout+stderr), D4 check rc contract, D5 json-vs-plain "
          f"parity, R1 no panic/hang")

    deadline = time.time() + args.time_budget
    per_bucket = {}
    findings = 0
    seen_hashes = set()
    path = os.path.join(TMPDIR, "p.op")

    i = 0
    while i < args.n and time.time() < deadline and findings < args.max_findings:
        bucket, idx, text = gen_program(args.seed, i)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        stat = per_bucket.setdefault(bucket, {"programs": 0, "findings": 0})
        stat["programs"] += 1

        verdict = diff_finding(binpath, args.oracle, path,
                               args.per_input_timeout)
        if verdict:
            kind, detail = verdict
            h = hashlib.sha256(text.encode()).hexdigest()
            stat["findings"] += 1
            if h not in seen_hashes:
                seen_hashes.add(h)
                findings += 1
                name = f"diff_{args.seed}_{i}_{kind}.op"
                fpath = os.path.join(args.corpus_dir, name)
                with open(fpath, "w", encoding="utf-8") as fh:
                    fh.write(text)
                with open(manifest, "a", encoding="utf-8") as fh:
                    fh.write(json.dumps({
                        "kind": kind,
                        "tool": "fuzz_diff",
                        "input": os.path.relpath(fpath, ROOT),
                        "bucket": bucket,
                        "seed": args.seed,
                        "idx": i,
                        "detail": detail,
                        "repro": (f"python3 scripts/fuzz/fuzz_diff.py "
                                  f"--seed {args.seed} --n {i + 1}"),
                    }) + "\n")
                print(f"  FINDING #{findings} [{kind}] {detail} | "
                      f"bucket={bucket} seed={args.seed} idx={i} | "
                      f"saved {name}")
        i += 1
        if i % 200 == 0:
            print(f"    ... {i}/{args.n} programs, "
                  f"{time.time() - (deadline - args.time_budget):.0f}s, "
                  f"findings={findings}")

    if os.path.isfile(path) and findings == 0:
        os.remove(path)
    try:
        os.rmdir(TMPDIR)
    except OSError:
        pass

    elapsed = args.time_budget - (deadline - time.time())
    covered = ", ".join(f"{b}:{s['programs']}" + (
        f"({s['findings']}f)" if s["findings"] else "")
        for b, s in per_bucket.items())
    print(f"\nfuzz_diff done: {i} program(s) x 5 surfaces in {elapsed:.0f}s, "
          f"seed {args.seed}")
    print(f"  per bucket: {covered}")
    if findings:
        print(f"  unique findings saved: {findings} "
              f"({os.path.relpath(args.corpus_dir, ROOT)}/, manifest: "
              f"MANIFEST.jsonl)")
        print("  findings are BUGS to triage (see scripts/fuzz/TRIAGE.md), "
              "not a score.")
        return 1
    print("  no findings — engines and checker agree at generated-program "
          "scale")
    return 0


if __name__ == "__main__":
    sys.exit(main())
