#!/usr/bin/env python3
"""gen_corpus.py — the Operon compatibility corpus generator.

Reliability push (docs/COMPAT.md): thousands of deterministic, randomized
Operon programs. Every program is self-contained, prints only deterministic
values (no clock, no rng, no threads, no wall-time builtins), and is run
through every engine pairing by scripts/compat_matrix.sh:

    tree-walk == VM == VM-opt (Rust, release)
    tree-walk == VM          (Rust, debug)
    tree-walk == Python oracle

Determinism contract for generated programs:
  - no rand/time/now/serve/recv/repressi builtins
  - no spawns or sequences (thread-cap trip order is load-dependent)
  - bounded loops (< 200 iterations), bounded recursion (depth <= 16)
  - allocation bounded (strings < 1 KiB, lists < 200 items)
  - stresses only where deliberately wrapped in stress/rescue with
    deterministic kind/message probes

Usage:
    python3 scripts/gen_corpus.py --seed 20260930 --count 1200 --out tests/compat

Incremental extension (appends new programs to an existing corpus without
overwriting: per-bucket numbering continues at idx = start // len(BUCKETS)):

    python3 scripts/gen_corpus.py --seed 20261030 --start 1200 --count 2004 \
        --out tests/compat

Manifest rebuild from the committed file set (parses each program's header
comment for bucket/seed/idx, so the manifest always matches the files):

    python3 scripts/gen_corpus.py --manifest-scan --out tests/compat
"""
import argparse
import json
import os
import random
import sys

BUCKETS = [
    "arith", "cmplogic", "strings", "lists", "maps", "control",
    "funcs", "matchpat", "optres", "stressfail", "nums", "mixed",
]

MAX_I64 = 9223372036854775807

WORDS = ["gene", "cell", "codon", "exon", "helix", "plasmid", "ribose",
         "enzyme", "ligase", "marker", "splicer", "vector"]


def rint(rng, lo=-10000, hi=10000):
    return rng.randint(lo, hi)


def rexpr_int(rng, depth=0):
    """Small integer expression tree: + - * // % & | ^ shifts unary."""
    if depth >= 3 or rng.random() < 0.3:
        return str(rint(rng, -500, 500))
    op = rng.choice(["+", "-", "*", "//", "%", "&", "|", "^", "<<", ">>"])
    a = rexpr_int(rng, depth + 1)
    b = rexpr_int(rng, depth + 1)
    if op in ("<<", ">>"):
        return f"({a} {op} {rng.randint(0, 5)})"
    if op in ("//", "%"):
        return f"({a} {op} ({rng.choice(['3', '7', '12', '100', str(rint(rng, 1, 97))])}))"
    if op == "*":
        return f"({a} * {rng.choice(['2', '3', '5', str(rint(rng, -20, 20))])})"
    return f"({a} {op} {b})"


def rfloat(rng):
    return f"{rng.uniform(-100, 100):.{rng.randint(1, 3)}f}"


def rexpr_num(rng, depth=0):
    """Numeric expression possibly mixing ints and floats."""
    if depth >= 2 or rng.random() < 0.4:
        return rng.choice([str(rint(rng)), rfloat(rng)])
    op = rng.choice(["+", "-", "*", "/"])
    a = rexpr_num(rng, depth + 1)
    b = rexpr_num(rng, depth + 1)
    if op == "/":
        b = rng.choice(["3.0", "4", "7.5", str(rint(rng, 1, 50)) + ".0"])
    return f"({a} {op} {b})"


def rstr(rng, n=None):
    n = n or rng.randint(2, 10)
    alphabet = "abcdefghijklmnopqrstuvwxyzABC_0123456789"
    return "".join(rng.choice(alphabet) for _ in range(n))


# ------------------------------------------------------------------ buckets
def bucket_arith(rng, tag):
    frags = []
    for i in range(rng.randint(5, 9)):
        e = rexpr_int(rng)
        frags.append(f'print("{tag}-{i}:", {e})')
    for i in range(rng.randint(2, 4)):
        e = rexpr_num(rng)
        frags.append(f'print("{tag}-f{i}:", {e})')
    frags.append(
        f'print("{tag}-neg:", -{rint(rng, 1, 999)}, -({rexpr_int(rng)}))'
    )
    return frags


def bucket_cmplogic(rng, tag):
    frags = []
    for i in range(rng.randint(4, 7)):
        a, b = rint(rng), rint(rng)
        op = rng.choice(["==", "!=", "<", "<=", ">", ">="])
        frags.append(f'print("{tag}-c{i}:", {a} {op} {b})')
    for i in range(rng.randint(2, 4)):
        a, b, c = rint(rng, 0, 50), rint(rng, 0, 50), rint(rng, 0, 50)
        frags.append(
            f'print("{tag}-b{i}:", {a} < {b} && {b} < {c}, {a} > {b} || {a} > {c})'
        )
    for i in range(rng.randint(2, 4)):
        x, y = rint(rng), rint(rng)
        frags.append(f'print("{tag}-t{i}:", {x} > {y} ? "gt" : "le")')
    for i in range(rng.randint(2, 3)):
        x = rint(rng)
        frags.append(f'let {tag}_n{i} = null')
        frags.append(f'print("{tag}-n{i}:", {tag}_n{i} ?? {x}, {tag}_n{i} == null)')
    for i in range(2):
        w = rstr(rng, 4)
        h = rstr(rng, 6)
        frags.append(f'print("{tag}-s{i}:", "{w}" in "{h}{w}", "{w}" == "{w}")')
    return frags


def bucket_strings(rng, tag):
    frags = []
    for i in range(rng.randint(4, 6)):
        s = rstr(rng, rng.randint(6, 14))
        k = rng.randint(1, 4)
        frags.append(
            f'print("{tag}-m{i}:", "{s}".upper(), "{s}".slice({k}), "{s}".len())'
        )
    for i in range(rng.randint(3, 5)):
        a, b = rstr(rng, 3), rstr(rng, 4)
        v = rint(rng, 0, 99)
        frags.append(f'print("{tag}-i{i}:", "{a}-{{{v}}}-{b}")')
        frags.append(f'let {tag}_v{i} = {v}')
        frags.append(f'print("{tag}-v{i}:", "n={{{tag}_v{i}}}" + "!" * {rng.randint(1, 3)})')
    for i in range(rng.randint(2, 4)):
        s = rstr(rng, 8)
        cut = rstr(rng, 2)
        frags.append(
            f'print("{tag}-r{i}:", "{s}".replace("{s[0]}", "{cut}"), "{s}".contains("{s[1:3]}"))'
        )
    for i in range(rng.randint(2, 3)):
        parts = [rstr(rng, 3) for _ in range(rng.randint(3, 5))]
        lst = ", ".join(f'"{p}"' for p in parts)
        frags.append(f'print("{tag}-j{i}:", [{lst}].join("-"))')
    for i in range(rng.randint(2, 3)):
        s = rstr(rng, 7)
        frags.append(f'print("{tag}-a{i}:", "{s}".at({rng.randint(0, 6)}), "{s}".trim() == "{s}")')
    return frags


def bucket_lists(rng, tag):
    frags = []
    for i in range(rng.randint(4, 6)):
        n = rng.randint(4, 8)
        xs = [rint(rng, -50, 50) for _ in range(n)]
        lit = ", ".join(str(x) for x in xs)
        idx = rng.randint(0, n - 1)
        frags.append(f'let {tag}_l{i} = [{lit}]')
        frags.append(
            f'print("{tag}-l{i}:", {tag}_l{i}[{idx}], {tag}_l{i}[-1], {tag}_l{i}.len())'
        )
    for i in range(rng.randint(3, 5)):
        n = rng.randint(5, 9)
        xs = [rint(rng, -99, 99) for _ in range(n)]
        lit = ", ".join(str(x) for x in xs)
        s, e = sorted(rng.sample(range(1, n), 2))
        frags.append(f'let {tag}_s{i} = [{lit}]')
        frags.append(
            f'print("{tag}-s{i}:", {tag}_s{i}.slice({s}, {e}), {tag}_s{i}.slice({-rng.randint(1, 3)}))'
        )
    for i in range(rng.randint(2, 4)):
        xs = [rint(rng, 0, 999) for _ in range(rng.randint(4, 7))]
        lit = ", ".join(str(x) for x in xs)
        probe = rng.choice(xs) if xs else 1
        frags.append(f'let {tag}_r{i} = [{lit}]')
        frags.append(
            f'print("{tag}-r{i}:", {tag}_r{i}.sort(), {tag}_r{i}.reverse(), {tag}_r{i}.contains({probe}))'
        )
    for i in range(rng.randint(2, 3)):
        xs = [rint(rng, 1, 40) for _ in range(rng.randint(3, 6))]
        lit = ", ".join(str(x) for x in xs)
        frags.append(f'let {tag}_t{i} = 0')
        frags.append(f'for v in [{lit}] {{ {tag}_t{i} += v }}')
        frags.append(f'print("{tag}-t{i}:", {tag}_t{i})')
    for i in range(2):
        inner = [rint(rng, 1, 9) for _ in range(3)]
        lit = ", ".join(str(x) for x in inner)
        frags.append(f'let {tag}_n{i} = [[{lit}], [{lit}], 77]')
        frags.append(f'print("{tag}-n{i}:", {tag}_n{i}[0][{rng.randint(0, 2)}], {tag}_n{i}[2])')
    return frags


def bucket_maps(rng, tag):
    frags = []
    for i in range(rng.randint(3, 5)):
        keys = rng.sample(WORDS, rng.randint(3, 5))
        pairs = ", ".join(f'{k}: {rint(rng, -99, 99)}' for k in keys)
        k0 = keys[0]
        frags.append(f'let {tag}_m{i} = {{{pairs}}}')
        frags.append(
            f'print("{tag}-m{i}:", {tag}_m{i}["{k0}"], len({tag}_m{i}), {tag}_m{i}.has("{k0}"))'
        )
    for i in range(rng.randint(2, 3)):
        keys = rng.sample(WORDS, 3)
        pairs = ", ".join(f'{k}: {rint(rng, 1, 50)}' for k in keys)
        frags.append(f'let {tag}_k{i} = {{{pairs}}}')
        frags.append(f'let {tag}_ks{i} = {tag}_k{i}.keys()')
        frags.append(f'let {tag}_sm{i} = 0')
        frags.append(f'for k in {tag}_k{i} {{ {tag}_sm{i} += {tag}_k{i}[k] }}')
        frags.append(
            f'print("{tag}-k{i}:", {tag}_sm{i}, {tag}_k{i}.keys().sort(), {tag}_k{i}.values().len())'
        )
    for i in range(2):
        keys = rng.sample(WORDS, 2)
        inner = ", ".join(f'{k}: {rint(rng, 1, 9)}' for k in keys)
        frags.append(f'let {tag}_w{i} = {{outer: {{' + inner + ', tag: "ok"}}}')
        frags.append(
            f'print("{tag}-w{i}:", {tag}_w{i}["outer"]["{keys[1]}"], {tag}_w{i}["outer"]["tag"])'
        )
    return frags


def bucket_control(rng, tag):
    frags = []
    for i in range(rng.randint(3, 5)):
        x = rint(rng, -20, 120)
        lines = [f'let {tag}_b{i} = "start"']
        lines.append(f'if {x} > 80 {{ {tag}_b{i} = "hi" }} elif {x} > 40 {{ {tag}_b{i} = "mid" }} else {{ {tag}_b{i} = "lo" }}')
        lines.append(f'print("{tag}-i{i}:", {tag}_b{i}, {x})')
        frags.extend(lines)
    for i in range(rng.randint(2, 4)):
        cap = rng.randint(5, 20)
        step = rng.randint(1, 4)
        lines = [f'let {tag}_a{i} = 0', f'let {tag}_n{i} = 0']
        lines.append(f'while {tag}_n{i} < {cap} {{')
        lines.append(f'    {tag}_n{i} += {step}')
        lines.append(f'    if {tag}_n{i} % {rng.randint(2, 3)} == 0 {{ continue }}')
        lines.append(f'    if {tag}_n{i} > {cap + rng.randint(2, 9)} {{ break }}')
        lines.append(f'    {tag}_a{i} += {tag}_n{i}')
        lines.append('}')
        lines.append(f'print("{tag}-w{i}:", {tag}_a{i}, {tag}_n{i})')
        frags.extend(lines)
    for i in range(rng.randint(2, 3)):
        n, m = rng.randint(2, 4), rng.randint(2, 4)
        lines = [f'let {tag}_c{i} = 0']
        lines.append(f'for a in [1, 2, {n}] {{')
        lines.append(f'    for b in [1, 2, {m}] {{')
        lines.append(f'        {tag}_c{i} += a * b')
        lines.append('    }')
        lines.append('}')
        lines.append(f'print("{tag}-l{i}:", {tag}_c{i})')
        frags.extend(lines)
    for i in range(rng.randint(2, 3)):
        acc = [f'let {tag}_acc{i} = ""']
        acc.append(f'loop {{')
        acc.append(f'    {tag}_acc{i} += "x"')
        acc.append(f'    if {tag}_acc{i}.len() >= {rng.randint(3, 7)} {{ break }}')
        acc.append('}')
        acc.append(f'print("{tag}-o{i}:", {tag}_acc{i})')
        frags.extend(acc)
    return frags


def bucket_funcs(rng, tag):
    frags = []
    for i in range(rng.randint(2, 3)):
        a, b, c = rint(rng, 1, 99), rint(rng, 1, 99), rint(rng, 1, 99)
        x = rint(rng, -50, 50)
        lines = [
            f'gene {tag}_g{i}(p, q = {a}, r = {b}) {{',
            f'    return p * {c} + q - r',
            '}',
            f'print("{tag}-d{i}:", {tag}_g{i}({x}), {tag}_g{i}({x}, {rint(rng, 1, 99)}), {tag}_g{i}({x}, {a}, {b}))',
        ]
        frags.extend(lines)
    for i in range(rng.randint(2, 3)):
        n = rng.randint(7, 13)
        lines = [
            f'gene {tag}_fib{i}(n) {{',
            '    if n < 2 {',
            '        return n',
            '    }',
            f'    return {tag}_fib{i}(n - 1) + {tag}_fib{i}(n - 2)',
            '}',
            f'print("{tag}-fib{i}:", {tag}_fib{i}({n}))',
        ]
        frags.extend(lines)
    for i in range(rng.randint(2, 3)):
        a, b = rint(rng, 10, 999), rint(rng, 10, 999)
        lines = [
            f'gene {tag}_gcd{i}(a, b) {{',
            '    if b == 0 {',
            '        return a',
            '    }',
            f'    return {tag}_gcd{i}(b, a % b)',
            '}',
            f'print("{tag}-gcd{i}:", {tag}_gcd{i}({a}, {b}))',
        ]
        frags.extend(lines)
    for i in range(2):
        start = rint(rng, 0, 40)
        lines = [
            f'gene {tag}_mk{i}(start) {{',
            '    let n = start',
            '    return gene (by) {',
            '        n += by',
            '        return n',
            '    }',
            '}',
            f'let {tag}_c{i} = {tag}_mk{i}({start})',
            f'{tag}_c{i}({rint(rng, 1, 9)})',
            f'print("{tag}-cl{i}:", {tag}_c{i}({rint(rng, 1, 9)}))',
        ]
        frags.extend(lines)
    for i in range(2):
        x = rint(rng, 1, 30)
        lines = [
            f'gene {tag}_tw{i}(f, v) {{',
            '    return f(f(v))',
            '}',
            f'print("{tag}-hof{i}:", {tag}_tw{i}(gene (z) => z * 2 + 1, {x}))',
        ]
        frags.extend(lines)
    for i in range(2):
        n = rint(rng, 100, 999999)
        lines = [
            f'gene {tag}_sd{i}(n) {{',
            '    let s = 0',
            '    while n > 0 {',
            '        s += n % 10',
            '        n = n // 10',
            '    }',
            '    return s',
            '}',
            f'print("{tag}-dg{i}:", {tag}_sd{i}({n}))',
        ]
        frags.extend(lines)
    return frags


def bucket_matchpat(rng, tag):
    frags = []
    for i in range(rng.randint(3, 5)):
        v = rint(rng, 1, 9)
        lines = [f'let {tag}_s{i} = some({v})']
        lines.append(f'match {tag}_s{i} {{')
        lines.append(f'    case Some(x) if x > {v} {{')
        lines.append(f'        print("{tag}-m{i}:", "big", x)')
        lines.append('    }')
        lines.append('    case Some(x) {')
        lines.append(f'        print("{tag}-m{i}:", "small", x)')
        lines.append('    }')
        lines.append('    case None {')
        lines.append(f'        print("{tag}-m{i}:", "none")')
        lines.append('    }')
        lines.append('}')
        frags.extend(lines)
    for i in range(rng.randint(2, 4)):
        a, b, c = (rint(rng, 1, 50) for _ in range(3))
        lines = [f'let {tag}_ls{i} = [{a}, {b}, {c}]']
        lines.append(f'match {tag}_ls{i} {{')
        lines.append(f'    case [{a}, *rest] {{')
        lines.append(f'        print("{tag}-l{i}:", "head", rest.len(), rest[-1])')
        lines.append('    }')
        lines.append('    case [x, y] {')
        lines.append(f'        print("{tag}-l{i}:", "pair", x, y)')
        lines.append('    }')
        lines.append('    case _ {')
        lines.append(f'        print("{tag}-l{i}:", "other")')
        lines.append('    }')
        lines.append('}')
        frags.extend(lines)
    for i in range(rng.randint(2, 3)):
        k0, k1 = rng.sample(WORDS, 2)
        v0, v1 = rint(rng), rint(rng)
        lines = [f'let {tag}_mp{i} = {{{k0}: {v0}, {k1}: {v1}}}']
        lines.append(f'match {tag}_mp{i} {{')
        lines.append(f'    case {{{k0}: p, {k1}: q}} {{')
        lines.append(f'        print("{tag}-mp{i}:", p, q)')
        lines.append('    }')
        lines.append('    case _ {')
        lines.append(f'        print("{tag}-mp{i}:", "miss")')
        lines.append('    }')
        lines.append('}')
        frags.extend(lines)
    for i in range(rng.randint(2, 3)):
        v = rint(rng, 1, 5)
        lines = [f'let {tag}_r{i} = ok({v})']
        lines.append(f'match {tag}_r{i} {{')
        lines.append(f'    case Ok(n) | Err({v}) {{')
        lines.append(f'        print("{tag}-r{i}:", "hit", n)')
        lines.append('    }')
        lines.append('    case Ok(0) {')
        lines.append(f'        print("{tag}-r{i}:", "zero")')
        lines.append('    }')
        lines.append('}')
        frags.extend(lines)
    for i in range(2):
        w = rng.choice(WORDS)
        lines = [f'match "{w}" {{']
        lines.append(f'    case "{w}" {{')
        lines.append(f'        print("{tag}-w{i}:", "word")')
        lines.append('    }')
        lines.append(f'    case "zz" | "qq" {{')
        lines.append(f'        print("{tag}-w{i}:", "alt")')
        lines.append('    }')
        lines.append('}')
        frags.extend(lines)
    return frags


def bucket_optres(rng, tag):
    frags = []
    for i in range(rng.randint(3, 5)):
        a, b = rint(rng), rint(rng)
        lines = [
            f'gene {tag}_f{i}(n) {{',
            f'    if n > {b} {{',
            f'        return err("too big: " + str(n))',
            '    }',
            '    return ok(n * 2)',
            '}',
            f'gene {tag}_u{i}(n) {{',
            f'    let v = {tag}_f{i}(n)?!',
            '    return ok(v + 1)',
            '}',
            f'print("{tag}-p{i}:", {tag}_u{i}({a}), {tag}_u{i}({b + rint(rng, 1, 20)}))',
        ]
        frags.extend(lines)
    for i in range(rng.randint(3, 4)):
        v = rint(rng)
        lines = [
            f'print("{tag}-o{i}:", is_ok(ok({v})), is_none(none()), is_err(err("{v}")), is_some(some({v})))',
            f'print("{tag}-u{i}:", unwrap_or(none(), {v}), unwrap_or(err("x"), {v - 1}), unwrap_or(ok({v}), 0))',
        ]
        frags.extend(lines)
    for i in range(2):
        a = rint(rng)
        lines = [
            f'print("{tag}-e{i}:", ok({a}) == ok({a}), some({a}) == ok({a}), none() == none(), err(1) == err(2))',
            f'print("{tag}-j{i}:", json_str(ok({a})), json_str(err("bad")), json_str(some([1, 2])), json_str(none()))',
        ]
        frags.extend(lines)
    for i in range(2):
        a, b = rint(rng, 1, 99), rint(rng, 1, 99)
        lines = [
            f'print("{tag}-n{i}:", some(some({a}))?!?!, unwrap(ok({a + b})) + unwrap(some({b})))',
            f'print("{tag}-s{i}:", "v=" + str(ok({a})?!), some("{b}k")?! + "!")',
        ]
        frags.extend(lines)
    return frags


def bucket_stressfail(rng, tag):
    frags = []
    probes = [
        ("dz", "0 / 0"),
        ("dzf", "0.0 / 0.0"),
        ("mz", f"{rint(rng, 1, 99)} % 0"),
        ("of1", f"{MAX_I64} + 1"),
        ("of2", f"{MAX_I64} * 2"),
        ("m1", f"{MAX_I64} % -1"),
    ]
    for i, (name, expr) in enumerate(probes[: rng.randint(4, 6)]):
        lines = [
            f'stress {{',
            f'    print("{tag}-{name}:", {expr})',
            f'}} rescue (e) {{',
            f'    print("{tag}-{name}:", "caught", e.kind, e.message)',
            f'}}',
        ]
        frags.extend(lines)
    for i in range(rng.randint(2, 4)):
        msg = rstr(rng, 6)
        lines = [
            f'gene {tag}_r{i}() {{',
            f'    raise "{tag}-{msg}"',
            '}',
            f'stress {{',
            f'    {tag}_r{i}()',
            f'    print("{tag}-r{i}:", "unreachable")',
            f'}} rescue (e) {{',
            f'    print("{tag}-r{i}:", e.kind, e.message)',
            f'}}',
        ]
        frags.extend(lines)
    for i in range(rng.randint(2, 3)):
        xs = [rint(rng, 1, 50) for _ in range(rng.randint(2, 4))]
        lit = ", ".join(str(x) for x in xs)
        lines = [
            f'let {tag}_xs{i} = [{lit}]',
            f'stress {{',
            f'    print("{tag}-x{i}:", {tag}_xs{i}[{len(xs) + rng.randint(1, 5)}])',
            f'}} rescue (e) {{',
            f'    print("{tag}-x{i}:", "caught", e.kind)',
            f'}}',
        ]
        frags.extend(lines)
    for i in range(2):
        deep = rng.randint(2, 4)
        lines = [f'stress {{']
        cur = f'    raise "deep-{tag}-{i}"'
        for d in range(deep):
            lines.append('    stress {')
        lines.append('    ' + cur)
        for d in range(deep):
            lines.append('    } rescue (e) {')
            lines.append(f'        print("{tag}-d{i}-{d}:", e.message)')
            lines.append('        raise e.message')
            lines.append('    }')
        lines.append('} rescue (outer) {')
        lines.append(f'    print("{tag}-d{i}-top:", outer.message)')
        lines.append('}')
        frags.extend(lines)
    return frags


def bucket_nums(rng, tag):
    frags = []
    for i in range(rng.randint(3, 5)):
        h = "".join(rng.choice("0123456789abcdefABCDEF") for _ in range(rng.randint(2, 6)))
        o = "".join(rng.choice("01234567") for _ in range(rng.randint(2, 5)))
        b = "".join(rng.choice("01") for _ in range(rng.randint(4, 10)))
        d = str(rint(rng, 10 ** 4, 10 ** 7))
        sep = rng.choice([f"{d[:3]}_{d[3:]}", f"{d[:2]}_{d[2:5]}_{d[5:]}"])
        frags.append(
            f'print("{tag}-r{i}:", 0x{h}, 0o{o}, 0b{b}, {sep})'
        )
    for i in range(rng.randint(2, 4)):
        a, b = rint(rng, -9999, 9999), rint(rng, 1, 97)
        c = rint(rng, -9999, 9999)
        frags.append(f'print("{tag}-fd{i}:", {a} // {b}, {a} % {b}, ({a} // {b}) * {b} + {a} % {b} == {a})')
        frags.append(f'print("{tag}-fm{i}:", {-a if a else 7} // {-b if b else 3}, {-a if a else 7} % {-b if b else 3}, {c} // {b})')
    for i in range(rng.randint(2, 3)):
        a, b = rint(rng, 0, 2 ** 20), rint(rng, 0, 2 ** 20)
        frags.append(f'print("{tag}-bit{i}:", {a} & {b}, {a} | {b}, {a} ^ {b}, ~{a}, ~{b})')
    for i in range(rng.randint(2, 3)):
        f1 = rfloat(rng)
        f2 = rfloat(rng)
        frags.append(f'print("{tag}-ff{i}:", {f1}, {f2}, {f1} + {f2}, {f1} * 2.0)')
    for i in range(2):
        frags.append(f'print("{tag}-sc{i}:", 1e3, 2.5e2, str(1.0), str({rint(rng, 10 ** 5, 10 ** 6)}.0))')
    for i in range(2):
        lines = [
            f'stress {{',
            f'    print("{tag}-ov{i}:", {rint(rng, 10 ** 9, 10 ** 12)} * {rint(rng, 10 ** 9, 10 ** 12)})',
            f'}} rescue (e) {{',
            f'    print("{tag}-ov{i}:", "caught", e.kind)',
            f'}}',
        ]
        frags.extend(lines)
    return frags


def bucket_mixed(rng, tag):
    frags = []
    for i in range(rng.randint(2, 3)):
        keys = rng.sample(WORDS, 3)
        pairs = ", ".join(f'{k}: {rint(rng, 1, 99)}' for k in keys)
        lines = [
            f'gene {tag}_b{i}(m) {{',
            '    let best = null',
            '    let bv = -1',
            '    for k in m {',
            '        if m[k] > bv {',
            '            bv = m[k]',
            '            best = k',
            '        }',
            '    }',
            '    return ok(best + "=" + str(bv))',
            '}',
            f'print("{tag}-b{i}:", {tag}_b{i}({{{pairs}}})?!)',
        ]
        frags.extend(lines)
    for i in range(rng.randint(2, 3)):
        n = rng.randint(4, 8)
        xs = ", ".join(str(rint(rng, 1, 60)) for _ in range(n))
        lines = [
            f'gene {tag}_f{i}(xs) {{',
            '    let evens = []',
            '    for v in xs {',
            '        if v % 2 == 0 {',
            '            evens.push(v * 10)',
            '        }',
            '    }',
            '    if evens.len() == 0 {',
            '        return err("no evens")',
            '    }',
            '    return ok(evens)',
            '}',
            f'match {tag}_f{i}([{xs}]) {{',
            '    case Ok(list) {',
            f'        print("{tag}-f{i}:", list.join("/"))',
            '    }',
            '    case Err(m) {',
            f'        print("{tag}-f{i}:", "e:", m)',
            '    }',
            '}',
        ]
        frags.extend(lines)
    for i in range(2):
        a, b = rint(rng, 1, 20), rint(rng, 1, 20)
        lines = [
            f'let {tag}_cl{i} = gene (x) {{',
            '    return gene (y) {',
            '        return x * 100 + y',
            '    }',
            '}',
            f'print("{tag}-cc{i}:", {tag}_cl{i}({a})({b}), {tag}_cl{i}({b})({a}))',
        ]
        frags.extend(lines)
    for i in range(2):
        v = rint(rng, 1, 9)
        w = rng.choice(WORDS)
        lines = [
            f'let {tag}_data{i} = some([some("{w}"), none(), some({v})])',
            f'match {tag}_data{i} {{',
            '    case Some([Some(s), none(), Some(n)]) {',
            f'        print("{tag}-x{i}:", s, n, s.len())',
            '    }',
            '    case _ {',
            f'        print("{tag}-x{i}:", "shape-miss")',
            '    }',
            '}',
        ]
        frags.extend(lines)
    return frags


# ----------------------------------------------------------- registry
GEN = {
    "arith": bucket_arith,
    "cmplogic": bucket_cmplogic,
    "strings": bucket_strings,
    "lists": bucket_lists,
    "maps": bucket_maps,
    "control": bucket_control,
    "funcs": bucket_funcs,
    "matchpat": bucket_matchpat,
    "optres": bucket_optres,
    "stressfail": bucket_stressfail,
    "nums": bucket_nums,
    "mixed": bucket_mixed,
}


def scan_manifest(out):
    """Build the manifest from the .op files already present in `out`.

    Each generated program carries its provenance in the header comment
    (`# bucket=... seed=... idx=... tag=...`), so the manifest can always be
    rebuilt from the committed file set — across multiple generation waves
    with different seeds — and stays consistent with what is actually there.
    """
    import re
    files = []
    for name in sorted(os.listdir(out)):
        if not name.endswith(".op"):
            continue
        with open(os.path.join(out, name), encoding="utf-8") as f:
            f.readline()  # "# compat corpus — generated, do not edit by hand"
            header = f.readline().strip()
        m = re.match(r"# bucket=(\S+) seed=(\d+) idx=(\d+) tag=(\S+)", header)
        if not m:
            sys.exit(f"unparseable corpus header in {name}: {header!r}")
        files.append({"file": name, "bucket": m.group(1),
                      "seed": int(m.group(2)), "idx": int(m.group(3))})
    return files


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=20260930)
    ap.add_argument("--count", type=int, default=1200)
    ap.add_argument("--start", type=int, default=0,
                    help="first global program index; per-bucket file "
                         "numbering continues at start // len(BUCKETS)")
    ap.add_argument("--out", default="tests/compat")
    ap.add_argument("--manifest", action="store_true")
    ap.add_argument("--manifest-scan", action="store_true",
                    help="rebuild MANIFEST.json from the .op files already "
                         "in --out instead of writing only this run's files")
    args = ap.parse_args()

    missing = [b for b, fn in GEN.items() if fn is None]
    if missing:
        sys.exit(f"unregistered buckets: {missing}")

    if args.manifest_scan:
        files = scan_manifest(args.out)
        seeds = sorted({f["seed"] for f in files})
        manifest = {"seed": seeds[0], "seeds": seeds, "count": len(files),
                    "buckets": BUCKETS, "files": files}
        with open(os.path.join(args.out, "MANIFEST.json"), "w", encoding="utf-8") as f:
            json.dump(manifest, f, indent=1)
        print(f"manifest: {len(files)} programs, seeds {seeds} -> "
              f"{os.path.join(args.out, 'MANIFEST.json')}")
        return

    os.makedirs(args.out, exist_ok=True)
    files = []
    for i in range(args.start, args.start + args.count):
        bucket = BUCKETS[i % len(BUCKETS)]
        idx = i // len(BUCKETS)
        tag = f"{bucket[0]}{idx:03d}"
        rng = random.Random(args.seed * 1000003 + i * 7919)
        frags = GEN[bucket](rng, tag)
        header = [
            "# compat corpus — generated, do not edit by hand",
            f"# bucket={bucket} seed={args.seed} idx={idx} tag={tag}",
            "",
        ]
        body = "\n\n".join(frags) + "\n"
        path = os.path.join(args.out, f"{bucket}_{idx:04d}.op")
        # F6/#50: utf-8 + \n pinned — the locale default (cp1252 on
        # Windows) once encoded the em-dash header as an invalid-UTF-8 byte
        # and the Rust engine refused every fresh program on the windows leg.
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(header) + "\n" + body)
        files.append({"file": os.path.basename(path), "bucket": bucket,
                      "seed": args.seed, "idx": idx})
    if args.manifest:
        with open(os.path.join(args.out, "MANIFEST.json"), "w", encoding="utf-8") as f:
            json.dump({"seed": args.seed, "count": len(files),
                       "buckets": BUCKETS, "files": files}, f, indent=1)
    print(f"generated {len(files)} programs across {len(BUCKETS)} buckets "
          f"(idx {args.start}..{args.start + args.count - 1}) -> {args.out}")


if __name__ == "__main__":
    main()
