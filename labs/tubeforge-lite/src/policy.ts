// TubeForge Lite — Operon policy lane.
//
// A policy is an ordinary Operon script (see operon/policies/*.op). It runs
// inside Operon's deny-by-default sandbox — no filesystem, no network, no
// processes — and emits `key = value` lines via promote(). The engine reads
// them here and enforces them on every enqueue and every transfer.
//
// Why this lane exists: it is the one feature in the TubeForge stack that
// yt-dlp / ffmpeg / aria2 / Deno cannot safely provide — user-WRITABLE and
// user-SHAREABLE download rules. A .op file downloaded from the internet
// cannot touch the OS; worst case it denies your downloads (fail-closed).
//
// Environment:
//   TUBEFORGE_POLICY  path to the .op policy file (unset = policy off)
//   TF_OPERON         path to the operon binary (default: PATH "operon")
// CLI: `tubeforge-lite serve --policy FILE` sets TUBEFORGE_POLICY.
//
// Semantics: policy enabled but operon missing / script error / non-zero
// exit => FAIL-CLOSED (all downloads refused). Deny-by-default, end to end.
import proc from "node:process";
import { setPolicySizeGate } from "../shared/engine.ts";
import type { JobOptions } from "../shared/types.ts";

export interface PolicyRules {
  file: string;
  name: string;
  version: string;
  allowVideo: boolean;
  allowAudio: boolean;
  allowPlaylist: boolean;
  maxBytes: number; // 0 = no cap (reactive: abort once total size is known)
  maxQuality: number; // 0 = no cap (preventive: downgrades at enqueue)
  forceKind: "audio" | null; // rewrite video requests to audio
  denyDomains: string[]; // substring match on the URL, lowercase
  allowDomains: string[]; // if non-empty, ONLY these substrings pass
  reasons: Record<string, string>; // custom denial messages
}

export type PolicyState =
  | { kind: "off" }
  | { kind: "ok"; rules: PolicyRules; operonVersion: string | null }
  | { kind: "error"; file: string; error: string };

const BOOL_TRUE = new Set(["1", "true", "yes", "on"]);
const KNOWN_REASON_KEYS = new Set([
  "reason_video",
  "reason_audio",
  "reason_playlist",
  "reason_domain",
  "reason_size",
]);

function truthy(v: string): boolean {
  return BOOL_TRUE.has(v.trim().toLowerCase());
}

function splitList(v: string): string[] {
  return v
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean);
}

/** Parse the promote() output of a policy script into typed rules. */
export function parsePolicyOut(file: string, out: string): PolicyRules {
  const r: PolicyRules = {
    file,
    name: "policy",
    version: "0",
    allowVideo: true,
    allowAudio: true,
    allowPlaylist: true,
    maxBytes: 0,
    maxQuality: 0,
    forceKind: null,
    denyDomains: [],
    allowDomains: [],
    reasons: {},
  };
  for (const rawLine of out.split(/\r?\n/)) {
    const line = rawLine.trim();
    const eq = line.indexOf("=");
    if (eq <= 0) continue;
    const key = line.slice(0, eq).trim().toLowerCase();
    if (!/^[a-z_]{1,40}$/.test(key)) continue;
    const val = line.slice(eq + 1).trim().replace(/^"(.*)"$/, "$1");
    switch (key) {
      case "name": r.name = val.slice(0, 60) || r.name; break;
      case "version": r.version = val.slice(0, 20) || r.version; break;
      case "allow_video": r.allowVideo = truthy(val); break;
      case "allow_audio": r.allowAudio = truthy(val); break;
      case "allow_playlist": r.allowPlaylist = truthy(val); break;
      case "max_bytes": r.maxBytes = Math.max(0, Math.floor(Number(val) || 0)); break;
      case "max_quality": r.maxQuality = Math.max(0, Math.floor(Number(val) || 0)); break;
      case "force_kind": r.forceKind = val === "audio" ? "audio" : null; break;
      case "deny_domains": r.denyDomains = splitList(val); break;
      case "allow_domains": r.allowDomains = splitList(val); break;
      default:
        if (KNOWN_REASON_KEYS.has(key)) r.reasons[key] = val.slice(0, 300);
        break; // unknown keys ignored (forward compatible)
    }
  }
  return r;
}

function runOperon(bin: string, args: string[]): { ok: boolean; out: string; error: string } {
  try {
    const cmd = new Deno.Command(bin, {
      args,
      stdin: "null",
      stdout: "piped",
      stderr: "piped",
    });
    const res = cmd.outputSync();
    const out = new TextDecoder().decode(res.stdout);
    const err = new TextDecoder().decode(res.stderr);
    if (!res.success) {
      return { ok: false, out: "", error: (err.trim() || `operon exited with code ${res.code}`).slice(0, 400) };
    }
    return { ok: true, out, error: "" };
  } catch (e) {
    return { ok: false, out: "", error: `cannot run operon (${e instanceof Error ? e.message : String(e)})` };
  }
}

let cache: PolicyState | null = null;

/** Re-read the policy from disk on the next loadPolicy(). */
export function reloadPolicy(): PolicyState {
  cache = null;
  return loadPolicy();
}

export function loadPolicy(): PolicyState {
  if (cache) return cache;
  const file = proc.env.TUBEFORGE_POLICY?.trim() ?? "";
  if (!file) {
    cache = { kind: "off" };
    return cache;
  }
  const bin = proc.env.TF_OPERON?.trim() || "operon";
  const verR = runOperon(bin, ["--version"]);
  const operonVersion = verR.ok ? (verR.out.trim().split(/\r?\n/)[0] ?? null) : null;
  const r = runOperon(bin, ["run", file]);
  if (!r.ok) {
    cache = { kind: "error", file, error: r.error };
    return cache;
  }
  cache = { kind: "ok", rules: parsePolicyOut(file, r.out), operonVersion };
  return cache;
}

/** Health/doctor payload. */
export function policyStatus(): Record<string, unknown> {
  const st = loadPolicy();
  if (st.kind === "off") return { enabled: false };
  if (st.kind === "error") {
    return { enabled: true, failClosed: true, file: st.file, error: st.error };
  }
  const r = st.rules;
  return {
    enabled: true,
    failClosed: false,
    file: r.file,
    name: r.name,
    version: r.version,
    allowVideo: r.allowVideo,
    allowAudio: r.allowAudio,
    allowPlaylist: r.allowPlaylist,
    maxBytes: r.maxBytes,
    maxQuality: r.maxQuality,
    operon: st.operonVersion,
  };
}

export interface GateResult {
  ok: boolean;
  opts: JobOptions; // set on success (kind/quality possibly rewritten)
  error?: string; // set on denial
}

/**
 * Preventive gate at enqueue time: kind/playlist/domain rules + kind
 * rewrite + quality ceiling. Returns rewritten opts on success.
 */
export function gateEnqueue(opts: JobOptions, urls: string[]): GateResult {
  const st = loadPolicy();
  if (st.kind === "off") return { ok: true, opts };
  if (st.kind === "error") {
    return {
      ok: false,
      opts,
      error: `policy: ${st.file} failed to load — refusing all downloads (fail-closed). ${st.error}`,
    };
  }
  const r = st.rules;
  const deny = (reason: string): GateResult => ({ ok: false, opts, error: `policy (${r.name}): ${reason}` });

  if (urls.length > 1 && !r.allowPlaylist) {
    return deny(r.reasons.reason_playlist ?? "playlists and multi-URL batches are not allowed");
  }
  for (const u of urls) {
    const low = u.toLowerCase();
    for (const d of r.denyDomains) {
      if (low.includes(d)) return deny(r.reasons.reason_domain ?? `domain is denied by policy: ${d}`);
    }
    if (r.allowDomains.length > 0 && !r.allowDomains.some((d) => low.includes(d))) {
      return deny(r.reasons.reason_domain ?? "domain is not on the policy whitelist");
    }
  }

  let kind = opts.kind;
  if (kind === "video" && !r.allowVideo) {
    if (r.forceKind === "audio") kind = "audio";
    else return deny(r.reasons.reason_video ?? "video downloads are disabled by policy");
  }
  if (kind === "audio" && !r.allowAudio) {
    return deny(r.reasons.reason_audio ?? "audio downloads are disabled by policy");
  }

  let quality = opts.quality;
  if (kind === "video" && r.maxQuality > 0 && (quality === 0 || quality > r.maxQuality)) {
    quality = r.maxQuality;
  }
  return { ok: true, opts: { ...opts, kind, quality } };
}

/**
 * Reactive gate during transfer: abort once the total size is known and
 * exceeds max_bytes. Returns the denial message, or null to proceed.
 */
export function gateSize(totalBytes: number): string | null {
  if (totalBytes <= 0) return null;
  const st = loadPolicy();
  if (st.kind !== "ok") return null; // error state already fail-closes at enqueue
  const r = st.rules;
  if (r.maxBytes > 0 && totalBytes > r.maxBytes) {
    const capMiB = Math.round(r.maxBytes / 1048576);
    const gotMiB = Math.round(totalBytes / 1048576);
    return r.reasons.reason_size ??
      `size cap exceeded: item is ${gotMiB} MiB, policy allows ${capMiB} MiB`;
  }
  return null;
}

/** Wire the reactive size gate into the engine (no-op while policy is off). */
export function installPolicySizeGate(): void {
  setPolicySizeGate((info) => gateSize(info.totalBytes));
}
