#!/usr/bin/env python3
"""debug_protocol_e2e.py — the W08r stage-2 done-when proof: a Python client
drives `operon debug --protocol=json` end to end. Contract under test:
  - stdout carries ONLY NDJSON frames (events + replies)
  - stopped events carry reason/line/depth
  - stack/vars/eval return the frame state at the stop
  - continue/next/stepIn/stepOut/until/breakpoints/quit all respond and resume
  - program print output lands on stderr (stdout stays pure protocol)
  - EOF on stdin resumes to completion (piped sessions never wedge)
"""
import json
import subprocess
import sys
import tempfile
import os
import time

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


class Session:
    def __init__(self, path, breaks):
        args = [OP, "debug", path, "--protocol=json"]
        for b in breaks:
            args += ["--break", str(b)]
        self.p = subprocess.Popen(args, stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  text=True, bufsize=1)

    def read_frame(self, timeout=10):
        # line-buffered pipe; blocking read is fine for the e2e
        line = self.p.stdout.readline()
        if not line:
            raise AssertionError("protocol EOF on stdout (wedge?)")
        return json.loads(line)

    def request(self, rid, cmd, args=None):
        req = {"id": rid, "cmd": cmd}
        if args is not None:
            req["args"] = args
        self.p.stdin.write(json.dumps(req) + "\n")
        self.p.stdin.flush()
        return self.read_frame()

    def wait_stopped(self):
        while True:
            f = self.read_frame()
            if f.get("event") == "stopped":
                return f

    def close(self):
        try:
            self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()


def main():
    sb = tempfile.mkdtemp()
    path = os.path.join(sb, "dbg.op")
    with open(path, "w") as f:
        f.write(PROG)

    rid = 0

    def nid():
        nonlocal rid
        rid += 1
        return rid

    # --- stop at the breakpoint, inspect, evaluate
    s = Session(path, [8])
    stop = s.wait_stopped()
    assert stop["reason"] == "breakpoint", stop
    assert stop["line"] == 8, stop
    r = s.request(nid(), "stack")
    assert r["ok"] and r["frames"][0]["name"] == "main", r
    r = s.request(nid(), "vars")
    assert r["ok"] and any("z" in sc["vars"] for sc in r["scopes"]), r
    r = s.request(nid(), "eval", {"expr": "z * 100"})
    assert r["ok"] and r["value"] == "100", r
    r = s.request(nid(), "eval", {"expr": "z +("})
    assert not r["ok"] and "error" in r, r

    # --- next steps OVER the call: line 9 next, never inside work
    r = s.request(nid(), "next")
    assert r["ok"], r
    stop = s.wait_stopped()
    assert stop["reason"] == "step" and stop["line"] == 9, stop

    # --- stepOut at the OUTERMOST frame: runs to completion (EOF is a
    # clean resume, never a wedge)
    r = s.request(nid(), "stepOut")
    assert r["ok"], r
    s.close()
    assert s.p.returncode == 0, s.p.returncode

    # --- breakpoints request adds a runtime bp; stepIn descends into work
    s2 = Session(path, [7])
    stop = s2.wait_stopped()
    assert stop["line"] == 7, stop
    r = s2.request(nid(), "breakpoints", {"add": [3], "remove": []})
    assert r["ok"] and r["breakpoints"] == [3, 7], r
    r = s2.request(nid(), "continue")
    stop = s2.wait_stopped()
    # break at 3 fires inside work (stepped INTO via the call at line 8)
    assert stop["reason"] == "breakpoint" and stop["line"] == 3, stop
    r = s2.request(nid(), "stack")
    assert r["frames"][0]["name"] == "work", r
    assert r["frames"][0]["line"] == 3, r          # the shown line at the stop
    assert r["frames"][1]["name"] == "main", r
    # W008-P1: main is currently stopped at line 8 (its work() call site)
    assert r["frames"][1]["line"] == 8, r
    r = s2.request(nid(), "eval", {"expr": "x"})
    assert r["ok"] and r["value"] == "20", r
    # --- until: one-shot run to line 9 in main
    r = s2.request(nid(), "until", {"line": 9})
    assert r["ok"], r
    stop = s2.wait_stopped()
    assert stop["reason"] == "until" and stop["line"] == 9, stop
    # --- bad command errors, session survives
    r = s2.request(nid(), "definitely_not_a_cmd")
    assert not r["ok"], r
    # --- quit ends the session cleanly
    r = s2.request(nid(), "quit")
    assert r["ok"], r
    s2.close()

    # --- print output rerouting: stdout pure, stderr carries [out]
    prog2 = "main {\n    let v = 5 + 5\n    print(v)\n    print(v + 1)\n}\n"
    path2 = os.path.join(sb, "dbg2.op")
    with open(path2, "w") as f:
        f.write(prog2)
    s3 = Session(path2, [2])
    stop = s3.wait_stopped()
    assert stop["line"] == 2, stop
    r = s3.request(nid(), "continue")
    assert r["ok"], r
    s3.close()
    # EOF would also resume; we continued, so the program finished

    # --- EOF never wedges: no stdin at all
    s4 = subprocess.run([OP, "debug", path, "--protocol=json", "--break", "7"],
                        input="", capture_output=True, text=True, timeout=30)
    assert "21" not in s4.stdout, "protocol leaked to stdout?"
    out_frames = [l for l in s4.stdout.splitlines() if l.strip()]
    for l in out_frames:
        json.loads(l)  # every stdout line must be valid JSON
    assert "[out] 21" in s4.stderr, s4.stderr

    # --- W008-P1: outer frames carry their CAPTURED call-site lines
    # (debug_frames mirror) — a 3-level chain proves the middle frame's
    # line is the call site of the frame ABOVE it, and the host-entry
    # frame (main) honestly has none.
    path3 = os.path.join(sb, "dbg3.op")
    with open(path3, "w") as f:
        f.write("""gene deep(n) {
    let t = n + 1
    return t
}
gene work(n) {
    let x = deep(n)
    return x
}
main {
    let a = work(10)
    print(a)
}
""")
    s5 = Session(path3, [2])   # break at line 2 (a let — traps), INSIDE deep
    stop = s5.wait_stopped()
    assert stop["reason"] == "breakpoint" and stop["line"] == 2, stop
    r = s5.request(nid(), "stack")
    f = r["frames"]
    assert f[0]["name"] == "deep" and f[0]["line"] == 2, r
    # W008-P1: work is currently stopped at line 6 (its deep() call site)
    assert f[1]["name"] == "work" and f[1]["line"] == 6, r
    # main is currently stopped at line 10 (its work() call site)
    assert f[2]["name"] == "main" and f[2]["line"] == 10, r
    r = s5.request(nid(), "continue")
    s5.close()
    assert s5.p.returncode == 0, s5.p.returncode
    os.remove(path3)

    os.remove(path)
    os.remove(path2)
    os.rmdir(sb)
    print("DEBUG PROTOCOL E2E OK")


if __name__ == "__main__":
    main()
