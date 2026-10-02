// TubeForge — settings store (JSON file on disk, runtime-agnostic).
import {
  defaultDataDir, defaultDownloadDir, join, existsSync,
  readFileSync, writeFileSync, mkdirSync,
} from "./platform.ts";

import type { Settings } from "./types.ts";

export function defaultSettings(): Settings {
  return {
    downloadDir: defaultDownloadDir(),
    filenameTemplate: "%(title).100B [%(id)s].%(ext)s",
    concurrentDownloads: 2,
    useAria2: true,
    aria2Connections: 8,
    defaultQuality: 1080,
    defaultContainer: "mp4",
    defaultAudioFormat: "mp3",
    embedThumbnail: true,
    embedMetadata: true,
    embedSubs: false,
    writeSubs: false,
    subLangs: "en.*",
    toolPaths: {},
  };
}

function settingsPath(): string {
  return join(defaultDataDir(), "settings.json");
}

let cache: Settings | null = null;

export function loadSettings(): Settings {
  if (cache) return cache;
  const p = settingsPath();
  try {
    if (existsSync(p)) {
      const raw = JSON.parse(readFileSync(p, "utf8")) as Partial<Settings>;
      const base = defaultSettings();
      cache = {
        ...base,
        ...raw,
        toolPaths: { ...base.toolPaths, ...(raw.toolPaths ?? {}) },
      };
      return cache;
    }
  } catch {
    // corrupted settings -> fall back to defaults (and let save repair it)
  }
  cache = defaultSettings();
  return cache;
}

export function saveSettings(next: Settings): Settings {
  // hard-clone + sanitize
  const base = defaultSettings();
  const clean: Settings = {
    ...base,
    ...next,
    downloadDir: String(next.downloadDir || base.downloadDir),
    filenameTemplate: String(next.filenameTemplate || base.filenameTemplate),
    concurrentDownloads: Math.min(4, Math.max(1, Number(next.concurrentDownloads) || 1)),
    aria2Connections: Math.min(16, Math.max(1, Number(next.aria2Connections) || 8)),
    toolPaths: { ...next.toolPaths },
  };
  cache = clean;
  try {
    mkdirSync(defaultDataDir(), { recursive: true });
    writeFileSync(settingsPath(), JSON.stringify(clean, null, 2), "utf8");
  } catch {
    // read-only FS (hosted sandbox edge) — settings stay in-memory
  }
  return clean;
}

/** For test isolation / runtime switches. */
export function resetSettingsCache(): void {
  cache = null;
}
