# A second test copy of the dev build (for a second SoundCloud account): own data folders, own
# keyring entry, own WebView folder, no single-instance lock. Start it after `dev-parallel.ps1`
# has built and is running: it uses the same Vite (:1420) and the same debug binary.
param([string]$Profile = '2', [int]$Debug = 9224)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$built = Join-Path $root 'src-tauri\target\debug\sc-desk.exe'
$exe = Join-Path $root "src-tauri\target\debug\sc-desk-p$Profile.exe"
if (-not (Test-Path $built)) { throw "no debug build yet: run dev-parallel.ps1 first" }
# a copy: the running binary of a second profile must not lock the one cargo rebuilds
Copy-Item $built $exe -Force
$env:SCDESK_PROFILE = $Profile
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $env:LOCALAPPDATA "dev.scdesk.app.dev.webview$Profile"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Debug"
Start-Process -FilePath $exe -WorkingDirectory (Join-Path $root 'src-tauri')
