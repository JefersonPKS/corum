@echo off
rem Abre o Corum (Rust) em modo jogo. Argumentos opcionais: -Classe 1..5  -Mapa 604  -Dev
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0jogar.ps1" %*
if errorlevel 1 pause
