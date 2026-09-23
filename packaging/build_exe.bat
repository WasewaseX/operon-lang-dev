@echo off
REM Windows packaging path (run on a Windows machine with Python + PyInstaller):
REM   pip install pyinstaller
REM   packaging\build_exe.bat
cd /d "%~dp0.."
pyinstaller packaging/genomelab.spec --noconfirm
echo Output: dist\GenomeLab-Operon.exe
