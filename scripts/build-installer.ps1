# Builds deck-host (release), bundles it as a Tauri sidecar, and builds the NSIS installer.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = 'C:\tmp\deck-target-release' }
Set-Location $root

cargo build -p deck-host --release -j 2
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$bin = Join-Path $root 'app\src-tauri\binaries'
New-Item -ItemType Directory -Force $bin | Out-Null
Copy-Item (Join-Path $env:CARGO_TARGET_DIR 'release\deck-host.exe') (Join-Path $bin 'deck-host-x86_64-pc-windows-msvc.exe') -Force

pnpm --dir app tauri build
if ($LASTEXITCODE -ne 0) { throw 'tauri build failed' }

$setup = Get-ChildItem (Join-Path $env:CARGO_TARGET_DIR 'release\bundle\nsis') -Filter '*-setup.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
Write-Host "Installer: $($setup.FullName) ($([math]::Round($setup.Length/1MB,1)) MB)"
