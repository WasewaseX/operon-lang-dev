#!/usr/bin/env python3
# fuzz_csv_equivalence.py — randomized differential: csv_parse (hybrid) vs
# csv_parse_machine (reference) over adversarial random documents built from
# {a, sep, quote, CR, LF}. Any mismatch prints the document and both outputs.
import random
import subprocess
import sys
import os

ROOT = "/home/z/my-project/operon-lang-dev"
os.chdir(ROOT)

def op_str(s):
    # build an Operon string literal: no escapes except via chr()
    out = []
    for ch in s:
        if ch == '"':
            out.append('\\"')
        elif ch == "\n":
            out.append('\\n')
        elif ch == "\r":
            out.append('chr(13)')
        elif ch == "\\":
            out.append('\\\\')
        else:
            out.append(ch)
    # chr(13) pieces must be concatenated
    parts = out
    expr = ""
    first = True
    buf = ""
    for p in parts:
        if p == 'chr(13)':
            if buf:
                expr += ('"' + buf + '"' if first else ' + "' + buf + '"')
                buf = ""
                first = False
            expr += ('chr(13)' if first else ' + chr(13)')
            first = False
        else:
            buf += p
    if buf:
        expr += ('"' + buf + '"' if first else ' + "' + buf + '"')
    if not expr:
        expr = '""'
    return expr

def gen_doc(rng):
    alphabet = ["a", "b", ",", '"', "\r", "\n"]
    n = rng.randrange(0, 24)
    # bias toward quotes and newlines to hit the tricky corners
    weights = [3, 3, 3, 4, 1, 3]
    return "".join(rng.choices(alphabet, weights=weights, k=n))

def main():
    rng = random.Random(20261004)
    docs = [gen_doc(rng) for _ in range(300)]
    # plus the systematic corners
    docs += [
        '"a""b"', '"a""', 'a""b', '""', '""""', '"a""b"c', 'a,"",b',
        '"x\ny"\n', '"x\ny"', '"x\ny"\nz\n', 'a\r\n"b\r\nc"\r\n',
        '"a,b"\n"c,d"\n', '"a"b"c\n', 'a"b"c\n', '"\n"', '"\n', '\n"',
        'a,,\n,b\n', '"",\n', ',""\n', '"a"\n"a"\n', 'a"b\n"c"d\n',
        '"a" "b"\n', '"a",""\n', '"",\n"",\n', '"a"""\n', '"""a\n',
    ]
    ok = 0
    fails = []
    batch = 40
    for start in range(0, len(docs), batch):
        chunk = docs[start:start + batch]
        lines = ["use std/csv as c", "gene run() {"]
        for idx, d in enumerate(chunk):
            e = op_str(d)
            lines.append(f'    if c.csv_parse({e}, ",") != c.csv_parse_machine({e}, ",") {{')
            lines.append(f'        print("MISMATCH {start + idx}")')
            lines.append(f'        print(c.csv_parse({e}, ","))')
            lines.append(f'        print(c.csv_parse_machine({e}, ","))')
            lines.append("    }")
        lines.append("    print(\"BATCH-OK\")")
        lines.append("}")
        lines.append("run()")
        open("/tmp/fuzz.op", "w").write("\n".join(lines) + "\n")
        r = subprocess.run(["./bin/operon", "run", "--vm", "/tmp/fuzz.op"],
                           capture_output=True, text=True)
        if "MISMATCH" in r.stdout or "MISMATCH" in r.stderr:
            for idx_l in r.stdout.splitlines():
                pass
            fails.append((chunk, r.stdout, r.stderr))
        else:
            ok += len(chunk)
    if fails:
        chunk, out, err = fails[0]
        print("FAILURES PRESENT — first batch:")
        print(out[:2000])
        print(err[:500])
        for i, d in enumerate(chunk):
            print(f"{i}: {d!r}")
        return 1
    print(f"fuzz: {ok} documents, hybrid == machine everywhere")
    return 0

if __name__ == "__main__":
    sys.exit(main())
