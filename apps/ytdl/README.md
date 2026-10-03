# ytdl — a YouTube downloader written in Operon (lightweight edition)

One **3.6 MB** runtime + the engines you already have in PATH. No bundle,
no Electron, no 80 MB Deno payload.

```
apps/ytdl/
  ytdl.op        the app (Operon)
  ytdl.cell      runtime contract: run timeout 5 min, exit grant
  ytdl.sh        POSIX launcher (sets capability grants, then runs ytdl.op)
  ytdl.bat       Windows launcher (same grants)
  test/          35-check battery + cross-language differential
```

## Quick start

```sh
# from the repo root (or install operon and put it on PATH):
./apps/ytdl/ytdl.sh doctor
./apps/ytdl/ytdl.sh info "https://www.youtube.com/watch?v=VIDEO_ID"
./apps/ytdl/ytdl.sh get VIDEO_URL --quality 1080 --out downloads
./apps/ytdl/ytdl.sh get VIDEO_URL --audio mp3 --out music
printf '%s\n' URL1 URL2 URL3 > queue.txt
./apps/ytdl/ytdl.sh queue queue.txt --jobs 3 --out downloads
```

Windows: `apps\ytdl\ytdl.bat get VIDEO_URL --quality 1080`.

## Options

| flag | meaning | default |
|---|---|---|
| `--out DIR` | destination directory (only directory the app may write) | `downloads` |
| `--quality N` | max height tier: 2160 1440 1080 720 480 360 240 144 | `best` |
| `--format EXPR` | raw yt-dlp format selector (wins over `--quality`) | — |
| `--audio KIND` | audio-only: mp3 m4a opus flac wav | — |
| `--subs` / `--sub-langs L` | write + convert subtitles to srt | `en` |
| `--template T` | filename template (yt-dlp `-o`) | `%(title)s [%(id)s].%(ext)s` |
| `--jobs N` | queue workers | `2` |
| `--no-aria2` | disable aria2c acceleration | aria2c on if found |
| `--playlist` | allow playlists | off |
| `--max-attempts N` | checkpoint-resume rounds | `4` |

## How it works

The app is a pure orchestrator: `yt-dlp` resolves formats and downloads,
`ffmpeg` converts audio/subs, `aria2c` (if present) parallelizes transfers.
Operon contributes the typed option layer, the format-table logic, a
concurrent queue over OS-thread `spawn`/`join`, and — because long
downloads outlive the interpreter's 5-minute `run()` ceiling — a
**checkpoint-resume loop**: while a `.part` file exists, `yt-dlp
--continue` is re-invoked in bounded rounds.

Everything runs in Operon's default-deny sandbox. The launcher grants
exactly `yt-dlp`, `ffmpeg`, `aria2c`, read on cwd/app-dir/out-dir, write
on out-dir, and `exit`. Anything else is a catchable denial — see
`docs/APP-COMPARISON.md` for the security model and the comparison story.

## Tests

```sh
bash apps/ytdl/test/run_tests.sh     # e2e on deterministic mock engines
```

The battery covers: doctor/info/get/get-audio, the stall-resume path, the
failure path, concurrent queues (including failing items), usage errors,
and the byte-identical decision-matrix differential against the Python
build (`apps/ytdl-compare/python/ytdl.py`).

## Benchmarks

Every build (this app + the three comparison builds) exposes a uniform
`bench-*` workload surface used by the deep cross-language benchmark:

```sh
python3 scripts/bench_deep.py        # medians, RSS, checksums -> apps/ytdl/bench/
```

Workloads: `bench-startup`, `bench-json F --n N`, `bench-table F --rounds R`,
`bench-lines --k K`, `bench-spawn --n N`, `bench-queue --k K --c C`.
Results and analysis: `docs/BENCHMARK-DEEP.md`.
