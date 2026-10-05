// TubeForge Lite (Go) — shared types. JSON shapes are byte-compatible with
// the v1.0.0 Deno build (labs/tubeforge-lite): the embedded UI talks to the
// same field names, and jobs.json / settings.json from a v1.0.0 data dir
// load unchanged.
package main

// Job / queue -------------------------------------------------------------

type JobStatus string

const (
        StatusQueued      JobStatus = "queued"
        StatusDownloading JobStatus = "downloading"
        StatusProcessing  JobStatus = "processing"
        StatusCompleted   JobStatus = "completed"
        StatusError       JobStatus = "error"
        StatusCanceled    JobStatus = "canceled"
)

type JobProgress struct {
        Percent         float64 `json:"percent"`
        Speed           string  `json:"speed"`
        ETA             string  `json:"eta"`
        DownloadedBytes int64   `json:"downloadedBytes"`
        TotalBytes      int64   `json:"totalBytes"`
}

type JobOptions struct {
        URL              string  `json:"url"`
        Kind             string  `json:"kind"` // "video" | "audio"
        Quality          int     `json:"quality"`
        Container        string  `json:"container"`
        AudioFormat      string  `json:"audioFormat"`
        AudioQuality     string  `json:"audioQuality"`
        EmbedThumbnail   bool    `json:"embedThumbnail"`
        EmbedMetadata    bool    `json:"embedMetadata"`
        EmbedSubs        bool    `json:"embedSubs"`
        WriteSubs        bool    `json:"writeSubs"`
        SubLangs         string  `json:"subLangs"`
        UseAria2         bool    `json:"useAria2"`
        Aria2Connections int     `json:"aria2Connections"`
        Title            *string `json:"title,omitempty"`
        Thumbnail        *string `json:"thumbnail,omitempty"`
}

type Job struct {
        ID        string      `json:"id"`
        CreatedAt int64       `json:"createdAt"`
        UpdatedAt int64       `json:"updatedAt"`
        Status    JobStatus   `json:"status"`
        Opts      JobOptions  `json:"opts"`
        Progress  JobProgress `json:"progress"`
        FilePath  *string     `json:"filePath,omitempty"`
        FileName  *string     `json:"fileName,omitempty"`
        FileSize  *int64      `json:"fileSize,omitempty"`
        Error     *string     `json:"error,omitempty"`
        LogTail   []string    `json:"logTail"`
}

type QueuedSnapshot struct {
        Jobs        []Job `json:"jobs"`
        ActiveCount int   `json:"activeCount"`
        Version     int64 `json:"version"`
}

// Probe -------------------------------------------------------------------

type ProbeFormat struct {
        FormatID       string   `json:"formatId"`
        Ext            string   `json:"ext"`
        Resolution     string   `json:"resolution"`
        Fps            *float64 `json:"fps,omitempty"`
        Vcodec         *string  `json:"vcodec,omitempty"`
        Acodec         *string  `json:"acodec,omitempty"`
        Filesize       *float64 `json:"filesize,omitempty"`
        FilesizeApprox *float64 `json:"filesizeApprox,omitempty"`
        Tbr            *float64 `json:"tbr,omitempty"`
        Note           *string  `json:"note,omitempty"`
}

type ProbeEntry struct {
        ID        string   `json:"id"`
        Title     string   `json:"title"`
        Duration  *float64 `json:"duration,omitempty"`
        URL       *string  `json:"url,omitempty"`
        Thumbnail *string  `json:"thumbnail,omitempty"`
}

type ProbeResult struct {
        Type              string       `json:"type"` // "video" | "playlist"
        ID                string       `json:"id"`
        Title             string       `json:"title"`
        Uploader          *string      `json:"uploader,omitempty"`
        Duration          *float64     `json:"duration,omitempty"`
        ViewCount         *float64     `json:"viewCount,omitempty"`
        UploadDate        *string      `json:"uploadDate,omitempty"`
        Thumbnail         *string      `json:"thumbnail,omitempty"`
        WebpageURL        string       `json:"webpageUrl"`
        Description       *string      `json:"description,omitempty"`
        Formats           []ProbeFormat `json:"formats"`
        Subtitles         []string     `json:"subtitles"`
        AutomaticCaptions []string     `json:"automaticCaptions"`
        Entries           []ProbeEntry `json:"entries"`
        PlaylistCount     *int         `json:"playlistCount,omitempty"`
        Extractor         *string      `json:"extractor,omitempty"`
}

// Settings ----------------------------------------------------------------

type ToolPaths struct {
        Ytdlp  *string `json:"ytdlp,omitempty"`
        Ffmpeg *string `json:"ffmpeg,omitempty"`
        Aria2c *string `json:"aria2c,omitempty"`
}

type Settings struct {
        DownloadDir         string    `json:"downloadDir"`
        FilenameTemplate    string    `json:"filenameTemplate"`
        ConcurrentDownloads int       `json:"concurrentDownloads"`
        UseAria2            bool      `json:"useAria2"`
        Aria2Connections    int       `json:"aria2Connections"`
        DefaultQuality      int       `json:"defaultQuality"`
        DefaultContainer    string    `json:"defaultContainer"`
        DefaultAudioFormat  string    `json:"defaultAudioFormat"`
        EmbedThumbnail      bool      `json:"embedThumbnail"`
        EmbedMetadata       bool      `json:"embedMetadata"`
        EmbedSubs           bool      `json:"embedSubs"`
        WriteSubs           bool      `json:"writeSubs"`
        SubLangs            string    `json:"subLangs"`
        CookiesFile         *string   `json:"cookiesFile,omitempty"`
        ToolPaths           ToolPaths `json:"toolPaths"`
}

// Health ------------------------------------------------------------------

type ToolInfo struct {
        Name    string  `json:"name"`
        Path    *string `json:"path"`
        Version *string `json:"version"`
        OK      bool    `json:"ok"`
}

type HealthReport struct {
        Tools               []ToolInfo `json:"tools"`
        DownloadDir         string     `json:"downloadDir"`
        DownloadDirWritable bool       `json:"downloadDirWritable"`
        Platform            string     `json:"platform"`
        Runtime             string     `json:"runtime"`
}

// Aria2 bundle ------------------------------------------------------------

type Aria2BundleInfo struct {
        Bundled  bool   `json:"bundled"`
        Platform string `json:"platform"`
        Version  string `json:"version"`
        Sha256   string `json:"sha256"`
}

type EnsureResult struct {
        Extracted bool   `json:"extracted"`
        Path      *string `json:"path"`
        Error     string `json:"error,omitempty"`
}

// Policy ------------------------------------------------------------------

type PolicyRules struct {
        File          string            `json:"file"`
        Name          string            `json:"name"`
        Version       string            `json:"version"`
        AllowVideo    bool              `json:"allowVideo"`
        AllowAudio    bool              `json:"allowAudio"`
        AllowPlaylist bool              `json:"allowPlaylist"`
        MaxBytes      int64             `json:"maxBytes"`
        MaxQuality    int               `json:"maxQuality"`
        ForceKind     string            `json:"-"` // "audio" or ""
        DenyDomains   []string          `json:"-"`
        AllowDomains  []string          `json:"-"`
        Reasons       map[string]string `json:"-"`
}
