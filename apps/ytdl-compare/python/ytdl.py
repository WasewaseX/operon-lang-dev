#!/usr/bin/env python3
"""ytdl.py — YouTube downloader orchestrator, Python comparison build.

Same subcommands, same option surface, same decision logic as the Operon
edition (apps/ytdl/ytdl.op). The `selfcheck` subcommand prints a decision
matrix that must be byte-identical to the Operon build's output — that is
the cross-language differential pin.

Engines: yt-dlp + ffmpeg from PATH (aria2c optional). No third-party deps.
"""

import json
import os
import subprocess
import sys

TIERS = ["2160", "1440", "1080", "720", "480", "360", "240", "144"]
AUDIO_KINDS = ["mp3", "m4a", "opus", "flac", "wav"]
PARTIAL_SUFFIXES = (".part", ".ytdl", ".aria2", ".tmp")


# ---------------------------------------------------------- formatting

def fmt_bytes(n):
    if n is None:
        return "?"
    f = float(n)
    if f >= 1073741824.0:
        return f"{f / 1073741824.0:.2f} GB"
    if f >= 1048576.0:
        return f"{f / 1048576.0:.1f} MB"
    if f >= 1024.0:
        return f"{f / 1024.0:.1f} KB"
    return f"{n} B"


def pad2(n):
    return f"0{n}" if n < 10 else str(n)


def fmt_dur(s):
    if s is None:
        return "?"
    t = int(s)
    h, rem = divmod(t, 3600)
    m, sec = divmod(rem, 60)
    if h > 0:
        return f"{h}:{pad2(m)}:{pad2(sec)}"
    return f"{m}:{pad2(sec)}"


def comma(n):
    if n is None:
        return "?"
    return f"{int(n):,}"


def mget(m, k):
    v = m.get(k)
    return "" if v is None else str(v)


def tail_lines(s, n):
    ls = [ln for ln in s.strip().split("\n") if ln != ""]
    if len(ls) <= n:
        return "\n".join(ls)
    return "\n".join(ls[-n:])


# ---------------------------------------------------------- engines

def run_prog(prog, args, timeout=300):
    try:
        p = subprocess.run([prog] + args, capture_output=True, text=True,
                           timeout=timeout)
        return {"code": p.returncode, "stdout": p.stdout, "stderr": p.stderr}
    except FileNotFoundError:
        raise RuntimeError(f"run denied — '{prog}' not found or not granted")
    except subprocess.TimeoutExpired:
        return {"code": 0, "stdout": "", "stderr": ""}


def tool_line(prog, flag):
    try:
        r = run_prog(prog, [flag], timeout=30)
        if r["code"] == 0:
            return r["stdout"].strip().split("\n")[0]
    except RuntimeError:
        pass
    return ""


def detect_tools():
    return {
        "ytdlp": tool_line("yt-dlp", "--version"),
        "ffmpeg": tool_line("ffmpeg", "-version"),
        "aria2": tool_line("aria2c", "--version"),
    }


# ---------------------------------------------------------- selection logic
# must stay IDENTICAL to apps/ytdl/ytdl.op (differential-pinned)

def quality_expr(q):
    if q == "best":
        return "bestvideo*+bestaudio/best"
    return f"bestvideo[height<={q}]+bestaudio/best[height<={q}]/best"


def selection_args(o):
    if o.get("fmt"):
        return ["-f", o["fmt"]]
    if o.get("audio"):
        return ["-f", "bestaudio/best", "-x", "--audio-format", o["audio"],
                "--audio-quality", "0"]
    return ["-f", quality_expr(o["quality"])]


def build_get_args(url, o):
    a = []
    if not o["playlist"]:
        a.append("--no-playlist")
    a += ["--continue", "--retries", "3", "--fragment-retries", "3",
          "--no-overwrites"]
    if o["aria2"] and o["aria2_ok"]:
        a += ["--downloader", "aria2c",
              "--downloader-args", "aria2c:-x16 -s16 -k1M --file-allocation=none"]
    if o["subs"]:
        a += ["--write-subs", "--sub-langs", o["sub_langs"],
              "--convert-subs", "srt"]
    a += ["-P", o["out"], "-o", o["template"]]
    a += selection_args(o)
    a.append(url)
    return a


# ---------------------------------------------------------- format table

def h_of(f):
    h = f.get("height")
    return 0 if h is None else h


def t_of(f):
    t = f.get("tbr")
    return 0 if t is None else t


def fmt_before(a, b):
    if h_of(a) > h_of(b):
        return True
    if h_of(a) < h_of(b):
        return False
    return t_of(a) > t_of(b)


def res_of(f):
    h = h_of(f)
    if h <= 0:
        return "audio" if mget(f, "vcodec") == "none" else "?"
    w = f.get("width") or 0
    return f"{w}x{h}" if w > 0 else f"?x{h}"


def size_of(f):
    if f.get("filesize") is not None:
        return fmt_bytes(f["filesize"])
    if f.get("filesize_approx") is not None:
        return f"~{fmt_bytes(f['filesize_approx'])}"
    return "?"


def clip(s, w):
    return (s[: w - 2] + "..") if len(s) > w else s


def fmt_row(f):
    fps = ""
    if f.get("fps") is not None:
        fps = str(int(f["fps"]))
    vc = mget(f, "vcodec") or "?"
    if vc == "none":
        vc = "-"
    ac = mget(f, "acodec") or "?"
    if ac == "none":
        ac = "-"
    return (f"{clip(mget(f, 'format_id'), 9):<9}  {clip(mget(f, 'ext'), 4):<4}  "
            f"{clip(res_of(f), 10):<10}  {clip(fps, 4):>4}  {clip(size_of(f), 9):>9}  "
            f"{clip(vc + '/' + ac, 19):<19}  {mget(f, 'format_note')}")


def info_lines(meta):
    out = []
    out.append(f"title: {mget(meta, 'title')}")
    out.append(f"by: {mget(meta, 'uploader')}  |  duration: "
               f"{fmt_dur(meta.get('duration'))}  |  views: {comma(meta.get('view_count'))}")
    n = len(meta.get("formats", []))
    out.append("formats: %d  (res = WxH, size from yt-dlp, ~ = approximate)" % n)
    out.append("id        ext   res         fps      size  vcodec/acodec        note")
    for f in sorted(meta.get("formats", []), key=lambda x: (-h_of(x), -t_of(x))):
        out.append(fmt_row(f))
    tools = detect_tools()
    if tools["aria2"]:
        out.append(f"accel: aria2c {tools['aria2']} (multi-connection enabled)")
    else:
        out.append("accel: aria2c not found — single-connection downloads")
    return out


# ---------------------------------------------------------- download

def has_partial(d):
    try:
        return any(n.endswith(PARTIAL_SUFFIXES) for n in os.listdir(d))
    except OSError:
        return False


def dest_lines(stdout, stderr):
    found = []
    for src in (stdout, stderr):
        for ln in src.split("\n"):
            l = ln.strip()
            if ("Destination:" in l or "has already been downloaded" in l
                    or "[Merger]" in l or "Download complete:" in l
                    or "ExtractAudio" in l):
                found.append(l)
    return found


def download(url, o):
    attempts = 0
    last = {"code": -1, "stdout": "", "stderr": ""}
    while True:
        attempts += 1
        last = run_prog("yt-dlp", build_get_args(url, o))
        if last["code"] == 0 and not has_partial(o["out"]):
            break
        if attempts >= o["max_attempts"]:
            break
    ok = last["code"] == 0 and not has_partial(o["out"])
    return {"ok": ok, "attempts": attempts, "r": last}


def report(d):
    for ln in dest_lines(d["r"]["stdout"], d["r"]["stderr"]):
        print(f"  {ln}")
    print(f"  attempts: {d['attempts']}  result: {'ok' if d['ok'] else 'FAIL'}")
    if not d["ok"]:
        r = d["r"]
        if r["code"] != 0:
            print(f"  yt-dlp exited {r['code']}:")
        else:
            print(f"  incomplete after {d['attempts']} rounds:")
        print(tail_lines(r["stderr"], 4))


# ---------------------------------------------------------- option layer

def get_opts(rest):
    def val(name, default):
        f = f"--{name}"
        for i, a in enumerate(rest):
            if a == f:
                return rest[i + 1] if i + 1 < len(rest) else default
            if a.startswith(f + "="):
                return a[len(f) + 1:]
        return default

    def flag(name):
        return f"--{name}" in rest

    pos = []
    i = 0
    while i < len(rest):
        a = rest[i]
        if a.startswith("--"):
            i += 1 if "=" in a else 2
        elif a.startswith("-"):
            i += 1
        else:
            pos.append(a)
            i += 1

    o = {
        "url": pos[0] if pos else "",
        "out": val("out", "downloads"),
        "quality": val("quality", "best"),
        "audio": val("audio", ""),
        "fmt": val("format", ""),
        "subs": flag("subs"),
        "sub_langs": val("sub-langs", "en"),
        "jobs": int(val("jobs", 2)),
        "aria2": not flag("no-aria2"),
        "playlist": flag("playlist"),
        "template": val("template", "%(title)s [%(id)s].%(ext)s"),
        "max_attempts": int(val("max-attempts", 4)),
        "aria2_ok": bool(tool_line("aria2c", "--version")),
    }
    if o["audio"] and o["audio"] not in AUDIO_KINDS:
        die(f"error: --audio must be one of mp3 m4a opus flac wav (got {o['audio']})")
    if o["quality"] != "best" and o["quality"] not in TIERS:
        die("error: --quality must be a height tier (got %s); use --format for raw selectors" % o["quality"])
    return o


def die(msg, code=2):
    print(msg)
    sys.exit(code)


def ensure_out(d):
    os.makedirs(d, exist_ok=True)


# ---------------------------------------------------------- commands

def cmd_doctor():
    t = detect_tools()
    print("ytdl doctor — probing PATH engines")
    ytdlp_ok = ffmpeg_ok = False
    if t["ytdlp"]:
        ytdlp_ok = True
        print(f"  yt-dlp   {t['ytdlp']}  [required: present]")
    else:
        print("  yt-dlp   MISSING  [required: pip install yt-dlp or brew install yt-dlp]")
    if t["ffmpeg"]:
        ffmpeg_ok = True
        print(f"  ffmpeg   {t['ffmpeg']}  [required: present]")
    else:
        print("  ffmpeg   MISSING  [required: apt/brew install ffmpeg]")
    if t["aria2"]:
        print(f"  aria2c   {t['aria2']}  [optional: multi-connection downloads]")
    else:
        print("  aria2c   not found  [optional: apt/brew install aria2 for 16-connection accel]")
    if ytdlp_ok and ffmpeg_ok:
        print("verdict: READY — python runtime + PATH engines")
        sys.exit(0)
    print("verdict: NOT READY — missing required engine(s)")
    sys.exit(1)


def cmd_info(rest):
    o = get_opts(rest)
    if not o["url"]:
        die("error: info needs a URL")
    try:
        r = run_prog("yt-dlp", ["-J", "--no-playlist", o["url"]])
    except RuntimeError as e:
        die(f"yt-dlp denied — {e}", 1)
    if r["code"] != 0:
        print(f"yt-dlp failed (code {r['code']}):")
        print(tail_lines(r["stderr"], 3))
        sys.exit(1)
    meta = json.loads(r["stdout"])
    for ln in info_lines(meta):
        print(ln)
    sys.exit(0)


def cmd_get(rest):
    o = get_opts(rest)
    if not o["url"]:
        die("error: get needs a URL")
    print(f"ytdl[python]: get {o['url']}")
    print(f"  out={o['out']}  selector: {' '.join(selection_args(o))}")
    ensure_out(o["out"])
    d = download(o["url"], o)
    report(d)
    sys.exit(0 if d["ok"] else 1)


def queue_worker(urls, o):
    rows = []
    for u in urls:
        line = u.strip()
        if not line or line.startswith("#"):
            continue
        d = download(line, o)
        rows.append(f"{'ok' if d['ok'] else 'FAIL'}\t{d['attempts']}\t{line}")
    return rows


def cmd_queue(rest):
    if not rest or rest[0].startswith("--"):
        die("error: queue needs a file (one URL per line, # comments)")
    file = rest[0]
    o = get_opts(rest[1:])
    try:
        with open(file, "r", encoding="utf-8") as fh:
            text = fh.read()
    except OSError as e:
        die(f"error: cannot read queue file {file} ({e})")
    urls = [ln.strip() for ln in text.split("\n")
            if ln.strip() and not ln.strip().startswith("#")]
    jobs = max(1, min(o["jobs"], len(urls)))
    if jobs == 1:
        rows = queue_worker(urls, o)
    else:
        chunks = [[] for _ in range(jobs)]
        for idx, u in enumerate(urls):
            chunks[idx % jobs].append(u)
        from concurrent.futures import ThreadPoolExecutor
        with ThreadPoolExecutor(max_workers=jobs) as ex:
            nested = list(ex.map(lambda c: queue_worker(c, o), chunks))
        rows = [r for chunk in nested for r in chunk]
    failures = 0
    for r in rows:
        print(r)
        if not r.startswith("ok\t"):
            failures += 1
    print(f"queue: {len(rows)} items, {failures} failed (workers: {jobs})")
    sys.exit(1 if failures else 0)


def cmd_selfcheck(rest):
    if not rest:
        die("error: selfcheck needs a fixture .json path")
    with open(rest[0], "r", encoding="utf-8") as fh:
        meta = json.loads(fh.read())
    for q in ["best"] + TIERS:
        o = {"quality": q, "audio": "", "fmt": ""}
        print(f"q={q} a=- :: {' '.join(selection_args(o))}")
    for kind in AUDIO_KINDS:
        o = {"quality": "best", "audio": kind, "fmt": ""}
        print(f"q=- a={kind} :: {' '.join(selection_args(o))}")
    fs = sorted(meta["formats"], key=lambda x: (-h_of(x), -t_of(x)))
    ids = [str(f.get("format_id")) for f in fs]
    print("sort-check: " + ",".join(ids))
    h = info_lines(meta)
    print(f"hdr-check: {h[0]}")
    print(f"hdr-check: {h[1]}")
    sys.exit(0)


def usage():
    print("ytdl — YouTube downloader, Python comparison build")
    print("")
    print("usage: ytdl.py doctor | info URL | get URL [options] | queue FILE [options]")
    print("       ytdl.py selfcheck FIXTURE.json   (differential decision matrix)")
    print("options: --out DIR --quality N --format EXPR --audio KIND --subs")
    print("         --sub-langs LANGS --template T --jobs N --no-aria2")
    print("         --playlist --max-attempts N")


def main():
    a = sys.argv[1:]
    if not a:
        usage()
        sys.exit(2)
    sub, rest = a[0], a[1:]
    if sub in ("help", "--help", "-h"):
        usage()
        sys.exit(0)
    if sub == "doctor":
        cmd_doctor()
    elif sub == "info":
        cmd_info(rest)
    elif sub == "get":
        cmd_get(rest)
    elif sub == "queue":
        cmd_queue(rest)
    elif sub == "selfcheck":
        cmd_selfcheck(rest)
    else:
        print(f"error: unknown subcommand {sub}")
        usage()
        sys.exit(2)


if __name__ == "__main__":
    main()
