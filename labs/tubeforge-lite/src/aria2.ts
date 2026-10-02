// TubeForge Lite — embedded aria2 support.
// The compile-time bundle (aria2-bundle.ts) carries ONE gzipped aria2c binary
// for the compile target. On first run it is extracted to <dataDir>/bin/ and
// the bin dir is PREPENDED to PATH so the (untouched) engine tool resolution
// finds it via the normal PATH candidate — zero engine changes needed.
import { ARIA2_GZ_B64, ARIA2_PLATFORM, ARIA2_VERSION, ARIA2_SHA256 } from "./aria2-bundle.ts";
import { join, delimiter } from "node:path";
import { existsSync, mkdirSync, writeFileSync, chmodSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { createHash } from "node:crypto";
import proc from "node:process";

export interface Aria2BundleInfo {
  bundled: boolean;
  platform: string;
  version: string;
  sha256: string;
}

export function aria2BundleInfo(): Aria2BundleInfo {
  return {
    bundled: ARIA2_GZ_B64 != null && ARIA2_GZ_B64.length > 0,
    platform: ARIA2_PLATFORM,
    version: ARIA2_VERSION,
    sha256: ARIA2_SHA256,
  };
}

export interface EnsureResult {
  extracted: boolean;
  path: string | null;
  error?: string;
}

/**
 * Extract the bundled aria2c (idempotent) and make it visible on PATH.
 * Safe to call at every startup; writes only when the binary is missing.
 */
export function ensureBundledAria2(dataDir: string): EnsureResult {
  if (!ARIA2_GZ_B64) return { extracted: false, path: null };
  try {
    const binDir = join(dataDir, "bin");
    const name = proc.platform === "win32" ? "aria2c.exe" : "aria2c";
    const p = join(binDir, name);
    if (!existsSync(p)) {
      mkdirSync(binDir, { recursive: true });
      const gz = Buffer.from(ARIA2_GZ_B64, "base64");
      const raw = gunzipSync(gz);
      // integrity: refuse to install a corrupted payload
      if (ARIA2_SHA256) {
        const got = createHash("sha256").update(raw).digest("hex");
        if (got !== ARIA2_SHA256) {
          return { extracted: false, path: null, error: "aria2 bundle sha256 mismatch" };
        }
      }
      writeFileSync(p, raw);
      if (proc.platform !== "win32") {
        try { chmodSync(p, 0o755); } catch { /* best effort */ }
      }
    }
    if (!(proc.env.PATH ?? "").split(delimiter).includes(binDir)) {
      proc.env.PATH = binDir + delimiter + (proc.env.PATH ?? "");
    }
    return { extracted: true, path: p };
  } catch (e) {
    return { extracted: false, path: null, error: e instanceof Error ? e.message : String(e) };
  }
}
