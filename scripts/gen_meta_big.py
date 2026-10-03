#!/usr/bin/env python3
"""gen_meta_big.py — generate a big deterministic yt-dlp-style metadata fixture.

The deep benchmark (scripts/bench_deep.py) parses this fixture in every
language implementation, so it must be committed and never regenerated
with different content. Invariants:
  - 48 formats, every (height, tbr) pair UNIQUE (sort order fully
    determined — the cross-language differential pin relies on it)
  - realistic field mix: heights 144..2160, audio-only rows (height 0),
    fps on some rows, filesize vs filesize_approx, format_note on some
"""
import json
import random

rng = random.Random(20261003)

VC = ["avc1.640028", "avc1.4d401f", "vp9", "vp9.2", "av01.0.08M.08"]
AC = ["mp4a.40.2", "opus", "none"]
NOTES = [None, "HDR", "premium", "dual", "storyboard", "1080p60-2"]

heights = [2160, 1440, 1080, 1080, 720, 720, 480, 360, 240, 144, 0, 0] * 4
formats = []
used = set()
fid = 394
for i in range(48):
    h = heights[i % len(heights)]
    # unique tbr per (height) band: spread deterministically
    while True:
        tbr = round(rng.uniform(80, 25000), 1)
        if h == 0:
            key = ("a", tbr)
        else:
            key = (h, tbr)
        if key not in used:
            used.add(key)
            break
    f = {"format_id": str(fid), "ext": rng.choice(["mp4", "webm", "mp4"])}
    fid += 3 if i % 2 == 0 else 7
    if h == 0:
        f["vcodec"] = "none"
        f["acodec"] = rng.choice(["mp4a.40.2", "opus"])
        f["abr"] = round(tbr, 1)
    else:
        f["height"] = h
        f["width"] = {2160: 3840, 1440: 2560, 1080: 1920, 720: 1280,
                      480: 854, 360: 640, 240: 426, 144: 256}[h]
        f["vcodec"] = rng.choice(VC[:4])
        f["acodec"] = "none"
        if i % 3 == 0 and h > 0:
            f["fps"] = 30 if i % 6 == 0 else 60
    f["tbr"] = tbr
    if i % 4 == 0:
        f["filesize"] = int(tbr * 1000 * 5021 / 8)
    elif i % 4 == 1:
        f["filesize_approx"] = int(tbr * 1000 * 5021 / 8)
    n = rng.choice(NOTES)
    if n:
        f["format_note"] = n
    formats.append(f)

meta = {
    "id": "b1gbench001",
    "title": "Operon Deep Benchmark Fixture — 48 formats",
    "uploader": "Operon Labs",
    "duration": 5021,
    "view_count": 2847113,
    "average_rating": 4.87,
    "channels": 2,
    "formats": formats,
}

out = "apps/ytdl/test/mock/fixtures/meta_big.json"
with open(out, "w", encoding="utf-8") as fh:
    json.dump(meta, fh, indent=1)
    fh.write("\n")
print(f"wrote {out}: {len(formats)} formats")
