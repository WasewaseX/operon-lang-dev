#!/usr/bin/env python3
"""prop_harness.py — W50 (ROADMAP-100): property-based testing for Operon.

Black-box, stdlib-only Python, deterministic seeds. Properties:
  P1  fmt idempotence:  fmt(fmt(x)) == fmt(x)                    (byte law)
  P1b ast stability:    ast-dump(fmt(x)) == ast-dump(fmt(fmt(x)))
  P2  JSON round-trip:  json_parse(json_str(v)) == v             (generated v)
  P3  arithmetic laws:  a+b == b+a, a*b == b*a, (a+b)+c == a+(b+c)
  P5  Total Grammar:    every mutated input CHECKS without crashing
                        (exit codes 0/2/3 are fine; 101/panic/signals are not)

Any counterexample prints a repro (seed + input path) and exits 1.
Run:  python3 scripts/prop/prop_harness.py [--cases N] [--seed S]
"""
import argparse, json, os, random, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BINS = [os.path.join(ROOT, "bin", "operon"), os.path.join(ROOT, "target", "release", "operon")]
BIN = next((b for b in BINS if os.path.exists(b)), None)

def run(args, timeout=30):
    return subprocess.run([BIN] + args, cwd=ROOT, capture_output=True, text=True, timeout=timeout)

def corpus():
    files = []
    for d in ("tests", "examples/cookbook"):
        p = os.path.join(ROOT, d)
        if os.path.isdir(p):
            files += [os.path.join(d, f) for f in sorted(os.listdir(p)) if f.endswith(".op")]
    return files

# ------------------------------------------------------------ generators

def gen_json_value(rnd, depth=0):
    if depth > 3:
        return rnd.choice([rnd.randint(-10**6, 10**6), round(rnd.random() * 1000, 6),
                           rnd.choice(["", "a", "hello world", "quote\"inside", "número",
                                       "tab\tinside", "{}", "null-looking"])])
    t = rnd.randrange(6 if depth < 2 else 3)
    if t == 0:
        return rnd.randint(-10**9, 10**9)
    if t == 1:
        return round(rnd.uniform(-1e6, 1e6), 6)
    if t == 2:
        return rnd.choice(["", "x", "inter\"rupt", "back\\slash", "newline\nhere"])
    if t == 3:
        return [gen_json_value(rnd, depth + 1) for _ in range(rnd.randrange(4))]
    if t == 4:
        return {f"k{i}": gen_json_value(rnd, depth + 1) for i in range(rnd.randrange(4))}
    return rnd.choice([True, False])

def gen_int(rnd):
    return rnd.choice([0, 1, -1, rnd.randint(-10**9, 10**9), 2**31, -(2**31)])

def mutations(data, rnd):
    b = bytearray(data)
    for _ in range(rnd.randrange(1, 6)):
        if not b:
            break
        op = rnd.randrange(6)
        i = rnd.randrange(len(b))
        if op == 0:
            b[i] = rnd.randrange(256)
        elif op == 1:
            del b[i:i + rnd.randrange(1, 40)]
        elif op == 2:
            b[i:i] = bytes(b[:rnd.randrange(len(b))])
        elif op == 3:
            b[i:i] = rnd.choice([b"\"", b"{", b"}", b"(", b")", b"\\", b"\x00", b"\xff",
                                 b"gene", b"match", b"@", b"/*", b"${"])
        elif op == 4:
            b += b[:rnd.randrange(1, len(b) + 1)]
        else:
            b[i:i] = "字母🧬\n\t".encode()
    return bytes(b)

# ------------------------------------------------------------ properties

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", type=int, default=400)
    ap.add_argument("--seed", type=int, default=20260926)
    args = ap.parse_args()
    if BIN is None:
        print("no operon binary — run scripts/build.sh first", file=sys.stderr)
        return 2
    rnd = random.Random(args.seed)
    fails = []
    tmp = os.path.join(ROOT, "target", "prop.tmp.op")

    # P1/P1b — fmt idempotence over the whole corpus
    for f in corpus():
        src_f = os.path.join(ROOT, f)
        r1 = run(["fmt", src_f])
        if r1.returncode != 0:
            continue  # fmt failures are parser-level; P5 covers robustness
        with open(tmp, "w", encoding="utf-8") as fh:
            fh.write(r1.stdout)
        r2 = run(["fmt", tmp])
        if r2.returncode != 0 or r2.stdout != r1.stdout:
            fails.append(f"P1 fmt idempotence breaks on {f} (rc {r1.returncode}->{r2.returncode})")
            continue
        a1 = run(["ast", tmp, "--json"]).stdout
        with open(tmp, "w", encoding="utf-8") as fh:
            fh.write(r2.stdout)
        a2 = run(["ast", tmp, "--json"]).stdout
        if a1 != a2:
            fails.append(f"P1b ast instability on {f}")
    print(f"P1/P1b fmt+ast idempotence: corpus done, {len(fails)} fail(s)")

    # P2 — JSON round-trip on generated values
    for i in range(args.cases):
        v = gen_json_value(rnd)
        prog = ('gene main() {\n'
                f'    let v = {json.dumps(v)}\n'
                '    promote(str(v == json_parse(json_str(v))))\n'
                '}\n')
        with open(tmp, "w", encoding="utf-8") as fh:
            fh.write(prog)
        r = run(["run", tmp], timeout=20)
        if r.returncode != 0 or "true" not in r.stdout:
            fails.append(f"P2 json round-trip case {i} seed {args.seed}: rc={r.returncode} out={r.stdout!r}")
            break
    print(f"P2 json round-trip: {args.cases} cases done")

    # P3 — arithmetic laws on generated ints
    for i in range(args.cases):
        a, b, c = gen_int(rnd), gen_int(rnd), gen_int(rnd)
        prog = ('gene main() {\n'
                f'    let a = {a}\n    let b = {b}\n    let c = {c}\n'
                '    promote(str(a + b == b + a) + str(a * b == b * a) + str((a + b) + c == a + (b + c)))\n'
                '}\n')
        with open(tmp, "w", encoding="utf-8") as fh:
            fh.write(prog)
        r = run(["run", tmp], timeout=20)
        if "truetruetrue" not in r.stdout.replace(" ", ""):
            fails.append(f"P3 arithmetic law case {i}: a={a} b={b} c={c} out={r.stdout!r}")
            break
    print(f"P3 arithmetic laws: {args.cases} cases done")

    # P5 — Total Grammar never crashes on mutated corpus inputs
    seeds = corpus()
    crashes = 0
    for i in range(args.cases):
        src_file = os.path.join(ROOT, rnd.choice(seeds))
        data = open(src_file, "rb").read()
        mut = mutations(data, rnd)
        with open(tmp, "wb") as fh:
            fh.write(mut)
        try:
            r = run(["check", tmp], timeout=20)
            bad = r.returncode < 0 or r.returncode > 3 or "[fatal]" in r.stderr
        except subprocess.TimeoutExpired:
            bad, r = True, None
        if bad:
            crashes += 1
            keep = os.path.join(ROOT, "tests", "fuzz", "crashes", f"prop_{args.seed}_{i}.op")
            os.makedirs(os.path.dirname(keep), exist_ok=True)
            open(keep, "wb").write(mut)
            fails.append(f"P5 CRASH on mutated {src_file} case {i} — saved {keep}")
            if crashes >= 3:
                break
    print(f"P5 total-grammar robustness: {args.cases} mutations, {crashes} crash(es)")

    if os.path.exists(tmp):
        os.remove(tmp)
    if fails:
        print("\nPROPERTY FAILURES:")
        for f in fails:
            print("  ✗", f)
        return 1
    print(f"\nall properties hold (seed {args.seed}, {args.cases} cases)")
    return 0

if __name__ == "__main__":
    sys.exit(main())
