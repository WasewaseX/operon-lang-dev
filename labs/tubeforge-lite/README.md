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
- **Operon policy lane** (optional): user-written `.op` scripts — sandboxed,
  shareable download rules enforced by the engine (see below)
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
src/policy.ts      Operon policy lane: load .op rules, gate enqueue + transfers
src/aria2.ts       bundle extraction + PATH wiring (sha256-pinned)
src/aria2-bundle.ts  AUTO-GENERATED gzip+base64 of one aria2c (per compile target)
operon/policies/   example policy scripts (default, audio-only, corporate)
shared/            the TubeForge engine, vendored verbatim:
                     platform.ts   tool resolution, spawn/kill-tree, fs utils
                     ytdlp.ts      probe (-J), arg building, progress parsing
                     engine.ts     queue, persistence, events, cancel, policy hook
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

## The Operon policy lane

The one feature nothing else in this stack can offer. A **policy** is an
ordinary [Operon](../../README.md) script that computes your download rules
and prints `key = value` lines. It executes in Operon's **deny-by-default
sandbox** — no filesystem, no network, no processes, no `exit` — so a policy
file from a stranger cannot touch your machine; the worst it can do is refuse
your downloads.

```bash
# enable (either way)
TUBEFORGE_POLICY=./operon/policies/audio-only.op tubeforge-lite serve
tubeforge-lite serve --policy ./operon/policies/corporate.op
# policy scripts need the operon binary (TF_OPERON or PATH "operon")
```

Example — `operon/policies/audio-only.op` (real computation, real sandbox):

```
gene mib(n) { return n * 1048576 }
cap = mib(120)
promote("name = audio-only")
promote("allow_video = 0")
promote("max_bytes =", cap)
```

| Key                        | Effect                                                   |
|----------------------------|----------------------------------------------------------|
| `allow_video` / `allow_audio` / `allow_playlist` | deny a whole class of jobs (HTTP 403 with reason) |
| `max_bytes`                | abort a transfer once its total size is known and over N |
| `max_quality`              | silently downgrade video requests above N px             |
| `force_kind`               | `"audio"` rewrites video requests to audio               |
| `deny_domains` / `allow_domains` | URL substring blocklist / strict whitelist          |
| `reason_*`                 | your own human-readable denial messages                  |

Enforcement is two-layered: **preventive** at enqueue (403 with the policy
reason, `force_kind` rewrite, quality ceiling) and **reactive** during the
transfer (size-cap abort the moment yt-dlp reports the total). `doctor` and
`/api/tf/health` always report the active policy; denials carry the policy
name. **Fail-closed**: if the policy is configured but operon is missing or
the runtime crashes, every download is refused until it is fixed. (Script
content itself cannot make operon fail — total grammar, stress containment,
and capability-gated `exit` see to that — which is exactly why the guard
points at the infrastructure instead.)

## Relationship to the Operon language project

The core downloader deliberately does not need Operon — that was the point of
the capability test (yt-dlp + ffmpeg + aria2 + Deno carry the app). The policy
lane above is Operon's earned seat: the one capability the rest of the stack
cannot replicate, because sharing arbitrary user scripts safely requires a
deny-by-default total-grammar VM, not a `bash -c`. What the TubeForge builds
also rehearse for Operon is single-exe release packaging discipline
(per-target embedding, cross-compile, self-extracting assets) — the same
discipline `operon publish` / release distribution needs.

## Test

```bash
bash scripts/e2e.sh                # 26-check hermetic suite (mock yt-dlp/ffmpeg)
```

Covers: tool health incl. bundled-aria2 extraction → probe (formats/subtitles)
→ video job → audio job → cancel mid-download → SSE stream → settings
persistence → files list/raw/delete → CLI download → doctor → **Operon policy
lane** (kind denial 403, computed caps, size-cap abort, fail-closed when the
operon binary is missing) — the policy phase auto-skips when no operon binary
is present.

## Honest limitations

- The binary embeds **aria2 1.37.0** (official release). yt-dlp and ffmpeg are
  never embedded — you manage them (that's the "lite" deal: one small file per
  target instead of a bloated mega-exe that goes stale in weeks).
- YouTube bot-checks datacenter IPs aggressively; age-gated videos need a
  cookies file (Settings → Cookies). This is a yt-dlp reality, not a Lite bug.
- Only x86-64 Linux + Windows targets are preconfigured in `build.sh`; macOS is
  `deno compile --target aarch64-apple-darwin` away.
- The policy lane adds nothing to the compiled binaries' size and stays fully
  off unless `TUBEFORGE_POLICY`/`--policy` is set, but it does need the `operon`
  binary on the machine (not bundled — same "you manage it" deal as yt-dlp).
- Use for content you have the right to download. Respect creators and platform
  ToS.
