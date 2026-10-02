// TubeForge Lite — headless CLI mode.
//   tubeforge-lite <url> [urls…] [--audio mp3|m4a|opus|flac|wav] [--quality N]
//     [--container mp4|mkv|webm] [--out DIR] [--subs "en,de"] [--embed-subs]
//     [--no-embed-thumb] [--no-embed-meta] [--no-aria2] [--aria2-conn N]
//     [--concurrency N] [--cookies FILE] [--quiet]
// Reuses the SAME engine as the server: queue, persistence, progress parsing.
import proc from "node:process";
import { loadSettings, saveSettings } from "../shared/settings.ts";
import { enqueue, snapshot, cancelJob } from "../shared/engine.ts";
import { parseJobOptions, isSafeUrl } from "../shared/validate.ts";
import { expandHome } from "../shared/platform.ts";
import { gateEnqueue, installPolicySizeGate, loadPolicy } from "./policy.ts";
import type { Job, JobOptions } from "../shared/types.ts";

interface CliOpts {
  kind?: "video" | "audio";
  quality?: number;
  container?: "mp4" | "mkv" | "webm";
  audioFormat?: "mp3" | "m4a" | "opus" | "flac" | "wav";
  out?: string;
  subs?: string;
  embedSubs?: boolean;
  embedThumb?: boolean;
  embedMeta?: boolean;
  useAria2?: boolean;
  aria2Conn?: number;
  concurrency?: number;
  cookies?: string;
  quiet?: boolean;
}

function parseCli(args: string[]): { urls: string[]; o: CliOpts } {
  const urls: string[] = [];
  const o: CliOpts = {};
  const val = (i: number): string | undefined => (args[i + 1] && !args[i + 1].startsWith("--") ? args[i + 1] : undefined);
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    const v = val(i);
    switch (a) {
      case "--audio": o.kind = "audio"; if (v) { o.audioFormat = v as CliOpts["audioFormat"]; i++; } break;
      case "--quality": if (v) { o.quality = Number(v); i++; } break;
      case "--container": if (v) { o.container = v as CliOpts["container"]; i++; } break;
      case "--out": if (v) { o.out = v; i++; } break;
      case "--subs": if (v) { o.subs = v; i++; } break;
      case "--embed-subs": o.embedSubs = true; break;
      case "--no-embed-thumb": o.embedThumb = false; break;
      case "--no-embed-meta": o.embedMeta = false; break;
      case "--no-aria2": o.useAria2 = false; break;
      case "--aria2-conn": if (v) { o.aria2Conn = Number(v); i++; } break;
      case "--concurrency": if (v) { o.concurrency = Number(v); i++; } break;
      case "--cookies": if (v) { o.cookies = v; i++; } break;
      case "--quiet": case "-q": o.quiet = true; break;
      default:
        if (!a.startsWith("--") && isSafeUrl(a)) urls.push(a);
    }
  }
  return { urls, o };
}

function active(j: Job): boolean {
  return j.status === "queued" || j.status === "downloading" || j.status === "processing";
}

function line(j: Job): string {
  const p = j.progress;
  const t = (j.opts.title ?? j.opts.url).slice(0, 52);
  const tag = j.status === "processing" ? "processing" : `${Math.round(p.percent)}%`;
  return `${tag.padEnd(11)} ${p.speed.padEnd(10)} ETA ${p.eta.padEnd(8)} ${t}`;
}

export async function runCli(args: string[], version: string): Promise<number> {
  const { urls, o } = parseCli(args);
  if (urls.length === 0) {
    console.log(`tubeforge-lite ${version} — headless download mode
Usage: tubeforge-lite <url> [urls…] [options]
  --audio mp3|m4a|opus|flac|wav   audio extraction
  --quality 2160|1440|1080|720|480|360|0     (0 = best)
  --container mp4|mkv|webm        merge container (video)
  --out DIR                       download directory
  --subs "en,de" [--embed-subs]   subtitles
  --no-aria2 / --aria2-conn N     aria2 control
  --concurrency N                 parallel downloads (1..4)
  --cookies FILE                  cookies.txt for age-gated content
  --quiet                         progress only
  (policy: TUBEFORGE_POLICY + TF_OPERON env apply here too)`);
    return 2;
  }

  // Operon policy lane: arm the reactive size gate + warm the policy cache
  // (both no-ops while TUBEFORGE_POLICY is unset).
  installPolicySizeGate();
  loadPolicy();

  const s = loadSettings();
  if (o.concurrency) s.concurrentDownloads = Math.min(4, Math.max(1, o.concurrency));
  if (o.cookies) s.cookiesFile = expandHome(o.cookies);
  if (o.out) s.downloadDir = expandHome(o.out);
  saveSettings(s);

  const jobs: Job[] = [];
  for (const url of urls) {
    const body: Record<string, unknown> = { url };
    if (o.kind) body.kind = o.kind;
    if (o.quality !== undefined) body.quality = o.quality;
    if (o.container) body.container = o.container;
    if (o.audioFormat) body.audioFormat = o.audioFormat;
    if (o.subs) { body.subLangs = o.subs; body.writeSubs = true; }
    if (o.embedSubs !== undefined) { body.embedSubs = o.embedSubs; }
    if (o.embedThumb !== undefined) body.embedThumbnail = o.embedThumb;
    if (o.embedMeta !== undefined) body.embedMetadata = o.embedMeta;
    if (o.useAria2 !== undefined) body.useAria2 = o.useAria2;
    if (o.aria2Conn) body.aria2Connections = o.aria2Conn;
    const parsed = parseJobOptions(body);
    if (!parsed.ok) {
      console.error(`  ✗ ${url}: ${(parsed as { error: string }).error}`);
      continue;
    }
    const gate = gateEnqueue((parsed as { opts: JobOptions }).opts, [url]);
    if (!gate.ok) {
      console.error(`  ✗ ${url}: ${gate.error}`);
      continue;
    }
    const job = enqueue(gate.opts);
    jobs.push({ ...job });
    if (!o.quiet) console.log(`  + queued [${jobs.length}/${urls.length}] ${url}`);
  }
  if (jobs.length === 0) return 2;

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  let last = "";
  for (;;) {
    await sleep(400);
    const snap = snapshot();
    const act = snap.jobs.filter((j) => j.id && jobs.some((x) => x.id === j.id) && active(j));
    const first = act[0];
    if (first && !o.quiet) {
      const l = line(first);
      proc.stdout.write("\r" + l.padEnd(last.length) );
      last = l;
    } else if (first && o.quiet) {
      proc.stdout.write(`\r${Math.round(first.progress.percent)}%`);
      last = `${first.progress.percent}%`;
    }
    if (act.length === 0) {
      proc.stdout.write("\r".padEnd(last.length + 2) + "\r");
      break;
    }
  }

  let failed = 0;
  const snap = snapshot();
  const byId = new Map(snap.jobs.map((j) => [j.id, j]));
  if (!o.quiet) console.log("");
  for (const x of jobs) {
    const j = byId.get(x.id);
    if (!j) { failed++; continue; }
    if (j.status === "completed") {
      console.log(`  ✓ ${j.fileName ?? j.opts.url}  (${fmt(j.fileSize ?? 0)}) → ${j.filePath ?? "?"}`);
    } else if (j.status === "canceled") {
      console.log(`  ✗ canceled ${j.opts.url}`);
      failed++;
    } else {
      console.error(`  ✗ ${j.status.toUpperCase()} ${j.opts.url}: ${j.error ?? "unknown error"}`);
      failed++;
      cancelJob(j.id);
    }
  }
  return failed === 0 ? 0 : 1;
}

function fmt(n: number): string {
  if (!n) return "0 B";
  const u = ["B", "KiB", "MiB", "GiB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${u[i]}`;
}
