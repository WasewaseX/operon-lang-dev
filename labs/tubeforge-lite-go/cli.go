// TubeForge Lite (Go) — headless CLI mode. Port of src/cli.ts:
//   tubeforge-lite <url> [urls…] [--audio mp3|m4a|opus|flac|wav] [--quality N]
//     [--container mp4|mkv|webm] [--out DIR] [--subs "en,de"] [--embed-subs]
//     [--no-embed-thumb] [--no-embed-meta] [--no-aria2] [--aria2-conn N]
//     [--concurrency N] [--cookies FILE] [--quiet]
package main

import (
	"fmt"
	"os"
	"strings"
	"time"
)

type cliOpts struct {
	kind         string
	quality      *int
	container    string
	audioFormat  string
	out          string
	subs         string
	embedSubs    *bool
	embedThumb   *bool
	embedMeta    *bool
	useAria2     *bool
	aria2Conn    *int
	concurrency  *int
	cookies      string
	quiet        bool
}

func parseCli(args []string) ([]string, cliOpts) {
	urls := []string{}
	var o cliOpts
	val := func(i int) (string, bool) {
		if i+1 < len(args) && !strings.HasPrefix(args[i+1], "--") {
			return args[i+1], true
		}
		return "", false
	}
	for i := 0; i < len(args); i++ {
		a := args[i]
		v, has := val(i)
		switch a {
		case "--audio":
			o.kind = "audio"
			if has {
				o.audioFormat = v
				i++
			}
		case "--quality":
			if has {
				if n, err := atoiSafe(v); err == nil {
					o.quality = &n
					i++
				}
			}
		case "--container":
			if has {
				o.container = v
				i++
			}
		case "--out":
			if has {
				o.out = v
				i++
			}
		case "--subs":
			if has {
				o.subs = v
				i++
			}
		case "--embed-subs":
			t := true
			o.embedSubs = &t
		case "--no-embed-thumb":
			f := false
			o.embedThumb = &f
		case "--no-embed-meta":
			f := false
			o.embedMeta = &f
		case "--no-aria2":
			f := false
			o.useAria2 = &f
		case "--aria2-conn":
			if has {
				if n, err := atoiSafe(v); err == nil {
					o.aria2Conn = &n
					i++
				}
			}
		case "--concurrency":
			if has {
				if n, err := atoiSafe(v); err == nil {
					o.concurrency = &n
					i++
				}
			}
		case "--cookies":
			if has {
				o.cookies = v
				i++
			}
		case "--quiet", "-q":
			o.quiet = true
		default:
			if !strings.HasPrefix(a, "--") && isSafeURL(a) {
				urls = append(urls, a)
			}
		}
	}
	return urls, o
}

func atoiSafe(s string) (int, error) {
	n := 0
	neg := false
	s = strings.TrimSpace(s)
	if strings.HasPrefix(s, "-") {
		neg = true
		s = s[1:]
	}
	if s == "" {
		return 0, fmt.Errorf("empty")
	}
	for _, c := range s {
		if c < '0' || c > '9' {
			return 0, fmt.Errorf("not a number")
		}
		n = n*10 + int(c-'0')
	}
	if neg {
		n = -n
	}
	return n, nil
}

func isActive(status JobStatus) bool {
	return status == StatusQueued || status == StatusDownloading || status == StatusProcessing
}

func fmtBytesCLI(n int64) string {
	if n == 0 {
		return "0 B"
	}
	units := []string{"B", "KiB", "MiB", "GiB"}
	i := 0
	v := float64(n)
	for v >= 1024 && i < len(units)-1 {
		v /= 1024
		i++
	}
	if v >= 100 || i == 0 {
		return fmt.Sprintf("%.0f %s", v, units[i])
	}
	return fmt.Sprintf("%.1f %s", v, units[i])
}

func runCli(args []string, version string) int {
	urls, o := parseCli(args)
	if len(urls) == 0 {
		fmt.Printf(`tubeforge-lite %s — headless download mode
Usage: tubeforge-lite <url> [urls…] [options]
  --audio mp3|m4a|opus|flac|wav   audio extraction
  --quality 2160|1440|1080|720|480|360|0     (0 = best)
  --container mp4|mkv|webm        merge container (video)
  --out DIR                       download directory
  --subs "en,de" [--embed-subs]   subtitles
  --no-aria2 / --aria2-conn N     aria2 control
  --concurrency N                 parallel downloads (1..4)
  --cookies FILE                  cookies.txt for age-gated content
  --quiet                         progress only
  (policy: TUBEFORGE_POLICY + TF_OPERON env apply here too)
`, version)
		return 2
	}

	s := loadSettings()
	if o.concurrency != nil {
		c := *o.concurrency
		if c < 1 {
			c = 1
		}
		if c > 4 {
			c = 4
		}
		s.ConcurrentDownloads = c
	}
	if o.cookies != "" {
		s.CookiesFile = strPtr(expandHome(o.cookies))
	}
	if o.out != "" {
		s.DownloadDir = expandHome(o.out)
	}
	saveSettings(s)

	jobs := []Job{}
	for _, url := range urls {
		body := map[string]any{"url": url}
		if o.kind != "" {
			body["kind"] = o.kind
		}
		if o.quality != nil {
			body["quality"] = *o.quality
		}
		if o.container != "" {
			body["container"] = o.container
		}
		if o.audioFormat != "" {
			body["audioFormat"] = o.audioFormat
		}
		if o.subs != "" {
			body["subLangs"] = o.subs
			body["writeSubs"] = true
		}
		if o.embedSubs != nil {
			body["embedSubs"] = *o.embedSubs
		}
		if o.embedThumb != nil {
			body["embedThumbnail"] = *o.embedThumb
		}
		if o.embedMeta != nil {
			body["embedMetadata"] = *o.embedMeta
		}
		if o.useAria2 != nil {
			body["useAria2"] = *o.useAria2
		}
		if o.aria2Conn != nil {
			body["aria2Connections"] = *o.aria2Conn
		}
		opts, errMsg := parseJobOptions(body)
		if errMsg != "" {
			fmt.Fprintf(os.Stderr, "  ✗ %s: %s\n", url, errMsg)
			continue
		}
		gated, gateErr := gateEnqueue(opts, []string{url})
		if gateErr != "" {
			fmt.Fprintf(os.Stderr, "  ✗ %s: %s\n", url, gateErr)
			continue
		}
		job := st.enqueue(gated)
		jobs = append(jobs, job)
		if !o.quiet {
			fmt.Printf("  + queued [%d/%d] %s\n", len(jobs), len(urls), url)
		}
	}
	if len(jobs) == 0 {
		return 2
	}

	last := ""
	for {
		time.Sleep(400 * time.Millisecond)
		snap := st.snapshot()
		byID := map[string]Job{}
		for _, j := range snap.Jobs {
			byID[j.ID] = j
		}
		var first *Job
		for i := range jobs {
			if j, ok := byID[jobs[i].ID]; ok && isActive(j.Status) {
				jj := j
				first = &jj
				break
			}
		}
		if first != nil && !o.quiet {
			t := (*first).Opts.URL
			if (*first).Opts.Title != nil {
				t = *(*first).Opts.Title
			}
			t = clip(t, 52)
			tag := fmt.Sprintf("%d%%", int((*first).Progress.Percent+0.5))
			if (*first).Status == StatusProcessing {
				tag = "processing"
			}
			l := fmt.Sprintf("%-11s %-10s ETA %-8s %s", tag, (*first).Progress.Speed, (*first).Progress.ETA, t)
			fmt.Printf("\r%-*s", len([]rune(last))+2, l)
			last = l
		} else if first != nil && o.quiet {
			p := fmt.Sprintf("%d%%", int((*first).Progress.Percent+0.5))
			fmt.Printf("\r%s", p)
			last = p
		}
		if first == nil {
			fmt.Printf("\r%s\r", strings.Repeat(" ", len([]rune(last))+2))
			break
		}
	}

	failed := 0
	snap := st.snapshot()
	byID := map[string]Job{}
	for _, j := range snap.Jobs {
		byID[j.ID] = j
	}
	if !o.quiet {
		fmt.Println()
	}
	for _, x := range jobs {
		j, ok := byID[x.ID]
		if !ok {
			failed++
			continue
		}
		switch j.Status {
		case StatusCompleted:
			name := j.Opts.URL
			if j.FileName != nil {
				name = *j.FileName
			}
			path := "?"
			if j.FilePath != nil {
				path = *j.FilePath
			}
			var size int64
			if j.FileSize != nil {
				size = *j.FileSize
			}
			fmt.Printf("  ✓ %s  (%s) → %s\n", name, fmtBytesCLI(size), path)
		case StatusCanceled:
			fmt.Printf("  ✗ canceled %s\n", j.Opts.URL)
			failed++
		default:
			errMsg := "unknown error"
			if j.Error != nil {
				errMsg = *j.Error
			}
			fmt.Fprintf(os.Stderr, "  ✗ %s %s: %s\n", strings.ToUpper(string(j.Status)), j.Opts.URL, errMsg)
			failed++
			st.cancelJob(j.ID)
		}
	}
	if failed == 0 {
		return 0
	}
	return 1
}

func strPtr(s string) *string { return &s }
