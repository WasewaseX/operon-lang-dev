// TubeForge Lite (Go) — yt-dlp integration: probing, argument building,
// progress parsing. Faithful port of shared/ytdlp.ts.
package main

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"
)

func ytdlpBin() string {
	s := loadSettings()
	return resolveTool("yt-dlp", s.ToolPaths.Ytdlp)
}

func commonFlags(cookiesFile *string) []string {
	flags := []string{"--no-warnings", "--no-color", "--no-progress"}
	if cookiesFile != nil && *cookiesFile != "" {
		flags = append(flags, "--cookies", *cookiesFile)
	}
	return flags
}

func fmtSize(f ProbeFormat) *float64 {
	if f.Filesize != nil {
		return f.Filesize
	}
	return f.FilesizeApprox
}

var storyboardRe = regexp.MustCompile(`storyboard`)

// mapFormats maps raw yt-dlp format objects into the compact wire shape,
// best-first (video by height then tbr, audio by tbr), capped at 60 entries.
func mapFormats(raw []any) []ProbeFormat {
	out := []ProbeFormat{}
	for _, r := range raw {
		f, ok := r.(map[string]any)
		if !ok {
			continue
		}
		formatID := ""
		if v, ok := f["format_id"].(string); ok {
			formatID = v
		}
		if formatID == "" {
			continue
		}
		note, _ := f["format_note"].(string)
		if storyboardRe.MatchString(note) || strings.HasPrefix(formatID, "sb") {
			continue
		}
		pf := ProbeFormat{FormatID: formatID}
		if v, ok := f["ext"].(string); ok {
			pf.Ext = v
		}
		if v, ok := f["resolution"].(string); ok {
			pf.Resolution = v
		} else if h, ok := f["height"].(float64); ok {
			pf.Resolution = fmt.Sprintf("%dp", int(h))
		} else {
			pf.Resolution = "audio"
		}
		if v, ok := f["fps"].(float64); ok {
			pf.Fps = &v
		}
		vc, _ := f["vcodec"].(string)
		if vc == "" {
			vc = "none"
		}
		if vc != "none" {
			pf.Vcodec = &vc
		}
		ac, _ := f["acodec"].(string)
		if ac == "" {
			ac = "none"
		}
		if ac != "none" {
			pf.Acodec = &ac
		}
		if v, ok := f["filesize"].(float64); ok {
			pf.Filesize = &v
		}
		if v, ok := f["filesize_approx"].(float64); ok {
			pf.FilesizeApprox = &v
		}
		if v, ok := f["tbr"].(float64); ok {
			pf.Tbr = &v
		}
		if note != "" {
			pf.Note = &note
		}
		out = append(out, pf)
	}
	// best first: video by height then tbr, audio by tbr
	heightOf := func(p ProbeFormat) int {
		h, err := strconv.Atoi(p.Resolution)
		if err != nil {
			return 0
		}
		return h
	}
	weightOf := func(p ProbeFormat) float64 {
		if p.Tbr != nil {
			return *p.Tbr
		}
		if s := fmtSize(p); s != nil {
			return *s
		}
		return 0
	}
	for i := 1; i < len(out); i++ {
		for j := i; j > 0; j-- {
			ah, bh := heightOf(out[j-1]), heightOf(out[j])
			if ah < bh || (ah == bh && weightOf(out[j-1]) < weightOf(out[j])) {
				out[j-1], out[j] = out[j], out[j-1]
			} else {
				break
			}
		}
	}
	if len(out) > 60 {
		out = out[:60]
	}
	return out
}

func strOf(m map[string]any, key string) string {
	v, _ := m[key].(string)
	return v
}

func optStr(m map[string]any, key string) *string {
	v, ok := m[key].(string)
	if !ok || v == "" {
		return nil
	}
	return &v
}

func optFloat(m map[string]any, key string) *float64 {
	v, ok := m[key].(float64)
	if !ok {
		return nil
	}
	return &v
}

func bestThumb(m map[string]any) *string {
	if t := optStr(m, "thumbnail"); t != nil {
		return t
	}
	if thumbs, ok := m["thumbnails"].([]any); ok && len(thumbs) > 0 {
		if last, ok := thumbs[len(thumbs)-1].(map[string]any); ok {
			return optStr(last, "url")
		}
	}
	return nil
}

func lastLines(msg string, n int) string {
	var lines []string
	for _, l := range strings.Split(strings.TrimSpace(msg), "\n") {
		if strings.TrimSpace(l) != "" {
			lines = append(lines, strings.TrimSpace(l))
		}
	}
	if len(lines) > n {
		lines = lines[len(lines)-n:]
	}
	return strings.Join(lines, " | ")
}

// probeUrl probes a URL (video or playlist) with yt-dlp -J --flat-playlist.
func probeUrl(url string) (*ProbeResult, string) {
	bin := ytdlpBin()
	if bin == "" {
		return nil, "yt-dlp binary not found. Install it (pip install yt-dlp) or set the path in Settings."
	}
	s := loadSettings()
	args := append([]string{"-J", "--flat-playlist"}, commonFlags(s.CookiesFile)...)
	args = append(args, url)
	res := runCapture(bin, args, 90*time.Second)
	if strings.TrimSpace(res.Stdout) == "" {
		msg := res.Stderr
		if msg == "" {
			msg = res.Err
		}
		if strings.TrimSpace(msg) == "" {
			msg = "yt-dlp returned no data"
		}
		return nil, humanizeError(lastLines(msg, 3))
	}
	var raw map[string]any
	if err := json.Unmarshal([]byte(res.Stdout), &raw); err != nil {
		return nil, "Could not parse yt-dlp output (unexpected extractor response)."
	}
	if raw == nil {
		msg := res.Stderr
		if strings.TrimSpace(msg) == "" {
			msg = res.Err
		}
		if strings.TrimSpace(msg) == "" {
			msg = "no media found at this URL"
		}
		return nil, humanizeError(lastLines(msg, 3))
	}
	webpageURL := strOf(raw, "webpage_url")
	if webpageURL == "" {
		webpageURL = url
	}
	if t, ok := raw["_type"].(string); ok && t == "playlist" {
		entries := []ProbeEntry{}
		if entriesRaw, ok := raw["entries"].([]any); ok {
			for i, er := range entriesRaw {
				if i >= 500 {
					break
				}
				e, ok := er.(map[string]any)
				if !ok {
					e = map[string]any{}
				}
				id := strOf(e, "id")
				if id == "" {
					id = strconv.Itoa(i)
				}
				title := strOf(e, "title")
				if title == "" {
					title = fmt.Sprintf("Entry %d", i+1)
				}
				entry := ProbeEntry{ID: id, Title: title, Duration: optFloat(e, "duration")}
				if u := optStr(e, "url"); u != nil {
					entry.URL = u
				} else {
					entry.URL = optStr(e, "webpage_url")
				}
				entry.Thumbnail = bestThumb(e)
				entries = append(entries, entry)
			}
		}
		count := len(entries)
		if pc, ok := raw["playlist_count"].(float64); ok {
			c := int(pc)
			count = c
		}
		countPtr := count
		result := &ProbeResult{
			Type:              "playlist",
			ID:                strOf(raw, "id"),
			Title:             orDefault(strOf(raw, "title"), "Playlist"),
			Uploader:          optStr(raw, "uploader"),
			Thumbnail:         bestThumb(raw),
			WebpageURL:        webpageURL,
			Formats:           []ProbeFormat{},
			Subtitles:         []string{},
			AutomaticCaptions: []string{},
			Entries:           entries,
			PlaylistCount:     &countPtr,
			Extractor:         optStr(raw, "extractor_key"),
		}
		return result, ""
	}
	subs := []string{}
	if so, ok := raw["subtitles"].(map[string]any); ok {
		for k := range so {
			subs = append(subs, k)
		}
		sortStrings(subs)
	}
	auto := []string{}
	if ao, ok := raw["automatic_captions"].(map[string]any); ok {
		for k := range ao {
			auto = append(auto, k)
		}
		sortStrings(auto)
		if len(auto) > 30 {
			auto = auto[:30]
		}
	}
	formats := []ProbeFormat{}
	if fr, ok := raw["formats"].([]any); ok {
		formats = mapFormats(fr)
	}
	var desc *string
	if d, ok := raw["description"].(string); ok {
		d = clip(d, 400)
		desc = &d
	}
	result := &ProbeResult{
		Type:              "video",
		ID:                strOf(raw, "id"),
		Title:             orDefault(strOf(raw, "title"), "Video"),
		Uploader:          firstNonNil(optStr(raw, "uploader"), optStr(raw, "channel")),
		Duration:          optFloat(raw, "duration"),
		ViewCount:         optFloat(raw, "view_count"),
		UploadDate:        optStr(raw, "upload_date"),
		Thumbnail:         bestThumb(raw),
		WebpageURL:        webpageURL,
		Description:       desc,
		Formats:           formats,
		Subtitles:         subs,
		AutomaticCaptions: auto,
		Entries:           []ProbeEntry{},
		Extractor:         optStr(raw, "extractor_key"),
	}
	return result, ""
}

func sortStrings(s []string) {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
}

func orDefault(v, def string) string {
	if v == "" {
		return def
	}
	return v
}

func firstNonNil(vals ...*string) *string {
	for _, v := range vals {
		if v != nil {
			return v
		}
	}
	return nil
}

func clip(s string, n int) string {
	if len(s) > n {
		return s[:n]
	}
	return s
}

func humanizeError(msg string) string {
	m := strings.ToLower(msg)
	switch {
	case strings.Contains(m, "sign in to confirm"):
		return "YouTube asked for sign-in (bot check from this IP). Set a cookies file in Settings or try another network."
	case strings.Contains(m, "video unavailable"):
		return "Video unavailable (removed, private, or region-locked)."
	case strings.Contains(m, "not a valid url") || strings.Contains(m, "unsupported url"):
		return "Unsupported URL — paste a direct video/playlist link."
	case strings.Contains(m, "enoent") || strings.Contains(m, "not found"):
		return "Tool not found — check Settings → Tools."
	}
	if msg == "" {
		return "Unknown engine error."
	}
	return msg
}

// ---------------------------------------------------------------------------
// Download command construction
// ---------------------------------------------------------------------------

type downloadSpec struct {
	url              string
	kind             string
	quality          int
	container        string
	audioFormat      string
	audioQuality     string
	embedThumbnail   bool
	embedMetadata    bool
	embedSubs        bool
	writeSubs        bool
	subLangs         string
	useAria2         bool
	aria2Connections int
	downloadDir      string
	filenameTemplate string
	cookiesFile      *string
}

// buildDownloadArgs mirrors shared/ytdlp.ts buildDownloadArgs arg-for-arg so
// the engine log and downloader behavior stay identical.
func buildDownloadArgs(o downloadSpec) []string {
	args := []string{"--no-warnings", "--no-color", "--quiet", "--progress", "--newline"}
	if o.cookiesFile != nil && *o.cookiesFile != "" {
		args = append(args, "--cookies", *o.cookiesFile)
	}
	args = append(args,
		"--progress-template",
		"download:__TFPROG__|%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s|%(progress.downloaded_bytes)s|%(progress.total_bytes_estimate)s|%(progress.total_bytes)s",
	)
	args = append(args, "--print", "after_move:__TFDONE__|%(filepath)s")
	conc := o.aria2Connections
	if conc > 8 {
		conc = 8
	}
	if conc < 1 {
		conc = 1
	}
	args = append(args, "--concurrent-fragments", strconv.Itoa(conc))

	if o.kind == "audio" {
		args = append(args, "-x", "--audio-format", o.audioFormat)
		if o.audioQuality != "0" {
			args = append(args, "--audio-quality", o.audioQuality+"K")
		}
	} else {
		h := ""
		if o.quality > 0 {
			h = fmt.Sprintf("[height<=%d]", o.quality)
		}
		prefer, aprefer := "", ""
		if o.container == "mp4" {
			prefer = "[vcodec^=avc1]"
			aprefer = "[acodec^=mp4a]"
		}
		args = append(args,
			"-f",
			fmt.Sprintf("bv*%s%s+ba%s/bv*%s+ba/b%s/b", h, prefer, aprefer, h, h),
			"--merge-output-format", o.container,
		)
	}

	if o.embedThumbnail {
		args = append(args, "--embed-thumbnail")
	}
	if o.embedMetadata {
		args = append(args, "--embed-metadata")
	}
	if o.embedSubs {
		langs := o.subLangs
		if langs == "" {
			langs = "en.*"
		}
		args = append(args, "--embed-subs", "--sub-langs", langs, "--sub-format", "srt/vtt/best")
	}
	if o.writeSubs && !o.embedSubs {
		langs := o.subLangs
		if langs == "" {
			langs = "en.*"
		}
		args = append(args, "--write-subs", "--sub-langs", langs)
	}

	if o.useAria2 {
		args = append(args,
			"--downloader", "aria2c",
			"--downloader-args",
			fmt.Sprintf("aria2c:-x %d -s %d -k 1M --file-allocation=none --console-log-level=warn --summary-interval=0", o.aria2Connections, o.aria2Connections),
		)
	}

	args = append(args,
		"-o", filepath.Join(o.downloadDir, o.filenameTemplate),
		"--no-mtime",
		"--no-restrict-filenames",
		o.url,
	)
	return args
}

// ---------------------------------------------------------------------------
// Progress line parsing
// ---------------------------------------------------------------------------

type ProgressUpdate struct {
	Percent         float64
	Speed           string
	ETA             string
	DownloadedBytes int64
	TotalBytes      int64
}

const DonePrefix = "__TFDONE__|"

func parseFloatSafe(s string) float64 {
	f, err := strconv.ParseFloat(strings.TrimSpace(s), 64)
	if err != nil {
		return 0
	}
	return f
}

func parseIntSafe(s string) int64 {
	f, err := strconv.ParseFloat(strings.TrimSpace(s), 64)
	if err != nil {
		return 0
	}
	return int64(f)
}

// parseProgressLine parses the __TFPROG__| template lines.
func parseProgressLine(line string) (ProgressUpdate, bool) {
	if !strings.HasPrefix(line, "__TFPROG__|") {
		return ProgressUpdate{}, false
	}
	parts := strings.Split(line, "|")
	get := func(i int) string {
		if i < len(parts) {
			return parts[i]
		}
		return ""
	}
	percent := parseFloatSafe(strings.TrimSuffix(get(1), "%"))
	speed := strings.TrimSpace(get(2))
	if speed == "" {
		speed = "—"
	}
	eta := strings.TrimSpace(get(3))
	if eta == "Unknown" || eta == "" {
		eta = "—"
	}
	dl := parseIntSafe(get(4))
	tEst := parseIntSafe(get(5))
	total := parseIntSafe(get(6))
	if total == 0 {
		total = tEst
	}
	if percent < 0 {
		percent = 0
	}
	if percent > 100 {
		percent = 100
	}
	return ProgressUpdate{
		Percent:         percent,
		Speed:           speed,
		ETA:             eta,
		DownloadedBytes: dl,
		TotalBytes:      total,
	}, true
}
