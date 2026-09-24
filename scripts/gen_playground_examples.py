#!/usr/bin/env python3
"""gen_playground_examples.py — B4 (builder-B).

Generates web/playground/examples.data.js:
  - window.PLAYGROUND_EXAMPLES: the playground example dropdown manifest,
    fed from examples/cookbook/*.op (+ the classic editor default)
  - window.PLAYGROUND_VERSION: parsed from Cargo.toml so the page banner
    matches the core version string (MASTER-PLAN B4 done-gate)

Every candidate example is EXECUTED against the playground's own JS
interpreter (web/playground/app.js, driven via node) and its stdout is
diffed against examples/cookbook/expected/<name>.out — the same frozen
differential outputs the Rust core and the Python oracle verify. Only
programs that pass in the subset ship with verified: true; everything
else is included as verified: false (shown, labeled, never silently
wrong) or skipped entirely for the dropdown (see --strict).

Usage:
  python3 scripts/gen_playground_examples.py            # regenerate manifest
  python3 scripts/gen_playground_examples.py --strict   # drop unverified ones
  python3 scripts/gen_playground_examples.py --report   # print verification table
"""
import argparse
import base64
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PG = ROOT / "web" / "playground"
COOK = ROOT / "examples" / "cookbook"
EXPECT = COOK / "expected"
NODE_DRIVER = PG / "_node_driver.js"

DEFAULT_DEMO = """gene fib(n) {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}

promote("fib(10) = {fib(10)}")

let xs = [5, 3, 8]
promote("sorted: {xs.sort()}")

les typo = "repaired, not rejected"
promote("wobble: {typo}")"""

SKIP_NAMES = {"bank_account"}  # subset skips stress/rescue; keep it out of the dropdown


def cargo_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    if not m:
        raise SystemExit("cannot parse Cargo.toml version")
    return m.group(1)


def header_field(path: pathlib.Path, key: str) -> str:
    """Pull '# key: value' from a cookbook header block."""
    for line in path.read_text(encoding="utf-8").splitlines():
        m = re.match(rf"^#\s*{re.escape(key)}:\s*(.+?)\s*$", line)
        if m:
            return m.group(1)
    return ""


def run_in_subset(code: str) -> dict:
    """Execute program text through the playground JS interpreter via node."""
    driver = f"""
const {{ runProgram }} = require({json.dumps(str(PG / 'app.js'))});
const code = {json.dumps(code)};
const r = runProgram(code);
process.stdout.write(JSON.stringify({{
    output: r.output, notes: r.notes || [], score: r.score
}}));
"""
    NODE_DRIVER.write_text(driver, encoding="utf-8")
    proc = subprocess.run(
        ["node", str(NODE_DRIVER)],
        capture_output=True, text=True, timeout=60,
        cwd=str(ROOT),
    )
    if proc.returncode != 0:
        return {"ok": False, "error": proc.stderr.strip().splitlines()[-1:] or ["node failed"]}
    try:
        return {"ok": True, **json.loads(proc.stdout)}
    except json.JSONDecodeError:
        return {"ok": False, "error": ["driver returned non-JSON"]}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--strict", action="store_true", help="drop unverified examples")
    ap.add_argument("--report", action="store_true", help="print the verification table")
    args = ap.parse_args()

    version = cargo_version()
    entries = [{
        "name": "welcome",
        "title": "Welcome — the wobble ladder",
        "teaches": "genes, interpolation, sort, and a typo the parser repairs instead of rejecting",
        "code": DEFAULT_DEMO,
        "verified": None,  # the built-in default; verified by smoke.js instead
        "expected": None,
    }]

    report = []
    for prog in sorted(COOK.glob("*.op")):
        name = prog.stem
        code = prog.read_text(encoding="utf-8")
        expected_path = EXPECT / f"{name}.out"
        expected = expected_path.read_text(encoding="utf-8").rstrip("\n") if expected_path.exists() else None
        res = run_in_subset(code)
        got = "\n".join(res.get("output", [])) if res.get("ok") else None
        verified = bool(expected) and got is not None and got.strip() == expected.strip()
        report.append((name, verified, res))
        entries.append({
            "name": name,
            "title": header_field(prog, "title").split("—")[0].strip() or name,
            "teaches": header_field(prog, "teaches"),
            "code": code,
            "verified": verified,
            "expected": expected,
        })

    NODE_DRIVER.unlink(missing_ok=True)

    if args.report:
        for name, verified, res in report:
            mark = "PASS" if verified else "fail"
            extra = "" if verified else f"  {res.get('error') or 'output differs'}"
            print(f"{mark:4} {name}{extra}")
        print(f"summary: {sum(1 for _, v, _ in report if v)}/{len(report)} cookbook programs verified in the JS subset")

    keep = [e for e in entries
            if e["verified"] is None or e["verified"] or not args.strict] \
        if not SKIP_NAMES else \
           [e for e in entries
            if e["name"] not in SKIP_NAMES and (e["verified"] is None or e["verified"] or not args.strict)]

    js = ("// GENERATED by scripts/gen_playground_examples.py — do not edit by hand.\n"
          "// Every verified:true entry ran through app.js and matched the same frozen\n"
          "// output the Rust core and the Python oracle verify (differential honesty).\n"
          f"window.PLAYGROUND_VERSION = {json.dumps(version)};\n"
          f"window.PLAYGROUND_EXAMPLES = {json.dumps(keep, indent=2)};\n")
    (PG / "examples.data.js").write_text(js, encoding="utf-8")

    n_ok = sum(1 for e in keep if e["verified"]) 
    print(f"wrote web/playground/examples.data.js — {len(keep)} entries "
          f"({n_ok} subset-verified, core version {version})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
