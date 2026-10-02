// TubeForge — yt-dlp integration: probing, argument building, progress parsing.
// Runtime-agnostic (node: APIs) so the Deno desktop server shares this file.
import { resolveTool, runCapture, join } from "./platform.ts";
import { loadSettings } from "./settings.ts";
import type { ProbeFormat, ProbeResult, ProbeEntry } from "./types.ts";

/** Resolve the yt-dlp binary (settings override > candidate dirs). */
export function ytdlpBin(): string | null {
  const s = loadSettings();
  return resolveTool("yt-dlp", s.toolPaths.ytdlp);
}

function commonFlags(cookiesFile?: string): string[] {
  const flags = ["--no-warnings", "--no-color", "--no-progress"];
  if (cookiesFile) flags.push("--cookies", cookiesFile);
  return flags;
}

function fmtSize(f: ProbeFormat): number | undefined {
  return f.filesize ?? f.filesizeApprox;
}

/** Map raw yt-dlp format objects into our compact shape. */
function mapFormats(raw: unknown[]): ProbeFormat[] {
  const out: ProbeFormat[] = [];
  for (const r of raw) {
    const f = r as Record<string, unknown>;
    if (!f || typeof f !== "object") continue;
    const formatId = String(f.format_id ?? "");
    if (!formatId) continue;
    const vcodec = (f.vcodec as string) ?? "none";
    const acodec = (f.acodec as string) ?? "none";
    const isStoryboard = String(f.format_note ?? "").includes("storyboard") || formatId.startsWith("sb");
    if (isStoryboard) continue;
    out.push({
      formatId,
      ext: String(f.ext ?? ""),
      resolution: String(f.resolution ?? (f.height ? `${f.height}p` : "audio")),
      fps: typeof f.fps === "number" ? f.fps : undefined,
      vcodec: vcodec === "none" ? undefined : vcodec,
      acodec: acodec === "none" ? undefined : acodec,
      filesize: typeof f.filesize === "number" ? f.filesize : undefined,
      filesizeApprox: typeof f.filesize_approx === "number" ? f.filesize_approx : undefined,
      tbr: typeof f.tbr === "number" ? f.tbr : undefined,
      note: (f.format_note as string) ?? undefined,
    });
  }
  // best first: video by height then tbr, audio by tbr
  out.sort((a, b) => {
    const ah = parseInt(a.resolution) || 0;
    const bh = parseInt(b.resolution) || 0;
    if (ah !== bh) return bh - ah;
    return (b.tbr ?? fmtSize(b) ?? 0) - (a.tbr ?? fmtSize(a) ?? 0);
  });
  return out.slice(0, 60); // keep payload sane
}

/** Probe a URL (video or playlist) with yt-dlp -J --flat-playlist. */
export async function probeUrl(url: string): Promise<{ ok: false; error: string } | { ok: true; result: ProbeResult }> {
  const bin = ytdlpBin();
  if (!bin) return { ok: false, error: "yt-dlp binary not found. Install it (pip install yt-dlp) or set the path in Settings." };
  const s = loadSettings();
  const res = await runCapture(bin, ["-J", "--flat-playlist", ...commonFlags(s.cookiesFile), url], 90_000);
  if (!res.stdout.trim()) {
    const msg = (res.stderr || res.error || "yt-dlp returned no data").trim().split("\n").filter(Boolean).slice(-3).join(" | ");
    return { ok: false, error: humanizeError(msg) };
  }
  let raw: Record<string, unknown> | null;
  try {
    raw = JSON.parse(res.stdout) as Record<string, unknown> | null;
  } catch {
    return { ok: false, error: "Could not parse yt-dlp output (unexpected extractor response)." };
  }
  // yt-dlp -J prints "null" (rc 0!) when extraction fails — treat as error
  if (!raw || typeof raw !== "object") {
    const msg = (res.stderr || res.error || "no media found at this URL").trim().split("\n").filter(Boolean).slice(-3).join(" | ");
    return { ok: false, error: humanizeError(msg) };
  }
  if (raw._type === "playlist") {
    const entriesRaw = Array.isArray(raw.entries) ? (raw.entries as Record<string, unknown>[]) : [];
    const entries: ProbeEntry[] = entriesRaw.slice(0, 500).map((e, i) => ({
      id: String(e.id ?? i),
      title: String(e.title ?? `Entry ${i + 1}`),
      duration: typeof e.duration === "number" ? e.duration : undefined,
      url: e.url ? String(e.url) : e.webpage_url ? String(e.webpage_url) : undefined,
      thumbnail: bestThumb(e),
    }));
    return {
      ok: true,
      result: {
        type: "playlist",
        id: String(raw.id ?? ""),
        title: String(raw.title ?? "Playlist"),
        uploader: raw.uploader ? String(raw.uploader) : undefined,
        thumbnail: bestThumb(raw),
        webpageUrl: String(raw.webpage_url ?? url),
        formats: [],
        subtitles: [],
        automaticCaptions: [],
        entries,
        playlistCount: typeof raw.playlist_count === "number" ? raw.playlist_count : entries.length,
        extractor: raw.extractor_key ? String(raw.extractor_key) : undefined,
      },
    };
  }
  const subs = raw.subtitles && typeof raw.subtitles === "object" ? Object.keys(raw.subtitles as Record<string, unknown>) : [];
  const auto = raw.automatic_captions && typeof raw.automatic_captions === "object"
    ? Object.keys(raw.automatic_captions as Record<string, unknown>)
    : [];
  return {
    ok: true,
    result: {
      type: "video",
      id: String(raw.id ?? ""),
      title: String(raw.title ?? "Video"),
      uploader: raw.uploader ? String(raw.uploader) : raw.channel ? String(raw.channel) : undefined,
      duration: typeof raw.duration === "number" ? raw.duration : undefined,
      viewCount: typeof raw.view_count === "number" ? raw.view_count : undefined,
      uploadDate: typeof raw.upload_date === "string" ? raw.upload_date : undefined,
      thumbnail: bestThumb(raw),
      webpageUrl: String(raw.webpage_url ?? url),
      description: typeof raw.description === "string" ? String(raw.description).slice(0, 400) : undefined,
      formats: mapFormats(Array.isArray(raw.formats) ? (raw.formats as unknown[]) : []),
      subtitles: subs,
      automaticCaptions: auto.slice(0, 30),
      entries: [],
      extractor: raw.extractor_key ? String(raw.extractor_key) : undefined,
    },
  };
}

function bestThumb(o: Record<string, unknown>): string | undefined {
  if (typeof o.thumbnail === "string" && o.thumbnail) return o.thumbnail;
  const thumbs = o.thumbnails;
  if (Array.isArray(thumbs) && thumbs.length) {
    const last = thumbs[thumbs.length - 1] as Record<string, unknown>;
    if (last && typeof last.url === "string") return last.url;
  }
  return undefined;
}

export function humanizeError(msg: string): string {
  const m = msg.toLowerCase();
  if (m.includes("sign in to confirm")) return "YouTube asked for sign-in (bot check from this IP). Set a cookies file in Settings or try another network.";
  if (m.includes("video unavailable")) return "Video unavailable (removed, private, or region-locked).";
  if (m.includes("not a valid url") || m.includes("unsupported url")) return "Unsupported URL — paste a direct video/playlist link.";
  if (m.includes("enoent") || m.includes("not found")) return "Tool not found — check Settings → Tools.";
  return msg || "Unknown engine error.";
}

// ---------------------------------------------------------------------------
// Download command construction
// ---------------------------------------------------------------------------

export interface BuiltCommand {
  args: string[];
}

export function buildDownloadArgs(opts: {
  url: string;
  kind: "video" | "audio";
  quality: number;
  container: "mp4" | "mkv" | "webm";
  audioFormat: "mp3" | "m4a" | "opus" | "flac" | "wav";
  audioQuality: string;
  embedThumbnail: boolean;
  embedMetadata: boolean;
  embedSubs: boolean;
  writeSubs: boolean;
  subLangs: string;
  useAria2: boolean;
  aria2Connections: number;
  downloadDir: string;
  filenameTemplate: string;
  cookiesFile?: string;
}): BuiltCommand {
  const args: string[] = ["--no-warnings", "--no-color", "--quiet", "--progress", "--newline"];
  if (opts.cookiesFile) args.push("--cookies", opts.cookiesFile);

  // stable, parseable progress lines
  args.push(
    "--progress-template",
    "download:__TFPROG__|%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s|%(progress.downloaded_bytes)s|%(progress.total_bytes_estimate)s|%(progress.total_bytes)s",
  );
  // final filepath after post-processing
  args.push("--print", "after_move:__TFDONE__|%(filepath)s");
  // keep the console clean of per-fragment noise
  args.push("--concurrent-fragments", String(Math.min(8, Math.max(1, opts.aria2Connections))));

  if (opts.kind === "audio") {
    args.push("-x", "--audio-format", opts.audioFormat);
    if (opts.audioQuality !== "0") args.push("--audio-quality", `${opts.audioQuality}K`);
  } else {
    const h = opts.quality > 0 ? `[height<=${opts.quality}]` : "";
    const prefer = opts.container === "mp4" ? "[vcodec^=avc1]" : "";
    const aprefer = opts.container === "mp4" ? "[acodec^=mp4a]" : "";
    args.push(
      "-f",
      `bv*${h}${prefer}+ba${aprefer}/bv*${h}+ba/b${h}/b`,
      "--merge-output-format",
      opts.container,
    );
  }

  if (opts.embedThumbnail) args.push("--embed-thumbnail");
  if (opts.embedMetadata) args.push("--embed-metadata");
  if (opts.embedSubs) args.push("--embed-subs", "--sub-langs", opts.subLangs || "en.*", "--sub-format", "srt/vtt/best");
  if (opts.writeSubs && !opts.embedSubs) args.push("--write-subs", "--sub-langs", opts.subLangs || "en.*");

  if (opts.useAria2) {
    args.push(
      "--downloader", "aria2c",
      "--downloader-args",
      `aria2c:-x ${opts.aria2Connections} -s ${opts.aria2Connections} -k 1M --file-allocation=none --console-log-level=warn --summary-interval=0`,
    );
  }

  args.push(
    "-o", join(opts.downloadDir, opts.filenameTemplate),
    "--no-mtime",
    "--no-restrict-filenames",
    opts.url,
  );
  return { args };
}

// ---------------------------------------------------------------------------
// Progress line parsing
// ---------------------------------------------------------------------------

export interface ProgressUpdate {
  percent: number;
  speed: string;
  eta: string;
  downloadedBytes: number;
  totalBytes: number;
}

export function parseProgressLine(line: string): ProgressUpdate | null {
  if (!line.startsWith("__TFPROG__|")) return null;
  const [, pct, speed, eta, dl, tEst, t] = line.split("|");
  const percent = parseFloat((pct ?? "0").replace("%", "").trim()) || 0;
  const total = Number(t) || Number(tEst) || 0;
  return {
    percent: Math.min(100, Math.max(0, percent)),
    speed: (speed ?? "").trim() || "—",
    eta: (eta ?? "").trim() === "Unknown" ? "—" : (eta ?? "—").trim(),
    downloadedBytes: Number(dl) || 0,
    totalBytes: total,
  };
}

export const DONE_PREFIX = "__TFDONE__|";
