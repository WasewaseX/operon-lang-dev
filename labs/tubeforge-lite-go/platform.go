// TubeForge Lite (Go) — platform layer: tool resolution, run/spawn/kill,
// filesystem helpers, data dirs. Ports labs/tubeforge-lite/shared/platform.ts.
package main

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

// expandHome expands a leading "~" to the user home directory.
func expandHome(p string) string {
	if p == "" {
		return p
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return p
	}
	if p == "~" {
		return home
	}
	if strings.HasPrefix(p, "~/") || strings.HasPrefix(p, `~\`) {
		return filepath.Join(home, p[2:])
	}
	return p
}

// resolveTool resolves a tool binary to an absolute path: settings override
// first, then well-known dirs, then a real PATH walk. On Windows the plain
// candidate is tried first, then .exe/.cmd/.bat suffixes — so tools sitting on
// the user's PATH as yt-dlp.exe / ffmpeg.exe resolve correctly (this also
// fixes a v1.0.0 latent bug where the PATH walk never found .exe files).
func resolveTool(name string, override *string) string {
	var candidates []string
	if override != nil && *override != "" {
		candidates = append(candidates, expandHome(*override))
	}
	home, _ := os.UserHomeDir()
	extraDirs := []string{
		"/usr/local/bin",
		"/usr/bin",
		"/bin",
		"/usr/local/sbin",
		filepath.Join(home, ".local", "bin"),
		filepath.Join(home, ".deno", "bin"),
		filepath.Join(home, ".cargo", "bin"),
		filepath.Join(home, ".venv", "bin"),
		filepath.Join(home, "bin"),
		"/opt/homebrew/bin",
		"/opt/homebrew/sbin",
	}
	for _, d := range extraDirs {
		candidates = append(candidates, filepath.Join(d, name))
	}
	for _, d := range filepath.SplitList(os.Getenv("PATH")) {
		if d != "" {
			candidates = append(candidates, filepath.Join(d, name))
		}
	}
	exts := []string{""}
	if runtime.GOOS == "windows" {
		exts = []string{"", ".exe", ".cmd", ".bat"}
	}
	for _, c := range candidates {
		for _, ext := range exts {
			p := c + ext
			if fi, err := os.Stat(p); err == nil && fi.Mode().IsRegular() {
				return p
			}
		}
	}
	return ""
}

// RunResult mirrors shared/platform.ts RunResult.
type RunResult struct {
	Code   int
	Stdout string
	Stderr string
	Err    string
}

// runCapture runs a command with a hard timeout and captures both streams.
func runCapture(cmd string, args []string, timeout time.Duration) RunResult {
	if cmd == "" {
		return RunResult{Code: -1, Err: "binary not found"}
	}
	c := exec.Command(cmd, args...)
	c.SysProcAttr = sysProcAttrs()
	var out, errb strings.Builder
	c.Stdout = &out
	c.Stderr = &errb
	timer := time.AfterFunc(timeout, func() {
		if c.Process != nil {
			killTree(c.Process.Pid)
		}
	})
	defer timer.Stop()
	err := c.Run()
	r := RunResult{Stdout: out.String(), Stderr: errb.String()}
	if err != nil {
		if ee, ok := err.(*exec.ExitError); ok {
			r.Code = ee.ExitCode()
		} else {
			r.Code = -1
			r.Err = err.Error()
		}
	}
	return r
}

// spawnDetached starts a long-running process in its own group so it can be
// tree-killed later; returns the cmd plus piped stdout/stderr.
func spawnDetached(cmd string, args []string) (*exec.Cmd, io.ReadCloser, io.ReadCloser, error) {
	c := exec.Command(cmd, args...)
	c.SysProcAttr = sysProcAttrs()
	c.Stdin = nil
	stdout, err := c.StdoutPipe()
	if err != nil {
		return nil, nil, nil, err
	}
	stderr, err := c.StderrPipe()
	if err != nil {
		return nil, nil, nil, err
	}
	if err := c.Start(); err != nil {
		return nil, nil, nil, err
	}
	return c, stdout, stderr, nil
}

func ensureDir(p string) bool {
	return os.MkdirAll(p, 0o755) == nil
}

func isWritableDir(p string) bool {
	if err := os.MkdirAll(p, 0o755); err != nil {
		return false
	}
	probe := filepath.Join(p, ".tf-write-"+randomUUID())
	if err := os.Mkdir(probe, 0o755); err != nil {
		return false
	}
	os.Remove(probe)
	return true
}

func fileSizeOf(p string) (int64, bool) {
	st, err := os.Stat(p)
	if err != nil || !st.Mode().IsRegular() {
		return 0, false
	}
	return st.Size(), true
}

func defaultDataDir() string {
	if v := os.Getenv("TUBEFORGE_DATA"); v != "" {
		return v
	}
	cwd, _ := os.Getwd()
	return filepath.Join(cwd, "data")
}

func defaultDownloadDir() string {
	if v := os.Getenv("TUBEFORGE_DOWNLOADS"); v != "" {
		return v
	}
	cwd, _ := os.Getwd()
	return filepath.Join(cwd, "downloads")
}

func randomUUID() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return fmt.Sprintf("%032x", time.Now().UnixNano())
	}
	b[6] = (b[6] & 0x0f) | 0x40
	b[8] = (b[8] & 0x3f) | 0x80
	h := hex.EncodeToString(b)
	return h[0:8] + "-" + h[8:12] + "-" + h[12:16] + "-" + h[16:20] + "-" + h[20:32]
}

func runtimeLabel() string {
	return runtime.Version() + " (TubeForge Lite)"
}
