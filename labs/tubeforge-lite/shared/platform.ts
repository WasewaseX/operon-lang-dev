// TubeForge — platform layer.
// Uses node: APIs exclusively (fs, path, child_process, os, crypto, process)
// so the identical module runs under Node (Next.js API routes) and Deno 2
// (desktop single-binary server) — Deno 2 implements the node: specifiers.
import { spawn, execFile, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, statSync, rmdirSync, readFileSync, writeFileSync, readdirSync, unlinkSync } from "node:fs";
import { join, dirname, resolve, basename, delimiter } from "node:path";
import { homedir, tmpdir, platform, arch } from "node:os";
import { randomUUID } from "node:crypto";
import process from "node:process";

export { spawn, execFile, existsSync, mkdirSync, statSync, rmdirSync, readFileSync, writeFileSync, readdirSync, unlinkSync, join, dirname, resolve, basename, homedir, tmpdir, platform, arch, randomUUID };
export type { ChildProcess };
export const proc = process;

/** Expand a leading "~" to the user home directory. */
export function expandHome(p: string): string {
  if (!p) return p;
  if (p === "~") return homedir();
  if (p.startsWith("~/") || p.startsWith("~\\")) return join(homedir(), p.slice(2));
  return p;
}

/**
 * Resolve a tool binary to an absolute path. Spawns inherit restricted PATHs
 * in hosted dev environments, so probe a candidate list of well-known dirs.
 */
export function resolveTool(name: string, override?: string): string | null {
  const candidates: string[] = [];
  if (override) candidates.push(expandHome(override));
  const extraDirs = [
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/local/sbin",
    join(homedir(), ".local", "bin"),
    join(homedir(), ".deno", "bin"),
    join(homedir(), ".cargo", "bin"),
    join(homedir(), ".venv", "bin"),
    join(homedir(), "bin"),
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
  ];
  for (const dir of extraDirs) candidates.push(join(dir, name));
  // real PATH walk last (the self-extracted aria2c lives in a dir we prepend
  // to PATH at startup — a bare existsSync(name) can never see it)
  for (const dir of (process.env.PATH ?? "").split(delimiter)) {
    if (dir) candidates.push(join(dir, name));
  }
  for (const c of candidates) {
    try {
      if (existsSync(c) && statSync(c).isFile()) {
        // On Windows, tools may carry .exe/.cmd suffixes
        if (platform() === "win32" && !/\.(exe|cmd|bat)$/i.test(c)) {
          const exe = c + ".exe";
          if (existsSync(exe)) return exe;
        }
        return c;
      }
    } catch {
      /* keep probing */
    }
  }
  return null;
}

export interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
  error?: string;
}

/** Simple promisified execFile with a hard timeout. */
export function runCapture(
  cmd: string,
  args: string[],
  timeoutMs = 120_000,
): Promise<RunResult> {
  return new Promise((acc) => {
    if (!cmd) {
      acc({ code: -1, stdout: "", stderr: "", error: "binary not found" });
      return;
    }
    execFile(
      cmd,
      args,
      { timeout: timeoutMs, maxBuffer: 64 * 1024 * 1024, windowsHide: true },
      (err, stdout, stderr) => {
        acc({
          code: err && typeof (err as { code?: number }).code === "number" ? (err as { code: number }).code : err ? -1 : 0,
          stdout: stdout?.toString() ?? "",
          stderr: stderr?.toString() ?? "",
          error: err && typeof (err as { code?: number }).code !== "number" ? String(err.message ?? err) : undefined,
        });
      },
    );
  });
}

/** Spawn a long-running process detached (own group) so we can tree-kill it. */
export function spawnDetached(cmd: string, args: string[], opts: { env?: Record<string, string> } = {}): ChildProcess {
  return spawn(cmd, args, {
    detached: true,
    windowsHide: true,
    stdio: ["ignore", "pipe", "pipe"],
    env: opts.env ? { ...proc.env, ...opts.env } : proc.env,
  });
}

/** Terminate a detached process group (POSIX) or the process (Windows). */
export function killTree(child: ChildProcess): void {
  if (!child.pid) return;
  try {
    if (platform() === "win32") {
      // taskkill tree is the reliable way on Windows
      spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true });
    } else {
      process.kill(-child.pid, "SIGTERM");
    }
  } catch {
    try { child.kill("SIGKILL"); } catch { /* already gone */ }
  }
  // hard-kill safety net
  setTimeout(() => {
    try {
      if (platform() !== "win32") process.kill(-child.pid!, "SIGKILL");
      else child.kill("SIGKILL");
    } catch { /* gone */ }
  }, 4000).unref?.();
}

export function ensureDir(p: string): boolean {
  try {
    mkdirSync(p, { recursive: true });
    return true;
  } catch {
    return false;
  }
}

export function isWritableDir(p: string): boolean {
  try {
    mkdirSync(p, { recursive: true });
    const probe = join(p, `.tf-write-${randomUUID()}`);
    mkdirSync(probe);
    try { rmdirSync(probe); } catch { /* ignore */ }
    return true;
  } catch {
    return false;
  }
}

export function fileSizeOf(p: string): number | undefined {
  try {
    const st = statSync(p);
    return st.isFile() ? st.size : undefined;
  } catch {
    return undefined;
  }
}

/** Default data dir (job store + settings): <project|exe dir>/data */
export function defaultDataDir(): string {
  return process.env.TUBEFORGE_DATA || join(proc.cwd(), "data");
}

/** Default download dir. */
export function defaultDownloadDir(): string {
  return process.env.TUBEFORGE_DOWNLOADS || join(proc.cwd(), "downloads");
}
