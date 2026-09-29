#!/usr/bin/env python3
"""patch_oracle_notes.py — one-shot parity fix for the oracle diagnostics
channel (compat matrix finding #1, 2026-09-30).

The Rust core renders parser/lexer notes as:

    [fallback] <file>:<line>: <message>

The Python oracle rendered them as:

    [fallback] <message>

and the pattern-wildcard message omitted the offending token text
(Rust: `pattern '(' treated as wildcard`). This patch:
  1. gives Note a line slot (default 0 — existing call sites stay valid)
  2. threads the line from every lexer note site (all have `line` in scope)
  3. stores the line in P.note (parser notes)
  4. mirrors Tok::describe() for the wildcard message
  5. renders `[tag] file:line: msg` in the run command, mirroring
     tools::flush_notes
Run once from the repo root; idempotence is verified by the git diff.
"""
import re

PATH = "bootstrap/oracle.py"
s = open(PATH).read()
orig = s

# 1. Note carries a line (default 0 keeps all existing sites valid)
s = s.replace(
    '''class Note:
    __slots__ = ("rung", "message")
    def __init__(self, rung, message):
        self.rung, self.message = rung, message''',
    '''class Note:
    __slots__ = ("rung", "line", "message")
    def __init__(self, rung, message, line=0):
        # compat matrix fix: the Rust core stores the origin line on every
        # note (ast.rs Note{line, rung, message}) and tools::flush_notes
        # renders `[tag] file:line: msg`. The oracle now mirrors both.
        self.rung, self.line, self.message = rung, line, message''',
)

# 2. lexer note sites — all inside lex() where `line` is in scope
lexer_sites = [
    ('notes.append(Note(4, "unclosed raw string consumed to end of input"))',
     'notes.append(Note(4, "unclosed raw string consumed to end of input", line))'),
    ('notes.append(Note(4, "unclosed multiline string consumed to end of input"))',
     'notes.append(Note(4, "unclosed multiline string consumed to end of input", line))'),
    ('notes.append(Note(4, "unclosed string consumed to end of line"))',
     'notes.append(Note(4, "unclosed string consumed to end of line", line))'),
    ('notes.append(Note(4, "single-quoted string repaired to double quotes"))',
     'notes.append(Note(4, "single-quoted string repaired to double quotes", line))'),
    ('notes.append(Note(4, "stray \'@\' skipped"))',
     'notes.append(Note(4, "stray \'@\' skipped", line))'),
    ('notes.append(Note(4, f"integer \'{raw}\' out of range treated as 0"))',
     'notes.append(Note(4, f"integer \'{raw}\' out of range treated as 0", line))'),
    ('notes.append(Note(4, f"malformed number \'{text}\' treated as 0"))',
     'notes.append(Note(4, f"malformed number \'{text}\' treated as 0", line))'),
    ('notes.append(Note(4, f"integer \'{text}\' out of range treated as 0"))',
     'notes.append(Note(4, f"integer \'{text}\' out of range treated as 0", line))'),
    ('notes.append(Note(4, f"unexpected character \'{c}\' skipped"))',
     'notes.append(Note(4, f"unexpected character \'{c}\' skipped", line))'),
]
for old, new in lexer_sites:
    if old in s:
        s = s.replace(old, new)

# 3. parser note stores the line it is given
s = s.replace(
    """    def note(self, line, rung, msg):
        self.notes.append(Note(rung, msg))""",
    """    def note(self, line, rung, msg):
        self.notes.append(Note(rung, msg, line))""",
)

# 4. describe() mirror for the wildcard note
s = s.replace(
    '''        if neg:
            self.note(t[2], 4, "dangling '-' in pattern treated as wildcard")
            self.next()
            return ("wild",)
        self.note(t[2], 4, "pattern treated as wildcard")
        self.next()
        return ("wild",)''',
    '''        if neg:
            self.note(t[2], 4, "dangling '-' in pattern treated as wildcard")
            self.next()
            return ("wild",)
        self.note(t[2], 4, f"pattern '{describe_tok(t)}' treated as wildcard")
        self.next()
        return ("wild",)''',
)

# describe_tok definition right before parse()
s = s.replace(
    "def parse(src):",
    '''def describe_tok(t):
    """Mirror of src/lexer.rs Tok::describe (dx-r1) for the token shapes
    that can reach the pattern-wildcard arm."""
    kind, val = t[0], t[1]
    if kind == "IDENT":
        return f"identifier '{val}'"
    if kind == "INT":
        return f"number {val}"
    if kind == "FLOAT":
        return f"number {val}"
    if kind == "STR":
        return f'string "{val}"'
    if kind == "INTERP":
        return f'interpolated string "{val}"'
    if kind == "EOF":
        return "end of input"
    if kind == "NL":
        return "end of line"
    return f"'{val}'"


def parse(src):''',
)

# 5. run-command renderer mirrors tools::flush_notes
s = s.replace(
    '''        it = run(pos[0], cell_dict, opts["variant"], opts["rna"], opts["entry"], opts["frame"], pos[1:], caps=opts.get("caps"))
        for nt in it.notes:
            tag = {1: "info", 2: "synonym", 3: "wobble"}.get(nt.rung, "fallback")
            print(f"[{tag}] {nt.message}", file=sys.stderr)''',
    '''        it = run(pos[0], cell_dict, opts["variant"], opts["rna"], opts["entry"], opts["frame"], pos[1:], caps=opts.get("caps"))
        for nt in it.notes:
            tag = {1: "info", 2: "synonym", 3: "wobble"}.get(nt.rung, "fallback")
            # A13/dx-r2 mirror: notes carry real locations (compat matrix
            # fix) — `[tag] file:line: msg` exactly like tools::flush_notes
            if getattr(nt, "line", 0) > 0:
                print(f"[{tag}] {pos[0]}:{nt.line}: {nt.message}", file=sys.stderr)
            else:
                print(f"[{tag}] {nt.message}", file=sys.stderr)''',
)

if s == orig:
    raise SystemExit("PATCH PRODUCED NO CHANGES — anchors drifted?")
open(PATH, "w").write(s)
print("oracle notes patched: line-carrying notes + describe mirror + renderer")
