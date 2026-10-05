// TubeForge Lite (Go) — health: tool resolution + versions, dir writability.
// Port of shared/health.ts. The v1.0.0 "deno" tool row is dropped: the Go
// build has no JS-runtime dependence and the chip was pure noise for users.
package main

import (
	"runtime"
	"strings"
	"time"
)

func toolVersion(cmd string, args []string) *string {
	if cmd == "" {
		return nil
	}
	r := runCapture(cmd, args, 15*time.Second)
	cand := r.Stdout
	if strings.TrimSpace(cand) == "" {
		cand = r.Stderr
	}
	for _, line := range strings.Split(cand, "\n") {
		t := strings.TrimSpace(line)
		if t != "" {
			if len(t) > 80 {
				t = t[:80]
			}
			return &t
		}
	}
	return nil
}

func healthReport(runtimeName string) HealthReport {
	s := loadSettings()
	ytdlp := resolveTool("yt-dlp", s.ToolPaths.Ytdlp)
	ffmpeg := resolveTool("ffmpeg", s.ToolPaths.Ffmpeg)
	aria2c := resolveTool("aria2c", s.ToolPaths.Aria2c)
	tools := []ToolInfo{
		{Name: "yt-dlp", Path: strPtrOrNil(ytdlp), Version: toolVersion(ytdlp, []string{"--version"}), OK: ytdlp != ""},
		{Name: "ffmpeg", Path: strPtrOrNil(ffmpeg), Version: toolVersion(ffmpeg, []string{"-version"}), OK: ffmpeg != ""},
		{Name: "aria2c", Path: strPtrOrNil(aria2c), Version: toolVersion(aria2c, []string{"--version"}), OK: aria2c != ""},
	}
	return HealthReport{
		Tools:               tools,
		DownloadDir:         s.DownloadDir,
		DownloadDirWritable: isWritableDir(s.DownloadDir),
		Platform:            runtime.GOOS + " " + runtime.GOARCH,
		Runtime:             runtimeName,
	}
}

func strPtrOrNil(s string) *string {
	if s == "" {
		return nil
	}
	return &s
}
