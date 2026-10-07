#!/usr/bin/env python3
"""superbench.py — 57-aspect super-detailed cross-language benchmark.

Lanes: operon (bin/operon, VM lane), native-py (CPython), native-js (Node.js),
native-rs (rustc -O). Every lane implements the SAME 57 algorithms and prints
"OK <checksum> <elapsed_ms>" with elapsed measured in-process around the
workload (clocks: operon now(), perf_counter, performance.now, Instant).

Phases:
  agreement — one run per lane per aspect; all four checksums must be equal
              (a benchmark that measures the wrong answer is rejected).
  bench     — min-of-N in-process elapsed per lane per aspect (N=3 default;
              the agreement run doubles as the warmup).

Usage:
  python3 superbench.py --phase both --json out.json
  python3 superbench.py --only num_int_add,ctl_fib
"""
import argparse
import json
import math
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
OPERON = os.path.join(ROOT, "bin", "operon")
SUPER_OP = os.path.join(HERE, "super.op")
SUPER_PY = os.path.join(HERE, "super_py.py")
SUPER_JS = os.path.join(HERE, "super_js.js")
SUPER_RS = os.path.join(HERE, "super_rs")

ASPECTS = [
    # NUM
    ("num_int_add", "num", "int add loop (2M iters)"),
    ("num_int_mixed", "num", "int mul/mod/div mix (600k)"),
    ("num_float_add", "num", "float add chain (2M)"),
    ("num_float_math", "num", "sqrt loop + branches (300k)"),
    ("num_trialdiv", "num", "trial-division primes < 40000"),
    ("num_roundtrip", "num", "int->str->int roundtrip (200k)"),
    ("num_parse_float", "num", "float parse + trunc (200k)"),
    ("num_divmod", "num", "modulo/branch counting (1M)"),
    # CTRL
    ("ctl_while", "ctrl", "while countdown (1.2M)"),
    ("ctl_for", "ctrl", "for-range empty body (1.2M)"),
    ("ctl_nested", "ctrl", "nested loops 600x600"),
    ("ctl_call", "ctrl", "small-fn call overhead (600k)"),
    ("ctl_fib", "ctrl", "recursive fib(22) x5"),
    ("ctl_deep_rec", "ctrl", "deep recursion depth 5000 x6"),
    ("ctl_mutual", "ctrl", "mutual recursion even/odd (100k calls)"),
    ("ctl_branch", "ctrl", "16-arm branch ladder (1M)"),
    ("ctl_match", "ctrl", "8-arm match dispatch (500k)"),
    ("ctl_closure", "ctrl", "closure create+call (200k)"),
    # STR
    ("str_cat", "str", "s = s + 'ab' loop (400k)"),
    ("str_join", "str", "build 200k pieces + join"),
    ("str_slice", "str", "substring slicing (200k)"),
    ("str_replace", "str", "double replace (100k)"),
    ("str_split", "str", "split 100-word line (20k)"),
    ("str_case", "str", "upper/lower/trim (60k)"),
    ("str_compare", "str", "==/starts/ends (300k)"),
    ("str_interp", "str", "3-expr interpolation (100k)"),
    ("str_contains", "str", "substring search (200k)"),
    ("str_build", "str", "unicode piece build + upper (60k)"),
    # LIST
    ("lst_push", "list", "append 300k + iterate"),
    ("lst_idx", "list", "index reads (600k)"),
    ("lst_iter", "list", "iterate 100k x6"),
    ("lst_slice", "list", "list slicing (60k)"),
    ("lst_sort", "list", "native sort 60k"),
    ("lst_sort_lang", "list", "in-language quicksort 3k"),
    ("lst_comp", "list", "filter+map comprehensions (200k)"),
    ("lst_search", "list", "linear search scans (20k x 1000)"),
    ("lst_reverse", "list", "reverse 1000-list (30k)"),
    ("lst_insert_del", "list", "mid-list insert/remove (10k)"),
    # MAP
    ("map_set", "map", "map stores str keys (300k)"),
    ("map_get", "map", "map reads str keys (600k)"),
    ("map_miss", "map", "map miss probes (300k)"),
    ("map_iter", "map", "keys()+reads over 20k x10"),
    ("map_incr", "map", "m[k] += 1 word-count (200k)"),
    ("map_nested", "map", "nested map chain reads (200k)"),
    ("map_del", "map", "del + re-insert cycles (50k)"),
    ("map_mixed", "map", "mixed int/str key traffic (150k)"),
    # SET
    ("set_algebra", "set", "dedup 16k + membership (lang set idiom)"),
    # ALGO
    ("alg_sieve", "algo", "sieve of Eratosthenes < 50000"),
    ("alg_mandel", "algo", "mandelbrot 240x160 iter 50"),
    ("alg_trees", "algo", "binary trees depth 14 x3"),
    ("alg_matrix", "algo", "96x96 int matrix multiply"),
    ("alg_wordfreq", "algo", "split + word count (2k lines)"),
    ("alg_json_rt", "algo", "json roundtrip 120-row doc x150"),
    ("alg_json_big", "algo", "json 800-row doc, 2 parses"),
    ("alg_deep_eq", "algo", "deep equality 50-elem x20k"),
    ("alg_opt", "algo", "ok/err pipeline (200k)"),
    # FLOOR
    ("floor", "floor", "dispatch floor (empty workload)"),
]

OK_RE = re.compile(r"^OK (-?\d+) ([0-9.eE+-]+)$")


def run_lane(lane, aspect_id, timeout=240):
    if lane == "operon":
        cmd = [OPERON, "run", SUPER_OP, "--", aspect_id]
    elif lane == "py":
        cmd = [sys.executable, SUPER_PY, aspect_id]
    elif lane == "js":
        cmd = ["node", SUPER_JS, aspect_id]
    elif lane == "rs":
        cmd = [SUPER_RS, aspect_id]
    else:
        raise SystemExit(f"unknown lane {lane}")
    t0 = time.perf_counter()
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=ROOT)
    wall = (time.perf_counter() - t0) * 1000.0
    if proc.returncode != 0:
        raise RuntimeError(f"{lane}/{aspect_id} rc={proc.returncode}: {proc.stderr[-400:]}")
    m = None
    for line in proc.stdout.strip().splitlines():
        m = OK_RE.match(line.strip())
        if m:
            break
    if not m:
        raise RuntimeError(f"{lane}/{aspect_id} unparseable stdout: {proc.stdout[-200:]!r}")
    return {"chk": int(m.group(1)), "ms": float(m.group(2)), "wall": wall}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--phase", default="both", choices=["agreement", "bench", "both"])
    ap.add_argument("--reps", type=int, default=3)
    ap.add_argument("--langs", default="operon,py,js,rs")
    ap.add_argument("--only", default="")
    ap.add_argument("--json", default="")
    ap.add_argument("--label", default="")
    args = ap.parse_args()

    langs = [x.strip() for x in args.langs.split(",") if x.strip()]
    keep = set(x for x in args.only.split(",") if x)
    aspects = [(i, c, t) for (i, c, t) in ASPECTS if not keep or i in keep]

    results = {}
    failures = []

    if args.phase in ("agreement", "both"):
        print(f"== agreement phase: {len(aspects)} aspects x {len(langs)} lanes ==")
        for aid, _, _ in aspects:
            chks = {}
            for lane in langs:
                try:
                    r = run_lane(lane, aid)
                    chks[lane] = r["chk"]
                except Exception as e:  # noqa: BLE001
                    failures.append(f"{lane}/{aid}: {e}")
                    chks[lane] = None
            vals = set(v for v in chks.values() if v is not None)
            ok = len(vals) == 1 and None not in chks.values()
            results.setdefault(aid, {})["agreement"] = "PASS" if ok else "FAIL"
            results[aid]["checksums"] = chks
            status = "PASS" if ok else f"FAIL {chks}"
            print(f"  {aid:18s} {status}")
            if not ok:
                failures.append(f"agreement {aid}: {chks}")

    if args.phase in ("bench", "both"):
        print(f"== bench phase: min-of-{args.reps} in-process ms ==")
        for aid, _, _ in aspects:
            for lane in langs:
                runs = []
                for _ in range(args.reps):
                    try:
                        runs.append(run_lane(lane, aid)["ms"])
                    except Exception as e:  # noqa: BLE001
                        failures.append(f"{lane}/{aid}: {e}")
                        runs = None
                        break
                if runs:
                    results.setdefault(aid, {}).setdefault("min_ms", {})[lane] = min(runs)
                    results[aid].setdefault("runs", {})[lane] = runs
            got = results.get(aid, {}).get("min_ms", {})
            pretty = "  ".join(f"{lane}={got[lane]:9.2f}" for lane in langs if lane in got)
            print(f"  {aid:18s} {pretty}")

    # ratios + scores
    ratios = {}
    for aid, _, _ in aspects:
        got = results.get(aid, {}).get("min_ms", {})
        if "operon" in got and "py" in got and got["py"] > 0:
            r = got["operon"] / got["py"]
            ratios.setdefault(aid, {})["op_py"] = r
        if "operon" in got and "js" in got and got["js"] > 0:
            ratios.setdefault(aid, {})["op_js"] = got["operon"] / got["js"]
        if "operon" in got and "rs" in got and got["rs"] > 0:
            ratios.setdefault(aid, {})["op_rs"] = got["operon"] / got["rs"]

    def geomean(key):
        vals = [v[key] for v in ratios.values() if key in v and "floor" not in str(v)]
        vals = [v[key] for k, v in ratios.items() if key in v and k != "floor"]
        return math.exp(sum(math.log(x) for x in vals) / len(vals)) if vals else float("nan")

    scores = {k: geomean(k) for k in ("op_py", "op_js", "op_rs")}
    wins = sum(1 for v in ratios.values() if v.get("op_py", 99) < 1.0)

    out = {
        "meta": {
            "label": args.label,
            "timestamp": time.strftime("%Y-%m-%d %H:%M:%S"),
            "langs": langs,
            "reps": args.reps,
            "aspects": len(aspects),
            "host": "x86-64 Linux sandbox",
        },
        "results": results,
        "ratios": ratios,
        "scores": scores,
        "wins_vs_py": wins,
        "failures": failures,
    }
    if args.json:
        with open(args.json, "w") as f:
            json.dump(out, f, indent=1)

    print("\n== SCORE (geomean of operon/native ratio, excl. floor; lower is better) ==")
    print(f"  operon/python : {scores['op_py']:.2f}x")
    print(f"  operon/node   : {scores['op_js']:.2f}x")
    print(f"  operon/rust   : {scores['op_rs']:.2f}x")
    print(f"  aspects where operon beats CPython: {wins}/{len(aspects) - 1}")
    if failures:
        print(f"\nFAILURES ({len(failures)}):")
        for f in failures[:20]:
            print(f"  {f}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
