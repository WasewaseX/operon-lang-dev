# ytdl — the same downloader app in 4 languages, benchmarked

A lightweight video-download orchestrator (doctor / info / get / meta /
queue) that drives **yt-dlp + ffmpeg + ffprobe from PATH** — implemented
once in **Operon** and ported to **Python**, **Rust (std-only)** and
**Node** with an identical CLI contract, then benchmarked across 8 metrics.

Full analysis, numbers and ratings: **[REPORT.md](REPORT.md)**.

## The Operon app (the point of this folder)

```bash
apps/ytdl/ytdl doctor
apps/ytdl/ytdl info <url>
apps/ytdl/ytdl get <url> [--format F] [--out D] [--audio] [--subs L] [--resume]
apps/ytdl/ytdl meta <file>
apps/ytdl/ytdl queue jobs.txt [--workers N]
```

- App logic: `operon/ytdl.op` — 100% Operon genes (294 code lines).
- Sandbox: default-deny capabilities; `operon/ytdl.cell` grants exactly
  `run = yt-dlp, ffmpeg, ffprobe, aria2c` + a 300 s per-child timeout;
  the launcher adds `--allow-read` (app dir), `--allow-exit`, fuel headroom.
- Concurrency: `spawn`/`join` map-reduce — one real OS thread per chunk,
  serialized values at the boundary, zero shared mutable state.
- Ships 2.53 MB total (9.9 KB app + 2.5 MB shared VM). No bundling of
  yt-dlp/ffmpeg; nothing else from PATH.

## Ports (same contract, for the benchmark)

- `python/ytdl.py` — CPython, `subprocess` + `ThreadPoolExecutor`
- `rust/` — std-only native binary, scoped threads, hand-rolled JSON parser
- `node/ytdl.mjs` — Node 24, promisified `execFile`, event-loop worker pool

## Benchmark

```bash
apps/ytdl/bench/bench.sh    # offline, deterministic shims; results in bench/results/
```

Measured: LOC, artifact size, cold start, spawn overhead, queue throughput
(1/4/8 workers), host peak RSS, error-path latency/exit codes, in-host
compute. Protocol and validity notes in REPORT.md §6.
