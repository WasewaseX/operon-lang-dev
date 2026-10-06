# TubeForge Lite for Android (Operon core)

One APK. The **full Operon language core** compiled natively for the phone
(`liboperon.so`, arm64) runs the TubeForge **policy lane** in its
deny-by-default sandbox — the same `.op` policy files, the same key set,
the same fail-closed contract as the desktop builds. Transfers ride the
platform DownloadManager. WebView UI, no Electron, no Node, no Python.

```
┌ TubeForge Lite (APK) ─────────────────────────────────────┐
│ WebView UI (assets/ui.html)                               │
│   ↕ JS bridge (TfBridge)                                  │
│ OperonBridge ⇄ jni_glue.c ⇄ liboperon.so (Operon core)    │
│   policy.op → promote("key = value") → rules              │
│ DownloadManager (enqueue / progress / cancel)             │
└───────────────────────────────────────────────────────────┘
```

## What's bundled vs. what isn't

| Piece     | Where it comes from                                                |
|-----------|--------------------------------------------------------------------|
| Operon    | **bundled** — the whole language core, ~2–3 MB per ABI, in the APK |
| Policies  | **bundled** — default / audio-only / corporate / mobile-data (.op) |
| Downloader| **platform** — Android DownloadManager (queued, resumable, honest) |
| yt-dlp    | **not bundled** — the "lite" deal, same as desktop: metadata extraction lives on the desktop lane; this build manages direct media URLs |
| ffmpeg    | **not bundled** — no merge/remux on-device in v0.1                 |

## The policy lane (Operon's earned seat, in your pocket)

A policy is an ordinary Operon program executed by the embedded core with
**zero capabilities** (no filesystem, no network, no processes — the CLI's
default-deny contract). It computes your download rules and prints
`key = value` lines; the app enforces them at the gate:

```operon
gene mib(n) { return n * 1048576 }
base_cap = mib(80)

promote("name = mobile-data")
promote("max_bytes =", base_cap)
promote("max_quality = 1080")
```

The UI shows the computed rules, the sandbox verdict (`ok`/stress kind),
and the core's evaluation time. A policy from a stranger cannot touch the
phone — the worst it can do is refuse a download.

## Build (CI does all of it)

`.github/workflows/tubeforge-android.yml`:
1. `cargo ndk -t arm64-v8a build --release` in `operon-ffi/` → `liboperon.so`
   into `app/src/main/jniLibs/` (the FFI crate is a thin C-ABI shell over
   `operon::tools::load_file` — default-deny caps, VM lane, notes captured)
2. Gradle `assembleRelease` (CMake compiles the 40-line JNI glue and links
   the prebuilt core) → signed with the **permanent** TubeForge keystore
   (vault `credentials/tubeforge-android-signing/`, base64 in the
   `TF_KEYSTORE_B64` Actions secret; cert SHA-256 pinned there)

## Honest limitations (v0.1)

- Direct-media URLs only — no yt-dlp on-device (Python is not bundle-able
  under the lite budget; the desktop lane owns extraction)
- Downloads land in the app-private external dir (no storage-permission
  prompts; copy out with any file manager)
- arm64-v8a only (the code path is ABI-agnostic — add targets in the
  workflow's cargo-ndk line as needed)
- The WebView UI is the v1.0.0 desktop page rethought for one hand, not a
  feature twin of the desktop queue
