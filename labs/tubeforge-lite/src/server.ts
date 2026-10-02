// TubeForge Lite — single-binary server (Deno 2).
// Same /api/tf/* contract as TubeForge desktop (shared engine, proven), plus:
//   • embedded vanilla UI served at /          (no React, no Next export)
//   • SSE stream at /api/tf/events             (live queue updates)
//   • aria2c resolved from the self-extracted bundle via PATH
import { UI_HTML } from "./ui.ts";
import { probeUrl } from "../shared/ytdlp.ts";
import {
  snapshot, subscribe, enqueue, cancelJob, retryJob, removeJob,
} from "../shared/engine.ts";
import { loadSettings, saveSettings } from "../shared/settings.ts";
import { listDownloads, safeResolve, deleteFile } from "../shared/files.ts";
import { healthReport } from "../shared/health.ts";
import { isSafeUrl, parseJobOptions } from "../shared/validate.ts";
import { fileSizeOf } from "../shared/platform.ts";
import type { JobOptions, ProbeResult, QueuedSnapshot } from "../shared/types.ts";
import { Readable } from "node:stream";
import { createReadStream } from "node:fs";
import proc from "node:process";
import { aria2BundleInfo } from "./aria2.ts";

const MIME: Record<string, string> = {
  html: "text/html; charset=utf-8",
  js: "text/javascript; charset=utf-8",
  css: "text/css; charset=utf-8",
  json: "application/json",
  svg: "image/svg+xml",
  png: "image/png",
};

function json(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

async function readBody(req: Request): Promise<unknown> {
  try { return await req.json(); } catch { return null; }
}

// ---------------------------------------------------------------------------
// API — identical contract to TubeForge desktop (+ /api/tf/events SSE)
// ---------------------------------------------------------------------------

async function api(req: Request, pathname: string): Promise<Response> {
  const method = req.method;

  if (pathname === "/api/tf/probe" && method === "POST") {
    const body = (await readBody(req)) as { url?: unknown } | null;
    if (!isSafeUrl(body?.url)) return json({ ok: false, error: "A valid http(s) URL is required." }, 400);
    const r = await probeUrl(body!.url as string);
    if (!r.ok) return json({ ok: false, error: (r as { error?: string }).error ?? "probe failed" }, 422);
    return json({ ok: true, result: (r as { result: ProbeResult }).result });
  }

  if (pathname === "/api/tf/download" && method === "POST") {
    const body = await readBody(req);
    const parsed = parseJobOptions(body);
    if (!parsed.ok) return json({ ok: false, error: (parsed as { error?: string }).error ?? "invalid options" }, 400);
    const opts = (parsed as { opts: JobOptions }).opts;
    const rawEntries = (body as { entryUrls?: unknown })?.entryUrls;
    const urls: string[] = [];
    if (Array.isArray(rawEntries)) {
      for (const u of rawEntries.slice(0, 500)) if (isSafeUrl(u)) urls.push(u);
    }
    if (urls.length === 0) urls.push(opts.url);
    const jobs = urls.map((u) =>
      enqueue({
        ...opts,
        url: u,
        title: urls.length > 1 ? undefined : opts.title,
        thumbnail: urls.length > 1 ? undefined : opts.thumbnail,
      }),
    );
    return json({ ok: true, count: jobs.length, jobs });
  }

  if (pathname === "/api/tf/queue" && method === "GET") return json(snapshot());

  if (pathname === "/api/tf/events" && method === "GET") {
    const enc = new TextEncoder();
    let unsub: () => void = () => {};
    const stream = new ReadableStream({
      start(controller) {
        const send = (snap: QueuedSnapshot) => {
          try { controller.enqueue(enc.encode(`data: ${JSON.stringify(snap)}\n\n`)); }
          catch { unsub(); }
        };
        send(snapshot());
        unsub = subscribe(send);
      },
      cancel() { unsub(); },
    });
    return new Response(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        "Access-Control-Allow-Origin": "*",
      },
    });
  }

  if (pathname === "/api/tf/cancel" && method === "POST") {
    const b = (await readBody(req)) as { id?: string } | null;
    if (!b?.id) return json({ ok: false, error: "id required" }, 400);
    return json({ ok: cancelJob(b.id) });
  }

  if (pathname === "/api/tf/retry" && method === "POST") {
    const b = (await readBody(req)) as { id?: string } | null;
    if (!b?.id) return json({ ok: false, error: "id required" }, 400);
    const job = retryJob(b.id);
    return job ? json({ ok: true, job }) : json({ ok: false, error: "job not found" }, 404);
  }

  if (pathname === "/api/tf/remove" && method === "POST") {
    const b = (await readBody(req)) as { id?: string } | null;
    if (!b?.id) return json({ ok: false, error: "id required" }, 400);
    return json({ ok: removeJob(b.id) });
  }

  if (pathname === "/api/tf/files" && method === "GET") return json({ ok: true, ...listDownloads() });

  if (pathname === "/api/tf/files" && method === "POST") {
    const b = (await readBody(req)) as { action?: string; name?: string } | null;
    if (b?.action === "delete" && typeof b.name === "string") return json({ ok: deleteFile(b.name) });
    return json({ ok: false, error: "unknown action" }, 400);
  }

  if (pathname === "/api/tf/settings" && method === "GET") return json({ ok: true, settings: loadSettings() });

  if (pathname === "/api/tf/settings" && method === "POST") {
    const body = (await readBody(req)) as Record<string, unknown> | null;
    if (!body) return json({ ok: false, error: "Invalid JSON body." }, 400);
    const current = loadSettings();
    const next = { ...current, ...body, toolPaths: { ...current.toolPaths, ...((body.toolPaths as object) ?? {}) } };
    return json({ ok: true, settings: saveSettings(next as Parameters<typeof saveSettings>[0]) });
  }

  if (pathname === "/api/tf/health" && method === "GET") {
    return json({
      ok: true,
      ...(await healthReport(`deno ${Deno.version.deno} (TubeForge Lite)`)),
      app: "tubeforge-lite",
      aria2Bundle: aria2BundleInfo(),
      dataDir: proc.env.TUBEFORGE_DATA ?? "",
    });
  }

  // raw file serving (range-aware, no traversal)
  if (pathname === "/api/tf/files/raw" && method === "GET") {
    const url = new URL(req.url);
    const name = url.searchParams.get("name") ?? "";
    const p = safeResolve(name);
    if (!p) return new Response("Not found", { status: 404 });
    const size = fileSizeOf(p) ?? 0;
    const ext = name.split(".").pop()?.toLowerCase() ?? "";
    const headers = new Headers({
      "Content-Type": MIME[ext] ?? "application/octet-stream",
      "Accept-Ranges": "bytes",
      "Content-Disposition": `attachment; filename*=UTF-8''${encodeURIComponent(name)}`,
    });
    const range = req.headers.get("range");
    if (range) {
      const m = /bytes=(\d*)-(\d*)/.exec(range);
      const start = m && m[1] ? parseInt(m[1]) : 0;
      const end = m && m[2] ? Math.min(parseInt(m[2]), size - 1) : size - 1;
      if (start >= size || start > end) {
        return new Response(null, { status: 416, headers: { "Content-Range": `bytes */${size}` } });
      }
      headers.set("Content-Range", `bytes ${start}-${end}/${size}`);
      headers.set("Content-Length", String(end - start + 1));
      const web = Readable.toWeb(createReadStream(p, { start, end })) as ReadableStream;
      return new Response(web, { status: 206, headers });
    }
    headers.set("Content-Length", String(size));
    const web = Readable.toWeb(createReadStream(p)) as ReadableStream;
    return new Response(web, { status: 200, headers });
  }

  return json({ ok: false, error: "not found" }, 404);
}

// ---------------------------------------------------------------------------
// server
// ---------------------------------------------------------------------------

export function serve(opts: { port: number; open: boolean }): void {
  Deno.serve({ port: opts.port }, async (req: Request) => {
    const url = new URL(req.url);
    try {
      if (url.pathname.startsWith("/api/")) return await api(req, url.pathname);
      if (url.pathname === "/" || url.pathname === "/index.html") {
        return new Response(UI_HTML, { headers: { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-cache" } });
      }
      return json({ ok: false, error: "not found" }, 404);
    } catch (err) {
      return json({ ok: false, error: err instanceof Error ? err.message : String(err) }, 500);
    }
  });

  const url = `http://localhost:${opts.port}`;
  const b = aria2BundleInfo();
  console.log(`\n  TubeForge Lite — one file, everything inside`);
  console.log(`  yt-dlp + ffmpeg: from PATH (or Settings) · aria2c: ${b.bundled ? `bundled v${b.version} (self-extracting)` : "from PATH"}`);
  console.log(`  → ${url}\n`);
  if (opts.open) openBrowser(url);
}

function openBrowser(u: string): void {
  const sys = proc.platform;
  const cmds: [string, string[]][] =
    sys === "win32"
      ? [["cmd", ["/c", "start", "", u]]]
      : sys === "darwin"
        ? [["open", [u]]]
        : [["xdg-open", [u]]];
  for (const [cmd, args] of cmds) {
    try {
      new Deno.Command(cmd, { args, stdin: "null", stdout: "null", stderr: "null" }).spawn();
      return;
    } catch { /* try next */ }
  }
}
