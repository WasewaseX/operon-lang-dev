#!/usr/bin/env python3
"""bench_deep.py — deep cross-language benchmark for the four ytdl app builds.

Measures ONE real application (the ytdl YouTube-downloader orchestrator,
implemented identically in Operon / Python / Deno/TypeScript / Bash)
across many aspects:

  startup       cold invocation of the whole app (parse/compile + dispatch)
  json          yt-dlp metadata parse throughput (JSON parse xN)
  table         decision pipeline: format sort + row-build xR (the app's
                real per-video work: comparator sort + string formatting)
  lines         progress-line string processing xK (build + scan + extract)
  spawn         sequential child-process orchestration xN (mock engine)
  queue1 / qC   concurrent queue: K mock jobs at C=1 vs C=8 (wall time;
                the speedup ratio shows whether the concurrency model
                actually overlaps child work)
  rss           peak resident memory of the heaviest runs
  sizes         runtime binary + app source footprint
  loc           app source lines per implementation
  cs            differential: every workload prints a checksum that must
                be byte-identical in every language that implements it

Each (impl, workload) gets one warmup run (discarded), then R timed runs;
medians are reported. All engines are deterministic mocks on PATH — no
network anywhere. Usage:

  python3 scripts/bench_deep.py [--runs-scale 1.0] [--out apps/ytdl/bench/results.json]
"""
import argparse
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
MOCKS = os.path.join(REPO, "apps", "ytdl", "test", "mock")
FIXTURE = os.path.join(MOCKS, "fixtures", "meta_big.json")
RSS_WRAP = os.path.join(HERE, "rss_wrap.py")


def env_with_mocks():
    e = dict(os.environ)
    e["PATH"] = MOCKS + os.pathsep + e.get("PATH", "")
    return e


ENV = env_with_mocks()

# ---------------------------------------------------------- impl registry

OP = os.path.join(REPO, "target", "release", "operon")


def operon_base():
    return [
        OP, "run", os.path.join(REPO, "apps/ytdl/ytdl.op"),
        "--cell", os.path.join(REPO, "apps/ytdl/ytdl.cell"),
        "--allow-read", REPO,
        "--allow-run", "mockspawn", "--allow-run", "mocksleep",
        "--fuel", "20000000000",
        "--",
    ]


IMPLS = {
    "operon": {
        "kind": "flag",
        "base": operon_base,
        "runtime": OP,
        "source": os.path.join(REPO, "apps/ytdl/ytdl.op"),
        "label": "Operon VM",
    },
    "python": {
        "kind": "flag",
        "base": lambda: [sys.executable, os.path.join(REPO, "apps/ytdl-compare/python/ytdl.py")],
        "runtime": shutil.which("python3") or sys.executable,
        "source": os.path.join(REPO, "apps/ytdl-compare/python/ytdl.py"),
        "label": "Python 3.12",
    },
    "deno": {
        "kind": "flag",
        "base": lambda: ["deno", "run", "--allow-read", "--allow-run",
                          os.path.join(REPO, "apps/ytdl-compare/deno/ytdl.ts")],
        "runtime": shutil.which("deno"),
        "source": os.path.join(REPO, "apps/ytdl-compare/deno/ytdl.ts"),
        "label": "Deno 2.9 (TS)",
    },
    "bash": {
        "kind": "positional",
        "base": lambda: ["bash", os.path.join(REPO, "apps/ytdl-compare/bash/ytdl.sh")],
        "runtime": shutil.which("bash"),
        "source": os.path.join(REPO, "apps/ytdl-compare/bash/ytdl.sh"),
        "label": "Bash 5",
    },
}

# ---------------------------------------------------------- workloads

# (name, params, runs) — the numbered variants use 10x the work so the
# per-unit rate is startup-independent (python/deno finish the base
# table/lines workloads faster than their own startup).
WORKLOADS = [
    ("startup", {}, 15),
    ("json", {"n": 200}, 7),
    ("table", {"rounds": 30}, 7),
    ("lines", {"k": 20000}, 7),
    ("spawn", {"n": 30}, 7),
    ("queue1", {"k": 16, "c": 1}, 5),
    ("queue8", {"k": 16, "c": 8}, 5),
    ("json2k", {"n": 2000}, 5),
    ("table300", {"rounds": 300}, 5),
    ("lines200k", {"k": 200000}, 5),
    ("spawn200", {"n": 200}, 5),
]

RSS_WORKLOADS = {"startup", "json2k", "queue8"}


def workload_cmd(impl, name, params):
    """Command list for (impl, workload), or None if the impl refuses it."""
    spec = IMPLS[impl]
    base = spec["base"]()
    kind = spec["kind"]
    if name == "startup":
        sub, args = "bench-startup", []
    elif name.startswith("json"):
        if impl == "bash":
            return None
        sub, args = "bench-json", [FIXTURE, "--n", str(params["n"])]
    elif name.startswith("table"):
        if impl == "bash":
            return None
        sub, args = "bench-table", [FIXTURE, "--rounds", str(params["rounds"])]
    elif name.startswith("lines"):
        sub, args = ("bench-lines", ["--k", str(params["k"])]) if kind == "flag" \
            else ("bench-lines", [str(params["k"])])
    elif name.startswith("spawn"):
        sub, args = ("bench-spawn", ["--n", str(params["n"])]) if kind == "flag" \
            else ("bench-spawn", [str(params["n"])])
    elif name in ("queue1", "queue8"):
        k, c = params["k"], params["c"]
        sub, args = ("bench-queue", ["--k", str(k), "--c", str(c)]) if kind == "flag" \
            else ("bench-queue", [str(k)])
    else:
        raise ValueError(name)
    return base + [sub] + args


def run_once(cmd, rss=False):
    """Run one measurement; returns (wall_ms, exit_code, cs, rss_kb)."""
    if rss:
        wrap = [sys.executable, RSS_WRAP] + cmd
    else:
        wrap = cmd
    t0 = time.perf_counter()
    p = subprocess.run(wrap, capture_output=True, text=True, env=ENV)
    wall = (time.perf_counter() - t0) * 1000.0
    cs = ""
    for ln in p.stdout.splitlines():
        if ln.startswith("bench-") and " cs=" in ln:
            cs = ln.split(" cs=", 1)[1].strip()
    rss_kb = None
    if rss:
        for ln in p.stderr.splitlines():
            if ln.startswith("RSS_KB="):
                rss_kb = int(ln.split("=", 1)[1])
    return wall, p.returncode, cs, rss_kb


def measure(impl, name, params, runs, rss=False):
    cmd = workload_cmd(impl, name, params)
    if cmd is None:
        return {"status": "refused"}
    # warmup (discarded): fills OS caches + deno's transpile cache
    run_once(cmd)
    walls, css, codes, rsses = [], [], [], []
    for _ in range(runs):
        w, rc, cs, rkb = run_once(cmd, rss=rss)
        walls.append(w)
        codes.append(rc)
        if cs:
            css.append(cs)
        if rkb is not None:
            rsses.append(rkb)
    ok = all(rc == 0 for rc in codes)
    out = {
        "status": "ok" if ok else "error",
        "median_ms": round(statistics.median(walls), 2),
        "min_ms": round(min(walls), 2),
        "max_ms": round(max(walls), 2),
        "runs": runs,
    }
    if css:
        out["cs"] = css[0]
        out["cs_stable"] = len(set(css)) == 1
    if rsses:
        out["rss_kb"] = max(rsses)
    return out


# ---------------------------------------------------------- static metrics

def human(n):
    if n <= 0:
        return "0 B"
    for unit in ["B", "KB", "MB", "GB"]:
        if n < 1024 or unit == "GB":
            return f"{n:.1f} {unit}" if unit != "B" else f"{int(n)} B"
        n /= 1024.0
    return f"{n:.1f} GB"


def size_of(path):
    try:
        return os.stat(path).st_size
    except OSError:
        return 0


def loc_of(path):
    n = 0
    try:
        with open(path, "r", encoding="utf-8") as fh:
            for ln in fh:
                s = ln.strip()
                if s and not s.startswith("#") and not s.startswith("//"):
                    n += 1
    except OSError:
        return 0
    return n


def cpu_model():
    try:
        with open("/proc/cpuinfo", "r", encoding="utf-8") as fh:
            for ln in fh:
                if ln.startswith("model name"):
                    return ln.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def version_line(cmd):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=ENV)
        out = (p.stdout or p.stderr).splitlines()
        return out[0].strip() if out else "?"
    except Exception:
        return "?"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs-scale", type=float, default=1.0)
    ap.add_argument("--out", default=os.path.join(REPO, "apps/ytdl/bench/results.json"))
    args = ap.parse_args()

    missing = [k for k, v in IMPLS.items() if not v["runtime"]]
    if missing:
        print(f"skipping impls (runtime not found): {', '.join(missing)}", file=sys.stderr)

    results = {
        "meta": {
            "cpu": cpu_model(),
            "cores": os.cpu_count(),
            "mem_kb": 0,
            "kernel": platform.release(),
            "os": sys.platform,
            "versions": {
                "operon": version_line([OP, "--version"]),
                "python": version_line([sys.executable, "--version"]),
                "deno": version_line(["deno", "--version"]),
                "bash": version_line(["bash", "--version"]),
            },
            "fixture": os.path.relpath(FIXTURE, REPO),
            "date": time.strftime("%Y-%m-%d %H:%M %Z"),
        },
        "workloads": {},
        "rss": {},
        "static": {},
        "cs_differential": {},
    }
    try:
        with open("/proc/meminfo", "r", encoding="utf-8") as fh:
            for ln in fh:
                if ln.startswith("MemTotal"):
                    results["meta"]["mem_kb"] = int(ln.split()[1])
    except OSError:
        pass

    active = [k for k in IMPLS if IMPLS[k]["runtime"]]

    for name, params, runs in WORKLOADS:
        runs = max(3, int(runs * args.runs_scale))
        results["workloads"][name] = {}
        for impl in active:
            rss = name in RSS_WORKLOADS
            r = measure(impl, name, params, runs, rss=rss)
            results["workloads"][name][impl] = r
            if r["status"] == "ok" and "cs" in r:
                results["cs_differential"].setdefault(name, {})[impl] = r["cs"]
            done = r.get("median_ms", "?")
            rss_s = f" rss={r['rss_kb']}KB" if "rss_kb" in r else ""
            print(f"[{name:8s}] {impl:7s} median={done} ms{rss_s}", file=sys.stderr)

    # static: sizes + loc
    st = {}
    for impl in active:
        spec = IMPLS[impl]
        rt = size_of(spec["runtime"])
        try:
            rt_real = os.path.realpath(spec["runtime"])
            if os.path.isfile(rt_real):
                rt = os.stat(rt_real).st_size
        except OSError:
            pass
        src = size_of(spec["source"])
        st[impl] = {
            "runtime_bytes": rt,
            "source_bytes": src,
            "loc": loc_of(spec["source"]),
            "runtime_plus_source": rt + src,
        }
    results["static"] = st

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as fh:
        json.dump(results, fh, indent=1)
        fh.write("\n")

    # ---- markdown report on stdout
    m = results["meta"]
    print(f"# ytdl deep benchmark — {m['date']}")
    print()
    print(f"CPU: {m['cpu']} x{m['cores']} | kernel {m['kernel']} | fixture {m['fixture']}")
    print()
    print("| workload | " + " | ".join(IMPLS[i]["label"] for i in active) + " |")
    print("|---" * (len(active) + 1) + "|")
    for name, params, _r in WORKLOADS:
        cells = []
        for impl in active:
            w = results["workloads"][name].get(impl, {})
            if w.get("status") == "refused":
                cells.append("refused")
            elif w.get("status") == "ok":
                cells.append(f"{w['median_ms']:.1f} ms")
            else:
                cells.append("ERROR")
        print(f"| {name} | " + " | ".join(cells) + " |")
    print()
    print("| impl | runtime | app source | runtime+source | app LOC |")
    print("|---|---|---|---|---|")
    for impl in active:
        s = st[impl]
        print(f"| {IMPLS[impl]['label']} | {human(s['runtime_bytes'])} | {human(s['source_bytes'])} "
              f"| {human(s['runtime_plus_source'])} | {s['loc']} |")
    print()
    print_cs(results, active)

    # failures for the caller
    bad = []
    for name, per in results["workloads"].items():
        for impl, w in per.items():
            if w.get("status") == "error":
                bad.append(f"{name}/{impl}")
    if bad:
        print(f"ERRORS: {', '.join(bad)}", file=sys.stderr)
        sys.exit(1)


def print_cs(results, active):
    flag_impls = [i for i in active if IMPLS[i]["kind"] == "flag"]
    print("checksums (flag-impls must agree per workload; bash is the honest subset):")
    for name, m2 in results["cs_differential"].items():
        vals = {m2[i] for i in flag_impls if i in m2}
        flag = "IDENTICAL" if len(vals) == 1 else "DIVERGENT"
        extra = ""
        if "bash" in m2 and name.startswith("queue"):
            extra = " (bash sequential pinned: " + m2["bash"] + ")"
        print(f"  {name}: {flag}{extra}")


if __name__ == "__main__":
    main()
