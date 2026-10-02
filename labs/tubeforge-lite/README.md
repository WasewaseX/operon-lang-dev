# TubeForge Lite

A **single-file** YouTube/video downloader for desktop. One native executable —
no installer, no Node, no Electron, no sidecar folder.

```
TubeForge-Lite.exe        ← that's the whole app (Windows)
tubeforge-lite-linux      ← same app, Linux
```

**Stack (as specced):**

| Piece    | Where it comes from                                             |
|----------|------------------------------------------------------------------|
| Deno 2   | **bundled** — it *is* the executable (`deno compile`), server+UI+CLI |
| aria2c   | **bundled** — gzipped inside the exe, self-extracts on first run |
| yt-dlp   | **from PATH** (or Settings override) — metadata, formats, orchestration |
| ffmpeg   | **from PATH** (or Settings override) — merge/remux/audio extract |

TubeForge Lite is the lean sibling of [TubeForge desktop](../desktop/README.md):
same proven download engine (byte-identical Node↔Deno parity was verified in the
full build), minus the React/Next.js export — the UI is a hand-rolled
single-file vanilla page (~30 KB source instead of 1.5 MB of embedded assets),
plus two things the big build doesn't have: **aria2 inside the exe** and a
**headless CLI mode**.

---

## Quickstart

1. Put `yt-dlp` and `ffmpeg` on your PATH
   (`winget install yt-dlp.yt-dlp ffmpeg` / `brew install yt-dlp ffmpeg` / `pip install yt-dlp`).
2. Run the executable. A browser window opens at `http://localhost:8484`.
3. First run extracts the bundled `aria2c` into the data dir (`~/.tubeforge-lite/bin/`)
   and prepends it to PATH — that's why the health chips show `aria2 bundled`.

### Headless CLI (same executable)

```bash
tubeforge-lite "https://youtube.com/watch?v=..." --quality 1080
tubeforge-lite "https://youtube.com/playlist?list=..." --quality 720          # playlist → one job per entry
tubeforge-lite "URL" --audio mp3 --audio-bitrate 320
tubeforge-lite "URL" --container mkv --out ~/Videos --subs "en,de" --embed-subs
tubeforge-lite "URL" --no-aria2                      # use yt-dlp's native downloader
tubeforge-lite doctor                                # tool health + bundle info
```

## Features

- **Analyze** any video/playlist URL: title, channel, duration, formats (id,
  codecs, fps, filesize, bitrate), subtitle languages
- **Download modes**: video (quality cap + mp4/mkv/webm container preference),
  audio-only (mp3/m4a/opus/flac/wav, 320→128k or best), subtitles (write or embed)
- **Playlist picker**: untick entries, enqueues one trackable job per entry
- **aria2c downloader**: multi-connection (`-x N -s N -k 1M`), concurrent
  fragments, automatic fallback to yt-dlp's native downloader when aria2 is off
- **Live queue**: SSE push updates, per-job progress/speed/ETA, engine log
  drawer, cancel / retry / remove, restart-safe persistence (`jobs.json`)
- **Files browser**: list, serve (range-aware streaming), delete downloads
- **Settings**: output dir, filename template, parallelism (1-4), aria2
  connections (1-16), embed flags, defaults, tool path overrides, cookies file
- **Thumbnail embed + metadata embed** via ffmpeg post-processing
- Deny-by-design request validation (http/https only, no path traversal, options
  whitelisted) — the engine never accepts a filesystem path from the browser

## Architecture

```
src/main.ts        entrypoint: serve | <urls> (CLI) | doctor | version
src/server.ts      HTTP API (/api/tf/*) + SSE + embedded UI + auto-open browser
src/ui.ts          single-file vanilla dark UI (zero dependencies)
src/cli.ts         headless mode reusing the same queue engine
src/aria2.ts       bundle extraction + PATH wiring (sha256-pinned)
src/aria2-bundle.ts  AUTO-GENERATED gzip+base64 of one aria2c (per compile target)
shared/            the TubeForge engine, vendored verbatim:
                     platform.ts   tool resolution, spawn/kill-tree, fs utils
                     ytdlp.ts      probe (-J), arg building, progress parsing
                     engine.ts     queue, persistence, events, cancel
                     settings.ts / files.ts / health.ts / validate.ts / types.ts
```

Data layout: `<data>/settings.json`, `<data>/jobs.json`, `<data>/bin/aria2c`,
downloads in `<data>/downloads/` (override: `--data DIR`, `--out DIR`, or the
`TUBEFORGE_DATA` / `TUBEFORGE_DOWNLOADS` env vars). Data dir precedence:
flag → env → portable `data/` next to the exe (if present) → `~/.tubeforge-lite`.

## Build from source

```bash
bash scripts/fetch-aria2.sh        # fetch static aria2 binaries into vendor/ (fresh clones)
bash build.sh                      # → dist/tubeforge-lite-linux + dist/TubeForge-Lite.exe
```

`build.sh` typechecks (`deno check`), embeds the aria2c matching each target
(`scripts/gen-aria2-bundle.ts --platform linux|windows`), compiles locally and
cross-compiles the Windows exe (`--target x86_64-pc-windows-msvc` — no Wine, no
Windows machine needed). A `--platform none` build yields a PATH-only exe
(~6 MB smaller) that still works if aria2c is installed separately.

Measured sizes: linux 114 MB, windows 85 MB — the Deno 2 runtime plus the
compressed per-target aria2c (3–7 MB). Compare Electron (150 MB+ before your app
is even counted) and the full TubeForge desktop (103 MB linux / 80 MB exe
**plus** a `win-tools/` sidecar folder — Lite ships as exactly one file).

## API (compatible with TubeForge desktop)

`POST /api/tf/probe` · `POST /api/tf/download` · `GET /api/tf/queue` ·
`GET /api/tf/events` (SSE, Lite-only) · `POST /api/tf/cancel|retry|remove` ·
`GET|POST /api/tf/files` · `GET /api/tf/files/raw?name=` ·
`GET|POST /api/tf/settings` · `GET /api/tf/health`

## Test

```bash
bash scripts/e2e.sh                # 17-check hermetic suite (mock yt-dlp/ffmpeg)
```

Covers: tool health incl. bundled-aria2 extraction → probe (formats/subtitles)
→ video job → audio job → cancel mid-download → SSE stream → settings
persistence → files list/raw/delete → CLI download → doctor.

## Honest limitations

- The binary embeds **aria2 1.37.0** (official release). yt-dlp and ffmpeg are
  never embedded — you manage them (that's the "lite" deal: one small file per
  target instead of a bloated mega-exe that goes stale in weeks).
- YouTube bot-checks datacenter IPs aggressively; age-gated videos need a
  cookies file (Settings → Cookies). This is a yt-dlp reality, not a Lite bug.
- Only x86-64 Linux + Windows targets are preconfigured in `build.sh`; macOS is
  `deno compile --target aarch64-apple-darwin` away.
- Use for content you have the right to download. Respect creators and platform
  ToS.

## Relationship to the Operon language project

None in the core — by design. Operon (the language under development in the
parent repo) is not required here; its natural future role is an **optional
sandboxed policy layer**: user-written `.op` rules ("cap 1080p, skip > 2 GB,
audio-only for shorts") executed in Operon's deny-by-default VM, safe by
construction. What this build *does* rehearse for Operon is single-exe release
packaging discipline (per-target embedding, cross-compile, self-extracting
assets) — the same discipline `operon publish` / release distribution needs.
