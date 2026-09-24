#!/usr/bin/env python3
"""LSP seed smoke test (G5): drives ./target/release/operon-ls over stdio.

Covers: initialize handshake, publishDiagnostics (phantom call), hover with
a gene signature, unknown-method error, shutdown/exit. Run from repo root:
    python3 tests/lsp_smoke.py
Exits 0 on success; asserts loudly otherwise.
"""
import json
import subprocess
import sys

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


# 1. initialize handshake
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
r = recv()
assert r["id"] == 1 and r["result"]["serverInfo"]["name"] == "operon-ls", r
caps = r["result"]["capabilities"]
assert caps["hoverProvider"] is True and caps["textDocumentSync"] == 1, caps

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

# 5a. hover over `main` → main IS a gene (the entry gene), signature shown
send({
    "jsonrpc": "2.0",
    "id": 3,
    "method": "textDocument/hover",
    "params": {
        "textDocument": {"uri": "file:///demo.op"},
        "position": {"line": 1, "character": 0},
    },
})
r = recv()
assert r["id"] == 3 and "gene main()" in r["result"]["contents"]["value"], r

# 5b. hover over a non-identifier (the `{`) → null result
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

print("LSP smoke: OK (initialize, diagnostics, hover, -32601, shutdown/exit)")
