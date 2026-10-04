#!/usr/bin/env python3
# bench_csvstat.py — app-level cross-language benchmark: Operon vs Python.
#
# Method (mirrors the deep-bench discipline in apps/ytdl/bench):
#   - identical workload, identical fixture (test/data/big.csv, 5000 rows);
#   - N timed runs per engine per workload, median reported;
#   - sha256 over stdout captured per run — the cross-engine checksum
#     contract asserts the outputs are byte-identical, so a perf win can
#     never be bought with a semantics change;
#   - checksums come from the FIRST timed run of each workload (each
#     subsequent run's checksum is also verified).
#
# Run from anywhere: the script resolves paths itself and cds to the repo
# root (std/ module resolution is CWD-relative — known quirk, see BENCH.md).

import hashlib
import json
import os
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
APP = os.path.join(ROOT, "apps", "csvstat")
BIG = os.path.join(APP, "test", "data", "big.csv")
SMALL = os.path.join(APP, "test", "data", "small.csv")
os.chdir(ROOT)

OPERON = os.environ.get("OPERON_BIN", os.path.join(ROOT, "target", "release", "operon"))
PYTHON = shutil.which("python3") or sys.executable

WORKLOADS = [
    # (name, argv-tail, runs) — argv passed after `--` to operon, verbatim to python
    ("startup", None, 15),                     # usage() print, no file work
    ("info_big", ["info", BIG], 7),
    ("stats_big", ["stats", BIG], 7),
    ("top_big", ["top", BIG, "product", "8"], 7),
    ("table_big", ["table", BIG, "50"], 7),
    ("stats_small", ["stats", SMALL], 15),
]

RUNS_FILE = os.path.join(HERE, "results_v1.json")


def operon_cmd(tail):
    cmd = [OPERON, "run", os.path.join(APP, "csvstat.op"), "--cell",
           os.path.join(APP, "csvstat.cell"), "--allow-read", ROOT,
           "--fuel", "20000000000"]
    if tail:
        cmd += ["--"] + tail
    return cmd


def python_cmd(tail):
    cmd = [PYTHON, os.path.join(APP, "csvstat.py")]
    if tail:
        cmd += tail
    return cmd


def one_run(cmd):
    t0 = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True)
    dt = (time.perf_counter() - t0) * 1000.0
    if p.returncode != 0:
        return dt, None, p.returncode
    return dt, hashlib.sha256(p.stdout).hexdigest(), 0


def bench_engine(name, tail, runs):
    cmd = operon_cmd(tail) if name == "operon" else python_cmd(tail)
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
    meta = {
        "cpu": os.uname().machine,
        "kernel": os.uname().release,
        "os": os.uname().sysname,
        "date": time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime()),
        "fixture_big": BIG,
        "fixture_small": SMALL,
        "versions": {
            "operon": subprocess.run([OPERON, "version"], capture_output=True,
                                     text=True).stdout.splitlines()[0],
            "python": subprocess.run([PYTHON, "--version"], capture_output=True,
                                     text=True).stdout.strip(),
        },
    }
    out = {"meta": meta, "workloads": {}}
    for wname, tail, runs in WORKLOADS:
        row = {}
        for eng in ("operon", "python"):
            row[eng] = bench_engine(eng, tail, runs)
        ok_o = row["operon"].get("status") == "ok"
        ok_p = row["python"].get("status") == "ok"
        if ok_o and ok_p:
            row["cs_identical"] = row["operon"]["cs"] == row["python"]["cs"]
            row["ratio_py_over_op"] = round(row["python"]["median_ms"] /
                                            row["operon"]["median_ms"], 2) \
                if row["operon"]["median_ms"] > 0 else None
        out["workloads"][wname] = row
        print(f"{wname}: operon {row['operon'].get('median_ms')} ms | "
              f"python {row['python'].get('median_ms')} ms | "
              f"cs_identical={row.get('cs_identical')}")

    with open(RUNS_FILE, "w") as f:
        json.dump(out, f, indent=1, sort_keys=True)
    print(f"\nresults -> {RUNS_FILE}")
    bad = [w for w, r in out["workloads"].items() if not r.get("cs_identical")]
    if bad:
        print(f"CHECKSUM CONTRACT VIOLATED on: {bad}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
