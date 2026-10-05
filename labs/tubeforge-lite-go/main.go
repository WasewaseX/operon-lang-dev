// TubeForge Lite — entrypoint (Go build, replaces the deno-compiled v1.0.0).
//
//   tubeforge-lite                      → server + UI (auto-opens browser)
//   tubeforge-lite serve [--port N] [--no-open] [--data DIR] [--policy FILE.op]
//   tubeforge-lite <url> [urls…] [opts] → headless CLI download
//   tubeforge-lite doctor               → tool health report
//   tubeforge-lite version
//
// ONE native executable; yt-dlp + ffmpeg come from PATH (or Settings
// overrides); aria2c is bundled INSIDE the exe and self-extracts on first run.
package main

import (
        "encoding/json"
        "fmt"
        "os"
        "path/filepath"
        "runtime"
        "strconv"
)

const VERSION = "1.1.0"

func flagValue(args []string, name string) string {
        for i, a := range args {
                if a == name && i+1 < len(args) {
                        return args[i+1]
                }
        }
        return ""
}

// resolveDataDir precedence: --data flag > $TUBEFORGE_DATA > portable ./data
// next to the executable > ~/.tubeforge-lite.
func resolveDataDir(flag string) string {
        if flag != "" {
                return flag
        }
        if v := os.Getenv("TUBEFORGE_DATA"); v != "" {
                return v
        }
        if exe, err := os.Executable(); err == nil {
                portable := filepath.Join(filepath.Dir(exe), "data")
                if st, err := os.Stat(portable); err == nil && st.IsDir() {
                        return portable
                }
        }
        home, _ := os.UserHomeDir()
        return filepath.Join(home, ".tubeforge-lite")
}

func main() {
        os.Exit(run(os.Args[1:]))
}

func run(args []string) int {
        dataDir := resolveDataDir(flagValue(args, "--data"))
        os.Setenv("TUBEFORGE_DATA", dataDir)
        if os.Getenv("TUBEFORGE_DOWNLOADS") == "" {
                os.Setenv("TUBEFORGE_DOWNLOADS", filepath.Join(dataDir, "downloads"))
        }

        // Extract the bundled aria2c BEFORE the engine resolves tools (PATH prepend).
        aria := ensureBundledAria2(dataDir)

        cmd := "serve"
        if len(args) > 0 {
                cmd = args[0]
        }

        switch cmd {
        case "version", "--version", "-V":
                fmt.Printf("tubeforge-lite %s (go %s)\n", VERSION, runtime.Version())
                return 0

        case "doctor":
                rep := healthReport(runtimeLabel())
                payload := map[string]any{
                        "tools":               rep.Tools,
                        "downloadDir":         rep.DownloadDir,
                        "downloadDirWritable": rep.DownloadDirWritable,
                        "platform":            rep.Platform,
                        "runtime":             rep.Runtime,
                        "aria2Bundle":         aria2BundleInfo(),
                        "policy":              policyStatus(),
                        "aria2Extract":        aria,
                        "dataDir":             dataDir,
                }
                b, _ := json.MarshalIndent(payload, "", "  ")
                fmt.Println(string(b))
                return 0

        case "serve", "ui":
                if pol := flagValue(args, "--policy"); pol != "" {
                        os.Setenv("TUBEFORGE_POLICY", pol)
                }
                port := 8484
                if v := flagValue(args, "--port"); v != "" {
                        if n, err := strconv.Atoi(v); err == nil && n > 0 {
                                port = n
                        }
                }
                if v := os.Getenv("TUBEFORGE_PORT"); v != "" && flagValue(args, "--port") == "" {
                        if n, err := strconv.Atoi(v); err == nil && n > 0 {
                                port = n
                        }
                }
                open := true
                for _, a := range args {
                        if a == "--no-open" {
                                open = false
                        }
                }
                serve(port, open)
                return 0
        }

        // anything else: treat leading non-flag args as URLs → headless CLI
        return runCli(args, VERSION)
}
