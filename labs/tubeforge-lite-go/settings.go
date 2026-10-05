// TubeForge Lite (Go) — settings store (settings.json in the data dir).
// Field names and defaults identical to shared/settings.ts so a v1.0.0 data
// directory keeps working after an upgrade.
package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
)

func defaultSettings() Settings {
	return Settings{
		DownloadDir:         defaultDownloadDir(),
		FilenameTemplate:    "%(title).100B [%(id)s].%(ext)s",
		ConcurrentDownloads: 2,
		UseAria2:            true,
		Aria2Connections:    8,
		DefaultQuality:      1080,
		DefaultContainer:    "mp4",
		DefaultAudioFormat:  "mp3",
		EmbedThumbnail:      true,
		EmbedMetadata:       true,
		EmbedSubs:           false,
		WriteSubs:           false,
		SubLangs:            "en.*",
		ToolPaths:           ToolPaths{},
	}
}

func settingsPath() string {
	return filepath.Join(defaultDataDir(), "settings.json")
}

var (
	settingsMu    sync.Mutex
	settingsCache *Settings
)

func loadSettings() Settings {
	settingsMu.Lock()
	defer settingsMu.Unlock()
	if settingsCache != nil {
		return *settingsCache
	}
	base := defaultSettings()
	raw, err := os.ReadFile(settingsPath())
	if err == nil {
		var loaded Settings
		if json.Unmarshal(raw, &loaded) == nil {
			// absent keys keep base values (Go unmarshal semantics)
			if loaded.DownloadDir == "" {
				loaded.DownloadDir = base.DownloadDir
			}
			if loaded.FilenameTemplate == "" {
				loaded.FilenameTemplate = base.FilenameTemplate
			}
			if loaded.DefaultContainer == "" {
				loaded.DefaultContainer = base.DefaultContainer
			}
			if loaded.DefaultAudioFormat == "" {
				loaded.DefaultAudioFormat = base.DefaultAudioFormat
			}
			if loaded.SubLangs == "" {
				loaded.SubLangs = base.SubLangs
			}
			if loaded.DefaultQuality == 0 {
				loaded.DefaultQuality = base.DefaultQuality
			}
			if loaded.Aria2Connections == 0 {
				loaded.Aria2Connections = base.Aria2Connections
			}
			if loaded.ConcurrentDownloads == 0 {
				loaded.ConcurrentDownloads = base.ConcurrentDownloads
			}
			settingsCache = &loaded
			return loaded
		}
		// corrupted settings -> fall through to defaults (save repairs it)
	}
	settingsCache = &base
	return base
}

func saveSettings(next Settings) Settings {
	base := defaultSettings()
	if next.DownloadDir == "" {
		next.DownloadDir = base.DownloadDir
	}
	if next.FilenameTemplate == "" {
		next.FilenameTemplate = base.FilenameTemplate
	}
	cd := next.ConcurrentDownloads
	if cd < 1 {
		cd = 1
	}
	if cd > 4 {
		cd = 4
	}
	next.ConcurrentDownloads = cd
	ac := next.Aria2Connections
	if ac < 1 {
		ac = 1
	}
	if ac > 16 {
		ac = 16
	}
	next.Aria2Connections = ac
	// deep-copy tool paths
	next.ToolPaths = ToolPaths{Ytdlp: next.ToolPaths.Ytdlp, Ffmpeg: next.ToolPaths.Ffmpeg, Aria2c: next.ToolPaths.Aria2c}
	settingsMu.Lock()
	settingsCache = &next
	settingsMu.Unlock()
	if err := os.MkdirAll(defaultDataDir(), 0o755); err == nil {
		if b, err := json.MarshalIndent(next, "", "  "); err == nil {
			_ = os.WriteFile(settingsPath(), b, 0o644)
		}
	}
	// read-only FS: settings stay in-memory
	return next
}
