# Build the one Windows Secpar 4 app and its installer.
#
# This is the product: the Qt window (secpar.exe), the app library
# (secpar_app.dll), the background part (secpar-service.exe), the Sunshine engine
# under engine\ and the Moonlight stream component under stream\, with the Qt
# libraries and QML beside them. One installer, one application.
#
# Usage:
#   scripts\build-secpar4-windows.ps1 [-QtDir D:\Qt\6.12.0] [-TargetDir ...]
#                                     [-QtBuildDir ...] [-EngineDir ...] [-StreamDir ...]
#                                     [-OutDir ...] [-PackageDir ...] [-SkipBuild]
#
# The defaults match this build machine. The version comes from version.txt.
param(
    [string]$QtDir = "D:\Qt\6.12.0",
    [string]$TargetDir = "D:\Secpar-CI\s4\target",
    [string]$QtBuildDir = "D:\Secpar-CI\s4\qt-build",
    [string]$EngineDir = "D:\Secpar-CI\s4\windows-engine\engine",
    [string]$StreamDir = "D:\Secpar-CI\s4\windows-engine\stream",
    [string]$OutDir = "D:\Secpar-CI\s4\publish-secpar4-qt\windows",
    [string]$PackageDir = "",
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$version = (Get-Content -Raw (Join-Path $repo 'version.txt')).Trim()
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "version.txt must be X.Y.Z, found '$version'" }
if ($PackageDir -eq "") { $PackageDir = Join-Path $OutDir "Secpar-$version" }

foreach ($needed in @((Join-Path $EngineDir 'sunshine.exe'), (Join-Path $EngineDir 'assets'),
                      (Join-Path $StreamDir 'SecparStream.exe'), (Join-Path $QtDir 'bin\windeployqt.exe'))) {
    if (!(Test-Path -LiteralPath $needed)) { throw "Missing $needed" }
}

if (!$SkipBuild) {
    # The window's own code is Rust (crates/secpar-app) behind the Qt interface.
    $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (!(Test-Path -LiteralPath $cargo)) { $cargo = 'cargo' }
    $env:CARGO_TARGET_DIR = $TargetDir
    foreach ($crate in @('secpar-app', 'secpar-service')) {
        & $cargo build --release --manifest-path (Join-Path $repo "crates\$crate\Cargo.toml")
        if ($LASTEXITCODE) { throw "cargo build --release for $crate failed" }
    }
    & (Join-Path $PSScriptRoot 'build-desktop-qt.cmd') $QtDir $QtBuildDir (Join-Path $TargetDir 'release')
    if ($LASTEXITCODE) { throw 'The Qt window failed to build' }
}

# The package folder is what the installer wraps: the whole application, nothing
# outside it. It is rebuilt from scratch so a stale file cannot ride along.
Remove-Item -LiteralPath $PackageDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $PackageDir -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $QtBuildDir 'secpar.exe') -Destination $PackageDir
Copy-Item -LiteralPath (Join-Path $TargetDir 'release\secpar_app.dll') -Destination $PackageDir
Copy-Item -LiteralPath (Join-Path $TargetDir 'release\secpar-service.exe') -Destination $PackageDir
Copy-Item -LiteralPath $EngineDir -Destination (Join-Path $PackageDir 'engine') -Recurse
Copy-Item -LiteralPath $StreamDir -Destination (Join-Path $PackageDir 'stream') -Recurse

# Every Qt library, plugin and QML module the window can load, including the
# dialogs the file chooser and the tray need.
$qml = Join-Path $repo 'desktop-qt/qml'
$env:PATH = "$QtDir\bin;$env:PATH"
& (Join-Path $QtDir 'bin\windeployqt.exe') --release --no-translations --no-compiler-runtime --qmldir $qml (Join-Path $PackageDir 'secpar.exe')
if ($LASTEXITCODE) { throw 'windeployqt failed' }

foreach ($needed in @('secpar.exe', 'secpar_app.dll', 'secpar-service.exe',
                      'engine\sunshine.exe', 'stream\SecparStream.exe', 'platforms\qwindows.dll')) {
    if (!(Test-Path -LiteralPath (Join-Path $PackageDir $needed))) { throw "The package is missing $needed" }
}

# A quick proof that the assembled folder really starts, before it is wrapped.
$shot = Join-Path $env:TEMP "secpar-build-check-$([guid]::NewGuid().ToString('N')).png"
$env:QT_PLUGIN_PATH = Join-Path $PackageDir 'plugins'
$env:QML2_IMPORT_PATH = Join-Path $PackageDir 'qml'
& (Join-Path $PackageDir 'secpar.exe') --sample --channel 'secpar-build-check' --screenshot $shot --size 640x400 --tab 0
if ($LASTEXITCODE) { throw 'The assembled window did not start' }
Remove-Item -LiteralPath $shot -Force -ErrorAction SilentlyContinue

& (Join-Path $repo 'packaging\windows\build-installer.ps1') -PackageDir $PackageDir -OutDir $OutDir -ArchiveDir (Join-Path (Split-Path -Parent $OutDir) 'previous')
if ($LASTEXITCODE) { throw 'The installer build failed' }
$installer = Join-Path $OutDir 'Secpar-Windows-x64-setup.exe'
$hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Output "package:   $PackageDir"
Write-Output "installer: $installer"
Write-Output "size:      $((Get-Item -LiteralPath $installer).Length)"
Write-Output "sha256:    $hash"
