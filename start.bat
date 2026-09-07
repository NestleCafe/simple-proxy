@echo off
rem Switch console to UTF-8 so Chinese log output from node renders correctly
chcp 65001 >nul

rem Switch to the directory where this script resides
cd /d "%~dp0"

echo ========================================
echo   trae-proxy reverse proxy launcher
echo ========================================
echo.

rem Check Node.js is installed
where node >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Node.js not found. Please install Node.js first.
    pause
    exit /b 1
)

echo Starting proxy server...
echo Press Ctrl+C to stop.
echo.
node proxy-server.js

echo.
echo Proxy server stopped.
pause