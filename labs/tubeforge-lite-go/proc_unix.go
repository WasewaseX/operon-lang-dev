//go:build !windows

// TubeForge Lite (Go) — POSIX process plumbing: own process group so the
// engine can tree-kill a running yt-dlp (mirrors shared/platform.ts killTree).
package main

import (
	"syscall"
	"time"
)

func sysProcAttrs() *syscall.SysProcAttr {
	return &syscall.SysProcAttr{Setpgid: true}
}

// killTree terminates a detached process group (TERM, then KILL after 4s).
func killTree(pid int) {
	if pid <= 0 {
		return
	}
	_ = syscall.Kill(-pid, syscall.SIGTERM)
	time.AfterFunc(4*time.Second, func() {
		_ = syscall.Kill(-pid, syscall.SIGKILL)
	})
}
