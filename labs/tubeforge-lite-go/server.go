// TubeForge Lite (Go) — single-binary HTTP server. Port of src/server.ts:
// same /api/tf/* contract as v1.0.0, embedded vanilla UI at /, SSE stream at
// /api/tf/events, range-aware file serving.
package main

import (
        "encoding/json"
        "fmt"
        "io"
        "net"
        "net/http"
        "net/url"
        "os"
        "os/exec"
        "path/filepath"
        "regexp"
        "runtime"
        "strconv"
        "strings"
)

var mimeByExt = map[string]string{
        "html": "text/html; charset=utf-8",
        "js":   "text/javascript; charset=utf-8",
        "css":  "text/css; charset=utf-8",
        "json": "application/json",
        "svg":  "image/svg+xml",
        "png":  "image/png",
}

func writeJSON(w http.ResponseWriter, data any, status int) {
        b, err := json.Marshal(data)
        if err != nil {
                http.Error(w, `{"ok":false,"error":"marshal error"}`, 500)
                return
        }
        w.Header().Set("Content-Type", "application/json")
        w.WriteHeader(status)
        w.Write(b)
}

func writeOK(w http.ResponseWriter, data any) { writeJSON(w, data, 200) }

func readBody(w http.ResponseWriter, r *http.Request) map[string]any {
        var body map[string]any
        if err := json.NewDecoder(io.LimitReader(r.Body, 4<<20)).Decode(&body); err != nil {
                return nil
        }
        return body
}

func apiError(w http.ResponseWriter, msg string, status int) {
        writeJSON(w, map[string]any{"ok": false, "error": msg}, status)
}

// ---------------------------------------------------------------------------
// API — identical contract to TubeForge desktop (+ /api/tf/events SSE)
// ---------------------------------------------------------------------------

func apiRoute(w http.ResponseWriter, r *http.Request, pathname string) {
        method := r.Method

        switch {
        case pathname == "/api/tf/probe" && method == "POST":
                body := readBody(w, r)
                url, _ := body["url"].(string)
                if !isSafeURL(url) {
                        apiError(w, "A valid http(s) URL is required.", 400)
                        return
                }
                result, errMsg := probeUrl(url)
                if errMsg != "" {
                        apiError(w, errMsg, 422)
                        return
                }
                writeOK(w, map[string]any{"ok": true, "result": result})

        case pathname == "/api/tf/download" && method == "POST":
                body := readBody(w, r)
                if body == nil {
                        body = map[string]any{}
                }
                opts, errMsg := parseJobOptions(body)
                if errMsg != "" {
                        apiError(w, errMsg, 400)
                        return
                }
                urls := []string{}
                if raw, ok := body["entryUrls"].([]any); ok {
                        for i, u := range raw {
                                if i >= 500 {
                                        break
                                }
                                if us, ok := u.(string); ok && isSafeURL(us) {
                                        urls = append(urls, us)
                                }
                        }
                }
                if len(urls) == 0 {
                        urls = []string{opts.URL}
                }
                // Operon policy lane: preventive gate (kind/playlist/domain + rewrites)
                gated, gateErr := gateEnqueue(opts, urls)
                if gateErr != "" {
                        writeJSON(w, map[string]any{"ok": false, "error": gateErr, "policy": policyStatus()}, 403)
                        return
                }
                jobs := make([]Job, 0, len(urls))
                for _, u := range urls {
                        o := gated
                        o.URL = u
                        if len(urls) > 1 {
                                o.Title = nil
                                o.Thumbnail = nil
                        }
                        jobs = append(jobs, st.enqueue(o))
                }
                writeOK(w, map[string]any{"ok": true, "count": len(jobs), "jobs": jobs})

        case pathname == "/api/tf/queue" && method == "GET":
                writeOK(w, st.snapshot())

        case pathname == "/api/tf/events" && method == "GET":
                flusher, ok := w.(http.Flusher)
                if !ok {
                        apiError(w, "streaming unsupported", 500)
                        return
                }
                w.Header().Set("Content-Type", "text/event-stream")
                w.Header().Set("Cache-Control", "no-cache")
                w.Header().Set("Access-Control-Allow-Origin", "*")
                id, ch := st.subscribe()
                defer st.unsubscribe(id)
                // initial snapshot
                if b, err := json.Marshal(st.snapshot()); err == nil {
                        fmt.Fprintf(w, "data: %s\n\n", b)
                        flusher.Flush()
                }
                for {
                        select {
                        case payload, open := <-ch:
                                if !open {
                                        return
                                }
                                w.Write(payload)
                                flusher.Flush()
                        case <-r.Context().Done():
                                return
                        }
                }

        case pathname == "/api/tf/cancel" && method == "POST":
                body := readBody(w, r)
                id, _ := body["id"].(string)
                if id == "" {
                        apiError(w, "id required", 400)
                        return
                }
                writeOK(w, map[string]any{"ok": st.cancelJob(id)})

        case pathname == "/api/tf/retry" && method == "POST":
                body := readBody(w, r)
                id, _ := body["id"].(string)
                if id == "" {
                        apiError(w, "id required", 400)
                        return
                }
                if job := st.retryJob(id); job != nil {
                        writeOK(w, map[string]any{"ok": true, "job": job})
                } else {
                        apiError(w, "job not found", 404)
                }

        case pathname == "/api/tf/remove" && method == "POST":
                body := readBody(w, r)
                id, _ := body["id"].(string)
                if id == "" {
                        apiError(w, "id required", 400)
                        return
                }
                writeOK(w, map[string]any{"ok": st.removeJob(id)})

        case pathname == "/api/tf/files" && method == "GET":
                dir, files := listDownloads()
                writeOK(w, map[string]any{"ok": true, "dir": dir, "files": files})

        case pathname == "/api/tf/files" && method == "POST":
                body := readBody(w, r)
                if body != nil {
                        action, _ := body["action"].(string)
                        name, _ := body["name"].(string)
                        if action == "delete" && name != "" {
                                writeOK(w, map[string]any{"ok": deleteFile(name)})
                                return
                        }
                }
                apiError(w, "unknown action", 400)

        case pathname == "/api/tf/settings" && method == "GET":
                writeOK(w, map[string]any{"ok": true, "settings": loadSettings()})

        case pathname == "/api/tf/settings" && method == "POST":
                body := readBody(w, r)
                if body == nil {
                        apiError(w, "Invalid JSON body.", 400)
                        return
                }
                current := loadSettings()
                // merge top-level known keys (partial-body semantics of {...current, ...body})
                if raw, err := json.Marshal(body); err == nil {
                        var patch Settings
                        if json.Unmarshal(raw, &patch) == nil {
                                if _, present := body["downloadDir"]; present {
                                        current.DownloadDir = patch.DownloadDir
                                }
                                if _, present := body["filenameTemplate"]; present {
                                        current.FilenameTemplate = patch.FilenameTemplate
                                }
                                if _, present := body["concurrentDownloads"]; present {
                                        current.ConcurrentDownloads = patch.ConcurrentDownloads
                                }
                                if _, present := body["useAria2"]; present {
                                        current.UseAria2 = patch.UseAria2
                                }
                                if _, present := body["aria2Connections"]; present {
                                        current.Aria2Connections = patch.Aria2Connections
                                }
                                if _, present := body["defaultQuality"]; present {
                                        current.DefaultQuality = patch.DefaultQuality
                                }
                                if _, present := body["defaultContainer"]; present {
                                        current.DefaultContainer = patch.DefaultContainer
                                }
                                if _, present := body["defaultAudioFormat"]; present {
                                        current.DefaultAudioFormat = patch.DefaultAudioFormat
                                }
                                if _, present := body["embedThumbnail"]; present {
                                        current.EmbedThumbnail = patch.EmbedThumbnail
                                }
                                if _, present := body["embedMetadata"]; present {
                                        current.EmbedMetadata = patch.EmbedMetadata
                                }
                                if _, present := body["embedSubs"]; present {
                                        current.EmbedSubs = patch.EmbedSubs
                                }
                                if _, present := body["writeSubs"]; present {
                                        current.WriteSubs = patch.WriteSubs
                                }
                                if _, present := body["subLangs"]; present {
                                        current.SubLangs = patch.SubLangs
                                }
                                if _, present := body["cookiesFile"]; present {
                                        current.CookiesFile = patch.CookiesFile
                                }
                                if tp, present := body["toolPaths"].(map[string]any); present {
                                        if v, ok := tp["ytdlp"].(string); ok && v != "" {
                                                current.ToolPaths.Ytdlp = &v
                                        } else if _, ok := tp["ytdlp"]; ok {
                                                current.ToolPaths.Ytdlp = nil
                                        }
                                        if v, ok := tp["ffmpeg"].(string); ok && v != "" {
                                                current.ToolPaths.Ffmpeg = &v
                                        } else if _, ok := tp["ffmpeg"]; ok {
                                                current.ToolPaths.Ffmpeg = nil
                                        }
                                        if v, ok := tp["aria2c"].(string); ok && v != "" {
                                                current.ToolPaths.Aria2c = &v
                                        } else if _, ok := tp["aria2c"]; ok {
                                                current.ToolPaths.Aria2c = nil
                                        }
                                }
                        }
                }
                writeOK(w, map[string]any{"ok": true, "settings": saveSettings(current)})

        case pathname == "/api/tf/health" && method == "GET":
                rep := healthReport(runtimeLabel())
                writeOK(w, map[string]any{
                        "ok":                  true,
                        "tools":               rep.Tools,
                        "downloadDir":         rep.DownloadDir,
                        "downloadDirWritable": rep.DownloadDirWritable,
                        "platform":            rep.Platform,
                        "runtime":             rep.Runtime,
                        "app":                 "tubeforge-lite",
                        "aria2Bundle":         aria2BundleInfo(),
                        "policy":              policyStatus(),
                        "dataDir":             os.Getenv("TUBEFORGE_DATA"),
                })

        case pathname == "/api/tf/files/raw" && method == "GET":
                name := r.URL.Query().Get("name")
                p := safeResolve(name)
                if p == "" {
                        http.Error(w, "Not found", 404)
                        return
                }
                size, _ := fileSizeOf(p)
                ext := strings.TrimPrefix(filepath.Ext(name), ".")
                ct := mimeByExt[strings.ToLower(ext)]
                if ct == "" {
                        ct = "application/octet-stream"
                }
                w.Header().Set("Content-Type", ct)
                w.Header().Set("Accept-Ranges", "bytes")
                w.Header().Set("Content-Disposition", "attachment; filename*=UTF-8''"+urlPathEscape(name))
                f, err := os.Open(p)
                if err != nil {
                        http.Error(w, "Not found", 404)
                        return
                }
                defer f.Close()
                rangeHdr := r.Header.Get("Range")
                if rangeHdr != "" {
                        m := regexp.MustCompile(`bytes=(\d*)-(\d*)`).FindStringSubmatch(rangeHdr)
                        if m != nil {
                                start := int64(0)
                                if m[1] != "" {
                                        start, _ = strconv.ParseInt(m[1], 10, 64)
                                }
                                end := size - 1
                                if m[2] != "" {
                                        end, _ = strconv.ParseInt(m[2], 10, 64)
                                        if end > size-1 {
                                                end = size - 1
                                        }
                                }
                                if start >= size || start > end {
                                        w.Header().Set("Content-Range", fmt.Sprintf("bytes */%d", size))
                                        w.WriteHeader(416)
                                        return
                                }
                                w.Header().Set("Content-Range", fmt.Sprintf("bytes %d-%d/%d", start, end, size))
                                w.Header().Set("Content-Length", strconv.FormatInt(end-start+1, 10))
                                w.WriteHeader(206)
                                io.CopyN(w, io.NewSectionReader(f, start, end-start+1), end-start+1)
                                return
                        }
                }
                w.Header().Set("Content-Length", strconv.FormatInt(size, 10))
                io.Copy(w, f)

        default:
                apiError(w, "not found", 404)
        }
}

func urlPathEscape(s string) string {
        return url.PathEscape(s)
}

// ---------------------------------------------------------------------------
// server
// ---------------------------------------------------------------------------

func serve(port int, open bool) {
        // Operon policy lane: warm the cache once at boot (no-op while
        // TUBEFORGE_POLICY is unset).
        pol := loadPolicy()

        handler := func(w http.ResponseWriter, r *http.Request) {
                defer func() {
                        if rec := recover(); rec != nil {
                                apiError(w, fmt.Sprint(rec), 500)
                        }
                }()
                if strings.HasPrefix(r.URL.Path, "/api/") {
                        apiRoute(w, r, r.URL.Path)
                        return
                }
                if r.URL.Path == "/" || r.URL.Path == "/index.html" {
                        w.Header().Set("Content-Type", "text/html; charset=utf-8")
                        w.Header().Set("Cache-Control", "no-cache")
                        w.Write(uiHTML)
                        return
                }
                apiError(w, "not found", 404)
        }

        addr := fmt.Sprintf(":%d", port)
        ln, err := net.Listen("tcp", addr)
        if err != nil {
                fmt.Fprintln(os.Stderr, "serve failed:", err)
                os.Exit(1)
        }
        go func() {
                if err := http.Serve(ln, http.HandlerFunc(handler)); err != nil {
                        fmt.Fprintln(os.Stderr, "server error:", err)
                        os.Exit(1)
                }
        }()

        url := fmt.Sprintf("http://localhost:%d", port)
        b := aria2BundleInfo()
        ariaLine := "from PATH"
        if b.Bundled {
                ariaLine = fmt.Sprintf("bundled v%s (self-extracting)", b.Version)
        }
        polLine := "off (set TUBEFORGE_POLICY or --policy to enable)"
        switch pol.Kind {
        case "ok":
                ver := "?"
                if pol.OperonVersion != nil {
                        ver = *pol.OperonVersion
                }
                polLine = fmt.Sprintf("%s (operon %s, %s)", pol.Rules.Name, ver, pol.Rules.File)
        case "error":
                polLine = fmt.Sprintf("FAIL-CLOSED — %s refused to load", pol.File)
        }
        fmt.Println("\n  TubeForge Lite — one file, everything inside")
        fmt.Printf("  yt-dlp + ffmpeg: from PATH (or Settings) · aria2c: %s\n", ariaLine)
        fmt.Printf("  policy: %s\n", polLine)
        fmt.Printf("  → %s\n\n", url)
        if open {
                openBrowser(url)
        }
        // block forever (http.Serve runs in its own goroutine)
        select {}
}

func openBrowser(u string) {
        var cmd *exec.Cmd
        switch runtime.GOOS {
        case "windows":
                cmd = exec.Command("cmd", "/c", "start", "", u)
                cmd.SysProcAttr = sysProcAttrs()
        case "darwin":
                cmd = exec.Command("open", u)
        default:
                cmd = exec.Command("xdg-open", u)
        }
        _ = cmd.Start()
}
