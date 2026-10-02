// TubeForge — downloads directory listing + safe file serving.
import { readdirSync, statSync, unlinkSync, join, resolve, ensureDir } from "./platform.ts";
import { loadSettings } from "./settings.ts";

export interface FileEntry {
  name: string;
  size: number;
  modified: number;
  isDir: boolean;
}

export function listDownloads(): { dir: string; files: FileEntry[] } {
  const s = loadSettings();
  ensureDir(s.downloadDir);
  const files: FileEntry[] = [];
  try {
    const names = readdirSync(s.downloadDir);
    for (const name of names) {
      try {
        const st = statSync(join(s.downloadDir, name));
        files.push({ name, size: st.size, modified: st.mtimeMs, isDir: st.isDirectory() });
      } catch {
        /* raced delete */
      }
    }
  } catch {
    /* unreadable dir */
  }
  files.sort((a, b) => b.modified - a.modified);
  return { dir: s.downloadDir, files };
}

/** Resolve a user-supplied file name inside the download dir — no traversal. */
export function safeResolve(name: string): string | null {
  if (!name || name.includes("/") || name.includes("\\") || name.startsWith(".")) return null;
  const s = loadSettings();
  const dir = resolve(s.downloadDir);
  const p = resolve(dir, name);
  if (p !== dir && !p.startsWith(dir + "/") && !(p === dir + "/" )) {
    if (!p.startsWith(dir)) return null;
  }
  try {
    if (!statSync(p).isFile()) return null;
  } catch {
    return null;
  }
  return p;
}

export function deleteFile(name: string): boolean {
  const p = safeResolve(name);
  if (!p) return false;
  try {
    unlinkSync(p);
    return true;
  } catch {
    return false;
  }
}
