#!/usr/bin/env python3
"""bench_compare.py — Operon benchmark suite (B1, builder-B).

Three runners on identical algorithms:
  operon    — the Rust core (bin/operon run <fixture>)
  oracle    — bootstrap/oracle.py, the CPython tree-walking semantic mirror
  native-py — the same algorithm hand-written in pure CPython (the v3.0
              "CPython-level speed" target bar)

Modes:
  default     — the six named workloads in scripts/bench/*.op
  --micro     — per-construct micro fixtures in scripts/bench/micro/*.op
  --json PATH — additionally write machine-readable results
  --iters N   — timing iterations (default 5; oracle uses max(3, N//2))
  --quick     — fewer iterations, smaller fixture list

Honesty notes:
  * timings are end-to-end process times (interpreter startup included);
    m_empty / an empty python call calibrate that floor, and BENCH.md
    reports it next to the numbers.
  * native-py mirrors replicate the OBSERVED net semantics of each fixture
    (e.g. strings.op's template-replace idiom), not a re-interpretation.
"""
import argparse
import json
import os
import platform
import statistics
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BENCH = os.path.join(ROOT, "scripts", "bench")
MICRO = os.path.join(BENCH, "micro")
OPERON = os.path.join(ROOT, "bin", "operon")
ORACLE = os.path.join(ROOT, "bootstrap", "oracle.py")

# --------------------------------------------------------------------------
# native CPython mirrors — identical algorithms, one Python statement at a
# time. Keep these in lockstep with scripts/bench/*.op and micro/*.op.
# --------------------------------------------------------------------------

def native_fib():
    def fib(n):
        return n if n < 2 else fib(n - 1) + fib(n - 2)
    return fib(25)

def native_loops():
    acc = 0
    for i in range(200000):
        acc = acc + i % 7
    return acc

def native_strings():
    s = ""
    for i in range(4000):
        s = s + "x{t}y".replace("{t}", str(i % 10))
    return len(s)

def native_collections():
    m = {}
    xs = []
    for i in range(20000):
        k = "k" + str(i % 2000)
        if k in m:
            m[k] = m[k] + 1
        else:
            m[k] = 1
        xs.append(i % 97)
    total = 0
    for v in xs:
        total = total + v
    distinct = 0
    for _ in m:
        distinct += 1
    return (total, distinct)

def native_recursion():
    def pasc(n, k):
        return 1 if (k == 0 or k == n) else pasc(n - 1, k - 1) + pasc(n - 1, k)
    return pasc(20, 10)

def native_grn():
    # mirror of grn.op: per-call gate check (level >= threshold, one gene
    # enhanced by 0.25), then arithmetic body. Happy path — all calls pass.
    level = {"driver": 0.0}

    def worker_a(n):
        if level.get("driver", 0.0) < 0.3 - 0.25:  # enhanced threshold
            return None
        return n + 1

    def worker_b(n):
        if level.get("driver", 0.0) < 0.5:
            return None
        return n * 2

    def reporter(n):
        if level.get("driver", 0.0) < 0.7:
            return None
        return n - 1

    level["driver"] = 1.0  # grn_fire("driver")
    acc = 0
    for i in range(20000):
        acc = acc + worker_a(i) + worker_b(i) + reporter(i)
    return acc

SUITES = [
    ("fib25",        "scripts/bench/fib25.op",        native_fib,        242785),
    ("loops",        "scripts/bench/loops.op",        native_loops,      200000),
    ("strings",      "scripts/bench/strings.op",      native_strings,      4000),
    ("collections",  "scripts/bench/collections.op",  native_collections, None),
    ("recursion",    "scripts/bench/recursion.op",    native_recursion,  369511),
    ("grn",          "scripts/bench/grn.op",          native_grn,         60000),
]

# micro fixtures: (name, ops_per_iteration, native mirror)
def native_m_empty(): return "ok"
def native_m_call():
    def nop(n): return n
    a = 0
    for i in range(100000): a = a + nop(i)
    return a
def native_m_forrange():
    a = 0
    for i in range(200000): a = a + 1
    return a
def native_m_while():
    i, a = 200000, 0
    while i > 0:
        a = a + 1
        i = i - 1
    return a
def native_m_varread():
    x, a = 7, 0
    for i in range(200000): a = a + x
    return a
def native_m_intadd():
    a, b = 0, 1
    for i in range(300000): a = a + b
    return a
def native_m_listpush():
    xs = []
    for i in range(50000): xs.append(i)
    return len(xs)
def native_m_listidx():
    xs = [i for i in range(2000)]
    a = 0
    for i in range(100000): a = a + xs[i % 2000]
    return a
def native_m_mapset():
    m = {}
    for i in range(40000): m[str(i % 4000)] = i
    return len(m)
def native_m_mapget():
    m = {}
    for i in range(4000): m[str(i)] = i
    a = 0
    for i in range(50000): a = a + m[str(i % 4000)]
    return a
def native_m_strcat():
    s = ""
    for i in range(12000): s = s + "ab"
    return len(s)

MICROS = [
    ("m_empty",    0,      native_m_empty),
    ("m_call",     300000, native_m_call),      # 100k calls, 3 evals each (loop+add+call)
    ("m_forrange", 600000, native_m_forrange),  # 200k iters x (loop bookkeeping + add)
    ("m_while",    600000, native_m_while),
    ("m_varread",  600000, native_m_varread),
    ("m_intadd",   900000, native_m_intadd),
    ("m_listpush", 150000, native_m_listpush),
    ("m_listidx",  300000, native_m_listidx),
    ("m_mapset",   80000,  native_m_mapset),
    ("m_mapget",   154000, native_m_mapget),
    ("m_strcat",   24000,  native_m_strcat),
]

# --------------------------------------------------------------------------
# timing helpers
# --------------------------------------------------------------------------

def time_fn(fn, iters, warmup=1):
    """min / median wall time of fn() over iters runs after warmup."""
    for _ in range(warmup):
        fn()
    ts = []
    for _ in range(iters):
        t0 = time.perf_counter()
        fn()
        ts.append(time.perf_counter() - t0)
    return min(ts), statistics.median(ts)

def time_cmd(cmd, cwd, iters, warmup=1):
    for _ in range(warmup):
        subprocess.run(cmd, capture_output=True, cwd=cwd)
    ts = []
    for _ in range(iters):
        t0 = time.perf_counter()
        subprocess.run(cmd, capture_output=True, cwd=cwd)
        ts.append(time.perf_counter() - t0)
    return min(ts), statistics.median(ts)

def run_operon(path, iters):
    return time_cmd([OPERON, "run", path], ROOT, iters)

def run_oracle(path, iters):
    return time_cmd([sys.executable, ORACLE, "run", path], ROOT, iters)

def cpu_model():
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"

def meta(iters):
    return {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "host": platform.node(),
        "os": platform.platform(),
        "cpu": cpu_model(),
        "python": sys.version.split()[0],
        "iters": iters,
        "timing": "end-to-end process wall time, min over iters (1 warmup)",
    }

# --------------------------------------------------------------------------
# report
# --------------------------------------------------------------------------

def fmt_row(cells):
    return "  ".join(f"{c:>{w}}" for c, w in cells)

def report_suites(results, iters):
    hdr = [("workload", 12), ("operon", 10), ("oracle", 11), ("native-py", 11),
           ("op/py", 8), ("op/oracle", 10), ("calls", 10)]
    print(fmt_row(hdr))
    print("  ".join("-" * w for _, w in hdr))
    for name, _path, _nat, calls in SUITES:
        r = results[name]
        op, orc, nat = r["operon"]["min"] * 1000, r["oracle"]["min"] * 1000, r["native"]["min"] * 1000
        cells = [(name, 12), (f"{op:.1f}", 10), (f"{orc:.1f}", 11), (f"{nat:.1f}", 11),
                 (f"{op/nat:.1f}x", 8), (f"{orc/op:.1f}x", 10),
                 (str(calls) if calls else "-", 10)]
        print(fmt_row(cells))
    print("(times in ms, end-to-end incl. startup; op/py = operon vs native CPython — the v3.0 gap)")

def report_micros(results, iters):
    hdr = [("micro", 12), ("operon", 10), ("oracle", 11), ("native-py", 11),
           ("op/py", 8), ("op ns/op", 11), ("py ns/op", 11)]
    print(fmt_row(hdr))
    print("  ".join("-" * w for _, w in hdr))
    for name, ops, _nat in MICROS:
        r = results[name]
        op, orc, nat = r["operon"]["min"] * 1000, r["oracle"]["min"] * 1000, r["native"]["min"] * 1000
        ns_op = f"{op * 1e6 / ops:.0f}" if ops else "-"
        py_ns = f"{nat * 1e6 / ops:.0f}" if ops else "-"
        cells = [(name, 12), (f"{op:.1f}", 10), (f"{orc:.1f}", 11), (f"{nat:.1f}", 11),
                 (f"{op/nat:.1f}x", 8), (ns_op, 11), (py_ns, 11)]
        print(fmt_row(cells))
    print("(times in ms; ns/op = per documented ops-per-iteration count)")

def main():
    ap = argparse.ArgumentParser(description="Operon benchmark suite")
    ap.add_argument("--micro", action="store_true", help="run per-construct micro fixtures")
    ap.add_argument("--json", metavar="PATH", help="also write results as JSON")
    ap.add_argument("--iters", type=int, default=5, help="timing iterations (default 5)")
    ap.add_argument("--quick", action="store_true", help="reduced iterations")
    args = ap.parse_args()
    iters = 3 if args.quick else args.iters
    oiters = max(2, iters // 2) if args.quick else max(3, iters // 2)

    results = {"meta": meta(iters)}
    suites, micros = {}, {}

    if not args.micro:
        for name, path, native, _calls in SUITES:
            full = os.path.join(ROOT, path)
            if not os.path.exists(full):
                print(f"skip {name}: {path} missing", file=sys.stderr)
                continue
            omin, omed = run_operon(path, iters)
            rmin, rmed = run_oracle(path, oiters)
            nmin, nmed = time_fn(native, iters)
            suites[name] = {
                "operon": {"min": omin, "median": omed, "iters": iters},
                "oracle": {"min": rmin, "median": rmed, "iters": oiters},
                "native": {"min": nmin, "median": nmed, "iters": iters},
            }
        results["suites"] = suites
        report_suites(suites, iters)

    if args.micro:
        for name, _ops, native in MICROS:
            path = os.path.join("scripts/bench/micro", f"{name}.op")
            full = os.path.join(ROOT, path)
            if not os.path.exists(full):
                print(f"skip {name}: {path} missing", file=sys.stderr)
                continue
            it = 3 if args.quick else iters
            omin, omed = run_operon(path, it)
            rmin, rmed = run_oracle(path, max(2, it // 2))
            nmin, nmed = time_fn(native, it)
            micros[name] = {
                "operon": {"min": omin, "median": omed, "iters": it},
                "oracle": {"min": rmin, "median": rmed, "iters": max(2, it // 2)},
                "native": {"min": nmin, "median": nmed, "iters": it},
            }
        results["micros"] = micros
        report_micros(micros, iters)

    if args.json:
        with open(args.json, "w") as f:
            json.dump(results, f, indent=2)
        print(f"\nJSON written to {args.json}")

if __name__ == "__main__":
    main()
