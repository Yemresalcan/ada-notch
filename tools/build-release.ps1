# Builds the release exe, embeds the app icon and copies it to dist\.
# Usage: powershell -ExecutionPolicy Bypass -File tools\build-release.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = 'F:\DevTools\rustup' }
if (-not $env:CARGO_HOME) { $env:CARGO_HOME = 'F:\DevTools\cargo' }
$env:Path = "$env:CARGO_HOME\bin;$env:Path"

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$dist = Join-Path $root 'dist'
$exe = Join-Path $dist 'ada.exe'
New-Item -ItemType Directory -Force $dist | Out-Null
Get-Process ada -ErrorAction SilentlyContinue | Where-Object Path -eq $exe | Stop-Process -Force
Copy-Item (Join-Path $root 'target\release\ada.exe') $exe -Force
# Absolute paths: the resource-update API does not resolve PowerShell's location.
python (Join-Path $root 'tools\set_icon.py') $exe (Join-Path $root 'assets\icon\ada.ico')
if ($LASTEXITCODE -ne 0) { throw 'icon embedding failed' }
Write-Host "dist\ada.exe ready"
