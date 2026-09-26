#!/usr/bin/env python3
"""oracle.py — Operon reference implementation (bootstrap layer).

Role in the stack: the semantic oracle used by the differential test harness
to cross-check the Rust core, plus the packaging path. It implements the same
frozen SPEC.md semantics as the Rust core: Total Grammar 4-rung ladder,
gene-expression regulation layer, and the same display rules.

Deliberately sequential: spawn() runs tasks inline (deterministic), which is
equivalent for the differential corpus.
"""
import sys, os, math, json as _json

# ----------------------------------------------------------------------------
# notes / values

class Note:
    __slots__ = ("rung", "message")
    def __init__(self, rung, message):
        self.rung, self.message = rung, message

class Stress(Exception):
    def __init__(self, kind, message):
        self.kind, self.message = kind, message
        # W007 mirror: gene call chain captured during unwinding — innermost
        # frame first, (gene, call-site line); capture cap 64 matches Rust.
        self.chain = []
        # W06 (D-014) mirror: propagation marker — Some payload means this is
        # NOT a failure but a `?!` signal unwinding to the gene boundary.
        # No user path constructs a Stress with a payload (raise/stress go
        # through the kind/message constructor), so rescue can never catch
        # or spoof propagation. Every catch site pre-arms on `prop`.
        self.prop = None
    def as_map(self):
        # W07 mirror: rescue binding carries the chain, same field order as
        # the Rust stress_map (kind, message, chain). Stress.line stays a
        # Rust-side stderr-rendering field (dx-r3) — not mirrored here.
        return {"kind": self.kind, "message": self.message,
                "chain": [{"gene": n, "line": l} for (n, l) in self.chain]}

class Variant:
    """W06 (D-014) mirror: first-class Option/Result variant value.
    tag is one of {Some, None, Ok, Err}; payload is None for None and a
    value otherwise. Families are distinct: Some(x) != Ok(x) — the tag IS
    the contract."""
    __slots__ = ("tag", "payload")
    def __init__(self, tag, payload):
        self.tag, self.payload = tag, payload
    def __repr__(self):
        return v_repr(self)

class Gene:
    __slots__ = ("name", "params", "guard", "body", "acetylate", "methylate", "m6a", "copies", "seq", "riboswitch", "burst", "closure", "param_anns", "ret_ann")
    def __init__(self, name, params, guard, body, ac=False, me=False, m6=False, copies=1, seq=False, riboswitch=None, burst=None, param_anns=None, ret_ann=None):
        self.name, self.params, self.guard, self.body = name, params, guard, body
        self.acetylate, self.methylate, self.m6a, self.copies, self.seq = ac, me, m6, copies, seq
        # loop-9 (F-5): cis riboswitch (ligand, bound_means_on, threshold)
        self.riboswitch = riboswitch
        # loop-9 (F-2): per-gene promoter identity (kon, koff)
        self.burst = burst
        # W01 (L2c): soft type annotations (mirror of GeneDef.param_anns/ret_ann)
        self.param_anns = param_anns if param_anns is not None else []
        self.ret_ann = ret_ann
        self.closure = None

ENHANCE_DELTA = 0.25  # T2e: super-enhancer activation boost (GRN threshold reduction)
M64 = 0xFFFFFFFFFFFFFFFF

# reg-bio (F-5): default ring kinetics — the historical constants.
DEFAULT_REPRESSI = {"alpha": 10.0, "gamma": 1.0, "hill": 4, "basal": 0.0, "noise": 0.0,
                    "seed": 0x9E3779B97F4A7C15}


def repressilator_levels(n, tick, params=None):
    """A11 (reg-r2) / reg-bio (F-5): mirror of the Rust `repressilator_levels_p`.

    Discrete Elowitz–Leibler ring: dA/dt = α/(1 + R^h) + basal − γA,
    Euler-integrated, 20 substeps of dt=0.05 per ring tick, init [5, 0, ...].
    Node j's repressor is node (j+n−1) mod n. With default params (noise off)
    this is bit-identical to the historical form. The optional noise kick is
    drawn per (tick, substep, node) from a stream derived from the ABSOLUTE
    position — the same derivation as the Rust core, so fold-from-init and
    incremental-cache paths agree bit-for-bit in BOTH implementations.
    """
    if n == 0:
        return []
    p = params if params is not None else DEFAULT_REPRESSI
    ALPHA, GAMMA, DT, SUB = p["alpha"], p["gamma"], 0.05, 20
    HILL, BASAL, NOISE, SEED = p["hill"], p["basal"], p["noise"], p["seed"]
    lv = [0.0] * n
    lv[0] = 5.0
    for t in range(tick):
        for ss in range(SUB):
            nx = 0
            if NOISE > 0.0:
                nx = (SEED ^ ((t * 0x9E3779B97F4A7C15) & M64)
                      ^ ((ss * 0xBF58476D1CE4E5B9) & M64)) & M64
                if nx == 0:
                    nx = 0x9E3779B97F4A7C15
            snap = lv[:]
            for j in range(n):
                rep = snap[(j + n - 1) % n]
                # rep^hill via repeated multiplication — op-identical to Rust
                # (default h=4 == the historical rep*rep*rep*rep exactly)
                rh = 1.0
                for _k in range(HILL):
                    rh *= rep
                d = ALPHA / (1.0 + rh) + BASAL - GAMMA * snap[j]
                v = snap[j] + DT * d
                if NOISE > 0.0:
                    nx ^= (nx >> 12) & M64
                    nx ^= (nx << 25) & M64
                    nx ^= (nx >> 27) & M64
                    nx &= M64
                    u = ((nx >> 11) & ((1 << 53) - 1)) / 9007199254740992.0
                    # reg-bio-2 (D6): multiplicative, dt-aware noise — op-for-op
                    # mirror of the Rust core. The constant is the literal double
                    # nearest 1/√20 on BOTH sides (no math.sqrt call — the value
                    # is fixed text, so IEEE parity is trivial).
                    v = v * (1.0 + NOISE * 0.22360679774997896 * (2.0 * u - 1.0))
                lv[j] = v if v > 0.0 else 0.0
    return lv

class Pheno:
    __slots__ = ("name", "parent", "fields", "methods")
    def __init__(self, name, parent, fields, methods):
        self.name, self.parent, self.fields, self.methods = name, parent, fields, methods

class ObjInst:
    __slots__ = ("defn", "fields")
    def __init__(self, defn, fields):
        self.defn, self.fields = defn, fields

class SeqObj:
    """Sequential-oracle sequence: values are produced by running the body to
    completion on first pull (buffered), then served one at a time. The Rust
    core pulls lazily via a rendezvous worker; corpus-visible output is
    identical because the differential programs do not print inside bodies."""
    __slots__ = ("interp", "gene", "args", "buf", "idx", "done")
    def __init__(self, interp, gene, args):
        self.interp, self.gene, self.args = interp, gene, args
        self.buf = None
        self.idx = 0
        self.done = False

    def pull(self):
        if self.buf is None:
            self.buf = []
            saved = self.interp.seq_buffer
            self.interp.seq_buffer = self.buf
            try:
                g = self.gene
                fenv = self.interp.new_scope(g.closure if g.closure is not None else self.interp.globals)
                for i, (pname, dflt) in enumerate(g.params):
                    if pname in ("?", ""):
                        continue
                    if i < len(self.args):
                        fenv[pname] = self.args[i]
                    elif dflt is not None:
                        fenv[pname] = self.interp.eval(fenv, dflt)
                    else:
                        fenv[pname] = None
                try:
                    self.interp.exec_block(fenv, g.body)
                except Return:
                    pass
                except Stress as st:
                    # W06 (D-014) mirror: propagation inside a sequence ends
                    # the stream — sequences are streams, not answers, so the
                    # variant has no return path; the stream ends cleanly
                    # (never leaked as a kind). Matches genes.rs run_seq_body.
                    if st.prop is not None:
                        self.interp.note(4, "propagation ended the sequence")
                    else:
                        raise
            finally:
                self.interp.seq_buffer = saved
        if self.idx < len(self.buf):
            v = self.buf[self.idx]
            self.idx += 1
            return v
        return None

class Return(Exception):
    def __init__(self, value):
        self.value = value

class BreakLoop(Exception):
    pass

class ContinueLoop(Exception):
    pass

# display rules (must match Rust value.rs)
def fmt_float(f):
    if f != f:
        return "nan"
    if f == float("inf"):
        return "inf"
    if f == float("-inf"):
        return "-inf"
    # Python repr IS the canonical float format (shortest round-trip,
    # scientific when exp < -4 or >= 16, ".0" on integral positional values)
    return repr(f)

def escape_str(s):
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t")

def is_identlike(s):
    return bool(s) and (s[0].isalpha() or s[0] == "_") and all(c.isalnum() or c == "_" for c in s)

def v_repr(v, _seen=None, _depth=0):
    # reg-r4 (re-audit B-3): cycle-safe — a container containing itself
    # renders the [...] / {...} marker at depth > 256 or on a re-entry
    # (mirror of the Rust core; the old version recursed forever)
    if _seen is None:
        _seen = set()
    if _depth > 256:
        return "[...]"
    if v is None: return "null"
    if v is True: return "true"
    if v is False: return "false"
    if isinstance(v, int): return str(v)
    if isinstance(v, float): return fmt_float(v)
    if isinstance(v, str): return f'"{escape_str(v)}"'
    if isinstance(v, list):
        marker = id(v)
        if marker in _seen:
            return "[...]"
        _seen.add(marker)
        out = "[" + ", ".join(v_repr(x, _seen, _depth + 1) for x in v) + "]"
        _seen.discard(marker)
        return out
    if isinstance(v, dict):
        marker = id(v)
        if marker in _seen:
            return "{...}"
        _seen.add(marker)
        out = "{" + ", ".join(
            (k if isinstance(k, str) and is_identlike(k) else v_repr(k, _seen, _depth + 1))
            + ": " + v_repr(val, _seen, _depth + 1)
            for k, val in v.items()) + "}"
        _seen.discard(marker)
        return out
    if isinstance(v, Gene):
        return f"<gene {v.name}>" if v.name else "<gene lambda>"
    # W06 mirror: variant repr mirrors mainstream constructor syntax; the
    # payload renders through v_repr so depth/cycle caps apply.
    if isinstance(v, Variant):
        if v.tag == "None":
            return "None"
        if v.payload is None:
            return v.tag
        return f"{v.tag}({v_repr(v.payload, _seen, _depth + 1)})"
    if isinstance(v, SeqObj):
        return f"<sequence {v.gene.name}>" if v.gene.name else "<sequence lambda>"
    if isinstance(v, ObjInst):
        return f"<phenotype {v.defn.name}>"
    return "<?>" 

def v_display(v):
    if isinstance(v, str):
        return v
    return v_repr(v)

def truthy(v):
    if v is None or v is False: return False
    if v is True: return True
    if isinstance(v, (int, float)): return v != 0
    if isinstance(v, str): return len(v) > 0
    if isinstance(v, list): return len(v) > 0
    if isinstance(v, dict): return len(v) > 0
    # W06 mirror: a carried success is truthy; a carried failure is falsy
    if isinstance(v, Variant): return v.tag in ("Some", "Ok")
    return True

# W01 (L2c) — soft annotation matching (mirror of interp.rs ann_matches).
# A value matches by type_name(); `any` accepts everything; `float` accepts
# int (safe numeric widening — `int` refuses float: no silent narrowing);
# unions match any alternative; optionals additionally accept null.
def ann_matches(v, ann):
    k = ann[0]
    if k == "named":
        name = ann[1]
        if name == "any":
            return True
        if type_name(v) == name:
            return True
        return name == "float" and isinstance(v, int) and not isinstance(v, bool)
    if k == "union":
        return any(ann_matches(v, a) for a in ann[1])
    if k == "opt":
        return v is None or ann_matches(v, ann[1])
    return False

def ann_render(ann):
    # Canonical rendering — must match TypeAnn::render op-for-op.
    k = ann[0]
    if k == "named":
        return ann[1]
    if k == "union":
        return " | ".join(ann_render(a) for a in ann[1])
    if k == "opt":
        return ann_render(ann[1]) + "?"
    return "any"

def type_name(v):
    if v is None: return "null"
    if isinstance(v, bool): return "bool"
    if isinstance(v, int): return "int"
    if isinstance(v, float): return "float"
    if isinstance(v, str): return "str"
    if isinstance(v, list): return "list"
    if isinstance(v, dict): return "map"
    if isinstance(v, Gene): return "gene"
    if isinstance(v, SeqObj): return "sequence"
    if isinstance(v, ObjInst): return "phenotype"
    if isinstance(v, Variant): return "option" if v.tag in ("Some", "None") else "result"
    return "native"

def deep_eq(a, b, _pairs=None):
    # reg-r4: cycle-safe — a pair of containers already being compared is
    # treated as equal (mirror of the Rust deep_eq's seen-pair set); the
    # old version recursed forever on `cyc == cyc`
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return a == b
    # W06 (D-014) mirror: variants equal iff same tag + payload deep_eq;
    # families distinct (Some(x) != Ok(x)); None == None.
    if isinstance(a, Variant) or isinstance(b, Variant):
        if not (isinstance(a, Variant) and isinstance(b, Variant)):
            return False
        if a.tag != b.tag:
            return False
        if a.payload is None and b.payload is None:
            return True
        if a.payload is None or b.payload is None:
            return False
        return deep_eq(a.payload, b.payload, _pairs)
    if type(a) is not type(b) and not (a is None and b is None):
        if isinstance(a, (int, float)) and isinstance(b, (int, float)):
            return a == b
        return False
    if isinstance(a, (list, dict)):
        if _pairs is None:
            _pairs = set()
        key = (id(a), id(b))
        if key in _pairs:
            return True  # already comparing this pair (cycle)
        _pairs.add(key)
        try:
            if isinstance(a, list):
                return len(a) == len(b) and all(
                    deep_eq(x, y, _pairs) for x, y in zip(a, b))
            if len(a) != len(b):
                return False
            return all(any(deep_eq(k, k2, _pairs) and deep_eq(v, v2, _pairs)
                           for k2, v2 in b.items()) for k, v in a.items())
        finally:
            _pairs.discard(key)
    return a == b

# ----------------------------------------------------------------------------
# lexer

SYMBOLS = ["**=", "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "==", "!=", "<=", ">=", "&&", "||",
           "//", "->", "=>", "**", "<<", ">>", "??", "?.", "?!", "&", "|", "^", "~", "?", "{", "}", "(", ")", "[", "]", ",", ":", "+", "-",
           "*", "/", "%", "=", "<", ">", "!", ".", ";"]

def lex(src):
    toks, notes = [], []
    i, line, n = 0, 1, len(src)
    while i < n:
        c = src[i]
        if c in " \t\r":
            i += 1; continue
        if c == "\n":
            if not toks or toks[-1][0] != "NL":
                toks.append(("NL", None, line))
            line += 1; i += 1; continue
        if c == "#":
            while i < n and src[i] != "\n":
                i += 1
            continue
        # W030 mirror: raw strings r"..." — no escapes, no interpolation
        if c == "r" and i + 1 < n and src[i+1] == '"':
            i += 2
            raw, closed = "", False
            while i < n:
                if src[i] == '"':
                    i += 1; closed = True; break
                if src[i] == "\n":
                    line += 1
                raw += src[i]; i += 1
            if not closed:
                notes.append(Note(4, "unclosed raw string consumed to end of input"))
            toks.append(("STR", raw, line))
            continue
        # W030 mirror: multiline triple-quoted strings """...""" — escapes and
        # interpolation processed, content verbatim
        if c == '"' and i + 2 < n and src[i+1] == '"' and src[i+2] == '"':
            i += 3
            raw, closed, interp, depth = "", False, False, 0
            while i < n:
                if src[i] == '"' and i + 2 < n and src[i+1] == '"' and src[i+2] == '"':
                    i += 3; closed = True; break
                if src[i] == "\\" and i + 1 < n:
                    e = src[i + 1]
                    raw += {"n": "\n", "t": "\t", "\\": "\\", '"': '"', "{": "{", "}": "}"}.get(e, "\\" + e)
                    i += 2; continue
                if src[i] == "\n":
                    line += 1
                if src[i] == "{":
                    depth += 1; interp = True
                if src[i] == "}" and depth > 0:
                    depth -= 1
                raw += src[i]; i += 1
            if not closed:
                notes.append(Note(4, "unclosed multiline string consumed to end of input"))
            toks.append(("INTERP" if interp else "STR", raw, line))
            continue
        if c == '"':
            raw, i2, closed, interp = "", i + 1, False, False
            depth = 0
            while i2 < n:
                ch = src[i2]
                if ch == "\\" and i2 + 1 < n:
                    e = src[i2 + 1]
                    raw += {"n": "\n", "t": "\t", "\\": "\\", '"': '"', "{": "{", "}": "}"}.get(e, "\\" + e)
                    i2 += 2; continue
                if depth == 0 and ch == '"':
                    closed = True; i2 += 1; break
                if ch == "\n":
                    line += 1
                if ch == "{":
                    depth += 1; interp = True
                if ch == "}" and depth > 0:
                    depth -= 1
                raw += ch; i2 += 1
            if not closed:
                notes.append(Note(4, "unclosed string consumed to end of line"))
            toks.append(("INTERP" if interp else "STR", raw, line))
            i = i2; continue
        if c == "'":
            notes.append(Note(4, "single-quoted string repaired to double quotes"))
            raw, i2, closed = "", i + 1, False
            while i2 < n:
                ch = src[i2]
                if ch == "\\" and i2 + 1 < n:
                    e = src[i2 + 1]
                    raw += {"n": "\n", "t": "\t", '"': '"'}.get(e, e)
                    i2 += 2; continue
                if ch == "'":
                    closed = True; i2 += 1; break
                if ch == "\n":
                    line += 1
                raw += ch; i2 += 1
            if not closed:
                notes.append(Note(4, "unclosed string consumed to end of line"))
            toks.append(("STR", raw, line))
            i = i2; continue
        if c == "@":
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            if j > i + 1:
                toks.append(("MARK", src[i + 1:j], line))
            else:
                notes.append(Note(4, "stray '@' skipped"))
            i = j; continue
        if c.isdigit():
            # W031 mirror: radix prefixes 0x/0b/0o (case-insensitive) with `_`
            # separators; prefix with no valid digit falls through to decimal
            if c == "0" and i + 1 < n and src[i+1] in "xXbBoO":
                radix = {"x": 16, "X": 16, "b": 2, "B": 2, "o": 8, "O": 8}[src[i+1]]
                vset = "0123456789abcdefABCDEF" if radix == 16 else ("01" if radix == 2 else "01234567")
                if i + 2 < n and (src[i+2] in vset or src[i+2] == "_"):
                    j = i + 2
                    while j < n and (src[j] in vset or src[j] == "_"):
                        j += 1
                    raw = src[i:j]
                    digits = raw[2:].replace("_", "")
                    val = int(digits, radix)
                    toks.append(("INT", val, line))
                    # i64 parity: out-of-range treated as 0 with the same note
                    if not (-2**63 <= val <= 2**63 - 1):
                        notes.append(Note(4, f"integer '{raw}' out of range treated as 0"))
                        toks[-1] = ("INT", 0, line)
                    i = j; continue
            j = i
            isf = False
            while j < n and (src[j].isdigit() or src[j] == "." or src[j] == "_"):
                if src[j] == ".":
                    if j + 1 >= n or not src[j + 1].isdigit():
                        break
                    isf = True
                j += 1
            if j < n and src[j] in "eE":
                k = j + 1
                if k < n and src[k] in "+-":
                    k += 1
                if k < n and src[k].isdigit():
                    isf = True
                    j = k
                    while j < n and src[j].isdigit():
                        j += 1
            text = src[i:j]
            # W031 mirror: `_` separators stripped before parsing
            cleaned = text.replace("_", "")
            try:
                toks.append(("FLOAT", float(cleaned), line) if isf else (("INT", int(cleaned), line)))
            except ValueError:
                notes.append(Note(4, f"malformed number '{text}' treated as 0"))
                toks.append(("INT", 0, line))
            else:
                # parity with the Rust lexer: an integer literal beyond i64
                # is out of range and treated as 0 with the same note (the
                # Rust core parses i64; a Python bignum would otherwise see
                # a value the compiled engine never did)
                if not isf and not (-2**63 <= int(cleaned) <= 2**63 - 1):
                    notes.append(Note(4, f"integer '{text}' out of range treated as 0"))
                    toks[-1] = ("INT", 0, line)
            i = j; continue
        if c.isalpha() or c == "_":
            j = i
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            toks.append(("IDENT", src[i:j], line))
            i = j; continue
        if c == "?" and i + 2 < n and src[i+1] == "." and src[i+2].isdigit():
            # L1a parity guard: '?.' followed by a digit lexes as '?' + number-dot
            toks.append(("SYM", "?", line))
            i += 1
            continue
        matched = False
        for sym in SYMBOLS:
            if src.startswith(sym, i):
                toks.append(("SYM", sym, line))
                i += len(sym)
                matched = True
                break
        if not matched:
            notes.append(Note(4, f"unexpected character '{c}' skipped"))
            i += 1
    toks.append(("EOF", None, line))
    return toks, notes

# ----------------------------------------------------------------------------
# parser — Total Grammar ladder

KEYWORDS = set("""gene let if elif else while loop for in return break continue match case use
tad anchor export import enhance silence stress rescue raise fate state regulate activates
inhibits strength toggle repressilator period frame proof guard splice variant edit replace
ires as collect enter phenotype sequence yield new threshold from self operon""".split())

ARMS = set("""gene let if elif else while loop for return break continue match use tad anchor
enhance silence stress raise fate regulate toggle repressilator frame splice edit ires
phenotype sequence yield operon""".split())

SYNONYMS = {
    "fn": "gene", "func": "gene", "fun": "gene", "def": "gene", "sub": "gene",
    "lambda": "gene", "proc": "gene", "var": "let", "val": "let", "const": "let",
    "foreach": "for", "each": "for", "import": "use", "include": "use",
    "require": "use", "ret": "return", "stop": "break", "next": "continue",
    "skip": "continue", "elseif": "elif",
    "class": "phenotype", "struct": "phenotype", "record": "phenotype", "prototype": "phenotype",
    "type": "phenotype", "generator": "sequence", "gen": "sequence", "stream": "sequence", "iter": "sequence",
}
VALUE_SYNONYMS = {"yes": True, "on": True, "no": False, "off": False,
                  "nil": None, "nothing": None}
# W06 (D-014) mirror: 'none' RETIRED from VALUE_SYNONYMS — it is now the
# Option constructor none(); bare 'none' degrades to an unbound ident
# (phantom note), never a silent null. Matches src/parser.rs.
MARKS = {"acetylate", "methylate", "m6a", "copies", "riboswitch", "burst"}

def edit_distance(a, b):
    if a == b:
        return 0
    la, lb = len(a), len(b)
    if la == 0: return lb
    if lb == 0: return la
    prev = list(range(lb + 1))
    for i in range(1, la + 1):
        cur = [i] + [0] * lb
        for j in range(1, lb + 1):
            cost = 0 if a[i - 1] == b[j - 1] else 1
            cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + cost)
        prev = cur
    return prev[lb]

def wobble_keyword(word):
    limit = 1 if len(word) <= 4 else 2
    best, bd = None, 99
    for k in KEYWORDS:
        d = edit_distance(word, k)
        if d <= limit and d < bd:
            best, bd = k, d
    return best

class P:
    def __init__(self, toks, notes):
        self.toks, self.pos, self.notes = toks, 0, notes

    def peek(self):
        return self.toks[self.pos]

    def next(self):
        t = self.toks[self.pos]
        if self.pos < len(self.toks) - 1:
            self.pos += 1
        return t

    def note(self, line, rung, msg):
        self.notes.append(Note(rung, msg))

    def eat_nl(self):
        while self.peek()[0] in ("NL", "SYM") and (self.peek()[0] == "NL" or self.peek()[1] == ";"):
            self.next()

    def expect_kw(self, word):
        t = self.peek()
        if t[0] == "IDENT" and t[1] == word:
            self.next(); return True
        if t[0] == "IDENT":
            syn = SYNONYMS.get(t[1])
            if syn == word:
                self.note(t[2], 2, f"synonym '{t[1]}' repaired to '{word}'")
                self.next(); return True
            wk = wobble_keyword(t[1])
            if wk == word:
                self.note(t[2], 3, f"wobble: '{t[1]}' repaired to keyword '{word}'")
                self.next(); return True
        return False

    def at_ident(self, w):
        t = self.peek()
        return t[0] == "IDENT" and t[1] == w

    # program
    def program(self):
        out = []
        while True:
            self.eat_nl()
            if self.peek()[0] == "EOF":
                break
            if self.peek() == ("SYM", "}", self.peek()[2]):
                self.note(self.peek()[2], 4, "unmatched '}' skipped")
                self.next(); continue
            before = self.pos
            s = self.stmt()
            if s is not None:
                out.append(s)
            if self.pos == before:
                self.note(self.peek()[2], 4, f"token skipped")
                self.next()
        return out

    # statements
    def stmt(self):
        t = self.peek()
        if t[0] == "MARK":
            marks = []
            # loop-9: stacked marks with newlines/semis between them (the
            # dx-r6 newline bridge now applies BETWEEN marks too)
            while True:
                self.eat_nl()
                if self.peek()[0] != "MARK":
                    break
                m = self.next()[1]
                if m in MARKS:
                    marks.append(m)
                else:
                    wk = None
                    for k in MARKS:
                        if edit_distance(m, k) <= (1 if len(m) <= 4 else 2):
                            self.note(t[2], 3, f"wobble: '@{m}' repaired to '@{k}'")
                            wk = k
                            break
                    if wk:
                        marks.append(wk)
                    else:
                        self.note(t[2], 4, f"unknown mark '@{m}' skipped")
            # reg-bio-3 (C10): `@copies n` carries its dosage argument
            copies = 1
            if "copies" in marks:
                tt = self.peek()
                if tt[0] == "INT":
                    self.next()
                    if tt[1] < 1 or tt[1] > 64:
                        self.note(tt[2], 4, "gene dosage clamped to 1..=64 copies (a level is a concentration, not an amplifier)")
                    copies = min(max(tt[1], 1), 64)
                else:
                    self.note(tt[2], 4, "@copies needs an integer 1..=64; default 1")
            # loop-9 (F-5): @riboswitch ligand off|on threshold t — a cis
            # aptamer on this gene's own transcript (mirror of the Rust drain)
            riboswitch = None
            if "riboswitch" in marks:
                tt = self.peek()
                lig = tt[1] if tt[0] == "IDENT" else ""
                if lig:
                    self.next()
                else:
                    self.note(tt[2], 4, "@riboswitch needs a ligand name; mark ignored")
                cls = self.peek()
                on = None
                if lig and cls[0] == "IDENT" and cls[1] in ("on", "off"):
                    self.next()
                    on = cls[1] == "on"
                elif lig:
                    self.note(cls[2], 4, "@riboswitch needs 'on' or 'off'; mark ignored")
                threshold = 0.5
                if on is not None:
                    nt = self.peek()
                    if nt[0] == "IDENT" and nt[1] == "threshold":
                        self.next()
                        vt = self.peek()
                        if vt[0] in ("FLOAT", "INT"):
                            self.next()
                            threshold = min(max(float(vt[1]), 0.0), 1.0)
                        else:
                            self.note(vt[2], 4, "riboswitch threshold needs a number 0..=1; default 0.5")
                    riboswitch = (lig, on, threshold)
            # loop-9 (F-2): @burst kon koff — per-gene promoter identity
            burst = None
            if "burst" in marks:
                vals = [0.3, 0.1]
                got = 0
                while got < 2:
                    vt = self.peek()
                    if vt[0] in ("FLOAT", "INT") and not isinstance(vt[1], bool):
                        self.next()
                        vals[got] = min(max(float(vt[1]), 0.0), 1.0)
                        got += 1
                    else:
                        break
                if got < 2:
                    self.note(t[2], 4, "@burst needs kon and koff (0..=1); defaults 0.3/0.1")
                burst = (vals[0], vals[1])
            self.eat_nl()
            if not self.expect_kw("gene"):
                self.note(t[2], 4, "mark must precede 'gene'; skipped line")
                self.skip_line()
                return None
            return self.gene_def(marks, copies, riboswitch, burst)
        if t[0] == "SYM" and t[1] == "{":
            self.note(t[2], 4, "bare block treated as scoped statements")
            return ("block", self.block())
        if t[0] == "IDENT":
            return self.word_stmt(t[1])
        self.note(t[2], 4, f"unexpected token at statement position")
        self.next()
        return None

    def skip_line(self):
        while self.peek()[0] not in ("NL", "EOF") and not (self.peek()[0] == "SYM" and self.peek()[1] == "}"):
            self.next()

    def end_stmt(self):
        if self.peek()[0] == "NL" or (self.peek()[0] == "SYM" and self.peek()[1] == ";"):
            self.next()

    def word_stmt(self, w):
        t1 = self.toks[self.pos + 1] if self.pos + 1 < len(self.toks) else ("EOF", None, 0)
        expr_head = (t1[0] == "SYM" and t1[1] in ("=", "(", "[", ".", "+=", "-=", "*=", "/=", "%="))
        word = w
        if w not in KEYWORDS and not expr_head:
            syn = SYNONYMS.get(w)
            if syn and syn in ARMS:
                self.note(self.peek()[2], 2, f"synonym '{w}' repaired to '{syn}'")
                word = syn
            else:
                wk = wobble_keyword(w)
                if wk and wk in ARMS:
                    self.note(self.peek()[2], 3, f"wobble: '{w}' repaired to keyword '{wk}'")
                    word = wk
        if word == "gene":
            self.next()
            return self.gene_def([])
        if word == "let":
            self.next()
            t = self.peek()
            # L1a: destructuring definitions
            if t[0] == "SYM" and t[1] in ("[", "{"):
                pat = self.destructure_pat()
                if self.peek() == ("SYM", "=", self.peek()[2]):
                    self.next()
                    e = self.expr()
                    self.end_stmt()
                    return ("letpat", pat, e)
                self.note(self.peek()[2], 4, "destructured 'let' without value binds nulls")
                self.end_stmt()
                return ("letpat", pat, ("null",))
            name = self.ident()
            # L1a: `let a, b = 1, 2` multi-define
            if self.peek() == ("SYM", ",", self.peek()[2]):
                names = [name]
                while self.peek() == ("SYM", ",", self.peek()[2]):
                    self.next()
                    names.append(self.ident())
                if self.peek() == ("SYM", "=", self.peek()[2]):
                    self.next()
                    values = [self.expr()]
                    while self.peek() == ("SYM", ",", self.peek()[2]):
                        self.next()
                        values.append(self.expr())
                    self.end_stmt()
                    return ("multi", [("ident", n) for n in names], values, True)
                self.note(self.peek()[2], 4, "multi 'let' without value binds nulls")
                self.end_stmt()
                return ("multi", [("ident", n) for n in names], [("null",)] * len(names), True)
            # W01 (L2c): soft type annotation — `let n: int = 3`
            t2 = self.peek()
            if t2[0] == "SYM" and t2[1] == ":":
                self.next()
                ann = self.type_ann()
                if self.peek() == ("SYM", "=", self.peek()[2]):
                    self.next()
                    e = self.expr()
                    self.end_stmt()
                    return ("letann", name, ann, e)
                self.note(self.peek()[2], 4, f"'let {name}: {ann_render(ann)}' without value binds null")
                self.end_stmt()
                return ("letann", name, ann, ("null",))
            if self.peek() == ("SYM", "=", self.peek()[2]):
                self.next()
                e = self.expr()
                self.end_stmt()
                return ("let", name, e)
            self.note(self.peek()[2], 4, f"'let {name}' without value binds null")
            self.end_stmt()
            return ("let", name, ("null",))
        if word in ("if",):
            self.next()
            branches = [(self.expr(), self.block())]
            els = None
            while True:
                self.eat_nl()
                if self.at_ident("elif"):
                    self.next()
                    branches.append((self.expr(), self.block()))
                elif self.expect_kw("else"):
                    els = self.block()
                    break
                else:
                    break
            return ("if", branches, els)
        if word == "while":
            self.next()
            cond = self.expr()
            return ("while", cond, self.block())
        if word == "loop":
            self.next()
            return ("loop", self.block())
        if word == "for":
            self.next()
            t = self.peek()
            # L1a: destructuring loop target
            if t[0] == "SYM" and t[1] in ("[", "{"):
                pat = self.destructure_pat()
                if not self.expect_kw("in"):
                    self.note(self.peek()[2], 4, "'for <pattern>' missing 'in'; iterating null")
                it = self.expr()
                return ("forpat", pat, it, self.block())
            name = self.ident()
            if not self.expect_kw("in"):
                self.note(self.peek()[2], 4, f"'for {name}' missing 'in'; iterating null")
            it = self.expr()
            return ("for", name, it, self.block())
        if word == "return":
            self.next()
            k = self.peek()[0]
            if k in ("NL", "EOF") or (k == "SYM" and self.peek()[1] in (";", "}")):
                self.end_stmt()
                return ("return", ("null",))
            e = self.expr()
            self.end_stmt()
            return ("return", e)
        if word == "break":
            self.next(); self.end_stmt(); return ("break",)
        if word == "continue":
            self.next(); self.end_stmt(); return ("continue",)
        if word == "match":
            self.next()
            subj = self.expr()
            cases = []
            if self.peek() == ("SYM", "{", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t = self.peek()
                    if t[0] == "SYM" and t[1] == "}":
                        self.next(); break
                    if t[0] == "EOF":
                        self.note(t[2], 4, "match block auto-closed at end of file")
                        break
                    if not self.expect_kw("case"):
                        self.note(t[2], 4, "expected 'case' in match; skipped token")
                        self.next(); continue
                    pat = self.pattern()
                    if self.peek() == ("SYM", "=>", t[2]):
                        self.next()
                        e = self.expr()
                        self.end_stmt()
                        body = [("expr", e)]
                    else:
                        body = self.block()
                    cases.append((pat, body))
            return ("match", subj, cases)
        if word == "use":
            self.next()
            path = self.use_path()
            alias = None
            if self.expect_kw("as"):
                alias = self.ident()
            self.end_stmt()
            return ("use", path, alias)
        if word == "tad":
            self.next()
            name = self.ident()
            return ("tad", name, self.block())
        if word == "anchor":
            self.next()
            is_export = self.expect_kw("export")
            if not is_export:
                self.expect_kw("import")
            names = [self.ident()]
            while self.peek() == ("SYM", ",", self.peek()[2]):
                self.next()
                names.append(self.ident())
            self.end_stmt()
            return ("anchor_export" if is_export else "anchor_import", [n for n in names if n])
        if word == "enhance":
            self.next()
            names = [self.ident()]
            while self.peek() == ("SYM", ",", self.peek()[2]):
                self.next()
                names.append(self.ident())
            self.end_stmt()
            return ("enhance", [n for n in names if n])
        if word == "silence":
            # reg-bio-3 (C9): stoichiometric RISC — strength/sites
            self.next()
            frm = self.ident()
            to = None
            if self.peek() == ("SYM", "->", self.peek()[2]):
                self.next()
                to = self.ident()
            strength, sites = 1.0, 1
            t2 = self.peek()
            if t2[0] == "IDENT" and t2[1] == "strength":
                self.next()
                tt = self.peek()
                if tt[0] in ("INT", "FLOAT"):
                    strength = min(max(float(tt[1]), 0.0), 1.0)
                    self.next()
                else:
                    self.note(tt[2], 4, "strength needs a number 0..=1; ignored")
            t2 = self.peek()
            if t2[0] == "IDENT" and t2[1] == "sites":
                self.next()
                tt = self.peek()
                if tt[0] == "INT":
                    if tt[1] < 1 or tt[1] > 64:
                        self.note(tt[2], 4, "sites clamped to 1..=64")
                    sites = min(max(tt[1], 1), 64)
                    self.next()
                else:
                    self.note(tt[2], 4, "sites needs an integer 1..=64; ignored")
            self.end_stmt()
            return ("silence", frm, to, strength, sites)
        if word == "stress":
            self.next()
            kind = None
            t = self.peek()
            nt = self.toks[self.pos + 1] if self.pos + 1 < len(self.toks) else ("EOF", None, 0)
            if t[0] == "IDENT" and nt == ("SYM", "{", t[2]):
                kind = t[1]
                self.next()
            body = self.block()
            rescue = None
            self.eat_nl()
            if self.expect_kw("rescue"):
                bind = None
                if self.peek() == ("SYM", "(", self.peek()[2]):
                    self.next()
                    if self.peek()[0] == "IDENT":
                        bind = self.next()[1]
                    if self.peek() == ("SYM", ")", self.peek()[2]):
                        self.next()
                rescue = (bind, self.block())
            return ("stress", kind, body, rescue)
        if word == "raise":
            self.next()
            save = self.pos
            t = self.peek()
            if t[0] == "IDENT" and t[1] in ("unfolded", "missing", "overflow", "burned", "interference"):
                self.next()
                if self.peek() == ("SYM", ",", t[2]):
                    self.next()
                    e = self.expr()
                    self.end_stmt()
                    return ("raise", t[1], e)
                self.pos = save
            e = self.expr()
            self.end_stmt()
            return ("raise", None, e)
        if word == "fate":
            self.next()
            name = self.ident()
            states, enter = [], None
            if self.peek() == ("SYM", "{", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t = self.peek()
                    if t[0] == "SYM" and t[1] == "}":
                        self.next(); break
                    if t[0] == "EOF":
                        break
                    if t[0] == "IDENT" and (t[1] == "state" or edit_distance(t[1], "state") <= 1):
                        if t[1] != "state":
                            self.note(t[2], 3, f"wobble: '{t[1]}' repaired to 'state'")
                        self.next()
                        frm = self.ident()
                        targets = []
                        if self.peek() == ("SYM", "->", t[2]):
                            self.next()
                            targets.append(self.ident())
                            while self.peek() == ("SYM", ",", t[2]):
                                self.next()
                                targets.append(self.ident())
                        states.append((frm, [x for x in targets if x]))
                        self.end_stmt()
                    elif t[0] == "IDENT" and (t[1] == "enter" or edit_distance(t[1], "enter") <= 1):
                        if t[1] != "enter":
                            self.note(t[2], 3, f"wobble: '{t[1]}' repaired to 'enter'")
                        self.next()
                        enter = self.ident()
                        self.end_stmt()
                    else:
                        self.note(t[2], 4, "unexpected token in fate block; skipped")
                        self.next()
            return ("fate", name, states, enter)
        if word == "regulate":
            self.next()
            edges = []
            trans = []
            binds = []
            if self.peek() == ("SYM", "{", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t = self.peek()
                    if t[0] == "SYM" and t[1] == "}":
                        self.next(); break
                    if t[0] == "EOF":
                        break
                    if t[0] != "IDENT":
                        self.note(t[2], 4, "unexpected token in regulate block; skipped")
                        self.next(); continue
                    frm = self.next()[1]
                    # reg-bio-2 (C1): `a translates b rate r decay d;`
                    if self.expect_kw("translates"):
                        to = self.ident()
                        rate = None
                        if self.expect_kw("rate"):
                            tt = self.peek()
                            if tt[0] in ("INT", "FLOAT"):
                                rate = float(tt[1])
                                self.next()
                            else:
                                self.note(tt[2], 4, "rate needs a number; using 1.0")
                        pdecay = None
                        if self.expect_kw("decay"):
                            tt = self.peek()
                            if tt[0] in ("INT", "FLOAT"):
                                pdecay = min(max(float(tt[1]), 0.0), 1.0)
                                self.next()
                            else:
                                self.note(tt[2], 4, "decay needs a number; ignored")
                        trans.append((frm, to, rate, pdecay))
                        self.end_stmt()
                        continue
                    # reg-bio-2 (A4): `bind tf inducer lg k v;` — `bind` is a
                    # HEAD keyword here (no edge source); head already consumed.
                    if frm == "bind":
                        tf = self.ident()
                        inducer = None
                        if self.expect_kw("inducer"):
                            inducer = True
                        elif self.expect_kw("cofactor"):
                            inducer = False
                        if inducer is None:
                            self.note(t[2], 4, "bind needs 'inducer' or 'cofactor'; binding dropped")
                            self.skip_line(); continue
                        lg = self.ident()
                        k = 0.1
                        if self.expect_kw("k"):
                            tt = self.peek()
                            if tt[0] in ("INT", "FLOAT"):
                                k = float(tt[1])
                                self.next()
                            else:
                                self.note(tt[2], 4, "k needs a number; using 0.1")
                            if k <= 0.0:
                                self.note(tt[2], 4, "k must be > 0; using 0.1")
                                k = 0.1
                        binds.append((tf, lg, inducer, k))
                        self.end_stmt()
                        continue
                    # reg-bio-2 (A5): per-edge attenuator flag
                    attenuating = False
                    inhibit = None
                    if self.expect_kw("activates"):
                        inhibit = False
                    elif self.expect_kw("inhibits"):
                        inhibit = True
                    elif self.expect_kw("attenuates"):
                        attenuating = True
                        inhibit = True
                    if inhibit is None:
                        self.note(t[2], 4, "regulate edge missing 'activates'/'inhibits'/'translates'; edge dropped")
                        self.skip_line(); continue
                    to = self.ident()
                    strength = 1.0
                    if self.expect_kw("strength"):
                        tt = self.peek()
                        if tt[0] in ("INT", "FLOAT"):
                            strength = float(tt[1]) if tt[0] == "FLOAT" else float(tt[1])
                            self.next()
                        else:
                            self.note(tt[2], 4, "strength needs a number; using 1.0")
                        # reg-bio-2 (D2c): strength clamps to the physical
                        # range (mirror of the Rust parser clamp + note).
                        if strength > 1.0:
                            self.note(tt[2], 4, "strength > 1.0 clamped to 1.0 (levels are concentration fractions)")
                            strength = 1.0
                        elif strength < 0.0:
                            self.note(tt[2], 4, "negative strength clamped to 0.0 (a negative repressor is not a booster)")
                            strength = 0.0
                    threshold = None
                    if self.expect_kw("threshold"):
                        tt = self.peek()
                        if tt[0] in ("INT", "FLOAT"):
                            threshold = float(tt[1])
                            self.next()
                        else:
                            self.note(tt[2], 4, "threshold needs a number; ignored")
                    # reg-bio (F-2): optional per-edge Hill exponent (1..=8)
                    hill = None
                    if self.expect_kw("hill"):
                        tt = self.peek()
                        if tt[0] == "INT":
                            iv = int(tt[1])
                            self.next()
                            if 1 <= iv <= 8:
                                hill = iv
                            else:
                                self.note(tt[2], 4, "hill needs an integer 1..=8; edge keeps the default n=2 shape")
                        else:
                            self.note(tt[2], 4, "hill needs an integer 1..=8; edge keeps the default n=2 shape")
                    # reg-bio (F-3): optional cis-regulatory OR membership
                    is_any = False
                    if self.expect_kw("any"):
                        is_any = True
                    # reg-bio-2 (D2b/B7): occupancy + pooling keywords
                    # (canonical order: strength -> threshold -> hill -> any -> occupy -> sum)
                    occupy = False
                    if self.expect_kw("occupy"):
                        occupy = True
                    is_sum = False
                    if self.expect_kw("sum"):
                        is_sum = True
                    if (hill is not None or is_any) and threshold is None:
                        self.note(tt[2], 4, "hill/any apply to thresholded edges; ignored (edge stays declarative)")
                        hill = None
                        is_any = False
                    if is_any and inhibit:
                        self.note(tt[2], 4, "any on an inhibiting edge is ignored (inhibitors already veto independently)")
                        is_any = False
                    if occupy and not inhibit:
                        self.note(tt[2], 4, "occupy applies to inhibiting edges; ignored (activators cannot occupy a promoter they activate)")
                        occupy = False
                    if is_sum and (inhibit or threshold is None):
                        self.note(tt[2], 4, "sum applies to thresholded activating edges; ignored")
                        is_sum = False
                    edges.append((frm, to, strength, inhibit, threshold, hill, is_any, occupy, is_sum, attenuating))
                    self.end_stmt()
            return ("regulate", edges, trans, binds)
        if word == "ligand":
            # reg-bio-2 (A4): `ligand iptg;` — a small-molecule pool
            self.next()
            name = self.ident()
            self.end_stmt()
            return ("ligand", name)
        if word == "autoinducer":
            # loop-9 (C8): `autoinducer ahl;` — register a quorum-sensing
            # signal species into the shared medium (parse mirror)
            self.next()
            name = self.ident()
            self.end_stmt()
            return ("autoinducer", name)
        if word == "decoy":
            # reg-bio-2 (C11): `decoy d for tf capacity 0.5;`
            self.next()
            d = self.ident()
            tf, cap = "", 0.0
            if self.expect_kw("for"):
                tf = self.ident()
                t2 = self.peek()
                if t2[0] == "IDENT" and t2[1] == "capacity":
                    self.next()
                    tt = self.peek()
                    if tt[0] in ("INT", "FLOAT"):
                        cap = min(max(float(tt[1]), 0.0), 1.0)
                        self.next()
                    else:
                        self.note(tt[2], 4, "capacity needs a number 0..=1; ignored")
                        cap = 0.0
                else:
                    self.note(t2[2], 4, "decoy needs 'capacity <num>'; declared inert")
            else:
                self.note(self.peek()[2], 4, "decoy needs 'for <tf> capacity <num>'; skipped")
            self.end_stmt()
            return ("decoy", d, tf, cap)
        if word == "operon":
            # reg-bio-3 (A1/A7): polycistronic transcription unit —
            # `operon lac { lacZ rbs 1.0; lacY rbs 0.6; lacA; }` (mirror)
            self.next()
            name = self.ident()
            members = []
            if self.peek() == ("SYM", "{", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t2 = self.peek()
                    if t2 == ("SYM", "}", t2[2]):
                        self.next()
                        break
                    if t2[0] == "EOF":
                        self.note(t2[2], 4, "operon block auto-closed")
                        break
                    if t2[0] == "IDENT":
                        self.next()
                        rbs = 1.0
                        t3 = self.peek()
                        if t3[0] == "IDENT" and t3[1] == "rbs":
                            self.next()
                            tt = self.peek()
                            if tt[0] in ("INT", "FLOAT"):
                                rbs = min(max(float(tt[1]), 0.0), 1.0)
                                self.next()
                            else:
                                self.note(tt[2], 4, "rbs needs a number 0..=1; using 1.0")
                        members.append((t2[1], rbs))
                        self.end_stmt()
                    else:
                        self.note(t2[2], 4, "unexpected token in operon block; skipped")
                        self.next()
            else:
                self.note(self.peek()[2], 4, "operon needs a block '{ cistron rbs r; ... }'; skipped")
            return ("operon", name, members)
        if word == "toggle":
            self.next()
            a = self.ident()
            b = a
            if self.peek() == ("SYM", ",", self.peek()[2]):
                self.next()
                b = self.ident()
            self.end_stmt()
            return ("toggle", a, b)
        if word == "repressilator":
            self.next()
            ring = [self.ident()]
            while self.peek() == ("SYM", "->", self.peek()[2]):
                self.next()
                ring.append(self.ident())
            period = None
            if self.expect_kw("period"):
                t = self.peek()
                if t[0] in ("INT", "FLOAT"):
                    period = float(t[1])
                    self.next()
            # reg-bio (F-5): inline kinetics — mirror of the Rust parser.
            # Canonical order: alpha, gamma, hill, basal, noise, seed.
            ov = {"alpha": None, "gamma": None, "hill": None, "basal": None,
                  "noise": None, "seed": None}
            while True:
                if self.expect_kw("alpha"):
                    t = self.peek()
                    if t[0] in ("INT", "FLOAT"):
                        self.next()
                        f = float(t[1])
                        if f > 0.0:
                            ov["alpha"] = f
                        else:
                            self.note(t[2], 4, "alpha needs a number > 0; ignored")
                    else:
                        self.note(t[2], 4, "alpha needs a number > 0; ignored")
                elif self.expect_kw("gamma"):
                    t = self.peek()
                    if t[0] in ("INT", "FLOAT"):
                        self.next()
                        f = float(t[1])
                        if f >= 0.0:
                            ov["gamma"] = f
                        else:
                            self.note(t[2], 4, "gamma needs a number >= 0; ignored")
                    else:
                        self.note(t[2], 4, "gamma needs a number >= 0; ignored")
                elif self.expect_kw("hill"):
                    t = self.peek()
                    if t[0] == "INT":
                        self.next()
                        iv = int(t[1])
                        if 1 <= iv <= 8:
                            ov["hill"] = iv
                        else:
                            self.note(t[2], 4, "hill needs an integer 1..=8; ignored")
                    else:
                        self.note(t[2], 4, "hill needs an integer 1..=8; ignored")
                elif self.expect_kw("basal"):
                    t = self.peek()
                    if t[0] in ("INT", "FLOAT"):
                        self.next()
                        f = float(t[1])
                        if f >= 0.0:
                            ov["basal"] = f
                        else:
                            self.note(t[2], 4, "basal needs a number >= 0; ignored")
                    else:
                        self.note(t[2], 4, "basal needs a number >= 0; ignored")
                elif self.expect_kw("noise"):
                    t = self.peek()
                    if t[0] in ("INT", "FLOAT"):
                        self.next()
                        f = float(t[1])
                        if 0.0 <= f <= 1.0:
                            ov["noise"] = f
                        else:
                            self.note(t[2], 4, "noise needs a number in 0..=1; ignored")
                    else:
                        self.note(t[2], 4, "noise needs a number in 0..=1; ignored")
                elif self.expect_kw("seed"):
                    t = self.peek()
                    if t[0] == "INT":
                        self.next()
                        iv = int(t[1])
                        ov["seed"] = 0x9E3779B97F4A7C15 if iv == 0 else (iv & M64)
                    else:
                        self.note(t[2], 4, "seed needs an integer; ignored")
                else:
                    break
            self.end_stmt()
            return ("repressilator", [r for r in ring if r], period, ov)
        if word == "frame":
            self.next()
            is_proof = self.expect_kw("proof")
            name = "proof" if is_proof else self.ident()
            return ("frame", name, is_proof, self.block())
        if word == "splice":
            self.next()
            root = self.ident()
            variants = []
            pending_marks = []  # T2c: marks before 'variant' apply to that variant
            if self.peek() == ("SYM", "{", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t = self.peek()
                    if t[0] == "SYM" and t[1] == "}":
                        self.next()
                        if pending_marks:
                            self.note(t[2], 4, "mark must precede 'variant' in splice block; skipped")
                            pending_marks = []
                        break
                    if t[0] == "EOF":
                        break
                    if t[0] == "MARK":
                        m = self.next()[1]
                        if m in MARKS:
                            pending_marks.append(m)
                        else:
                            wk = None
                            for k in MARKS:
                                if edit_distance(m, k) <= (1 if len(m) <= 4 else 2):
                                    self.note(t[2], 3, f"wobble: '@{m}' repaired to '@{k}'")
                                    wk = k
                                    break
                            if wk:
                                pending_marks.append(wk)
                            else:
                                self.note(t[2], 4, f"unknown mark '@{m}' skipped")
                        continue
                    if t[0] == "IDENT" and (t[1] == "variant" or edit_distance(t[1], "variant") <= 1):
                        if t[1] != "variant":
                            self.note(t[2], 3, f"wobble: '{t[1]}' repaired to 'variant'")
                        self.next()
                        vname = self.ident()
                        vmarks = pending_marks
                        pending_marks = []
                        vparams = []
                        if self.peek() == ("SYM", "(", self.peek()[2]):
                            self.next()
                            while True:
                                self.eat_nl()
                                t2 = self.peek()
                                if t2[0] == "SYM" and t2[1] == ")":
                                    self.next(); break
                                if t2[0] == "EOF":
                                    break
                                before = self.pos
                                pname = self.ident()
                                dflt = None
                                if self.peek() == ("SYM", "=", self.peek()[2]):
                                    self.next()
                                    dflt = self.expr()
                                vparams.append((pname, dflt))
                                if self.peek() == ("SYM", ",", self.peek()[2]):
                                    self.next()
                                if self.pos == before:
                                    self.note(t2[2], 4, "unclosed variant parameter list; auto-closed")
                                    break
                        variants.append((vname, vparams, self.block(), vmarks))
                    else:
                        if pending_marks:
                            self.note(t[2], 4, "mark must precede 'variant' in splice block; skipped")
                            pending_marks = []
                        self.note(t[2], 4, "unexpected token in splice block; skipped")
                        self.next()
            return ("splice", root, variants)
        if word == "edit":
            self.next()
            target = self.ident()
            reps = []
            if self.peek() == ("SYM", "{", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t = self.peek()
                    if t[0] == "SYM" and t[1] == "}":
                        self.next(); break
                    if t[0] == "EOF":
                        break
                    if self.expect_kw("replace"):
                        frm = ""
                        if self.peek()[0] in ("STR", "INTERP"):
                            frm = self.next()[1]
                        if self.peek() == ("SYM", "->", t[2]):
                            self.next()
                        to = ""
                        if self.peek()[0] in ("STR", "INTERP"):
                            to = self.next()[1]
                        reps.append((frm, to))
                        self.end_stmt()
                    else:
                        self.note(t[2], 4, "expected 'replace' in edit block; skipped")
                        self.next()
            return ("edit", target, reps)
        if word == "ires":
            self.next()
            name = self.ident()
            self.end_stmt()
            return ("ires", name)
        if word == "phenotype":
            self.next()
            name = self.ident()
            parent = None
            if self.at_ident("from"):
                self.next()
                parent = self.ident()
            fields, methods = [], []
            if self.peek() == ("SYM", "{", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t = self.peek()
                    if t[0] == "SYM" and t[1] == "}":
                        self.next(); break
                    if t[0] == "EOF":
                        self.note(t[2], 4, "phenotype body auto-closed")
                        break
                    if t[0] == "MARK":
                        self.next()
                        marks = [t[1]] if t[1] in MARKS else []
                        if self.expect_kw("gene"):
                            methods.append(self.gene_def(marks)[1])
                        else:
                            self.note(t[2], 4, "mark inside phenotype must precede 'gene'; skipped")
                            self.skip_line()
                        continue
                    if t[0] == "IDENT" and (t[1] == "gene" or SYNONYMS.get(t[1]) == "gene"):
                        if t[1] != "gene":
                            self.note(t[2], 2, f"synonym '{t[1]}' repaired to 'gene'")
                        self.next()
                        methods.append(self.gene_def([])[1])
                        continue
                    if t[0] == "IDENT" and (t[1] == "let" or SYNONYMS.get(t[1]) == "let"):
                        if t[1] != "let":
                            self.note(t[2], 2, f"synonym '{t[1]}' repaired to 'let'")
                        self.next()
                        fname = self.ident()
                        dflt = ("null",)
                        if self.peek() == ("SYM", "=", self.peek()[2]):
                            self.next()
                            dflt = self.expr()
                        self.end_stmt()
                        if fname != "?":
                            fields.append((fname, dflt))
                        continue
                    self.note(t[2], 4, "unexpected token in phenotype body; skipped")
                    before = self.pos
                    self.next()
                    if self.pos == before:
                        break
            return ("pheno", Pheno(name, parent, fields, methods))
        if word == "sequence":
            self.next()
            g = self.gene_def([])
            if g[0] == "gene":
                g[1].seq = True
            return ("expr", ("null",)) if g[0] != "gene" else ("gene", g[1])
        if word == "yield":
            self.next()
            t = self.peek()
            e = None
            if not (t[0] in ("NL", "EOF") or (t[0] == "SYM" and t[1] in (";", "}"))):
                e = self.expr()
            self.end_stmt()
            return ("yield", e)
        # bare-name definition: `main { ... }` / `route(req) { ... }` are
        # gene definitions — lookahead: NAME '{' or NAME '(' ... ')' '{'
        t1 = self.toks[self.pos + 1] if self.pos + 1 < len(self.toks) else ("EOF", None, 0)
        is_def = False
        if word not in KEYWORDS:
            if t1[0] == "SYM" and t1[1] == "{":
                is_def = True
            elif t1[0] == "SYM" and t1[1] == "(":
                j = self.pos + 1
                depth = 0
                n = len(self.toks)
                while j < n:
                    tk = self.toks[j]
                    if tk[0] == "SYM" and tk[1] == "(":
                        depth += 1
                    elif tk[0] == "SYM" and tk[1] == ")":
                        depth -= 1
                        if depth == 0:
                            j += 1
                            while j < n and self.toks[j][0] == "NL":
                                j += 1
                            is_def = j < n and self.toks[j][0] == "SYM" and self.toks[j][1] == "{"
                            break
                    elif tk[0] == "EOF":
                        break
                    j += 1
        if is_def:
            self.note(self.peek()[2], 4, f"bare name block '{word}' treated as gene definition")
            self.next()  # consume the name
            params = []
            if self.peek() == ("SYM", "(", self.peek()[2]):
                self.next()
                while True:
                    self.eat_nl()
                    t = self.peek()
                    if t[0] == "SYM" and t[1] == ")":
                        self.next(); break
                    if t[0] == "EOF":
                        self.note(t[2], 4, "parameter list auto-closed")
                        break
                    before = self.pos
                    pname = self.ident()
                    dflt = None
                    if self.peek() == ("SYM", "=", self.peek()[2]):
                        self.next()
                        dflt = self.expr()
                    params.append((pname, dflt))
                    if self.peek() == ("SYM", ",", self.peek()[2]):
                        self.next()
                    if self.pos == before:
                        self.note(t[2], 4, "unclosed parameter list; auto-closed")
                        break
            g = Gene(word, params, None, self.block())
            return ("gene", g)
        return self.assign_or_expr(w)

    def ident(self):
        t = self.peek()
        if t[0] == "IDENT":
            self.next()
            return t[1]
        self.note(t[2], 4, f"expected a name, found token; used '?'")
        return "?"

    def use_path(self):
        # segments joined by separators; boundary words ('as') never glue in
        parts = []
        while True:
            t = self.peek()
            if t[0] == "IDENT":
                if t[1] in ("as", "from"):
                    break
                parts.append(t[1]); self.next()
                nt = self.peek()
                if not (nt[0] == "SYM" and nt[1] in ("/", ".", "-")):
                    break
            elif t[0] == "SYM" and t[1] in ("/", ".", "-"):
                parts.append(t[1]); self.next()
            elif t[0] == "STR":
                parts.append(t[1]); self.next()
                nt = self.peek()
                if not (nt[0] == "SYM" and nt[1] in ("/", ".", "-")):
                    break
            else:
                break
        return "".join(parts)

    def assign_or_expr(self, w):
        t1 = self.toks[self.pos + 1] if self.pos + 1 < len(self.toks) else ("EOF", None, 0)
        if t1[0] == "SYM" and t1[1] == "=":
            self.next()  # name
            self.next()  # =
            v = self.expr()
            self.end_stmt()
            return ("assign", w, None, v)
        if t1[0] == "SYM" and t1[1] in ("+=", "-=", "*=", "/=", "%="):
            opmap = {"+=": "+", "-=": "-", "*=": "*", "/=": "/", "%=": "%"}
            self.next()  # name
            self.next()  # op=
            v = self.expr()
            self.end_stmt()
            return ("assign", w, opmap[t1[1]], v)
        # expression statement (possibly index/member assignment)
        e = self.expr()
        t = self.peek()
        # L1a: multiple assignment / swap — a, b = b, a
        if t[0] == "SYM" and t[1] == "," and e[0] in ("ident", "index", "member"):
            save = self.pos
            targets = [e]
            valid = True
            while self.peek() == ("SYM", ",", self.peek()[2]):
                self.next()
                te = self.expr()
                if te[0] not in ("ident", "index", "member"):
                    valid = False
                targets.append(te)
            if valid and self.peek() == ("SYM", "=", self.peek()[2]):
                self.next()
                values = [self.expr()]
                while self.peek() == ("SYM", ",", self.peek()[2]):
                    self.next()
                    values.append(self.expr())
                self.end_stmt()
                return ("multi", targets, values, False)
            self.pos = save
            self.note(self.peek()[2], 4, "expression statement not terminated; rest of line skipped")
            self.skip_line()
            self.end_stmt()
            return ("expr", e)
        if t[0] == "SYM" and t[1] == "=":
            self.next()
            v = self.expr()
            self.end_stmt()
            if e[0] == "index":
                return ("idx_assign", e[1], e[2], None, v)
            if e[0] == "member":
                return ("mem_assign", e[1], e[2], None, v)
            self.note(t[2], 4, "assignment target must be a name, index or member; value computed and dropped")
            return ("expr", e)
        if t[0] == "SYM" and t[1] in ("+=", "-=", "*=", "/=", "%="):
            opmap = {"+=": "+", "-=": "-", "*=": "*", "/=": "/", "%=": "%"}
            self.next()
            v = self.expr()
            self.end_stmt()
            if e[0] == "index":
                return ("idx_assign", e[1], e[2], opmap[t[1]], v)
            if e[0] == "member":
                return ("mem_assign", e[1], e[2], opmap[t[1]], v)
            if e[0] == "ident":
                return ("assign", e[1], opmap[t[1]], v)
            self.note(t[2], 4, "compound assignment target invalid; dropped")
            return ("expr", e)
        # same-line garbage check BEFORE terminator
        if not (t[0] in ("NL", "EOF") or (t[0] == "SYM" and t[1] in (";", "}"))):
            self.note(t[2], 4, "expression statement not terminated; rest of line skipped")
            self.skip_line()
        self.end_stmt()
        return ("expr", e)

    def gene_def(self, marks, copies=1, riboswitch=None, burst=None):
        ac = "acetylate" in marks
        me = "methylate" in marks
        m6 = "m6a" in marks
        # reg-bio-3 (C10): @copies dosage arrives via the mark-argument
        # parse (the clamps/notes mirror the Rust core op-for-op)
        name = None
        t = self.peek()
        if t[0] == "IDENT" and t[1] != "guard":
            name = t[1]
            self.next()
        params = []
        param_anns = []
        if self.peek() == ("SYM", "(", self.peek()[2]):
            self.next()
            while True:
                self.eat_nl()
                t = self.peek()
                if t[0] == "SYM" and t[1] == ")":
                    self.next(); break
                if t[0] == "EOF":
                    self.note(t[2], 4, "parameter list auto-closed")
                    break
                before = self.pos
                pname = self.ident()
                # W01 (L2c): parameter annotation — `gene f(x: int) { }`
                ann = None
                if self.peek() == ("SYM", ":", self.peek()[2]):
                    self.next()
                    ann = self.type_ann()
                dflt = None
                if self.peek() == ("SYM", "=", self.peek()[2]):
                    self.next()
                    dflt = self.expr()
                params.append((pname, dflt))
                param_anns.append(ann)
                if self.peek() == ("SYM", ",", self.peek()[2]):
                    self.next()
                if self.pos == before:
                    self.note(t[2], 4, "unclosed parameter list; auto-closed")
                    break
        # W01 (L2c): return annotation — `gene f(x) -> int { }`
        ret_ann = None
        if self.peek() == ("SYM", "->", self.peek()[2]):
            self.next()
            ret_ann = self.type_ann()
        guard = None
        self.eat_nl()
        if self.expect_kw("guard"):
            if self.peek() == ("SYM", "(", self.peek()[2]):
                self.next()
                cond = self.expr()
                if self.peek() == ("SYM", ")", self.peek()[2]):
                    self.next()
                else:
                    self.note(self.peek()[2], 4, "guard condition auto-closed")
                self.expect_kw("else")
                gbody = self.block()
                guard = (cond, gbody)
            else:
                self.note(self.peek()[2], 4, "guard without condition ignored")
        self.eat_nl()
        if self.peek() == ("SYM", "=>", self.peek()[2]):
            self.next()
            e = self.expr()
            self.end_stmt()
            return ("gene", Gene(name, params, guard, [("return", e)], ac, me, m6, copies, seq=False, riboswitch=riboswitch, burst=burst, param_anns=param_anns, ret_ann=ret_ann))
        body = self.block()
        return ("gene", Gene(name, params, guard, body, ac, me, m6, copies, seq=False, riboswitch=riboswitch, burst=burst, param_anns=param_anns, ret_ann=ret_ann))

    def block(self):
        if not (self.peek() == ("SYM", "{", self.peek()[2])):
            self.note(self.peek()[2], 4, "block without braces; single statement accepted")
            s = self.stmt()
            return [s] if s else []
        self.next()
        out = []
        while True:
            self.eat_nl()
            t = self.peek()
            if t[0] == "SYM" and t[1] == "}":
                self.next(); break
            if t[0] == "EOF":
                self.note(t[2], 4, "block auto-closed at end of file")
                break
            before = self.pos
            s = self.stmt()
            if s is not None:
                out.append(s)
            if self.pos == before:
                self.note(t[2], 4, "token skipped")
                self.next()
        return out

    # L1a: destructuring pattern parser (mirrors src/parser.rs parse_destructure_pat)
    def destructure_pat(self):
        t = self.peek()
        if t[0] == "SYM" and t[1] == "[":
            self.next()
            elems, rest = [], None
            while True:
                self.eat_nl()
                t2 = self.peek()
                if t2 == ("SYM", "]", t2[2]):
                    self.next(); break
                if t2[0] == "EOF":
                    self.note(t2[2], 4, "pattern bracket auto-closed")
                    break
                if t2 == ("SYM", "*", t2[2]):
                    self.next()
                    t3 = self.peek()
                    if t3[0] == "IDENT":
                        self.next()
                        rest = t3[1]
                    else:
                        self.note(t3[2], 4, "'*' in pattern needs a name; tail skipped")
                elif t2 == ("SYM", ",", t2[2]):
                    self.next()
                else:
                    elems.append(self.destructure_pat())
            return ("plist", elems, rest)
        if t[0] == "SYM" and t[1] == "{":
            self.next()
            keys = []
            while True:
                self.eat_nl()
                t2 = self.peek()
                if t2 == ("SYM", "}", t2[2]):
                    self.next(); break
                if t2[0] == "EOF":
                    self.note(t2[2], 4, "pattern brace auto-closed")
                    break
                if t2 == ("SYM", ",", t2[2]):
                    self.next()
                elif t2[0] == "IDENT":
                    self.next()
                    keys.append(t2[1])
                else:
                    self.note(t2[2], 4, f"'{t2[1]}' is not a map-pattern key; skipped")
                    self.next()
            return ("pmap", keys)
        t2 = self.peek()
        if t2[0] == "IDENT":
            self.next()
            return ("pbind", t2[1])
        self.note(t2[2], 4, f"'{t2[1]}' cannot start a pattern; binding null")
        self.next()
        return ("pbind", "_")

    # expressions
    def expr(self):
        cond = self.or_expr()
        t = self.peek()
        if t[0] == "SYM" and t[1] == "?":
            self.next()
            a = self.expr()
            t2 = self.peek()
            if t2[0] == "SYM" and t2[1] == ":":
                self.next()
            else:
                self.note(t[2], 4, "ternary missing ':'; else-branch is null")
            b = self.expr()
            return ("tern", cond, a, b)
        return cond

    def or_expr(self):
        left = self.nullish_expr()
        while True:
            t = self.peek()
            if (t[0] == "IDENT" and t[1] == "or") or (t[0] == "SYM" and t[1] == "||"):
                self.next()
                left = ("bin", "or", left, self.nullish_expr(), self.peek()[2])  # W07: line = right-operand start (Rust stamps after next())
            else:
                return left

    def nullish_expr(self):
        # L1a: a ?? b — sits between or and and (mirrors src/parser.rs)
        left = self.and_expr()
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] == "??":
                self.next()
                left = ("bin", "nullish", left, self.and_expr(), t[2])  # W07: op line
            else:
                return left

    def and_expr(self):
        left = self.not_expr()
        while True:
            t = self.peek()
            if (t[0] == "IDENT" and t[1] == "and") or (t[0] == "SYM" and t[1] == "&&"):
                self.next()
                left = ("bin", "and", left, self.not_expr(), self.peek()[2])  # W07: line = right-operand start
            else:
                return left

    def not_expr(self):
        t = self.peek()
        if (t[0] == "IDENT" and t[1] == "not") or (t[0] == "SYM" and t[1] == "!"):
            self.next()
            return ("un", "not", self.not_expr())
        return self.cmp_expr()

    def cmp_expr(self):
        left = self.bitor_expr()
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] in ("==", "!=", "<", "<=", ">", ">="):
                self.next()
                left = ("bin", t[1], left, self.bitor_expr(), t[2])
            elif t[0] == "IDENT" and t[1] == "in":
                self.next()
                left = ("bin", "in", left, self.bitor_expr(), t[2])
            else:
                return left

    def bitor_expr(self):
        left = self.bitxor_expr()
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] == "|":
                self.next()
                left = ("bin", "|", left, self.bitxor_expr(), t[2])
            else:
                return left

    def bitxor_expr(self):
        left = self.bitand_expr()
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] == "^":
                self.next()
                left = ("bin", "^", left, self.bitand_expr(), t[2])
            else:
                return left

    def bitand_expr(self):
        left = self.shift_expr()
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] == "&":
                self.next()
                left = ("bin", "&", left, self.shift_expr(), t[2])
            else:
                return left

    def shift_expr(self):
        left = self.add_expr()
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] in ("<<", ">>"):
                self.next()
                left = ("bin", t[1], left, self.add_expr(), t[2])
            else:
                return left

    def add_expr(self):
        left = self.mul_expr()
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] in ("+", "-"):
                self.next()
                left = ("bin", t[1], left, self.mul_expr(), t[2])
            else:
                return left

    def mul_expr(self):
        left = self.unary()
        while True:
            t = self.peek()
            if t[0] == "SYM" and t[1] in ("*", "/", "//", "%"):
                self.next()
                left = ("bin", t[1], left, self.unary(), t[2])
            else:
                return left

    def unary(self):
        if self.peek() == ("SYM", "-", self.peek()[2]):
            self.next()
            return ("un", "neg", self.unary())
        if self.peek() == ("SYM", "~", self.peek()[2]):
            self.next()
            return ("un", "bitnot", self.unary())
        return self.pow_expr()

    def pow_expr(self):
        left = self.postfix()
        t = self.peek()
        if t[0] == "SYM" and t[1] == "**":
            self.next()
            # right-assoc; right operand re-enters unary so 2**-3 parses
            return ("bin", "**", left, self.unary(), t[2])
        return left

    def postfix(self):
        e = self.primary()
        while True:
            t = self.peek()
            if t == ("SYM", "(", t[2]):
                self.next()
                args = []
                while True:
                    self.eat_nl()
                    t2 = self.peek()
                    if t2 == ("SYM", ")", t2[2]):
                        self.next(); break
                    if t2[0] == "EOF":
                        self.note(t2[2], 4, "call arguments auto-closed")
                        break
                    args.append(self.expr())
                    if self.peek() == ("SYM", ",", t2[2]):
                        self.next()
                e = ("call", e, args, t[2])  # W07: LParen line (Rust stamps before next())
            elif t == ("SYM", "[", t[2]):
                self.next()
                idx = self.expr()
                if self.peek() == ("SYM", "]", self.peek()[2]):
                    self.next()
                else:
                    self.note(self.peek()[2], 4, "index bracket auto-closed")
                e = ("index", e, idx, t[2])  # W07: LBrack line
            elif t == ("SYM", ".", t[2]):
                self.next()
                t2 = self.peek()
                if t2[0] == "IDENT":
                    self.next()
                    if self.peek() == ("SYM", "(", t2[2]):
                        self.next()
                        args = []
                        while True:
                            self.eat_nl()
                            t3 = self.peek()
                            if t3 == ("SYM", ")", t3[2]):
                                self.next(); break
                            if t3[0] == "EOF":
                                break
                            args.append(self.expr())
                            if self.peek() == ("SYM", ",", t3[2]):
                                self.next()
                        e = ("method", e, t2[1], args)
                    else:
                        e = ("member", e, t2[1])
                else:
                    self.note(t2[2], 4, "'.' followed by non-name; member skipped")
                    break
            elif t == ("SYM", "?!", t[2]):
                # W06 (D-014): `e?!` — Option/Result propagation, a postfix
                # operator (binds tighter than every binary op, repeats:
                # Some(Some(3))?!?! unwraps twice). Mirrors src/parser.rs
                # Tok::QuestionBang postfix arm.
                self.next()
                e = ("prop", e, t[2])
            elif t == ("SYM", "?.", t[2]):
                # L1a: optional chaining (mirrors src/parser.rs QuestionDot)
                self.next()
                t2 = self.peek()
                if t2[0] == "IDENT":
                    self.next()
                    if self.peek() == ("SYM", "(", t2[2]):
                        self.next()
                        args = []
                        while True:
                            self.eat_nl()
                            t3 = self.peek()
                            if t3 == ("SYM", ")", t3[2]):
                                self.next(); break
                            if t3[0] == "EOF":
                                break
                            args.append(self.expr())
                            if self.peek() == ("SYM", ",", t3[2]):
                                self.next()
                        e = ("method?", e, t2[1], args)
                    else:
                        e = ("member?", e, t2[1])
                else:
                    self.note(t2[2], 4, "'?.' followed by non-name; chain resolves to null")
                    e = ("member?", e, "")
                    break
            else:
                return e

    def primary(self):
        t = self.next()
        kind, val, line = t
        if kind == "INT":
            return ("lit", val)
        if kind == "FLOAT":
            return ("lit", val)
        if kind == "STR":
            return ("lit", val)
        if kind == "INTERP":
            return self.build_interp(val, line)
        if t == ("SYM", "(", line):
            e = self.expr()
            if self.peek() == ("SYM", ")", self.peek()[2]):
                self.next()
            else:
                self.note(self.peek()[2], 4, "parenthesis auto-closed")
            return e
        if t == ("SYM", "[", line):
            items = []
            while True:
                self.eat_nl()
                t2 = self.peek()
                if t2 == ("SYM", "]", t2[2]):
                    self.next(); break
                if t2[0] == "EOF":
                    self.note(t2[2], 4, "list auto-closed at end of file")
                    break
                items.append(self.expr())
                if self.peek() == ("SYM", ",", t2[2]):
                    self.next()
                elif not (self.peek() == ("SYM", "]", self.peek()[2])):
                    self.note(self.peek()[2], 4, "list items separated automatically")
            return ("list", items)
        if t == ("SYM", "{", line):
            pairs = []
            while True:
                self.eat_nl()
                t2 = self.peek()
                if t2 == ("SYM", "}", t2[2]):
                    self.next(); break
                if t2[0] == "EOF":
                    self.note(t2[2], 4, "map literal auto-closed at end of file")
                    break
                if t2[0] == "IDENT":
                    self.next()
                    key = ("lit", t2[1])
                elif t2[0] == "STR":
                    self.next()
                    key = ("lit", t2[1])
                elif t2[0] == "INT":
                    self.next()
                    key = ("lit", t2[1])
                else:
                    self.note(t2[2], 4, "map key must be a name or string; key null")
                    self.next()
                    key = ("null",)
                if self.peek() == ("SYM", ":", self.peek()[2]):
                    self.next()
                else:
                    self.note(self.peek()[2], 4, "map entry missing ':'; value null")
                val = self.expr()
                pairs.append((key, val))
                if self.peek() == ("SYM", ",", self.peek()[2]):
                    self.next()
            return ("map", pairs)
        if kind == "IDENT":
            if val in ("true", "false"):
                if val not in ("true", "false"):
                    self.note(line, 2, f"synonym '{val}' repaired")
                return ("lit", val == "true")
            if val in VALUE_SYNONYMS:
                self.note(line, 2, f"synonym '{val}' repaired to '{'null' if VALUE_SYNONYMS[val] is None else str(VALUE_SYNONYMS[val]).lower()}'")
                v = VALUE_SYNONYMS[val]
                return ("lit", v)
            if val == "null":
                return ("null",)
            if val in ("gene", "fn", "func", "def", "lambda"):
                if val != "gene":
                    self.note(line, 2, f"synonym '{val}' repaired to 'gene'")
                s = self.gene_def([])
                return ("lambda", s[1])
            if val == "new":
                cname = self.ident()
                cargs = []
                if self.peek() == ("SYM", "(", self.peek()[2]):
                    self.next()
                    while True:
                        self.eat_nl()
                        t2 = self.peek()
                        if t2[0] == "SYM" and t2[1] == ")":
                            self.next(); break
                        if t2[0] == "EOF":
                            break
                        before = self.pos
                        cargs.append(self.expr())
                        if self.peek() == ("SYM", ",", self.peek()[2]):
                            self.next()
                        if self.pos == before:
                            self.note(t2[2], 4, "unclosed constructor argument list; auto-closed")
                            break
                return ("new", cname, cargs)
            if val == "for":
                var = self.ident()
                self.expect_kw("in")
                it = self.expr()
                filt = None
                if self.at_ident("if"):
                    self.next()
                    filt = self.expr()
                self.expect_kw("collect")
                body = self.expr()
                return ("collect", var, it, filt, body)
            return ("ident", val)
        self.note(line, 4, f"unexpected token in expression; null substituted")
        return ("null",)

    def build_interp(self, raw, line):
        parts = []
        lit = []
        i, n = 0, len(raw)
        while i < n:
            if raw[i] == "{":
                if lit:
                    parts.append(("lit", "".join(lit)))
                    lit = []
                depth, j = 1, i + 1
                expr_txt = []
                while j < n and depth > 0:
                    if raw[j] == "{":
                        depth += 1
                    elif raw[j] == "}":
                        depth -= 1
                        if depth == 0:
                            break
                    expr_txt.append(raw[j])
                    j += 1
                i = j + 1
                sub_toks, sub_notes = lex("".join(expr_txt))
                sub = P(sub_toks, [])
                e = sub.expr()
                for nt in sub.notes:
                    self.note(line, nt.rung, nt.message)
                for nt in sub_notes:
                    self.note(line, nt.rung, nt.message)
                parts.append(("expr", e))
            else:
                lit.append(raw[i])
                i += 1
        if lit:
            parts.append(("lit", "".join(lit)))
        return ("interp", parts)

    # W01 (L2c): type-annotation grammar — `name`, `name?` (optional),
    # `a | b` (union). Malformed annotation degrades to any (Total Grammar).
    def type_ann(self):
        first = self.type_ann_atom()
        t = self.peek()
        if t[0] == "SYM" and t[1] == "|":
            alts = [first]
            while t[0] == "SYM" and t[1] == "|":
                self.next()
                alts.append(self.type_ann_atom())
                t = self.peek()
            return ("union", alts)
        return first

    def type_ann_atom(self):
        t = self.peek()
        if t[0] == "IDENT":
            self.next()
            t2 = self.peek()
            if t2[0] == "SYM" and t2[1] == "?":
                self.next()
                return ("opt", ("named", t[1]))
            return ("named", t[1])
        self.note(t[2], 4, f"'{t[1]}' is not a type name; annotation treated as any")
        self.next()
        return ("named", "any")

    def pattern(self):
        # W02 (match-v2) mirror of parser.rs parse_pattern: atom | or-chain |
        # guard, with the legacy literal comma-run kept verbatim. Total
        # Grammar: a malformed pattern degrades to wildcard/bind + note.
        first = self.pattern_atom()
        # Legacy literal comma-run — only for a leading literal; a
        # non-literal in the run discards the collected literals (legacy
        # edge behavior preserved op-for-op with the Rust core).
        if first[0] == "lit":
            t = self.peek()
            if t[0] == "SYM" and t[1] == ",":
                lits = [first]
                while True:
                    t = self.peek()
                    if not (t[0] == "SYM" and t[1] == ","):
                        break
                    self.next()
                    t2 = self.peek()
                    neg2 = t2[0] == "SYM" and t2[1] == "-"
                    if neg2:
                        self.next()
                        t2 = self.peek()
                    if t2[0] in ("INT", "FLOAT"):
                        self.next()
                        lits.append(("lit", -t2[1] if neg2 else t2[1]))
                    elif t2[0] == "STR" and not neg2:
                        self.next()
                        lits.append(("lit", t2[1]))
                    else:
                        return self.pattern_atom()
                return lits[0] if len(lits) == 1 else ("multi", lits)
        # Or-pattern chain: `p1 | p2 | ...` — newlines allowed before any
        # alternative (multi-line chains), mirror of parser.rs.
        while self.peek()[0] == "NL":
            self.next()
        t = self.peek()
        if t[0] == "SYM" and t[1] == "|":
            alts = [first]
            while True:
                while self.peek()[0] == "NL":
                    self.next()
                t = self.peek()
                if not (t[0] == "SYM" and t[1] == "|"):
                    break
                self.next()
                alts.append(self.pattern_atom())
                while self.peek()[0] == "NL":
                    self.next()
            pat = ("or", alts)
        else:
            pat = first
        # Guarded arm: `pat if cond` — applies to the WHOLE or-chain.
        if self.at_ident("if"):
            self.next()
            cond = self.expr()
            return ("guard", pat, cond)
        return pat

    def pattern_atom(self):
        t = self.peek()
        neg = t[0] == "SYM" and t[1] == "-"
        if neg:
            self.next()
            t = self.peek()
        if t[0] in ("INT", "FLOAT"):
            self.next()
            return ("lit", -t[1] if neg else t[1])
        if t[0] == "STR":
            # W02 parity fix: '-' before a string pattern is ignored with a
            # note (was: oracle said "dangling '-'" and went wildcard while
            # the Rust core kept the literal — latent corner divergence,
            # closed by mirroring the Rust behavior).
            if neg:
                self.note(t[2], 4, "'-' before a string pattern ignored")
            self.next()
            return ("lit", t[1])
        if t[0] == "SYM" and t[1] == "[" and not neg:
            self.next()
            elems = []
            rest = None
            while True:
                while self.peek()[0] == "NL":
                    self.next()
                t2 = self.peek()
                if t2[0] == "SYM" and t2[1] == "]":
                    self.next()
                    break
                if t2[0] == "EOF":
                    self.note(t2[2], 4, "pattern bracket auto-closed")
                    break
                if t2[0] == "SYM" and t2[1] == "*":
                    self.next()
                    t3 = self.peek()
                    if t3[0] == "IDENT":
                        self.next()
                        rest = t3[1]
                    else:
                        self.note(t3[2], 4, "'*' in pattern needs a name; tail skipped")
                elif t2[0] == "SYM" and t2[1] == ",":
                    self.next()
                else:
                    elems.append(self.pattern_atom())
            return ("list", elems, rest)
        if t[0] == "SYM" and t[1] == "{" and not neg:
            self.next()
            keys = []
            while True:
                while self.peek()[0] == "NL":
                    self.next()
                t2 = self.peek()
                if t2[0] == "SYM" and t2[1] == "}":
                    self.next()
                    break
                if t2[0] == "EOF":
                    self.note(t2[2], 4, "pattern brace auto-closed")
                    break
                if t2[0] == "SYM" and t2[1] == ",":
                    self.next()
                elif t2[0] == "IDENT":
                    self.next()
                    sub = None
                    t3 = self.peek()
                    if t3[0] == "SYM" and t3[1] == ":":
                        self.next()
                        sub = self.pattern_atom()
                    keys.append((t2[1], sub))
                else:
                    self.note(t2[2], 4, f"'{t2[1]}' is not a map-pattern key; skipped")
                    self.next()
            return ("map", keys)
        if t[0] == "IDENT" and not neg:
            if t[1] == "_":
                self.next()
                return ("wild",)
            if t[1] in ("true", "false"):
                self.next()
                return ("lit", t[1] == "true")
            if t[1] == "null":
                self.next()
                return ("null",)
            if t[1] in ("Some", "None", "Ok", "Err"):
                self.next()
                payload = None
                t2 = self.peek()
                if t2[0] == "SYM" and t2[1] == "(":
                    self.next()
                    line = t2[2]
                    t3 = self.peek()
                    if t3[0] == "SYM" and t3[1] == ")":
                        self.next()
                        self.note(line, 4, "empty variant payload pattern — treated as tag-only")
                    else:
                        payload = self.pattern_atom()
                        t3 = self.peek()
                        if t3[0] == "SYM" and t3[1] == ",":
                            self.note(
                                line, 4,
                                "variant payload is a single value; extra pattern elements ignored",
                            )
                            while not (
                                (self.peek()[0] == "SYM" and self.peek()[1] == ")")
                                or self.peek()[0] == "EOF"
                            ):
                                self.next()
                        t3 = self.peek()
                        if t3[0] == "SYM" and t3[1] == ")":
                            self.next()
                        else:
                            self.note(self.peek()[2], 4, "variant payload pattern auto-closed")
                return ("variant", t[1], payload)
            if t[1][:1].isupper():
                # Unknown capitalized tag: soft fallback to a binding. A
                # parenthesized payload is consumed and ignored so the token
                # stream stays aligned (mirror of parser.rs).
                self.note(t[2], 4, f"unknown variant tag '{t[1]}' — pattern treated as a binding")
                self.next()
                t2 = self.peek()
                if t2[0] == "SYM" and t2[1] == "(":
                    self.note(t[2], 4, "unknown tag payload ignored (bound as a whole)")
                    self.next()
                    if not (self.peek()[0] == "SYM" and self.peek()[1] == ")"):
                        self.pattern_atom()
                        t3 = self.peek()
                        if t3[0] == "SYM" and t3[1] == ",":
                            while not (
                                (self.peek()[0] == "SYM" and self.peek()[1] == ")")
                                or self.peek()[0] == "EOF"
                            ):
                                self.next()
                    if self.peek()[0] == "SYM" and self.peek()[1] == ")":
                        self.next()
                    else:
                        self.note(self.peek()[2], 4, "unknown tag payload auto-closed")
                return ("bind", t[1])
            self.next()
            return ("bind", t[1])
        if neg:
            self.note(t[2], 4, "dangling '-' in pattern treated as wildcard")
            self.next()
            return ("wild",)
        self.note(t[2], 4, "pattern treated as wildcard")
        self.next()
        return ("wild",)

def parse(src):
    toks, notes = lex(src)
    p = P(toks, notes)
    stmts = p.program()
    return stmts, p.notes

# ----------------------------------------------------------------------------
# evaluator

BUILTINS = set("""promote expr_on expr_off decay_clock ligand_set ligand secrete quorum quench quorum_state splice_shift promoter_telemetry burst_set len push pop insert remove keys values has del range str num type
abs min max sum clock exit assert codon distance similar transcribe reverse_complement
gc_content translate find_orf memory methyl methylate demethylate m6a_write m6a_erase passage grn_set grn_get fingerprint toggle_on toggle_state repressi_next
repressi_state repressi_start grn_fire grn_state spawn join floor ceil sqrt pow random
randomize chr ord now sleep argv read_file write_file append_file exists file_size read_dir run
fs_delete fs_rename fs_mkdir re_replace
http_get serve recv_request send_response json_parse json_str env call items py
re_match re_find re_groups unix_time date_parts date_fmt
some none ok err is_some is_none is_ok is_err unwrap unwrap_or
enumerate zip sorted reversed any all first last take drop unique flatten chunk round clamp divmod""".split())

BUILTIN_SYNONYMS = {"print": "promote", "echo": "promote", "say": "promote", "show": "promote"}

class Interp:
    def __init__(self, cell=None, cli_args=None):
        self.notes = []
        self.cell = cell or {}
        # W069: importing-entry directory — candidate root #1 for `use`
        # (SPEC §8 resolution table). None until load_file sets it.
        self.base_dir = None
        self.silences = []
        # reg-bio-3: operons / stoichiometric-RISC bookkeeping / m6A levels /
        # generation counter / gene dosage registry (mirror of the Rust core)
        self.operons = []
        # loop-10 (F-7/F-8): Rho/queue state — rho_pins is the worker-side
        # resolved knob tuple (None on the host: .cell resolved per use,
        # mirror of the Rust rho_knobs); ribo_queue is the per-cistron
        # queue register (rho.termination-gated bookkeeping).
        self.rho_pins = None
        self.ribo_queue = {}
        self.risc_escaped = set()
        self.m6a_levels = {}
        self.generation = 0
        self.copies = {}
        self.fates = {}
        self.phenos = {}
        self.grn_edges = []
        self.grn_levels = {}
        self.toggles = []
        self.repressi_ring = []
        self.repressi_i = 0
        self.repressi_tick = 0
        self.ires = []
        self.enhanced = []
        self.defined_genes = []
        self.call_counts = {}
        self.call_clock = 0
        self.gene_buckets = {}
        self.modules = {}
        self.loading = []
        self.globals = {"__parent__": None}
        self.cli_args = cli_args or []
        self.steps = 0
        self.depth = 0
        self.depth_limit = 10_000
        # W07 mirror: line of the call/binop/index expression currently
        # executing (A13/dx-r4 parity) — feeds traceback chain frames.
        self.cur_line = 0
        self.methyl_quiet = False
        self.methyl_noted = set()
        # T2b graded methylation: per-gene silencing level (@methylate defs +1,
        # @acetylate defs -1). Calls are blocked at methyl_threshold (default 3).
        self.methyl_levels = {}
        self.methyl_threshold = 3
        self.asserts_run = 0
        self.rng = 0x9E3779B97F4A7C15
        # reg-bio (F-6): enhancer dose (default 0.25)
        self.enhance_delta = 0.25
        # reg-bio (F-1): telegraph promoter layer (opt-in via .cell)
        self.expr_stochastic = False
        self.expr_kon = 0.3
        self.expr_koff = 0.1
        # reg-bio-2 (C2): decay-clock runtime override (decay_clock builtin)
        self.decay_clock_n = None
        self.decay_clock_f = None
        self.promoter_states = {}
        self.burst_off = {}
        # reg-bio (F-5): repressilator kinetics (defaults = historical constants)
        self.repressi_params = dict(DEFAULT_REPRESSI)
        # reg-bio-2 (C1/C11): translation layer + decoy sites
        self.trans_edges = []
        self.trans_last = {}
        self.decoys = []
        # reg-bio-2 (A4): ligand pools + allosteric bindings
        self.ligands = []
        self.ligand_pools = {}
        # loop-9 (C8): quorum-sensing signal species + the shared medium
        # (species -> integer molecule count). The oracle is sequential, so
        # one dict IS the shared medium; inline spawn shares it naturally.
        self.signals = []
        self.medium = {}
        # loop-9 (F-4): runtime splice shifts (root -> variant name)
        self.splice_shift = {}
        self.splice_registry = {}
        # loop-9 (F-3): per-gene promoter attempt telemetry (mirror)
        self.promoter_tel = {}
        # loop-9 (R9): runtime promoter-rate modulation (mirror)
        self.burst_overrides = {}
        self.grn_binds = []
        self.seq_buffer = None
        self.caps = {"enabled": True, "read": [], "write": [], "run": [], "net": [], "env": [], "py": []}
        self.cell_entry = None
        self.proof_mode = False

    # ---- capabilities (mirror of Rust Caps)
    @staticmethod
    def _norm_path(p):
        out = []
        for seg in p.split("/"):
            if seg in ("", "."):
                continue
            if seg == "..":
                if out:
                    out.pop()
            else:
                out.append(seg)
        return "/".join(out)

    @staticmethod
    def _resolved(p):
        try:
            return os.path.realpath(p)
        except OSError:
            return p

    def _path_allowed(self, cap, what):
        # resolved-prefix comparison (symlink-aware, like the Rust core)
        rw = self._resolved(what)
        for g in self.caps[cap]:
            rg = self._resolved(g)
            if rw == rg or rw.startswith(rg + os.sep):
                return True
            # non-existent target: nearest existing ancestor must resolve inside
            probe = what
            parent = os.path.dirname(probe)
            if parent and parent != probe:
                rp = self._resolved(parent)
                if rp == rg or rp.startswith(rg + os.sep):
                    return True
        # lexical fallback for grants that do not exist on disk
        np = self._norm_path(what)
        for g in self.caps[cap]:
            ng = self._norm_path(g)
            if ng != "" and (np == ng or np.startswith(ng + "/")):
                return True
        return False

    def cap_check(self, cap, kind, what):
        c = self.caps
        if not c["enabled"] or "*" in c[cap]:
            return
        ok = False
        if kind in ("read", "write"):
            ok = self._path_allowed(cap, what)
        else:
            ok = what in c[cap]
        if not ok:
            raise Stress("interference", f"{kind} denied — no capability grant covers '{what}' (grant with --allow-{kind} or --allow-all)")

    def new_scope(self, parent):
        return {"__parent__": parent}

    def lookup(self, env, name):
        node = env
        while node is not None:
            if name in node:
                return node[name]
            node = node["__parent__"]
        return None

    def find_env(self, env, name):
        node = env
        while node is not None:
            if name in node:
                return node
            node = node["__parent__"]
        return None

    def assign(self, env, name, val):
        target = self.find_env(env, name)
        if target is None:
            env[name] = val
            return False
        target[name] = val
        return True

    def note(self, rung, msg):
        # loop-9 (C8): worker-note prefix parity — the Rust join path tags
        # every spawned worker note with "[task <name>] " (genes.rs); the
        # sequential oracle applies the same prefix while the spawn body
        # runs inline, so cross-cell notes are byte-identical.
        prefix = getattr(self, "task_note_prefix", None)
        if prefix:
            msg = f"{prefix} {msg}"
        self.notes.append(Note(rung, msg))

    def tick(self):
        self.steps += 1
        if self.steps > 20_000_000:
            raise Stress("overflow", "step budget exhausted")

    # ---- statements
    def bind_pattern(self, env, pat, v):
        # L1a: destructuring binder — soft-miss semantics (Total Grammar)
        k = pat[0]
        if k == "pbind":
            name = pat[1]
            if name in env:
                self.note(4, f"rebinding '{name}'")
            env[name] = v
        elif k == "plist":
            elems, rest = pat[1], pat[2]
            if isinstance(v, list):
                n = len(v)
                for i, ep in enumerate(elems):
                    if i < n:
                        self.bind_pattern(env, ep, v[i])
                    else:
                        self.note(4, f"destructure: index {i} missing; null")
                        self.bind_pattern(env, ep, None)
                if rest is not None:
                    tail = list(v[len(elems):]) if len(elems) < n else []
                    if rest in env:
                        self.note(4, f"rebinding '{rest}'")
                    env[rest] = tail
                elif n > len(elems):
                    self.note(4, f"destructure: {n - len(elems)} extra element(s) dropped")
            elif isinstance(v, str):
                # list patterns destructure strings by char (like for-in)
                chars = list(v)
                n = len(chars)
                for i, ep in enumerate(elems):
                    if i < n:
                        self.bind_pattern(env, ep, chars[i])
                    else:
                        self.note(4, f"destructure: index {i} missing; null")
                        self.bind_pattern(env, ep, None)
                if rest is not None:
                    tail = chars[len(elems):] if len(elems) < n else []
                    if rest in env:
                        self.note(4, f"rebinding '{rest}'")
                    env[rest] = tail
                elif n > len(elems):
                    self.note(4, f"destructure: {n - len(elems)} extra element(s) dropped")
            elif v is None:
                self.note(4, "destructure of null binds nulls")
                for ep in elems:
                    self.bind_pattern(env, ep, None)
                if rest is not None:
                    if rest in env:
                        self.note(4, f"rebinding '{rest}'")
                    env[rest] = []
            else:
                self.note(4, f"cannot destructure {type_name(v)}; pattern binds nulls")
                for ep in elems:
                    self.bind_pattern(env, ep, None)
                if rest is not None:
                    if rest in env:
                        self.note(4, f"rebinding '{rest}'")
                    env[rest] = []
        elif k == "pmap":
            keys = pat[1]
            if isinstance(v, dict):
                for key in keys:
                    if key in v:
                        if key in env:
                            self.note(4, f"rebinding '{key}'")
                        env[key] = v[key]
                    else:
                        self.note(4, f"destructure: key '{key}' missing; null")
                        if key in env:
                            self.note(4, f"rebinding '{key}'")
                        env[key] = None
            else:
                self.note(4, f"cannot destructure {type_name(v)}; pattern binds nulls")
                for key in keys:
                    if key in env:
                        self.note(4, f"rebinding '{key}'")
                    env[key] = None

    # ------------------------------------------------------------- W02 match-v2
    def eval_pat_val(self, env, l):
        """Mirror of the Rust pattern-literal eval: `?!` propagation is a
        return (never contained); any other stress becomes a note + null."""
        try:
            return self.eval(env, l)
        except Stress as st:
            if st.prop is not None:
                raise
            self.note(4, f"pattern evaluation contained: [{st.kind}] {st.message}")
            return None

    def match_pat(self, env, bindings, sv, pat):
        """W02 (match-v2) recursive pattern matcher — op-for-op mirror of
        interp.rs match_pat. Total Grammar: a pattern never hard-fails; a
        non-matching shape simply misses; only `?!` propagation escapes."""
        k = pat[0]
        if k == "wild":
            return True
        if k == "bind":
            bindings[pat[1]] = sv
            return True
        if k == "lit":
            # literal evals in the arm scope so same-arm bindings are visible
            gev = self.new_scope(env)
            gev.update(bindings)
            return deep_eq(sv, self.eval_pat_val(gev, pat))
        if k == "null":
            return deep_eq(sv, None)
        if k == "multi":
            # legacy literal comma-run — any literal hits (first wins)
            gev = self.new_scope(env)
            gev.update(bindings)
            for lt in pat[1]:
                if deep_eq(sv, self.eval_pat_val(gev, lt)):
                    return True
            return False
        if k == "or":
            # alternatives in order, first hit binds; a failed alternative's
            # partial captures never leak (scratch dict, lifted on hit)
            for alt in pat[1]:
                scratch = {}
                if self.match_pat(env, scratch, sv, alt):
                    bindings.update(scratch)
                    return True
            return False
        if k == "variant":
            _, tag, payload = pat
            if isinstance(sv, Variant) and sv.tag == tag:
                if payload is None:
                    return True  # tag-only form
                if sv.payload is None:
                    return False  # payload pattern vs payload-less value
                return self.match_pat(env, bindings, sv.payload, payload)
            return False
        if k == "list":
            _, elems, rest = pat
            if not isinstance(sv, list):
                return False
            if rest is None:
                if len(sv) != len(elems):
                    return False
            elif len(sv) < len(elems):
                return False
            for i, ep in enumerate(elems):
                if not self.match_pat(env, bindings, sv[i], ep):
                    return False
            if rest is not None:
                bindings[rest] = sv[len(elems):]
            return True
        if k == "map":
            _, keys = pat
            if not isinstance(sv, dict):
                return False
            for key, sub in keys:
                if key not in sv:
                    return False
                v = sv[key]
                if sub is None:
                    bindings[key] = v
                elif not self.match_pat(env, bindings, v, sub):
                    return False
            return True
        if k == "guard":
            _, p, cond = pat
            if not self.match_pat(env, bindings, sv, p):
                return False
            gev = self.new_scope(env)
            gev.update(bindings)
            try:
                return truthy(self.eval(gev, cond))
            except Stress as st:
                if st.prop is not None:
                    raise  # W06: propagation is a return, never a failure
                self.note(4, f"pattern guard contained: [{st.kind}] {st.message}")
                return False
        return False

    def exec_block(self, env, stmts):
        for s in stmts:
            self.exec_stmt(env, s)

    def exec_stmt(self, env, s):
        self.tick()
        k = s[0]
        if k == "block":
            self.exec_block(self.new_scope(env), s[1])
        elif k == "let":
            v = self.eval(env, s[2])
            if s[1] in env:
                self.note(4, f"rebinding '{s[1]}'")
            env[s[1]] = v
        elif k == "letann":
            # W01 (L2c): annotated definition — mismatch = catchable unfolded
            # Stress; the binding does NOT happen (mirror of interp.rs).
            _, name, ann, ex = s
            v = self.eval(env, ex)
            if not ann_matches(v, ann):
                raise Stress("unfolded", f"type annotation violated: '{name}' expects {ann_render(ann)}, got {type_name(v)}")
            if name in env:
                self.note(4, f"rebinding '{name}'")
            env[name] = v
        elif k == "letpat":
            # L1a: destructuring definition
            v = self.eval(env, s[2])
            self.bind_pattern(env, s[1], v)
        elif k == "forpat":
            # L1a: destructuring loop
            _, pat, it, body = s
            itv = self.eval(env, it)
            if isinstance(itv, SeqObj):
                while True:
                    self.tick()
                    item = itv.pull()
                    if item is None:
                        break
                    child = self.new_scope(env)
                    self.bind_pattern(child, pat, item)
                    try:
                        self.exec_block(child, body)
                    except BreakLoop:
                        break
                    except ContinueLoop:
                        continue
                    except Return as r:
                        raise r
                return
            if isinstance(itv, list):
                items = list(itv)
            elif isinstance(itv, str):
                items = list(itv)
            elif isinstance(itv, dict):
                items = list(itv.keys())
            else:
                self.note(4, f"cannot iterate {type_name(itv)}; loop skipped")
                items = []
            for item in items:
                self.tick()
                child = self.new_scope(env)
                self.bind_pattern(child, pat, item)
                try:
                    self.exec_block(child, body)
                except BreakLoop:
                    break
                except ContinueLoop:
                    continue
                except Return as r:
                    raise r
        elif k == "multi":
            # L1a: multiple assignment / swap — all values evaluated first
            _, targets, values, define = s
            vals = [self.eval(env, v) for v in values]
            if len(vals) < len(targets):
                self.note(4, "multi-assign: fewer values than targets; the rest bind null")
            elif len(vals) > len(targets):
                self.note(4, "multi-assign: extra values dropped")
            for i, t in enumerate(targets):
                val = vals[i] if i < len(vals) else None
                if define:
                    if t[0] == "ident":
                        name = t[1]
                        if name in env:
                            self.note(4, f"rebinding '{name}'")
                        env[name] = val
                    else:
                        self.note(4, "multi 'let' target must be a name; dropped")
                else:
                    if t[0] == "ident":
                        name = t[1]
                        target = self.find_env(env, name)
                        if target is None:
                            self.note(4, f"'{name}' was not declared; auto-declared")
                        self.assign(env, name, val)
                    elif t[0] == "index":
                        self.cur_line = t[3] if len(t) > 3 else 0  # W07 mirror: assign-target stamp
                        tv = self.eval(env, t[1])
                        iv = self.eval(env, t[2])
                        if isinstance(tv, list):
                            try:
                                idx = self.as_index(iv, len(tv))
                            except Stress:
                                idx = None
                            if idx is not None and idx < len(tv):
                                tv[idx] = val
                            else:
                                tv.append(val)
                                self.note(4, "index out of range; value appended")
                        elif isinstance(tv, dict):
                            key = iv if isinstance(iv, (str, int, float, bool)) else v_display(iv)
                            tv[key] = val
                        else:
                            self.note(4, "index assignment on non-container ignored")
                    elif t[0] == "member":
                        tv = self.eval(env, t[1])
                        if isinstance(tv, ObjInst):
                            tv.fields[t[2]] = val
                        elif isinstance(tv, dict):
                            tv[t[2]] = val
                        else:
                            self.note(4, "member assignment on non-map ignored")
                    else:
                        self.note(4, "multi-assign target invalid; value dropped")
        elif k == "assign":
            _, name, op, ve = s
            v = self.eval(env, ve)
            target = self.find_env(env, name)
            if op is None:
                if target is None:
                    self.note(4, f"'{name}' was not declared; auto-declared")
                self.assign(env, name, v)
            else:
                cur = self.lookup(env, name) if target is not None else None
                nv = self.binop(op, cur, v)
                if not self.assign(env, name, nv):
                    self.note(4, f"'{name}' was not declared; auto-declared")
        elif k == "idx_assign":
            _, te, ie, op, ve = s
            self.cur_line = te[3] if isinstance(te, tuple) and len(te) > 3 else 0  # W07 mirror
            tv = self.eval(env, te)
            iv = self.eval(env, ie)
            v = self.eval(env, ve)
            if isinstance(tv, list):
                try:
                    idx = self.as_index(iv, len(tv))
                except Stress:
                    idx = None
                cur = tv[idx] if (idx is not None and idx < len(tv)) else None
                nv = self.binop(op, cur, v) if op else v
                if idx is not None and idx < len(tv):
                    tv[idx] = nv
                else:
                    tv.append(nv)
                    self.note(4, "index out of range; value appended")
            elif isinstance(tv, dict):
                cur = None
                for kk, vv in tv.items():
                    if deep_eq(kk, iv):
                        cur = vv; break
                nv = self.binop(op, cur, v) if op else v
                key = iv if isinstance(iv, (str, int, float, bool)) else v_display(iv)
                tv[key] = nv
            else:
                self.note(4, "index assignment on non-container ignored")
        elif k == "mem_assign":
            _, te, key, op, ve = s
            tv = self.eval(env, te)
            v = self.eval(env, ve)
            if isinstance(tv, ObjInst):
                cur = tv.fields.get(key)
                tv.fields[key] = self.binop(op, cur, v) if op else v
            elif isinstance(tv, dict):
                cur = tv.get(key)
                tv[key] = self.binop(op, cur, v) if op else v
            else:
                self.note(4, "member assignment on non-map ignored")
        elif k == "if":
            _, branches, els = s
            for cond, body in branches:
                if truthy(self.eval(env, cond)):
                    self.exec_block(self.new_scope(env), body)
                    return
            if els is not None:
                self.exec_block(self.new_scope(env), els)
        elif k == "while":
            _, cond, body = s
            while truthy(self.eval(env, cond)):
                self.tick()
                try:
                    self.exec_block(self.new_scope(env), body)
                except BreakLoop:
                    break
                except ContinueLoop:
                    continue
                except Return as r:
                    raise r
        elif k == "loop":
            _, body = s
            while True:
                self.tick()
                try:
                    self.exec_block(self.new_scope(env), body)
                except BreakLoop:
                    break
                except ContinueLoop:
                    continue
                except Return as r:
                    raise r
        elif k == "for":
            _, name, it, body = s
            itv = self.eval(env, it)
            items = []
            if isinstance(itv, SeqObj):
                while True:
                    self.tick()
                    item = itv.pull()
                    if item is None:
                        break
                    child = self.new_scope(env)
                    child[name] = item
                    try:
                        self.exec_block(child, body)
                    except BreakLoop:
                        break
                    except ContinueLoop:
                        continue
                    except Return as r:
                        raise r
                return
            if isinstance(itv, list):
                items = list(itv)
            elif isinstance(itv, str):
                items = list(itv)
            elif isinstance(itv, dict):
                items = list(itv.keys())
            else:
                self.note(4, f"cannot iterate {type_name(itv)}; loop skipped")
            for item in items:
                self.tick()
                child = self.new_scope(env)
                child[name] = item
                try:
                    self.exec_block(child, body)
                except BreakLoop:
                    break
                except ContinueLoop:
                    continue
                except Return as r:
                    raise r
        elif k == "return":
            raise Return(self.eval(env, s[1]))
        elif k == "break":
            raise BreakLoop()
        elif k == "continue":
            raise ContinueLoop()
        elif k == "expr":
            self.eval(env, s[1])  # propagates: stress frames or top-level contain
        elif k == "match":
            _, subj, cases = s
            sv = self.eval(env, subj)
            for pat, body in cases:
                # W02 (match-v2): one binding dict per arm; captures live only
                # in the arm that hits (mirror of interp.rs Stmt::Match).
                bindings = {}
                if self.match_pat(env, bindings, sv, pat):
                    child = self.new_scope(env)
                    child.update(bindings)
                    self.exec_block(child, body)
                    return
        elif k == "use":
            modv = self.load_module(s[1])
            name = s[2] or os.path.basename(s[1].replace("\\", "/")).split(".")[0].split("/")[-1]
            env[name] = modv
            # flat-bind ALL exports beside the alias map (genes AND data)
            if isinstance(modv, dict):
                for kname, v in modv.items():
                    env[kname] = v
        elif k == "raise":
            msg = self.eval(env, s[2])
            raise Stress(s[1] or "unfolded", v_display(msg))
        elif k == "stress":
            _, kind, body, rescue = s
            try:
                self.exec_block(env, body)
            except Stress as st:
                # W06 (D-014) mirror: propagation is a RETURN, not a failure —
                # it crosses stress/rescue boundaries on its way to the gene
                # boundary. Pre-arms BEFORE kind matching so rescue (including
                # `rescue any`) can never contain or spoof it.
                if st.prop is not None:
                    raise Return(st.prop)
                if kind is not None and kind != st.kind and kind != "any":
                    raise st
                if rescue is not None:
                    bind, rbody = rescue
                    child = self.new_scope(env)
                    if bind:
                        child[bind] = st.as_map()
                    self.exec_block(child, rbody)
                else:
                    self.note(4, f"stress contained: [{st.kind}] {st.message}")
        elif k == "gene":
            g = s[1]
            name = g.name or "<lambda>"
            # T2b graded methylation (D-005): mirror of the Rust core
            if g.methylate:
                self.methyl_levels[name] = self.methyl_levels.get(name, 0) + 1
            elif g.acetylate:
                self.methyl_levels[name] = max(0, self.methyl_levels.get(name, 0) - 1)
            # reg-bio-3 (B3): executed @m6a defs deepen the site-density level
            if g.m6a:
                self.m6a_levels[name] = min(self.m6a_levels.get(name, 0) + 1, 3)
            # reg-bio-3 (C10): gene dosage registry
            if g.copies > 1:
                self.copies[name] = g.copies
            else:
                self.copies.pop(name, None)
            # @m6a-stabilized transcripts win dispatch among same-name candidates
            # (reg-bio-3 (B3): resistance reads the quantitative level >= 1)
            if not g.m6a and self.m6a_levels.get(name, 0) >= 1:
                self.note(4, f"'{name}' is @m6a-stabilized; redefinition ignored (mark the new copy to replace it)")
                return
            if g.name and g.name not in self.defined_genes:
                self.defined_genes.append(g.name)
            g.closure = env
            env[g.name or "<lambda>"] = g
        elif k == "pheno":
            p = s[1]
            self.phenos[p.name] = p
        elif k == "yield":
            v = self.eval(env, s[1]) if s[1] is not None else None
            if self.seq_buffer is not None:
                self.seq_buffer.append(v)
            else:
                self.note(4, "yield outside a sequence; treated as return")
                raise Return(v)
        elif k == "splice":
            _, root, variants = s
            # loop-9 (F-4): registry for runtime splice_shift re-resolution
            self.splice_registry[root] = s
            chosen = self.choose_variant(root, variants)
            if chosen:
                vname, vparams, body, vmarks = chosen
                self.note(1, f"splice '{root}' → variant '{vname}' active")
                g = Gene(root, vparams, None, body,
                         ac="acetylate" in vmarks, me="methylate" in vmarks, m6="m6a" in vmarks)
                if root not in self.defined_genes:
                    self.defined_genes.append(root)
                env[root] = g
        elif k == "silence":
            _, sfrm, sto, sstr, ssites = s
            self.silences.append((sfrm, sto, sstr, ssites))
            if sstr < 1.0 or ssites > 1:
                suffix = f" (strength {sstr!r}, sites {ssites})"
            else:
                suffix = ""
            if sto:
                self.note(1, f"RISC loaded: '{sfrm}' silenced → '{sto}'{suffix}")
            else:
                # reg-bio (F-4): pure RISC degradation (was a silent no-op)
                self.note(1, f"RISC loaded: '{sfrm}' degraded (no replacement){suffix}")
        elif k == "operon":
            # reg-bio-3 (A1/A7): register the unit (membership by name,
            # last-wins redefinition, cross-unit ownership ignored) (mirror)
            _, oname, omembers = s
            if not omembers:
                self.note(4, f"operon '{oname}': no cistrons declared; skipped")
            else:
                self.operons = [u for u in self.operons if u["name"] != oname]
                unit = {"name": oname, "members": [], "transcripts": 0}
                for og, orbs in omembers:
                    if any(om == og for u2 in self.operons for om, _o in u2["members"]):
                        self.note(4, f"cistron '{og}' already belongs to another operon; ignored")
                        continue
                    unit["members"].append((og, orbs))
                if not unit["members"]:
                    self.note(4, f"operon '{oname}': every cistron belonged to another unit; skipped")
                else:
                    self.operons.append(unit)
                    self.note(1, f"operon '{oname}': {len(unit['members'])} cistron(s) on one transcript")
        elif k == "enhance":
            for n in s[1]:
                if n not in self.enhanced:
                    self.enhanced.append(n)
        elif k == "ires":
            if s[1] not in self.ires:
                self.ires.append(s[1])
        elif k == "fate":
            self.fates[s[1]] = (s[2], s[3])
        elif k == "regulate":
            self.grn_edges.extend(s[1])
            self.trans_edges.extend(s[2])
            self.grn_binds.extend(s[3])
        elif k == "ligand":
            if s[1] not in self.ligands:
                self.ligands.append(s[1])
        elif k == "autoinducer":
            # loop-9 (C8): register a quorum-sensing signal species (mirror)
            self._signal_register(s[1])
        elif k == "decoy":
            self.decoys.append((s[1], s[2], s[3]))
        elif k == "toggle":
            self.toggles.append((s[1], s[2], True))
        elif k == "repressilator":
            self.repressi_ring = s[1]
            self.repressi_i = 0
            self.repressi_tick = 0
            # reg-bio (F-5): inline kinetics layer onto the current params
            for _fk in ("alpha", "gamma", "hill", "basal", "noise", "seed"):
                if s[3][_fk] is not None:
                    self.repressi_params[_fk] = s[3][_fk]
        elif k in ("frame", "edit", "anchor_export", "anchor_import"):
            pass
        elif k == "tad":
            self.exec_block(env, s[2])

    def choose_variant(self, root, variants):
        if not variants:
            return None
        cv = self.cell.get(f"variant.{root}")
        if cv:
            for vn, vp, b, mk in variants:
                if vn == cv:
                    return (vn, vp, b, mk)
        cv = self.cell.get("cli.variant")
        if cv:
            for vn, vp, b, mk in variants:
                if vn == cv:
                    return (vn, vp, b, mk)
        # 3. loop-9 (F-4): runtime splice_shift (a bound splicing factor
        # overrides the static mark; the operator pins above override it)
        sv = self.splice_shift.get(root)
        if sv:
            for vn, vp, b, mk in variants:
                if vn == sv:
                    return (vn, vp, b, mk)
        # 4. m6a-marked variant (T2c: mirrors the Rust core)
        for vn, vp, b, mk in variants:
            if "m6a" in mk:
                return (vn, vp, b, mk)
        # 5. first declared
        return variants[0]

    # ---- expressions
    def member_value(self, tv, key):
        # L1a: shared member read for `.` and `?.` (safe form pre-checks Null)
        if isinstance(tv, ObjInst):
            if key in tv.fields:
                return tv.fields[key]
            self.note(4, f"field '{key}' missing on phenotype {tv.defn.name}; null")
            return None
        if isinstance(tv, dict):
            if key in tv:
                return tv[key]
            self.note(4, f"member '{key}' missing on map; null")
            return None
        self.note(4, f"member '{key}' on {type_name(tv)} is null")
        return None

    def eval(self, env, e):
        self.tick()
        k = e[0]
        if k == "lit" or k == "null":
            return None if k == "null" else e[1]
        if k == "interp":
            out = []
            for pk, pv in e[1]:
                if pk == "lit":
                    out.append(pv)
                else:
                    out.append(v_display(self.eval(env, pv)))
            return "".join(out)
        if k == "list":
            return [self.eval(env, x) for x in e[1]]
        if k == "map":
            m = {}
            for ke, ve in e[1]:
                kv = self.eval(env, ke)
                key = kv if isinstance(kv, (str, int, float, bool)) else v_display(kv)
                m[key] = self.eval(env, ve)
            return m
        if k == "ident":
            v = self.lookup(env, e[1])
            if v is not None or e[1] in env or self.find_env(env, e[1]) is not None:
                return v
            self.note(4, f"unbound '{e[1]}' read as null")
            return None
        if k == "un":
            v = self.eval(env, e[2])
            if e[1] == "neg":
                if isinstance(v, (int, float)) and not isinstance(v, bool):
                    if isinstance(v, int) and v == -(2**63):
                        raise Stress("overflow", "int overflow in negation (i64::MIN)")
                    return -v
                raise Stress("unfolded", f"cannot negate {type_name(v)}")
            if e[1] == "bitnot":
                if isinstance(v, bool):
                    return ~int(v)
                if isinstance(v, int):
                    return ~v
                raise Stress("unfolded", f"cannot bit-invert {type_name(v)}")
            return not truthy(v)
        if k == "tern":
            _, cond, a, b = e
            return self.eval(env, a) if truthy(self.eval(env, cond)) else self.eval(env, b)
        if k == "prop":
            # W06 (D-014) mirror: `e?!` — Some/Ok unwrap to the payload;
            # None/Err unwind to the nearest enclosing gene boundary (the
            # gene RETURNS the variant). Plain values pass through untouched
            # (silent identity, the same contract as `?.` on non-null).
            _, sub, line = e
            v = self.eval(env, sub)
            if isinstance(v, Variant):
                if v.tag in ("Some", "Ok") and v.payload is not None:
                    return v.payload
                sig = Stress("propagate", "")
                sig.prop = v
                sig.line = line
                raise sig
            return v
        if k == "new":
            _, name, args_e = e
            p = self.phenos.get(name)
            if p is None:
                self.note(4, f"phenotype '{name}' not declared; instance is an empty map")
                return {}
            args = [self.eval(env, a) for a in args_e]
            return self.construct_obj(p, args)
        if k == "bin":
            self.cur_line = e[4] if len(e) > 4 else 0  # W07 mirror: dx-r4 stamp
            op = e[1]
            if op == "and":
                lv = self.eval(env, e[2])
                return self.eval(env, e[3]) if truthy(lv) else lv
            if op == "or":
                lv = self.eval(env, e[2])
                return lv if truthy(lv) else self.eval(env, e[3])
            if op == "nullish":
                # L1a: coalesce Null only — falsy non-null passes through
                lv = self.eval(env, e[2])
                return self.eval(env, e[3]) if lv is None else lv
            lv = self.eval(env, e[2])
            rv = self.eval(env, e[3])
            return self.binop(op, lv, rv)
        if k == "call":
            self.cur_line = e[3] if len(e) > 3 else 0  # W07 mirror: A13 stamp
            if e[1][0] == "ident":
                args = [self.eval(env, a) for a in e[2]]
                return self.call_named(env, e[1][1], args)
            callee = self.eval(env, e[1])
            args = [self.eval(env, a) for a in e[2]]
            return self.call_value(env, callee, args)
        if k == "index":
            self.cur_line = e[3] if len(e) > 3 else 0  # W07 mirror: dx-r4 stamp
            tv = self.eval(env, e[1])
            iv = self.eval(env, e[2])
            if isinstance(tv, list):
                idx = self.as_index(iv, len(tv))
                if idx < len(tv):
                    return tv[idx]
                raise Stress("missing", f"index {idx} out of range")
            if isinstance(tv, dict):
                for kk, vv in tv.items():
                    if deep_eq(kk, iv):
                        return vv
                raise Stress("missing", "key not found")
            if isinstance(tv, str):
                idx = self.as_index(iv, len(tv))
                if idx < len(tv):
                    return tv[idx]
                raise Stress("missing", "char index out of range")
            raise Stress("unfolded", f"cannot index {type_name(tv)}")
        if k == "member":
            tv = self.eval(env, e[1])
            return self.member_value(tv, e[2])
        if k == "member?":
            # L1a: optional chaining — Null receiver is Null, silently
            tv = self.eval(env, e[1])
            if tv is None:
                return None
            return self.member_value(tv, e[2])
        if k == "method?":
            tv = self.eval(env, e[1])
            if tv is None:
                return None
            args = [self.eval(env, a) for a in e[3]]
            return self.call_method(env, tv, e[2], args)
        if k == "method":
            tv = self.eval(env, e[1])
            args = [self.eval(env, a) for a in e[3]]
            return self.call_method(env, tv, e[2], args)
        if k == "lambda":
            g = e[1]
            g.name = g.name or "<lambda>"
            g.closure = env
            return g
        if k == "collect":
            _, var, it, filt, body = e
            itv = self.eval(env, it)
            if isinstance(itv, SeqObj):
                items = []
                while True:
                    v = itv.pull()
                    if v is None:
                        break
                    items.append(v)
            else:
                items = itv if isinstance(itv, list) else (list(itv) if isinstance(itv, str) else (list(itv.keys()) if isinstance(itv, dict) else []))
            out = []
            for item in items:
                self.tick()
                child = self.new_scope(env)
                child[var] = item
                if filt is not None and not truthy(self.eval(child, filt)):
                    continue
                out.append(self.eval(child, body))
            return out
        raise Stress("unfolded", f"unknown expression {k}")

    def as_index(self, v, ln):
        if isinstance(v, int) and not isinstance(v, bool):
            i = v
            if i < 0:
                i = ln + i
                if i < 0:
                    raise Stress("missing", f"index {v} out of range")
            return i
        raise Stress("missing", f"index must be int, found {type_name(v)}")

    def binop(self, op, l, r):
        if op == "+":
            if isinstance(l, bool) or isinstance(r, bool):
                raise Stress("unfolded", f"cannot add {type_name(l)} and {type_name(r)}")
            if isinstance(l, (int, float)) and isinstance(r, (int, float)):
                if isinstance(l, int) and isinstance(r, int) and not (-(2**63) <= l + r <= 2**63 - 1):
                    raise Stress("overflow", "int overflow in '+'")
                return l + r
            if isinstance(l, str) and isinstance(r, str):
                # reg-r4: concat ceiling parity with the Rust core — "exactly
                # at the 512 MiB line" + concat must raise, not allocate 1 GiB
                if len(l) + len(r) > 512 * 1024 * 1024:
                    raise Stress("overflow", "string concat exceeds the 512 MiB ceiling")
                return l + r
            if isinstance(l, list) and isinstance(r, list):
                if len(l) + len(r) > 64 * 1024 * 1024:
                    raise Stress("overflow", "list concat exceeds the 64M-element ceiling")
                return l + r
            raise Stress("unfolded", f"cannot add {type_name(l)} and {type_name(r)}")
        if op in ("-", "*", "/", "//", "%"):
            # string repetition (Python parity): "ab" * 3 / 3 * "ab"
            if op == "*" and isinstance(l, str) and not isinstance(r, str):
                n = int(r)
                if n < 0:
                    raise Stress("unfolded", "repeat count must be non-negative")
                if n * len(l) > 512 * 1024 * 1024:
                    raise Stress("overflow", "repeat exceeds the 512 MiB string ceiling")
                return l * n
            if op == "*" and isinstance(r, str) and not isinstance(l, str):
                n = int(l)
                if n < 0:
                    raise Stress("unfolded", "repeat count must be non-negative")
                if n * len(r) > 512 * 1024 * 1024:
                    raise Stress("overflow", "repeat exceeds the 512 MiB string ceiling")
                return r * n
            if isinstance(l, (int, float)) and isinstance(r, (int, float)) and not isinstance(l, bool) and not isinstance(r, bool):
                I64MIN, I64MAX = -(2**63), 2**63 - 1
                if op == "-":
                    if isinstance(l, int) and isinstance(r, int) and not (I64MIN <= l - r <= I64MAX):
                        raise Stress("overflow", "int overflow in '-'")
                    return l - r
                if op == "*":
                    if isinstance(l, int) and isinstance(r, int) and not (I64MIN <= l * r <= I64MAX):
                        raise Stress("overflow", "int overflow in '*'")
                    return l * r
                if op == "/":
                    if r == 0:
                        raise Stress("unfolded", "division by zero")
                    return l / r
                if op == "//":
                    if r == 0:
                        raise Stress("unfolded", "division by zero in '//'")
                    # SPEC/parity: floor division, ALWAYS int (i64)
                    q = l // r
                    if isinstance(q, float):
                        q = int(q)
                    if not (-(2**63) <= q <= 2**63 - 1):
                        raise Stress("overflow", "int overflow in '//'")
                    return q
                if r == 0:
                    raise Stress("unfolded", "modulo by zero")
                # SPEC: sign follows divisor (= Python % semantics)
                return l % r
            raise Stress("unfolded", f"cannot apply '{op}' to {type_name(l)} and {type_name(r)}")
        if op == "**":
            if isinstance(l, int) and isinstance(r, int) and not isinstance(l, bool) and not isinstance(r, bool) and r >= 0:
                if r > 10_000_000:
                    raise Stress("overflow", "int overflow in '**'")
                val = l ** r
                if not (-(2**63) <= val <= 2**63 - 1):
                    raise Stress("overflow", "int overflow in '**'")
                return val
            if isinstance(l, (int, float)) and isinstance(r, (int, float)) and not isinstance(l, bool) and not isinstance(r, bool):
                try:
                    return float(l) ** float(r)
                except OverflowError:
                    raise Stress("overflow", "float '**' overflowed to infinity")
            raise Stress("unfolded", f"cannot apply '**' to {type_name(l)} and {type_name(r)}")
        if op in ("&", "|", "^", "<<", ">>"):
            def as_i(x):
                if isinstance(x, bool):
                    return int(x)
                if isinstance(x, int):
                    return x
                raise Stress("unfolded", f"bitwise op needs ints, found {type_name(x)}")
            a, b = as_i(l), as_i(r)
            if op == "&": return a & b
            if op == "|": return a | b
            if op == "^": return a ^ b
            if b < 0 or b > 63:
                raise Stress("overflow", f"shift amount {b} out of range")
            if op == "<<":
                val = a << b
                # mirror the release-build i64 wrap (two's complement)
                val &= (1 << 64) - 1
                if val >= 2**63:
                    val -= 2**64
                return val
            return a >> b
        if op == "==":
            return deep_eq(l, r)
        if op == "!=":
            return not deep_eq(l, r)
        if op in ("<", "<=", ">", ">="):
            if isinstance(l, str) and isinstance(r, str):
                pass
            elif isinstance(l, (int, float)) and isinstance(r, (int, float)):
                pass
            else:
                raise Stress("unfolded", f"cannot order {type_name(l)} and {type_name(r)}")
            if op == "<": return l < r
            if op == "<=": return l <= r
            if op == ">": return l > r
            return l >= r
        if op == "in":
            if isinstance(r, list):
                return any(deep_eq(l, x) for x in r)
            if isinstance(r, str) and isinstance(l, str):
                return l in r
            if isinstance(r, dict):
                return any(deep_eq(k, l) for k in r.keys())
            raise Stress("unfolded", f"'in' not defined for {type_name(r)}")
        raise Stress("unfolded", f"unknown op {op}")

    # ---- calls
    def call_value(self, env, callee, args):
        if isinstance(callee, Gene):
            if not callee.seq:
                # reg-r4 (re-audit B-5): value-bound (higher-order) gene calls
                # pass the toggle gate too — "the pair gates calls" is
                # unqualified; the seq branch already gated, genes did not
                gname = callee.name or "<lambda>"
                for a, b, a_on in self.toggles:
                    if a == gname or b == gname:
                        this_is_a = a == gname
                        active = (a_on and this_is_a) or ((not a_on) and (not this_is_a))
                        if not active and not callee.acetylate:
                            winner = a if a_on else b
                            self.note(4, f"toggle repressed: '{gname}' is the inactive allele ('{winner}' is on)")
                            return None
                        break
            if callee.seq:
                # reg-r3 (re-audit): sequences honor ALL creation gates —
                # GRN veto, methylation, toggle — in call_gene_inner order
                # (mirror of the Rust branch; reg-r1 gated only the toggle)
                seq_name = callee.name or "<seq>"
                dl = getattr(callee, "line", 0)
                veto = self.grn_veto(seq_name)
                if veto is not None:
                    self.note(4, f"grn gate: sequence '{seq_name}' call suppressed ({veto})")
                    return None
                if not callee.acetylate:
                    lvl = self.methyl_levels.get(seq_name, 0)
                    if lvl >= self.methyl_threshold:
                        self.note(4, f"methylation silences: sequence '{seq_name}' (level {lvl} >= threshold {self.methyl_threshold}) — call returns null")
                        return None
                # reg-r4 (re-audit B-1): the toggle gate was missing here —
                # a toggle-repressed sequence created via a value binding
                # transcribed in the oracle while the Rust core refused it
                for a, b, a_on in self.toggles:
                    if a == seq_name or b == seq_name:
                        this_is_a = a == seq_name
                        active = (a_on and this_is_a) or ((not a_on) and (not this_is_a))
                        if not active and not callee.acetylate:
                            winner = a if a_on else b
                            self.note(4, f"toggle repressed: sequence '{seq_name}' is the inactive allele ('{winner}' is on)")
                            return None
                        break
                # reg-bio (F-1): promoter gate last — the pinned funnel order
                # ends here (bursting is the promoter's own stochastic
                # dynamics; no @acetylate exemption — open chromatin bursts too)
                if self._promoter_veto(seq_name, callee.burst):
                    self.note(4, f"promoter inactive: sequence '{seq_name}' burst-off — call returns null")
                    return None
                return SeqObj(self, callee, args)
            return self.call_gene(callee, args)
        self.note(4, f"called a {type_name(callee)} (not a gene); result null")
        return None

    def call_named(self, env, name, args):
        if name in BUILTIN_SYNONYMS:
            return self.builtin(env, BUILTIN_SYNONYMS[name], args)
        # reg-r4 (re-audit B-4): gate ORDER is pinned SPEC-wide — RISC at the
        # call site first, then the toggle gate (mirror of the Rust core:
        # silencing wins over repression because it rewrites the callee).
        # RISC silencing: redirect calls (acetylated genes are immune).
        # reg-bio-3 (C9): stoichiometric capture — every entry for the
        # target is one binding site; per-call capture p = 1 - Π(1-s)^sites.
        # strength 1.0 / one site = legacy binary redirect (no draw).
        entries = [e for e in self.silences if e[0] == name]
        if entries:
            target_gene = self.lookup(env, name)
            immune = isinstance(target_gene, Gene) and target_gene.acetylate
            if not immune:
                surv = 1.0
                for _f, _t, s_i, sites_i in entries:
                    base = 1.0 - s_i
                    for _k in range(sites_i):
                        surv *= base
                p = 1.0 - surv
                captured = True
                if p < 1.0:
                    x = self.rng
                    x ^= (x >> 12) & M64
                    x ^= (x << 25) & M64
                    x ^= (x >> 27) & M64
                    self.rng = x & M64
                    u = ((self.rng >> 11) & ((1 << 53) - 1)) / 9007199254740992.0
                    captured = u < p
                    if not captured and name not in self.risc_escaped:
                        self.risc_escaped.add(name)
                        self.note(4, f"RISC escape: '{name}' escaped silencing (strength {entries[0][2]!r}, sites {entries[0][3]})")
                if captured:
                    to = entries[0][1]
                    if to is not None:
                        self.note(4, f"RISC: call to '{name}' silenced → '{to}'")
                        tgt = self.lookup(env, to)
                        return self.call_value(env, tgt, args)
                    # reg-bio (F-4): pure degradation — no replacement executes
                    self.note(4, f"RISC: call to '{name}' degraded (no replacement)")
                    return None
        # toggle gate: the repressed allele refuses calls
        for a, b, a_on in self.toggles:
            if a == name or b == name:
                this_is_a = a == name
                active = (a_on and this_is_a) or ((not a_on) and (not this_is_a))
                target_gene = self.lookup(env, name)
                immune = isinstance(target_gene, Gene) and target_gene.acetylate
                if not active and not immune:
                    winner = a if a_on else b
                    self.note(4, f"toggle repressed: '{name}' is the inactive allele ('{winner}' is on)")
                    return None
                break
        tgt = self.lookup(env, name)
        if tgt is not None or self.find_env(env, name) is not None:
            return self.call_value(env, tgt, args)
        # fate constructor: Name() creates a fate-landscape instance
        if name in self.fates:
            states, enter = self.fates[name]
            return {"#fate": name, "#state": enter or (states[0][0] if states else "")}
        if name in BUILTINS:
            return self.builtin(env, name, args)
        limit2 = 1 if len(name) <= 4 else 2
        best, bd = None, 99
        for b in BUILTINS:
            d = edit_distance(name, b)
            if d <= limit2 and d < bd:
                best, bd = b, d
        if best:
            self.note(3, f"wobble: unknown gene '{name}' repaired to builtin '{best}'")
            return self.builtin(env, best, args)
        best, bd = None, 99
        for g in self.defined_genes:
            d = edit_distance(name, g)
            if d <= limit2 and d < bd:
                best, bd = g, d
        if best and bd > 0:
            self.note(3, f"wobble: unknown gene '{name}' repaired to gene '{best}'")
            gt = self.lookup(env, best)
            return self.call_value(env, gt, args)
        self.note(4, f"phantom call to '{name}'; result null")
        return None

    def call_gene(self, g, args):
        name = g.name or "<lambda>"
        # recursion depth limit (mirror of Rust core)
        self.depth += 1
        if self.depth > self.depth_limit:
            self.depth -= 1
            raise Stress("overflow", f"recursion depth limit ({self.depth_limit}) exceeded")
        # W007 mirror: the traceback frame for THIS gene — captured at entry
        # (cur_line is the call site); appended only on the error path.
        frame = (name, self.cur_line)
        try:
            result = self.call_gene_inner(g, args)
            return result
        except Stress as st:
            # W06 (D-014) mirror: propagation is a return, not a failure — no
            # chain frame. A returned variant is not an error in flight.
            if st.prop is not None:
                raise
            if len(st.chain) < 64:
                st.chain.append(frame)
            raise
        finally:
            self.depth -= 1

    def grn_veto(self, name):
        """reg-bio-3 (A1/A7): the call gate. A call to a cistron of a
        polycistronic unit is a transcription attempt of the WHOLE unit:
        edges targeting the unit veto every member first (induction acts
        on the unit's promoter), then the cistron's own edges apply.
        The unit pass short-circuits — its message wins. (mirror)"""
        if not self.grn_edges:
            return None
        ui = self._operon_of(name)
        if ui is not None:
            uname = self.operons[ui]["name"]
            if any(e[1] == uname for e in self.grn_edges):
                reason = self._gate_veto_for(uname)
                if reason is not None:
                    return f"operon '{uname}': {reason}"
        return self._gate_veto_for(name)

    def _operon_of(self, gene):
        """reg-bio-3 (A1/A7): index of the unit owning `gene`, if any."""
        for i, u in enumerate(self.operons):
            if any(m == gene for m, _r in u["members"]):
                return i
        return None

    def _gate_veto_for(self, name):
        """GRN cis-gate for ONE target (T2a / SPEC §11) — mirror of the
        Rust core (the pinned body, target as a parameter).

        Activating edge with explicit threshold t vetoes while
        level(source) < t; inhibiting edge with explicit threshold t vetoes
        while level(inhibitor) >= t; edges without a threshold stay
        declarative; a threshold of 0 never blocks (back-compat).
        T2e: an enhance'd gene lowers its activating thresholds by
        ENHANCE_DELTA (0.25)."""
        if not self.grn_edges:
            return None
        boosted = name in self.enhanced
        veto = None
        # reg-bio (F-3): AND/OR cis-regulatory logic — first-wins message
        # order preserved exactly (mirror of the Rust grn_veto). The gate
        # opens iff (every AND member passes) OR (any OR member passes) —
        # an `any` edge is an ALTERNATIVE activator that alone suffices.
        and_fail = None
        and_present = False
        or_present = False
        or_pass = False
        for frm, to, st, inh, thr, hill, is_any, occupy, is_sum, attenuating in self.grn_edges:
            if veto is not None:
                break
            if to != name:
                continue
            # reg-bio-2 (B7): sum edges are pooled members, evaluated by the
            # group pass below — never as individual AND members (mirror).
            if is_sum:
                continue
            # reg-bio-2 (C11 + A4): regulation reads the DNA-available
            # fraction (mirror of the Rust veto).
            lvl = self._regulated_level(self._grn_level(frm), frm)
            if inh:
                if thr is not None and lvl >= thr and and_fail is None:
                    # reg-bio-2 (A5): an `attenuates` edge reports the
                    # RNA-level mechanism (leader-termination outcome).
                    if attenuating:
                        veto = f"attenuator '{frm}' level {lvl!r} >= threshold {thr!r} (leader terminated)"
                    else:
                        veto = f"inhibitor '{frm}' level {lvl!r} >= threshold {thr!r}"
            elif thr is not None:
                t = thr
                if boosted:
                    t = max(0.0, t - self.enhance_delta)
                if is_any:
                    or_present = True
                    if lvl >= t:
                        or_pass = True
                else:
                    and_present = True
                    if lvl < t and and_fail is None:
                        and_fail = f"regulator '{frm}' level {lvl!r} < threshold {t!r}"
        # reg-bio-2 (B7): pooled (`sum`) edges — group pass (mirror of the
        # Rust grn_veto). Groups keyed by (target, threshold, hill) pool
        # weighted inputs P = min(1, Σ s·lvl); each group acts as ONE
        # conjunctive member passing iff P >= t (with the enhance boost).
        if veto is None:
            groups = []
            for frm, to, st, inh, thr, hill, is_any, occupy, is_sum, attenuating in self.grn_edges:
                if inh or not is_sum or to != name:
                    continue
                if thr is None or thr <= 0.0:
                    continue
                lvl = self._regulated_level(self._grn_level(frm), frm)
                n_h = hill if hill is not None else 2
                for g in groups:
                    if g[0] == to and g[1] == thr and g[2] == n_h:
                        g[3] += st * lvl
                        break
                else:
                    groups.append([to, thr, n_h, st * lvl])
            for to, thr, _n, pooled in groups:
                p = min(pooled, 1.0)
                t = thr
                if boosted:
                    t = max(0.0, t - self.enhance_delta)
                if p < t and and_fail is None:
                    and_fail = f"pooled regulators of '{to}' level {p!r} < threshold {t!r}"
        if veto is None and not or_pass:
            if and_fail is not None:
                veto = and_fail
            elif or_present and not and_present:
                veto = "no OR activator above threshold"
        return veto

    def _grn_level(self, frm):
        """A11 (reg-r2): mirror of the Rust grn_veto level resolution —
        explicit grn levels first, then the repressilator ring overlay
        (normalized raw/α, clamped 0..1) so the emergent oscillator
        genuinely drives downstream genes.
        reg-bio-2 (C11): the read returns the FREE fraction — decoy sites
        sequester their regulator (competitive titration, mirror of the
        Rust regulated_level)."""
        if frm in self.grn_levels:
            lvl = self.grn_levels[frm]
        elif self.repressi_ring and frm in self.repressi_ring:
            idx = self.repressi_ring.index(frm)
            lvls = repressilator_levels(len(self.repressi_ring), self.repressi_tick, self.repressi_params)
            lvl = min(lvls[idx] / self.repressi_params["alpha"], 1.0)
        elif frm in self.ligands:
            # reg-bio-2 (A4/C7): a ligand edge source reads its metabolite
            # pool — a riboswitch-style, protein-free gate (mirror).
            lvl = self._ligand_level(frm)
        elif frm in self.signals:
            # loop-9 (C8): a signal species reads the shared medium (the
            # LuxR-AHL population gate — mirror of Rust signal_level).
            lvl = self._signal_level(frm)
        else:
            lvl = 0.0
        return lvl

    def _signal_register(self, name):
        """loop-9 (C8): mirror of Rust signal_register — idempotent, cap 64."""
        if name in self.signals:
            return True
        if len(self.signals) >= 64:
            self.note(4, f"signal species cap (64) reached: '{name}' not registered")
            return False
        self.signals.append(name)
        return True

    def _signal_level(self, name):
        """loop-9 (C8): mirror of Rust signal_level — molecules / 1e9, ONE
        division (counts capped at 1e9 < 2**53, so the double is identical)."""
        return self.medium.get(name, 0) / 1e9

    def _ligand_level(self, name):
        """reg-bio-2 (A4): mirror of the Rust ligand_level — the runtime pool
        (`ligand_set`) wins; the `.cell [ligand.<name>]` bath is the default."""
        if name in self.ligand_pools:
            return self.ligand_pools[name]
        raw = self.cell.get("ligand." + name)
        if raw is None:
            return 0.0
        try:
            return min(max(float(raw), 0.0), 1.0)
        except ValueError:
            return 0.0

    def _regulated_level(self, raw, frm):
        """reg-bio-2 (C11 + A4): mirror of the Rust regulated_level — the
        DNA-available fraction: (1) decoy titration (subtractive), then
        (2) allosteric modulation (inducers: Π(1 − occ), cofactors: Π occ,
        occ = L/(k+L))."""
        l = raw
        for d, tf, cap in self.decoys:
            if tf == frm:
                l -= cap * self.grn_levels.get(d, 0.0)
        if l < 0.0:
            l = 0.0
        factor = 1.0
        for tf, lg, inducer, k in self.grn_binds:
            if tf == frm:
                lig = self._ligand_level(lg)
                occ = (lig / (k + lig)) if k > 0.0 else 1.0
                factor *= (1.0 - occ) if inducer else occ
        out = l * factor
        # reg-bio-3 (C10): gene dosage — @copies amplifies the CONCENTRATION
        # the gene feeds its edges, saturating on the 0..1 lattice (mirror)
        c = self.copies.get(frm)
        if c is not None and c > 1:
            out *= c
        return out if out <= 1.0 else 1.0

    def _rho_knobs(self):
        """loop-10 (F-7/F-8): mirror of the Rust rho_knobs — (armed, catch,
        queue_floor, queue_cap, drain). Host: .cell parsed per use (garbage
        falls back identically: catch clamps 0..1, the rest parse-or-default);
        worker: the pinned snapshot tuple. Default-off: absent
        rho.termination = false — legacy runs draw nothing, bit-identical."""
        if self.rho_pins is not None:
            return self.rho_pins
        armed = self.cell.get("rho.termination") == "true"
        catch = self.cell.get("rho.catch")
        try:
            catch = min(max(float(catch), 0.0), 1.0) if catch is not None else 0.5
        except ValueError:
            catch = 0.5

        def _g(key, dflt):
            v = self.cell.get(key)
            try:
                return float(v) if v is not None else dflt
            except (TypeError, ValueError):
                return dflt

        return (armed, catch, _g("rho.queue_floor", 0.5), _g("ribosome.queue_cap", 1.0), _g("ribosome.drain", 0.5))

    def _trans_integrate(self):
        """reg-bio-2 (C1): mirror of the Rust trans_integrate — one Euler
        step per translates edge: p += rate·Δcalls − decay·p (clamped 0..1)
        where Δcalls is the source's call-count delta since the last
        integration (checkpoints start at 0)."""
        # loop-10 (F-8 + R10 kinetics jury L1): ribosome queue drain (mirror)
        # — at ENTRY (before the empty-check: "every integration point") and
        # before the edge loop (pinned order; both entry points reach
        # _trans_integrate). Exists only under rho.termination; max(0.0) is
        # mirror-safe.
        rho_on, _c, _fl, _cap, rho_drain = self._rho_knobs()
        if rho_on:
            for k2 in list(self.ribo_queue):
                self.ribo_queue[k2] = max(0.0, self.ribo_queue[k2] - rho_drain)
        if not self.trans_edges:
            return
        for frm, to, rate, pdecay in self.trans_edges:
            key = frm + "\u0000" + to
            now = self.call_counts.get(frm, 0)
            last = self.trans_last.get(key, 0)
            self.trans_last[key] = now
            delta = now - last
            if delta < 0:
                delta = 0
            dec = pdecay if pdecay is not None else 0.0
            # loop-9 (F-6): m6A reader fate (mirror) — engage only at mark
            # density >= min_level (default 2); the {0,1} legacy lattice is
            # bit-identical. Knobs: .cell m6a.reader.decay (0.25) /
            # m6a.reader.translation (0.10) / m6a.reader.min_level (2).
            yd2 = self.cell.get("m6a.reader.decay")
            try:
                yd2 = min(max(float(yd2), 0.0), 1.0) if yd2 is not None else 0.25
            except ValueError:
                yd2 = 0.25
            yatt = self.cell.get("m6a.reader.translation")
            try:
                yatt = min(max(float(yatt), 0.0), 1.0) if yatt is not None else 0.10
            except ValueError:
                yatt = 0.10
            minl = self.cell.get("m6a.reader.min_level")
            try:
                minl = min(max(int(minl), 0), 3) if minl is not None else 2
            except ValueError:
                minl = 2
            reader = self.m6a_levels.get(frm, 0) >= minl
            dec_eff = dec + yatt if reader else dec
            if delta == 0 and dec_eff == 0.0:
                continue
            r = rate if rate is not None else 1.0
            # reg-bio-3 (A1/A7): per-cistron rbs efficiency + transcriptional
            # polarity (upstream blocking reduces downstream yield) (mirror)
            # loop-9 (P0-1): per-call-WEIGHTED polarity (mirror) — each
            # upstream member contributes its expected factor: methylation
            # past threshold => pol; a target-less RISC silence with capture
            # p = 1 − Π(1−s_i)^sites_i => surv + (1−surv)·pol with
            # surv = Π(1−s_i)^sites_i; nothing => 1.0. Legacy inputs
            # degenerate to exactly 1.0 or pol in the same member order,
            # bit-identical to the old binary rule; no randomness consumed.
            ui = self._operon_of(frm)
            if ui is not None:
                u = self.operons[ui]
                pos = next(i for i, (m, _r) in enumerate(u["members"]) if m == frm)
                r *= u["members"][pos][1]
                # loop-10 (W2, R10 biology jury): under Rho ON the polarity
                # factor is IDENTITY — D2 resolves the upstream failure per
                # call, so a surviving transcript translates at full rate
                # (D1's expected-value derate would double-count the same
                # loss). Rho OFF keeps the exact legacy fold (bit-identical).
                if not rho_on:
                    pol = self.cell.get("operon.polarity")
                    pv = 0.5
                    if pol is not None:
                        try:
                            pv = min(max(float(pol), 0.0), 1.0)
                        except ValueError:
                            pv = 0.5
                    f2 = 1.0
                    for g2, _r2 in u["members"][:pos]:
                        methylated = self.methyl_levels.get(g2, 0) >= self.methyl_threshold
                        if methylated:
                            factor = pv
                        else:
                            surv = 1.0
                            silenced = False
                            for f3, t3, s3, n3 in self.silences:
                                if f3 == g2 and t3 is None:
                                    silenced = True
                                    base = 1.0 - s3
                                    for _k in range(n3):
                                        surv *= base
                            if silenced:
                                factor = surv + (1.0 - surv) * pv
                            else:
                                factor = 1.0
                        f2 *= factor
                    r *= f2
            # loop-9 (F-6): YTHDF2 decay routing — the LAST production
            # multiply (normative order: rate x rbs x polarity x (1-yd2))
            if reader:
                r *= 1.0 - yd2
            cur = self.grn_levels.get(to, 0.0)
            p = cur + r * delta - dec_eff * cur
            if p < 0.0:
                p = 0.0
            if p > 1.0:
                p = 1.0
            self.grn_levels[to] = p

    def _grn_decay_tick(self):
        """reg-bio-2 (C2): mirror of the Rust grn_decay_tick — the
        decay_clock builtin overrides the .cell keys (`grn.decay_calls`/
        `grn.decay`); one decay step every N calls, then translation
        integrates. Unset = no-op (byte-identical event-driven contract)."""
        if self.decay_clock_n is not None:
            n = self.decay_clock_n
        else:
            raw = self.cell.get("grn.decay_calls")
            if raw is None:
                return
            try:
                n = int(raw)
            except (TypeError, ValueError):
                return
            if n <= 0:
                return
        if self.call_clock % n != 0:
            return
        if self.decay_clock_f is not None:
            decay = self.decay_clock_f
        else:
            d = self.cell.get("grn.decay")
            decay = 0.0
            if d is not None:
                try:
                    decay = min(max(float(d), 0.0), 1.0)
                except ValueError:
                    decay = 0.0
        if decay > 0.0 and self.grn_levels:
            retention = 1.0 - decay
            for k2 in list(self.grn_levels):
                self.grn_levels[k2] *= retention
                if self.grn_levels[k2] < 2.220446049250313e-16:
                    self.grn_levels[k2] = 0.0
        # reg-bio-3 (B3): m6A decay — `.cell m6a.decay f` erases site
        # density as time passes (half-down rounding on the 0..=3 lattice;
        # a diluted mark never reads as MORE marked). Unset = byte-identical.
        mdecay = self.cell.get("m6a.decay")
        if mdecay is not None and self.m6a_levels:
            try:
                mf = min(max(float(mdecay), 0.0), 1.0)
            except ValueError:
                mf = 0.0
            if mf > 0.0:
                import math as _math
                for k2 in list(self.m6a_levels):
                    x = self.m6a_levels[k2] * (1.0 - mf) - 0.5
                    self.m6a_levels[k2] = max(0, int(_math.ceil(x)))
        self._trans_integrate()

    def _promoter_veto(self, name, burst=None):
        """reg-bio (F-1): telegraph promoter draw — mirror of the Rust
        promoter_veto. One draw per call attempt on the SHARED xorshift64*
        stream (the `random()` state machine): active → off with p=koff,
        inactive → on with p=kon. State persists across calls (the burst).
        loop-9 (F-2): a per-gene @burst mark overrides (kon, koff) for THIS
        gene only — promoter identity. loop-9 (F-3): attempt telemetry."""
        if not self.expr_stochastic:
            return False
        if burst is None:
            burst = self.burst_overrides.get(name)
        kon, koff = burst if burst is not None else (self.expr_kon, self.expr_koff)
        was_active = self.promoter_states.get(name, True)
        x = self.rng
        x ^= (x >> 12) & M64
        x ^= (x << 25) & M64
        x ^= (x >> 27) & M64
        self.rng = x & M64
        x = self.rng
        u = ((x >> 11) & ((1 << 53) - 1)) / 9007199254740992.0
        if was_active:
            now_active = u >= koff
        else:
            now_active = u < kon
        self.promoter_states[name] = now_active
        e = self.promoter_tel.get(name, (0, 0, 0))
        self.promoter_tel[name] = (e[0] + 1, e[1] + (1 if now_active else 0), e[2] + (1 if (not now_active) and was_active else 0))
        if not now_active:
            self.burst_off[name] = self.burst_off.get(name, 0) + 1
            return True
        return False

    def call_gene_inner(self, g, args):
        name = g.name or "<lambda>"
        # GRN gate first: a suppressed call is not expression — it must not
        # reach the call counters, the burst bins, or the gene body.
        veto = self.grn_veto(name)
        if veto is not None:
            self.note(4, f"grn gate: '{name}' call suppressed ({veto})")
            return None
        # T2b methylation gate: level >= threshold blocks transcription;
        # @acetylate genes are exempt (open chromatin wins — D-005).
        if not g.acetylate:
            lvl = self.methyl_levels.get(name, 0)
            if lvl >= self.methyl_threshold:
                self.note(4, f"methylation silences: '{name}' (level {lvl} >= threshold {self.methyl_threshold}) — call returns null")
                return None
        # loop-9 (F-5): CIS riboswitch — after chromatin, before the
        # promoter. The pinned order extends to
        # RISC → toggle → GRN → methylation → riboswitch → promoter.
        if g.riboswitch is not None:
            lig, on, rt = g.riboswitch
            lvl = self._ligand_level(lig)
            bound = lvl >= rt
            veto = (not bound) if on else bound
            if veto:
                why = "unbound: RBS sequestered" if on else "bound: terminator hairpin folded"
                self.note(4, f"riboswitch '{lig}' {why}: '{name}' call suppressed")
                return None
        # reg-bio (F-1): telegraph promoter layer — the pinned gate order
        # ends here: RISC → toggle → GRN → methylation → riboswitch → promoter.
        if self._promoter_veto(name, g.burst):
            self.note(4, f"promoter inactive: '{name}' burst-off — call returns null")
            return None
        # loop-10 (F-7): Rho-dependent termination (mirror) — opt-in (.cell
        # rho.termination = true). Pinned gate order ends: ... promoter → RHO.
        # Naked upstream RNA scan in member order: p_g = 1 (methylated) or
        # 1 − surv(g) (target-less RISC silence) or 0; draws only where
        # 0 < p_g < 1 (member order) then one catch-up draw where 0 < q < 1;
        # catch probability compounding over the naked runway (no pow; the
        # R10 W1 fix — pressure GROWS with distance); F-8 shield: queue
        # >= rho.queue_floor occludes the rut sites. A terminated call is
        # not expression: returns null before counters/transcript/queue.
        rho_on, rho_catch, rho_floor, rho_cap, _drain = self._rho_knobs()
        if rho_on:
            rui = self._operon_of(name)
            if rui is not None:
                pos = next(i for i, (m, _r) in enumerate(self.operons[rui]["members"]) if m == name)
                terminated = None
                for i in range(pos):
                    g2 = self.operons[rui]["members"][i][0]
                    if self.methyl_levels.get(g2, 0) >= self.methyl_threshold:
                        p_g = 1.0
                    else:
                        surv = 1.0
                        has_silence = False
                        for f3, t3, s3, n3 in self.silences:
                            if f3 == g2 and t3 is None:
                                has_silence = True
                                base = 1.0 - s3
                                for _k in range(n3):
                                    surv *= base
                        p_g = (1.0 - surv) if has_silence else 0.0
                    if p_g == 0.0:
                        continue
                    x = self.rng
                    if 0.0 < p_g and p_g < 1.0:
                        x ^= (x >> 12) & M64
                        x ^= (x << 25) & M64
                        x ^= (x >> 27) & M64
                        self.rng = x & M64
                    if p_g == 1.0:
                        naked_g = True
                    else:
                        x = self.rng
                        u = ((x >> 11) & ((1 << 53) - 1)) / 9007199254740992.0
                        naked_g = u < p_g
                    shielded = self.ribo_queue.get(g2, 0.0) >= rho_floor
                    if naked_g and not shielded:
                        # q = 1 - (1-catch)^d — per-cistron catch compounding
                        # over the naked runway (mirror of the Rust W1 fix:
                        # catch^d had the distance profile inverted — the
                        # FURTHER downstream the reader, the MORE time Rho
                        # has had to catch up)
                        per = 1.0 - rho_catch
                        surv = 1.0
                        for _k in range(pos - i):
                            surv *= per
                        q = 1.0 - surv
                        if q == 0.0:
                            break
                        x2 = self.rng
                        if 0.0 < q and q < 1.0:
                            x2 ^= (x2 >> 12) & M64
                            x2 ^= (x2 << 25) & M64
                            x2 ^= (x2 >> 27) & M64
                            self.rng = x2 & M64
                        if q == 1.0:
                            caught = True
                        else:
                            x2 = self.rng
                            u2 = ((x2 >> 11) & ((1 << 53) - 1)) / 9007199254740992.0
                            caught = u2 < q
                        if caught:
                            terminated = g2
                        break
                if terminated is not None:
                    self.note(4, f"rho terminated: transcript lost at '{terminated}' — call returns null")
                    return None
        # reg-bio-3 (A1/A7): the call passed every gate — one transcript of
        # the unit is made (a suppressed call is NOT expression) (mirror)
        ui = self._operon_of(name)
        if ui is not None:
            self.operons[ui]["transcripts"] += 1
            # loop-10 (F-8): ribosome queue register (mirror) — every
            # member's queue grows by its rbs, capped at ribosome.queue_cap;
            # rho.termination-gated (inert bookkeeping otherwise).
            if rho_on:
                for m2, rbs2 in self.operons[ui]["members"]:
                    nv = self.ribo_queue.get(m2, 0.0) + rbs2
                    self.ribo_queue[m2] = rho_cap if nv > rho_cap else nv
        self.call_counts[name] = self.call_counts.get(name, 0) + 1
        self.call_clock += 1
        # reg-bio-2 (C2): the decay clock (mirror of the Rust hook)
        self._grn_decay_tick()
        bucket = self.call_clock // 20
        self.gene_buckets.setdefault(name, {}).setdefault(bucket, 0)
        self.gene_buckets[name][bucket] += 1
        if g.methylate and not self.methyl_quiet and name not in self.methyl_noted:
            self.methyl_noted.add(name)
            self.note(4, f"methylated call: '{name}' (chromatin repressed)")
        fenv = self.new_scope(g.closure if g.closure is not None else self.globals)
        for i, (pname, dflt) in enumerate(g.params):
            if pname in ("?", ""):
                continue
            if i < len(args):
                # W01 (L2c): soft param annotation at the funnel (mirror).
                anns = g.param_anns
                if i < len(anns) and anns[i] is not None and not ann_matches(args[i], anns[i]):
                    raise Stress("unfolded", f"argument '{pname}' for gene '{name}' expects {ann_render(anns[i])}, got {type_name(args[i])}")
                fenv[pname] = args[i]
            elif dflt is not None:
                dv = self.eval(fenv, dflt)
                anns = g.param_anns
                if i < len(anns) and anns[i] is not None and not ann_matches(dv, anns[i]):
                    raise Stress("unfolded", f"default of '{pname}' for gene '{name}' expects {ann_render(anns[i])}, got {type_name(dv)}")
                fenv[pname] = dv
            else:
                self.note(4, f"missing argument '{pname}' in call to {name}; bound null")
                fenv[pname] = None
        if len(args) > len(g.params) and g.params:
            self.note(4, f"{len(args) - len(g.params)} extra argument(s) in call to {name} ignored")
        if g.guard is not None:
            cond, gbody = g.guard
            ok = False
            try:
                ok = truthy(self.eval(fenv, cond))
            except Stress:
                ok = False
            if not ok:
                self.note(4, f"guard tripped calling {name}")
                try:
                    self.exec_block(fenv, gbody)
                    self.note(4, f"guard of {name} returned null (uORF repression)")
                    return None
                except Return as r:
                    return r.value
                except Stress as st:
                    # W06 (D-014) mirror: propagation from a guard body returns
                    # from the gene (same contract as a guard `return`).
                    if st.prop is not None:
                        return st.prop
                    raise
        try:
            self.exec_block(fenv, g.body)
            # no explicit return → null; a non-optional return annotation is
            # violated by the implicit null too (mirror of interp.rs)
            return self._check_ret(name, g, None, False)
        except Return as r:
            return self._check_ret(name, g, r.value, True)
        except Stress as st:
            # W06 (D-014) mirror: a propagated variant IS the gene's return
            # value — the signal unwinds here and becomes the result.
            if st.prop is not None:
                return self._check_ret(name, g, st.prop, True)
            raise

    def _check_ret(self, name, g, v, explicit):
        """W01 (L2c): soft return annotation — checked on the value the gene
        actually returns (including a `?!`-propagated variant)."""
        if g.ret_ann is None:
            return v
        if explicit:
            if not ann_matches(v, g.ret_ann):
                raise Stress("unfolded", f"return of gene '{name}' expects {ann_render(g.ret_ann)}, got {type_name(v)}")
        else:
            if not ann_matches(None, g.ret_ann):
                raise Stress("unfolded", f"return of gene '{name}' expects {ann_render(g.ret_ann)}, got null (no return statement ran)")
        return v

    # ---- methods
    def call_method_gene(self, g, self_val, args):
        self.depth += 1
        if self.depth > self.depth_limit:
            self.depth -= 1
            raise Stress("overflow", f"recursion depth limit ({self.depth_limit}) exceeded")
        try:
            name = g.name or "<method>"
            # T2b methylation gate for phenotype methods (same contract).
            if not g.acetylate:
                lvl = self.methyl_levels.get(name, 0)
                if lvl >= self.methyl_threshold:
                    self.note(4, f"methylation silences: '{name}' (level {lvl} >= threshold {self.methyl_threshold}) — call returns null")
                    return None
            # reg-bio (F-1): promoter gate (funnel order preserved)
            if self._promoter_veto(name):
                self.note(4, f"promoter inactive: '{name}' burst-off — call returns null")
                return None
            self.call_counts[name] = self.call_counts.get(name, 0) + 1
            self.call_clock += 1
            # reg-bio-2 (C2): the decay clock (mirror of the Rust hook)
            self._grn_decay_tick()
            bucket = self.call_clock // 20
            self.gene_buckets.setdefault(name, {}).setdefault(bucket, 0)
            self.gene_buckets[name][bucket] += 1
            if g.methylate and not self.methyl_quiet and name not in self.methyl_noted:
                self.methyl_noted.add(name)
                self.note(4, f"methylated call: '{name}' (chromatin repressed)")
            fenv = self.new_scope(self.globals)
            fenv["self"] = self_val
            for i, (pname, dflt) in enumerate(g.params):
                if pname in ("?", "", "self"):
                    continue
                if i < len(args):
                    # W01 (L2c): method param annotations — same soft contract.
                    anns = g.param_anns
                    if i < len(anns) and anns[i] is not None and not ann_matches(args[i], anns[i]):
                        raise Stress("unfolded", f"argument '{pname}' for method '{name}' expects {ann_render(anns[i])}, got {type_name(args[i])}")
                    fenv[pname] = args[i]
                elif dflt is not None:
                    fenv[pname] = self.eval(fenv, dflt)
                else:
                    fenv[pname] = None
            if g.guard is not None:
                cond, gbody = g.guard
                try:
                    ok = truthy(self.eval(fenv, cond))
                except Stress:
                    ok = False
                if not ok:
                    self.note(4, f"guard tripped calling {name}")
                    try:
                        self.exec_block(fenv, gbody)
                        return self._check_ret_method(name, g, None, False)
                    except Return as r:
                        return self._check_ret_method(name, g, r.value, True)
                    except Stress as st:
                        # W06 (D-014) mirror: guard-body propagation returns
                        # from the method (same contract as a guard `return`).
                        if st.prop is not None:
                            return self._check_ret_method(name, g, st.prop, True)
                        raise
            try:
                self.exec_block(fenv, g.body)
                return self._check_ret_method(name, g, None, False)
            except Return as r:
                return self._check_ret_method(name, g, r.value, True)
            except Stress as st:
                # W06 (D-014) mirror: a propagated variant IS the method's
                # return value.
                if st.prop is not None:
                    return self._check_ret_method(name, g, st.prop, True)
                raise
        finally:
            self.depth -= 1

    def _check_ret_method(self, name, g, v, explicit):
        """W01 (L2c): method return annotation — same soft contract."""
        if g.ret_ann is None:
            return v
        if explicit:
            if not ann_matches(v, g.ret_ann):
                raise Stress("unfolded", f"return of method '{name}' expects {ann_render(g.ret_ann)}, got {type_name(v)}")
        else:
            if not ann_matches(None, g.ret_ann):
                raise Stress("unfolded", f"return of method '{name}' expects {ann_render(g.ret_ann)}, got null (no return statement ran)")
        return v

    def call_method(self, env, recv, name, args):
        if isinstance(recv, SeqObj):
            if name == "next":
                return recv.pull()
            if name == "collect":
                out = []
                while True:
                    v = recv.pull()
                    if v is None:
                        break
                    out.append(v)
                return out
            self.note(4, f"unknown sequence method '{name}'; null")
            return None
        if isinstance(recv, ObjInst):
            chain = []
            d = recv.defn
            hops = 0
            while d is not None and hops <= 32:
                chain.append(d)
                d = self.phenos.get(d.parent) if d.parent else None
                hops += 1
            for dd in chain:
                for g in dd.methods:
                    if g.name == name:
                        return self.call_method_gene(g, recv, args)
            if name in recv.fields:
                return self.call_value(env, recv.fields[name], args)
            self.note(4, f"phenotype {recv.defn.name} has no method '{name}'; null")
            return None
        if isinstance(recv, dict) and "#fate" in recv:
            fate_name, cur = recv["#fate"], recv["#state"]
            if name == "shift":
                target = v_display(args[0]) if args else ""
                states, _ = self.fates.get(fate_name, ([], None))
                allowed = any(f == cur and target in tg for f, tg in states)
                if allowed:
                    recv["#state"] = target
                    return True
                self.note(4, f"fate {fate_name}: '{cur}' → '{target}' crosses a valley; state held")
                return False
            if name == "state":
                return recv.get("#state")
            if name == "can":
                target = v_display(args[0]) if args else ""
                states, _ = self.fates.get(fate_name, ([], None))
                return any(f == cur and target in tg for f, tg in states)
        if isinstance(recv, str):
            if name == "at":
                # L1a: safe char access with an optional default
                i = args[0] if args else None
                if isinstance(i, bool) or not isinstance(i, int):
                    self.note(4, "at needs an int index; null")
                    return None
                j = len(recv) + i if i < 0 else i
                if 0 <= j < len(recv):
                    return recv[j]
                if len(args) > 1:
                    return args[1]
                self.note(4, "char index out of range; null")
                return None
            if name == "upper": return recv.upper()
            if name == "lower": return recv.lower()
            if name == "trim": return recv.strip()
            if name == "split":
                sep = v_display(args[0]) if args else " "
                return recv.split(sep)
            if name == "join":
                sep = recv
                lst = args[0] if args else []
                return sep.join(v_display(x) for x in lst)
            if name == "replace":
                return recv.replace(v_display(args[0]), v_display(args[1]))
            if name == "contains": return v_display(args[0]) in recv
            if name == "starts": return recv.startswith(v_display(args[0]))
            if name == "ends": return recv.endswith(v_display(args[0]))
            if name == "repeat": return recv * int(args[0])
            if name == "slice":
                a = int(args[0]) if len(args) > 0 else 0
                b = int(args[1]) if len(args) > 1 else len(recv)
                return recv[a:b]
            if name == "len": return len(recv)
        elif isinstance(recv, list):
            if name == "map":
                return [self.call_value(env, args[0], [x]) for x in recv]
            if name == "filter":
                return [x for x in recv if truthy(self.call_value(env, args[0], [x]))]
            if name == "reduce":
                acc = args[1] if len(args) > 1 else 0
                for x in recv:
                    acc = self.call_value(env, args[0], [acc, x])
                return acc
            if name == "each":
                for x in recv:
                    self.call_value(env, args[0], [x])
                return None
            if name == "sort":
                if args and isinstance(args[0], Gene):
                    import functools
                    return sorted(recv, key=functools.cmp_to_key(
                        lambda a, b: -1 if truthy(self.call_value(env, args[0], [a, b])) else 1))
                return sorted(recv, key=lambda x: (
                    (0, float(x), "") if isinstance(x, (int, float)) and not isinstance(x, bool)
                    else ((0, float(int(x)), "") if isinstance(x, bool)
                          else ((1, 0.0, x) if isinstance(x, str)
                                else (2, 0.0, repr_of(x))))
                ))
            if name == "reverse": return list(reversed(recv))
            if name == "contains": return any(deep_eq(x, args[0]) for x in recv)
            if name == "index_of":
                for i, x in enumerate(recv):
                    if deep_eq(x, args[0]):
                        return i
                return -1
            if name == "slice":
                a = int(args[0]) if len(args) > 0 else 0
                b = int(args[1]) if len(args) > 1 else len(recv)
                return recv[a:b]
            if name == "join":
                sep = v_display(args[0]) if args else ""
                return sep.join(v_display(x) for x in recv)
            if name == "len": return len(recv)
            if name == "push": recv.append(args[0]); return recv
            if name == "pop": return recv.pop() if recv else None
            if name == "get":
                # L1a: safe index read with an optional default
                i = args[0] if args else None
                if isinstance(i, bool) or not isinstance(i, int):
                    self.note(4, "list get needs an int index; null")
                    return None
                j = len(recv) + i if i < 0 else i
                if 0 <= j < len(recv):
                    return recv[j]
                if len(args) > 1:
                    return args[1]
                self.note(4, "index out of range; null")
                return None
        elif isinstance(recv, dict):
            if name == "keys": return list(recv.keys())
            if name == "values": return list(recv.values())
            if name == "items": return [[k, v] for k, v in recv.items()]
            if name == "get":
                # L1a: safe key access with an optional default
                t = args[0] if args else None
                hit = None
                found = False
                for k in recv.keys():
                    if deep_eq(k, t):
                        hit = recv[k]
                        found = True
                        break
                if found:
                    return hit
                if len(args) > 1:
                    return args[1]
                self.note(4, f"key '{v_display(t)}' missing; null")
                return None
            if name == "has": return any(deep_eq(k, args[0]) for k in recv.keys())
            if name == "del":
                for k in list(recv.keys()):
                    if deep_eq(k, args[0]):
                        del recv[k]
                return None
            if name == "len": return len(recv)
            if name in recv and isinstance(recv[name], (Gene,)):
                return self.call_value(env, recv[name], args)
        self.note(4, f"{type_name(recv)} has no method '{name}'; null")
        return None

    # ---- builtins
    def builtin(self, env, name, args):
        if name == "promote":
            print(" ".join(v_display(a) for a in args))
            return None
        if name == "len":
            v = args[0] if args else None
            if isinstance(v, (str, list, dict)):
                return len(v)
            self.note(4, "len() of non-container is 0")
            return 0
        if name == "push":
            args[0].append(args[1]); return args[0]
        if name == "pop":
            return args[0].pop() if args[0] else None
        if name == "insert":
            idx = self.as_index(args[1], len(args[0]))
            args[0].insert(min(idx, len(args[0])), args[2]); return args[0]
        if name == "remove":
            idx = self.as_index(args[1], len(args[0]))
            if idx < len(args[0]):
                return args[0].pop(idx)
            raise Stress("missing", "remove index out of range")
        if name == "keys":
            return list(args[0].keys()) if isinstance(args[0], dict) else []
        if name == "values":
            return list(args[0].values()) if isinstance(args[0], dict) else []
        if name == "has":
            return isinstance(args[0], dict) and any(deep_eq(k, args[1]) for k in args[0].keys())
        if name == "del":
            if isinstance(args[0], dict):
                for k in list(args[0].keys()):
                    if deep_eq(k, args[1]):
                        del args[0][k]
            return None
        if name == "range":
            if len(args) == 1:
                a, b, st = 0, args[0], 1
            elif len(args) == 2:
                a, b, st = args[0], args[1], 1
            else:
                a, b, st = args[0], args[1], args[2]
            if st == 0:
                raise Stress("unfolded", "range step cannot be 0")
            out = []
            i = a
            while (st > 0 and i < b) or (st < 0 and i > b):
                out.append(i)
                i += st
                if len(out) > 10_000_000:
                    raise Stress("overflow", "range too large")
            return out
        if name == "str":
            return v_display(args[0]) if args else ""
        if name == "num":
            v = args[0] if args else None
            if isinstance(v, (int, float)) and not isinstance(v, bool):
                return v
            try:
                t = str(v).strip()
                return int(t)
            except ValueError:
                try:
                    return float(t)
                except (ValueError, UnboundLocalError):
                    self.note(4, f"num('{v}') failed; 0")
                    return 0
        if name == "type":
            if args and isinstance(args[0], ObjInst):
                return args[0].defn.name
            return type_name(args[0]) if args else "null"
        if name == "abs":
            a0 = args[0] if args else 0
            if isinstance(a0, int) and not isinstance(a0, bool) and a0 == -(2**63):
                raise Stress("overflow", "int overflow in abs(i64::MIN)")
            return abs(a0)
        if name in ("min", "max"):
            vals = []
            for a in args:
                if isinstance(a, list):
                    vals.extend(a)
                else:
                    vals.append(a)
            if not vals:
                return None
            return (min if name == "min" else max)(vals)
        if name == "sum":
            src = args[0] if len(args) == 1 and isinstance(args[0], list) else args
            acc = 0
            for v in src:
                acc = self.binop("+", acc, v)
            return acc
        if name == "clock":
            import time
            return time.monotonic()
        if name == "now":
            import time
            return time.monotonic()
        if name == "exit":
            sys.exit(int(args[0]) if args else 0)
        if name == "assert":
            ok = truthy(args[0]) if args else False
            if not ok:
                msg = v_display(args[1]) if len(args) > 1 else "assertion failed"
                raise Stress("burned", msg)
            return True
        if name == "codon":
            s = v_display(args[0]) if args else ""
            # mirror of the C++ kernel: 3-letter groups scored by usage class
            USAGE = [7,1,5,3,15,6,8,4,3,2,0,0,4,6,0,5,
                     4,1,2,13,5,2,4,1,6,2,12,9,5,3,9,4,
                     3,1,2,15,8,5,6,2,14,7,12,6,2,1,8,3,
                     6,2,8,1,8,3,6,10,4,2,13,11,6,7,3,9]
            def bidx(c):
                return {"t": 0, "u": 0, "c": 1, "a": 2, "g": 3}.get(c.lower(), -1)
            acc, groups = 0, 0
            s2 = v_display(args[0]) if args else ""
            i = 0
            while i + 2 < len(s2):
                b1, b2, b3 = bidx(s2[i]), bidx(s2[i+1]), bidx(s2[i+2])
                if b1 < 0 or b2 < 0 or b3 < 0:
                    # reg-r4 (re-audit B-2): NEUTRAL 50, mirroring the kernel
                    # (never reward biological nonsense, never punish ids)
                    acc += 50
                else:
                    w = USAGE[b1 * 16 + b2 * 4 + b3]
                    acc += 30 + w * 5
                groups += 1
                i += 3
            if groups == 0:
                return 0
            mean = acc // groups
            return int(min(mean, 100))
        if name == "distance":
            a = v_display(args[0]) if args else ""
            b = v_display(args[1]) if len(args) > 1 else ""
            return edit_distance(a, b)
        if name == "similar":
            a = v_display(args[0]) if args else ""
            b = v_display(args[1]) if len(args) > 1 else ""
            maxd = int(args[2]) if len(args) > 2 else 2
            return edit_distance(a, b) <= maxd
        if name == "transcribe":
            s = (v_display(args[0]) if args else "").upper()
            return s.replace("T", "U")
        if name == "reverse_complement":
            s = (v_display(args[0]) if args else "").upper()
            comp = {"A": "T", "T": "A", "G": "C", "C": "G"}
            return "".join(comp.get(c, c) for c in reversed(s))
        if name == "gc_content":
            s = (v_display(args[0]) if args else "").upper()
            n = sum(1 for c in s if c in "ATGC")
            if n == 0:
                return 0.0
            gc = sum(1 for c in s if c in "GC")
            return gc * 100.0 / n
        if name == "translate":
            s = (v_display(args[0]) if args else "").upper()
            TABLE = {}
            bases = "TCAG"
            aas = "FFLLSSSSYY**CC*WLLLLPPPPHHQQRRRRIIIMTTTTNNKKSSRRVVVVAAAADDEEGGGG"
            i = 0
            for b1 in bases:
                for b2 in bases:
                    for b3 in bases:
                        TABLE[b1 + b2 + b3] = aas[i]
                        i += 1
            # NOTE: this enumeration order is TTT,TTC,TTA,TTG,TCT... (b3 fastest)
            protein = []
            s2 = s.replace("U", "T")
            i = 0
            while i + 3 <= len(s2):
                codon = s2[i:i + 3]
                if codon in ("TAA", "TAG", "TGA"):
                    break
                aa = None
                for kk, vv in (("TTT","F"),("TTC","F"),("TTA","L"),("TTG","L"),
                               ("TCT","S"),("TCC","S"),("TCA","S"),("TCG","S"),
                               ("TAT","Y"),("TAC","Y"),("TAA","*"),("TAG","*"),
                               ("TGT","C"),("TGC","C"),("TGA","*"),("TGG","W"),
                               ("CTT","L"),("CTC","L"),("CTA","L"),("CTG","L"),
                               ("CCT","P"),("CCC","P"),("CCA","P"),("CCG","P"),
                               ("CAT","H"),("CAC","H"),("CAA","Q"),("CAG","Q"),
                               ("CGT","R"),("CGC","R"),("CGA","R"),("CGG","R"),
                               ("ATT","I"),("ATC","I"),("ATA","I"),("ATG","M"),
                               ("ACT","T"),("ACC","T"),("ACA","T"),("ACG","T"),
                               ("AAT","N"),("AAC","N"),("AAA","K"),("AAG","K"),
                               ("AGT","S"),("AGC","S"),("AGA","R"),("AGG","R"),
                               ("GTT","V"),("GTC","V"),("GTA","V"),("GTG","V"),
                               ("GCT","A"),("GCC","A"),("GCA","A"),("GCG","A"),
                               ("GAT","D"),("GAC","D"),("GAA","E"),("GAG","E"),
                               ("GGT","G"),("GGC","G"),("GGA","G"),("GGG","G")):
                    if kk == codon:
                        aa = vv
                        break
                protein.append(aa or "X")
                i += 3
            return "".join(protein)
        if name == "find_orf":
            s = (v_display(args[0]) if args else "").upper()
            out = []
            i = 0
            while i + 2 < len(s):
                if s[i:i+3] == "ATG":
                    j = i
                    protein = []
                    stopped = False
                    while j + 2 < len(s):
                        c2 = s[j:j+3]
                        if c2 in ("TAA", "TAG", "TGA"):
                            stopped = True
                            break
                        protein.append(self.builtin(env, "translate", [c2]) or "")
                        j += 3
                    if stopped and protein:
                        out.append("".join(protein))
                i += 1
            return out
        if name == "memory":
            import resource
            return {"arena_bytes": 0, "interns": 0, "allocs": 0}
        if name == "methyl":
            k = v_display(args[0]) if args else ""
            d = args[1] if len(args) > 1 else None
            v = self.cell.get(k)
            if v is None:
                return d
            if v == "true": return True
            if v == "false": return False
            try:
                return int(v)
            except ValueError:
                try:
                    return float(v)
                except ValueError:
                    return v
        if name == "splice_shift":
            # loop-9 (F-4): mirror of the Rust builtin — rebinds the splice
            # root to the named variant exactly like the splice statement.
            root = v_display(args[0]) if args else ""
            variant = v_display(args[1]) if len(args) > 1 else ""
            sp = self.splice_registry.get(root)
            if sp is None:
                self.note(4, f"splice_shift: no splice '{root}'; selection unchanged")
                return None
            variants = sp[2]
            if not any(v[0] == variant for v in variants):
                cur = self.choose_variant(root, variants)
                cur = cur[0] if cur else ""
                self.note(4, f"splice_shift: variant '{variant}' not declared in splice '{root}'; selection unchanged")
                return cur
            self.splice_shift[root] = variant
            vname, vparams, body, vmarks = self.choose_variant(root, variants)
            g = Gene(root, vparams, None, body,
                     ac="acetylate" in vmarks, me="methylate" in vmarks, m6="m6a" in vmarks)
            if root not in self.defined_genes:
                self.defined_genes.append(root)
            # trans-acting factor: replace the binding up the env chain
            self.assign(env, root, g)
            self.note(1, f"splice shift: '{root}' -> variant '{variant}' (was '{vname}')")
            return vname
        if name in ("m6a_write", "m6a_erase"):
            # reg-bio-3 (B3): quantitative m6A site density 0..=3 (mirror)
            k = v_display(args[0]) if args else ""
            n = args[1] if len(args) > 1 and isinstance(args[1], int) and not isinstance(args[1], bool) else 1
            n = max(n, 0)
            if name == "m6a_write":
                lvl = min(self.m6a_levels.get(k, 0) + n, 3)
                verb = "written"
            else:
                lvl = max(0, self.m6a_levels.get(k, 0) - n)
                verb = "erased"
            self.m6a_levels[k] = lvl
            marked = lvl >= 1
            if not self.methyl_quiet:
                self.note(2, f"m6A {verb}: '{k}' (level {lvl}) — redefinition {'resisted' if marked else 'no longer resisted'}")
            return lvl
        if name == "passage":
            # reg-bio-3 (B2/B6): cell divisions — marks dilute unless
            # maintained (mirror of the Rust passage builtin)
            n = args[0] if args and isinstance(args[0], int) and not isinstance(args[0], bool) else 1
            n = max(n, 0)
            if n > 1_000_000:
                self.note(4, "passage: n clamped to 1000000 divisions (a culture that old is not a useful model)")
                n = 1_000_000
            f = self.cell.get("methyl.maintenance")
            fv = 0.5
            if f is not None:
                try:
                    fv = min(max(float(f), 0.0), 1.0)
                except ValueError:
                    fv = 0.5
            import math as _math
            for _i in range(n):
                for k2 in list(self.methyl_levels):
                    x = self.methyl_levels[k2] * fv - 0.5
                    self.methyl_levels[k2] = max(0, int(_math.ceil(x)))
            # loop-9 (C8): the signal medium dilutes with the culture —
            # floor(m * d) per division, d = .cell quorum.dilution (default
            # 0.5, binary-exact halving). Empty medium = no-op (mirror).
            qd = self.cell.get("quorum.dilution")
            qdv = 0.5
            if qd is not None:
                try:
                    qdv = min(max(float(qd), 0.0), 1.0)
                except ValueError:
                    qdv = 0.5
            for _i in range(n):
                for k2 in list(self.medium):
                    self.medium[k2] = int(_math.floor(self.medium[k2] * qdv))
            self.generation += n
            if not self.methyl_quiet:
                self.note(1, f"passage: {n} divisions (maintenance {fv!r}) — generation {self.generation}")
            return self.generation
        if name == "methylate":
            # A12 (reg-r2): mirror of the Rust runtime API — same graded
            # semantics as the @methylate attribute (D-005): level += 1.
            k = v_display(args[0]) if args else ""
            self.methyl_levels[k] = self.methyl_levels.get(k, 0) + 1
            return self.methyl_levels[k]
        if name == "demethylate":
            # A12: the @acetylate direction — saturating relaxation.
            k = v_display(args[0]) if args else ""
            self.methyl_levels[k] = max(0, self.methyl_levels.get(k, 0) - 1)
            return self.methyl_levels[k]
        if name == "grn_set":
            # A12: write a GRN node's level directly (0..1, clamped).
            k = v_display(args[0]) if args else ""
            v = args[1] if len(args) > 1 else 0.0
            if not isinstance(v, float):
                v = float(v)
            v = min(max(v, 0.0), 1.0)
            self.grn_levels[k] = v
            return v
        if name == "grn_get":
            # A12: read a GRN node's level.
            k = v_display(args[0]) if args else ""
            return self.grn_levels.get(k, 0.0)
        if name == "fingerprint":
            # loop-9 (P0-3): parity fix — the Rust core measures COMPLETE
            # 20-call bins only (`call_clock / 20`); this mirror's former
            # `+ 1` included the trailing partial bin, so the same 41-call
            # history scored 2.439 on Rust and 0.053 here. Mirror the
            # complete-bin rule op-for-op (complete_bins == 0 lists the gene
            # at 0.0 but excludes it from the average, exactly like Rust).
            complete_bins = self.call_clock // 20
            burst_by = {}
            burst_total, burst_n = 0.0, 0
            # reg-bio-2 (D9): iterate buckets in SORTED key order — the Rust
            # core accumulates burst_total in sorted order (float addition is
            # not associative; order must match for the last-ulp parity).
            for gname in sorted(self.gene_buckets):
                bins = self.gene_buckets[gname]
                if complete_bins == 0:
                    burst_by[gname] = 0.0
                    continue
                n = float(complete_bins)
                total = sum(bins.get(b, 0) for b in range(complete_bins))
                mean = total / n
                # reg-bio-2 (D9): d*d (explicit multiply), never ** — op-for-op
                # with the Rust core's un-multiplied square.
                var = 0.0
                for b in range(complete_bins):
                    d = float(bins.get(b, 0)) - mean
                    var += d * d
                var /= n
                burst = (var / mean) if mean > 0 else 0.0
                burst_by[gname] = burst
                burst_total += burst
                burst_n += 1
            burst_avg = (burst_total / burst_n) if burst_n else 0.0
            total_defined = len(self.defined_genes)
            mature = min(len(self.call_counts), total_defined)
            nascent = max(total_defined - mature, 0)
            maturation = (mature / total_defined) if total_defined else 0.0
            # reg-bio-2 (D9): "calls" emits in sorted key order (Rust parity).
            # reg-bio-3 (A1/A7/B2): polycistronic transcript counts + division
            # counter (sorted-key determinism, D9) (mirror)
            transcripts = {u["name"]: u["transcripts"] for u in self.operons}
            return {"calls": dict(sorted(self.call_counts.items())), "burst": burst_avg,
                    "burst_by_gene": dict(sorted(burst_by.items())),
                    "bursts": dict(sorted(self.burst_off.items())),
                    "mature": mature, "nascent": nascent, "maturation": maturation,
                    "transcripts": dict(sorted(transcripts.items())),
                    "generation": self.generation}
        if name == "toggle_on":
            nm = v_display(args[0]) if args else ""
            for i, (a, b, _on) in enumerate(self.toggles):
                if nm in (a, b):
                    self.toggles[i] = (a, b, a == nm)
                    return True
            self.note(4, f"toggle pair containing '{nm}' not declared")
            return False
        if name == "toggle_state":
            out = {}
            for a, b, a_on in self.toggles:
                out[a] = a_on
                out[b] = not a_on
            return out
        if name == "repressi_next":
            if not self.repressi_ring:
                self.note(4, "no repressilator declared")
                return None
            self.repressi_i = (self.repressi_i + 1) % len(self.repressi_ring)
            self.repressi_tick += 1
            return self.repressi_i
        if name == "repressi_start":
            if not self.repressi_ring:
                self.note(4, "repressi_start: no repressilator ring declared")
                return False
            ms = 1000
            if args and isinstance(args[0], (int, float)):
                ms = int(max(args[0], 0))
            if ms == 0:
                self.note(4, "repressi_start period must be > 0 ms")
                return False
            self.note(1, f"repressilator oscillating every {ms} ms (sequential oracle: manual ring only)")
            return True
        if name == "repressi_state":
            # A11 (reg-r2) — mirror of the Rust core: the ring integrates
            # the discrete Elowitz–Leibler repressilator. State is a pure
            # function of the tick count; arithmetic order matches Rust
            # op-for-op (bit-identical IEEE-754).
            n = len(self.repressi_ring)
            if not n:
                return {}
            lvls = repressilator_levels(n, self.repressi_tick, self.repressi_params)
            return {nm: lvls[j] for j, nm in enumerate(self.repressi_ring)}
        if name == "grn_fire":
            seed = v_display(args[0]) if args else ""
            # A10 (reg-r2): decay mirror — decay comes from the pulse itself
            # (grn_fire(seed, f)) or falls back to .cell `[grn] decay`;
            # unset/0 skips the loop (byte-identical to pre-A10).
            raw_decay = args[1] if len(args) > 1 else None
            if raw_decay is not None:
                if not isinstance(raw_decay, float):
                    raw_decay = float(raw_decay)
                decay = min(max(raw_decay, 0.0), 1.0)
            else:
                decay = self.cell.get("grn.decay")
                try:
                    decay = min(max(float(decay), 0.0), 1.0) if decay is not None else 0.0
                except ValueError:
                    decay = 0.0
            if decay > 0.0 and self.grn_levels:
                retention = 1.0 - decay
                for k2 in self.grn_levels:
                    self.grn_levels[k2] *= retention
                    if self.grn_levels[k2] < 2.220446049250313e-16:  # f64::EPSILON
                        self.grn_levels[k2] = 0.0
            # reg-bio-2 (C1): the translation layer integrates at every
            # engine update point (mirror of the Rust grn_fire).
            self._trans_integrate()
            # STATEFUL network: levels persist across fires (a latch by default — decay makes the dilution explicit)
            # loop-9 (P0-2): ligand/signal-backed endpoints stay OUT of
            # grn_levels (they resolve through their own pools — seeding
            # them at 0.0 would shadow those reads forever) (mirror).
            for frm, to, st, inh, thr, hill, is_any, occupy, is_sum, attenuating in self.grn_edges:
                if frm not in self.ligands and frm not in self.signals:
                    self.grn_levels.setdefault(frm, 0.0)
                if to not in self.ligands and to not in self.signals:
                    self.grn_levels.setdefault(to, 0.0)
            # loop-9 (C8): firing a signal species directly would shadow the
            # medium read — the Rust core refuses with a note (mirror).
            if seed in self.signals:
                self.note(4, f"'{seed}' is a signal species: its level lives in the shared medium — use secrete()")
            else:
                self.grn_levels[seed] = min(self.grn_levels.get(seed, 0.0) + 1.0, 1.0)
            # reg-r3 (re-audit): TWO-PHASE fire, mirroring the Rust core and
            # the SPEC exactly — phase 1 propagates ACTIVATION in waves with
            # inhibitors excluded; phase 2 applies each inhibitor ONCE,
            # post-activation. (The old mirror subtracted inside the wave
            # loop — up to 10 times — diverging from the SPEC's "applied
            # exactly once per fire"; a pre-charged inhibited target shows
            # it: Rust yields 0.1, the old mirror yielded 0.0.)
            for wave in range(1, 11):
                changed = False
                snap = dict(self.grn_levels)
                for frm, to, st, inh, thr, hill, is_any, occupy, is_sum, attenuating in self.grn_edges:
                    if inh:
                        continue  # inhibitors propagate in phase 2
                    if is_sum:
                        continue  # pooled edges propagate as groups, below
                    parent = snap.get(frm, 0.0)
                    if parent <= 0:
                        continue
                    # reg-bio-2 (C11): propagation reads the FREE fraction
                    parent = self._regulated_level(parent, frm)
                    if thr is not None and thr > 0.0:
                        # reg-bio (F-2): per-edge Hill exponent (mirror of
                        # the Rust repeated multiplication — never **)
                        n_h = hill if hill is not None else 2
                        ph = 1.0
                        th2 = 1.0
                        for _k in range(n_h):
                            ph *= parent
                            th2 *= thr
                        influence = st * (ph / (ph + th2))
                    else:
                        # op-identical to the Rust repeated multiplication
                        # (never ** — pow vs mul rounding must not diverge)
                        s = st
                        for _ in range(wave - 1):
                            s *= st
                        influence = parent * s
                    cur = self.grn_levels.get(to, 0.0)
                    # reg-bio-2 (D2c): influence clamps at 1.0 (mirror)
                    nxt = max(cur, min(influence, 1.0))
                    if abs(nxt - cur) > 1e-12:
                        self.grn_levels[to] = nxt
                        changed = True
                # reg-bio-2 (B7): pooled (`sum`) edges — group pass per wave,
                # BEFORE the convergence check (mirror of the Rust core).
                groups = []
                for frm, to, st, inh, thr, hill, is_any, occupy, is_sum, attenuating in self.grn_edges:
                    if inh or not is_sum:
                        continue
                    if thr is None or thr <= 0.0:
                        continue
                    parent = snap.get(frm, 0.0)
                    parent = self._regulated_level(parent, frm)
                    n_h = hill if hill is not None else 2
                    for g in groups:
                        if g[0] == to and g[1] == thr and g[2] == n_h:
                            g[3] += st * parent
                            break
                    else:
                        groups.append([to, thr, n_h, st * parent])
                for to, thr, n_h, pooled in groups:
                    p = min(pooled, 1.0)
                    ph = 1.0
                    th2 = 1.0
                    for _k in range(n_h):
                        ph *= p
                        th2 *= thr
                    influence = ph / (ph + th2)
                    cur = self.grn_levels.get(to, 0.0)
                    nxt = max(cur, min(influence, 1.0))
                    if abs(nxt - cur) > 1e-12:
                        self.grn_levels[to] = nxt
                        changed = True
                if not changed:
                    break
            # phase 2: inhibition subtracts once, from the source's
            # post-activation level (sources read the PRE-phase-2 snapshot —
            # parity with the Rust `activated` map, so a node that is both a
            # target and a later source keeps its post-activation level)
            post = dict(self.grn_levels)
            for frm, to, st, inh, thr, hill, is_any, occupy, is_sum, attenuating in self.grn_edges:
                if not inh:
                    continue
                parent = post.get(frm, 0.0)
                if parent <= 0.0:
                    continue
                # reg-bio-2 (C11): inhibition also reads the FREE fraction
                parent = self._regulated_level(parent, frm)
                if thr is not None and thr > 0.0:
                    # reg-bio (F-2): cooperative repressor influence
                    n_h = hill if hill is not None else 2
                    ph = 1.0
                    th2 = 1.0
                    for _k in range(n_h):
                        ph *= parent
                        th2 *= thr
                    influence = st * (ph / (ph + th2))
                else:
                    influence = parent * st
                cur = self.grn_levels.get(to, 0.0)
                # reg-bio-2 (D2b): occupancy repression — multiplicative
                # survival (mirror of the Rust core); legacy subtracts once.
                if occupy:
                    self.grn_levels[to] = cur * (1.0 - min(influence, 1.0))
                else:
                    self.grn_levels[to] = max(0.0, cur - influence)
            return dict(sorted(self.grn_levels.items()))
        if name == "grn_state":
            # reg-bio-2 (D9): sorted key order — Rust HashMap order varies per
            # process; both implementations now emit byte-order sorted maps.
            return dict(sorted(self.grn_levels.items()))
        if name == "items":
            v = args[0] if args else {}
            if isinstance(v, dict):
                return [[k, val] for k, val in v.items()]
            return []
        if name == "spawn":
            callee = args[0] if args else None
            targs = args[1] if len(args) > 1 and isinstance(args[1], list) else []
            if isinstance(callee, Gene):
                # reg-r4: SendValue depth ceiling mirror (Rust SEND_DEPTH_CAP
                # = 100_000) — deep spawn payloads raise a catchable
                # `overflow` stress exactly like the Rust serialization path
                for _a in targs:
                    _stack = [(_a, 1)]
                    while _stack:
                        _item, _d = _stack.pop()
                        if _d > 100_000:
                            raise Stress("overflow", "spawn payload exceeds the depth ceiling")
                        if isinstance(_item, list):
                            _stack.extend((x, _d + 1) for x in _item)
                        elif isinstance(_item, dict):
                            _stack.extend((x, _d + 1) for x in _item.values())
                # reg-r4 (re-audit B-2): route through call_named like the
                # Rust worker does — a direct call_gene bypasses the
                # toggle/RISC/GRN/methyl gates ("a repressed allele stays
                # repressed in worker cells", reg-r1's own contract)
                # reg-bio-2 (C3): worker RNG decorrelation mirror — the Rust
                # worker derives its stream from the task id; the sequential
                # oracle runs the body inline but saves / derives / restores
                # the host stream, so stochastic worker bodies draw from the
                # identical derived sequence bit-for-bit.
                _saved_rng = self.rng
                _task_id = getattr(self, "next_id", 0) + 1
                _derived = (0x9E3779B97F4A7C15 ^ ((_task_id * 0x9E3779B97F4A7C15) & M64)) & M64
                self.rng = _derived
                spawn_name = callee.name
                # loop-9 (C8): the Rust worker tags its notes "[task <name>]"
                # — mirror the prefix while the body runs inline
                _saved_prefix = getattr(self, "task_note_prefix", None)
                self.task_note_prefix = f"[task {spawn_name or '<lambda>'}]"
                # loop-9: worker-cell isolation (mirror of the snapshot
                # semantics) — the Rust worker gets a COPY of regulation
                # state and FRESH expression counters; mutations inside the
                # cell never propagate to the host. The sequential oracle
                # must save/restore everything the snapshot carries.
                _saved_state = {
                    "grn_edges": list(self.grn_edges),
                    "grn_levels": dict(self.grn_levels),
                    "toggles": list(self.toggles),
                    "methyl_levels": dict(self.methyl_levels),
                    "methyl_threshold": self.methyl_threshold,
                    "enhanced": list(self.enhanced),
                    "repressi_ring": list(self.repressi_ring),
                    "repressi_tick": self.repressi_tick,
                    "repressi_params": dict(self.repressi_params),
                    "promoter_states": dict(self.promoter_states),
                    "burst_off": dict(self.burst_off),
                    "expr_stochastic": self.expr_stochastic,
                    "expr_kon": self.expr_kon,
                    "expr_koff": self.expr_koff,
                    "decay_clock_n": self.decay_clock_n,
                    "decay_clock_f": self.decay_clock_f,
                    "trans_edges": list(self.trans_edges),
                    "trans_last": dict(self.trans_last),
                    "decoys": list(self.decoys),
                    "ligands": list(self.ligands),
                    "ligand_pools": dict(self.ligand_pools),
                    "grn_binds": list(self.grn_binds),
                    "silences": list(self.silences),
                    "risc_escaped": set(self.risc_escaped),
                    "operons": [dict(u) for u in self.operons],
                    "m6a_levels": dict(self.m6a_levels),
                    "generation": self.generation,
                    "copies": dict(self.copies),
                    "signals": list(self.signals),
                    "splice_shift": dict(self.splice_shift),
                    "promoter_tel": dict(self.promoter_tel),
                    "burst_overrides": dict(self.burst_overrides),
                    "rho_pins": self.rho_pins,
                    "ribo_queue": dict(self.ribo_queue),
                    "call_counts": dict(self.call_counts),
                    "call_clock": self.call_clock,
                    "gene_buckets": {k: dict(v) for k, v in self.gene_buckets.items()},
                    "defined_genes": list(self.defined_genes),
                    "methyl_noted": set(self.methyl_noted),
                }
                # the worker starts with FRESH expression counters (its own
                # call clock starts at zero)
                self.call_counts = {}
                self.call_clock = 0
                self.gene_buckets = {}
                try:
                    if spawn_name:
                        result = self.call_named(env, spawn_name, targs)
                    else:
                        result = self.call_value(env, callee, targs)
                except Stress as st:
                    # W06 (D-014) mirror: a propagated variant IS the worker
                    # gene's return value — converted at the boundary.
                    if st.prop is not None:
                        result = st.prop
                    else:
                        result = {"kind": st.kind, "message": st.message}
                finally:
                    for k2, v2 in _saved_state.items():
                        setattr(self, k2, v2)
                    self.rng = _saved_rng
                    self.task_note_prefix = _saved_prefix
                self.next_id = _task_id
                self.tasks = getattr(self, "tasks", {})
                self.tasks[self.next_id] = result
                return self.next_id
            self.note(4, "spawn() needs a gene; null task")
            return None
        if name == "join":
            tid = args[0] if args else None
            tasks = getattr(self, "tasks", {})
            if isinstance(tid, int) and tid in tasks:
                return tasks.pop(tid)
            self.note(4, f"task {tid} already joined or unknown")
            return None
        # ---- math
        if name == "floor":
            v = args[0] if args else 0
            if isinstance(v, float):
                import math as _m
                if not _m.isfinite(v) or v >= 9.223372036854776e18 or v <= -9.223372036854776e18:
                    raise Stress("overflow", "float too large for floor/ceil to int")
                return int(_m.floor(v))
            return v if isinstance(v, int) else 0
        if name == "ceil":
            v = args[0] if args else 0
            if isinstance(v, float):
                import math as _m
                if not _m.isfinite(v) or v >= 9.223372036854776e18 or v <= -9.223372036854776e18:
                    raise Stress("overflow", "float too large for floor/ceil to int")
                return int(_m.ceil(v))
            return v if isinstance(v, int) else 0
        if name == "sqrt":
            v = float(args[0]) if args else 0.0
            if v < 0:
                raise Stress("unfolded", "sqrt of negative number")
            return v ** 0.5
        if name == "pow":
            a = float(args[0]) if len(args) > 0 else 0.0
            b = float(args[1]) if len(args) > 1 else 0.0
            return a ** b
        if name in ("re_match", "re_find", "re_groups"):
            import re as _re
            pat = args[0] if args else ""
            subj = args[1] if len(args) > 1 else ""
            try:
                rx = _re.compile(pat)
            except _re.error as e:
                raise Stress("unfolded", f"regex: {e}")
            try:
                if name == "re_match":
                    # prefix match (anchored at 0), mirroring the Rust core
                    return rx.match(subj) is not None
                start = int(args[2]) if len(args) > 2 else 0
                m = rx.search(subj, start)
                if m is None:
                    return None
                if name == "re_groups":
                    return [g if g is not None else None for g in m.groups()]
                return {"text": m.group(0), "start": m.start(), "end": m.end(),
                        "groups": [g if g is not None else None for g in m.groups()]}
            except _re.error as e:
                raise Stress("unfolded", f"regex: {e}")
            except RecursionError:
                raise Stress("overflow", "regex backtracking exceeded 2M steps")
        if name == "re_replace":
            # dx-r6 mirror of the Rust implementation: global literal
            # substitution (empty patterns behave like Python re.sub)
            import re as _re
            pat = args[0] if args else ""
            subj = args[1] if len(args) > 1 else ""
            repl = args[2] if len(args) > 2 else ""
            try:
                rx = _re.compile(pat)
            except _re.error as e:
                raise Stress("unfolded", f"regex: {e}")
            # literal replacement: a callable repl treats the text as data
            # (no \1 backref interpretation — mirrors the Rust implementation)
            return rx.sub(lambda _m: repl, subj)
        if name == "unix_time":
            import time as _t
            return int(_t.time())
        if name == "date_parts":
            import time as _t
            ts = int(args[0]) if args else 0
            st = _t.gmtime(ts)
            return {"year": st.tm_year, "month": st.tm_mon, "day": st.tm_mday,
                    "hour": st.tm_hour, "min": st.tm_min, "sec": st.tm_sec,
                    "wday": (st.tm_wday + 1) % 7}
        if name == "date_fmt":
            import time as _t
            ts = int(args[0]) if args else 0
            fmt = args[1] if len(args) > 1 else "%Y-%m-%d %H:%M:%S"
            st = _t.gmtime(ts)
            out = []
            i = 0
            while i < len(fmt):
                if fmt[i] == "%" and i + 1 < len(fmt):
                    c = fmt[i + 1]
                    out.append({"Y": f"{st.tm_year:04d}", "m": f"{st.tm_mon:02d}",
                                "d": f"{st.tm_mday:02d}", "H": f"{st.tm_hour:02d}",
                                "M": f"{st.tm_min:02d}", "S": f"{st.tm_sec:02d}",
                                "%": "%"}.get(c, "%" + c))
                    i += 2
                else:
                    out.append(fmt[i])
                    i += 1
            return "".join(out)
        # ------------------------------------------------ Option / Result (W06, D-014)
        # Constructors + predicates + extraction. Error messages byte-match
        # the Rust core (Stress::at texts); families are distinct.
        if name == "some":
            if len(args) != 1:
                raise Stress("unfolded", "some(v) needs exactly 1 argument")
            return Variant("Some", args[0])
        if name == "none":
            if args:
                raise Stress("unfolded", "none() takes no arguments")
            return Variant("None", None)
        if name == "ok":
            if len(args) != 1:
                raise Stress("unfolded", "ok(v) needs exactly 1 argument")
            return Variant("Ok", args[0])
        if name == "err":
            if len(args) != 1:
                raise Stress("unfolded", "err(e) needs exactly 1 argument")
            return Variant("Err", args[0])
        if name == "is_some":
            return isinstance(args[0] if args else None, Variant) and args[0].tag == "Some"
        if name == "is_none":
            return isinstance(args[0] if args else None, Variant) and args[0].tag == "None"
        if name == "is_ok":
            return isinstance(args[0] if args else None, Variant) and args[0].tag == "Ok"
        if name == "is_err":
            return isinstance(args[0] if args else None, Variant) and args[0].tag == "Err"
        if name == "unwrap_or":
            if len(args) != 2:
                raise Stress("unfolded", "unwrap_or(v, default) needs exactly 2 arguments")
            v = args[0]
            if isinstance(v, Variant) and v.tag in ("Some", "Ok") and v.payload is not None:
                return v.payload
            return args[1]
        if name == "unwrap":
            if len(args) != 1:
                raise Stress("unfolded", "unwrap(v) needs exactly 1 argument")
            v = args[0]
            if isinstance(v, Variant):
                if v.tag in ("Some", "Ok") and v.payload is not None:
                    return v.payload
                raise Stress("unwrap", f"unwrap on {v.tag}")
            raise Stress("unwrap", f"unwrap on a plain {type_name(v)} value")
        if name == "random":
            x = self.rng
            x ^= (x >> 12) & 0xFFFFFFFFFFFFFFFF
            x ^= (x << 25) & 0xFFFFFFFFFFFFFFFF
            x ^= (x >> 27) & 0xFFFFFFFFFFFFFFFF
            self.rng = x & 0xFFFFFFFFFFFFFFFF
            x = self.rng
            if args and isinstance(args[0], int) and not isinstance(args[0], bool) and args[0] > 0:
                r = (x * 0x2545F4914F6CDD1D) & 0xFFFFFFFFFFFFFFFF
                return r % args[0]
            return ((x >> 11) & ((1 << 53) - 1)) / 9007199254740992.0
        if name == "randomize":
            s = 0x9E3779B97F4A7C15
            if args and isinstance(args[0], (int, float)) and not isinstance(args[0], bool):
                s = int(args[0]) & 0xFFFFFFFFFFFFFFFF
            self.rng = s if s != 0 else 0x9E3779B97F4A7C15
            return None
        if name == "promoter_telemetry":
            # loop-9 (F-3): per-gene promoter attempt telemetry (mirror)
            nm = v_display(args[0]) if args else ""
            attempts, on_total, episodes = self.promoter_tel.get(nm, (0, 0, 0))
            on_frac = (on_total / attempts) if attempts > 0 else 0.0
            burst_size = (on_total / episodes) if episodes > 0 else 0.0
            return {"attempts": attempts, "on_total": on_total, "episodes": episodes,
                    "on_frac": on_frac, "burst_size": burst_size}
        if name == "burst_set":
            # loop-9 (R9): runtime promoter-rate modulation (mirror)
            nm = v_display(args[0]) if args else ""
            if len(args) >= 3 and isinstance(args[1], (int, float)) and isinstance(args[2], (int, float)):
                kon = min(max(float(args[1]), 0.0), 1.0)
                koff = min(max(float(args[2]), 0.0), 1.0)
                self.burst_overrides[nm] = (kon, koff)
                if not self.methyl_quiet:
                    self.note(1, f"burst override: '{nm}' (kon={kon!r}, koff={koff!r})")
                return {"kon": kon, "koff": koff}
            self.burst_overrides.pop(nm, None)
            if not self.methyl_quiet:
                self.note(1, f"burst override cleared: '{nm}'")
            return None
        if name == "expr_on":
            # reg-bio (F-1): mirror of the Rust builtin — in-source switch for
            # the telegraph promoter layer (kon/koff clamped 0..1)
            kon = 0.3
            koff = 0.1
            if args and isinstance(args[0], (int, float)) and not isinstance(args[0], bool):
                kon = min(max(float(args[0]), 0.0), 1.0)
            if len(args) > 1 and isinstance(args[1], (int, float)) and not isinstance(args[1], bool):
                koff = min(max(float(args[1]), 0.0), 1.0)
            self.expr_stochastic = True
            self.expr_kon = kon
            self.expr_koff = koff
            self.note(1, f"telegraph promoter on (kon={kon}, koff={koff})")
            return True
        if name == "expr_off":
            self.expr_stochastic = False
            return False
        if name == "decay_clock":
            # reg-bio-2 (C2): mirror of the Rust builtin — in-source switch
            # for the decay clock (interval n, optional fraction f)
            n = None
            if args and isinstance(args[0], (int, float)) and not isinstance(args[0], bool) and args[0] > 0:
                n = int(args[0])
            frac = None
            if len(args) > 1 and isinstance(args[1], (int, float)) and not isinstance(args[1], bool):
                frac = min(max(float(args[1]), 0.0), 1.0)
            self.decay_clock_n = n
            self.decay_clock_f = frac
            if n is not None:
                self.note(1, f"decay clock on (one decay step every {n} calls)")
                return True
            self.note(1, "decay clock off (event-driven)")
            return False
        # ---- ligands (reg-bio-2 A4)
        if name == "ligand_set":
            nm = v_display(args[0]) if args else ""
            v = 0.0
            if len(args) > 1 and isinstance(args[1], (int, float)) and not isinstance(args[1], bool):
                v = min(max(float(args[1]), 0.0), 1.0)
            if nm not in self.ligands:
                self.ligands.append(nm)
            self.ligand_pools[nm] = v
            self.note(1, f"ligand pool '{nm}': {v}")
            return v
        if name == "ligand":
            nm = v_display(args[0]) if args else ""
            return self._ligand_level(nm)
        if name == "secrete":
            # loop-9 (C8): mirror of the Rust secrete — Int exact, Float
            # floors (never rounds), negative clamps to 0 with a note,
            # non-finite clamps to 0, saturating add capped at 1e9.
            nm = v_display(args[0]) if args else ""
            amt = 1
            if len(args) > 1:
                a = args[1]
                if isinstance(a, bool):
                    amt = 1
                elif isinstance(a, int):
                    if a < 0:
                        self.note(4, f"secrete: negative amount clamps to 0 molecules ('{nm}')")
                    amt = max(0, min(a, 1_000_000_000))
                elif isinstance(a, float):
                    if a != a or a in (float("inf"), float("-inf")):
                        self.note(4, f"secrete: non-finite amount clamps to 0 molecules ('{nm}')")
                        amt = 0
                    else:
                        if a < 0.0:
                            self.note(4, f"secrete: negative amount floors to 0 molecules ('{nm}')")
                        amt = int(min(max(math.floor(a), 0.0), 1e9))
                else:
                    amt = 1
            committed = 0
            if self._signal_register(nm):
                cur = self.medium.get(nm, 0)
                nxt = min(cur + amt, 1_000_000_000)
                self.medium[nm] = nxt
                committed = nxt - cur
            self.note(1, f"secrete '{nm}': +{committed} molecules (level {fmt_float(committed / 1e9)})")
            return committed
        if name == "quorum":
            # loop-9 (C8): mirror — one arg -> level; two args -> level >= t.
            nm = v_display(args[0]) if args else ""
            lvl = self._signal_level(nm)
            if len(args) > 1:
                t = args[1]
                if isinstance(t, bool):
                    t = 0.0
                elif isinstance(t, (int, float)):
                    t = float(t)
                else:
                    t = 0.0
                return lvl >= t
            return lvl
        if name == "quench":
            # loop-9 (C8): mirror — m <- floor(m * (1-f)); no arg destroys all.
            nm = v_display(args[0]) if args else ""
            f = 1.0
            if len(args) > 1:
                a = args[1]
                if isinstance(a, bool):
                    f = 1.0
                elif isinstance(a, (int, float)):
                    f = min(max(float(a), 0.0), 1.0)
                else:
                    f = 1.0
            cur = self.medium.get(nm, 0)
            nxt = int(math.floor(cur * (1.0 - f)))
            self.medium[nm] = nxt
            removed = cur - nxt
            self.note(1, f"quench '{nm}': -{removed} molecules")
            return removed
        if name == "quorum_state":
            # loop-9 (C8): mirror — sorted-key species -> molecule counts.
            return {k: self.medium[k] for k in sorted(self.medium)}
        if name == "chr":
            i = args[0] if args and isinstance(args[0], int) else 0
            return chr(i) if 0 <= i <= 0x10FFFF else ""
        if name == "ord":
            s = args[0] if args and isinstance(args[0], str) else ""
            return ord(s[0]) if s else 0
        if name == "sleep":
            import time as _t
            ms = 0
            if args and isinstance(args[0], (int, float)):
                ms = max(args[0], 0)
            _t.sleep(ms / 1000.0)
            return None
        if name == "argv":
            return list(self.cli_args)
        # ---- filesystem (capability-gated)
        if name == "read_file":
            path = v_display(args[0]) if args else ""
            self.cap_check("read", "read", path)
            try:
                with open(path, "r", encoding="utf-8", errors="strict") as f:
                    return f.read()
            except OSError as e:
                raise Stress("missing", f"read_file '{path}': {e}")
        if name == "write_file":
            path = v_display(args[0]) if args else ""
            body = v_display(args[1]) if len(args) > 1 else ""
            self.cap_check("write", "write", path)
            try:
                with open(path, "w", encoding="utf-8") as f:
                    f.write(body)
                return True
            except OSError as e:
                raise Stress("missing", f"write_file '{path}': {e}")
        if name == "append_file":
            path = v_display(args[0]) if args else ""
            body = v_display(args[1]) if len(args) > 1 else ""
            self.cap_check("write", "write", path)
            try:
                with open(path, "a", encoding="utf-8") as f:
                    f.write(body)
                return True
            except OSError as e:
                raise Stress("missing", f"append_file '{path}': {e}")
        if name == "exists":
            path = v_display(args[0]) if args else ""
            self.cap_check("read", "read", path)
            return os.path.exists(path)
        if name == "file_size":
            path = v_display(args[0]) if args else ""
            self.cap_check("read", "read", path)
            try:
                return os.path.getsize(path)
            except OSError:
                return -1
        if name == "read_dir":
            path = v_display(args[0]) if args else ""
            self.cap_check("read", "read", path)
            try:
                return sorted(os.listdir(path))
            except OSError as e:
                raise Stress("missing", f"read_dir '{path}': {e}")
        # ---- dx-r6: fs mutate ops (mirror of the Rust capability discipline)
        if name == "fs_delete":
            path = v_display(args[0]) if args else ""
            self.cap_check("write", "write", path)
            if os.path.islink(path):
                raise Stress("interference", f"fs_delete '{path}': refused — path is a symlink")
            try:
                if os.path.isdir(path):
                    os.rmdir(path)  # empty dirs only: no recursive bombs
                else:
                    os.remove(path)
                return True
            except OSError as e:
                raise Stress("missing", f"fs_delete '{path}': {e}")
        if name == "fs_rename":
            src = v_display(args[0]) if args else ""
            dst = v_display(args[1]) if len(args) > 1 else ""
            self.cap_check("write", "write", src)
            self.cap_check("write", "write", dst)
            try:
                os.rename(src, dst)
                return True
            except OSError as e:
                raise Stress("missing", f"fs_rename '{src}' -> '{dst}': {e}")
        if name == "fs_mkdir":
            path = v_display(args[0]) if args else ""
            self.cap_check("write", "write", path)
            try:
                os.makedirs(path, exist_ok=True)
                return True
            except OSError as e:
                raise Stress("missing", f"fs_mkdir '{path}': {e}")
        # ---- process / net (capability-gated)
        if name == "run":
            prog = v_display(args[0]) if args else ""
            pargs = [v_display(x) for x in args[1]] if len(args) > 1 and isinstance(args[1], list) else []
            self.cap_check("run", "run", prog)
            import subprocess as _sp
            try:
                cp = _sp.run([prog] + pargs, capture_output=True, text=True)
                return {"code": cp.returncode, "stdout": cp.stdout, "stderr": cp.stderr, "ok": cp.returncode == 0}
            except OSError as e:
                raise Stress("missing", f"run '{prog}': {e}")
        if name == "http_get":
            host = v_display(args[0]) if args else ""
            port = int(args[1]) if len(args) > 1 and isinstance(args[1], int) else 80
            path = v_display(args[2]) if len(args) > 2 else "/"
            self.cap_check("net", "net", f"{host}:{port}")
            import socket as _s
            try:
                s = _s.create_connection((host, port), timeout=10)
                s.sendall(f"GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: operon\r\n\r\n".encode())
                buf = b""
                while True:
                    chunk = s.recv(65536)
                    if not chunk:
                        break
                    buf += chunk
                s.close()
                pos = buf.find(b"\r\n\r\n")
                return buf[pos + 4:].decode("utf-8", "replace") if pos >= 0 else buf.decode("utf-8", "replace")
            except OSError as e:
                raise Stress("missing", f"http_get '{host}:{port}': {e}")
        if name in ("serve", "recv_request", "send_response"):
            # the sequential oracle does not host a server; these are only
            # reachable in the Rust core (differential corpus avoids them)
            if name == "serve":
                port = int(args[0]) if args and isinstance(args[0], int) else 8080
                self.cap_check("net", "net", f"127.0.0.1:{port}")
            self.note(4, f"{name} is not available in the sequential oracle; null")
            return None
        # ---- json
        if name == "json_parse":
            s = v_display(args[0]) if args else "null"
            try:
                return _json.loads(s, object_pairs_hook=lambda pairs: dict(pairs))
            except ValueError as e:
                raise Stress("unfolded", f"json_parse: {e}")
        if name == "json_str":
            v = args[0] if args else None
            return self._json_str(v)
        # ---- env (capability-gated)
        if name == "env":
            nm = v_display(args[0]) if args else ""
            self.cap_check("env", "env", nm)
            return os.environ.get(nm)
        # ---- py (substrate-r1): the Python ecosystem bridge. The oracle
        # runs in-process (it IS python); the ok/value/error/code contract
        # matches the Rust bridge. Timeout/env-scrub containment is not
        # mirrored here — the granted suite that exercises those paths is
        # Rust-runner-only, and the differential corpus avoids py effects.
        if name == "py":
            module = v_display(args[0]) if args else ""
            func = v_display(args[1]) if len(args) > 1 else ""
            call_args = args[2] if len(args) > 2 and isinstance(args[2], list) else []
            if not module:
                raise Stress("missing", "py: module name required")
            self.cap_check("py", "py", module)
            import importlib as _il
            try:
                mod = _il.import_module(module)
                obj = mod
                for part in str(func or "").split("."):
                    if part:
                        obj = getattr(obj, part)
                val = obj(*call_args)
                return {"ok": True, "value": val, "error": None, "code": 0}
            except BaseException as e:
                import traceback as _tb
                msg = "".join(_tb.format_exception_only(type(e), e)).strip()
                return {"ok": False, "value": None, "error": msg, "code": 1}
        if name == "call":
            # dynamic dispatch: call(name_or_gene, args_list)
            target = args[0] if args else None
            call_args = args[1] if len(args) > 1 and isinstance(args[1], list) else []
            if isinstance(target, str):
                if target in BUILTINS or target in BUILTIN_SYNONYMS:
                    return self.builtin(env, BUILTIN_SYNONYMS.get(target, target), call_args)
                tv = self.lookup(env, target)
                return self.call_value(env, tv, call_args)
            return self.call_value(env, target, call_args)
        # ---- L1a: iteration + numeric builtins (mirror src/interp.rs) ----
        if name == "enumerate":
            v = args[0] if args else None
            if isinstance(v, list):
                return [[i, x] for i, x in enumerate(v)]
            if isinstance(v, str):
                return [[i, c] for i, c in enumerate(v)]
            self.note(4, f"enumerate of a {type_name(v)}; null")
            return None
        if name == "zip":
            a = args[0] if args else None
            b = args[1] if len(args) > 1 else None
            if not isinstance(a, list) or not isinstance(b, list):
                self.note(4, "zip needs two lists; null")
                return None
            return [[a[i], b[i]] for i in range(min(len(a), len(b)))]
        if name == "sorted":
            v = args[0] if args else None
            if not isinstance(v, list):
                self.note(4, f"sorted of a {type_name(v)}; null")
                return None
            out = list(v)
            if len(args) > 1 and isinstance(args[1], Gene):
                cmp = args[1]
                for i in range(1, len(out)):
                    j = i
                    while j > 0:
                        before = truthy(self.call_value(env, cmp, [out[j - 1], out[j]]))
                        if not before:
                            out[j - 1], out[j] = out[j], out[j - 1]
                            j -= 1
                        else:
                            break
            else:
                # identical key to the .sort() method — one ordering contract
                out.sort(key=lambda x: (
                    (0, float(x), "") if isinstance(x, (int, float)) and not isinstance(x, bool)
                    else ((0, float(int(x)), "") if isinstance(x, bool)
                          else ((1, 0.0, x) if isinstance(x, str)
                                else (2, 0.0, repr_of(x))))
                ))
            return out
        if name == "reversed":
            v = args[0] if args else None
            if isinstance(v, list):
                return list(reversed(v))
            self.note(4, f"reversed of a {type_name(v)}; null")
            return None
        if name == "any":
            v = args[0] if args else None
            if isinstance(v, list):
                return any(truthy(x) for x in v)
            self.note(4, f"any of a {type_name(v)}; false")
            return False
        if name == "all":
            v = args[0] if args else None
            if isinstance(v, list):
                return all(truthy(x) for x in v)
            self.note(4, f"all of a {type_name(v)}; false")
            return False
        if name == "first":
            v = args[0] if args else None
            if isinstance(v, list):
                if not v:
                    self.note(4, "first of an empty list; null")
                    return None
                return v[0]
            if isinstance(v, str):
                if not v:
                    self.note(4, "first of an empty string; null")
                    return None
                return v[0]
            self.note(4, f"first of a {type_name(v)}; null")
            return None
        if name == "last":
            v = args[0] if args else None
            if isinstance(v, list):
                if not v:
                    self.note(4, "last of an empty list; null")
                    return None
                return v[-1]
            if isinstance(v, str):
                if not v:
                    self.note(4, "last of an empty string; null")
                    return None
                return v[-1]
            self.note(4, f"last of a {type_name(v)}; null")
            return None
        if name == "take":
            v = args[0] if args else None
            n = args[1] if len(args) > 1 else None
            if not isinstance(n, int) or isinstance(n, bool):
                self.note(4, "take needs a count; null")
                return None
            if n < 0:
                n = 0
            if isinstance(v, list):
                return v[:n]
            if isinstance(v, str):
                return v[:n]
            self.note(4, f"take of a {type_name(v)}; null")
            return None
        if name == "drop":
            v = args[0] if args else None
            n = args[1] if len(args) > 1 else None
            if not isinstance(n, int) or isinstance(n, bool):
                self.note(4, "drop needs a count; null")
                return None
            if n < 0:
                n = 0
            if isinstance(v, list):
                return v[n:]
            if isinstance(v, str):
                return v[n:]
            self.note(4, f"drop of a {type_name(v)}; null")
            return None
        if name == "unique":
            v = args[0] if args else None
            if isinstance(v, list):
                out = []
                for x in v:
                    if not any(deep_eq(u, x) for u in out):
                        out.append(x)
                return out
            self.note(4, f"unique of a {type_name(v)}; null")
            return None
        if name == "flatten":
            v = args[0] if args else None
            if isinstance(v, list):
                out = []
                for x in v:
                    if isinstance(x, list):
                        out.extend(x)
                    else:
                        out.append(x)
                return out
            self.note(4, f"flatten of a {type_name(v)}; null")
            return None
        if name == "chunk":
            v = args[0] if args else None
            n = args[1] if len(args) > 1 else None
            if not isinstance(n, int) or isinstance(n, bool):
                self.note(4, "chunk needs a size; null")
                return None
            if n <= 0:
                self.note(4, "chunk size must be positive; empty")
                return []
            if isinstance(v, list):
                return [v[i:i + n] for i in range(0, len(v), n)]
            self.note(4, f"chunk of a {type_name(v)}; null")
            return None
        if name == "round":
            v = args[0] if args else None
            d = args[1] if len(args) > 1 else 0
            if not isinstance(d, int) or isinstance(d, bool):
                self.note(4, "digit count must be an int; treated as 0")
                d = 0
            if d < 0:
                self.note(4, "negative digit count; treated as 0")
                d = 0
            if isinstance(v, bool):
                self.note(4, f"round of a {type_name(v)}; 0")
                return 0
            if isinstance(v, int):
                return v
            if isinstance(v, float):
                if v != v or v in (float("inf"), float("-inf")):
                    return v
                scale = 10.0 ** d
                r = (abs(v) * scale + 0.5) // 1 / scale
                r = r if v >= 0 else -r
                if d == 0:
                    if not (-(2**63) <= r <= 2**63 - 1):
                        raise Stress("overflow", "float too large for round to int")
                    return int(r)
                return r
            self.note(4, f"round of a {type_name(v)}; 0")
            return 0
        if name == "clamp":
            v = args[0] if args else None
            lo = args[1] if len(args) > 1 else None
            hi = args[2] if len(args) > 2 else None
            nums = (int, float)
            if not (isinstance(v, nums) and not isinstance(v, bool)
                    and isinstance(lo, nums) and not isinstance(lo, bool)
                    and isinstance(hi, nums) and not isinstance(hi, bool)):
                self.note(4, "clamp needs three numbers; null")
                return None
            if v < lo:
                return lo
            if v > hi:
                return hi
            return v
        if name == "divmod":
            # parity by construction: reuses the // and % operators
            a = args[0] if args else None
            b = args[1] if len(args) > 1 else None
            q = self.binop("//", a, b)
            r = self.binop("%", a, b)
            return [q, r]
        self.note(4, f"unknown builtin '{name}'; null")
        return None

    @staticmethod
    def _json_str(v, _seen=None, _depth=0):
        # reg-r4: cycle-safe — repeated branch serializes as null, depth
        # cap 512 (mirror of the Rust json_stringify_g)
        if _seen is None:
            _seen = set()
        if _depth > 512:
            return "null"
        if v is None: return "null"
        if v is True: return "true"
        if v is False: return "false"
        if isinstance(v, int): return str(v)
        if isinstance(v, float): return fmt_float(v)
        if isinstance(v, str): return _json.dumps(v)
        if isinstance(v, (list, dict)):
            marker = id(v)
            if marker in _seen:
                return "null"
            _seen.add(marker)
            if isinstance(v, list):
                out = "[" + ",".join(Interp._json_str(x, _seen, _depth + 1) for x in v) + "]"
            else:
                out = "{" + ",".join(
                    _json.dumps(str(k) if not isinstance(k, str) else k) + ":"
                    + Interp._json_str(val, _seen, _depth + 1)
                    for k, val in v.items()) + "}"
            _seen.discard(marker)
            return out
        # W06 (D-014) mirror: variants serialize as single-key objects —
        # {"some": v} / {"ok": v} / {"err": v}; None serializes as null.
        # Matches the Rust json_stringify_g Variant arms byte-for-byte.
        if isinstance(v, Variant):
            if v.tag == "None" or v.payload is None:
                return "null"
            key = {"Some": "some", "Ok": "ok", "Err": "err"}.get(v.tag, "none")
            return "{" + _json.dumps(key) + ":" + Interp._json_str(v.payload, _seen, _depth + 1) + "}"
        return _json.dumps(v_display(v))

    def construct_obj(self, p, args):
        chain = []
        d = p
        hops = 0
        while d is not None and hops <= 32:
            chain.append(d)
            d = self.phenos.get(d.parent) if d.parent else None
            hops += 1
        fields = {}
        for dd in reversed(chain):
            for fname, fexpr in dd.fields:
                fields[fname] = self.eval(self.globals, fexpr)
        obj = ObjInst(p, fields)
        for dd in reversed(chain):
            for g in dd.methods:
                if g.name == "init":
                    self.call_method_gene(g, obj, args)
                    break
            else:
                continue
            break
        return obj

    # ---- modules
    def load_module(self, path):
        if path in self.modules:
            return self.modules[path]
        if path in self.loading:
            return {}
        p = path if path.endswith(".op") else path + ".op"
        # W069: resolution chain mirrors genes.rs resolve_path — the
        # importing file's directory first, then CWD, then std/, then
        # $OPERON_STD. (Exe-relative roots are runtime-only: the oracle
        # never ships beside a std/ tree; see SPEC §8 table.)
        cands = [p, os.path.join("std", p)]
        if self.base_dir:
            cands.insert(0, os.path.join(self.base_dir, p))
        std_dir = os.environ.get("OPERON_STD")
        if std_dir:
            cands.append(os.path.join(std_dir, p))
        resolved = next((c for c in cands if os.path.exists(c)), None)
        if resolved is None:
            return {}
        src = open(resolved, encoding="utf-8", errors="replace").read()  # W59: explicit UTF-8 (Windows locale default is cp1252)
        self.loading.append(path)
        stmts, notes = parse(src)
        for nt in notes:
            self.note(nt.rung, f"[{path}] {nt.message}")
        menv = self.new_scope(self.globals)
        for st in stmts:
            try:
                self.exec_stmt(menv, st)
            except Stress as e:
                # W06 (D-014) mirror: propagation with no enclosing gene —
                # the variant value passes through (noted, never rejected).
                if e.prop is not None:
                    self.note(4, f"propagation reached top level: {v_repr(e.prop)} passes through")
                else:
                    self.note(4, f"stress contained: [{e.kind}] {e.message}")
            except (Return, BreakLoop, ContinueLoop):
                pass
        self.loading.pop()
        # export rule: anchor export wins, else top-level genes/lets
        has_anchor = any(s[0] == "anchor_export" for s in stmts) or \
            any(s[0] == "tad" and any(x[0] == "anchor_export" for x in s[2]) for s in stmts)
        exports = {}
        if has_anchor:
            names = []
            for s in stmts:
                if s[0] == "anchor_export":
                    names.extend(s[1])
                if s[0] == "tad":
                    for x in s[2]:
                        if x[0] == "anchor_export":
                            names.extend(x[1])
            for nm in names:
                v = self.lookup(menv, nm)
                if v is not None or nm in menv:
                    exports[nm] = v
        else:
            for kk, vv in menv.items():
                if kk != "__parent__" and not kk.startswith("#"):
                    exports[kk] = vv
        self.modules[path] = exports
        return exports

def collect_structure(stmts):
    proofs, frames, exports, tad_exports, tad_members, ires = [], [], [], [], [], []
    def walk(body):
        for s in body:
            if s[0] == "frame":
                if s[2]:
                    proofs.append(s[3])
                else:
                    frames.append((s[1], s[3]))
                walk(s[3])
            elif s[0] == "tad":
                members, exps = [], []
                for x in s[2]:
                    if x[0] == "anchor_export":
                        exps.extend(x[1])
                    if x[0] == "gene" and x[1].name:
                        members.append(x[1].name)
                    if x[0] == "let":
                        members.append(x[1])
                tad_exports.append((s[1], exps))
                tad_members.append((s[1], members))
                walk(s[2])
            elif s[0] == "anchor_export":
                exports.extend(s[1])
            elif s[0] == "ires":
                ires.append(s[1])
            elif s[0] == "gene":
                walk(s[1].body)
                if s[1].guard:
                    walk(s[1].guard[1])
            elif s[0] == "splice":
                for _, _vp, vb, _mk in s[2]:
                    walk(vb)
    walk(stmts)
    return proofs, frames, exports, tad_exports, tad_members, ires

# ----------------------------------------------------------------------------
# nmd + orf helpers (mirror of genes.rs)

def nmd_sweep(stmts, called, enhanced):
    findings = []
    def scan_body(body):
        returned = False
        for s in body:
            if returned:
                findings.append(("premature-stop", "statement after unconditional return (premature stop codon)"))
            scan_inner(s)
            if s[0] == "return":
                returned = True
    def scan_inner(s):
        k = s[0]
        if k == "gene":
            scan_body(s[1].body)
            if s[1].guard:
                scan_body(s[1].guard[1])
        elif k == "if":
            for _, b in s[1]:
                scan_body(b)
            if s[2]:
                scan_body(s[2])
        elif k in ("while", "loop"):
            scan_body(s[2] if k == "while" else s[1])
        elif k == "for":
            scan_body(s[3])
        elif k == "block" or k == "tad":
            scan_body(s[2] if k == "tad" else s[1])
        elif k == "frame":
            scan_body(s[3])
        elif k == "stress":
            scan_body(s[2])
            if s[3]:
                scan_body(s[3][1])
    for s in stmts:
        scan_inner(s)
    for s in stmts:
        if s[0] == "gene" and s[1].name:
            name = s[1].name
            if name in ("main",) or name in enhanced or name in called:
                continue
            if not s[1].methylate:
                findings.append(("untranslated", f"gene '{name}' defined but never translated (dead transcript)"))
    return findings

def find_orfs_python(dna):
    out = []
    i = 0
    while i + 2 < len(dna):
        if dna[i:i+3] == "ATG":
            j, protein, stopped = i, [], False
            while j + 2 < len(dna):
                c2 = dna[j:j+3]
                if c2 in ("TAA", "TAG", "TGA"):
                    stopped = True
                    break
                protein.append(translate_codon(c2))
                j += 3
            if stopped and protein:
                out.append("".join(protein))
        i += 1
    return out

def translate_codon(codon):
    T = {}
    pairs = [("TTT","F"),("TTC","F"),("TTA","L"),("TTG","L"),("TCT","S"),("TCC","S"),("TCA","S"),("TCG","S"),
             ("TAT","Y"),("TAC","Y"),("TAA","*"),("TAG","*"),("TGT","C"),("TGC","C"),("TGA","*"),("TGG","W"),
             ("CTT","L"),("CTC","L"),("CTA","L"),("CTG","L"),("CCT","P"),("CCC","P"),("CCA","P"),("CCG","P"),
             ("CAT","H"),("CAC","H"),("CAA","Q"),("CAG","Q"),("CGT","R"),("CGC","R"),("CGA","R"),("CGG","R"),
             ("ATT","I"),("ATC","I"),("ATA","I"),("ATG","M"),("ACT","T"),("ACC","T"),("ACA","T"),("ACG","T"),
             ("AAT","N"),("AAC","N"),("AAA","K"),("AAG","K"),("AGT","S"),("AGC","S"),("AGA","R"),("AGG","R"),
             ("GTT","V"),("GTC","V"),("GTA","V"),("GTG","V"),("GCT","A"),("GCC","A"),("GCA","A"),("GCG","A"),
             ("GAT","D"),("GAC","D"),("GAA","E"),("GAG","E"),("GGT","G"),("GGC","G"),("GGA","G"),("GGG","G")]
    for k, v in pairs:
        T[k] = v
    return T.get(codon, "X")

# ----------------------------------------------------------------------------
# load & run

def parse_cell(src):
    out = {}
    section = ""
    for line in src.splitlines():
        t = line.strip()
        if not t or t.startswith("#"):
            continue
        if t.startswith("[") and t.endswith("]"):
            section = t[1:-1].strip()
            continue
        if "=" in t:
            k, v = t.split("=", 1)
            kk = f"{section}.{k.strip()}" if section else k.strip()
            out[kk] = v.strip().strip('"')
    return out

def apply_rna(src, patch_src, stem):
    applied = []
    stmts, _ = parse(patch_src)
    text = src
    for s in stmts:
        if s[0] == "edit":
            target, reps = s[1], s[2]
            if target not in (stem, "anywhere"):
                start = text.find(f"gene {target}")
                if start >= 0:
                    end = text.find("\ngene ", start + 5)
                    end = end + 1 if end >= 0 else len(text)
                    seg = text[start:end]
                    seg2 = seg
                    for frm, to in reps:
                        if frm in seg2:
                            seg2 = seg2.replace(frm, to)
                            applied.append(f"{target}: '{frm}' -> '{to}'")
                    if seg2 != seg:
                        text = text[:start] + seg2 + text[end:]
            else:
                for frm, to in reps:
                    if frm in text:
                        text = text.replace(frm, to)
                        applied.append(f"{target}: '{frm}' -> '{to}'")
    return text, applied

def load_file(path, cell=None, variant=None, rna=None, args=None, caps=None):
    src = open(path, encoding="utf-8", errors="replace").read()  # W59: explicit UTF-8
    stem = os.path.basename(path).rsplit(".", 1)[0]
    it = Interp(cell=cell or {}, cli_args=args or [])
    # W069: entry file's directory = resolution root #1 (matches Rust
    # tools.rs interp.base_dir = entry-file dir).
    d = os.path.dirname(os.path.abspath(path))
    if d and d != os.path.abspath("."):
        it.base_dir = d
    if caps is not None:
        it.caps = caps
    if variant:
        it.cell["cli.variant"] = variant
    if rna:
        patch = open(rna, encoding="utf-8", errors="replace").read()  # W59: explicit UTF-8
        src2, applied = apply_rna(src, patch, stem)
        for a in applied:
            it.note(1, f"rna edit applied: {a}")
        src = src2
    stmts, notes = parse(src)
    for nt in notes:
        it.note(nt.rung, nt.message)
    it.ires = [s[1] for s in stmts if s[0] == "ires"]
    # capability grants from .cell allow.* keys — only an EXPLICITLY passed
    # --cell may grant; auto-detected operon.cell keys are ignored with a note
    cell_explicit = cell is not None and isinstance(cell, dict)
    for k, v in list(it.cell.items()):
        if k.startswith("allow."):
            rest = k[len("allow."):]
            if rest in ("read", "write", "run", "net", "env", "py"):
                if not cell_explicit:
                    it.note(1, f"cell key '{k}={v}' ignored: auto-detected operon.cell cannot grant capabilities (pass --cell explicitly)")
                    continue
                # cell values may carry a comma-separated grant list (parity
                # with the Rust runner's cell parsing)
                for g in v.split(","):
                    g = g.strip()
                    if g:
                        it.caps[rest].append(g)
    if it.cell.get("entry"):
        it.cell_entry = it.cell["entry"]
    if it.cell.get("methylate.quiet") == "true":
        it.methyl_quiet = True
    # .cell methylate.threshold (T2b graded silencing gate; default 3)
    _mthr = it.cell.get("methylate.threshold")
    if _mthr is not None:
        try:
            it.methyl_threshold = int(str(_mthr).strip())
        except ValueError:
            it.note(4, f"cell key 'methylate.threshold = {_mthr}' ignored: needs a non-negative integer")
    # reg-bio (F-6): enhancer dose (default 0.25)
    _ed = it.cell.get("enhance.delta")
    if _ed is not None:
        try:
            _f = float(str(_ed).strip())
            if 0.0 <= _f <= 1.0:
                it.enhance_delta = _f
            else:
                it.note(4, f"cell key 'enhance.delta = {_ed}' ignored: needs a number in 0..=1")
        except ValueError:
            it.note(4, f"cell key 'enhance.delta = {_ed}' ignored: needs a number in 0..=1")
    # reg-bio (F-1): telegraph promoter layer (opt-in)
    if it.cell.get("expression.stochastic", "").strip() == "true":
        it.expr_stochastic = True
    for _ck, _attr in (("expression.kon", "expr_kon"), ("expression.koff", "expr_koff")):
        _cv = it.cell.get(_ck)
        if _cv is not None:
            try:
                _f = float(str(_cv).strip())
                if 0.0 <= _f <= 1.0:
                    setattr(it, _attr, _f)
                else:
                    it.note(4, f"cell key '{_ck} = {_cv}' ignored: needs a number in 0..=1")
            except ValueError:
                it.note(4, f"cell key '{_ck} = {_cv}' ignored: needs a number in 0..=1")
    _esd = it.cell.get("expression.seed")
    if _esd is not None:
        try:
            _s = int(str(_esd).strip())
            it.rng = 0x9E3779B97F4A7C15 if _s == 0 else (_s & M64)
        except ValueError:
            it.note(4, f"cell key 'expression.seed = {_esd}' ignored: needs an integer")
    # reg-bio (F-5): repressilator kinetics — plasmid engineering surface
    for _ck, _lo, _hi in (("repressi.alpha", None, None), ("repressi.gamma", None, None),
                          ("repressi.basal", None, None), ("repressi.noise", None, None)):
        _cv = it.cell.get(_ck)
        if _cv is None:
            continue
        try:
            _f = float(str(_cv).strip())
        except ValueError:
            it.note(4, f"cell key '{_ck} = {_cv}' ignored: needs a number")
            continue
        _key = _ck.split(".")[1]
        if _key == "alpha" and _f > 0.0:
            it.repressi_params["alpha"] = _f
        elif _key == "gamma" and _f >= 0.0:
            it.repressi_params["gamma"] = _f
        elif _key == "basal" and _f >= 0.0:
            it.repressi_params["basal"] = _f
        elif _key == "noise" and 0.0 <= _f <= 1.0:
            it.repressi_params["noise"] = _f
        elif _key == "alpha":
            it.note(4, f"cell key '{_ck} = {_cv}' ignored: needs a number > 0")
        elif _key == "gamma" or _key == "basal":
            it.note(4, f"cell key '{_ck} = {_cv}' ignored: needs a number >= 0")
        elif _key == "noise":
            it.note(4, f"cell key '{_ck} = {_cv}' ignored: needs a number in 0..=1")
    _rh = it.cell.get("repressi.hill")
    if _rh is not None:
        try:
            _n = int(str(_rh).strip())
            if 1 <= _n <= 8:
                it.repressi_params["hill"] = _n
            else:
                it.note(4, f"cell key 'repressi.hill = {_rh}' ignored: needs an integer 1..=8")
        except ValueError:
            it.note(4, f"cell key 'repressi.hill = {_rh}' ignored: needs an integer 1..=8")
    _rsd = it.cell.get("repressi.seed")
    if _rsd is not None:
        try:
            _s = int(str(_rsd).strip())
            it.repressi_params["seed"] = 0x9E3779B97F4A7C15 if _s == 0 else (_s & M64)
        except ValueError:
            it.note(4, f"cell key 'repressi.seed = {_rsd}' ignored: needs an integer")
    for st in stmts:
        try:
            it.exec_stmt(it.globals, st)
        except Stress as e:
            # W06 (D-014) mirror: top-level propagation passes through.
            if e.prop is not None:
                it.note(4, f"propagation reached top level: {v_repr(e.prop)} passes through")
            else:
                it.note(4, f"stress contained: [{e.kind}] {e.message}")
        except (Return, BreakLoop, ContinueLoop):
            pass
    proofs, frames, exports, tad_exports, tad_members, ires = collect_structure(stmts)
    return it, stmts, proofs, frames

def resolve_entry(it, stmts, entry=None, use_ires=False):
    if entry:
        return entry
    if it.cell_entry:
        return it.cell_entry
    if use_ires and it.ires:
        return it.ires[0]
    if any(s[0] == "gene" and s[1].name == "main" for s in stmts):
        return "main"
    if it.ires:
        return it.ires[0]
    return None

def run(path, cell=None, variant=None, rna=None, entry=None, frame=None, args=None, caps=None):
    it, stmts, proofs, frames = load_file(path, cell, variant, rna, args, caps)
    if frame:
        for nm, body in frames:
            if nm == frame:
                it.exec_block(it.new_scope(it.globals), body)
    else:
        e = resolve_entry(it, stmts, entry)
        if e:
            argv = list(args or [])
            g = it.globals.get(e)
            if isinstance(g, Gene) and g.params:
                it.call_gene(g, [argv])
            elif isinstance(g, Gene):
                it.call_gene(g, [])
            else:
                it.call_named(it.globals, e, [argv])
    return it

def main():
    argv = sys.argv[1:]
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        sys.exit(2)
    cmd = argv[0]
    rest = argv[1:]
    opts = {"cell": None, "variant": None, "rna": None, "entry": None, "frame": None, "args": []}
    pos = []
    i = 0
    while i < len(rest):
        a = rest[i]
        if a == "--cell":
            i += 1; opts["cell"] = rest[i]
        elif a == "--variant":
            i += 1; opts["variant"] = rest[i]
        elif a == "--rna":
            i += 1; opts["rna"] = rest[i]
        elif a == "--entry":
            i += 1; opts["entry"] = rest[i]
        elif a == "--frame":
            i += 1; opts["frame"] = rest[i]
        elif a == "--json":
            opts["json"] = True
        elif a in ("--allow-read", "--allow-write", "--allow-run", "--allow-net", "--allow-env", "--allow-py"):
            i += 1
            cap = a.replace("--allow-", "")
            it_caps = opts.setdefault("caps", {"enabled": True, "read": [], "write": [], "run": [], "net": [], "env": [], "py": []})
            it_caps[cap].append(rest[i])
        elif a == "--allow-all":
            opts["caps"] = {"enabled": False, "read": [], "write": [], "run": [], "net": [], "env": [], "py": []}
        else:
            pos.append(a)
        i += 1
    # loop-10: load the --cell FILE into the dict the interpreter expects
    # (the flag previously leaked the path string into Interp(cell=...) —
    # AttributeError on .items(); the Rust CLI parses the file, so did we)
    cell_dict = None
    if opts["cell"]:
        with open(opts["cell"], encoding="utf-8", errors="replace") as _cf:  # W59: explicit UTF-8
            cell_dict = parse_cell(_cf.read())
    if cmd == "version":
        print("Operon 2.0.0 (python-oracle)")
    elif cmd == "run":
        it = run(pos[0], cell_dict, opts["variant"], opts["rna"], opts["entry"], opts["frame"], pos[1:], caps=opts.get("caps"))
        for nt in it.notes:
            tag = {1: "info", 2: "synonym", 3: "wobble"}.get(nt.rung, "fallback")
            print(f"[{tag}] {nt.message}", file=sys.stderr)
    elif cmd == "test":
        paths = pos or ["tests"]
        files = []
        for p in paths:
            if os.path.isdir(p):
                for root, _, fns in os.walk(p):
                    # reg-r4: adversarial red-team payloads are containment
                    # expectations, not proof frames — the Rust runner skips
                    # this directory too (parity; the walk used to crash on
                    # one of the payloads)
                    if "redteam" in root.replace(os.sep, "/"):
                        continue
                    # substrate-r1: capability-granted proofs run only under
                    # an explicit operator cell (Rust runner skips them too)
                    if "granted" in root.replace(os.sep, "/"):
                        continue
                    for fn in sorted(fns):
                        if fn.endswith(".op"):
                            files.append(os.path.join(root, fn))
            else:
                files.append(p)
        files.sort()
        files_t, proofs, passed, failed, failures = 0, 0, 0, 0, []
        for f in files:
            # reg-r4: a file that fails to LOAD is a suite failure, not a
            # crash of the runner (the Rust runner records it and continues)
            try:
                it, stmts, proofs_l, frames = load_file(f, cell_dict, opts["variant"], opts["rna"])
            except RecursionError:
                failed += 1
                failures.append(f"{f}: load error: recursion limit (parser depth)")
                continue
            except Exception as ex:
                failed += 1
                failures.append(f"{f}: load error: {type(ex).__name__}: {ex}")
                continue
            files_t += 1
            proofs += len(proofs_l)
            it.proof_mode = True
            for pf in proofs_l:
                try:
                    it.exec_block(it.new_scope(it.globals), pf)
                    passed += 1
                except Stress as st:
                    failed += 1
                    # W06 (D-014) mirror: propagation abandoning a proof frame
                    # is a failure (the frame did not complete).
                    if st.prop is not None:
                        failures.append(f"{f}: propagation left the proof frame ({v_repr(st.prop)})")
                    else:
                        failures.append(f"{f}: [{st.kind}] {st.message}")
                except Exception as ex:
                    failed += 1
                    failures.append(f"{f}: [oracle-error] {type(ex).__name__}: {ex}")
        print(f"operon test — {files_t} file(s), {proofs} proof(s): {passed} passed, {failed} failed")
        for f in failures:
            print(f"  FAIL {f}", file=sys.stderr)
        if failed:
            sys.exit(1)
    elif cmd == "check":
        src = open(pos[0], encoding="utf-8", errors="replace").read()  # W59: explicit UTF-8
        stmts, notes = parse(src)
        score = 100 - sum(1 if n.rung == 2 else (2 if n.rung == 3 else (3 if n.rung == 4 else 0)) for n in notes)
        letter = "A" if score >= 90 else "B" if score >= 80 else "C" if score >= 70 else "D" if score >= 60 else "F"
        print(f"operon check: {pos[0]} — score {max(score, 50)}/100 (grade {letter})")
    else:
        print("oracle supports: run | test | check | version", file=sys.stderr)
        sys.exit(2)

if __name__ == "__main__":
    # deep recursion support: run on a worker thread with a large stack and a
    # raised recursion limit, mirroring the Rust core's 512 MiB worker + 10k
    # Operon-frame depth limit. Runaway recursion surfaces as RecursionError,
    # never a native crash.
    #
    # W59 (Windows truthing), two fixes to the runner itself:
    #
    # 1. Honest exit codes. The worker-thread form used to swallow every
    #    failure: an exception inside main() printed a traceback and the
    #    process still exited 0, and main()'s own sys.exit() only ended the
    #    thread. A dead oracle therefore looked SUCCESSFUL to any caller
    #    that checks exit codes — the differential harness compares them.
    #    The worker now records the outcome and the main thread re-raises it
    #    as the process exit code.
    #
    # 2. Portable stack size. 1 << 29 sits exactly at Windows' 512 MiB
    #    threading.stack_size() ceiling; treat the request as best-effort
    #    with a fallback ladder instead of a hard startup dependency.
    sys.setrecursionlimit(150_000)
    import threading as _th

    for _sz in (1 << 29, 1 << 28, 1 << 26):
        try:
            _th.stack_size(_sz)
            break
        except (ValueError, RuntimeError, OverflowError):
            continue

    _rc = {"code": 0}

    def _worker():
        try:
            main()
        except SystemExit as _e:
            _rc["code"] = _e.code if isinstance(_e.code, int) else (0 if _e.code is None else 1)
        except BaseException:
            import traceback
            traceback.print_exc()
            _rc["code"] = 1

    _t = _th.Thread(target=_worker, name="oracle-worker")
    _t.start()
    _t.join()
    if _rc["code"]:
        sys.exit(_rc["code"])
