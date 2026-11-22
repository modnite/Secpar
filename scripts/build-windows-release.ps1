param(
    [string]$DesktopTarget = "",
    [string]$HostTarget = "",
    [string]$Features = "ffmpeg,gpu-isolation",
    [string]$TauriCli = "",
    [string]$EncoderDepsCacheRoot = "",
    [string]$BundleConfig = ""
)
$ErrorActionPreference = 'Stop'
$Features=($Features -split '[,\s]+' | Where-Object {$_}) -join ','
$repo = Split-Path $PSScriptRoot -Parent
if (!$DesktopTarget) { $DesktopTarget = Join-Path $repo 'desktop/target' }
if (!$HostTarget) { $HostTarget = Join-Path $repo 'host/src-tauri/target' }
$DesktopTarget = [IO.Path]::GetFullPath($DesktopTarget)
$HostTarget = [IO.Path]::GetFullPath($HostTarget)
$manifest = Get-Content -LiteralPath (Join-Path $repo 'desktop/Cargo.toml') -Raw
$version = [regex]::Match($manifest, '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+(-r[0-9]+)?)"').Groups[1].Value
if (!$version) { throw 'An x.x.x or x.x.x-rN release version is required' }
$hostManifest = Get-Content -LiteralPath (Join-Path $repo 'host/src-tauri/Cargo.toml') -Raw
if ($hostManifest -notmatch ('(?m)^version = "' + [regex]::Escape($version) + '"')) { throw 'Desktop and host versions differ' }
$config = Get-Content -LiteralPath (Join-Path $repo 'host/src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json
if ($config.version -ne $version) { throw 'Installer version differs' }
$oldTarget=$env:CARGO_TARGET_DIR; $oldFlags=$env:RUSTFLAGS; $oldTag=$env:SECPAR_RELEASE_TAG
$oldDebug=$env:CARGO_PROFILE_RELEASE_DEBUG; $oldStrip=$env:CARGO_PROFILE_RELEASE_STRIP
try {
    $bundleConfig=if ($BundleConfig) { [IO.Path]::GetFullPath($BundleConfig) } else { Join-Path $repo 'host/src-tauri/tauri.unified.conf.json' }
    $unified=Get-Content -LiteralPath $bundleConfig -Raw | ConvertFrom-Json
    $resources=@{}
    foreach($property in $unified.bundle.resources.PSObject.Properties){$resources[$property.Name]=$property.Value}
    $changed=$false
    # The virtual display: the signed Virtual Display Driver (MIT), pinned by release and checksum.
    $vdd=Join-Path $repo 'host/src-tauri/vdd'
    $vddZip=Join-Path ([IO.Path]::GetTempPath()) 'secpar-vdd-25.7.23.zip'
    $vddBase='https://github.com/VirtualDrivers/Virtual-Display-Driver'
    if (!(Test-Path $vddZip) -or (Get-FileHash $vddZip -Algorithm SHA256).Hash -ne 'E24210692B442B39AF763536330CE78B423F19342B7A7792C26DE3944E418B3A') {
        Invoke-WebRequest "$vddBase/releases/download/25.7.23/VirtualDisplayDriver-x86.Driver.Only.zip" -OutFile $vddZip -UseBasicParsing
    }
    if ((Get-FileHash $vddZip -Algorithm SHA256).Hash -ne 'E24210692B442B39AF763536330CE78B423F19342B7A7792C26DE3944E418B3A') { throw 'Virtual display driver checksum differs' }
    Remove-Item -Recurse -Force $vdd -ErrorAction SilentlyContinue
    $vddUnpacked=Join-Path ([IO.Path]::GetTempPath()) 'secpar-vdd-unpacked'
    Remove-Item -Recurse -Force $vddUnpacked -ErrorAction SilentlyContinue
    Expand-Archive -LiteralPath $vddZip -DestinationPath $vddUnpacked
    New-Item -ItemType Directory -Force $vdd | Out-Null
    foreach($f in 'MttVDD.inf','MttVDD.dll','mttvdd.cat'){Copy-Item -LiteralPath (Join-Path $vddUnpacked "VirtualDisplayDriver/$f") -Destination $vdd}
    $vddLicense=Join-Path $vdd 'LICENSE.txt'
    Invoke-WebRequest 'https://raw.githubusercontent.com/VirtualDrivers/Virtual-Display-Driver/25.7.23/LICENSE' -OutFile $vddLicense -UseBasicParsing
    if ((Get-FileHash $vddLicense -Algorithm SHA256).Hash -ne 'C2285C867DDC9B88F43B4F8FA2C77A9EEC66207166C1217B37353A2CCD62AC03') { throw 'Virtual display licence differs' }
    $resources['vdd/']='vdd/'
    $changed=$true
    if ($Features.Split(',') -contains 'ffmpeg') {
        if (!$env:SECPAR_FFMPEG_DIR -or !(Test-Path (Join-Path $env:SECPAR_FFMPEG_DIR 'share/licenses/encoders/sources.json'))) {
            & (Join-Path $PSScriptRoot 'build-windows-encoder-deps.ps1') -CacheRoot $EncoderDepsCacheRoot
        }
        if (!(Test-Path (Join-Path $env:SECPAR_FFMPEG_DIR 'lib/avcodec.lib'))) {throw 'FFmpeg encoder libraries are missing'}
        $licenseDirectory=Join-Path $env:SECPAR_FFMPEG_DIR 'share/licenses/encoders'
        $stagedLicenses=Join-Path $repo 'host/src-tauri/encoder-licenses'
        New-Item -ItemType Directory -Force $stagedLicenses | Out-Null
        foreach($notice in Get-ChildItem -LiteralPath $licenseDirectory -File){Copy-Item -LiteralPath $notice.FullName -Destination $stagedLicenses -Force}
        $resources['encoder-licenses/']='licenses/encoders/'
        $changed=$true
    }
    if ($changed) {
        $unified.bundle.resources=$resources
        $bundleConfig=Join-Path $repo 'host/src-tauri/tauri.encoder-build.conf.json'
        [IO.File]::WriteAllText($bundleConfig,($unified | ConvertTo-Json -Depth 20))
    }
    $env:SECPAR_RELEASE_TAG = "v$version"
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    $env:CARGO_PROFILE_RELEASE_DEBUG = '0'
    $env:CARGO_PROFILE_RELEASE_STRIP = 'symbols'
    $env:CARGO_TARGET_DIR = $DesktopTarget
    $buildArgs = @('build','--release','--locked','--manifest-path',(Join-Path $repo 'desktop/Cargo.toml'))
    if ($Features) { $buildArgs += @('--features',$Features) }
    & cargo @buildArgs
    if ($LASTEXITCODE) { throw 'Desktop release build failed' }
    $desktop = Join-Path $DesktopTarget 'release/secpar-desktop.exe'
    # One program: the installer wraps the desktop executable, which also runs the Windows service.
    New-Item -ItemType Directory -Path (Join-Path $HostTarget 'release') -Force | Out-Null
    Copy-Item -LiteralPath $desktop -Destination (Join-Path $HostTarget 'release/secpar-desktop.exe') -Force
    New-Item -ItemType Directory -Path (Join-Path $repo 'host/dist') -Force | Out-Null
    $env:CARGO_TARGET_DIR = $HostTarget
    if ($TauriCli) {
        Push-Location (Join-Path $repo 'host')
        try { & node $TauriCli bundle --config $bundleConfig --bundles nsis }
        finally { Pop-Location }
    } else {
        & npm --prefix (Join-Path $repo 'host') run tauri bundle -- --config $bundleConfig --bundles nsis
    }
    if ($LASTEXITCODE) { throw 'Unified release installer build failed' }
    $installer = Join-Path $HostTarget "release/bundle/nsis/Secpar_${version}_x64-setup.exe"
    if (!(Test-Path -LiteralPath $installer)) { throw 'Release installer is missing' }
    Write-Output $installer
} finally {
    $env:CARGO_TARGET_DIR=$oldTarget; $env:RUSTFLAGS=$oldFlags; $env:SECPAR_RELEASE_TAG=$oldTag
    $env:CARGO_PROFILE_RELEASE_DEBUG=$oldDebug; $env:CARGO_PROFILE_RELEASE_STRIP=$oldStrip
}
