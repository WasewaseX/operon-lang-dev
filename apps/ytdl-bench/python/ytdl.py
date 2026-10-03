#!/usr/bin/env python3
# ============================================================
# ytdl.py — lightweight video-download orchestrator, python edition
# Same CLI contract as operon/ytdl.op, rust/ and node/ ports.
#   python3 ytdl.py doctor | info <url> | get <url> [options] |
#           meta <file> | queue <jobs.txt> [--workers N] |
#           startup | spawnbench N | cpu N
# Engines from PATH: yt-dlp, ffmpeg, ffprobe (aria2c optional).
# ============================================================
import json
import os
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

TOOLS = [("yt-dlp", "--version"), ("ffmpeg", "-version"),
         ("ffprobe", "-version"), ("aria2c", "--version")]
OUT_DEFAULT = "downloads"
CLIP = 60


def clip(s, n=CLIP):
    return str(s).strip()[:n]


def now_s():
    return time.perf_counter()


def probe(tool, flag):
    try:
        r = subprocess.run([tool, flag], capture_output=True, text=True, timeout=10)
        return r.stdout.strip() if r.returncode == 0 else ""
    except (OSError, subprocess.SubprocessError):
        return ""


def cmd_doctor():
    found = 0
    for tool, flag in TOOLS:
        v = probe(tool, flag)
        if v:
            found += 1
            print(f"tool={tool} version={clip(v)}")
        else:
            print(f"tool={tool} MISSING")
    print(f"doctor: ok {found}/{len(TOOLS)}")
    if found < 3:
        sys.exit(1)


def cmd_info(url):
    t0 = now_s()
    try:
        r = subprocess.run(["yt-dlp", "-J", "--no-warnings", url],
                           capture_output=True, text=True, timeout=300)
    except (OSError, subprocess.SubprocessError) as e:
        print(f"info: FAIL err={clip(e, 160)}")
        sys.exit(1)
    if r.returncode != 0:
        print(f"info: FAIL code={r.returncode} err={clip(r.stderr, 160)}")
        sys.exit(1)
    j = json.loads(r.stdout)
    fmts = j.get("formats", [])
    best = ""
    for f in fmts:
        if "format_id" in f:
            best = str(f["format_id"])
    print(f"title={j.get('title', '')}")
    print(f"formats={len(fmts)}")
    print(f"best={best}")
    print(f"info: ok secs={round(now_s() - t0, 3)}")


def build_args(url, mode, fmt, out, resume):
    args = []
    if resume:
        args.append("-c")
    args.append("--no-warnings")
    if mode == "audio":
        args += ["-x", "--audio-format", "mp3"]
    elif mode == "subs":
        args += ["--write-subs", "--skip-download", "--sub-langs", "en"]
    else:
        args += ["-f", fmt]
    args += ["-o", f"{out}/%(title)s.%(ext)s", url]
    return args


def do_job(idx, url, mode, fmt="best", out=OUT_DEFAULT, resume=False):
    t0 = now_s()
    print(f"[get] start url={url} mode={mode} fmt={fmt}")
    try:
        r = subprocess.run(["yt-dlp"] + build_args(url, mode, fmt, out, resume),
                           capture_output=True, text=True, timeout=300)
    except (OSError, subprocess.SubprocessError) as e:
        return 0, f"job {idx}: {mode} {url} -> FAIL(err={clip(e, 60)}) secs={round(now_s() - t0, 3)}"
    wall = round(now_s() - t0, 3)
    if r.returncode != 0:
        return 0, f"job {idx}: {mode} {url} -> FAIL(code={r.returncode}) secs={wall}"
    return 1, f"job {idx}: {mode} {url} -> ok secs={wall}"


def cmd_get(url, fmt, out, mode, resume):
    ok, _ = do_job(0, url, mode, fmt, out, resume)
    if not ok:
        sys.exit(1)
    print("get: ok")


def cmd_meta(path):
    try:
        r = subprocess.run(["ffprobe", "-v", "quiet", "-print_format", "json",
                            "-show_format", path],
                           capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.SubprocessError) as e:
        print(f"meta: FAIL err={clip(e, 160)}")
        sys.exit(1)
    if r.returncode != 0:
        print(f"meta: FAIL code={r.returncode} err={clip(r.stderr, 160)}")
        sys.exit(1)
    f = json.loads(r.stdout).get("format", {})
    print(f"duration={f.get('duration', '?')} size={f.get('size', '?')}")
    print("meta: ok")


def load_jobs(path):
    jobs = []
    with open(path) as fh:
        for i, line in enumerate(fh):
            parts = line.strip().split(" ")
            if len(parts) >= 2:
                jobs.append((len(jobs), parts[0], parts[1]))
    return jobs


def cmd_queue(jobsfile, workers):
    t0 = now_s()
    jobs = load_jobs(jobsfile)
    total = len(jobs)
    if total == 0:
        print("queue: no jobs")
        sys.exit(2)
    w = min(workers, total)
    with ThreadPoolExecutor(max_workers=w) as pool:
        futs = [pool.submit(do_job, idx, url, mode) for idx, url, mode in jobs]
        oks = 0
        for fut in futs:
            ok, line = fut.result()
            oks += ok
            print(line)
    wall = round(now_s() - t0, 3)
    print(f"queue: {oks}/{total} ok in {wall}s (workers={w})")
    if oks < total:
        sys.exit(1)


def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)


def cmd_spawnbench(n):
    t0 = now_s()
    for i in range(n):
        r = subprocess.run(["yt-dlp", "--version"], capture_output=True)
        if r.returncode != 0:
            print(f"spawnbench: FAIL at {i}")
            sys.exit(1)
    print(f"spawnbench: ok n={n} secs={round(now_s() - t0, 3)}")


def cmd_cpu(n):
    t0 = now_s()
    v = fib(n)
    print(f"cpu: ok fib({n})={v} secs={round(now_s() - t0, 3)}")


def flag_of(a, name, dflt):
    return a[a.index(name) + 1] if name in a else dflt


def usage(code=2):
    print("usage: ytdl doctor | info <url> | get <url> [options] | "
          "meta <file> | queue <jobs> [--workers N] | startup | "
          "spawnbench N | cpu N")
    sys.exit(code)


def main():
    a = sys.argv[1:]
    if not a:
        usage()
    cmd = a[0]
    if cmd == "doctor":
        cmd_doctor()
    elif cmd == "info":
        cmd_info(a[1]) if len(a) >= 2 else usage()
    elif cmd == "get":
        if len(a) < 2:
            usage()
        mode = "audio" if "--audio" in a else ("subs" if "--subs" in a else "video")
        cmd_get(a[1], flag_of(a, "--format", "bv*+ba/b"),
                flag_of(a, "--out", OUT_DEFAULT), mode, "--resume" in a)
    elif cmd == "meta":
        cmd_meta(a[1]) if len(a) >= 2 else usage()
    elif cmd == "queue":
        if len(a) < 2:
            usage()
        cmd_queue(a[1], int(flag_of(a, "--workers", "4")))
    elif cmd == "startup":
        print("startup: ok")
    elif cmd == "spawnbench":
        cmd_spawnbench(int(a[1])) if len(a) >= 2 else usage()
    elif cmd == "cpu":
        cmd_cpu(int(a[1])) if len(a) >= 2 else usage()
    else:
        print(f"ytdl: unknown command '{cmd}'")
        sys.exit(2)


if __name__ == "__main__":
    main()
