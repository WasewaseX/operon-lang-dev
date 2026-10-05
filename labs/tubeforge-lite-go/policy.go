// TubeForge Lite (Go) — Operon policy lane. Port of src/policy.ts.
//
// A policy is an ordinary Operon script (see ../tubeforge-lite/operon/policies).
// It runs inside Operon's deny-by-default sandbox and emits `key = value`
// lines via promote(); the engine reads them here and enforces them on every
// enqueue and every transfer.
//
// Environment:
//   TUBEFORGE_POLICY  path to the .op policy file (unset = policy off)
//   TF_OPERON         path to the operon binary (default: PATH "operon")
//
// Semantics: policy enabled but operon missing / script error / non-zero exit
// => FAIL-CLOSED (all downloads refused). Deny-by-default, end to end.
package main

import (
	"math"
	"os"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
)

type PolicyState struct {
	Kind          string // "off" | "ok" | "error"
	Rules         *PolicyRules
	OperonVersion *string
	File          string
	Error         string
}

var (
	policyMu sync.Mutex
	policyC  *PolicyState
)

var (
	boolTrue           = map[string]bool{"1": true, "true": true, "yes": true, "on": true}
	knownReasonKeys    = map[string]bool{"reason_video": true, "reason_audio": true, "reason_playlist": true, "reason_domain": true, "reason_size": true}
	policyKeyRe        = regexp.MustCompile(`^[a-z_]{1,40}$`)
	quotedValueRe      = regexp.MustCompile(`^"(.*)"$`)
)

func truthy(v string) bool {
	return boolTrue[strings.ToLower(strings.TrimSpace(v))]
}

func splitList(v string) []string {
	out := []string{}
	for _, s := range strings.Split(v, ",") {
		t := strings.ToLower(strings.TrimSpace(s))
		if t != "" {
			out = append(out, t)
		}
	}
	return out
}

// parsePolicyOut parses the promote() output of a policy script into rules.
func parsePolicyOut(file, out string) *PolicyRules {
	r := &PolicyRules{
		File:          file,
		Name:          "policy",
		Version:       "0",
		AllowVideo:    true,
		AllowAudio:    true,
		AllowPlaylist: true,
		Reasons:       map[string]string{},
	}
	for _, rawLine := range strings.Split(out, "\n") {
		line := strings.TrimSpace(rawLine)
		eq := strings.Index(line, "=")
		if eq <= 0 {
			continue
		}
		key := strings.ToLower(strings.TrimSpace(line[:eq]))
		if !policyKeyRe.MatchString(key) {
			continue
		}
		val := strings.TrimSpace(line[eq+1:])
		if m := quotedValueRe.FindStringSubmatch(val); m != nil {
			val = m[1]
		}
		switch key {
		case "name":
			r.Name = clip(val, 60)
			if r.Name == "" {
				r.Name = "policy"
			}
		case "version":
			r.Version = clip(val, 20)
			if r.Version == "" {
				r.Version = "0"
			}
		case "allow_video":
			r.AllowVideo = truthy(val)
		case "allow_audio":
			r.AllowAudio = truthy(val)
		case "allow_playlist":
			r.AllowPlaylist = truthy(val)
		case "max_bytes":
			if f, err := strconv.ParseFloat(strings.TrimSpace(val), 64); err == nil && f > 0 {
				r.MaxBytes = int64(math.Floor(f))
			} else {
				r.MaxBytes = 0
			}
		case "max_quality":
			if f, err := strconv.ParseFloat(strings.TrimSpace(val), 64); err == nil && f > 0 {
				r.MaxQuality = int(math.Floor(f))
			} else {
				r.MaxQuality = 0
			}
		case "force_kind":
			if val == "audio" {
				r.ForceKind = "audio"
			} else {
				r.ForceKind = ""
			}
		case "deny_domains":
			r.DenyDomains = splitList(val)
		case "allow_domains":
			r.AllowDomains = splitList(val)
		default:
			if knownReasonKeys[key] {
				r.Reasons[key] = clip(val, 300)
			}
			// unknown keys ignored (forward compatible)
		}
	}
	return r
}

func runOperon(bin string, args []string) (bool, string, string) {
	c := exec.Command(bin, args...)
	c.SysProcAttr = sysProcAttrs()
	var out, errb strings.Builder
	c.Stdout = &out
	c.Stderr = &errb
	done := make(chan error, 1)
	if err := c.Start(); err != nil {
		return false, "", "cannot run operon (" + err.Error() + ")"
	}
	go func() { done <- c.Wait() }()
	select {
	case <-time.After(60 * time.Second):
		if c.Process != nil {
			go killTree(c.Process.Pid)
		}
		return false, "", "operon timed out after 60s"
	case err := <-done:
		if err != nil {
			msg := strings.TrimSpace(errb.String())
			if msg == "" {
				msg = "operon exited non-zero"
			}
			return false, "", clip(msg, 400)
		}
		return true, out.String(), ""
	}
}

// reloadPolicy forces a re-read on the next loadPolicy().
func reloadPolicy() PolicyState {
	policyMu.Lock()
	policyC = nil
	policyMu.Unlock()
	return loadPolicy()
}

func loadPolicy() PolicyState {
	policyMu.Lock()
	if policyC != nil {
		st := *policyC
		policyMu.Unlock()
		return st
	}
	policyMu.Unlock()

	file := strings.TrimSpace(os.Getenv("TUBEFORGE_POLICY"))
	var state PolicyState
	if file == "" {
		state = PolicyState{Kind: "off"}
	} else {
		bin := strings.TrimSpace(os.Getenv("TF_OPERON"))
		if bin == "" {
			bin = "operon"
		}
		var operonVersion *string
		if ok, out, _ := runOperon(bin, []string{"--version"}); ok {
			for _, line := range strings.Split(out, "\n") {
				t := strings.TrimSpace(line)
				if t != "" {
					operonVersion = &t
					break
				}
			}
		}
		if ok, out, errMsg := runOperon(bin, []string{"run", file}); !ok {
			state = PolicyState{Kind: "error", File: file, Error: errMsg}
		} else {
			state = PolicyState{Kind: "ok", Rules: parsePolicyOut(file, out), OperonVersion: operonVersion}
		}
	}

	policyMu.Lock()
	policyC = &state
	policyMu.Unlock()
	return state
}

// policyStatus builds the health/doctor payload.
func policyStatus() map[string]any {
	st := loadPolicy()
	switch st.Kind {
	case "off":
		return map[string]any{"enabled": false}
	case "error":
		return map[string]any{"enabled": true, "failClosed": true, "file": st.File, "error": st.Error}
	}
	r := st.Rules
	return map[string]any{
		"enabled":       true,
		"failClosed":    false,
		"file":          r.File,
		"name":          r.Name,
		"version":       r.Version,
		"allowVideo":    r.AllowVideo,
		"allowAudio":    r.AllowAudio,
		"allowPlaylist": r.AllowPlaylist,
		"maxBytes":      r.MaxBytes,
		"maxQuality":    r.MaxQuality,
		"operon":        st.OperonVersion,
	}
}

// gateEnqueue is the preventive gate at enqueue time: kind/playlist/domain
// rules + kind rewrite + quality ceiling. Returns ("", opts) on success or
// (errorMessage, opts) on denial.
func gateEnqueue(opts JobOptions, urls []string) (JobOptions, string) {
	st := loadPolicy()
	if st.Kind == "off" {
		return opts, ""
	}
	if st.Kind == "error" {
		return opts, "policy: " + st.File + " failed to load — refusing all downloads (fail-closed). " + st.Error
	}
	r := st.Rules
	deny := func(reason string) (JobOptions, string) {
		return opts, "policy (" + r.Name + "): " + reason
	}

	if len(urls) > 1 && !r.AllowPlaylist {
		return deny(reasonOr(r, "reason_playlist", "playlists and multi-URL batches are not allowed"))
	}
	for _, u := range urls {
		low := strings.ToLower(u)
		for _, d := range r.DenyDomains {
			if strings.Contains(low, d) {
				return deny(reasonOr(r, "reason_domain", "domain is denied by policy: "+d))
			}
		}
		if len(r.AllowDomains) > 0 {
			allowed := false
			for _, d := range r.AllowDomains {
				if strings.Contains(low, d) {
					allowed = true
					break
				}
			}
			if !allowed {
				return deny(reasonOr(r, "reason_domain", "domain is not on the policy whitelist"))
			}
		}
	}

	kind := opts.Kind
	if kind == "video" && !r.AllowVideo {
		if r.ForceKind == "audio" {
			kind = "audio"
		} else {
			return deny(reasonOr(r, "reason_video", "video downloads are disabled by policy"))
		}
	}
	if kind == "audio" && !r.AllowAudio {
		return deny(reasonOr(r, "reason_audio", "audio downloads are disabled by policy"))
	}

	quality := opts.Quality
	if kind == "video" && r.MaxQuality > 0 && (quality == 0 || quality > r.MaxQuality) {
		quality = r.MaxQuality
	}
	opts.Kind = kind
	opts.Quality = quality
	return opts, ""
}

func reasonOr(r *PolicyRules, key, def string) string {
	if v, ok := r.Reasons[key]; ok && v != "" {
		return v
	}
	return def
}

// gateSize is the reactive gate during transfer: abort once the total size is
// known and exceeds max_bytes. Returns the denial message or "" to proceed.
func gateSize(totalBytes int64) string {
	if totalBytes <= 0 {
		return ""
	}
	st := loadPolicy()
	if st.Kind != "ok" {
		return "" // error state already fail-closes at enqueue
	}
	r := st.Rules
	if r.MaxBytes > 0 && totalBytes > r.MaxBytes {
		if v, ok := r.Reasons["reason_size"]; ok && v != "" {
			return v
		}
		capMiB := int64(math.Round(float64(r.MaxBytes) / 1048576))
		gotMiB := int64(math.Round(float64(totalBytes) / 1048576))
		return "size cap exceeded: item is " + strconv.FormatInt(gotMiB, 10) + " MiB, policy allows " + strconv.FormatInt(capMiB, 10) + " MiB"
	}
	return ""
}
