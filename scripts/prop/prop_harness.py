#!/usr/bin/env python3
"""prop_harness.py — W050 (M100 batch 2): shrinkable property-based differential testing.

One opt-in script, fully deterministic under a CLI seed. For every generated
program the ONE property is the differential law of the project (the same law
bootstrap/harness.py enforces on the checked-in corpus):

    Rust stdout  ==  oracle stdout   (byte-for-byte)
    Rust exit    ==  oracle exit

with the oracle invoked exactly the way bootstrap/harness.py invokes it:
`python3 bootstrap/oracle.py run <file>` (stdout + exit code compared; stderr
is engine-specific noise — [info]/[fallback] notes, Python tracebacks — and is
NEVER compared).

Generator lanes (each pinned to a std API surface, every program TOTAL —
no blocking, no file I/O, no spawn, no entropy/time builtins):
  arith     expression trees over i64. Respects the pinned overflow contract
            (tests/overflow_contract.op: overflow RAISES a catchable stress of
            kind "overflow", div-by-zero is kind "unfolded", shift amounts must
            be 0..=63): boundary-touching expressions are wrapped in
            stress/rescue and the caught kind is printed, so no wrapped value
            can ever leak into a comparison.
  strings   the str method surface (SPEC: upper/lower/trim/split/join/replace/
            contains/starts/ends/repeat/slice/len) + std/strings + std/unicode
            (fold_case/ord/chr/grapheme_len/char_at/char_slice).
  colls     list/map surface: literals, range, push/pop/insert/remove, index,
            slice, sort/reverse/contains/index_of, keys/values/has/del, collect
            comprehensions, for-in loops.
  programs  small total programs: gene defs + calls + if/elif/else + bounded
            loops + match + promote.

On a mismatch the case is SHRUNK (fewer statements, smaller expressions,
shorter lists, smaller literals) to a minimal reproducer, which is printed and
saved under tests/property/repro/. The harness never fixes bugs — it finds and
minimizes them.

Determinism: one seeded RNG drives everything; the run prints a
"determinism digest" — sha256 over every case's source and both engines'
stdout/exit codes. Two runs with the same seed MUST produce the same digest
(and, with --dump-corpus, byte-identical corpora). This is the proof gate.

NOT in scope here: tooling robustness under mutation (that is W051,
scripts/fuzz/fuzz.py) and fmt/AST idempotence (pinned by tests/fmt_idempotence.rs).

Run:  python3 scripts/prop/prop_harness.py [--cases N] [--seed S] [--lanes ...]
      [--dump-corpus DIR] [--bin PATH]
Exit: 0 all properties held, 1 findings (printed + saved), 2 harness error.
"""
import argparse
import hashlib
import os
import random
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ORACLE = os.path.join(ROOT, "bootstrap", "oracle.py")
TMP = os.path.join(ROOT, "target", "prop.tmp.op")  # fixed path: any engine-side
# echo of the file name is then identical across runs (same path every time).
REPRO_DIR = os.path.join(ROOT, "tests", "property", "repro")
DEFAULT_SEED = 20260926
CONTAINED_RCS = {0, 1, 2, 3}  # 0 ok, 1 uncaught stress, 2 parse, 3 check failure


def find_binary(explicit):
    """Prefer the gate-built debug binary, then the wrapper, then release."""
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


def run_rust(binpath, timeout):
    p = subprocess.run([binpath, "run", TMP], cwd=ROOT,
                       capture_output=True, timeout=timeout)
    return p.returncode, p.stdout


def run_oracle(timeout):
    # bootstrap/harness.py's invocation pattern, byte capture for the
    # byte-for-byte law (harness itself decodes with errors="replace"; here we
    # compare the raw bytes, which is strictly stronger).
    p = subprocess.run([sys.executable, ORACLE, "run", TMP], cwd=ROOT,
                       capture_output=True, timeout=timeout)
    return p.returncode, p.stdout


def looks_panic(rc, out):
    if rc not in CONTAINED_RCS:
        return True
    head = out[:4096].decode("utf-8", errors="replace")
    return ("panicked at" in head or "stack overflow" in head
            or "fatal runtime error" in head or "[fatal]" in head)


class CaseRunner:
    """Runs one rendered program through both engines, decides BAD, keeps the
    transcript that feeds the determinism digest."""

    def __init__(self, binpath, rust_timeout, oracle_timeout):
        self.binpath = binpath
        self.rust_timeout = rust_timeout
        self.oracle_timeout = oracle_timeout
        self.digest = hashlib.sha256()
        self.last_rust = None
        self.last_oracle = None

    def run_source(self, src):
        """Returns (bad, rust_rc, rust_out, py_rc, py_out). Timeouts count as
        bad (a generated TOTAL program must terminate on both engines)."""
        with open(TMP, "w", encoding="utf-8", newline="") as fh:
            fh.write(src)
        try:
            rust_rc, rust_out = run_rust(self.binpath, self.rust_timeout)
        except subprocess.TimeoutExpired:
            return True, "TIMEOUT", b"", None, b""
        try:
            py_rc, py_out = run_oracle(self.oracle_timeout)
        except subprocess.TimeoutExpired:
            return True, rust_rc, rust_out, "TIMEOUT", b""
        bad = ((rust_rc != py_rc) or (rust_out != py_out)
               or looks_panic(rust_rc, rust_out))
        return bad, rust_rc, rust_out, py_rc, py_out

    def check(self, lane, idx, src):
        bad, rust_rc, rust_out, py_rc, py_out = self.run_source(src)
        # digest input: lane, idx, source, both engines' rc + stdout hash
        h = hashlib.sha256()
        h.update(lane.encode()); h.update(b"\x00")
        h.update(str(idx).encode()); h.update(b"\x00")
        h.update(src.encode("utf-8")); h.update(b"\x00")
        h.update(str(rust_rc).encode()); h.update(b"\x00")
        h.update(hashlib.sha256(rust_out).digest())
        h.update(str(py_rc).encode()); h.update(b"\x00")
        h.update(hashlib.sha256(py_out).digest())
        self.digest.update(h.digest())
        self.last_rust = (rust_rc, rust_out)
        self.last_oracle = (py_rc, py_out)
        return bad


# ============================================================ shrinker

def render_case(items, prelude):
    # top-level items (gene definitions) render before the entry gene;
    # everything else is a main-body statement
    pre, body = [], []
    for it in items:
        if getattr(it, "toplevel", False):
            pre.extend(it.render(0))
        else:
            body.extend(it.render(1))
    out = prelude
    for line in pre:
        out += line + "\n"
    if pre:
        out += "\n"
    return out + "gene main() {\n" + "\n".join(body) + "\n}\n"


def shrink(runner, items, prelude, budget=400):
    """Greedy shrink: try deleting items, then simplifying each item.
    `bad` = still a finding. Returns the minimal item list found."""
    def is_bad(cand):
        if not cand:
            return False  # never shrink to an empty program
        return runner.run_source(render_case(cand, prelude))[0]

    cur = list(items)

    def note():
        nonlocal budget
        budget -= 1
        return budget > 0

    changed = True
    while changed and note():
        changed = False
        # 1) delete whole items (fewer statements = smaller case)
        i = 0
        while i < len(cur) and note():
            cand = cur[:i] + cur[i + 1:]
            if is_bad(cand):
                cur = cand
                changed = True
            else:
                i += 1
        # 2) simplify items (smaller expressions / literals / lengths)
        i = 0
        while i < len(cur) and note():
            for simp in cur[i].shrinks():
                if not note():
                    break
                cand = cur[:i] + [simp] + cur[i + 1:]
                if is_bad(cand):
                    cur = cand
                    changed = True
                    break
            i += 1
    return cur


# ============================================================ arithmetic lane

I64_MAX = "9223372036854775807"
I64_MIN = "(-9223372036854775807 - 1)"  # the bare literal itself would overflow
ALWAYS_RISKY_OPS = {"//", "%", "**", "<<", ">>"}  # pinned: can raise per contract


class Arith:
    """Expression tree node. `risky` = can raise per the pinned overflow
    contract (so it must be evaluated inside stress/rescue)."""

    def __init__(self, kind, a=None, b=None, op=None, text=None, risky=False):
        self.kind, self.a, self.b, self.op, self.text = kind, a, b, op, text
        self.risky = risky

    def render(self):
        if self.kind == "num":
            return self.text
        if self.kind == "neg":
            return f"(-{self.a.render()})"
        return f"({self.a.render()} {self.op} {self.b.render()})"

    def shrinks(self):
        out = []
        if self.kind == "num":
            try:
                v = int(self.text)
            except ValueError:
                return out  # boundary constants stay atomic (they ARE minimal)
            for cand in (0, 1, -1, v // 2):
                if cand != v:
                    out.append(Arith("num", text=str(cand)))
            return out
        for child in (self.a, self.b):
            if child is not None:
                out.append(Arith("num", text=child.render()))  # lift a child
        out.append(Arith("num", text="1"))
        out.append(Arith("num", text="0"))
        return out


def gen_arith_expr(rnd, depth):
    if depth <= 0 or rnd.random() < 0.35:
        t = rnd.random()
        if t < 0.72:
            return Arith("num", text=str(rnd.randint(-999, 999)))
        if t < 0.86:
            return Arith("num", text=str(rnd.randint(2**52, 2**62)), risky=True)
        if t < 0.93:
            return Arith("num", text=I64_MAX, risky=True)
        return Arith("num", text=I64_MIN, risky=True)
    op = rnd.choice(["+", "-", "*", "+", "-", "*", "//", "%", "**", "<<", ">>"])
    a = gen_arith_expr(rnd, depth - 1)
    b = gen_arith_expr(rnd, depth - 1)
    risky = a.risky or b.risky or op in ALWAYS_RISKY_OPS
    return Arith("bin", a=a, b=b, op=op, risky=risky)


class ArithItem:
    def __init__(self, expr):
        self.expr = expr

    def render(self, ind):
        pad = "    " * ind
        e = self.expr.render()
        if self.expr.risky:
            return [
                pad + "stress {",
                pad + "    promote(" + e + ")",
                pad + "} rescue (e) {",
                pad + '    promote("ov:" + e.kind)',
                pad + "}",
            ]
        return [pad + "promote(" + e + ")"]

    def shrinks(self):
        return [ArithItem(s) for s in self.expr.shrinks()]


def gen_arith_case(rnd):
    return [ArithItem(gen_arith_expr(rnd, rnd.randint(1, 4)))
            for _ in range(rnd.randint(1, 3))]


# ============================================================ strings lane

STR_PRELUDE = "use std/strings as st\nuse std/unicode as uni\n\n"
ALPHABET = ["a", "b", "Z", " ", "-", "_", "\"", "'", "\\", "\n", "\t", "0", "9",
            "é", "À", "σ", "Ω", "ж", "字", "🧬", "✓", "😀", "\u0301", "\u200d"]
WORDS = ["", "gene", "wobble", "hello world", "  pad  ", "a-b-c", "myVarName",
         "HTTPServer", "line1\nline2", "x\"y", "átomo", "🧬abc", "s"]
CASE_FOLD_METHODS = ["upper", "lower", "trim"]


def quote(s):
    return '"' + (s.replace("\\", "\\\\").replace('"', '\\"')
                   .replace("\n", "\\n").replace("\t", "\\t")) + '"'


def gen_str_expr(rnd, depth=0):
    """Returns (source_text, risky, atomic). Compound forms are parenthesized
    so precedence can never silently reinterpret the intended tree (a missing
    pair of parens once turned `"x" * 1` + `.upper()` into `"x" * 1.upper()`
    — a different program, and the soft tier it poked is now probed ON PURPOSE
    by gen_strings_case)."""
    t = rnd.random()
    if depth >= 2 or t < 0.45:
        s = rnd.choice(WORDS) if rnd.random() < 0.6 else \
            "".join(rnd.choice(ALPHABET) for _ in range(rnd.randint(0, 6)))
        return quote(s), False, True
    if t < 0.55:  # char-indexed access: out-of-range is catchable `missing`
        b, _, _ = gen_str_expr(rnd, depth + 1)
        return f"char_at({b}, {rnd.randint(-4, 6)})", True, True
    if t < 0.62:
        b, br, _ = gen_str_expr(rnd, depth + 1)
        return f"len({b})", br, True
    if t < 0.70:  # slice: clamped negatives per SPEC, still contained if odd
        b, _, _ = gen_str_expr(rnd, depth + 1)
        return f"{b}.slice({rnd.randint(-4, 7)}, {rnd.randint(-4, 7)})", True, True
    if t < 0.76:  # chr of an arbitrary codepoint: risky (surrogates / < 0)
        return f"chr({rnd.randint(-1, 65568)})", True, True
    if t < 0.82:  # repeat: negative or oversized counts are catchable
        b, _, _ = gen_str_expr(rnd, depth + 1)
        return f"({b} * {rnd.randint(-2, 6)})", True, False
    if t < 0.90:
        a, _, _ = gen_str_expr(rnd, depth + 1)
        b, _, _ = gen_str_expr(rnd, depth + 1)
        return f"({a} + {b})", False, False
    b, br, _ = gen_str_expr(rnd, depth + 1)
    return f"{b}.{rnd.choice(CASE_FOLD_METHODS)}()", br, True


def gen_strings_case(rnd):
    items = []
    for _ in range(rnd.randint(1, 4)):
        # rare deliberate soft-tier probe: a method call on an INT is legal
        # Total Grammar (soft tier: null + fallback note); the str*null repeat
        # it feeds crashed the oracle with a bare TypeError during bring-up
        # (finding saved: tests/property/repro/strings_20260926_58.op). Keep
        # this surface covered until the semantic lane closes it.
        if rnd.random() < 0.02:
            items.append(StrItem(
                'promote("x" * 1.upper())',
                [StrItem('promote("x" * 0.upper())', [], True)],
                True))
            continue
        e, risky, _atomic = gen_str_expr(rnd)
        call = rnd.choice([
            f"st.title_case({e})", f"st.to_snake({e})", f"st.unquote({e})",
            f"st.ellipsis({e}, {rnd.randint(0, 6)})",
            f"st.pad_left({e}, {rnd.randint(0, 5)}, \"-\")",
            f"st.strip_prefix({e}, {quote(rnd.choice(['a', 'x-', '']))})",
            f"uni.fold_case({e})", f"uni.byte_width({e})",
            f"uni.grapheme_len({e})", f"uni.char_codes({e})",  # all take the STR
            f"ord({e})",  # multi-char arg is a stress -> always wrapped below
        ])
        stmt_risky = risky or call.startswith("ord(")
        stmt = "promote(" + call + ")"
        variants = []
        for e0 in ('""', '"x"'):
            if e0 != e and e in call:
                variants.append(StrItem("promote(" + call.replace(e, e0, 1) + ")",
                                        call.replace(e, e0, 1), stmt_risky))
        items.append(StrItem(stmt, variants, stmt_risky))
    return items


class StrItem:
    def __init__(self, stmt, variants, risky):
        self.stmt, self._variants, self.risky = stmt, variants, risky

    def render(self, ind):
        pad = "    " * ind
        if self.risky:
            return [pad + "stress {",
                    pad + "    " + self.stmt,
                    pad + "} rescue (e) {",
                    pad + '    promote("st:" + e.kind)',
                    pad + "}"]
        return [pad + self.stmt]

    def shrinks(self):
        return list(self._variants)


# ============================================================ lists/maps lane

def gen_list_lit(rnd):
    n = rnd.randint(0, 5)
    if rnd.random() < 0.6:
        vals = [str(rnd.randint(-20, 20)) for _ in range(n)]
    else:
        vals = [quote(rnd.choice(["x", "yy", "", "zz"])) for _ in range(n)]
    return "[" + ", ".join(vals) + "]", n


def wrap_stress(lines, kind_tag):
    return (["stress {"] + ["    " + ln for ln in lines] +
            ["} rescue (e) {", f"    promote(\"{kind_tag}:\" + e.kind)", "}"])


class CollItem:
    def __init__(self, lines, variants=None):
        self.lines = lines
        self._variants = variants or []

    def render(self, ind):
        pad = "    " * ind
        return [pad + ln for ln in self.lines]

    def shrinks(self):
        return [CollItem(v) for v in self._variants]


def gen_colls_case(rnd):
    items = []
    counter = [0]

    def newname():
        counter[0] += 1
        return f"c{counter[0]}"

    for _ in range(rnd.randint(1, 4)):
        pick = rnd.random()
        lit, n = gen_list_lit(rnd)
        v = newname()
        if pick < 0.32:
            # core list ops; position/value calls are uncertain (missing is
            # catchable) -> the whole op group rides inside one stress frame
            i = rnd.randint(-1, 6)
            ops = []
            for op in rnd.sample(["push", "insert", "remove", "index", "slice",
                                  "sortrev", "contains", "index_of", "len",
                                  "agg"], rnd.randint(1, 3)):
                if op == "push":
                    ops.append(f"push({v}, {rnd.randint(-9, 9)})")
                    ops.append(f"promote(len({v}))")
                elif op == "insert":
                    ops.append(f"insert({v}, {i}, {rnd.randint(-9, 9)})")
                elif op == "remove":
                    ops.append(f"remove({v}, {i})")
                elif op == "index":
                    ops.append(f"promote({v}[{i}])")
                elif op == "slice":
                    ops.append(f"promote({v}.slice({rnd.randint(-2, 5)}, {rnd.randint(-2, 5)}))")
                elif op == "sortrev":
                    ops.append(f"promote({v}.sort())")
                    ops.append(f"promote({v}.reverse())")
                elif op == "contains":
                    ops.append(f"promote({v}.contains({rnd.randint(-9, 9)}))")
                elif op == "index_of":
                    ops.append(f"promote({v}.index_of({rnd.randint(-9, 9)}))")
                elif op == "len":
                    ops.append(f"promote(len({v}))")
                else:
                    ops.append(f"promote(sum({v}))")
                    ops.append(f"promote(min({v}))")
                    ops.append(f"promote(max({v}))")
            items.append(CollItem([f"let {v} = {lit}"] + wrap_stress(ops, "co"),
                                  variants=[[f"let {v} = [5]"] + wrap_stress(ops, "co")]))
        elif pick < 0.55:
            # bounded pop loop; pop-on-empty is contained by the inner frame
            k = rnd.randint(0, 6)
            items.append(CollItem([
                f"let {v} = {lit}",
                f"let got = 0",
                f"while got < {k} {{",
                f"    stress {{",
                f"        pop({v})",
                f"    }} rescue (e) {{",
                f"        got = {k}",
                f"    }}",
                f"    got += 1",
                f"}}",
                f"promote(len({v}))",
            ], variants=[[
                f"let {v} = [5]",
                f"let got = 0",
                f"while got < {k} {{",
                f"    stress {{",
                f"        pop({v})",
                f"    }} rescue (e) {{",
                f"        got = {k}",
                f"    }}",
                f"    got += 1",
                f"}}",
                f"promote(len({v}))",
            ]]))
        elif pick < 0.80:
            # map ops: get/del of a possibly-missing key is the risky core
            keys = rnd.sample(["a", "b", "c", "k1", "k2", "zz"], rnd.randint(1, 3))
            ents = ", ".join(f"{k}: {rnd.randint(-9, 9)}" for k in keys)
            m = newname()
            probe = rnd.choice(keys + ["missing"])
            items.append(CollItem([
                f"let {m} = {{{ents}}}",
                f"{m}[{quote(probe)}] = {rnd.randint(-9, 9)}",
            ] + wrap_stress([
                f"promote({m}[{quote(probe)}])",
                f"del({m}, {quote(rnd.choice(keys + ['ghost']))})",
                f"promote({m}[{quote(rnd.choice(keys + ['ghost']))}])",
            ], "co") + [
                f"promote(has({m}, {quote(keys[0])}))",
                f"promote(len(keys({m})))",
                f"promote(sum(values({m})))",
            ], variants=[[
                f"let {m} = {{{keys[0]}: 1}}",
                f"{m}[{quote(probe)}] = 1",
            ] + wrap_stress([
                f"promote({m}[{quote(probe)}])",
                f"del({m}, {quote(rnd.choice(keys + ['ghost']))})",
                f"promote({m}[{quote(rnd.choice(keys + ['ghost']))}])",
            ], "co") + [
                f"promote(has({m}, {quote(keys[0])}))",
                f"promote(len(keys({m})))",
                f"promote(sum(values({m})))",
            ]]))
        else:
            # collect comprehension + order-insensitive aggregation
            a = rnd.randint(-5, 5)
            b = a + rnd.randint(0, 8)
            step = rnd.choice([1, 2, 3])
            items.append(CollItem([
                f"let acc = for x in range({a}, {b}, {step}) if x % 2 == 0 collect x * 2",
                f"promote(acc)",
                f"let t = 0",
                f"for y in acc {{ t += y }}",
                f"promote(t)",
                f"promote(len(acc))",
            ], variants=[[
                f"let acc = for x in range(0, 1) if x % 2 == 0 collect x * 2",
                f"promote(acc)",
                f"let t = 0",
                f"for y in acc {{ t += y }}",
                f"promote(t)",
                f"promote(len(acc))",
            ]]))
    return items


# ============================================================ programs lane

class ProgItem:
    def __init__(self, lines, variants=None, toplevel=False):
        self.lines = lines
        self._variants = variants or []
        self.toplevel = toplevel

    def render(self, ind):
        pad = "    " * ind
        return [pad + ln for ln in self.lines]

    def shrinks(self):
        return [ProgItem(v, toplevel=self.toplevel) for v in self._variants]


def gen_programs_case(rnd):
    items = []
    ngene = rnd.randint(1, 2)
    names = []
    for gi in range(ngene):
        gname = f"g{gi}"
        names.append(gname)
        kind = rnd.random()
        if kind < 0.40:
            k = rnd.randint(0, 12)
            body = [f"gene {gname}(n) {{",
                    f"    let acc = 0",
                    f"    let i = 0",
                    f"    while i < n {{",
                    f"        acc += i",
                    f"        i += 1",
                    f"    }}",
                    f"    return acc",
                    f"}}"]
            variants = [[f"gene {gname}(n) {{", f"    return {k2}", f"}}"]
                        for k2 in (0, 1, 2)]
            items.append(ProgItem(body, variants, toplevel=True))
        elif kind < 0.70:
            body = [f"gene {gname}(s) {{",
                    f"    let out = \"\"",
                    f"    for c in s {{",
                    f"        out = c + out",
                    f"    }}",
                    f"    if len(out) > 1 {{",
                    f"        return out + \":\" + str(len(out))",
                    f"    }} elif len(out) == 1 {{",
                    f"        return \"one\"",
                    f"    }} else {{",
                    f"        return \"empty\"",
                    f"    }}",
                    f"}}"]
            items.append(ProgItem(body, toplevel=True))
        else:
            # bounded recursion (calls the sibling gene, or itself if alone)
            other = f"g{1 - gi}" if ngene > 1 else gname
            body = [f"gene {gname}(n) {{",
                    f"    if n <= 0 {{",
                    f"        return \"base\"",
                    f"    }}",
                    f"    return {other}(n - 1) + str(n)",
                    f"}}"]
            items.append(ProgItem(body, toplevel=True))
    for _ in range(rnd.randint(1, 3)):
        g = rnd.choice(names)
        arg = rnd.randint(0, 12)
        pick = rnd.random()
        if pick < 0.35:
            variants = [ProgItem([f"promote({g}({a}))"])
                        for a in (0, 1, 2) if a != arg]
            items.append(ProgItem([f"promote({g}({arg}))"], variants))
        elif pick < 0.60:
            arms = rnd.sample([-1, 0, 1, 7, 100], rnd.randint(1, 3))
            lines = [f"let mv = {rnd.randint(0, 999)}", "match mv {"]
            for arm in arms:
                lines.append(f"    case {arm} {{ promote(\"hit {arm}\") }}")
            lines.append("    case other { promote(\"rest {other}\") }")
            lines.append("}")
            items.append(ProgItem(lines))
        elif pick < 0.80:
            a = rnd.randint(-3, 3)
            b = a + rnd.randint(0, 6)
            lines = [f"let coll = for x in range({a}, {b}) collect \"{g}\" + str(x)",
                     f"promote(coll.len())",
                     f"promote(coll.join(\"|\"))"]
            variants = [ProgItem([
                f"let coll = for x in range(0, 2) collect \"{g}\" + str(x)",
                f"promote(coll.len())",
                f"promote(coll.join(\"|\"))"])]
            items.append(ProgItem(lines, variants))
        else:
            s = rnd.choice(["gene", "wobble", "🧬x"])
            head = s[:2] if len(s) > 2 else s
            items.append(ProgItem([
                f"let s = {quote(s)}",
                f"promote(s.replace({quote(head)}, \"GG\"))",
                f"promote(s.contains(\"e\"))",
            ]))
    return items


# ============================================================ lanes table

LANES = {
    "arith":    dict(gen=gen_arith_case, prelude=""),
    "strings":  dict(gen=gen_strings_case, prelude=STR_PRELUDE),
    "colls":    dict(gen=gen_colls_case, prelude=""),
    "programs": dict(gen=gen_programs_case, prelude=""),
}


# ============================================================ main

def main():
    ap = argparse.ArgumentParser(description="W050 property-based differential harness")
    ap.add_argument("--cases", type=int, default=200,
                    help="cases per generator lane (default 200)")
    ap.add_argument("--seed", type=int, default=DEFAULT_SEED)
    ap.add_argument("--lanes", default="arith,strings,colls,programs",
                    help="comma list from: arith,strings,colls,programs")
    ap.add_argument("--bin", default=None,
                    help="operon binary (default: target/debug > bin > target/release)")
    ap.add_argument("--rust-timeout", type=float, default=20.0)
    ap.add_argument("--oracle-timeout", type=float, default=60.0)
    ap.add_argument("--dump-corpus", default=None,
                    help="write every generated program to DIR (diff-based determinism proof)")
    ap.add_argument("--no-shrink", action="store_true")
    args = ap.parse_args()

    binpath = find_binary(args.bin)
    if binpath is None:
        print("no operon binary found — cargo build first", file=sys.stderr)
        return 2
    if not os.path.isfile(ORACLE):
        print(f"missing oracle: {ORACLE}", file=sys.stderr)
        return 2
    lanes = [l.strip() for l in args.lanes.split(",") if l.strip()]
    for l in lanes:
        if l not in LANES:
            print(f"unknown lane {l!r} (choose from {', '.join(LANES)})",
                  file=sys.stderr)
            return 2

    print(f"prop harness — seed {args.seed}, {args.cases} case(s)/lane, "
          f"lanes: {', '.join(lanes)}")
    print(f"  rust  : {binpath}")
    print(f"  oracle: {ORACLE}")

    if args.dump_corpus:
        os.makedirs(args.dump_corpus, exist_ok=True)

    runner = CaseRunner(binpath, args.rust_timeout, args.oracle_timeout)
    findings = []
    t0 = time.time()

    for lane in lanes:
        rnd = random.Random(args.seed)  # fresh stream per lane, fixed order
        gen = LANES[lane]["gen"]
        prelude = LANES[lane]["prelude"]
        lane_bad = 0
        t_lane = time.time()
        for i in range(args.cases):
            items = gen(rnd)
            src = render_case(items, prelude)
            if args.dump_corpus:
                with open(os.path.join(args.dump_corpus, f"{lane}_{i:04d}.op"),
                          "w", encoding="utf-8") as fh:
                    fh.write(src)
            if runner.check(lane, i, src):
                lane_bad += 1
                rust_rc, rust_out = runner.last_rust
                py_rc, py_out = runner.last_oracle
                print(f"\n  DIVERGENCE [{lane} case {i}] seed {args.seed}")
                print(f"    rust   rc={rust_rc} out={rust_out[:300]!r}")
                print(f"    oracle rc={py_rc} out={py_out[:300]!r}")
                if not args.no_shrink:
                    items = shrink(runner, items, prelude)
                    src = render_case(items, prelude)
                    print("    minimal reproducer (shrunk):")
                    for line in src.splitlines():
                        print("      " + line)
                os.makedirs(REPRO_DIR, exist_ok=True)
                rp = os.path.join(REPRO_DIR, f"{lane}_{args.seed}_{i}.op")
                with open(rp, "w", encoding="utf-8") as fh:
                    fh.write(src)
                findings.append((lane, i, rp))
                print(f"    saved: {rp}")
            if (i + 1) % 50 == 0:
                print(f"    [{lane}] {i + 1}/{args.cases} cases, {lane_bad} "
                      f"finding(s) ({time.time() - t_lane:.1f}s)")
        print(f"  lane {lane:9s}: {args.cases} cases, {lane_bad} finding(s), "
              f"{time.time() - t_lane:.1f}s")

    if os.path.exists(TMP):
        os.remove(TMP)

    print(f"\ncases: {args.cases * len(lanes)}  findings: {len(findings)}  "
          f"wall: {time.time() - t0:.1f}s")
    print(f"determinism digest: {runner.digest.hexdigest()}")
    if findings:
        print("\nPROPERTY FINDINGS (the harness found and minimized them — "
              "it never fixes):")
        for lane, i, rp in findings:
            print(f"  ✗ {lane} case {i} -> {rp}")
        return 1
    print("all generated programs matched the oracle byte-for-byte")
    return 0


if __name__ == "__main__":
    sys.exit(main())
