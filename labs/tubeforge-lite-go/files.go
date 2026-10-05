// TubeForge Lite (Go) — downloads dir listing + safe file serving.
// Port of shared/files.ts (traversal-proof).
package main

import (
	"os"
	"path/filepath"
	"sort"
	"strings"
)

type FileEntry struct {
	Name     string `json:"name"`
	Size     int64  `json:"size"`
	Modified int64  `json:"modified"` // ms since epoch
	IsDir    bool   `json:"isDir"`
}

func listDownloads() (string, []FileEntry) {
	s := loadSettings()
	ensureDir(s.DownloadDir)
	files := []FileEntry{}
	entries, err := os.ReadDir(s.DownloadDir)
	if err == nil {
		for _, e := range entries {
			info, err := e.Info()
			if err != nil {
				continue // raced delete
			}
			files = append(files, FileEntry{
				Name:     e.Name(),
				Size:     info.Size(),
				Modified: info.ModTime().UnixMilli(),
				IsDir:    info.IsDir(),
			})
		}
	}
	sort.SliceStable(files, func(i, j int) bool { return files[i].Modified > files[j].Modified })
	return s.DownloadDir, files
}

// safeResolve resolves a user-supplied file name inside the download dir —
// no traversal.
func safeResolve(name string) string {
	if name == "" || strings.ContainsAny(name, "/\\") || strings.HasPrefix(name, ".") {
		return ""
	}
	s := loadSettings()
	dir, err := filepath.Abs(s.DownloadDir)
	if err != nil {
		return ""
	}
	p, err := filepath.Abs(filepath.Join(dir, name))
	if err != nil {
		return ""
	}
	if p != dir && !strings.HasPrefix(p, dir+string(os.PathSeparator)) {
		return ""
	}
	fi, err := os.Stat(p)
	if err != nil || !fi.Mode().IsRegular() {
		return ""
	}
	return p
}

func deleteFile(name string) bool {
	p := safeResolve(name)
	if p == "" {
		return false
	}
	return os.Remove(p) == nil
}
