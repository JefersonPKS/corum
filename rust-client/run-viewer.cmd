@echo off
setlocal
cd /d "%~dp0"

if "%~1"=="" (
    cargo run -p corum-viewer --bin corum-viewer
) else (
    cargo run -p corum-viewer --bin corum-viewer -- "%~1"
)

if errorlevel 1 pause
