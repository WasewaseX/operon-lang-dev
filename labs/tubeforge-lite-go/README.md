# TubeForge Lite (Go build, v1.1.0)

A **single-file** YouTube/video downloader for desktop. One native executable —
no installer, no Node/Deno, no Electron, no sidecar folder.

**Why a Go rebuild exists (v1.1.0):** v1.0.0 was `deno compile`, which embeds
the entire Deno/V8 runtime — an 82.5 MB exe for ~120 KB of app logic. This
build keeps the exact same API contract, embedded UI, and bundled-aria2
behavior on the Go stdlib instead:

| Build                    | Windows x64   | Linux x64 |
|--------------------------|---------------|-----------|
| v1.0.0 (deno compile)    | 82.5 MB       | 113.6 MB  |
| v1.1.0 (go, this dir)    | **8.2 MB**    | **11 MB** |

## What's bundled vs. what you need

| Piece  | Where it comes from                                                        |
|--------|----------------------------------------------------------------------------|
| aria2c | **bundled** — gzipped inside the exe (v1.37.0, sha256-pinned), self-extracts on first run into `<dataDir>/bin/` |
| yt-dlp | **from PATH** (or Settings override) — metadata, formats, orchestration     |
| ffmpeg | **from PATH** (or Settings override) — merge/remux/audio extract            |

The server prepends the self-extracted `bin/` dir to PATH, and `resolveTool`
probes well-known dirs plus a real PATH walk. Windows note: PATH resolution now
tries `.exe`/`.cmd`/`.bat` suffixes (fixes a latent v1.0.0 bug where
`yt-dlp.exe` on PATH was never found by the health check).

## Build

```bash
bash scripts/fetch-aria2.sh   # vendor/aria2c.exe + vendor/aria2c-linux
GO=go bash build.sh           # dist/TubeForge-Lite.exe + dist/tubeforge-lite-linux
bash scripts/e2e.sh           # hermetic e2e (mock yt-dlp/ffmpeg/operon)
```

`scripts/gen-bundle.sh` gzips the vendor binaries into `assets/` and regenerates
the build-tagged `aria2_bundle_{windows,linux}.go` (embed + sha256 pin). Each
target only carries the aria2c for its own platform. Missing vendor binaries
produce a neutral placeholder bundle (bundled=false), so a fresh clone always
builds.

## Layout

- `main.go` — entrypoint: `serve` / `doctor` / `version` / headless CLI
- `server.go` — `/api/tf/*` contract (probe/download/queue/events SSE/cancel/retry/remove/files/settings/health) + range-aware raw file serving
- `engine.go` — job store, queue pump (1–4 concurrent), jobs.json persistence, yt-dlp runner, process-tree cancel, policy size gate
- `ytdlp.go` — probe (`-J --flat-playlist`), download args, `__TFPROG__`/`__TFDONE__` progress protocol
- `policy.go` — Operon policy lane (`TUBEFORGE_POLICY` + `TF_OPERON`); fail-closed on missing/broken operon
- `aria2.go` + `aria2_bundle_*.go` — embedded aria2 self-extraction with sha256 verification
- `assets/ui.html` — the v1.0.0 embedded UI, extracted verbatim

The Operon policy scripts still live in `../tubeforge-lite/operon/policies/`
and are evaluated by the external `operon` binary exactly as before.
