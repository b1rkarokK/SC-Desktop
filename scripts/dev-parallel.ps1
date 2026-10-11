# Dev version next to the stable copy: its own identifier (own data folder, own single-instance
# lock), so both can run at once and the stable one keeps playing. Needs Vite on :1420 (started by tauri dev).
# Seeds the dev data folder once from the stable one (database, settings), volume 3 %.
$ErrorActionPreference = 'Stop'
$src = Join-Path $env:APPDATA 'dev.scdesk.app'
$dst = Join-Path $env:APPDATA 'dev.scdesk.app.dev'
if (-not (Test-Path (Join-Path $dst 'config.json'))) {
    New-Item -ItemType Directory -Force $dst | Out-Null
    foreach ($f in 'cache.sqlite3', 'cache.sqlite3-wal', 'cache.sqlite3-shm', 'config.json') {
        if (Test-Path (Join-Path $src $f)) { Copy-Item (Join-Path $src $f) (Join-Path $dst $f) -Force }
    }
    $cfg = Get-Content (Join-Path $dst 'config.json') -Raw | ConvertFrom-Json
    $cfg.volume = 0.03
    $cfg | ConvertTo-Json -Depth 20 | Set-Content (Join-Path $dst 'config.json') -Encoding UTF8
}
Set-Location (Split-Path -Parent $PSScriptRoot)
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223'
npm run tauri dev -- --config src-tauri/tauri.dev.conf.json
