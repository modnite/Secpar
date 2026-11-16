# Builds Secpar-Windows-x64-setup.exe from the Windows package folder.
#
# The folder holds the whole application: the window (secpar.exe), the app
# library, the background part, the Sunshine engine under engine\ and the
# Moonlight component under stream\, with the Qt libraries and QML beside them.
# Nothing is compiled here; this only packages what the release build produced.
#
# Usage: packaging\windows\build-installer.ps1 [-PackageDir <folder>] [-OutDir <folder>]
param(
    [string]$PackageDir = "D:\Secpar-CI\s4\publish-secpar4-qt\windows\Secpar-4.0.0",
    [string]$OutDir = "D:\Secpar-CI\s4\publish-secpar4-qt\windows",
    [string]$OutFile = "",
    [string]$ArchiveDir = "",
    [string]$Maker = "C:\Users\c\AppData\Local\tauri\NSIS\Bin\makensis.exe"
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$version = (Get-Content -Raw (Join-Path $repo 'version.txt')).Trim()
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "version.txt must be X.Y.Z, found '$version'" }
if (!(Test-Path -LiteralPath $PackageDir)) { throw "Package folder not found: $PackageDir" }
if (!(Test-Path -LiteralPath $Maker)) { throw "makensis not found: $Maker" }
foreach ($needed in @('secpar.exe', 'secpar_app.dll', 'secpar-service.exe', 'engine\sunshine.exe',
                      'stream\SecparStream.exe')) {
    if (!(Test-Path -LiteralPath (Join-Path $PackageDir $needed))) {
        throw "The package is missing $needed"
    }
}
$icon = Join-Path $repo 'host/src-tauri/icons/icon.ico'
if (!(Test-Path -LiteralPath $icon)) { throw "Icon not found: $icon" }
if ($OutFile -eq "") { $OutFile = Join-Path $OutDir 'Secpar-Windows-x64-setup.exe' }
New-Item -ItemType Directory -Path (Split-Path -Parent $OutFile) -Force | Out-Null
# An installer this build replaces is kept, not overwritten: a previous build is
# what someone falls back to, and its hash is what was published. The release
# layout keeps one previous\ beside windows\ and linux\, so the caller can say
# where that is; on its own this writes into $OutDir\previous.
if ($ArchiveDir -eq "") { $ArchiveDir = Join-Path $OutDir 'previous' }
if (Test-Path -LiteralPath $OutFile) {
    New-Item -ItemType Directory -Path $ArchiveDir -Force | Out-Null
    $existing = Get-FileHash -LiteralPath $OutFile -Algorithm SHA256
    $existingSize = (Get-Item -LiteralPath $OutFile).Length
    $stamp = (Get-Item -LiteralPath $OutFile).LastWriteTime.ToString('yyyyMMdd-HHmmss')
    $named = Join-Path $ArchiveDir "Secpar-Windows-x64-setup-$stamp-$($existing.Hash.Substring(0,8)).exe"
    Move-Item -LiteralPath $OutFile -Destination $named -Force
    Write-Output "kept the previous installer: $named ($existingSize bytes, $($existing.Hash.ToLowerInvariant()))"
}

$stage = Join-Path $env:TEMP ("secpar-installer-" + [guid]::NewGuid().ToString('N'))
try {
    # The payload is the package as it stands, under one name so the script does
    # not have to list several hundred files.
    $payload = Join-Path $stage 'payload'
    New-Item -ItemType Directory -Path $payload -Force | Out-Null
    Get-ChildItem -LiteralPath $PackageDir -Force | Copy-Item -Destination $payload -Recurse -Force
    $script = Join-Path $PSScriptRoot 'installer.nsi'
    & $Maker "/DAPP_VERSION=$version" "/DPAYLOAD=$payload" "/DOUTFILE=$OutFile" "/DICON=$icon" $script
    if ($LASTEXITCODE) { throw "makensis failed with $LASTEXITCODE" }
} finally {
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}

$file = Get-Item -LiteralPath $OutFile
$sha = (Get-FileHash -LiteralPath $OutFile -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Output "installer: $($file.FullName)"
Write-Output "size:      $($file.Length)"
Write-Output "sha256:    $sha"
