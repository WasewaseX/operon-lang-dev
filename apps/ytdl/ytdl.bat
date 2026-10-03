@echo off
rem ytdl — lightweight YouTube downloader: Operon runtime (~2.5 MB) + PATH engines.
rem Grants: exactly yt-dlp/ffmpeg/aria2c + the output directory (default-deny sandbox).
setlocal enabledelayedexpansion
set "HERE=%~dp0"
set "OP=%HERE%..\..\target\release\operon.exe"
if not exist "%OP%" set "OP=operon.exe"

set "OUT=downloads"
set "PREV="
:scan
if "%~1"=="" goto run
if "%PREV%"=="--out" set "OUT=%~1"
echo %~1| findstr /b "--out=" >nul && set "OUT=%~1"
set "PREV=%~1"
shift
goto scan
:run
if not exist "%OUT%\" mkdir "%OUT%"

"%OP%" run "%HERE%ytdl.op" ^
  --cell "%HERE%ytdl.cell" ^
  --allow-run yt-dlp --allow-run ffmpeg --allow-run aria2c ^
  --allow-read "%CD%" --allow-read "%OUT%" --allow-read "%HERE%" ^
  --allow-write "%OUT%" ^
  --fuel 20000000000 ^
  -- %*
exit /b %ERRORLEVEL%
