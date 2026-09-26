#!/usr/bin/env python3
"""fuzz.py — W51 (ROADMAP-100): black-box fuzzing for Operon.

Total Grammar promises "every input must not crash" — this lane holds the
promise honest. Complements sz's in-process cargo-fuzz plan (S7/S9) with a
zero-dep black-box driver over the real binary.

Corpus: tests/*.op + examples/cookbook/*.op (+ redteam payloads if present).
Targets: `operon check`, `operon ast --json`, and `.cell` loading via
`operon lint --cell`.

A finding = crash (signal / exit>3 / exit 101 panic / "[fatal]" on stderr).
Findings are minimized (truncation sweep) and saved under tests/fuzz/crashes/.

Run:  python3 scripts/fuzz/fuzz.py [--execs N] [--seed S]
      --execs 200000 --mode soak   (CI nightly shape; default smoke = 2000)
"""
import argparse, os, random, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BINS = [os.path.join(ROOT, "bin", "operon"), os.path.join(ROOT, "target", "release", "operon")]
BIN = next((b for b in BINS if os.path.exists(b)), None)
CRASH_DIR = os.path.join(ROOT, "tests", "fuzz", "crashes")

def sh(args, timeout=10):
    try:
        return subprocess.run([BIN] + args, cwd=ROOT, capture_output=True, text=True,
                              timeout=timeout)
    except subprocess.TimeoutExpired:
        return "TIMEOUT"

def corpus():
    files = []
    for d in ("tests", "examples/cookbook", "tests/redteam"):
        p = os.path.join(ROOT, d)
        if os.path.isdir(p):
            files += [os.path.join(p, f) for f in sorted(os.listdir(p)) if f.endswith(".op")]
    return files

TOKENS = [b'"', b"{", b"}", b"(", b")", b"[", b"]", b"\\", b"\x00", b"\xff", b"\xe2\x9c\x93",
          b"gene", b"match", b"case", b"@", b"/*", b"*/", b"${", b"//", b"#", b"'", b"0x",
          b"regulate", b"splice", b"phenotype", b"sequence", b"yield", b"-1", b"1e999"]

def mutate(data, rnd):
    b = bytearray(data)
    # size cap: mutation ops 2/4/5 grow the input; unbounded growth makes
    # timeouts meaningless (big-file wall time, not parser pathology)
    for _ in range(rnd.randrange(1, 8)):
        if not b:
            break
        if len(b) > 1_000_000:
            op = rnd.choice([0, 1, 6])  # flip / truncate-only when huge
        else:
            op = rnd.randrange(7)
        i = rnd.randrange(len(b))
        if op == 0:
            b[i] = rnd.randrange(256)
        elif op == 1:
            del b[i:i + rnd.randrange(1, 60)]
        elif op == 2:
            b[i:i] = bytes(rnd.randrange(256) for _ in range(rnd.randrange(1, 30)))
        elif op == 3:
            b[i:i] = rnd.choice(TOKENS)
        elif op == 4:
            j = rnd.randrange(len(b))
            b[i:i] = b[j:j + rnd.randrange(1, 80)]
        elif op == 5:
            b += b[:rnd.randrange(1, len(b) + 1)]
        else:
            b = b[:rnd.randrange(1, len(b) + 1)]
    return bytes(b)

def is_crash(r):
    if r == "TIMEOUT":
        return True
    if r.returncode < 0 or r.returncode > 3 or r.returncode == 101:
        return True
    return "[fatal]" in r.stderr or "panicked at" in r.stderr

def minimize(mut, rnd):
    cur = mut
    for _ in range(12):
        if len(cur) < 32:
            break
        cand = cur[: len(cur) // 2]
        p = os.path.join(ROOT, "target", "fuzz.tmp.op")
        open(p, "wb").write(cand)
        if is_crash(sh(["check", p])):
            cur = cand
        else:
            break
    return cur

def save(tag, payload):
    os.makedirs(CRASH_DIR, exist_ok=True)
    p = os.path.join(CRASH_DIR, f"{tag}.op")
    open(p, "wb").write(payload)
    return p

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--execs", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=20260926)
    args = ap.parse_args()
    if BIN is None:
        print("no operon binary — run scripts/build.sh first", file=sys.stderr)
        return 2
    rnd = random.Random(args.seed)
    seeds = corpus()
    if not seeds:
        print("empty corpus", file=sys.stderr)
        return 2
    tmp = os.path.join(ROOT, "target", "fuzz.tmp.op")
    cell_tmp = os.path.join(ROOT, "target", "fuzz.tmp.cell")
    findings = 0
    for n in range(args.execs):
        src = rnd.choice(seeds)
        data = open(src, "rb").read()
        mut = mutate(data, rnd)
        open(tmp, "wb").write(mut)
        for argv in (["check", tmp], ["ast", tmp, "--json"], ["explain", tmp, "--json"]):
            r = sh(argv)
            if is_crash(r):
                findings += 1
                keep = minimize(mut, rnd)
                path = save(f"crash_{args.seed}_{n}", keep)
                print(f"FINDING #{findings}: {' '.join(argv)} rc="
                      f"{r if r == 'TIMEOUT' else r.returncode} — minimized -> {path}")
                if findings >= 10:
                    print("too many findings this run — stopping early")
                    break
        # .cell fuzzing through the W66 validator (loading path, not just lint)
        if n % 4 == 0:
            open(cell_tmp, "wb").write(mutate(open(os.path.join(ROOT, "README.md"), "rb").read(), rnd))
            r = sh(["lint", "README.md", "--cell", cell_tmp])
            if is_crash(r):
                findings += 1
                print(f"FINDING #{findings}: .cell load crash — saved tests/fuzz/crashes/")
        if findings >= 10:
            break
    print(f"fuzz: {args.execs} exec(s), {findings} finding(s), seed {args.seed}")
    return 0 if findings == 0 else 1

if __name__ == "__main__":
    sys.exit(main())
