#!/usr/bin/env node
// ============================================================
// ytdl.mjs — lightweight video-download orchestrator, node edition
// Same CLI contract as operon/ytdl.op, python/, rust/ ports.
//   node ytdl.mjs doctor | info <url> | get <url> [options] |
//        meta <file> | queue <jobs.txt> [--workers N] |
//        startup | spawnbench N | cpu N
// Engines from PATH: yt-dlp, ffmpeg, ffprobe (aria2c optional).
// Concurrency: worker-pool over child_process.execFile promises.
// ============================================================
import { execFile } from "node:child_process";
import { openSync, readFileSync } from "node:fs";
import { promisify } from "node:util";

const run = promisify(execFile);
const OUT_DEFAULT = "downloads";
const CLIP = 60;
const TOOLS = [
  ["yt-dlp", "--version"],
  ["ffmpeg", "-version"],
  ["ffprobe", "-version"],
  ["aria2c", "--version"],
];

const clip = (s, n = CLIP) => String(s).trim().slice(0, n);

async function probe(tool, flag) {
  try {
    const { stdout } = await run(tool, [flag], { timeout: 10000 });
    return stdout.trim();
  } catch {
    return "";
  }
}

async function cmdDoctor() {
  let found = 0;
  for (const [tool, flag] of TOOLS) {
    const v = await probe(tool, flag);
    if (v) {
      found++;
      console.log(`tool=${tool} version=${clip(v)}`);
    } else {
      console.log(`tool=${tool} MISSING`);
    }
  }
  console.log(`doctor: ok ${found}/${TOOLS.length}`);
  return found < 3 ? 1 : 0;
}

async function cmdInfo(url) {
  const t0 = performance.now();
  let out;
  try {
    out = await run("yt-dlp", ["-J", "--no-warnings", url], {
      timeout: 300000,
      maxBuffer: 64 * 1024 * 1024,
    });
  } catch (e) {
    console.log(`info: FAIL code=${e.code ?? "?"} err=${clip(e.stderr ?? e.message, 160)}`);
    return 1;
  }
  const j = JSON.parse(out.stdout);
  const fmts = j.formats ?? [];
  let best = "";
  for (const f of fmts) if ("format_id" in f) best = String(f.format_id);
  console.log(`title=${j.title ?? ""}`);
  console.log(`formats=${fmts.length}`);
  console.log(`best=${best}`);
  console.log(`info: ok secs=${((performance.now() - t0) / 1000).toFixed(3)}`);
  return 0;
}

function buildArgs(url, mode, fmt, out, resume) {
  const args = [];
  if (resume) args.push("-c");
  args.push("--no-warnings");
  if (mode === "audio") args.push("-x", "--audio-format", "mp3");
  else if (mode === "subs") args.push("--write-subs", "--skip-download", "--sub-langs", "en");
  else args.push("-f", fmt);
  args.push("-o", `${out}/%(title)s.%(ext)s`, url);
  return args;
}

async function doJob(idx, url, mode, fmt = "best", out = OUT_DEFAULT, resume = false) {
  const t0 = performance.now();
  console.log(`[get] start url=${url} mode=${mode} fmt=${fmt}`);
  try {
    await run("yt-dlp", buildArgs(url, mode, fmt, out, resume), {
      timeout: 300000,
      maxBuffer: 64 * 1024 * 1024,
    });
    return [1, `job ${idx}: ${mode} ${url} -> ok secs=${((performance.now() - t0) / 1000).toFixed(3)}`];
  } catch (e) {
    return [0,
      `job ${idx}: ${mode} ${url} -> FAIL(code=${e.code ?? "?"}) secs=${((performance.now() - t0) / 1000).toFixed(3)}`];
  }
}

async function cmdGet(url, fmt, out, mode, resume) {
  const [ok] = await doJob(0, url, mode, fmt, out, resume);
  if (!ok) return 1;
  console.log("get: ok");
  return 0;
}

async function cmdMeta(path) {
  let out;
  try {
    out = await run("ffprobe",
      ["-v", "quiet", "-print_format", "json", "-show_format", path],
      { timeout: 60000, maxBuffer: 16 * 1024 * 1024 });
  } catch (e) {
    console.log(`meta: FAIL code=${e.code ?? "?"} err=${clip(e.stderr ?? e.message, 160)}`);
    return 1;
  }
  const f = JSON.parse(out.stdout).format ?? {};
  console.log(`duration=${f.duration ?? "?"} size=${f.size ?? "?"}`);
  console.log("meta: ok");
  return 0;
}

function loadJobs(path) {
  const jobs = [];
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const parts = line.trim().split(" ");
    if (parts.length >= 2) jobs.push([jobs.length, parts[0], parts[1]]);
  }
  return jobs;
}

async function cmdQueue(jobsfile, workers) {
  const t0 = performance.now();
  const jobs = loadJobs(jobsfile);
  const total = jobs.length;
  if (total === 0) {
    console.log("queue: no jobs");
    return 2;
  }
  const w = Math.min(workers, total);
  // worker pool over the shared job list (event-loop concurrency;
  // each worker pulls the next index — no chunk partitioning needed)
  let next = 0;
  let oks = 0;
  const lines = [];
  async function worker() {
    while (next < total) {
      const [idx, url, mode] = jobs[next++];
      const [ok, line] = await doJob(idx, url, mode);
      oks += ok;
      lines.push(line);
    }
  }
  await Promise.all(Array.from({ length: w }, worker));
  for (const line of lines) console.log(line);
  const wall = ((performance.now() - t0) / 1000).toFixed(3);
  console.log(`queue: ${oks}/${total} ok in ${wall}s (workers=${w})`);
  return oks < total ? 1 : 0;
}

function fib(n) {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
}

async function cmdSpawnbench(n) {
  const t0 = performance.now();
  for (let i = 0; i < n; i++) {
    try {
      await run("yt-dlp", ["--version"], { timeout: 30000 });
    } catch {
      console.log(`spawnbench: FAIL at ${i}`);
      return 1;
    }
  }
  console.log(`spawnbench: ok n=${n} secs=${((performance.now() - t0) / 1000).toFixed(3)}`);
  return 0;
}

async function cmdCpu(n) {
  const t0 = performance.now();
  const v = fib(n);
  console.log(`cpu: ok fib(${n})=${v} secs=${((performance.now() - t0) / 1000).toFixed(3)}`);
  return 0;
}

function flagOf(a, name, dflt) {
  const i = a.indexOf(name);
  return i >= 0 && i + 1 < a.length ? a[i + 1] : dflt;
}

function usage() {
  console.log("usage: ytdl doctor | info <url> | get <url> [options] | " +
    "meta <file> | queue <jobs> [--workers N] | startup | spawnbench N | cpu N");
  return 2;
}

async function main() {
  const a = process.argv.slice(2);
  if (a.length < 1) return usage();
  const [cmd] = a;
  switch (cmd) {
    case "doctor": return cmdDoctor();
    case "info": return a.length >= 2 ? cmdInfo(a[1]) : usage();
    case "get":
      if (a.length < 2) return usage();
      return cmdGet(a[1],
        flagOf(a, "--format", "bv*+ba/b"),
        flagOf(a, "--out", OUT_DEFAULT),
        a.includes("--audio") ? "audio" : a.includes("--subs") ? "subs" : "video",
        a.includes("--resume"));
    case "meta": return a.length >= 2 ? cmdMeta(a[1]) : usage();
    case "queue": return a.length >= 2 ? cmdQueue(a[1], parseInt(flagOf(a, "--workers", "4"), 10)) : usage();
    case "startup": console.log("startup: ok"); return 0;
    case "spawnbench": return a.length >= 2 ? cmdSpawnbench(parseInt(a[1], 10)) : usage();
    case "cpu": return a.length >= 2 ? cmdCpu(parseInt(a[1], 10)) : usage();
    default:
      console.log(`ytdl: unknown command '${cmd}'`);
      return 2;
  }
}

main().then((code) => process.exit(code));
