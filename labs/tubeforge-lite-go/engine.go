// TubeForge Lite (Go) — job store + queue engine. Port of shared/engine.ts:
// in-memory queue with JSON persistence, SSE fan-out, process-tree
// cancellation, concurrency limiting, policy size gate.
package main

import (
        "bufio"
        "encoding/json"
        "fmt"
        "os"
        "os/exec"
        "path/filepath"
        "regexp"
        "strings"
        "sync"
        "time"
)

const (
        maxLog       = 40
        maxPersisted = 300
)

type runtimeJob struct {
        job      Job
        canceled bool
        child    *exec.Cmd
}

// copy returns a deep copy safe to hand to JSON encoders / other goroutines.
func (rj *runtimeJob) copy() Job {
        j := rj.job
        j.LogTail = append([]string{}, rj.job.LogTail...)
        return j
}

type store struct {
        mu        sync.Mutex
        jobs      map[string]*runtimeJob
        order     []string
        listeners map[int]chan []byte
        nextSub   int
        version   int64
        pumping   bool
        persistAr bool
        lastEmit  time.Time
}

var st = &store{
        jobs:      map[string]*runtimeJob{},
        listeners: map[int]chan []byte{},
}

// newestFirst sorts a job slice by createdAt descending (stable).
func newestFirst(jobs []Job) {
        for i := 1; i < len(jobs); i++ {
                for j := i; j > 0 && jobs[j-1].CreatedAt < jobs[j].CreatedAt; j-- {
                        jobs[j-1], jobs[j] = jobs[j], jobs[j-1]
                }
        }
}

// restore loads jobs.json once at startup; in-flight jobs become errors.
func restore() {
        raw, err := os.ReadFile(filepath.Join(defaultDataDir(), "jobs.json"))
        if err != nil {
                return
        }
        var arr []Job
        if json.Unmarshal(raw, &arr) != nil {
                return // corrupted store: start clean
        }
        if len(arr) > maxPersisted {
                arr = arr[len(arr)-maxPersisted:]
        }
        st.mu.Lock()
        defer st.mu.Unlock()
        for _, j := range arr {
                if j.ID == "" || j.Opts.URL == "" {
                        continue
                }
                status := j.Status
                switch status {
                case StatusQueued, StatusDownloading, StatusProcessing:
                        status = StatusError
                }
                if j.LogTail == nil {
                        j.LogTail = []string{}
                }
                if status == StatusError && j.Error == nil {
                        msg := "Interrupted by restart"
                        j.Error = &msg
                }
                j.Status = status
                st.jobs[j.ID] = &runtimeJob{job: j}
                st.order = append(st.order, j.ID)
        }
}

func init() { restore() }

// snapshot builds the public queue view (newest first).
func (s *store) snapshot() QueuedSnapshot {
        s.mu.Lock()
        defer s.mu.Unlock()
        jobs := make([]Job, 0, len(s.order))
        for _, id := range s.order {
                if rj, ok := s.jobs[id]; ok {
                        jobs = append(jobs, rj.copy())
                }
        }
        newestFirst(jobs)
        active := 0
        for _, rj := range s.jobs {
                switch rj.job.Status {
                case StatusQueued, StatusDownloading, StatusProcessing:
                        active++
                }
        }
        return QueuedSnapshot{Jobs: jobs, ActiveCount: active, Version: s.version}
}

func (s *store) subscribe() (int, <-chan []byte) {
        s.mu.Lock()
        defer s.mu.Unlock()
        id := s.nextSub
        s.nextSub++
        ch := make(chan []byte, 32)
        s.listeners[id] = ch
        return id, ch
}

func (s *store) unsubscribe(id int) {
        s.mu.Lock()
        defer s.mu.Unlock()
        if ch, ok := s.listeners[id]; ok {
                delete(s.listeners, id)
                close(ch)
        }
}

func (s *store) schedulePersistLocked() {
        if s.persistAr {
                return
        }
        s.persistAr = true
        time.AfterFunc(800*time.Millisecond, s.persistNow)
}

func (s *store) persistNow() {
        s.mu.Lock()
        s.persistAr = false
        arr := make([]Job, 0, len(s.order))
        for _, id := range s.order {
                if rj, ok := s.jobs[id]; ok {
                        arr = append(arr, rj.copy())
                }
        }
        s.mu.Unlock()
        if len(arr) > maxPersisted {
                arr = arr[len(arr)-maxPersisted:]
        }
        if err := os.MkdirAll(defaultDataDir(), 0o755); err != nil {
                return // read-only FS: memory-only mode
        }
        b, err := json.Marshal(arr)
        if err != nil {
                return
        }
        _ = os.WriteFile(filepath.Join(defaultDataDir(), "jobs.json"), b, 0o644)
}

// emit bumps the version, fans the snapshot out to SSE subscribers and
// schedules a debounced persist. Marshals once, outside the lock.
func (s *store) emit() {
        s.mu.Lock()
        s.version++
        jobs := make([]Job, 0, len(s.order))
        for _, id := range s.order {
                if rj, ok := s.jobs[id]; ok {
                        jobs = append(jobs, rj.copy())
                }
        }
        newestFirst(jobs)
        active := 0
        for _, rj := range s.jobs {
                switch rj.job.Status {
                case StatusQueued, StatusDownloading, StatusProcessing:
                        active++
                }
        }
        version := s.version
        s.schedulePersistLocked()
        s.mu.Unlock()

        snap := QueuedSnapshot{Jobs: jobs, ActiveCount: active, Version: version}
        b, err := json.Marshal(snap)
        if err != nil {
                return
        }
        payload := append(append([]byte("data: "), b...), []byte("\n\n")...)
        s.mu.Lock()
        targets := make([]chan []byte, 0, len(s.listeners))
        for _, ch := range s.listeners {
                targets = append(targets, ch)
        }
        s.mu.Unlock()
        for _, ch := range targets {
                select {
                case ch <- payload:
                default: // slow subscriber: drop, next emit catches it up
                }
        }
}

func (s *store) emitThrottled() {
        s.mu.Lock()
        if time.Since(s.lastEmit) < 200*time.Millisecond {
                s.mu.Unlock()
                return
        }
        s.lastEmit = time.Now()
        s.mu.Unlock()
        s.emit()
}

// enqueue adds a job and starts the pump.
func (s *store) enqueue(opts JobOptions) Job {
        now := time.Now().UnixMilli()
        rj := &runtimeJob{job: Job{
                ID:        randomUUID(),
                CreatedAt: now,
                UpdatedAt: now,
                Status:    StatusQueued,
                Opts:      opts,
                Progress:  JobProgress{Percent: 0, Speed: "—", ETA: "—"},
                LogTail:   []string{},
        }}
        s.mu.Lock()
        s.jobs[rj.job.ID] = rj
        s.order = append(s.order, rj.job.ID)
        s.mu.Unlock()
        s.emit()
        go s.pump()
        return rj.copy()
}

func (s *store) cancelJob(id string) bool {
        s.mu.Lock()
        rj, ok := s.jobs[id]
        if !ok {
                s.mu.Unlock()
                return false
        }
        switch rj.job.Status {
        case StatusCompleted, StatusCanceled, StatusError:
                s.mu.Unlock()
                return false
        }
        rj.canceled = true
        if rj.child != nil && rj.child.Process != nil {
                pid := rj.child.Process.Pid
                go killTree(pid)
                rj.child = nil
        }
        s.setStatusLocked(rj, StatusCanceled)
        s.mu.Unlock()
        s.emit()
        go s.pump()
        return true
}

func (s *store) retryJob(id string) *Job {
        s.mu.Lock()
        old, ok := s.jobs[id]
        if !ok {
                s.mu.Unlock()
                return nil
        }
        opts := old.job.Opts
        s.mu.Unlock()
        job := s.enqueue(opts)
        s.mu.Lock()
        delete(s.jobs, id)
        filtered := s.order[:0]
        for _, x := range s.order {
                if x != id {
                        filtered = append(filtered, x)
                }
        }
        s.order = filtered
        s.mu.Unlock()
        s.emit()
        return &job
}

func (s *store) removeJob(id string) bool {
        s.mu.Lock()
        rj, ok := s.jobs[id]
        if !ok {
                s.mu.Unlock()
                return false
        }
        active := rj.job.Status == StatusDownloading || rj.job.Status == StatusProcessing || rj.job.Status == StatusQueued
        s.mu.Unlock()
        if active {
                s.cancelJob(id)
        }
        s.mu.Lock()
        delete(s.jobs, id)
        filtered := s.order[:0]
        for _, x := range s.order {
                if x != id {
                        filtered = append(filtered, x)
                }
        }
        s.order = filtered
        s.mu.Unlock()
        s.emit()
        return true
}

func (s *store) pushLog(rj *runtimeJob, line string) {
        rj.job.LogTail = append(rj.job.LogTail, line)
        if len(rj.job.LogTail) > maxLog {
                rj.job.LogTail = rj.job.LogTail[len(rj.job.LogTail)-maxLog:]
        }
}

func (s *store) setStatusLocked(rj *runtimeJob, status JobStatus) {
        rj.job.Status = status
        rj.job.UpdatedAt = time.Now().UnixMilli()
}

var processingRe = regexp.MustCompile(`(?i)^\[(Merger|ExtractAudio|EmbedThumbnail|Fixup|Metadata|VideoRemuxer|SubtitlesConvertor)`)

// pump starts queued jobs while the configured concurrency allows it.
func (s *store) pump() {
        s.mu.Lock()
        if s.pumping {
                s.mu.Unlock()
                return
        }
        s.pumping = true
        s.mu.Unlock()
        defer func() {
                s.mu.Lock()
                s.pumping = false
                s.mu.Unlock()
        }()
        for {
                conc := loadSettings().ConcurrentDownloads
                if conc < 1 {
                        conc = 1
                }
                if conc > 4 {
                        conc = 4
                }
                s.mu.Lock()
                active := 0
                for _, rj := range s.jobs {
                        switch rj.job.Status {
                        case StatusDownloading, StatusProcessing:
                                active++
                        }
                }
                if active >= conc {
                        s.mu.Unlock()
                        return
                }
                var next *runtimeJob
                for _, id := range s.order {
                        if rj, ok := s.jobs[id]; ok && rj.job.Status == StatusQueued {
                                next = rj
                                break
                        }
                }
                s.mu.Unlock()
                if next == nil {
                        return
                }
                go s.startJob(next)
                time.Sleep(50 * time.Millisecond) // let the starter update counts
        }
}

// startJob spawns yt-dlp for one job and streams its progress into the store.
func (s *store) startJob(rj *runtimeJob) {
        sSet := loadSettings()
        ensureDir(sSet.DownloadDir)
        bin := ytdlpBin()
        if bin == "" {
                s.mu.Lock()
                msg := "yt-dlp binary not found. Install it or set the path in Settings."
                rj.job.Error = &msg
                s.setStatusLocked(rj, StatusError)
                s.mu.Unlock()
                s.emit()
                return
        }
        aria := ""
        if sSet.UseAria2 {
                aria = resolveTool("aria2c", sSet.ToolPaths.Aria2c)
                if aria == "" {
                        s.mu.Lock()
                        s.pushLog(rj, "[tubeforge] aria2c not found — falling back to yt-dlp native downloader")
                        s.mu.Unlock()
                }
        }
        ariaConns := sSet.Aria2Connections
        if rj.job.Opts.Aria2Connections > 0 {
                ariaConns = rj.job.Opts.Aria2Connections
        }
        spec := downloadSpec{
                url:              rj.job.Opts.URL,
                kind:             rj.job.Opts.Kind,
                quality:          rj.job.Opts.Quality,
                container:        rj.job.Opts.Container,
                audioFormat:      rj.job.Opts.AudioFormat,
                audioQuality:     rj.job.Opts.AudioQuality,
                embedThumbnail:   rj.job.Opts.EmbedThumbnail,
                embedMetadata:    rj.job.Opts.EmbedMetadata,
                embedSubs:        rj.job.Opts.EmbedSubs,
                writeSubs:        rj.job.Opts.WriteSubs,
                subLangs:         rj.job.Opts.SubLangs,
                useAria2:         sSet.UseAria2 && aria != "",
                aria2Connections: ariaConns,
                downloadDir:      sSet.DownloadDir,
                filenameTemplate: sSet.FilenameTemplate,
                cookiesFile:      sSet.CookiesFile,
        }
        args := buildDownloadArgs(spec)

        s.mu.Lock()
        s.setStatusLocked(rj, StatusDownloading)
        q := "best"
        if spec.quality > 0 {
                q = fmt.Sprintf("%dp", spec.quality)
        }
        s.pushLog(rj, fmt.Sprintf("[tubeforge] spawn: yt-dlp %s %s", rj.job.Opts.Kind, q))
        s.mu.Unlock()
        s.emit()

        cmd, stdout, stderr, err := spawnDetached(bin, args)
        if err != nil {
                s.mu.Lock()
                msg := "Failed to launch yt-dlp: " + err.Error()
                rj.job.Error = &msg
                s.setStatusLocked(rj, StatusError)
                s.mu.Unlock()
                s.emit()
                go s.pump()
                return
        }
        s.mu.Lock()
        rj.child = cmd
        s.mu.Unlock()

        doneResolved := false
        var (
                tailMu    sync.Mutex
                stderrBuf strings.Builder
        )

        // stdout: progress + done + notable lines
        go func() {
                sc := bufio.NewScanner(stdout)
                sc.Buffer(make([]byte, 0, 64*1024), 1024*1024)
                for sc.Scan() {
                        line := strings.TrimSpace(sc.Text())
                        if line == "" {
                                continue
                        }
                        if prog, ok := parseProgressLine(line); ok {
                                // reactive policy gate: abort once the total size is known
                                if prog.TotalBytes > 0 {
                                        if denied := gateSize(prog.TotalBytes); denied != "" {
                                                s.mu.Lock()
                                                rj.canceled = true
                                                if rj.child != nil && rj.child.Process != nil {
                                                        pid := rj.child.Process.Pid
                                                        go killTree(pid)
                                                        rj.child = nil
                                                }
                                                s.pushLog(rj, "[policy] "+denied)
                                                rj.job.Error = &denied
                                                s.setStatusLocked(rj, StatusError)
                                                s.mu.Unlock()
                                                s.emit()
                                                return
                                        }
                                }
                                s.mu.Lock()
                                rj.job.Progress = JobProgress{
                                        Percent:         prog.Percent,
                                        Speed:           prog.Speed,
                                        ETA:             prog.ETA,
                                        DownloadedBytes: prog.DownloadedBytes,
                                        TotalBytes:      prog.TotalBytes,
                                }
                                rj.job.UpdatedAt = time.Now().UnixMilli()
                                s.mu.Unlock()
                                s.emitThrottled()
                                continue
                        }
                        if strings.HasPrefix(line, DonePrefix) {
                                filePath := strings.TrimSpace(strings.TrimPrefix(line, DonePrefix))
                                doneResolved = true
                                s.mu.Lock()
                                size, _ := fileSizeOf(filePath)
                                rj.job.FilePath = &filePath
                                base := filepath.Base(filePath)
                                rj.job.FileName = &base
                                rj.job.FileSize = &size
                                rj.job.Progress.Percent = 100
                                rj.job.Progress.Speed = "—"
                                rj.job.Progress.ETA = "—"
                                s.setStatusLocked(rj, StatusCompleted)
                                s.pushLog(rj, "[tubeforge] done: "+filePath)
                                s.mu.Unlock()
                                s.emit()
                                continue
                        }
                        if processingRe.MatchString(line) {
                                s.mu.Lock()
                                if rj.job.Status != StatusProcessing {
                                        s.setStatusLocked(rj, StatusProcessing)
                                }
                                s.pushLog(rj, line)
                                s.mu.Unlock()
                                s.emit()
                                continue
                        }
                        if strings.HasPrefix(line, "[download] Destination") || strings.HasPrefix(line, "[info]") {
                                s.mu.Lock()
                                s.pushLog(rj, line)
                                s.mu.Unlock()
                        }
                }
        }()

        // stderr: raw engine chatter into the log tail
        go func() {
                sc := bufio.NewScanner(stderr)
                sc.Buffer(make([]byte, 0, 64*1024), 1024*1024)
                for sc.Scan() {
                        line := strings.TrimSpace(sc.Text())
                        if line == "" {
                                continue
                        }
                        tailMu.Lock()
                        stderrBuf.WriteString(line + "\n")
                        tailMu.Unlock()
                        s.mu.Lock()
                        s.pushLog(rj, clip(line, 300))
                        s.mu.Unlock()
                }
        }()

        waitErr := cmd.Wait()
        s.mu.Lock()
        rj.child = nil
        canceled := rj.canceled
        status := rj.job.Status
        s.mu.Unlock()
        if canceled {
                return // cancelJob already set the status
        }
        code := 0
        if waitErr != nil {
                if ee, ok := waitErr.(*exec.ExitError); ok {
                        code = ee.ExitCode()
                } else {
                        code = -1
                }
        }
        switch {
        case doneResolved && status != StatusCompleted:
                s.mu.Lock()
                if code == 0 {
                        s.setStatusLocked(rj, StatusCompleted)
                } else {
                        msg := fmt.Sprintf("yt-dlp exited with code %d", code)
                        rj.job.Error = &msg
                        s.setStatusLocked(rj, StatusError)
                }
                s.mu.Unlock()
                s.emit()
        case !doneResolved && code == 0:
                s.mu.Lock()
                rj.job.Progress.Percent = 100
                s.setStatusLocked(rj, StatusCompleted)
                s.mu.Unlock()
                s.emit()
        case !doneResolved:
                s.mu.Lock()
                lastErr := ""
                for _, l := range rj.job.LogTail {
                        if strings.HasPrefix(l, "ERROR") {
                                lastErr = l
                        }
                }
                tailMu.Lock()
                tail := strings.TrimSpace(stderrBuf.String())
                tailMu.Unlock()
                if lastErr == "" {
                        if tail != "" {
                                lastErr = tail
                        } else {
                                lastErr = fmt.Sprintf("yt-dlp exited with code %d", code)
                        }
                }
                msg := humanizeError(lastErr)
                rj.job.Error = &msg
                s.setStatusLocked(rj, StatusError)
                s.mu.Unlock()
                s.emit()
        }
        go s.pump()
}
