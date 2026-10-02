#!/usr/bin/env bash
# ytdl.sh — YouTube downloader, Bash comparison build (honest subset).
#
# Bash has no JSON parser and no concurrency primitives beyond job
# control, so this build deliberately ships the shell-native subset:
#   - doctor / info via yt-dlp --print (no JSON parsing anywhere)
#   - get as a checked passthrough with the same default flags
#   - queue sequential only (jobs accepted but capped at 1)
#   - selfcheck: not implementable (see APP-COMPARISON.md)
set -u

TIERS="2160 1440 1080 720 480 360 240 144"
AUDIO_KINDS="mp3 m4a opus flac wav"

die() { echo "$1"; exit "${2:-2}"; }

val() { # val NAME DEFAULT ARGS...
  local name="--$1" def="$2"; shift 2
  local prev=""
  for a in "$@"; do
    if [ "$prev" = "$name" ]; then echo "$a"; return; fi
    case "$a" in "$name"=*) echo "${a#*=}"; return ;; esac
    prev="$a"
  done
  echo "$def"
}

has_flag() {
  local name="--$1"; shift
  for a in "$@"; do [ "$a" = "$name" ] && return 0; done
  return 1
}

positional() { # everything that is not a flag or flag value
  local prev="" a
  for a in "$@"; do
    case "$prev" in
      --out|--quality|--format|--audio|--sub-langs|--template|--jobs|--max-attempts) : ;;
      *)
        case "$a" in
          -*) : ;;
          *) echo "$a" ;;
        esac
        ;;
    esac
    case "$a" in --out|--quality|--format|--audio|--sub-langs|--template|--jobs|--max-attempts) prev="$a" ;; *) prev="" ;; esac
  done
}

quality_expr() {
  if [ "$1" = "best" ]; then echo "bestvideo*+bestaudio/best"
  else echo "bestvideo[height<=$1]+bestaudio/best[height<=$1]/best"; fi
}

selection_args() { # builds the selector, printed for the human
  if [ -n "$FMT" ]; then echo "-f $FMT"
  elif [ -n "$AUDIO" ]; then echo "-f bestaudio/best -x --audio-format $AUDIO --audio-quality 0"
  else echo "-f $(quality_expr "$QUALITY")"; fi
}

tool_line() { "$1" "$2" 2>/dev/null | head -1; }

cmd_doctor() {
  local ytdlp ff aria2 ok=0
  ytdlp="$(tool_line yt-dlp --version)"
  ff="$(tool_line ffmpeg -version)"
  aria2="$(tool_line aria2c --version)"
  echo "ytdl doctor — probing PATH engines"
  if [ -n "$ytdlp" ]; then echo "  yt-dlp   $ytdlp  [required: present]"; ok=$((ok+1));
  else echo "  yt-dlp   MISSING  [required: pip install yt-dlp or brew install yt-dlp]"; fi
  if [ -n "$ff" ]; then echo "  ffmpeg   $ff  [required: present]"; ok=$((ok+1));
  else echo "  ffmpeg   MISSING  [required: apt/brew install ffmpeg]"; fi
  if [ -n "$aria2" ]; then echo "  aria2c   $aria2  [optional: multi-connection downloads]"
  else echo "  aria2c   not found  [optional: apt/brew install aria2 for 16-connection accel]"; fi
  if [ "$ok" = 2 ]; then echo "verdict: READY — bash runtime + PATH engines"; exit 0; fi
  echo "verdict: NOT READY — missing required engine(s)"; exit 1
}

cmd_info() {
  local url; url="$(positional "$@")"
  [ -z "$url" ] && die "error: info needs a URL"
  # shell-native: no JSON; yt-dlp --print does the field extraction
  echo "title: $(yt-dlp --no-playlist --print '%(title)s' "$url" 2>/dev/null)" \
    || die "yt-dlp failed" 1
  echo "id: $(yt-dlp --no-playlist --print '%(id)s' "$url" 2>/dev/null)"
  echo "duration: $(yt-dlp --no-playlist --print '%(duration)s' "$url" 2>/dev/null)s"
  echo "(bash build prints raw fields; per-format table needs JSON — see python/deno/operon builds)"
}

cmd_get() {
  local args=() url
  OUT="$(val out downloads "$@")"
  QUALITY="$(val quality best "$@")"
  AUDIO="$(val audio '' "$@")"
  FMT="$(val format '' "$@")"
  MAXA="$(val max-attempts 4 "$@")"
  if has_flag no-aria2 "$@"; then ARIA_ARGS=(); else ARIA_ARGS=(); fi
  url="$(positional "$@")"
  [ -z "$url" ] && die "error: get needs a URL"
  mkdir -p "$OUT"
  echo "ytdl[bash]: get $url"
  echo "  out=$OUT  selector: $(selection_args)"
  # selection args are re-derived for yt-dlp
  local sel=()
  if [ -n "$FMT" ]; then sel=(-f "$FMT")
  elif [ -n "$AUDIO" ]; then sel=(-f bestaudio/best -x --audio-format "$AUDIO" --audio-quality 0)
  else sel=(-f "$(quality_expr "$QUALITY")"); fi
  local attempt=1 rc=1
  while [ "$attempt" -le "$MAXA" ]; do
    yt-dlp --no-playlist --continue --retries 3 --fragment-retries 3 \
      --no-overwrites -P "$OUT" "${sel[@]}" "$url"
    rc=$?
    [ "$rc" = 0 ] && break
    attempt=$((attempt+1))
  done
  echo "  attempts: $attempt  result: $([ "$rc" = 0 ] && echo ok || echo FAIL)"
  exit "$rc"
}

cmd_queue() {
  local file="$1"; shift
  [ -z "$file" ] && die "error: queue needs a file (one URL per line, # comments)"
  OUT="$(val out downloads "$@")"
  QUALITY="$(val quality best "$@")"
  AUDIO="$(val audio '' "$@")"
  FMT="$(val format '' "$@")"
  MAXA="$(val max-attempts 4 "$@")"
  local failures=0 total=0 line rc
  while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"; line="${line%"${line##*[![:space:]]}"}"
    [ -z "$line" ] && continue
    case "$line" in \#*) continue ;; esac
    total=$((total+1))
    cmd_get "$line" --out "$OUT" --quality "$QUALITY" ${AUDIO:+--audio "$AUDIO"} \
      ${FMT:+--format "$FMT"} --max-attempts "$MAXA" >/dev/null 2>&1
    rc=$?
    if [ "$rc" = 0 ]; then echo "ok\t1\t$line"; else echo "FAIL\t$MAXA\t$line"; failures=$((failures+1)); fi
  done < "$file"
  echo "queue: $total items, $failures failed (workers: 1 — bash has no safe concurrency here)"
  [ "$failures" -gt 0 ] && exit 1
  exit 0
}

usage() {
  echo "ytdl — YouTube downloader, Bash comparison build (subset)"
  echo "cmds: doctor | info URL | get URL | queue FILE"
  echo "options: --out DIR --quality N --format EXPR --audio KIND --max-attempts N"
  echo "selfcheck is not implementable in pure bash (no JSON parser)."
  exit 2
}

OUT="downloads" QUALITY="best" AUDIO="" FMT=""

[ $# -eq 0 ] && usage
case "$1" in
  help|--help|-h) usage ;;
  doctor) shift; cmd_doctor "$@" ;;
  info) shift; cmd_info "$@" ;;
  get) shift; cmd_get "$@" ;;
  queue) shift; cmd_queue "$@" ;;
  selfcheck) die "selfcheck: not implementable in pure bash (no JSON parser)" 2 ;;
  *) echo "error: unknown subcommand $1"; usage ;;
esac
