#!/usr/bin/env python3
"""LSP smoke test (G5 seed → lsp-r1 v2 → W44/W45/W46/W45-v2): drives ./target/release/operon-ls.

Covers: initialize handshake + advertised capabilities (signatureHelp trigger
chars included), publishDiagnostics (phantom call + repair provenance + the
W46 operonRepairs summary), hover with a gene signature, definition,
references (declaration + call sites + a variable used three times),
semanticTokens (delta-encoded, legend-mapped), signatureHelp (W44: annotated
two-parameter gene, activeParameter by comma depth, null-clean off-call),
prepareRename + rename (all-or-nothing refusals), documentSymbol,
completion, formatting, didClose state clearing, the CWD-independence fix
(server launched from a foreign directory must not phantom stdlib calls),
unknown-method error, shutdown/exit. Run from repo root:
    python3 tests/lsp_smoke.py
Exits 0 on success; asserts loudly otherwise.
"""
import json
import os
import subprocess
import sys
import tempfile

BIN = sys.argv[1] if len(sys.argv) > 1 else "./target/release/operon-ls"

proc = subprocess.Popen(
    [BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
)


def send(msg):
    body = json.dumps(msg).encode()
    proc.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    proc.stdin.flush()


def recv():
    headers = {}
    while True:
        line = proc.stdout.readline().decode()
        if line in ("\r\n", "\n", ""):
            break
        k, _, v = line.partition(":")
        headers[k.strip().lower()] = v.strip()
    n = int(headers["content-length"])
    return json.loads(proc.stdout.read(n))


# 1. initialize handshake — every advertised capability must be listed
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
r = recv()
assert r["id"] == 1 and r["result"]["serverInfo"]["name"] == "operon-ls", r
# W62: the handshake carries the versioned LSP contract (docs/specs/LSP-VERSIONING.md)
assert r["result"]["operonLsp"]["version"] == 1, r["result"].get("operonLsp")
_feats = r["result"]["operonLsp"]["features"]
assert "signatureHelp" in _feats and "hover" in _feats and "formatting" in _feats, _feats
# W45: the new breadth features are advertised
assert "references" in _feats and "semanticTokens" in _feats, _feats
# W45-v2: rename joins additively (lsp 1 unchanged per LSP-VERSIONING rule 1)
assert "rename" in _feats, _feats
caps = r["result"]["capabilities"]
assert caps["hoverProvider"] is True and caps["textDocumentSync"] == 1, caps
assert caps["definitionProvider"] is True, caps
assert caps["referencesProvider"] is True, caps
assert caps["renameProvider"] == {"prepareProvider": True}, caps
assert caps["documentSymbolProvider"] is True, caps
assert caps["documentFormattingProvider"] is True, caps
# W45-v3: inlay hints join additively (lsp 1 unchanged per LSP-VERSIONING rule 1)
assert caps["inlayHintProvider"] is True, caps
assert "inlayHints" in _feats, _feats
assert caps["completionProvider"]["resolveProvider"] is False, caps
# W45: semantic-token legend matches the core's fixed array
_st = caps["semanticTokensProvider"]
assert _st["full"] is True, _st
_legend = _st["legend"]["tokenTypes"]
assert _legend == [
    "keyword", "function", "variable", "string", "number", "comment"
], _legend
# W44: signature help advertises its trigger characters (call open + commas)
assert caps["signatureHelpProvider"]["triggerCharacters"] == ["(", ","], caps

# 2. initialized notification (no reply expected)
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

# 3. didOpen → publishDiagnostics with a phantom-call error
text = (
    "gene boost(x) { return x }\n"
    "main { let y = boost(1) + missing(2) + y * y }\n"
)
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didOpen",
    "params": {
        "textDocument": {"uri": "file:///demo.op", "languageId": "operon", "version": 1},
        "text": text,
    },
})
d = recv()
assert d["method"] == "textDocument/publishDiagnostics", d
assert d["params"]["uri"] == "file:///demo.op", d
msgs = [x["message"] for x in d["params"]["diagnostics"]]
assert any("missing" in m and "phantom" in m for m in msgs), msgs
# diagnostics carry real ranges now (asserted by the wobble case below too)
assert all("range" in x and "start" in x["range"] for x in d["params"]["diagnostics"]), msgs
# W46: a canonical file carries NO repair summary, absence is the signal
assert "operonRepairs" not in d["params"], d["params"]

# 4. hover over the gene name on line 0 → signature markdown
send({
    "jsonrpc": "2.0",
    "id": 2,
    "method": "textDocument/hover",
    "params": {
        "textDocument": {"uri": "file:///demo.op"},
        "position": {"line": 0, "character": 6},
    },
})
r = recv()
assert r["id"] == 2, r
value = r["result"]["contents"]["value"]
assert "gene boost(x)" in value, value

# 3b-Z121. didOpen with the text at the SPEC position (params.textDocument.
# text — what every real client sends) must store the document: the phantom
# call is reported immediately. The pre-fix server read params.text only and
# stored the EMPTY document (all features dead until a didChange).
text_spec = "gene boost(x) { return x }\nmain { missing(2) }\n"
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didOpen",
    "params": {
        "textDocument": {
            "uri": "file:///specpos.op",
            "languageId": "operon",
            "version": 1,
            "text": text_spec,
        }
    },
})
d = recv()
assert d["method"] == "textDocument/publishDiagnostics", d
assert d["params"]["uri"] == "file:///specpos.op", d
msgs_spec = [x["message"] for x in d["params"]["diagnostics"]]
assert any("missing" in m and "phantom" in m for m in msgs_spec), msgs_spec

# 3c-Z121. a RANGED-ONLY didChange must KEEP the previous document (the
# pre-fix fallthrough blanked it — ranged edits are not applied, sync=1
# limitation, but blanking destroyed the analyzed state outright)
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didChange",
    "params": {
        "textDocument": {"uri": "file:///specpos.op", "version": 2},
        "contentChanges": [
            {"range": {"start": {"line": 1, "character": 6}, "end": {"line": 1, "character": 13}}, "text": "absent("}
        ],
    },
})
d = recv()
assert d["method"] == "textDocument/publishDiagnostics", d
msgs_ranged = [x["message"] for x in d["params"]["diagnostics"]]
# document retained: the ORIGINAL phantom (missing) is still diagnosed
assert any("missing" in m and "phantom" in m for m in msgs_ranged), msgs_ranged
# the ranged fragment text did not REPLACE the document (that would
# re-diagnose `absent(` as a top-level parse error instead)
assert not any("absent" in m for m in msgs_ranged), msgs_ranged

# 4b. definition on the CALL site of boost (line 1) → jumps to line 0
send({
    "jsonrpc": "2.0",
    "id": 7,
    "method": "textDocument/definition",
    "params": {
        "textDocument": {"uri": "file:///demo.op"},
        "position": {"line": 1, "character": 18},
    },
})
r = recv()
assert r["id"] == 7, r
assert r["result"]["range"]["start"]["line"] == 0, r
assert r["result"]["uri"] == "file:///demo.op", r

# 4b-W45. references on the boost call site → declaration + the call site
send({
    "jsonrpc": "2.0",
    "id": 21,
    "method": "textDocument/references",
    "params": {
        "textDocument": {"uri": "file:///demo.op"},
        "position": {"line": 1, "character": 18},
        "context": {"includeDeclaration": True},
    },
})
r = recv()
assert r["id"] == 21, r
_locs = r["result"]
_lines = sorted(x["range"]["start"]["line"] for x in _locs)
assert _lines == [0, 1], ("declaration + the one call site", _locs)
assert _locs[0]["range"]["start"]["character"] == 5, ("declaration span", _locs)

# 4b-W45. references on the variable y (decl + two uses = three positions,
# all on the same line: `let y = ... + y * y`)
send({
    "jsonrpc": "2.0",
    "id": 41,
    "method": "textDocument/references",
    "params": {
        "textDocument": {"uri": "file:///demo.op"},
        "position": {"line": 1, "character": 11},
        "context": {"includeDeclaration": True},
    },
})
r = recv()
assert r["id"] == 41, r
_ylocs = r["result"]
assert len(_ylocs) == 3, ("decl + two uses of y", _ylocs)
assert all(x["range"]["start"]["line"] == 1 for x in _ylocs), _ylocs
assert sorted(x["range"]["start"]["character"] for x in _ylocs) == [11, 39, 43], _ylocs

# 4b-W45. semanticTokens → delta-encoded, first token is the `gene` keyword
send({
    "jsonrpc": "2.0",
    "id": 22,
    "method": "textDocument/semanticTokens/full",
    "params": {"textDocument": {"uri": "file:///demo.op"}},
})
r = recv()
assert r["id"] == 22, r
_data = r["result"]["data"]
assert len(_data) >= 15 and len(_data) % 5 == 0, _data[:15]
# line 0 col 0: `gene` — keyword, length 4
assert _data[0:5] == [0, 0, 4, 0, 0], _data[0:5]
# some token is classified function (boost/main/missing)
assert any(_data[i + 3] == 1 for i in range(0, len(_data), 5)), _data
# the legend maps non-empty: every emitted type index is inside the list
# the initialize handshake advertised (a stray index would render wrong)
assert all(0 <= _data[i + 3] < len(_legend) for i in range(0, len(_data), 5)), _data

# 4b-W46. a wobble gets repair provenance: didOpen a file with `retrn`
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didOpen",
    "params": {
        "textDocument": {"uri": "file:///wobble.op", "languageId": "operon", "version": 1},
        "text": "gene g() { retrn 1 }\n",
    },
})
d = recv()
assert d["method"] == "textDocument/publishDiagnostics", d
_wd = [x for x in d["params"]["diagnostics"] if "return" in x["message"]]
assert _wd, d["params"]["diagnostics"]
assert any(
    x.get("relatedInformation")
    and any("interpreted as 'return'" in ri["message"] for ri in x["relatedInformation"])
    for x in _wd
), _wd
# W46: the rung tag is machine-readable on the diagnostic itself...
assert _wd[0]["data"] == {"rung": "wobble", "canonical": False}, _wd[0]
# ...and the publish carries the workspace-visible repair summary
_rep = d["params"].get("operonRepairs")
assert _rep and _rep["applied"] == 1 and _rep["canonical"] is False, d["params"]
assert "canonical form differs" in _rep["note"], _rep
# hovering the repaired token shows the provenance instead of a null
send({
    "jsonrpc": "2.0",
    "id": 23,
    "method": "textDocument/hover",
    "params": {
        "textDocument": {"uri": "file:///wobble.op"},
        "position": {"line": 0, "character": 13},
    },
})
r = recv()
assert r["id"] == 23, r
assert r["result"] is not None, r
assert "repair" in r["result"]["contents"]["value"], r["result"]
assert "return" in r["result"]["contents"]["value"], r["result"]

# 4c-W44. signatureHelp on a call with two annotated params (W01 types) and a
## doc comment; trigger positions are right after `(` and right after a `,`
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didOpen",
    "params": {
        "textDocument": {"uri": "file:///sig.op", "languageId": "operon", "version": 1},
        "text": (
            "## power raises base to the exponent\n"
            "gene power(base: int, exp: int) -> int { return base * exp }\n"
            "main {\n"
            "    let r = power(3, 4)\n"
            "    let q = unknown(1)\n"
            "}\n"
        ),
    },
})
d = recv()
assert d["method"] == "textDocument/publishDiagnostics", d

# right after the call open: first parameter active, annotations in labels
send({
    "jsonrpc": "2.0",
    "id": 40,
    "method": "textDocument/signatureHelp",
    "params": {
        "textDocument": {"uri": "file:///sig.op"},
        "position": {"line": 3, "character": 18},
    },
})
r = recv()
assert r["id"] == 40, r
_sig = r["result"]["signatures"][0]
assert _sig["label"] == "power(base: int, exp: int) -> int", _sig
assert [p["label"] for p in _sig["parameters"]] == ["base: int", "exp: int"], _sig
assert r["result"]["activeSignature"] == 0, r["result"]
assert r["result"]["activeParameter"] == 0, r["result"]
# W074 feed: the `##` doc line rides along as the documentation
assert "raises base to the exponent" in _sig["documentation"], _sig

# right after the argument comma: activeParameter advances to 1
send({
    "jsonrpc": "2.0",
    "id": 42,
    "method": "textDocument/signatureHelp",
    "params": {
        "textDocument": {"uri": "file:///sig.op"},
        "position": {"line": 3, "character": 20},
    },
})
r = recv()
assert r["id"] == 42 and r["result"]["activeParameter"] == 1, r

# an unresolvable callee answers null cleanly (never an error)
send({
    "jsonrpc": "2.0",
    "id": 44,
    "method": "textDocument/signatureHelp",
    "params": {
        "textDocument": {"uri": "file:///sig.op"},
        "position": {"line": 4, "character": 20},
    },
})
r = recv()
assert r["id"] == 44 and r["result"] is None, r

# 4b-W45v2. rename — prepareRename + the all-or-nothing rename (W67 discipline)
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didOpen",
    "params": {
        "textDocument": {"uri": "file:///rename.op", "languageId": "operon", "version": 1},
        "text": (
            "gene boost(x) { return x * 2 }\n"
            "# boost in a comment\n"
            "main {\n"
            "    let s = \"boost stays\"\n"
            "    let y = boost(1)\n"
            "}\n"
        ),
    },
})
d = recv()
assert d["method"] == "textDocument/publishDiagnostics", d

# prepareRename on the call site answers the full span + placeholder
send({
    "jsonrpc": "2.0",
    "id": 30,
    "method": "textDocument/prepareRename",
    "params": {
        "textDocument": {"uri": "file:///rename.op"},
        "position": {"line": 4, "character": 15},
    },
})
r = recv()
assert r["id"] == 30, r
assert r["result"]["placeholder"] == "boost", r
assert r["result"]["range"]["start"] == {"line": 4, "character": 12}, r

# a keyword position refuses (prepareRename → null)
send({
    "jsonrpc": "2.0",
    "id": 31,
    "method": "textDocument/prepareRename",
    "params": {
        "textDocument": {"uri": "file:///rename.op"},
        "position": {"line": 0, "character": 21},
    },
})
r = recv()
assert r["id"] == 31 and r["result"] is None, r

# rename boost → enlarge: decl + call site; string and comment untouched
send({
    "jsonrpc": "2.0",
    "id": 32,
    "method": "textDocument/rename",
    "params": {
        "textDocument": {"uri": "file:///rename.op"},
        "position": {"line": 4, "character": 15},
        "newName": "enlarge",
    },
})
r = recv()
assert r["id"] == 32, r
_edits = r["result"]["changes"]["file:///rename.op"]
assert len(_edits) == 2, _edits
assert all(e["newText"] == "enlarge" for e in _edits), _edits
assert sorted(
    (e["range"]["start"]["line"], e["range"]["start"]["character"]) for e in _edits
) == [(0, 5), (4, 12)], _edits

# a reserved new name refuses the WHOLE rename (JSON-RPC error, not a null)
send({
    "jsonrpc": "2.0",
    "id": 33,
    "method": "textDocument/rename",
    "params": {
        "textDocument": {"uri": "file:///rename.op"},
        "position": {"line": 4, "character": 15},
        "newName": "return",
    },
})
r = recv()
assert r["id"] == 33 and r.get("error", {}).get("code") == -32001, r
assert "reserved" in r["error"]["message"], r

# an in-file collision refuses with the all-or-nothing reason
send({
    "jsonrpc": "2.0",
    "id": 34,
    "method": "textDocument/rename",
    "params": {
        "textDocument": {"uri": "file:///rename.op"},
        "position": {"line": 4, "character": 15},
        "newName": "main",
    },
})
r = recv()
assert r["id"] == 34 and "error" in r, r
assert "merge unrelated bindings" in r["error"]["message"], r

# 4c. documentSymbol → boost + main listed
send({
    "jsonrpc": "2.0",
    "id": 8,
    "method": "textDocument/documentSymbol",
    "params": {"textDocument": {"uri": "file:///demo.op"}},
})
r = recv()
assert r["id"] == 8, r
names = [s["name"] for s in r["result"]]
assert any(n.startswith("boost(") for n in names), names
assert any(n.startswith("main(") for n in names), names

# 4d. completion → contains the in-file gene, builtins, and keywords
send({
    "jsonrpc": "2.0",
    "id": 9,
    "method": "textDocument/completion",
    "params": {
        "textDocument": {"uri": "file:///demo.op"},
        "position": {"line": 1, "character": 10},
    },
})
r = recv()
assert r["id"] == 9, r
labels = [c["label"] for c in r["result"]]
assert "boost" in labels, labels
assert "promote" in labels, labels
assert "gene" in labels, labels

# 4e. formatting → a full-document TextEdit with the formatted text
send({
    "jsonrpc": "2.0",
    "id": 10,
    "method": "textDocument/formatting",
    "params": {
        "textDocument": {"uri": "file:///demo.op"},
        "options": {"tabSize": 4},
    },
})
r = recv()
assert r["id"] == 10, r
edit = r["result"][0]
assert "newText" in edit and "range" in edit, edit
assert "gene boost" in edit["newText"], edit["newText"][:200]

# 5. hover over a non-identifier (the `{`) → null result
send({
    "jsonrpc": "2.0",
    "id": 6,
    "method": "textDocument/hover",
    "params": {
        "textDocument": {"uri": "file:///demo.op"},
        "position": {"line": 1, "character": 6},
    },
})
r = recv()
assert r["id"] == 6 and r["result"] is None, r

# 5a-W45v3. inlay hints — the checker's inferred types for un-annotated
# bindings, precomputed per document version, filtered by the visible range.
# Laws pinned: concrete types hint (": int" class, kind 1 = Type, position
# exactly past the binding name); annotated bindings never hint (the source
# already shows it); `any` never hints (the dynamic escape hatch carries no
# information — never a vacuous hint); a narrowed range returns only its rows.
hints_src = (
    "let n = 42\n"
    "let s = \"hi\"\n"
    "let ann: str = \"x\"\n"
    "let lst = [1, 2, 3]\n"
    "let opaq = unknown_lane()\n"
)
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didOpen",
    "params": {
        "textDocument": {"uri": "file:///hints.op", "languageId": "operon", "version": 1},
        "text": hints_src,
    },
})
d = recv()
assert d["method"] == "textDocument/publishDiagnostics", d

def _hints_over(start, end):
    send({
        "jsonrpc": "2.0",
        "id": 60,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": {"uri": "file:///hints.op"},
            "range": {"start": {"line": start, "character": 0},
                      "end": {"line": end, "character": 0}},
        },
    })
    r = recv()
    assert r["id"] == 60, r
    return r["result"]

all_hints = _hints_over(0, 4)
_by_name_pos = {(h["position"]["line"], h["position"]["character"]): h["label"] for h in all_hints}
# `let n = 42` → `: int` right past the name (line 0, char 5); kind = Type
assert _by_name_pos.get((0, 5)) == ": int", all_hints
assert _by_name_pos.get((1, 5)) == ": str", all_hints
assert _by_name_pos.get((3, 7)) == ": list[int]", all_hints
assert all(h["kind"] == 1 for h in all_hints), all_hints
# annotated (`ann`, line 2) and any-typed (`opaq`, line 4) NEVER hint
assert (2, 9) not in _by_name_pos and (4, 10) not in _by_name_pos, all_hints
assert len(all_hints) == 3, all_hints
# narrowed window: only rows inside [start.line, end.line]
assert _hints_over(0, 1) == [
    h for h in all_hints if h["position"]["line"] <= 1
], all_hints
assert _hints_over(3, 4) == [
    h for h in all_hints if h["position"]["line"] >= 3
], all_hints
# unknown document → honest empty, not an error
send({
    "jsonrpc": "2.0",
    "id": 61,
    "method": "textDocument/inlayHint",
    "params": {
        "textDocument": {"uri": "file:///never-opened.op"},
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 9, "character": 0}},
    },
})
r = recv()
assert r["id"] == 61 and r["result"] == [], r

# 5b. didClose → diagnostics cleared, doc forgotten
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didClose",
    "params": {"textDocument": {"uri": "file:///demo.op"}},
})
d = recv()
assert d["method"] == "textDocument/publishDiagnostics", d
assert d["params"]["diagnostics"] == [], d

# 6. unknown request → error -32601
send({"jsonrpc": "2.0", "id": 4, "method": "foo/bar", "params": {}})
r = recv()
assert r["id"] == 4 and r["error"]["code"] == -32601, r

# 7. shutdown + exit
send({"jsonrpc": "2.0", "id": 5, "method": "shutdown"})
r = recv()
assert r["id"] == 5 and "result" in r, r
send({"jsonrpc": "2.0", "method": "exit"})
proc.wait(timeout=5)
assert proc.returncode == 0, proc.returncode

# 8. lsp-r1 P0 regression: launched from a FOREIGN CWD (how editors launch
# servers), a doc that `use`s the stdlib must NOT produce phantom-call
# diagnostics for stdlib genes. Run a second server instance from /tmp.
tmp = tempfile.mkdtemp()
env_proc = subprocess.Popen(
    [os.path.abspath(BIN)],
    cwd=tmp,
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
)


def send2(msg):
    body = json.dumps(msg).encode()
    env_proc.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    env_proc.stdin.flush()


def recv2():
    headers = {}
    while True:
        line = env_proc.stdout.readline().decode()
        if line in ("\r\n", "\n", ""):
            break
        k, _, v = line.partition(":")
        headers[k.strip().lower()] = v.strip()
    n = int(headers["content-length"])
    return json.loads(env_proc.stdout.read(n))


send2({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
recv2()
send2({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send2({
    "jsonrpc": "2.0",
    "method": "textDocument/didOpen",
    "params": {
        "textDocument": {"uri": "file:///tmp/whatever/app.op", "version": 1},
        "text": "use math\nmain { let r = mean([1, 2, 3]) }\n",
    },
})
d = recv2()
msgs = [x["message"] for x in d["params"]["diagnostics"]]
assert not any("phantom" in m and "mean" in m for m in msgs), msgs
send2({"jsonrpc": "2.0", "id": 9, "method": "shutdown"})
recv2()
send2({"jsonrpc": "2.0", "method": "exit"})
env_proc.wait(timeout=5)
assert env_proc.returncode == 0, env_proc.returncode

print(
    "LSP smoke: OK (initialize+caps+operonLsp, diagnostics, hover, definition, "
    "symbols, completion, formatting, didClose, CWD-independence, -32601, shutdown/exit)"
)

# W62: `--version` prints the machine-readable pin pair and never touches stdio
_v = subprocess.run([BIN, "--version"], capture_output=True, text=True, timeout=10)
assert _v.returncode == 0 and "/ lsp 1" in _v.stdout and _v.stdout.startswith("operon "), _v
# unknown args are refused loudly (exit 2), protecting editors from typo args
_bad = subprocess.run([BIN, "--bogus"], capture_output=True, text=True, timeout=10)
assert _bad.returncode == 2 and "--version" in _bad.stderr, _bad
print("LSP smoke: version contract OK (--version pin line, unknown-arg refusal)")

# W46: --explain FILE wraps the W38 Total Grammar report for a workspace file
import tempfile as _tf
_wob = _tf.NamedTemporaryFile("w", suffix=".op", delete=False)
_wob.write("gene g() { retrn 1 }\n")
_wob.close()
_e = subprocess.run([BIN, "--explain", _wob.name], capture_output=True, text=True, timeout=10)
assert _e.returncode == 0, _e
assert "Total Grammar report" in _e.stdout and "wobble" in _e.stdout, _e.stdout
assert "retrn" in _e.stdout and "return" in _e.stdout, _e.stdout
_missing = subprocess.run([BIN, "--explain", "/nonexistent/x.op"], capture_output=True, text=True, timeout=10)
assert _missing.returncode == 2, _missing
print("LSP smoke: --explain OK (W38 wrap, missing-file refusal)")
