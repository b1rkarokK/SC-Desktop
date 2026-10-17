# Builds the proxy engine for the Android app (sing-box, a slimmed build: pure Go, no CGO) and puts it where Gradle
# packs it as a native library (android/app/src/main/jniLibs/arm64-v8a/libsingbox.so, about 42 MB, 14 MB in the APK).
# Needs Go (1.25 or later) and git; the sources and the caches go to a work folder (default F:\gobuild).
#   powershell -ExecutionPolicy Bypass -File scripts\build-singbox.ps1
param([string]$Work = 'F:\gobuild', [string]$Version = 'v1.14.3')

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Work | Out-Null
$src = Join-Path $Work 'src'
if (-not (Test-Path $src)) { git clone --depth 1 --branch $Version https://github.com/SagerNet/sing-box.git $src }
$env:GOPATH = Join-Path $Work 'gopath'; $env:GOCACHE = Join-Path $Work 'cache'; $env:GOMODCACHE = Join-Path $Work 'mod'
$env:GOFLAGS = '-mod=mod'; $env:GOMAXPROCS = '2'
$env:GOOS = 'android'; $env:GOARCH = 'arm64'; $env:CGO_ENABLED = '0'
$out = Join-Path $PSScriptRoot '..\android\app\src\main\jniLibs\arm64-v8a'
New-Item -ItemType Directory -Force -Path $out | Out-Null
Push-Location $src
# with_utls: Reality and the browser-like greeting; with_quic: Hysteria2 and TUIC
go build -p 2 -trimpath -ldflags '-s -w -buildid=' -tags 'with_utls,with_quic' -o (Join-Path $out 'libsingbox.so') ./cmd/sing-box
Pop-Location
Get-Item (Join-Path $out 'libsingbox.so') | ForEach-Object { "built: $($_.Length) bytes" }
