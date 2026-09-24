#!/usr/bin/env python3
"""LSP smoke test (G5 seed → lsp-r1 v2): drives ./target/release/operon-ls.

Covers: initialize handshake + advertised capabilities, publishDiagnostics
(phantom call), hover with a gene signature, definition, documentSymbol,
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
caps = r["result"]["capabilities"]
assert caps["hoverProvider"] is True and caps["textDocumentSync"] == 1, caps
assert caps["definitionProvider"] is True, caps
assert caps["documentSymbolProvider"] is True, caps
assert caps["documentFormattingProvider"] is True, caps
assert caps["completionProvider"]["resolveProvider"] is False, caps

# 2. initialized notification (no reply expected)
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

# 3. didOpen → publishDiagnostics with a phantom-call error
text = (
    "gene boost(x) { return x }\n"
    "main { let y = boost(1) + missing(2) }\n"
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
    "LSP smoke: OK (initialize+caps, diagnostics, hover, definition, "
    "symbols, completion, formatting, didClose, CWD-independence, -32601, shutdown/exit)"
)
