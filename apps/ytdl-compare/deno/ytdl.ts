#!/usr/bin/env -S deno run --allow-run --allow-read --allow-write
/**
 * ytdl.ts — YouTube downloader orchestrator, Deno/TypeScript comparison build.
 *
 * Same subcommands, same option surface, same decision logic as the Operon
 * edition (apps/ytdl/ytdl.op). `selfcheck` prints the decision matrix that
 * must be byte-identical across all three builds — the differential pin.
 *
 * Engines: yt-dlp + ffmpeg from PATH (aria2c optional). Zero npm deps.
 *
 * run: deno run --allow-run --allow-read --allow-write ytdl.ts <cmd> ...
 */

const TIERS = ["2160", "1440", "1080", "720", "480", "360", "240", "144"];
const AUDIO_KINDS = ["mp3", "m4a", "opus", "flac", "wav"];
const PARTIAL_SUFFIXES = [".part", ".ytdl", ".aria2", ".tmp"];

interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}

interface Opts {
  url: string;
  out: string;
  quality: string;
  audio: string;
  fmt: string;
  subs: boolean;
  sub_langs: string;
  jobs: number;
  aria2: boolean;
  playlist: boolean;
  template: string;
  max_attempts: number;
  aria2_ok: boolean;
}

interface Fmt {
  format_id: string;
  ext: string;
  height?: number;
  width?: number;
  fps?: number;
  vcodec?: string;
  acodec?: string;
  tbr?: number;
  filesize?: number;
  filesize_approx?: number;
  format_note?: string;
}

interface Meta {
  id?: string;
  title?: string;
  uploader?: string;
  duration?: number;
  view_count?: number;
  formats?: Fmt[];
}

// ---------------------------------------------------------- formatting

function fmtBytes(n: number | undefined | null): string {
  if (n == null) return "?";
  const f = Number(n);
  if (f >= 1073741824.0) return `${(f / 1073741824.0).toFixed(2)} GB`;
  if (f >= 1048576.0) return `${(f / 1048576.0).toFixed(1)} MB`;
  if (f >= 1024.0) return `${(f / 1024.0).toFixed(1)} KB`;
  return `${n} B`;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}

function fmtDur(s: number | undefined | null): string {
  if (s == null) return "?";
  const t = Math.floor(Number(s));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const sec = t % 60;
  if (h > 0) return `${h}:${pad2(m)}:${pad2(sec)}`;
  return `${m}:${pad2(sec)}`;
}

function comma(n: number | undefined | null): string {
  if (n == null) return "?";
  return Math.floor(Number(n)).toLocaleString("en-US");
}

function mget(m: Record<string, unknown>, k: string): string {
  const v = m[k];
  return v == null ? "" : String(v);
}

function tailLines(s: string, n: number): string {
  const ls = s.trim().split("\n").filter((l) => l !== "");
  if (ls.length <= n) return ls.join("\n");
  return ls.slice(ls.length - n).join("\n");
}

// ---------------------------------------------------------- engines

async function runProg(prog: string, args: string[], timeoutMs = 300000): Promise<RunResult> {
  try {
    const cmd = new Deno.Command(prog, { args, stdout: "piped", stderr: "piped" });
    const { code, stdout, stderr } = await cmd.output();
    return {
      code,
      stdout: new TextDecoder().decode(stdout),
      stderr: new TextDecoder().decode(stderr),
    };
  } catch (_e) {
    throw new Error(`run denied — '${prog}' not found or not granted`);
  }
}

async function toolLine(prog: string, flag: string): Promise<string> {
  try {
    const r = await runProg(prog, [flag], 30000);
    if (r.code === 0) return r.stdout.trim().split("\n")[0];
  } catch (_e) {
    /* not granted / not installed */
  }
  return "";
}

async function detectTools() {
  return {
    ytdlp: await toolLine("yt-dlp", "--version"),
    ffmpeg: await toolLine("ffmpeg", "-version"),
    aria2: await toolLine("aria2c", "--version"),
  };
}

// ---------------------------------------------------------- selection logic
// must stay IDENTICAL to apps/ytdl/ytdl.op (differential-pinned)

function qualityExpr(q: string): string {
  if (q === "best") return "bestvideo*+bestaudio/best";
  return `bestvideo[height<=${q}]+bestaudio/best[height<=${q}]/best`;
}

function selectionArgs(o: Partial<Opts>): string[] {
  if (o.fmt) return ["-f", o.fmt];
  if (o.audio) {
    return ["-f", "bestaudio/best", "-x", "--audio-format", o.audio, "--audio-quality", "0"];
  }
  return ["-f", qualityExpr(o.quality ?? "best")];
}

function buildGetArgs(url: string, o: Opts): string[] {
  const a: string[] = [];
  if (!o.playlist) a.push("--no-playlist");
  a.push("--continue", "--retries", "3", "--fragment-retries", "3", "--no-overwrites");
  if (o.aria2 && o.aria2_ok) {
    a.push("--downloader", "aria2c", "--downloader-args", "aria2c:-x16 -s16 -k1M --file-allocation=none");
  }
  if (o.subs) {
    a.push("--write-subs", "--sub-langs", o.sub_langs, "--convert-subs", "srt");
  }
  a.push("-P", o.out, "-o", o.template);
  a.push(...selectionArgs(o));
  a.push(url);
  return a;
}

// ---------------------------------------------------------- format table

function hOf(f: Fmt): number {
  return f.height == null ? 0 : f.height;
}

function tOf(f: Fmt): number {
  return f.tbr == null ? 0 : f.tbr;
}

function fmtBefore(a: Fmt, b: Fmt): boolean {
  if (hOf(a) > hOf(b)) return true;
  if (hOf(a) < hOf(b)) return false;
  return tOf(a) > tOf(b);
}

function resOf(f: Fmt): string {
  const h = hOf(f);
  if (h <= 0) return mget(f, "vcodec") === "none" ? "audio" : "?";
  const w = f.width ?? 0;
  return w > 0 ? `${w}x${h}` : `?x${h}`;
}

function sizeOf(f: Fmt): string {
  if (f.filesize != null) return fmtBytes(f.filesize);
  if (f.filesize_approx != null) return `~${fmtBytes(f.filesize_approx)}`;
  return "?";
}

function clip(s: string, w: number): string {
  return s.length > w ? s.slice(0, w - 2) + ".." : s;
}

function fmtRow(f: Fmt): string {
  const fps = f.fps != null ? String(Math.floor(f.fps)) : "";
  let vc = mget(f, "vcodec") || "?";
  if (vc === "none") vc = "-";
  let ac = mget(f, "acodec") || "?";
  if (ac === "none") ac = "-";
  const p = (s: string, w: number, dir: "l" | "r") =>
    dir === "l" ? clip(s, w).padEnd(w) : clip(s, w).padStart(w);
  return (
    p(mget(f, "format_id"), 9, "l") + "  " +
    p(mget(f, "ext"), 4, "l") + "  " +
    p(resOf(f), 10, "l") + "  " +
    p(fps, 4, "r") + "  " +
    p(sizeOf(f), 9, "r") + "  " +
    p(`${vc}/${ac}`, 19, "l") + "  " +
    mget(f, "format_note")
  );
}

function tableRows(meta: Meta): string[] {
  // pure decision pipeline: sort + row-build, NO engine probes — the
  // workload the deep benchmark times in every language build
  const out: string[] = [];
  out.push(`title: ${mget(meta, "title")}`);
  out.push(
    `by: ${mget(meta, "uploader")}  |  duration: ${fmtDur(meta.duration)}  |  views: ${comma(meta.view_count)}`,
  );
  const n = meta.formats ? meta.formats.length : 0;
  out.push(`formats: ${n}  (res = WxH, size from yt-dlp, ~ = approximate)`);
  out.push("id        ext   res         fps      size  vcodec/acodec        note");
  const fs = (meta.formats ?? []).slice().sort((a, b) => {
    if (hOf(a) > hOf(b)) return -1;
    if (hOf(a) < hOf(b)) return 1;
    return tOf(a) > tOf(b) ? -1 : 1;
  });
  for (const f of fs) out.push(fmtRow(f));
  return out;
}

function infoLines(meta: Meta): string[] {
  const out = tableRows(meta);
  const tools = detectToolsSync();
  if (tools.aria2) {
    out.push(`accel: aria2c ${tools.aria2} (multi-connection enabled)`);
  } else {
    out.push("accel: aria2c not found — single-connection downloads");
  }
  return out;
}

function detectToolsSync() {
  // shell-out probe kept synchronous so table paths stay simple
  const line = (prog: string, flag: string): string => {
    try {
      const cmd = new Deno.Command(prog, { args: [flag], stdout: "piped", stderr: "piped" });
      const { code, stdout } = cmd.outputSync();
      if (code === 0) return new TextDecoder().decode(stdout).trim().split("\n")[0];
    } catch (_e) {
      /* not granted / not installed */
    }
    return "";
  };
  return {
    ytdlp: line("yt-dlp", "--version"),
    ffmpeg: line("ffmpeg", "-version"),
    aria2: line("aria2c", "--version"),
  };
}

// ---------------------------------------------------------- download

function hasPartial(d: string): boolean {
  try {
    for (const name of Deno.readDirSync(d)) {
      if (PARTIAL_SUFFIXES.some((s) => name.name.endsWith(s))) return true;
    }
  } catch (_e) {
    return false;
  }
  return false;
}

function destLines(stdout: string, stderr: string): string[] {
  const found: string[] = [];
  for (const src of [stdout, stderr]) {
    for (const raw of src.split("\n")) {
      const l = raw.trim();
      if (
        l.includes("Destination:") || l.includes("has already been downloaded") ||
        l.includes("[Merger]") || l.includes("Download complete:") || l.includes("ExtractAudio")
      ) {
        found.push(l);
      }
    }
  }
  return found;
}

async function download(url: string, o: Opts) {
  let attempts = 0;
  let last: RunResult = { code: -1, stdout: "", stderr: "" };
  while (true) {
    attempts += 1;
    last = await runProg("yt-dlp", buildGetArgs(url, o));
    if (last.code === 0 && !hasPartial(o.out)) break;
    if (attempts >= o.max_attempts) break;
  }
  const ok = last.code === 0 && !hasPartial(o.out);
  return { ok, attempts, r: last };
}

function report(d: { ok: boolean; attempts: number; r: RunResult }) {
  for (const ln of destLines(d.r.stdout, d.r.stderr)) console.log(`  ${ln}`);
  console.log(`  attempts: ${d.attempts}  result: ${d.ok ? "ok" : "FAIL"}`);
  if (!d.ok) {
    console.log(d.r.code !== 0 ? `  yt-dlp exited ${d.r.code}:` : `  incomplete after ${d.attempts} rounds:`);
    console.log(tailLines(d.r.stderr, 4));
  }
}

// ---------------------------------------------------------- option layer

function getOpts(rest: string[]): Opts {
  function val(name: string, def: string): string {
    const f = `--${name}`;
    for (let i = 0; i < rest.length; i++) {
      if (rest[i] === f) return i + 1 < rest.length ? rest[i + 1] : def;
      if (rest[i].startsWith(f + "=")) return rest[i].slice(f.length + 1);
    }
    return def;
  }
  const flag = (name: string) => rest.includes(`--${name}`);
  const pos: string[] = [];
  let i = 0;
  while (i < rest.length) {
    const a = rest[i];
    if (a.startsWith("--")) i += a.includes("=") ? 1 : 2;
    else if (a.startsWith("-")) i += 1;
    else {
      pos.push(a);
      i += 1;
    }
  }
  const o: Opts = {
    url: pos[0] ?? "",
    out: val("out", "downloads"),
    quality: val("quality", "best"),
    audio: val("audio", ""),
    fmt: val("format", ""),
    subs: flag("subs"),
    sub_langs: val("sub-langs", "en"),
    jobs: parseInt(val("jobs", "2")),
    aria2: !flag("no-aria2"),
    playlist: flag("playlist"),
    template: val("template", "%(title)s [%(id)s].%(ext)s"),
    max_attempts: parseInt(val("max-attempts", "4")),
    aria2_ok: false,
  };
  o.aria2_ok = false; // set async in main before get/queue use it
  return o;
}

function die(msg: string, code = 2): never {
  console.log(msg);
  Deno.exit(code);
}

// ---------------------------------------------------------- commands

async function cmdDoctor(): Promise<never> {
  const t = await detectTools();
  console.log("ytdl doctor — probing PATH engines");
  let ytdlpOk = false;
  let ffmpegOk = false;
  if (t.ytdlp) {
    ytdlpOk = true;
    console.log(`  yt-dlp   ${t.ytdlp}  [required: present]`);
  } else {
    console.log("  yt-dlp   MISSING  [required: pip install yt-dlp or brew install yt-dlp]");
  }
  if (t.ffmpeg) {
    ffmpegOk = true;
    console.log(`  ffmpeg   ${t.ffmpeg}  [required: present]`);
  } else {
    console.log("  ffmpeg   MISSING  [required: apt/brew install ffmpeg]");
  }
  if (t.aria2) {
    console.log(`  aria2c   ${t.aria2}  [optional: multi-connection downloads]`);
  } else {
    console.log("  aria2c   not found  [optional: apt/brew install aria2 for 16-connection accel]");
  }
  if (ytdlpOk && ffmpegOk) {
    console.log("verdict: READY — deno runtime + PATH engines");
    Deno.exit(0);
  }
  console.log("verdict: NOT READY — missing required engine(s)");
  Deno.exit(1);
}

async function cmdInfo(rest: string[]): Promise<never> {
  const o = getOpts(rest);
  if (!o.url) die("error: info needs a URL");
  let r: RunResult;
  try {
    r = await runProg("yt-dlp", ["-J", "--no-playlist", o.url]);
  } catch (e) {
    die(`yt-dlp denied — ${(e as Error).message}`, 1);
  }
  if (r.code !== 0) {
    console.log(`yt-dlp failed (code ${r.code}):`);
    console.log(tailLines(r.stderr, 3));
    Deno.exit(1);
  }
  const meta = JSON.parse(r.stdout) as Meta;
  for (const ln of infoLines(meta)) console.log(ln);
  Deno.exit(0);
}

async function cmdGet(rest: string[]): Promise<never> {
  const o = getOpts(rest);
  o.aria2_ok = (await toolLine("aria2c", "--version")) !== "";
  if (!o.url) die("error: get needs a URL");
  console.log(`ytdl[deno]: get ${o.url}`);
  console.log(`  out=${o.out}  selector: ${selectionArgs(o).join(" ")}`);
  Deno.mkdirSync(o.out, { recursive: true });
  const d = await download(o.url, o);
  report(d);
  Deno.exit(d.ok ? 0 : 1);
}

async function cmdQueue(rest: string[]): Promise<never> {
  if (!rest.length || rest[0].startsWith("--")) {
    die("error: queue needs a file (one URL per line, # comments)");
  }
  const file = rest[0];
  const o = getOpts(rest.slice(1));
  o.aria2_ok = (await toolLine("aria2c", "--version")) !== "";
  let text: string;
  try {
    text = Deno.readTextFileSync(file);
  } catch (e) {
    die(`error: cannot read queue file ${file} (${(e as Error).message})`);
  }
  const urls = text.split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#"));
  const jobs = Math.max(1, Math.min(o.jobs, urls.length));
  const rows: string[] = [];
  if (jobs === 1) {
    for (const u of urls) {
      const d = await download(u, o);
      rows.push(`${d.ok ? "ok" : "FAIL"}\t${d.attempts}\t${u}`);
    }
  } else {
    const chunks: string[][] = Array.from({ length: jobs }, () => []);
    urls.forEach((u, idx) => chunks[idx % jobs].push(u));
    // run chunks concurrently on the event loop (async downloads interleave)
    const results = await Promise.all(chunks.map(async (chunk) => {
      const out: string[] = [];
      for (const u of chunk) {
        const d = await download(u, o);
        out.push(`${d.ok ? "ok" : "FAIL"}\t${d.attempts}\t${u}`);
      }
      return out;
    }));
    for (const chunk of results) rows.push(...chunk);
  }
  let failures = 0;
  for (const r of rows) {
    console.log(r);
    if (!r.startsWith("ok\t")) failures += 1;
  }
  console.log(`queue: ${rows.length} items, ${failures} failed (workers: ${jobs})`);
  Deno.exit(failures > 0 ? 1 : 0);
}

async function cmdSelfcheck(rest: string[]): Promise<never> {
  if (!rest.length) die("error: selfcheck needs a fixture .json path");
  const meta = JSON.parse(Deno.readTextFileSync(rest[0])) as Meta;
  for (const q of ["best", ...TIERS]) {
    const o: Partial<Opts> = { quality: q, audio: "", fmt: "" };
    console.log(`q=${q} a=- :: ${selectionArgs(o).join(" ")}`);
  }
  for (const kind of AUDIO_KINDS) {
    const o: Partial<Opts> = { quality: "best", audio: kind, fmt: "" };
    console.log(`q=- a=${kind} :: ${selectionArgs(o).join(" ")}`);
  }
  const fs = (meta.formats ?? []).slice().sort((a, b) => {
    if (hOf(a) > hOf(b)) return -1;
    if (hOf(a) < hOf(b)) return 1;
    return tOf(a) > tOf(b) ? -1 : 1;
  });
  console.log(`sort-check: ${fs.map((f) => String(f.format_id)).join(",")}`);
  const h = infoLines(meta);
  console.log(`hdr-check: ${h[0]}`);
  console.log(`hdr-check: ${h[1]}`);
  Deno.exit(0);
}

// ---------------------------------------------------------- deep bench
// Uniform workloads for the cross-language deep benchmark
// (docs/BENCHMARK-DEEP.md, harness: scripts/bench_deep.py). Each prints
// ONE "bench-* cs=..." line; cs must be byte-identical in every build.

function benchArg(rest: string[], name: string, def: string): string {
  const f = `--${name}`;
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === f && i + 1 < rest.length) return rest[i + 1];
    if (rest[i].startsWith(f + "=")) return rest[i].slice(f.length + 1);
  }
  return def;
}

function progressLine(i: number): string {
  const pp = String((i * 7) % 100).padStart(2, "0");
  const ss = String((i * 3) % 60).padStart(2, "0");
  return `[download]  ${pp}% of 12.00MiB at 2.00MiB/s ETA 00:${ss}`;
}

function cmdBenchStartup(): never {
  console.log("ytdl-bench ready");
  Deno.exit(0);
}

function cmdBenchJson(rest: string[]): never {
  if (!rest.length) die("error: bench-json needs a fixture path");
  const n = parseInt(benchArg(rest, "n", "100"));
  const text = Deno.readTextFileSync(rest[0]);
  const meta = JSON.parse(text) as Meta;
  let nf = 0;
  let title = "";
  for (let i = 0; i < n; i++) {
    const m = JSON.parse(text) as Meta;
    nf = (m.formats ?? []).length;
    title = mget(m, "title");
  }
  if (n > 0 && nf !== (meta.formats ?? []).length) {
    console.log("bench-json MISMATCH");
    Deno.exit(1);
  }
  console.log(`bench-json cs=${title}:${nf}:${n}`);
  Deno.exit(0);
}

function cmdBenchTable(rest: string[]): never {
  if (!rest.length) die("error: bench-table needs a fixture path");
  const rounds = parseInt(benchArg(rest, "rounds", "20"));
  const meta = JSON.parse(Deno.readTextFileSync(rest[0])) as Meta;
  let first = "";
  let last = "";
  let nrows = 0;
  for (let i = 0; i < rounds; i++) {
    const rows = tableRows(meta);
    first = rows[0];
    last = rows[rows.length - 1];
    nrows = rows.length;
  }
  console.log(`bench-table cs=${nrows}|${first}|${last}`);
  Deno.exit(0);
}

function cmdBenchLines(rest: string[]): never {
  const k = parseInt(benchArg(rest, "k", "20000"));
  let eta = 0;
  const samples: string[] = [];
  const step = Math.max(1, Math.floor(k / 40));
  for (let i = 0; i < k; i++) {
    const ln = progressLine(i);
    if (ln.includes("ETA")) eta += 1;
    if (i % step === 0) samples.push(ln.slice(12, 14));
  }
  console.log(`bench-lines cs=${eta},${samples.join(",")}`);
  Deno.exit(0);
}

function cmdBenchSpawn(rest: string[]): never {
  const n = parseInt(benchArg(rest, "n", "30"));
  let ok = 0;
  for (let i = 0; i < n; i++) {
    try {
      const cmd = new Deno.Command("mockspawn", { args: ["--version"], stdout: "piped", stderr: "piped" });
      const { code } = cmd.outputSync();
      if (code === 0) ok += 1;
    } catch (_e) {
      /* missing engine — counted as failure below */
    }
  }
  console.log(`bench-spawn cs=${ok}/${n}`);
  Deno.exit(ok === n ? 0 : 1);
}

function benchQueueWorker(chunk: number[]): number {
  let ok = 0;
  for (const _u of chunk) {
    try {
      const cmd = new Deno.Command("mocksleep", { args: ["80"], stdout: "piped", stderr: "piped" });
      const { code } = cmd.outputSync();
      if (code === 0) ok += 1;
    } catch (_e) {
      /* missing engine — counted as failure below */
    }
  }
  return ok;
}

async function cmdBenchQueue(rest: string[]): Promise<never> {
  const k = Math.max(1, parseInt(benchArg(rest, "k", "16")));
  const c = Math.min(Math.max(1, parseInt(benchArg(rest, "c", "8"))), k);
  const chunks: number[][] = Array.from({ length: c }, () => []);
  for (let i = 0; i < k; i++) chunks[i % c].push(i);
  let ok = 0;
  if (c === 1) {
    ok = benchQueueWorker(chunks[0]);
  } else {
    // run chunks concurrently: workers are synchronous sleeps, so the
    // honest way to overlap them here is worker threads — but to keep the
    // comparison shape identical to the queue path (event-loop tasks), we
    // spawn Deno.Command with async output and await all
    const ps = chunks.map((chunk) =>
      (async () => {
        let o = 0;
        for (const _u of chunk) {
          try {
            const cmd = new Deno.Command("mocksleep", { args: ["80"], stdout: "piped", stderr: "piped" });
            const { code } = await cmd.output();
            if (code === 0) o += 1;
          } catch (_e) {
            /* missing engine */
          }
        }
        return o;
      })(),
    );
    const oks = await Promise.all(ps);
    ok = oks.reduce((a, b) => a + b, 0);
  }
  console.log(`bench-queue cs=ok${ok},k${k},c${c}`);
  Deno.exit(ok === k ? 0 : 1);
}

function usage(): never {
  console.log("ytdl — YouTube downloader, Deno comparison build");
  console.log("");
  console.log("usage: deno run --allow-run --allow-read --allow-write ytdl.ts <cmd> [options]");
  console.log("cmds: doctor | info URL | get URL | queue FILE | selfcheck FIXTURE.json");
  console.log("options: --out DIR --quality N --format EXPR --audio KIND --subs");
  console.log("         --sub-langs LANGS --template T --jobs N --no-aria2");
  console.log("         --playlist --max-attempts N");
  console.log("bench (cross-language deep benchmark, docs/BENCHMARK-DEEP.md):");
  console.log("  bench-startup | bench-json F --n N | bench-table F --rounds R");
  console.log("  bench-lines --k K | bench-spawn --n N | bench-queue --k K --c C");
  Deno.exit(2);
}

// ---------------------------------------------------------- main

const a = [...Deno.args];
if (!a.length) usage();
const sub = a[0];
const rest = a.slice(1);
if (["help", "--help", "-h"].includes(sub)) {
  usage();
} else if (sub === "doctor") {
  await cmdDoctor();
} else if (sub === "info") {
  await cmdInfo(rest);
} else if (sub === "get") {
  await cmdGet(rest);
} else if (sub === "queue") {
  await cmdQueue(rest);
} else if (sub === "selfcheck") {
  await cmdSelfcheck(rest);
} else if (sub === "bench-startup") {
  cmdBenchStartup();
} else if (sub === "bench-json") {
  cmdBenchJson(rest);
} else if (sub === "bench-table") {
  cmdBenchTable(rest);
} else if (sub === "bench-lines") {
  cmdBenchLines(rest);
} else if (sub === "bench-spawn") {
  cmdBenchSpawn(rest);
} else if (sub === "bench-queue") {
  await cmdBenchQueue(rest);
} else {
  console.log(`error: unknown subcommand ${sub}`);
  usage();
}
