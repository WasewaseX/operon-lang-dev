// TubeForge — health: tool resolution + versions, dir writability.
import { resolveTool, runCapture, isWritableDir, platform, arch } from "./platform.ts";
import { loadSettings } from "./settings.ts";
import type { HealthReport, ToolInfo } from "./types.ts";
import proc from "node:process";

async function toolVersion(cmd: string | null, args: string[]): Promise<string | null> {
  if (!cmd) return null;
  const r = await runCapture(cmd, args, 15_000);
  const line = (r.stdout || r.stderr).split(/\r?\n/).find(Boolean);
  return line ? line.trim().slice(0, 80) : null;
}

export async function healthReport(runtime = "node"): Promise<HealthReport> {
  const s = loadSettings();
  const ytdlp = resolveTool("yt-dlp", s.toolPaths.ytdlp);
  const ffmpeg = resolveTool("ffmpeg", s.toolPaths.ffmpeg);
  const aria2c = resolveTool("aria2c", s.toolPaths.aria2c);
  const deno = resolveTool("deno");
  const tools: ToolInfo[] = [
    { name: "yt-dlp", path: ytdlp, version: await toolVersion(ytdlp, ["--version"]), ok: !!ytdlp },
    { name: "ffmpeg", path: ffmpeg, version: await toolVersion(ffmpeg, ["-version"]), ok: !!ffmpeg },
    { name: "aria2c", path: aria2c, version: await toolVersion(aria2c, ["--version"]), ok: !!aria2c },
    { name: "deno", path: deno, version: await toolVersion(deno, ["--version"]), ok: !!deno },
  ];
  return {
    tools,
    downloadDir: s.downloadDir,
    downloadDirWritable: isWritableDir(s.downloadDir),
    platform: `${platform()} ${arch()}`,
    runtime,
  };
}

export const runtimeName = () => `node ${proc.versions?.node ?? "?"}`;
