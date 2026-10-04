#!/usr/bin/env python3
# gen_fixture.py — deterministic CSV fixtures for apps/csvstat.
#
# Both fixtures are generated from the LCG below (seed 42); regenerating
# must be byte-identical. Edge coverage baked in: a quoted field with a
# comma ("gpu, pro"), a field with doubled quotes (say "hi"), a field
# with surrounding spaces (verbatim contract — csvstat never trims),
# and empty notes (the null class).
LCG = 42


def nxt():
    global LCG
    LCG = (1103515245 * LCG + 12345) % 2147483648
    return LCG


REGIONS = ["north", "south", "east", "west"]
PRODUCTS = ["alpha", "beta", "gamma", "delta", "gpu, pro", "epsilon", "zeta", "eta boost"]
NOTES = ["ok", "ok", "ok", "recheck", "ok", " expedited ", 'say "hi"', "ok", "backorder", ""]


def esc(v):
    if any(ch in v for ch in [",", '"', "\n"]):
        return '"' + v.replace('"', '""') + '"'
    return v


# osid is a GLOBAL stream: it is initialized ONCE and advances across both
# fixture generations. The reproducibility contract is: generate small THEN
# big, in one process, from a fresh interpreter — always the same bytes.
OSID = 0


def gen(nrows, path):
    global OSID
    lines = ["order_id,region,product,qty,unit_cents,note"]
    for _ in range(nrows):
        OSID += 1
        r = REGIONS[nxt() % 4]
        p = PRODUCTS[nxt() % 8]
        q = 1 + nxt() % 9
        c = 100 + nxt() % 9801
        note = NOTES[nxt() % 10]
        lines.append(f"{OSID},{r},{esc(p)},{q},{c},{esc(note)}")
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    root = __file__.rsplit("/test/", 1)[0]
    OSID = 1000 + nxt() % 5000
    gen(40, f"{root}/test/data/small.csv")
    gen(5000, f"{root}/test/data/big.csv")
    print("fixtures written")
