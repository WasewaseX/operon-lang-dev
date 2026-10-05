//go:build windows

// TubeForge Lite (Go) — Windows process plumbing: hide-window + taskkill tree
// (mirrors shared/platform.ts killTree).
package main

import (
	"os/exec"
	"strconv"
	"syscall"
	"time"
)

const createNoWindow = 0x08000000

func sysProcAttrs() *syscall.SysProcAttr {
	return &syscall.SysProcAttr{HideWindow: true, CreationFlags: createNoWindow}
}

// killTree terminates the process tree via taskkill (the reliable way on
// Windows), with a forceful fallback after 4s.
func killTree(pid int) {
	if pid <= 0 {
		return
	}
	_ = exec.Command("taskkill", "/PID", strconv.Itoa(pid), "/T", "/F").Start()
	time.AfterFunc(4*time.Second, func() {
		_ = exec.Command("taskkill", "/PID", strconv.Itoa(pid), "/T", "/F").Start()
	})
}
