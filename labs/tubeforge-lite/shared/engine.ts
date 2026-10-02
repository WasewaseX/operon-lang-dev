// TubeForge — job store + queue engine.
// In-memory queue with JSON persistence, event emitter for SSE, process-tree
// cancellation, concurrency limiting. Runtime-agnostic (node: APIs).
import {
  resolveTool, spawnDetached, killTree, ensureDir, fileSizeOf,
  join, basename, existsSync, readFileSync, writeFileSync, mkdirSync,
  defaultDataDir, randomUUID, proc,
} from "./platform.ts";
import { loadSettings } from "./settings.ts";
import { buildDownloadArgs, parseProgressLine, DONE_PREFIX, ytdlpBin, humanizeError } from "./ytdlp.ts";
import type { Job, JobOptions, JobStatus, QueuedSnapshot } from "./types.ts";

const MAX_LOG = 40;
const MAX_PERSISTED = 300;

type Listener = (snap: QueuedSnapshot) => void;

// --- policy size gate (additive; no-op until a host installs one) -----------
export interface PolicySizeInfo {
  totalBytes: number;
}
export type PolicySizeGate = (info: PolicySizeInfo) => string | null;
let policySizeGate: PolicySizeGate | null = null;
/** Hosts (TubeForge Lite policy lane) install a gate to abort oversized
 *  transfers; the returned string becomes the job's error message. */
export function setPolicySizeGate(fn: PolicySizeGate | null): void {
  policySizeGate = fn;
}
// --------------------------------------------------------------------------

interface RuntimeJob extends Job {
  child?: import("./platform.ts").ChildProcess;
  canceled?: boolean;
}

interface StoreState {
  jobs: Map<string, RuntimeJob>;
  order: string[];
  listeners: Set<Listener>;
  version: number;
  persistTimer: ReturnType<typeof setTimeout> | null;
  pumping: boolean;
}

const g = globalThis as unknown as { __tubeforgeStore?: StoreState };

function state(): StoreState {
  if (!g.__tubeforgeStore) {
    g.__tubeforgeStore = {
      jobs: new Map(),
      order: [],
      listeners: new Set(),
      version: 0,
      persistTimer: null,
      pumping: false,
    };
    void restore();
  }
  return g.__tubeforgeStore;
}

// ---------------------------------------------------------------------------
// persistence
// ---------------------------------------------------------------------------

function persistPath(): string {
  return join(defaultDataDir(), "jobs.json");
}

function restore(): void {
  try {
    if (!existsSync(persistPath())) return;
    const arr = JSON.parse(readFileSync(persistPath(), "utf8")) as Job[];
    if (!Array.isArray(arr)) return;
    for (const j of arr.slice(-MAX_PERSISTED)) {
      if (!j?.id || !j?.opts?.url) continue;
      // never resurrect an in-flight job after a restart
      const status: JobStatus = j.status === "queued" || j.status === "downloading" || j.status === "processing"
        ? "error"
        : j.status;
      const job: RuntimeJob = {
        ...j,
        status,
        error: status === "error" ? (j.error ?? "Interrupted by restart") : j.error,
        logTail: j.logTail ?? [],
      };
      state().jobs.set(job.id, job);
      state().order.push(job.id);
    }
  } catch {
    /* corrupted store: start clean */
  }
}

function schedulePersist(): void {
  const st = state();
  if (st.persistTimer) return;
  st.persistTimer = setTimeout(() => {
    st.persistTimer = null;
    try {
      mkdirSync(defaultDataDir(), { recursive: true });
      const arr = st.order
        .map((id) => st.jobs.get(id))
        .filter((j): j is RuntimeJob => !!j)
        .slice(-MAX_PERSISTED)
        .map(({ child: _c, canceled: _x, ...rest }) => rest);
      writeFileSync(persistPath(), JSON.stringify(arr), "utf8");
    } catch {
      /* read-only FS: memory-only mode */
    }
  }, 800);
}

// ---------------------------------------------------------------------------
// snapshot + subscribe
// ---------------------------------------------------------------------------

export function snapshot(): QueuedSnapshot {
  const st = state();
  const jobs = st.order
    .map((id) => st.jobs.get(id))
    .filter((j): j is RuntimeJob => !!j)
    .sort((a, b) => b.createdAt - a.createdAt)
    .map((j) => ({ ...j, child: undefined, canceled: undefined }) as unknown as Job);
  let activeCount = 0;
  for (const j of st.jobs.values()) {
    if (j.status === "queued" || j.status === "downloading" || j.status === "processing") activeCount++;
  }
  return { jobs, activeCount, version: st.version };
}

export function subscribe(fn: Listener): () => void {
  const st = state();
  st.listeners.add(fn);
  return () => st.listeners.delete(fn);
}

function emit(): void {
  const st = state();
  st.version++;
  const snap = snapshot();
  for (const fn of st.listeners) {
    try { fn(snap); } catch { /* listener error: drop */ }
  }
  schedulePersist();
}

// ---------------------------------------------------------------------------
// job lifecycle
// ---------------------------------------------------------------------------

export function enqueue(opts: JobOptions): Job {
  const st = state();
  const now = Date.now();
  const job: RuntimeJob = {
    id: randomUUID(),
    createdAt: now,
    updatedAt: now,
    status: "queued",
    opts,
    progress: { percent: 0, speed: "—", eta: "—", downloadedBytes: 0, totalBytes: 0 },
    logTail: [],
  };
  st.jobs.set(job.id, job);
  st.order.push(job.id);
  emit();
  void pump();
  const { child: _c, canceled: _x, ...safe } = job;
  return safe as Job;
}

export function cancelJob(id: string): boolean {
  const st = state();
  const job = st.jobs.get(id);
  if (!job) return false;
  if (job.status === "completed" || job.status === "canceled" || job.status === "error") return false;
  job.canceled = true;
  if (job.child) {
    killTree(job.child);
    job.child = undefined;
  }
  setStatus(job, "canceled");
  void pump();
  return true;
}

export function retryJob(id: string): Job | null {
  const st = state();
  const old = st.jobs.get(id);
  if (!old) return null;
  const job = enqueue({ ...old.opts });
  // drop the failed/canceled original to keep the list tidy
  st.jobs.delete(id);
  st.order = st.order.filter((x) => x !== id);
  emit();
  return job;
}

export function removeJob(id: string): boolean {
  const st = state();
  const job = st.jobs.get(id);
  if (!job) return false;
  if (job.status === "downloading" || job.status === "processing" || job.status === "queued") {
    cancelJob(id);
  }
  st.jobs.delete(id);
  st.order = st.order.filter((x) => x !== id);
  emit();
  return true;
}

function pushLog(job: RuntimeJob, line: string): void {
  job.logTail.push(line);
  if (job.logTail.length > MAX_LOG) job.logTail.splice(0, job.logTail.length - MAX_LOG);
}

function setStatus(job: RuntimeJob, status: JobStatus, extra?: Partial<Job>): void {
  job.status = status;
  job.updatedAt = Date.now();
  if (extra) Object.assign(job, extra);
  emit();
}

// ---------------------------------------------------------------------------
// queue pump
// ---------------------------------------------------------------------------

async function pump(): Promise<void> {
  const st = state();
  if (st.pumping) return;
  st.pumping = true;
  try {
    const conc = Math.min(4, Math.max(1, loadSettings().concurrentDownloads));
    for (;;) {
      const active = [...st.jobs.values()].filter(
        (j) => j.status === "downloading" || j.status === "processing",
      ).length;
      if (active >= conc) break;
      const next = st.order
        .map((id) => st.jobs.get(id))
        .find((j) => j && j.status === "queued");
      if (!next) break;
      void startJob(next);
      // give the starter a tick so active counts update
      await new Promise((r) => setTimeout(r, 50));
    }
  } finally {
    st.pumping = false;
  }
}

async function startJob(job: RuntimeJob): Promise<void> {
  const s = loadSettings();
  ensureDir(s.downloadDir);
  const bin = ytdlpBin();
  if (!bin) {
    setStatus(job, "error", { error: "yt-dlp binary not found. Install it or set the path in Settings." });
    return;
  }
  const aria = s.useAria2 ? resolveTool("aria2c", s.toolPaths.aria2c) : null;
  if (s.useAria2 && !aria) {
    pushLog(job, "[tubeforge] aria2c not found — falling back to yt-dlp native downloader");
  }
  const { args } = buildDownloadArgs({
    url: job.opts.url,
    kind: job.opts.kind,
    quality: job.opts.quality,
    container: job.opts.container,
    audioFormat: job.opts.audioFormat,
    audioQuality: job.opts.audioQuality,
    embedThumbnail: job.opts.embedThumbnail,
    embedMetadata: job.opts.embedMetadata,
    embedSubs: job.opts.embedSubs,
    writeSubs: job.opts.writeSubs,
    subLangs: job.opts.subLangs,
    useAria2: s.useAria2 && !!aria,
    aria2Connections: job.opts.aria2Connections || s.aria2Connections,
    downloadDir: s.downloadDir,
    filenameTemplate: s.filenameTemplate,
    cookiesFile: s.cookiesFile,
  });

  setStatus(job, "downloading");
  pushLog(job, `[tubeforge] spawn: yt-dlp ${job.opts.kind} ${job.opts.quality ? `${job.opts.quality}p` : "best"}`);

  const child = spawnDetached(bin, args);
  job.child = child;

  let stderrBuf = "";
  let doneResolved = false;

  child.stdout?.on("data", (chunk: Buffer) => {
    const text = chunk.toString();
    for (const rawLine of text.split(/\r?\n/)) {
      const line = rawLine.trim();
      if (!line) continue;
      const prog = parseProgressLine(line);
      if (prog) {
        if (prog.totalBytes > 0 && policySizeGate) {
          const denied = policySizeGate({ totalBytes: prog.totalBytes });
          if (denied) {
            job.canceled = true;
            if (job.child) {
              killTree(job.child);
              job.child = undefined;
            }
            pushLog(job, `[policy] ${denied}`);
            setStatus(job, "error", { error: denied });
            return;
          }
        }
        job.progress = {
          percent: prog.percent,
          speed: prog.speed,
          eta: prog.eta,
          downloadedBytes: prog.downloadedBytes,
          totalBytes: prog.totalBytes,
        };
        job.updatedAt = Date.now();
        emitThrottled();
        continue;
      }
      if (line.startsWith(DONE_PREFIX)) {
        const filePath = line.slice(DONE_PREFIX.length).trim();
        doneResolved = true;
        const size = fileSizeOf(filePath);
        setStatus(job, "completed", {
          filePath,
          fileName: basename(filePath),
          fileSize: size,
          progress: { ...job.progress, percent: 100, speed: "—", eta: "—" },
        });
        pushLog(job, `[tubeforge] done: ${filePath}`);
        continue;
      }
      if (/^\[(Merger|ExtractAudio|EmbedThumbnail|Fixup|Metadata|VideoRemuxer|SubtitlesConvertor)/i.test(line)) {
        if (job.status !== "processing") setStatus(job, "processing");
        pushLog(job, line);
        continue;
      }
      // keep other notable lines
      if (line.startsWith("[download] Destination") || line.startsWith("[info]")) pushLog(job, line);
    }
  });

  child.stderr?.on("data", (chunk: Buffer) => {
    stderrBuf += chunk.toString();
    const lines = stderrBuf.split(/\r?\n/);
    stderrBuf = lines.pop() ?? "";
    for (const rawLine of lines) {
      const line = rawLine.trim();
      if (!line) continue;
      pushLog(job, line.slice(0, 300));
      if (job.logTail.length > MAX_LOG) job.logTail.splice(0, job.logTail.length - MAX_LOG);
    }
  });

  child.on("error", (err) => {
    doneResolved = true;
    setStatus(job, "error", { error: `Failed to launch yt-dlp: ${err.message}` });
    void pump();
  });

  child.on("close", (code) => {
    job.child = undefined;
    if (job.canceled) return; // cancelJob already set the status
    if (doneResolved) {
      if (job.status !== "completed") {
        setStatus(job, code === 0 ? "completed" : "error", code === 0 ? undefined : { error: `yt-dlp exited with code ${code}` });
      }
    } else if (code === 0) {
      setStatus(job, "completed", { progress: { ...job.progress, percent: 100 } });
    } else {
      const lastErr = job.logTail.filter((l) => l.startsWith("ERROR")).slice(-1)[0];
      setStatus(job, "error", {
        error: humanizeError(lastErr ?? (stderrBuf.trim() || `yt-dlp exited with code ${code}`)),
      });
    }
    void pump();
  });
}

// throttle snapshots: progress lines can be very chatty
let lastEmit = 0;
function emitThrottled(): void {
  const now = Date.now();
  if (now - lastEmit < 200) return;
  lastEmit = now;
  emit();
}
