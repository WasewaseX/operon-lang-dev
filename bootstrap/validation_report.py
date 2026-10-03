#!/usr/bin/env python3
"""validation_report.py, W093 remainder: the machine-readable validation report.

For every entry in bootstrap/validation_registry.json (the SINGLE SOURCE of
truth for expected values, comparison kinds and tolerance magnitudes) this
script:

  1. runs the entry's enforcing test(s) through the repo's normal runners
     (./bin/operon test AND python3 bootstrap/oracle.py test, the same proof
     lanes scripts/test.sh drives),
  2. runs the entry's probe program(s) (bootstrap/validation_probes/*.op) on
     BOTH cores and requires byte-identical RESULT lines (probe parity),
  3. compares each registered quantity's observed value against the
     registry expectation using the registry's comparison kind and
     tolerance, and
  4. emits a machine-readable JSON report (entry id, expected, observed,
     tolerance, pass/fail) plus a human summary on stderr.

NO tolerance constant lives in this file. The TOLERANCE FRAMEWORK table in
docs/spec/VALIDATION.md is GENERATED from the registry (--emit-table) and
verified against it (--check-doc), so the doc can never drift from the
registry.

Usage:
  python3 bootstrap/validation_report.py                    # report to stdout
  python3 bootstrap/validation_report.py --out report.json  # report to file
  python3 bootstrap/validation_report.py --entry V7         # single entry
  python3 bootstrap/validation_report.py --emit-table       # generated block
  python3 bootstrap/validation_report.py --check-doc        # doc drift gate

Exit 0 iff every enforcing test is green on both cores, every probe is
parity-clean and every registered quantity passes its comparison.
"""
import argparse, json, os, platform, re, subprocess, sys
from datetime import datetime, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REGISTRY = os.path.join(ROOT, "bootstrap", "validation_registry.json")
DOC = os.path.join(ROOT, "docs", "spec", "VALIDATION.md")
BIN = os.path.join(ROOT, "bin", "operon")
ORACLE = os.path.join(ROOT, "bootstrap", "oracle.py")

BEGIN_MARK = "<!-- BEGIN GENERATED: tolerance-framework"
END_MARK = "<!-- END GENERATED: tolerance-framework -->"

# ---------------------------------------------------------------------------
# registry access (the single source of truth)

def load_registry():
    with open(REGISTRY, encoding="utf-8") as f:
        return json.load(f)

# ---------------------------------------------------------------------------
# probe RESULT parsing and value typing

RESULT_RE = re.compile(r"^RESULT (\S+)=(.*)$")

def parse_results(stdout):
    """Extract `RESULT name=value` lines as an ordered (name, value) list."""
    out = []
    for line in stdout.splitlines():
        m = RESULT_RE.match(line.strip())
        if not m:
            continue
        out.append((m.group(1), parse_value(m.group(2))))
    return out

def parse_value(text):
    """Operon-printed scalars: true/false/null, int, float, else raw string."""
    if text == "true":
        return True
    if text == "false":
        return False
    if text == "null":
        return None
    if re.fullmatch(r"-?\d+", text):
        try:
            return int(text)
        except ValueError:
            return text
    try:
        return float(text)
    except ValueError:
        return text

# ---------------------------------------------------------------------------
# comparisons (kinds defined in the registry's framework block)

def _num(x):
    return isinstance(x, (int, float)) and not isinstance(x, bool)

def sigfig(x, n):
    if x == 0:
        return 0.0
    return round(x, -int(floor_log10(abs(x))) + (n - 1))

def floor_log10(x):
    import math
    return math.floor(math.log10(x))

def exact_equal(expected, observed):
    # bool/int pitfall: in Python True == 1, so compare types first
    if isinstance(expected, bool) or isinstance(observed, bool):
        return isinstance(expected, bool) and isinstance(observed, bool) \
            and expected == observed
    if expected is None or observed is None:
        return expected is None and observed is None
    if isinstance(expected, str) or isinstance(observed, str):
        return str(expected) == str(observed)
    if _num(expected) and _num(observed):
        # floats: bit-identical means identical formatted value (SPEC 19);
        # the shared formatting rule makes this the honest bit-level compare
        return repr(float(expected)) == repr(float(observed)) \
            if isinstance(expected, float) or isinstance(observed, float) \
            else int(expected) == int(observed)
    return expected == observed

def compare(kind, expected, observed, tolerance):
    """Return (ok, detail). Kinds are defined in the registry framework block."""
    if kind == "exact":
        ok = exact_equal(expected, observed)
        return ok, f"exact: expected {fmt(expected)}, observed {fmt(observed)}"
    if not _num(expected) or not _num(observed):
        return False, (f"non-numeric observed {fmt(observed)} for {kind} "
                       f"comparison against {fmt(expected)}")
    if kind == "absolute":
        gap = abs(observed - expected)
        return gap < tolerance, f"|{fmt(observed)} - {fmt(expected)}| = {gap:.3e} < {fmt(tolerance)}"
    if kind == "relative":
        gap = abs(observed - expected)
        scale = abs(expected) if expected != 0 else 1.0
        return gap < tolerance * scale, f"|{fmt(observed)} - {fmt(expected)}| = {gap:.3e} < {fmt(tolerance)} * |{fmt(expected)}|"
    if kind == "significant-figure":
        ok = sigfig(observed, int(tolerance)) == sigfig(expected, int(tolerance))
        return ok, (f"{int(tolerance)} significant digits: "
                    f"expected {fmt(expected)}, observed {fmt(observed)}")
    return False, f"unknown comparison kind '{kind}' in the registry"

def fmt(x):
    if x is None:
        return "null"
    if isinstance(x, bool):
        return "true" if x else "false"
    if isinstance(x, float):
        return repr(x)
    return str(x)

# ---------------------------------------------------------------------------
# subprocess helpers (decode as utf-8 explicitly, like bootstrap/harness.py)

def run(cmd, timeout=120):
    p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                       errors="replace", timeout=timeout, cwd=ROOT)
    return p.stdout, p.returncode, p.stderr

def engine_version():
    try:
        out, code, _ = run([BIN, "version"])
        return out.strip() if code == 0 else "unknown"
    except OSError:
        return "binary missing"

def rust_toolchain():
    try:
        out, code, _ = run(["rustc", "--version"])
        return out.strip() if code == 0 else "rustc not on PATH"
    except OSError:
        return "rustc not on PATH"

# ---------------------------------------------------------------------------
# per-entry evaluation

def run_enforcing_test(binary_cmd, test_file):
    """Run one enforcing test; returns (green, parsed_summary_dict)."""
    out, code, _err = run(binary_cmd + ["test", test_file, "--json"])
    info = {"file": test_file, "runner": binary_cmd[0] if binary_cmd else "?",
            "exit_code": code, "green": code == 0, "proofs": None,
            "passed": None, "failed": None, "asserts": None}
    # the Rust runner prints one JSON summary line with --json; the oracle
    # prints the human summary (or nothing usable), so parse defensively
    for line in reversed(out.strip().splitlines()):
        line = line.strip()
        if line.startswith("{") and '"proofs"' in line:
            try:
                j = json.loads(line)
                info["proofs"] = j.get("proofs")
                info["passed"] = j.get("passed")
                info["failed"] = j.get("failed")
                info["asserts"] = j.get("asserts")
            except json.JSONDecodeError:
                pass
            break
        m = re.match(r"operon test, (\d+) file\(s\), (\d+) proof\(s\): "
                     r"(\d+) passed, (\d+) failed", line)
        if m:
            info["proofs"] = int(m.group(2))
            info["passed"] = int(m.group(3))
            info["failed"] = int(m.group(4))
            break
    return info

def evaluate_entry(entry, rust_bin, oracle_py, skip_oracle):
    """Run one registry entry end-to-end. Returns its report dict."""
    rid = entry["id"]
    rep = {"id": rid, "title": entry["title"], "tests": [], "probe_parity": {},
           "quantities": [], "pass": True}

    # 1. enforcing tests, both runners
    for tf in entry["enforcing_tests"]:
        t_rust = run_enforcing_test([rust_bin], tf)
        t_rust["runner"] = "rust-core"
        rep["tests"].append(t_rust)
        if not t_rust["green"]:
            rep["pass"] = False
        if not skip_oracle:
            t_or = run_enforcing_test([sys.executable, oracle_py], tf)
            t_or["runner"] = "python-oracle"
            rep["tests"].append(t_or)
            if not t_or["green"]:
                rep["pass"] = False

    # 2. probes on both cores, byte-identical RESULT lines
    observed = {}
    for probe in entry["probes"]:
        rel = os.path.relpath(probe, ROOT)
        r_out, r_code, _ = run([rust_bin, "run", probe])
        o_out, o_code, _ = run([sys.executable, oracle_py, "run", probe])
        r_res = parse_results(r_out)
        o_res = parse_results(o_out)
        parity = "match" if (r_code == 0 and o_code == 0
                             and r_res == o_res) else "diverge"
        rep["probe_parity"][rel] = parity
        if parity != "match":
            rep["pass"] = False
        for name, value in r_res:
            observed[name] = value

    # 3. quantity comparisons against the registry (never against this file)
    for q in entry["quantities"]:
        name = q["name"]
        kind = q["comparison"]
        tol = q.get("tolerance")
        expected = q["expected"]
        if name not in observed:
            rep["quantities"].append({
                "name": name, "expected": expected, "observed": None,
                "comparison": kind, "tolerance": tol, "pass": False,
                "detail": "probe did not report this quantity"})
            rep["pass"] = False
            continue
        obs = observed[name]
        ok, detail = compare(kind, expected, obs, tol)
        rep["quantities"].append({
            "name": name, "expected": expected, "observed": obs,
            "comparison": kind, "tolerance": tol, "pass": ok,
            "detail": detail})
        if not ok:
            rep["pass"] = False
    return rep

# ---------------------------------------------------------------------------
# generated doc block (the TOLERANCE FRAMEWORK table in VALIDATION.md)

def tol_cell(q):
    if q["comparison"] == "exact":
        return "bit-identical"
    if q["comparison"] == "significant-figure":
        return f"{fmt(q['tolerance'])} significant digits"
    return f"`{fmt(q['tolerance'])}`"

def emit_table(reg):
    lines = []
    for e in reg["entries"]:
        lines.append(f"### {e['id']}, {e['title']}")
        lines.append("")
        lines.append(f"Source: {e['source']}")
        lines.append("")
        lines.append("| Quantity | Expected | Comparison | Tolerance | Derivation |")
        lines.append("|---|---|---|---|---|")
        for q in e["quantities"]:
            exp = f"`{fmt(q['expected'])}`"
            der = q["derivation"].replace("|", "\\|")
            lines.append(f"| `{q['name']}` | {exp} | {q['comparison']} | "
                         f"{tol_cell(q)} | {der} |")
        lines.append("")
        lines.append(f"Why this magnitude is defensible: {e['why']}")
        lines.append("")
        tests = ", ".join(f"`{t}`" for t in e["enforcing_tests"])
        probes = ", ".join(f"`{p}`" for p in e["probes"])
        lines.append(f"Enforcing tests: {tests}. Measurement probes: {probes}.")
        lines.append("")
        lines.append(f"Repro (end-to-end): `{e['repro']}`")
        lines.append("")
    return "\n".join(lines).rstrip("\n")

def check_doc(reg):
    try:
        with open(DOC, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        print(f"doc drift: {os.path.relpath(DOC, ROOT)} missing", file=sys.stderr)
        return 1
    begin = text.find(BEGIN_MARK)
    end = text.find(END_MARK)
    if begin == -1 or end == -1 or end < begin:
        print("doc drift: generated block markers missing from "
              f"{os.path.relpath(DOC, ROOT)}", file=sys.stderr)
        return 1
    # the block is everything after the BEGIN marker line, up to END marker
    line_end = text.find("\n", begin)
    block = text[line_end + 1:end]
    want = emit_table(reg)
    if block.strip() != want.strip():
        print("doc drift: TOLERANCE FRAMEWORK generated block does not match "
              "bootstrap/validation_registry.json; regenerate with "
              "`python3 bootstrap/validation_report.py --emit-table` and paste "
              "the block between the GENERATED markers", file=sys.stderr)
        return 1
    print("doc block OK: TOLERANCE FRAMEWORK matches "
          "bootstrap/validation_registry.json", file=sys.stderr)
    return 0

# ---------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", help="write the JSON report to this file (default: stdout)")
    ap.add_argument("--entry", help="evaluate a single registry entry by id")
    ap.add_argument("--bin", default=BIN, help="Rust core binary (default bin/operon)")
    ap.add_argument("--skip-oracle", action="store_true",
                    help="skip the python-oracle enforcing-test lane (probe parity stays on)")
    ap.add_argument("--emit-table", action="store_true",
                    help="print the generated TOLERANCE FRAMEWORK block and exit")
    ap.add_argument("--check-doc", action="store_true",
                    help="verify the doc's generated block matches the registry and exit")
    args = ap.parse_args()

    reg = load_registry()

    if args.emit_table:
        print(emit_table(reg))
        return 0
    if args.check_doc:
        return check_doc(reg)

    entries = reg["entries"]
    if args.entry:
        entries = [e for e in entries if e["id"] == args.entry]
        if not entries:
            print(f"registry has no entry '{args.entry}'", file=sys.stderr)
            return 2

    oracle_py = os.path.join(ROOT, "bootstrap", "oracle.py")
    reps = [evaluate_entry(e, args.bin, oracle_py, args.skip_oracle)
            for e in entries]

    n_q = sum(len(r["quantities"]) for r in reps)
    n_pass = sum(1 for r in reps for q in r["quantities"] if q["pass"])
    n_test_fail = sum(1 for r in reps for t in r["tests"] if not t["green"])
    n_parity_fail = sum(1 for r in reps
                        for v in r["probe_parity"].values() if v != "match")

    report = {
        "schema": "operon-validation-report/1",
        # informational only: the determinism contract never pins wall-clock
        # values, and this timestamp is never asserted anywhere
        "generated_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "registry": os.path.relpath(REGISTRY, ROOT),
        "framework": reg["framework"],
        "engine": {
            "binary": os.path.relpath(args.bin, ROOT),
            "version": engine_version(),
            "rust_toolchain": rust_toolchain(),
            "python": platform.python_version(),
            "platform": f"{platform.system()}-{platform.machine()}",
        },
        "entries": reps,
        "summary": {
            "entries": len(reps),
            "quantities": n_q,
            "quantities_passed": n_pass,
            "quantities_failed": n_q - n_pass,
            "test_runs_failed": n_test_fail,
            "probe_parity_divergences": n_parity_fail,
            "all_green": n_pass == n_q and n_test_fail == 0 and n_parity_fail == 0,
        },
    }

    blob = json.dumps(report, indent=2, ensure_ascii=False) + "\n"
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(blob)
        print(f"report written to {args.out}", file=sys.stderr)
    else:
        print(blob)

    for r in reps:
        q_ok = sum(1 for q in r["quantities"] if q["pass"])
        t_ok = sum(1 for t in r["tests"] if t["green"])
        par = "match" if all(v == "match" for v in r["probe_parity"].values()) \
            else "DIVERGE"
        status = "PASS" if r["pass"] else "FAIL"
        print(f"  {status} {r['id']}: quantities {q_ok}/{len(r['quantities'])}, "
              f"tests {t_ok}/{len(r['tests'])} green, probe parity {par}",
              file=sys.stderr)
    s = report["summary"]
    verdict = "ALL GREEN" if s["all_green"] else "RED, fix before merge"
    print(f"validation report: {s['entries']} entries, {s['quantities']} "
          f"quantities, {s['quantities_passed']} pass, "
          f"{s['quantities_failed']} fail, {s['test_runs_failed']} test-run "
          f"failures, {s['probe_parity_divergences']} parity divergences, "
          f"{verdict}", file=sys.stderr)
    return 0 if s["all_green"] else 1

if __name__ == "__main__":
    sys.exit(main())
