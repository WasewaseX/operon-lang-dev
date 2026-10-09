#!/usr/bin/env python3
"""dap_reentry_e2e.py — the Z-121 (#121 item 2) protocol pin: a DAP evaluate
that executes debuggee code must NEVER re-enter the trap mid-request.

Contract pinned here (pre-fix main fails every "no events between" row):
  1. After `next` stops (debug_step pending, debug_step_depth cleared), an
     `evaluate` whose call crosses statements returns its response as the
     VERY NEXT protocol message — no stopped event, no output event, no
     second request loop on the pipe (the pre-fix bug: debug_step survives
     the stop with depth None => depth_ok, so the evaluated call's first
     statement re-armed the trap and the pending response was deferred
     behind a nested serve_at_trap forever).
  2. A breakpoint INSIDE the evaluated call does not fire during the
     evaluate either (no hidden stops) — the evaluated work() crosses the
     breakpointed print line and must stay silent mid-request.
  3. The evaluated calls' side effects are exactly the requested ones (an
     evaluated work() prints its marker — that is the USER asking, not a
     bug); the pre-fix hidden re-execution class is gone, and the outer
     run's own semantics are unchanged (x = the one real bump() result).
  4. The session still terminates cleanly (exited 0) with exactly the
     expected stop count.
"""
import json
import os
import subprocess
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OP = os.path.join(REPO, "bin", "operon")

PROG = """let runs = 0
gene bump() {
    runs = runs + 100
    return runs
}
gene work() {
    runs = runs + 1
    print("work-ran")
    return bump()
}
main {
    let x = work()
    print(x)
}
"""
# line map: 1 let runs / 2 gene bump / 3 runs+=100 / 4 return / 5 }
#           6 gene work / 7 runs+=1 / 8 print / 9 return bump() / 10 }
#           11 main { / 12 let x = work() / 13 print(x)


class DapClient:
    def __init__(self, path):
        self.p = subprocess.Popen(
            [OP, "dap", path], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        self.seq = 0
        self.events = []
        self.stopped_events = []

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
        n = None
        for l in headers.decode().splitlines():
            if l.lower().startswith("content-length:"):
                n = int(l.split(":", 1)[1].strip())
        assert n is not None, headers
        body = b""
        while len(body) < n:
            chunk = self.p.stdout.read(n - len(body))
            if not chunk:
                raise AssertionError("DAP EOF mid-body")
            body += chunk
        return json.loads(body)

    def request_strict(self, command, args=None):
        """Z-121 core assertion helper: the response to this request must be
        the VERY NEXT protocol message — ZERO events in between (no stopped,
        no output, no second request loop). Returns (response, events_seen)."""
        seq = self.send(command, args)
        seen = []
        while True:
            m = self.read_msg()
            if m.get("type") == "response" and m.get("request_seq") == seq:
                return m, seen
            seen.append(m)
            if m.get("type") == "event" and m.get("event") == "stopped":
                self.stopped_events.append(m)

    def request(self, command, args=None):
        seq = self.send(command, args)
        while True:
            m = self.read_msg()
            if m.get("type") == "response" and m.get("request_seq") == seq:
                return m
            self.events.append(m)
            if m.get("type") == "event" and m.get("event") == "stopped":
                self.stopped_events.append(m)

    def wait_event(self, name, timeout_frames=100):
        for i, m in enumerate(self.events):
            if m.get("type") == "event" and m.get("event") == name:
                return self.events.pop(i)
        for _ in range(timeout_frames):
            m = self.read_msg()
            if m.get("type") == "event" and m.get("event") == "stopped":
                self.stopped_events.append(m)
            if m.get("type") == "event" and m.get("event") == name:
                return m
            self.events.append(m)
        raise AssertionError(f"no {name} event")

    def all_output(self):
        texts = []
        for m in self.events:
            if m.get("type") == "event" and m.get("event") == "output":
                texts.append(m.get("body", {}).get("output", ""))
        return "".join(texts)


def main():
    sb = tempfile.mkdtemp()
    path = os.path.join(sb, "reentry.op")
    with open(path, "w") as f:
        f.write(PROG)

    c = DapClient(path)
    r = c.request("initialize", {"adapterID": "operon"})
    assert r["success"], r
    c.wait_event("initialized")

    c.request("launch", {"program": path})
    r = c.request("setBreakpoints", {
        "source": {"path": path},
        "breakpoints": [{"line": 8}],
    })
    assert r["success"], r
    c.request("configurationDone")

    # --- stop 1: bp 8 (print inside work) fires while main line 12 runs
    ev = c.wait_event("stopped")
    assert ev["body"]["reason"] == "breakpoint", ev
    r = c.request("stackTrace", {"threadId": 1})
    assert r["body"]["stackFrames"][0]["name"] == "work", r

    # --- next: resumes past line 8, line 9 `return bump()` runs to ITS own
    # line, stop 2 carries reason step (debug_step pending, depth cleared)
    r, between = c.request_strict("next")
    assert r["success"], r
    ev = c.wait_event("stopped")
    assert ev["body"]["reason"] == "step", ev
    assert not between or all(
        m.get("event") != "stopped" for m in between if m.get("type") == "event"), between

    # --- THE Z-121 ROW: evaluate a call that CROSSES statements while the
    # step request is still pending. Pre-fix main: bump's first statement
    # re-armed the trap (debug_step && depth_ok) -> a SECOND stopped event
    # arrived before the response and the response was deferred forever.
    # Post-fix: the response is the very next protocol message.
    r, between = c.request_strict("evaluate", {"expression": "bump()", "frameId": 0})
    assert r["success"], r
    assert r["body"]["result"] == "201", r  # runs: 1 (work) + 100 (line 9's bump) + 100 (evaluated bump)
    assert between == [], ("events between evaluate request and response", between)

    # --- second row: a breakpoint INSIDE the evaluated call must not fire
    # during the evaluate either. work() crosses the breakpointed line 8
    # print — pre-fix that was a hidden stopped(breakpoint) mid-request.
    # (Mid-session setBreakpoints is refused by the adapter with
    # notSupported — report-only finding, out of #121 scope — so the bp is
    # the one set up front.) The evaluated work's own print is a REQUESTED
    # side effect and shows up in the output stream exactly once.
    r, between = c.request_strict("evaluate", {"expression": "work()", "frameId": 0})
    assert r["success"], r
    # work: runs+1 -> 202 (the marker print), then its own bump() -> 302,
    # and work RETURNS bump()'s value = 302
    assert r["body"]["result"] == "302", r
    assert between == [], ("events between evaluate request and response", between)

    # --- the real run completes normally: NO third stop (bp 3's line never
    # re-executes on the real path), exited 0
    c.request("continue")
    m = c.read_msg()
    while not (m.get("type") == "event" and m.get("event") == "exited"):
        if m.get("type") == "event" and m.get("event") == "stopped":
            self_stops = c.stopped_events
            raise AssertionError("unexpected stop during continue: %s (all: %s)" % (m, self_stops))
        c.events.append(m)
        m = c.read_msg()
    assert m["body"]["exitCode"] == 0, m

    # exactly two stops in the whole session: the breakpoint + the step
    assert len(c.stopped_events) == 2, c.stopped_events

    # side-effect truth: work-ran appears exactly TWICE — once from the
    # real run, once from the explicitly evaluated work() (a requested
    # side effect, not a bug) — and NEVER a third time (the pre-fix
    # hidden re-execution class); the outer x is the one real bump()
    # result from line 9 (101) — the evaluated calls did NOT leak into
    # the program's own control flow beyond their real effect
    out = c.all_output()
    assert out.count("work-ran") == 2, ("work-ran count", out)
    assert "101" in out, ("outer print(x) must be 101", out)

    c.p.wait(timeout=10)

    print("DAP RE-ENTRY E2E OK (Z-121: evaluate is re-entrancy safe — "
          "no events between request and response, no hidden stops, no double execution)")


if __name__ == "__main__":
    main()
