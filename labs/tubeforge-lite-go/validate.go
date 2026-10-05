// TubeForge Lite (Go) — request validation. Ports shared/validate.ts.
package main

import (
        "net/url"
        "strconv"
        "strings"
)

// isSafeURL accepts only absolute http(s) URLs (<= 2048 chars, host present).
func isSafeURL(u string) bool {
        if len(u) == 0 || len(u) > 2048 {
                return false
        }
        p, err := url.Parse(u)
        if err != nil {
                return false
        }
        return (p.Scheme == "http" || p.Scheme == "https") && p.Host != ""
}

var qualities = map[int]bool{0: true, 360: true, 480: true, 720: true, 1080: true, 1440: true, 2160: true}
var containers = map[string]bool{"mp4": true, "mkv": true, "webm": true}
var audioFormats = map[string]bool{"mp3": true, "m4a": true, "opus": true, "flac": true, "wav": true}
var audioQualities = map[string]bool{"320": true, "256": true, "192": true, "128": true, "0": true}

// toNumber converts an untrusted JSON value to float64 the way JS Number()
// would for the shapes we accept (numbers, numeric strings).
func toNumber(v any) (float64, bool) {
        switch x := v.(type) {
        case float64:
                return x, true
        case string:
                if f, err := strconv.ParseFloat(strings.TrimSpace(x), 64); err == nil {
                        return f, true
                }
        case bool:
                return 0, false
        }
        return 0, false
}

func toBool(v any) (bool, bool) {
        b, ok := v.(bool)
        return b, ok
}

func toStr(v any) (string, bool) {
        s, ok := v.(string)
        return s, ok
}

// parseJobOptions builds a fully-typed JobOptions from an untrusted JSON body.
// Returns (opts, "") on success or (zero, error) on rejection.
func parseJobOptions(body map[string]any) (JobOptions, string) {
        if body == nil {
                return JobOptions{}, "Body must be a JSON object."
        }
        urlV, ok := body["url"].(string)
        if !ok || !isSafeURL(urlV) {
                return JobOptions{}, "A valid http(s) URL is required."
        }
        s := loadSettings()
        kind := "video"
        if k, ok := body["kind"].(string); ok && k == "audio" {
                kind = "audio"
        }
        quality := s.DefaultQuality
        if n, ok := toNumber(body["quality"]); ok && qualities[int(n)] {
                quality = int(n)
        }
        container := s.DefaultContainer
        if c, ok := body["container"].(string); ok && containers[c] {
                container = c
        }
        audioFormat := s.DefaultAudioFormat
        if a, ok := body["audioFormat"].(string); ok && audioFormats[a] {
                audioFormat = a
        }
        audioQuality := "320"
        if aq, ok := body["audioQuality"].(string); ok && audioQualities[aq] {
                audioQuality = aq
        }
        subLangs := s.SubLangs
        if sl, ok := toStr(body["subLangs"]); ok && strings.TrimSpace(sl) != "" {
                trimmed := strings.TrimSpace(sl)
                if len(trimmed) > 200 {
                        trimmed = trimmed[:200]
                }
                subLangs = trimmed
        }
        boolOr := func(key string, def bool) bool {
                if v, ok := toBool(body[key]); ok {
                        return v
                }
                return def
        }
        ariaConn := s.Aria2Connections
        if n, ok := toNumber(body["aria2Connections"]); ok {
                c := int(n)
                if c < 1 {
                        c = 1
                }
                if c > 16 {
                        c = 16
                }
                ariaConn = c
        }
        opts := JobOptions{
                URL:              urlV,
                Kind:             kind,
                Quality:          quality,
                Container:        container,
                AudioFormat:      audioFormat,
                AudioQuality:     audioQuality,
                EmbedThumbnail:   boolOr("embedThumbnail", s.EmbedThumbnail),
                EmbedMetadata:    boolOr("embedMetadata", s.EmbedMetadata),
                EmbedSubs:        boolOr("embedSubs", s.EmbedSubs),
                WriteSubs:        boolOr("writeSubs", s.WriteSubs),
                SubLangs:         subLangs,
                UseAria2:         boolOr("useAria2", s.UseAria2),
                Aria2Connections: ariaConn,
        }
        if t, ok := toStr(body["title"]); ok {
                if len(t) > 300 {
                        t = t[:300]
                }
                opts.Title = &t
        }
        if th, ok := toStr(body["thumbnail"]); ok {
                if len(th) > 1000 {
                        th = th[:1000]
                }
                opts.Thumbnail = &th
        }
        return opts, ""
}
