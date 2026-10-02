// TubeForge — request validation (server side).
import { loadSettings } from "./settings.ts";
import type { JobOptions } from "./types.ts";

export function isSafeUrl(url: unknown): url is string {
  if (typeof url !== "string" || url.length > 2048) return false;
  try {
    const u = new URL(url);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

const QUALITIES = [0, 360, 480, 720, 1080, 1440, 2160];
const CONTAINERS = ["mp4", "mkv", "webm"] as const;
const AUDIO_FORMATS = ["mp3", "m4a", "opus", "flac", "wav"] as const;
const AUDIO_QUALITIES = ["320", "256", "192", "128", "0"] as const;

/** Build a fully-typed JobOptions from an untrusted JSON body. */
export function parseJobOptions(body: unknown): { ok: false; error: string } | { ok: true; opts: JobOptions } {
  if (!body || typeof body !== "object") return { ok: false, error: "Body must be a JSON object." };
  const b = body as Record<string, unknown>;
  if (!isSafeUrl(b.url)) return { ok: false, error: "A valid http(s) URL is required." };
  const s = loadSettings();
  const kind = b.kind === "audio" ? "audio" : "video";
  const quality = QUALITIES.includes(Number(b.quality)) ? Number(b.quality) : s.defaultQuality;
  const container = CONTAINERS.includes(b.container as (typeof CONTAINERS)[number])
    ? (b.container as JobOptions["container"])
    : s.defaultContainer;
  const audioFormat = AUDIO_FORMATS.includes(b.audioFormat as (typeof AUDIO_FORMATS)[number])
    ? (b.audioFormat as JobOptions["audioFormat"])
    : s.defaultAudioFormat;
  const audioQuality = AUDIO_QUALITIES.includes(String(b.audioQuality) as (typeof AUDIO_QUALITIES)[number])
    ? String(b.audioQuality) as JobOptions["audioQuality"]
    : "320";
  const subLangs = typeof b.subLangs === "string" && b.subLangs.trim() ? b.subLangs.trim().slice(0, 200) : s.subLangs;
  return {
    ok: true,
    opts: {
      url: b.url,
      kind,
      quality,
      container,
      audioFormat,
      audioQuality,
      embedThumbnail: b.embedThumbnail !== undefined ? !!b.embedThumbnail : s.embedThumbnail,
      embedMetadata: b.embedMetadata !== undefined ? !!b.embedMetadata : s.embedMetadata,
      embedSubs: b.embedSubs !== undefined ? !!b.embedSubs : s.embedSubs,
      writeSubs: b.writeSubs !== undefined ? !!b.writeSubs : s.writeSubs,
      subLangs,
      useAria2: b.useAria2 !== undefined ? !!b.useAria2 : s.useAria2,
      aria2Connections: Math.min(16, Math.max(1, Number(b.aria2Connections) || s.aria2Connections)),
      title: typeof b.title === "string" ? b.title.slice(0, 300) : undefined,
      thumbnail: typeof b.thumbnail === "string" ? b.thumbnail.slice(0, 1000) : undefined,
    },
  };
}
