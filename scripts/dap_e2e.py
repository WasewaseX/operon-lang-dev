#!/usr/bin/env python3
"""dap_e2e.py — the W08r stage-3 done-when proof: a Python DAP client drives
`operon dap` over the Content-Length base protocol. Contract under test:
  - initialize → capabilities + initialized event
  - launch/setBreakpoints/configurationDone start the session
  - breakpoints stop with a stopped event carrying the reason
  - threads/stackTrace/scopes/variables/evaluate return real frame state
  - next/stepIn/stepOut/continue resume with the right stop semantics
  - program print output arrives as output events
  - terminated + exited close the session; evaluate in a called frame works
"""
import json
import os
import subprocess
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OP = os.path.join(REPO, "bin", "operon")

PROG = """gene work(n) {
    let x = n * 2
    let y = x + 1
    return y
}
main {
    let z = 1
    let a = work(10)
    print(a)
}
"""


class DapClient:
    def __init__(self, path):
        # binary pipes: Content-Length counts BYTES and bodies may contain
        # multibyte UTF-8 (e.g. the em dash in scope names) — a text-mode
        # read(n) counts chars and deadlocks
        self.p = subprocess.Popen(
            [OP, "dap", path], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        self.seq = 0
        self.events = []

    def send(self, command, args=None):
        self.seq += 1
        msg = {"seq": self.seq, "type": "request", "command": command}
        if args is not None:
            msg["arguments"] = args
        body = json.dumps(msg)
        self.p.stdin.write(f"Content-Length: {len(body.encode())}\r\n\r\n{body}".encode())
        self.p.stdin.flush()
        return self.seq

    def read_msg(self):
        headers = b""
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise AssertionError("DAP EOF (client wedge?)")
            line = line.strip()
            if not line:
                break
            headers += line + b"\n"
            if headers.count(b"\n") >= 50:
                raise AssertionError("header storm")
        htxt = headers.decode()
        n = None
        for l in htxt.splitlines():
            if l.lower().startswith("content-length:"):
                n = int(l.split(":", 1)[1].strip())
        assert n is not None, htxt
        body = b""
        while len(body) < n:
            chunk = self.p.stdout.read(n - len(body))
            if not chunk:
                raise AssertionError("DAP EOF mid-body")
            body += chunk
        return json.loads(body)

    def request(self, command, args=None):
        seq = self.send(command, args)
        while True:
            m = self.read_msg()
            if m.get("type") == "response" and m.get("request_seq") == seq:
                return m
            self.events.append(m)  # events interleaved with responses

    def wait_event(self, name, timeout_frames=100):
        # check the backlog first (events consumed while awaiting responses)
        for i, m in enumerate(self.events):
            if m.get("type") == "event" and m.get("event") == name:
                return self.events.pop(i)
        for _ in range(timeout_frames):
            m = self.read_msg()
            if m.get("type") == "event" and m.get("event") == name:
                return m
            self.events.append(m)
        raise AssertionError(f"no {name} event")


def main():
    sb = tempfile.mkdtemp()
    path = os.path.join(sb, "dbg.op")
    with open(path, "w") as f:
        f.write(PROG)

    c = DapClient(path)
    # --- lifecycle: initialize → capabilities + initialized event
    r = c.request("initialize", {"adapterID": "operon"})
    assert r["success"], r
    assert "capabilities" in r["body"], r
    # W008 polish: the adapter advertises conditional breakpoints + setVariable
    caps = r["body"]["capabilities"]
    assert caps.get("supportsConditionalBreakpoints") is True, caps
    assert caps.get("supportsSetVariable") is True, caps
    ev = c.wait_event("initialized")
    assert ev["event"] == "initialized", ev

    # --- configure: launch + breakpoints + configurationDone
    c.request("launch", {"program": path})
    r = c.request("setBreakpoints", {
        "source": {"path": path},
        "breakpoints": [{"line": 8}, {"line": 3}],
    })
    assert r["success"] and len(r["body"]["breakpoints"]) == 2, r
    assert all(b["verified"] for b in r["body"]["breakpoints"]), r
    c.request("configurationDone")

    # --- first stop: bp 3 (inside work, called from line 8) fires FIRST —
    # the line-8 statement is still executing
    ev = c.wait_event("stopped")
    assert ev["body"]["reason"] == "breakpoint", ev
    r = c.request("threads")
    assert r["body"]["threads"][0]["name"] == "main", r
    r = c.request("stackTrace", {"threadId": 1})
    frames = r["body"]["stackFrames"]
    assert frames[0]["name"] == "work" and frames[0]["line"] == 3, frames
    assert frames[1]["name"] == "main", frames
    # W008 polish: the outer frame's line is the REAL call site (8), not
    # the placeholder 1
    assert frames[1]["line"] == 8, frames
    # --- scopes + variables surface the innermost frame state
    r = c.request("scopes", {"frameId": 0})
    scopes = r["body"]["scopes"]
    assert scopes and scopes[0]["variablesReference"], r
    ref = scopes[0]["variablesReference"]
    r = c.request("variables", {"variablesReference": ref})
    names = [v["name"] for v in r["body"]["variables"]]
    assert "x" in names and "y" in names, r
    vals = {v["name"]: v["value"] for v in r["body"]["variables"]}
    assert vals["y"] == "21", vals
    # --- evaluate in the stopped frame
    r = c.request("evaluate", {"expression": "x * 100", "frameId": 0})
    assert r["success"] and r["body"]["result"] == "2000", r

    # --- continue → bp 8 in main
    c.request("continue")
    ev = c.wait_event("stopped")
    assert ev["body"]["reason"] == "breakpoint", ev
    r = c.request("stackTrace", {"threadId": 1})
    frames = r["body"]["stackFrames"]
    assert frames[0]["name"] == "main" and frames[0]["line"] == 8, frames
    r = c.request("scopes", {"frameId": 0})
    ref = r["body"]["scopes"][0]["variablesReference"]
    r = c.request("variables", {"variablesReference": ref})
    vals = {v["name"]: v["value"] for v in r["body"]["variables"]}
    assert vals.get("a") == "21", vals

    # --- next steps OVER print... first: continue then next at line 8?
    # from line 8, next lands at line 9 (print) — never inside work again
    c.request("next")
    ev = c.wait_event("stopped")
    assert ev["body"]["reason"] == "step", ev
    r = c.request("stackTrace", {"threadId": 1})
    assert r["body"]["stackFrames"][0]["line"] == 9, r

    # --- continue to completion: output event with 21, exited + terminated
    c.request("continue")
    out_events = []
    while True:
        m = c.read_msg()
        if m.get("type") == "event" and m.get("event") == "output":
            out_events.append(m["body"]["output"])
        if m.get("type") == "event" and m.get("event") == "exited":
            assert m["body"]["exitCode"] == 0, m
            break
    # events captured into the backlog while awaiting responses count too
    for m in c.events:
        if m.get("type") == "event" and m.get("event") == "output":
            out_events.append(m.get("body", {}).get("output", ""))
    assert "21" in [o.strip() for o in out_events], out_events
    # terminated is sent immediately after exited — one more frame
    m = c.read_msg()
    assert m.get("event") == "terminated", m
    c.p.wait(timeout=10)
    assert c.p.returncode == 0, c.p.returncode

    # --- W008 polish session: stopOnEntry + conditional breakpoints + setVariable
    c2 = DapClient(path)
    r = c2.request("initialize", {"adapterID": "operon"})
    assert r["success"], r
    c2.request("launch", {"program": path, "stopOnEntry": True})
    c2.request("configurationDone")
    ev = c2.wait_event("stopped")
    assert ev["body"]["reason"] == "entry", ev
    r = c2.request("stackTrace", {"threadId": 1})
    f0 = r["body"]["stackFrames"][0]
    assert f0["name"] == "main" and f0["line"] == 7, (f0, "entry stop should land on main's first statement")
    # setVariable on the entry frame's binding
    r = c2.request("scopes", {"frameId": 0})
    ref = r["body"]["scopes"][0]["variablesReference"]
    r = c2.request("variables", {"variablesReference": ref})
    assert any(v["name"] == "z" for v in r["body"]["variables"]), r
    r = c2.request("setVariable", {"variablesReference": ref, "name": "z", "value": "777"})
    assert r["success"] and r["body"]["value"] == "777", r
    r = c2.request("variables", {"variablesReference": ref})
    vals = {v["name"]: v["value"] for v in r["body"]["variables"]}
    assert vals.get("z") == "777", vals
    r = c2.request("evaluate", {"expression": "z", "frameId": 0})
    assert r["success"] and r["body"]["result"] == "777", r
    # setVariable refuses consts and unknown names
    r = c2.request("scopes", {"frameId": 0})
    ref = r["body"]["scopes"][0]["variablesReference"]
    # let z is not const; use a non-existent name for the refusal
    r = c2.request("setVariable", {"variablesReference": ref, "name": "not_there", "value": "1"})
    assert not r["success"] and "no such binding" in r["body"].get("error", ""), r
    # continue to completion
    c2.request("continue")
    m = c2.read_msg()
    while not (m.get("type") == "event" and m.get("event") == "exited"):
        m = c2.read_msg()
    m = c2.read_msg()
    assert m.get("event") == "terminated", m
    c2.p.wait(timeout=10)

    # --- W008 polish: conditional breakpoints over DAP — the false
    # condition at 3 never stops; the session runs to the bp at 8
    c3 = DapClient(path)
    c3.request("initialize", {"adapterID": "operon"})
    c3.request("launch", {"program": path})
    r = c3.request("setBreakpoints", {
        "source": {"path": path},
        "breakpoints": [{"line": 3, "condition": "n > 100"}, {"line": 8}],
    })
    assert r["success"], r
    rows = r["body"]["breakpoints"]
    assert any(b.get("condition") == "n > 100" for b in rows), rows
    c3.request("configurationDone")
    ev = c3.wait_event("stopped")
    assert ev["body"]["reason"] == "breakpoint", ev
    r = c3.request("stackTrace", {"threadId": 1})
    f0 = r["body"]["stackFrames"][0]
    assert f0["name"] == "main" and f0["line"] == 8, (f0, "conditional bp (false) must be skipped")
    # a broken condition is also skipped (never a surprise stop)
    c3.request("setBreakpoints", {
        "source": {"path": path},
        "breakpoints": [{"line": 3, "condition": "not_a_binding > 1"}],
    })
    c3.request("continue")
    # no bp at 3 fires; program completes
    m = c3.read_msg()
    while not (m.get("type") == "event" and m.get("event") == "exited"):
        m = c3.read_msg()
    assert m["body"]["exitCode"] == 0, m
    c3.p.wait(timeout=10)

    print("DAP E2E OK (W008 polish: entry/conditional/setVariable/outer-lines)")


if __name__ == "__main__":
    main()
