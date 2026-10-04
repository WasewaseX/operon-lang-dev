#!/usr/bin/env python3
# bench_loglens.py — app-level cross-language benchmark:
# Operon vs Python (Bar B) vs Node.js (mainstream runtime) vs native
# Rust (Bar C), per the roadmap's three-bar methodology (§6).
#
# Method (mirrors the deep-bench discipline in apps/csvstat/bench):
#   - identical workload, identical fixture (test/data/big.log, 50k
#     records), identical algorithm in all four engines;
#   - N timed runs per engine per workload, median reported;
#   - sha256 over stdout captured per run — the cross-engine checksum
#     contract asserts the four outputs are byte-identical, so a perf
#     win can never be bought with a semantics change;
#   - checksums come from the FIRST timed run of each workload (each
#     subsequent run's checksum is also verified).
#
# The Rust baseline is compiled on demand with `rustc -O` into this
# directory (gitignored artifact target); set RUSTC_BIN to override.

import hashlib
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
APP = os.path.join(ROOT, "apps", "loglens")
BIG = os.path.join(APP, "test", "data", "big.log")
SMALL = os.path.join(APP, "test", "data", "small.log")
os.chdir(ROOT)

OPERON = os.environ.get("OPERON_BIN", os.path.join(ROOT, "target", "release", "operon"))
PYTHON = shutil.which("python3") or sys.executable
NODE = shutil.which("node")
RUSTC = os.environ.get("RUSTC_BIN", os.path.join(os.path.expanduser("~"), ".cargo", "bin", "rustc"))

RS_BIN = os.path.join(tempfile.gettempdir(), "loglens_rs_bench")


def build_rust():
    if os.path.exists(RS_BIN):
        return True
    if not os.path.exists(RUSTC):
        return False
    p = subprocess.run(
        [RUSTC, "-O", os.path.join(APP, "loglens.rs"), "-o", RS_BIN],
        capture_output=True, text=True,
    )
    if p.returncode != 0:
        sys.stderr.write(p.stderr[:2000])
        return False
    return True


WORKLOADS = [
    # (name, argv-tail, runs) — argv passed after `--` to operon, verbatim
    # to python/node/rust
    ("startup", None, 15),                      # usage() print, no file work
    ("stats_big", ["stats", BIG], 7),
    ("top_url_big", ["top", BIG, "url", "8"], 7),
    ("top_status_big", ["top", BIG, "status", "8"], 7),
    ("errors_big", ["errors", BIG, "10"], 7),
    ("table_big", ["table", BIG, "50"], 7),
    ("stats_small", ["stats", SMALL], 15),
]

RUNS_FILE = os.path.join(HERE, "results_v1.json")


def eng_cmd(engine, tail):
    if engine == "operon":
        cmd = [OPERON, "run", os.path.join(APP, "loglens.op"), "--cell",
               os.path.join(APP, "loglens.cell"), "--allow-read", ROOT,
               "--fuel", "20000000000"]
        if tail:
            cmd += ["--"] + tail
        return cmd
    if engine == "python":
        return [PYTHON, os.path.join(APP, "loglens.py")] + (tail or [])
    if engine == "node":
        return [NODE, os.path.join(APP, "loglens.js")] + (tail or [])
    if engine == "rust":
        return [RS_BIN] + (tail or [])
    raise ValueError(engine)


def one_run(cmd):
    t0 = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True)
    dt = (time.perf_counter() - t0) * 1000.0
    if p.returncode != 0:
        return dt, None, p.returncode
    return dt, hashlib.sha256(p.stdout).hexdigest(), 0


def bench_engine(engine, tail, runs):
    cmd = eng_cmd(engine, tail)
    times, cks = [], []
    for _ in range(runs):
        dt, ck, rc = one_run(cmd)
        if rc != 0 or ck is None:
            return {"status": "error", "rc": rc}
        times.append(dt)
        cks.append(ck)
    return {
        "status": "ok",
        "median_ms": round(statistics.median(times), 2),
        "min_ms": round(min(times), 2),
        "max_ms": round(max(times), 2),
        "runs": runs,
        "cs": cks[0],
        "cs_stable": len(set(cks)) == 1,
    }


def main():
    rust_ok = build_rust()
    engines = ["operon", "python"] + (["node"] if NODE else []) + \
              (["rust"] if rust_ok else [])
    meta = {
        "cpu": os.uname().machine,
        "kernel": os.uname().release,
        "os": os.uname().sysname,
        "date": time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime()),
        "fixture_big": BIG,
        "fixture_small": SMALL,
        "engines": engines,
        "versions": {
            "operon": subprocess.run([OPERON, "version"], capture_output=True,
                                     text=True).stdout.splitlines()[0],
            "python": subprocess.run([PYTHON, "--version"], capture_output=True,
                                     text=True).stdout.strip(),
            "node": subprocess.run([NODE, "--version"], capture_output=True,
                                   text=True).stdout.strip() if NODE else None,
            "rust": subprocess.run([RUSTC, "--version"], capture_output=True,
                                   text=True).stdout.strip() if rust_ok else None,
        },
    }
    out = {"meta": meta, "workloads": {}}
    bad = []
    for wname, tail, runs in WORKLOADS:
        row = {}
        for eng in engines:
            row[eng] = bench_engine(eng, tail, runs)
        cks = {eng: row[eng].get("cs") for eng in engines
               if row[eng].get("status") == "ok"}
        row["cs_identical"] = len(set(cks.values())) == 1 and len(cks) == len(engines)
        if not row["cs_identical"]:
            bad.append(wname)
        base = row["operon"].get("median_ms")
        for eng in engines:
            m = row[eng].get("median_ms")
            if m and base:
                row[f"ratio_{eng}_over_op"] = round(m / base, 2)
        out["workloads"][wname] = row
        pretty = " | ".join(
            f"{eng} {row[eng].get('median_ms')} ms" for eng in engines
        )
        print(f"{wname}: {pretty} | cs_identical={row['cs_identical']}")

    with open(RUNS_FILE, "w") as f:
        json.dump(out, f, indent=1, sort_keys=True)
    print(f"\nresults -> {RUNS_FILE}")
    if bad:
        print(f"CHECKSUM CONTRACT VIOLATED on: {bad}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
