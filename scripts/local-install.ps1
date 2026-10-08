# Build a release locally and install it over the current SC Desk (test before publishing).
# Usage: powershell -ExecutionPolicy Bypass -File scripts\local-install.ps1
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

$key = Join-Path $env:USERPROFILE '.tauri\sc-desk.key'
if (-not (Test-Path $key)) { throw "Нет ключа подписи: $key" }
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''

npm run tauri build -- --bundles nsis
if ($LASTEXITCODE -ne 0) { throw "сборка не удалась ($LASTEXITCODE)" }

$setup = Get-ChildItem 'src-tauri\target\release\bundle\nsis\*-setup.exe' | Sort-Object LastWriteTime | Select-Object -Last 1
Write-Host "Установщик: $($setup.Name) ($([math]::Round($setup.Length / 1MB, 1)) МБ)"

Get-Process sc-desk -ErrorAction SilentlyContinue | Stop-Process -Force
$p = Start-Process -FilePath $setup.FullName -ArgumentList '/S' -PassThru -Wait
if ($p.ExitCode -ne 0) { throw "установщик вернул $($p.ExitCode)" }
Start-Process (Join-Path $env:LOCALAPPDATA 'SC Desk\sc-desk.exe')
Write-Host 'Установлено и запущено.'
