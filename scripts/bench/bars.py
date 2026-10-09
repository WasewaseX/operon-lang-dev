#!/usr/bin/env python3
"""bars.py — R0.8 Bar A/B/C performance harness (L-039, sz lane).

The STANDARD bar structure digest-17 specified, per workload:

  HEAD  — bin/operon built from the current tree (rule 1: fresh binary)
  Bar A — the previous RELEASE binary, downloaded from the GitHub release
          and sha256-verified against the release's own SHA256SUMS (the
          F-REL2100-POSTTAG pattern). Today that is v2.10.0.
  Bar B — CPython, same machine: the bench_compare.py native-py mirrors,
          IMPORTED — never reimplemented (the import-the-law discipline).
  Bar C — native Rust floor: single-file twins compiled with `rustc -O`.
          regex/json/modules are documented REFUSALS (no std-only Rust
          analog — the deep-bench refusal-is-a-finding honesty).

Scaling: every workload also runs a 2x twin fixture (*_2x.op) and reports
t(2N)/t(N) for HEAD / Bar A / Bar C (Bar B's mirrors are size-fixed by
design — noted, not a gap). fib25 + recursion are n/a: exponential by
construction.

Differential-verified benchmarking: every runner prints the value the .op
promotes; a timing whose value disagrees across runners is NOT recorded
(the house rule: a benchmark that measures the wrong answer must fail
before it can mislead anyone).

Methodology (identical to bench_compare.py — imported, not copied):
end-to-end process wall time, min over N runs, 1 warmup.

Usage:
  python3 scripts/bench/bars.py [--quick] [--iters N] [--json PATH]
         [--bara PATH] [--skip-c]
Reproducibility half: scripts/bench/bars_check.py re-derives
docs/perf/BARS.md from the saved JSON.
"""
import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request

# the bench.sh convention: the cargo toolchain lives in ~/.cargo/bin (or the
# relocated $PROJECT/.cargo + .rustup after a sandbox HOME wipe) — put it on
# PATH and point rustup at it so rustc is reachable for the Bar C twins.
for _cargo in (os.path.expanduser("~/.cargo/bin"),
               "/home/z/my-project/.cargo/bin"):
    if os.path.isdir(_cargo) and _cargo not in os.environ["PATH"]:
        os.environ["PATH"] = _cargo + os.pathsep + os.environ["PATH"]
for _home in (os.path.expanduser("~/.rustup"), "/home/z/my-project/.rustup"):
    # only a rustup home that actually holds a toolchain counts — a stale
    # empty ~/.rustup (created by failed proxy runs) must not shadow the
    # relocated one
    if os.path.isdir(os.path.join(_home, "toolchains")):
        os.environ.setdefault("RUSTUP_HOME", _home)
        break
for _home in (os.path.expanduser("~/.cargo"), "/home/z/my-project/.cargo"):
    if os.path.isdir(_home):
        os.environ.setdefault("CARGO_HOME", _home)
        break

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# import-the-law: the workload table, the native-py mirrors and the timing
# law come from scripts/bench_compare.py. Nothing here re-implements them.
from bench_compare import SUITES, time_cmd, time_fn, run_operon  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BENCH = os.path.join(ROOT, "scripts", "bench")
HEAD_BIN = os.path.join(ROOT, "bin", "operon")
REPO = "WasewaseX/operon-lang-dev"
CACHE = os.environ.get("OPERON_BARS_CACHE", "/tmp/operon-bars-cache")

# Bar C twins: workload -> (rust source, kind). kind "callfib" = the
# existing call_fib_rs.rs twin (argv n reps, prints CALLFIB ... r=<value>);
# kind "std" = the R0.8 twins (argv n, prints the .op promote line).
# None = documented REFUSAL (no std-only Rust analog).
C_TWINS = {
    "fib25":      ("call_fib_rs.rs", "callfib"),
    "loops":      ("loops_rs.rs", "std"),
    "strings":    ("strings_rs.rs", "std"),
    "collections":("collections_rs.rs", "std"),
    "recursion":  ("recursion_rs.rs", "std"),
    "grn":        ("grn_rs.rs", "std"),
    "seq":        ("seq_rs.rs", "std"),
    "large_map":  ("large_map_rs.rs", "std"),
    "file_io":    ("file_io_rs.rs", "std"),
    "json":       None,
    "regex":      None,
    "modules":    None,
}

# base size per workload (the .op constant; passed EXPLICITLY to the twins
# so a silent twin-default drift can never go unnoticed).
BASE_N = {
    "fib25": 25, "loops": 200000, "strings": 4000, "collections": 20000,
    "recursion": 20, "grn": 20000, "seq": 6000, "large_map": 6000,
    "file_io": 300,
}

def c_cmd(name, src_path, n2=None, reps=1):
    """The argv law for a Bar C twin at size n (n2 = explicit override)."""
    if name == "fib25":
        return [src_path, str(n2 or BASE_N[name]), str(reps)]
    return [src_path, str(n2 or BASE_N[name])]

# py-mirror value -> the string the .op promotes (the parity contract).
def py_line(name, v):
    if name == "loops":      return f"acc = {v}"
    if name == "strings":    return f"len = {v}"
    if name == "collections":
        total, distinct = v
        # k1 is a constant of the shape: n_base/2000 hits of key "k1"
        return f"total = {total}, distinct = {distinct}, k1 = {20000 // 2000}"
    if name == "recursion":  return f"C(20,10) = {v}"
    if name == "grn":        return f"acc = {v}"
    if name == "seq":        return None  # sum only; triple checked via decomposition
    if name == "large_map":  return f"map sum = {v}"
    if name == "file_io":    return f"io ok = {v}"
    if name == "json":       return f"json acc = {v}"
    if name == "regex":      return f"regex hits = {v}"
    if name == "fib25":      return f"fib(25) = {v}"
    if name == "modules":    return None  # shape-compare (BENCH.md honesty note)
    raise KeyError(name)


def sh(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def git(*args):
    r = sh(["git", "-C", ROOT, *args])
    return r.stdout.strip() if r.returncode == 0 else None


def bin_version(path):
    r = sh([path, "version"])
    return r.stdout.strip().splitlines()[0] if r.returncode == 0 else "?"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# --------------------------------------------------------------------------
# Bar A: the previous release binary
# --------------------------------------------------------------------------

def resolve_bara(explicit=None):
    """Return (binary_path, meta). explicit PATH wins; else the cache; else
    download the latest release tarball for this platform + verify sha256
    against the release's own SHA256SUMS."""
    if explicit:
        return explicit, {"source": "explicit", "binary_sha256": sha256(explicit),
                          "version": bin_version(explicit)}
    machine = "x86_64-unknown-linux-gnu"
    asset = tag = None
    try:
        headers = {"User-Agent": "operon-bars"}
        tok = os.environ.get("GITHUB_TOKEN")
        if tok:
            headers["Authorization"] = f"token {tok}"
        req = urllib.request.Request(
            f"https://api.github.com/repos/{REPO}/releases/latest", headers=headers)
        with urllib.request.urlopen(req, timeout=30) as r:
            rel = json.load(r)
        tag = rel["tag_name"]
        for a in rel["assets"]:
            if a["name"].endswith(f"{machine}.tar.gz"):
                asset = a["name"]
                break
    except Exception as e:  # offline: fall through to any cached binary
        print(f"[bars] release lookup failed ({e}); trying cache", file=sys.stderr)
    if asset and tag:
        tgz = os.path.join(CACHE, asset)
        sums = os.path.join(CACHE, "SHA256SUMS")
        if not (os.path.exists(tgz) and os.path.exists(sums)):
            os.makedirs(CACHE, exist_ok=True)
            base = f"https://github.com/{REPO}/releases/download/{tag}"
            urllib.request.urlretrieve(f"{base}/{asset}", tgz)
            urllib.request.urlretrieve(f"{base}/SHA256SUMS", sums)
        expected = None
        for line in open(sums):
            parts = line.split()
            if len(parts) == 2 and parts[1] == asset:
                expected = parts[0]
        if expected is None:
            raise SystemExit(f"[bars] {asset} not covered by the release SHA256SUMS")
        got = sha256(tgz)
        if got != expected:
            raise SystemExit(f"[bars] sha256 MISMATCH for {asset}: {got} != {expected}")
        d = os.path.join(CACHE, asset.replace(".tar.gz", ""))
        if not os.path.exists(os.path.join(d, "operon")):
            with tarfile.open(tgz) as t:
                t.extractall(CACHE, filter="tar")
        b = os.path.join(d, "operon")
        if os.path.exists(b):
            return b, {"source": f"release {tag}", "tag": tag, "asset": asset,
                       "tarball_sha256": got, "binary_sha256": sha256(b),
                       "version": bin_version(b)}
    raise SystemExit("[bars] no Bar A binary available (offline and cache empty); "
                     "pass --bara PATH")


# --------------------------------------------------------------------------
# Bar C: compile + run the rust twins
# --------------------------------------------------------------------------

def twin_bin(src_name):
    """Compile (cached by mtime) and return the twin binary path."""
    if shutil.which("rustc") is None:
        raise SystemExit("[bars] rustc not found on PATH — the Bar C lane needs "
                         "the Rust toolchain (bash scripts/build.sh provisions "
                         "the expectation; install rustup 1.99.0 to run Bar C)")
    src = os.path.join(BENCH, src_name)
    out = os.path.join(tempfile.gettempdir(), "operon-bars-c",
                       src_name.replace(".rs", ""))
    os.makedirs(os.path.dirname(out), exist_ok=True)
    if not os.path.exists(out) or os.path.getmtime(src) > os.path.getmtime(out):
        r = sh(["rustc", "-O", src, "-o", out])
        if r.returncode != 0:
            raise SystemExit(f"[bars] rustc failed for {src_name}:\n{r.stderr}")
    return out


def run_c_once(name, src_name, n2=None):
    """Run a Bar C twin once; returns its stdout value line."""
    path = twin_bin(src_name)
    cmd = c_cmd(name, path, n2)
    r = sh(cmd)
    if r.returncode != 0:
        raise SystemExit(f"[bars] {name} twin failed: {r.stderr[:200]}")
    return r.stdout.strip(), cmd


def c_time(name, src_name, iters):
    path = twin_bin(src_name)
    cmd = c_cmd(name, path)
    mn, med = time_cmd(cmd, ROOT, iters)
    return mn, med


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------

def cpu_model():
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def main():
    ap = argparse.ArgumentParser(description="R0.8 Bar A/B/C harness")
    ap.add_argument("--iters", type=int, default=5)
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--json", default=os.path.join(ROOT, "docs", "perf", "bars.json"))
    ap.add_argument("--bara", help="explicit Bar A binary (skips the download)")
    ap.add_argument("--skip-c", action="store_true", help="skip the Bar C lane")
    args = ap.parse_args()
    iters = 3 if args.quick else args.iters

    if not os.path.exists(HEAD_BIN):
        print("[bars] bin/operon missing — building (rule 1)", file=sys.stderr)
        r = sh(["bash", os.path.join(ROOT, "scripts", "build.sh")])
        if r.returncode != 0:
            raise SystemExit("[bars] build.sh failed")

    bara_bin, bara_meta = resolve_bara(args.bara)
    head_meta = {"source": f"main @ {git('rev-parse', '--short', 'HEAD')}",
                 "binary_sha256": sha256(HEAD_BIN),
                 "version": bin_version(HEAD_BIN)}

    suites, scaling = {}, {}
    print(f"HEAD  : {head_meta['version']}  ({head_meta['source']})")
    print(f"Bar A : {bara_meta.get('version', '?')}  ({bara_meta['source']})")
    print(f"Bar B : CPython {platform.python_version()} (native-py mirrors, imported)")
    print(f"Bar C : rustc twins{' (SKIPPED)' if args.skip_c else ''}")
    print()

    hdr = [("workload", 12), ("HEAD", 9), ("barA", 9), ("barB", 10), ("barC", 9),
           ("op/py", 8), ("HEAD/A", 8), ("parity", 7)]
    w = [c[1] for c in hdr]
    print("  ".join(f"{c:>{x}}" for c, x in hdr))
    print("  ".join("-" * x for x in w))

    for name, path, native, _calls, extra in SUITES:
        full = os.path.join(ROOT, path)
        if not os.path.exists(full):
            print(f"skip {name}: {path} missing", file=sys.stderr)
            continue
        row = {}

        # HEAD (operon main build)
        hmn, hmed = run_operon(path, iters, extra)
        hval = sh([HEAD_BIN, "run", path] + list(extra or [])).stdout.strip()
        # Bar A (release binary) — same argv law
        amn, amed = time_cmd([bara_bin, "run", path] + list(extra or []), ROOT, iters)
        aval = sh([bara_bin, "run", path] + list(extra or [])).stdout.strip()
        # Bar B (imported native mirror, in-process)
        bmn, bmed = time_fn(native, iters)

        row["head"] = {"min": hmn, "median": hmed, "value": hval}
        row["bara"] = {"min": amn, "median": amed, "value": aval}
        row["barb"] = {"min": bmn, "median": bmed}

        # parity: HEAD == Bar A == (py-derived where defined) == (Bar C where run)
        expected = py_line(name, native())
        parity = hval == aval
        checks = ["head==bara"]
        if expected is not None:
            parity = parity and hval == expected
            checks.append("==py")
        row["values"] = {"head": hval, "bara": aval, "py": expected,
                         "contract": "; ".join(checks)}

        cmn = None
        if args.skip_c or C_TWINS.get(name) is None:
            if not args.skip_c:
                row["barc"] = {"refused": True}
        else:
            src_name, kind = C_TWINS[name]
            cval, _ = run_c_once(name, src_name)
            row["values"]["barc"] = cval
            if kind == "callfib":
                # "CALLFIB n=25 r=242785 ms=..." — r IS fib(25)
                r_tok = [t for t in cval.split() if t.startswith("r=")][0][2:]
                parity = parity and (r_tok == expected.split("= ")[-1])
                row["values"]["barc"] = f"fib(25) = {r_tok}"
            elif name == "seq":
                # py mirror returns only the sum; parity = triple-sum
                # decomposition + the triple matching Bar C byte-for-byte
                parts = dict(p.split("=") for p in hval.split())
                tot = sum(int(v) for v in parts.values())
                parity = parity and (tot == native()) and (cval == hval)
                row["values"]["py"] = f"triple-sum = {tot}"
            else:
                parity = parity and (cval == hval) and (
                    expected is None or cval == expected)
            cmn, cmed = c_time(name, src_name, iters)
            row["barc"] = {"min": cmn, "median": cmed}

        if not parity:
            print(f"[bars] VALUE PARITY FAILURE on {name}: "
                  f"head={hval!r} bara={aval!r} py={expected!r} "
                  f"barc={row['values'].get('barc')!r}", file=sys.stderr)
            raise SystemExit("[bars] refusing to record timings for a disagreeing "
                             "runner (differential-verified benchmarking)")
        row["parity"] = True

        opms = row["head"]["min"] * 1000
        ccell = ("refused" if row.get("barc", {}).get("refused")
                 else f"{cmn*1000:.1f}" if cmn is not None else "-")
        cells = [
            name, f"{opms:.1f}", f"{amn*1000:.1f}",
            f"{bmn*1000:.1f}", ccell,
            f"{opms/(bmn*1000):.1f}x", f"{hmn/amn:.2f}", "OK",
        ]
        print("  ".join(f"{c:>{x}}" for c, x in zip(cells, w)))
        suites[name] = row

    # ------------------------------------------------------------------
    # scaling: base vs *_2x.op on HEAD / Bar A / Bar C
    # ------------------------------------------------------------------
    print("\nscaling t(2N)/t(N) — HEAD / Bar A / Bar C (Bar B: size-fixed "
          "mirrors, noted not a gap; fib25+recursion: exponential by "
          "construction)")
    shdr = [("workload", 12), ("HEAD", 10), ("barA", 10), ("barC", 10)]
    sw = [c[1] for c in shdr]
    print("  ".join(f"{c:>{x}}" for c, x in shdr))
    print("  ".join("-" * x for x in sw))

    NO_SCALE = {"fib25", "recursion"}
    for name, path, native, _calls, extra in SUITES:
        if name in NO_SCALE:
            continue
        base_p = os.path.join(ROOT, path)
        x2_p = os.path.join(ROOT, path.replace(".op", "_2x.op"))
        if not os.path.exists(x2_p):
            continue
        srow = {}
        vals2 = {}
        for lane, binary in (("head", HEAD_BIN), ("bara", bara_bin)):
            t1 = time_cmd([binary, "run", base_p] + list(extra or []), ROOT, iters)[0]
            t2 = time_cmd([binary, "run", x2_p] + list(extra or []), ROOT, iters)[0]
            srow[lane] = {"t1": t1, "t2": t2, "ratio": t2 / t1}
            # the 2x fixtures are NEW programs — differential-verify them too:
            # both binaries must print the same value at the doubled size.
            vals2[lane] = sh([binary, "run", x2_p] + list(extra or [])).stdout.strip()
        if vals2["head"] != vals2["bara"]:
            raise SystemExit(f"[bars] 2x VALUE PARITY FAILURE on {name}: "
                             f"head={vals2['head']!r} bara={vals2['bara']!r} — "
                             "refusing a scaling row for disagreeing runners")
        crow = None
        if not args.skip_c and C_TWINS.get(name) and name in BASE_N \
                and C_TWINS[name][1] != "callfib":
            src_name = C_TWINS[name][0]
            p = twin_bin(src_name)
            c1 = time_cmd(c_cmd(name, p), ROOT, iters)[0]
            c2 = time_cmd(c_cmd(name, p, BASE_N[name] * 2), ROOT, iters)[0]
            crow = {"t1": c1, "t2": c2, "ratio": c2 / c1}
            # twin at 2n must agree with the engines' 2x fixture value
            cval2 = run_c_once(name, src_name, BASE_N[name] * 2)[0]
            if cval2 != vals2["head"]:
                raise SystemExit(f"[bars] 2x twin disagreement on {name}: "
                                 f"twin={cval2!r} engines={vals2['head']!r}")
        scaling[name] = {"head": srow["head"], "bara": srow["bara"],
                         "barc": crow, "value_2x": vals2["head"]}
        cells = [
            name,
            f"{srow['head']['ratio']:.2f}",
            f"{srow['bara']['ratio']:.2f}",
            f"{crow['ratio']:.2f}" if crow else "-",
        ]
        print("  ".join(f"{c:>{x}}" for c, x in zip(cells, sw)))

    # ------------------------------------------------------------------
    out = {"meta": {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "host": platform.node(), "os": platform.platform(), "cpu": cpu_model(),
        "python": platform.python_version(), "iters": iters,
        "timing": "end-to-end process wall time, min over iters (1 warmup)",
        "head": head_meta, "bara": bara_meta,
        "method": ("Bar A = previous release binary (sha256 vs the release "
                   "SHA256SUMS); Bar B = bench_compare native-py mirrors "
                   "(imported); Bar C = rustc -O twins; regex/json/modules "
                   "refused (no std-only Rust analog); value parity enforced "
                   "before any timing is recorded"),
    }, "suites": suites, "scaling": scaling}
    os.makedirs(os.path.dirname(args.json), exist_ok=True)
    with open(args.json, "w") as f:
        json.dump(out, f, indent=2)
    print(f"\nJSON written to {args.json}")


if __name__ == "__main__":
    main()
