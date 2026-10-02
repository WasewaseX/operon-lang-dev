// TubeForge Lite — entrypoint.
//
//   tubeforge-lite                      → server + UI (auto-opens browser)
//   tubeforge-lite serve [--port N] [--no-open] [--data DIR]
//   tubeforge-lite <url> [urls…] [opts] → headless CLI download
//   tubeforge-lite doctor               → tool health report
//   tubeforge-lite version
//
// ONE deno-compiled executable; yt-dlp + ffmpeg come from PATH (or Settings
// overrides); aria2c is bundled INSIDE the exe and self-extracts on first run.
import proc from "node:process";
import { join, dirname } from "node:path";
import { homedir } from "node:os";
import { existsSync } from "node:fs";

const VERSION = "1.0.0";
export { VERSION };

function flagValue(args: string[], name: string): string | undefined {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
}

/**
 * Data dir precedence: --data flag > $TUBEFORGE_DATA > portable ./data next to
 * the executable > ~/.tubeforge-lite. The engine reads $TUBEFORGE_DATA at call
 * time, so set it before any engine import does work.
 */
function resolveDataDir(flag?: string): string {
  if (flag) return flag;
  if (proc.env.TUBEFORGE_DATA) return proc.env.TUBEFORGE_DATA;
  try {
    const portable = join(dirname(Deno.execPath()), "data");
    if (existsSync(portable)) return portable;
  } catch { /* non-file exec context: fall through */ }
  return join(homedir(), ".tubeforge-lite");
}

async function main(): Promise<number> {
  const args = [...Deno.args];
  const dataDir = resolveDataDir(flagValue(args, "--data"));
  proc.env.TUBEFORGE_DATA = dataDir;
  if (!proc.env.TUBEFORGE_DOWNLOADS) proc.env.TUBEFORGE_DOWNLOADS = join(dataDir, "downloads");

  // Extract the bundled aria2c BEFORE the engine resolves tools (PATH prepend).
  const { ensureBundledAria2 } = await import("./aria2.ts");
  const aria = ensureBundledAria2(dataDir);

  const cmd = args[0] ?? "serve";
  if (cmd === "version" || cmd === "--version" || cmd === "-V") {
    console.log(`tubeforge-lite ${VERSION} (deno ${Deno.version.deno})`);
    return 0;
  }
  if (cmd === "doctor") {
    const { healthReport } = await import("../shared/health.ts");
    const { aria2BundleInfo } = await import("./aria2.ts");
    const rep = await healthReport(`deno ${Deno.version.deno} (TubeForge Lite)`);
    console.log(JSON.stringify({ ...rep, aria2Bundle: aria2BundleInfo(), aria2Extract: aria, dataDir }, null, 2));
    return 0;
  }
  if (cmd === "serve" || cmd === "ui") {
    const { serve } = await import("./server.ts");
    serve({
      port: Number(flagValue(args, "--port") ?? proc.env.TUBEFORGE_PORT ?? 8484) || 8484,
      open: !args.includes("--no-open"),
    });
    return 0;
  }
  // anything else: treat leading non-flag args as URLs → headless CLI
  const { runCli } = await import("./cli.ts");
  return await runCli(args, VERSION);
}

if (import.meta.main) {
  const code = await main();
  if (code !== 0) proc.exitCode = code;
}
