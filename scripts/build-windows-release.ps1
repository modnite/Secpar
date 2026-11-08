param(
    [string]$DesktopTarget = "",
    [string]$HostTarget = ""
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
if (!$DesktopTarget) { $DesktopTarget = Join-Path $repo 'desktop/target' }
if (!$HostTarget) { $HostTarget = Join-Path $repo 'host/src-tauri/target' }
$DesktopTarget = [IO.Path]::GetFullPath($DesktopTarget)
$HostTarget = [IO.Path]::GetFullPath($HostTarget)
$manifest = Get-Content -LiteralPath (Join-Path $repo 'desktop/Cargo.toml') -Raw
$version = [regex]::Match($manifest, '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
if (!$version) { throw 'A plain x.x.x release version is required' }
$hostManifest = Get-Content -LiteralPath (Join-Path $repo 'host/src-tauri/Cargo.toml') -Raw
if ($hostManifest -notmatch ('(?m)^version = "' + [regex]::Escape($version) + '"')) { throw 'Desktop and host versions differ' }
$config = Get-Content -LiteralPath (Join-Path $repo 'host/src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
if ($config.version -ne $version) { throw 'Installer version differs' }
$oldTarget=$env:CARGO_TARGET_DIR; $oldFlags=$env:RUSTFLAGS; $oldTag=$env:SECPAR_RELEASE_TAG
try {
    $env:SECPAR_RELEASE_TAG = "v$version"
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    $env:CARGO_PROFILE_RELEASE_DEBUG = '0'
    $env:CARGO_PROFILE_RELEASE_STRIP = 'symbols'
    $env:CARGO_TARGET_DIR = $DesktopTarget
    & cargo build --release --locked --manifest-path (Join-Path $repo 'desktop/Cargo.toml')
    if ($LASTEXITCODE) { throw 'Desktop release build failed' }
    $desktop = Join-Path $DesktopTarget 'release/secpar-desktop.exe'
    # One program: the installer wraps the desktop executable, which also runs the Windows service.
    New-Item -ItemType Directory -Path (Join-Path $HostTarget 'release') -Force | Out-Null
    Copy-Item -LiteralPath $desktop -Destination (Join-Path $HostTarget 'release/secpar-desktop.exe') -Force
    New-Item -ItemType Directory -Path (Join-Path $repo 'host/dist') -Force | Out-Null
    $env:CARGO_TARGET_DIR = $HostTarget
    & npm --prefix (Join-Path $repo 'host') run tauri bundle -- --config src-tauri/tauri.unified.conf.json --bundles nsis
    if ($LASTEXITCODE) { throw 'Unified release installer build failed' }
    $installer = Join-Path $HostTarget "release/bundle/nsis/Secpar_${version}_x64-setup.exe"
    if (!(Test-Path -LiteralPath $installer)) { throw 'Release installer is missing' }
    Write-Output $installer
} finally {
    $env:CARGO_TARGET_DIR=$oldTarget; $env:RUSTFLAGS=$oldFlags; $env:SECPAR_RELEASE_TAG=$oldTag
    Remove-Item Env:CARGO_PROFILE_RELEASE_DEBUG,Env:CARGO_PROFILE_RELEASE_STRIP -ErrorAction SilentlyContinue
}
