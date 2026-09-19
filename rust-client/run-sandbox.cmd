@echo off
setlocal
cd /d "%~dp0"

if "%~1"=="" (
    cargo run -p corum-viewer --bin corum-sandbox
) else (
    cargo run -p corum-viewer --bin corum-sandbox -- "%~1"
)

if errorlevel 1 pause
