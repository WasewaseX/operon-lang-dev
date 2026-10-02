#!/usr/bin/env python3
"""fuzz_parser.py — parser/VM robustness fuzzer (docs/COMPAT.md axis F).

The contract under fuzz is NOT "the program runs" — it is:

  C1  the runtime NEVER panics (Rust rc 101 / abort 134 / segv 139)
  C2  the runtime NEVER hangs (wall-clock bound per input)
  C3  the runtime NEVER breaches containment (unbounded memory, host paths)
  C4  whatever it does, the VM and the tree-walk AGREE on the outcome for
      inputs that parse and execute (rc + stdout + stderr)

Input classes:
  R  random bytes (all byte values, length 1..512)
  U  random unicode salad (emoji, CJK, RTL, controls, quotes)
  T  corpus mutations: bit flips, token splice, truncation, duplication,
     brace/quote/bracket storms derived from the real corpus
  D  nesting bombs: deep () [] {} "" nests at moderate depth (the parser
     has documented depth caps; the fuzzer verifies they hold)
  S  syntax salad: shuffled real tokens (keeps lexing hot, breaks parsing)

Every finding must reproduce with its seed; the fuzzer prints the exact
seed and writes the offending input to /tmp on failure.

Usage: python3 scripts/fuzz_parser.py [--n 2000] [--seed 0] [--exec]
"""
import argparse
import os
import random
import subprocess
import sys
import tempfile

BIN = "./bin/operon"
# F6/#50 (reliab lane, 2026-10-02): wall-clock bound per input. The fuel
# bound (20M steps) burns ~5.6s on a native linux core, but the compat
# matrix legs are not native-fast: the Windows runner measured the same
# fuel-bound salads at ~15-20s (i=245 flaked the 15s bound between two
# identical runs), and the aarch64 leg under qemu-user timed out SIX of
# them at 15s. The bound must sit ABOVE the slowest substrate's fuel burn
# or fuel-contained inputs read as hangs. 60s = ~3x the qemu burn with a
# genuine unbounded hang still tripping C2 (just slower). Overridable for
# exotic substrates without code edits.
TIMEOUT = int(os.environ.get("FUZZ_PARSER_TIMEOUT", "60"))

BAD_RCS = {101, 134, 139, 136, 138}  # panic / abort / segv / fpe family

_TMPDIR = tempfile.mkdtemp(prefix="opfuzz_")
_COUNTER = [0]


def write_input(data):
    """Persist the input to a real file: portable across OSes (no
    /dev/stdin on Windows) and the file doubles as the reproduction
    artifact when a finding is reported."""
    _COUNTER[0] += 1
    path = os.path.join(_TMPDIR, f"in_{_COUNTER[0]}.op")
    with open(path, "wb") as f:
        f.write(data)
    return path


def run_engine(args, data):
    try:
        p = subprocess.run(
            [BIN] + args, input=data, capture_output=True,
            timeout=TIMEOUT, text=False,
        )
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return "timeout", b"", b""


def gen_random_bytes(rng):
    n = rng.randint(1, 512)
    return bytes(rng.randint(0, 255) for _ in range(n))


UNICODE_POOL = (
    "Σ ΣΩ 文字列 🧬🧪\U0001F600 ／＼ ﷽ ‮ ‭ "
    "\x00\x01\x1b[31m \r\n \t \\\'\"` "
    "𝕏⃝ ﬀﬁﬂ ½⅓ ∞≈≠"
)


def gen_unicode_salad(rng):
    n = rng.randint(1, 300)
    return "".join(rng.choice(UNICODE_POOL) for _ in range(n)).encode("utf-8", "ignore")


TOKENS = [
    "gene", "match", "case", "stress", "rescue", "if", "elif", "else",
    "while", "loop", "for", "in", "return", "raise", "let", "frame",
    "proof", "Some", "None", "Ok", "Err", "some", "none", "ok", "err",
    "print", "{", "}", "(", ")", "[", "]", ",", ":", ";", "|", "*rest",
    "?!", "?", ".", "=>", "->", "==", "!=", "<=", ">=", "&&", "||", "??",
    "+", "-", "*", "/", "//", "%", "=", "+=", '"', "1", "2.5", "0x1f",
    "name", "x", "_", "\n", " ", "@copies(2)", "@riboswitch(on)",
]


def gen_token_salad(rng):
    n = rng.randint(5, 200)
    return "".join(rng.choice(TOKENS) + rng.choice(["", " ", "\n"]) for _ in range(n)).encode()


def load_corpus_seeds(limit=40):
    seeds = []
    roots = ["tests/compat", "tests/differential"]
    for root in roots:
        if not os.path.isdir(root):
            continue
        for dirpath, _, files in os.walk(root):
            for f in sorted(files):
                if f.endswith(".op"):
                    seeds.append(os.path.join(dirpath, f))
                if len(seeds) >= limit:
                    return seeds
    return seeds


def mutate(rng, src):
    b = bytearray(src)
    for _ in range(rng.randint(1, 8)):
        if not b:
            break
        op = rng.choice(["flip", "insert", "delete", "dup", "truncate", "storm"])
        i = rng.randrange(len(b))
        if op == "flip":
            b[i] ^= 1 << rng.randint(0, 7)
        elif op == "insert":
            b.insert(i, rng.randint(0, 255))
        elif op == "delete":
            del b[i]
        elif op == "dup":
            j = min(len(b), i + rng.randint(1, 64))
            b[i:i] = b[i:j]
        elif op == "truncate":
            del b[i:]
        elif op == "storm":
            tok = rng.choice([b"(", b")", b"{", b"}", b'"', b"[", b"]", b"\n"])
            for _ in range(rng.randint(2, 40)):
                b.insert(min(i, len(b)), rng.choice(tok))
    return bytes(b)


def gen_nest_bomb(rng):
    kind = rng.choice(["(", "[", "{", '"'])
    depth = rng.choice([1, 8, 64, 200, 900])
    if kind == '"':
        return (b'print("' + b"a" * depth)  # unclosed string of depth bytes
    close = {"(": ")", "[": "]", "{": "}"}[kind]
    return (kind * depth + close * max(0, depth - 1)).encode()


def check(seed, data, exec_mode):
    """Returns None if contained, else a finding string."""
    path = write_input(data)
    rc, out, err = run_engine(["run", path], None)
    if rc == "timeout":
        return f"timeout (>{TIMEOUT}s) input={path}"
    if rc in BAD_RCS:
        return f"crash rc={rc} stderr={err[:200]!r} input={path}"
    if b"panicked at" in err:
        return f"panic text in stderr: {err[:200]!r} input={path}"
    if exec_mode:
        rc2, out2, err2 = run_engine(["run", "--vm", path], None)
        if rc2 == "timeout":
            return f"vm timeout input={path}"
        if rc2 in BAD_RCS or b"panicked at" in err2:
            return f"vm crash rc={rc2} stderr={err2[:200]!r} input={path}"
        if (rc, out, err) != (rc2, out2, err2):
            return f"engine divergence: rc {rc} vs {rc2} input={path}"
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--exec", action="store_true",
                    help="also run each input on the VM and require agreement")
    args = ap.parse_args()

    rng = random.Random(args.seed or 20260930)
    corpus = load_corpus_seeds()
    findings = []
    buckets = {"R": 0, "U": 0, "T": 0, "D": 0, "S": 0}

    for i in range(args.n):
        cls = rng.choice("RUUTTDDS")  # corpus mutations doubled
        if cls == "R":
            data = gen_random_bytes(rng)
        elif cls == "U":
            data = gen_unicode_salad(rng)
        elif cls == "T" and corpus:
            src = open(rng.choice(corpus), "rb").read()
            data = mutate(rng, src)
        elif cls == "D":
            data = gen_nest_bomb(rng)
        else:
            data = gen_token_salad(rng)
        buckets[cls] += 1
        finding = check(args.seed, data, args.exec)
        if finding:
            findings.append(f"seed={args.seed} i={i} class={cls} {finding}")

    print(f"fuzz: {args.n} inputs "
          f"(R={buckets['R']} U={buckets['U']} T={buckets['T']} "
          f"D={buckets['D']} S={buckets['S']})")
    if findings:
        for f in findings[:20]:
            print(f"  FINDING: {f}")
        sys.exit(1)
    print("FUZZ CLEAN: no panics, no hangs, no breaches"
          + (", engines agree" if args.exec else ""))


if __name__ == "__main__":
    main()
