// TubeForge — shared types. Runtime-agnostic: imported by both the Next.js
// API layer (Node) and the Deno desktop server (`desktop/deno-server.ts`).
// Only standard TS types here — no node:/Deno-specific imports.

export type JobStatus =
  | "queued"
  | "downloading"
  | "processing"
  | "completed"
  | "error"
  | "canceled";

export interface JobProgress {
  percent: number; // 0..100
  speed: string; // human-readable, e.g. "4.2MiB/s"
  eta: string; // human-readable, e.g. "00:12"
  downloadedBytes: number;
  totalBytes: number;
}

export interface JobOptions {
  url: string;
  kind: "video" | "audio";
  quality: number; // max height in px; 0 = best
  container: "mp4" | "mkv" | "webm";
  audioFormat: "mp3" | "m4a" | "opus" | "flac" | "wav";
  audioQuality: "320" | "256" | "192" | "128" | "0"; // kbps; 0 = best/native
  embedThumbnail: boolean;
  embedMetadata: boolean;
  embedSubs: boolean;
  writeSubs: boolean;
  subLangs: string;
  useAria2: boolean;
  aria2Connections: number; // 1..16
  title?: string; // display title captured at enqueue time
  thumbnail?: string; // display thumbnail url
}

export interface Job {
  id: string;
  createdAt: number;
  updatedAt: number;
  status: JobStatus;
  opts: JobOptions;
  progress: JobProgress;
  filePath?: string;
  fileName?: string;
  fileSize?: number;
  error?: string;
  logTail: string[]; // last N raw engine lines for the details drawer
}

export interface ProbeFormat {
  formatId: string;
  ext: string;
  resolution: string;
  fps?: number;
  vcodec?: string;
  acodec?: string;
  filesize?: number;
  filesizeApprox?: number;
  tbr?: number;
  note?: string;
}

export interface ProbeEntry {
  id: string;
  title: string;
  duration?: number;
  url?: string;
  thumbnail?: string;
}

export interface ProbeResult {
  type: "video" | "playlist";
  id?: string;
  title: string;
  uploader?: string;
  duration?: number;
  viewCount?: number;
  uploadDate?: string;
  thumbnail?: string;
  webpageUrl?: string;
  description?: string;
  formats: ProbeFormat[];
  subtitles: string[]; // language codes with subs available
  automaticCaptions: string[];
  entries: ProbeEntry[]; // playlist only
  playlistCount?: number;
  extractor?: string;
}

export interface Settings {
  downloadDir: string;
  filenameTemplate: string;
  concurrentDownloads: number; // 1..4
  useAria2: boolean;
  aria2Connections: number;
  defaultQuality: number;
  defaultContainer: JobOptions["container"];
  defaultAudioFormat: JobOptions["audioFormat"];
  embedThumbnail: boolean;
  embedMetadata: boolean;
  embedSubs: boolean;
  writeSubs: boolean;
  subLangs: string;
  cookiesFile?: string;
  toolPaths: {
    ytdlp?: string;
    ffmpeg?: string;
    aria2c?: string;
  };
}

export interface ToolInfo {
  name: "yt-dlp" | "ffmpeg" | "aria2c" | "deno";
  path: string | null;
  version: string | null;
  ok: boolean;
}

export interface HealthReport {
  tools: ToolInfo[];
  downloadDir: string;
  downloadDirWritable: boolean;
  platform: string;
  runtime: string;
}

export interface QueuedSnapshot {
  jobs: Job[];
  activeCount: number;
  version: number;
}
