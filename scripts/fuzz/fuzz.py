#!/usr/bin/env python3
"""fuzz.py — W51 (M100 batch 2): mutation-based black-box fuzzing for Operon.

Total Grammar promises "every input must not crash" — this lane holds the
promise honest against INPUTS NOBODY WROTE YET. Mutation-based (not coverage
guided, deliberately: zero dependencies, runs anywhere the binary runs).

Corpus (small files only, cap per file):  tests/*.op + tests/redteam/*.op
+ examples/**/*.op + std/*.op.  Redteam payloads are legal seeds: they are
run through PARSE-ONLY tooling (never executed), and a contained diagnostic
from one is the EXPECTED outcome, never a finding.

Targets:  operon check <f>   |  operon ast <f> --json   |  operon fmt <f>
          (parser + tooling surface; `explain` is a batch-2 WIP lane and is
          deliberately excluded this cycle to keep findings attributable;
          `run`/spawn paths stay with scripts/redteam.sh).

Mutations (chain is tracked per input): byte flip, truncation, chunk
duplication, quote/bracket token injection, unicode splices, random-byte
insertion, head-clone extension. Output size is capped so a per-input timeout
measures parser pathology, not file size.

Classification (the only vocabulary this tool reports):
  clean      rc 0, no panic text
  contained  rc in {1,2,3} — the language's OWN diagnostic (uncaught stress /
             parse / check); redteam payloads land here BY DESIGN. Never a
             finding, never a number to brag about.
  crash      signal (rc < 0), rc 101, any other rc, or panic/fatal text on
             stderr — a FINDING.
  hang       exceeds the per-input timeout — a FINDING.

Findings are saved to fuzz_corpus/<kind>_<seed>_<exec>.op (truncation-sweep
minimized when possible) with one JSON line each in fuzz_corpus/MANIFEST.jsonl:
input, seed file, mutation chain, argv, exit code, stderr head.

Run:  python3 scripts/fuzz/fuzz.py [--time-budget 120] [--execs N] [--seed S]
Exit: 0 no findings, 1 findings (they are SAVED — triage them, do not gloat).
"""
import argparse
import hashlib
import json
import os
import random
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DEFAULT_CORPUS_DIR = os.path.join(ROOT, "fuzz_corpus")
TMP = os.path.join(ROOT, "target", "fuzz.tmp.op")
MAX_MUTATED_BYTES = 512 * 1024  # beyond this only flip/truncate: keep timeouts honest
PANIC_TEXTS = ("panicked at", "stack overflow", "fatal runtime error", "[fatal]")
CONTAINED_RCS = {0, 1, 2, 3}


def find_binary(explicit):
    cands = [
        explicit,
        os.environ.get("OPERON_BIN"),
        os.path.join(ROOT, "target", "debug", "operon"),
        os.path.join(ROOT, "bin", "operon"),
        os.path.join(ROOT, "target", "release", "operon"),
    ]
    for c in cands:
        if c and os.path.isfile(c) and os.access(c, os.X_OK):
            return c
    return None


# ------------------------------------------------------------ corpus

def build_corpus(max_file_bytes):
    """tests/*.op (top level) + tests/redteam/*.op + examples/**/*.op + std/*.op,
    small files only, deduped, sorted (deterministic)."""
    roots = [os.path.join(ROOT, "tests"),          # top level only
             os.path.join(ROOT, "tests", "redteam"),
             os.path.join(ROOT, "examples"),       # recursive (cookbook, ...)
             os.path.join(ROOT, "std")]
    files, skipped = {}, 0
    for i, r in enumerate(roots):
        if not os.path.isdir(r):
            continue
        if i == 2:  # examples: recurse
            for dirpath, _, names in os.walk(r):
                for n in sorted(names):
                    p = os.path.join(dirpath, n)
                    if n.endswith(".op"):
                        files[os.path.realpath(p)] = p
        else:
            for n in sorted(os.listdir(r)):
                p = os.path.join(r, n)
                if n.endswith(".op") and os.path.isfile(p):
                    files[os.path.realpath(p)] = p
    out = []
    for p in sorted(files.values()):
        try:
            if os.path.getsize(p) <= max_file_bytes:
                out.append(p)
            else:
                skipped += 1
        except OSError:
            skipped += 1
    return out, skipped


# ------------------------------------------------------------ mutations

TOKENS = [b'"', b"'", b"{", b"}", b"(", b")", b"[", b"]", b"\\", b"${", b"/*",
          b"*/", b"//", b"#", b"\x00", b"\xff", b"gene", b"match", b"case",
          b"frame", b"@methylate", b"-1", b"1e999", b"0x"]
UNICODE_SPLICES = ["\u00e9", "\u03c3", "\u042f", "\u5b57", "\U0001f9ec",
                   "\u0301", "\u200d", "\U0001f600", "\ufe0f", "\u2028"]


def mutate(data, rnd):
    """Returns (mutated_bytes, chain). Chain entries are human-readable."""
    b = bytearray(data)
    chain = []
    for _ in range(rnd.randrange(1, 8)):
        if not b:
            break
        if len(b) > MAX_MUTATED_BYTES:
            op = rnd.choice(["flip", "truncate"])
        else:
            op = rnd.choice(["flip", "truncate", "dup_chunk", "insert_bytes",
                             "inject_token", "unicode_splice", "extend"])
        i = rnd.randrange(len(b))
        if op == "flip":
            b[i] ^= 1 << rnd.randrange(8)
            chain.append(f"flip@{i}")
        elif op == "truncate":
            keep = rnd.randrange(1, len(b) + 1)
            del b[keep:]
            chain.append(f"truncate:{keep}")
        elif op == "dup_chunk":
            j = rnd.randrange(len(b))
            ln = rnd.randrange(1, 80)
            b[i:i] = b[j:j + ln]
            chain.append(f"dup_chunk@{i}<-{j}+{ln}")
        elif op == "insert_bytes":
            blob = bytes(rnd.randrange(256) for _ in range(rnd.randrange(1, 30)))
            b[i:i] = blob
            chain.append(f"insert_bytes@{i}+{len(blob)}")
        elif op == "inject_token":
            t = rnd.choice(TOKENS)
            b[i:i] = t
            chain.append(f"inject_token@{i}:{t.decode('latin1')}")
        elif op == "unicode_splice":
            u = rnd.choice(UNICODE_SPLICES).encode()
            b[i:i] = u
            chain.append(f"unicode_splice@{i}+{len(u)}")
        else:  # extend: clone the head (unbalanced-quote / depth storms)
            b += b[:rnd.randrange(1, len(b) + 1)]
            chain.append("extend_head")
    return bytes(b), chain


# ------------------------------------------------------------ engine probing

def probe(argv, timeout):
    try:
        p = subprocess.run(argv, cwd=ROOT, capture_output=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired as e:
        head = (e.stderr or b"")[:512]
        return "TIMEOUT", e.stdout or b"", head


def classify(rc, stderr):
    """Returns (kind, detail). kind in crash / hang / contained / clean."""
    if rc == "TIMEOUT":
        return "hang", "per-input timeout"
    if isinstance(rc, int) and rc < 0:
        return "crash", f"signal {-rc}"
    if rc not in CONTAINED_RCS:
        return "crash", f"exit {rc}"
    blob = (stderr or b"").decode("utf-8", errors="replace")
    for t in PANIC_TEXTS:
        if t in blob:
            return "crash", f"panic text {t!r}"
    return ("clean", "rc 0") if rc == 0 else ("contained", f"rc {rc}")


def minimize(binpath, payload, argv, timeout, rounds=12):
    """Truncation sweep: halve while the finding survives. Cheap, not smart."""
    cur = payload
    for _ in range(rounds):
        if len(cur) < 32:
            break
        cand = cur[: len(cur) // 2]
        with open(TMP, "wb") as fh:
            fh.write(cand)
        rc, _, err = probe([binpath] + argv[1:], timeout)
        kind, _ = classify(rc, err)
        if kind in ("crash", "hang"):
            cur = cand
        else:
            break
    return cur


# ------------------------------------------------------------ main

def main():
    ap = argparse.ArgumentParser(description="W51 mutation-based fuzzer")
    ap.add_argument("--time-budget", type=float, default=120.0,
                    help="wall-clock budget in seconds (default 120)")
    ap.add_argument("--execs", type=int, default=200000,
                    help="hard cap on exec count")
    ap.add_argument("--seed", type=int, default=20260926)
    ap.add_argument("--per-input-timeout", type=float, default=10.0)
    ap.add_argument("--corpus-dir", default=DEFAULT_CORPUS_DIR)
    ap.add_argument("--max-file-bytes", type=int, default=65536,
                    help="seed files larger than this are skipped")
    ap.add_argument("--max-findings", type=int, default=20)
    ap.add_argument("--bin", default=None)
    ap.add_argument("--no-minimize", action="store_true")
    args = ap.parse_args()

    binpath = find_binary(args.bin)
    if binpath is None:
        print("no operon binary found — cargo build first", file=sys.stderr)
        return 2

    seeds, skipped = build_corpus(args.max_file_bytes)
    if not seeds:
        print("empty corpus", file=sys.stderr)
        return 2
    os.makedirs(args.corpus_dir, exist_ok=True)
    manifest = os.path.join(args.corpus_dir, "MANIFEST.jsonl")

    print(f"fuzz — seed {args.seed}, budget {args.time_budget:.0f}s, "
          f"per-input timeout {args.per_input_timeout:.0f}s")
    print(f"  binary: {binpath}")
    print(f"  corpus: {len(seeds)} file(s) "
          f"(tests + tests/redteam + examples/** + std; {skipped} skipped > "
          f"{args.max_file_bytes // 1024} KiB)")
    print(f"  targets: check / ast --json / fmt / explain [--json] (parse-only; contained rc "
          f"1-3 is the EXPECTED redteam outcome, never a finding)")

    rnd = random.Random(args.seed)
    deadline = time.time() + args.time_budget
    stats = {"clean": 0, "contained": 0, "crash": 0, "hang": 0}
    seen_hashes = set()
    unique_findings = 0
    execs = 0

    while execs < args.execs and time.time() < deadline \
            and unique_findings < args.max_findings:
        src = rnd.choice(seeds)
        try:
            with open(src, "rb") as fh:
                data = fh.read()
        except OSError:
            continue
        mut, chain = mutate(data, rnd)
        with open(TMP, "wb") as fh:
            fh.write(mut)
        # S7 slice 1: `explain` joins the target row (it was excluded as a
        # batch-2 WIP lane; the surface is stable now and its --json shape is
        # contract-pinned). Parse-only by construction: explain never runs
        # the program, so redteam seeds stay contained here too.
        # z-fuzz-infra (#138.5): --write integrity leg on a SANDBOX COPY
        # (never repo paths). Invariant: when the tool exits non-zero
        # (parse failure), the target bytes must be UNCHANGED — the #113
        # class (fmt --write truncating unreadable/non-UTF-8 files to 0
        # bytes at exit 0) was invisible to every lane by construction
        # because no writing surface was ever fuzzed with before/after
        # hashes.
        WRITETMP = TMP + ".fuzzwrite"
        for argv in (["check", TMP], ["ast", TMP, "--json"], ["fmt", TMP],
                     ["explain", TMP], ["explain", TMP, "--json"]):
            rc, out, err = probe([binpath] + argv, args.per_input_timeout)
            kind, detail = classify(rc, err)
            stats[kind] += 1
            execs += 1
            # the --write leg itself: copy the current input, hash, run
            # fmt --write on the COPY, hash again — a non-zero exit must
            # leave the bytes identical (a zero exit may rewrite freely)
            if execs % 4 == 0:
                try:
                    import shutil
                    with open(WRITETMP, "wb") as wfh:
                        wfh.write(mut)
                    before = hashlib.sha256(mut).hexdigest()
                    wrc, wout, werr = probe([binpath, "fmt", WRITETMP, "--write"],
                                            args.per_input_timeout)
                    after_bytes = open(WRITETMP, "rb").read()
                    after = hashlib.sha256(after_bytes).hexdigest()
                    if isinstance(wrc, int) and wrc != 0 and after != before:
                        kind2, _ = classify(wrc, werr)
                        stats[kind2] += 1
                        unique_findings += 1
                        name = f"write_integrity_{args.seed}_{execs}.op"
                        path2 = os.path.join(args.corpus_dir, name)
                        with open(path2, "wb") as wfh:
                            wfh.write(mut)
                        with open(manifest, "a", encoding="utf-8") as wfh:
                            wfh.write(json.dumps({
                                "kind": "write-integrity",
                                "input": os.path.relpath(path2, ROOT),
                                "bytes": len(mut),
                                "seed": args.seed,
                                "exec": execs,
                                "argv": ["fmt", "<copy>", "--write"],
                                "exit_code": wrc,
                                "detail": f"rc!=0 but bytes changed: {before[:12]} -> {after[:12]}",
                            }))
                except OSError:
                    pass
            if kind in ("crash", "hang"):
                h = hashlib.sha256(mut).hexdigest()
                if h in seen_hashes:
                    continue  # same input already saved this run
                seen_hashes.add(h)
                unique_findings += 1
                saved = mut
                min_note = "not minimized"
                if not args.no_minimize:
                    saved = minimize(binpath, mut, [argv[0]] + argv[1:],
                                     args.per_input_timeout)
                    min_note = f"minimized {len(mut)} -> {len(saved)} bytes"
                name = f"{kind}_{args.seed}_{execs}.op"
                path = os.path.join(args.corpus_dir, name)
                with open(path, "wb") as fh:
                    fh.write(saved)
                with open(manifest, "a", encoding="utf-8") as fh:
                    fh.write(json.dumps({
                        "kind": kind,
                        "input": os.path.relpath(path, ROOT),
                        "bytes": len(saved),
                        "seed": args.seed,
                        "exec": execs,
                        "seed_file": os.path.relpath(src, ROOT),
                        "mutation_chain": chain,
                        "argv": argv,
                        "exit_code": rc if isinstance(rc, int) else "TIMEOUT",
                        "stderr_head": err.decode("utf-8", errors="replace")[:500],
                        "minimized": min_note,
                    }) + "\n")
                print(f"  FINDING #{unique_findings} [{kind}] {detail} | "
                      f"{os.path.relpath(src, ROOT)} | {' '.join(argv[1:])} | "
                      f"chain: {' > '.join(chain)} | saved {name} ({min_note})")
                if unique_findings >= args.max_findings:
                    break
        if execs % 100 < 3:
            print(f"    ... {execs} execs, {time.time() - (deadline - args.time_budget):.0f}s, "
                  f"clean={stats['clean']} contained={stats['contained']} "
                  f"crash={stats['crash']} hang={stats['hang']}")

    if os.path.exists(TMP):
        os.remove(TMP)

    elapsed = args.time_budget - (deadline - time.time())
    print(f"\nfuzz done: {execs} exec(s) in {elapsed:.0f}s, seed {args.seed}")
    print(f"  clean={stats['clean']}  contained={stats['contained']}  "
          f"crash={stats['crash']}  hang={stats['hang']}")
    if unique_findings:
        print(f"  unique findings saved: {unique_findings} "
              f"({os.path.relpath(args.corpus_dir, ROOT)}/, manifest: "
              f"{os.path.relpath(manifest, ROOT)})")
        print("  findings are BUGS to triage (see scripts/fuzz/TRIAGE.md), "
              "not a score.")
        return 1
    print("  no findings — manifest stays unwritten (nothing to triage)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
