param([string]$CacheRoot='', [string]$MsysRoot='C:/msys64', [ValidateRange(1,16)][int]$Jobs=2, [switch]$Offline)
$ErrorActionPreference='Stop'
$repo=Split-Path $PSScriptRoot -Parent
if(!$CacheRoot){$CacheRoot=Join-Path $repo '.secpar-build-tools/windows-encoder'}
$CacheRoot=[IO.Path]::GetFullPath($CacheRoot)
$lock=Get-Content (Join-Path $PSScriptRoot 'encoder-deps.lock.json') -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Force $CacheRoot | Out-Null
function Run-Git($directory,$arguments){
 $directory=$directory.Replace('\','/')
 & git -c "safe.directory=$directory" -C $directory @arguments
 if($LASTEXITCODE){throw "Source command failed in $directory"}
}
foreach($source in $lock.sources){
 $directory=(Join-Path $CacheRoot $source.name).Replace('\','/')
 New-Item -ItemType Directory -Force $directory | Out-Null
 if(!(Test-Path (Join-Path $directory '.git'))){& git init -q $directory;if($LASTEXITCODE){throw 'Source cache initialization failed'}}
 $head=& git -c "safe.directory=$directory" -C $directory rev-parse --verify --quiet HEAD
 if($head -ne $source.commit){
  if($Offline){throw "The pinned source $($source.name) is not cached"}
  Run-Git $directory @('fetch','--filter=blob:none','--depth','1',$source.repo,$source.commit)
  if($source.name -eq 'amf-sunshine'){
   Run-Git $directory @('sparse-checkout','set','--no-cone','/amf/public/include/','/LICENSE.txt')
  }
  Run-Git $directory @('checkout','-q',$source.commit)
 }
}
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs=& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if(!$vs){throw 'Visual Studio C++ build tools are required'}
$environmentFile=Join-Path $CacheRoot 'msvc-env.cmd'
[IO.File]::WriteAllText($environmentFile,('@echo off'+"`r`n"+'call "'+$vs+'\Common7\Tools\VsDevCmd.bat" -no_logo -arch=x64'+"`r`n"+'if errorlevel 1 exit /b %errorlevel%'+"`r`n"+'set'+"`r`n"))
$environmentLines=& cmd.exe /d /c $environmentFile
if($LASTEXITCODE){throw 'Visual Studio environment setup failed'}
foreach($line in $environmentLines){$pair=$line.Split('=',2);if($pair.Length -eq 2 -and $pair[0]){[Environment]::SetEnvironmentVariable($pair[0],$pair[1],'Process')}}
$env:MSYS2_PATH_TYPE='inherit'
$prefix=Join-Path $CacheRoot 'ffmpeg-win-full'
& cmake -S (Join-Path $CacheRoot 'libvpl') -B (Join-Path $CacheRoot 'vpl-build') -G Ninja -DCMAKE_BUILD_TYPE=MinSizeRel "-DCMAKE_INSTALL_PREFIX=$prefix" -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTS=OFF -DBUILD_EXAMPLES=OFF -DINSTALL_EXAMPLES=OFF -DUSE_MSVC_STATIC_RUNTIME=ON -DCMAKE_POLICY_DEFAULT_CMP0091=NEW '-DCMAKE_CXX_FLAGS_MINSIZEREL=/O1 /Ob1 /DNDEBUG' '-DCMAKE_C_FLAGS_MINSIZEREL=/O1 /Ob1 /DNDEBUG'
if($LASTEXITCODE){throw 'Intel dispatcher configuration failed'}
& cmake --build (Join-Path $CacheRoot 'vpl-build') --parallel $Jobs
if($LASTEXITCODE){throw 'Intel dispatcher build failed'}
& cmake --install (Join-Path $CacheRoot 'vpl-build')
if($LASTEXITCODE){throw 'Intel dispatcher installation failed'}
$ffmpeg=(Join-Path $CacheRoot 'ffmpeg-sunshine').Replace('\','/')
foreach($patch in $lock.patches){
 $file=Join-Path (Join-Path $CacheRoot 'sunshine-build-deps/patches/FFmpeg/FFmpeg') $patch
 $previousPreference=$ErrorActionPreference
 try {
  $ErrorActionPreference='Continue'
  & git -c "safe.directory=$ffmpeg" -C $ffmpeg apply --check $file 2>$null
  $canApply=$LASTEXITCODE -eq 0
 } finally {$ErrorActionPreference=$previousPreference}
 if($canApply){Run-Git $ffmpeg @('apply',$file)}
 else {Run-Git $ffmpeg @('apply','--reverse','--check',$file)}
}
$env:SECPAR_ENCODER_DEPS_ROOT=$CacheRoot
$env:SECPAR_ENCODER_JOBS="$Jobs"
& (Join-Path $MsysRoot 'usr/bin/bash.exe') --noprofile --norc (Join-Path $PSScriptRoot 'build-windows-encoder-deps.sh')
if($LASTEXITCODE){throw 'FFmpeg encoder build failed'}
$licenses=Join-Path $prefix 'share/licenses/encoders'
New-Item -ItemType Directory -Force $licenses | Out-Null
foreach($notice in @(
 @('ffmpeg-sunshine/COPYING.LGPLv2.1','FFmpeg-LGPL-2.1.txt'),
 @('ffmpeg-sunshine/COPYING.LGPLv3','FFmpeg-LGPL-3.txt'),
 @('sunshine-build-deps/LICENSE','Sunshine-build-deps-MIT.txt'),
 @('libvpl/LICENSE','Intel-libvpl-MIT.txt'),
 @('amf-sunshine/LICENSE.txt','AMD-AMF-MIT.txt')
)){Copy-Item -LiteralPath (Join-Path $CacheRoot $notice[0]) -Destination (Join-Path $licenses $notice[1]) -Force}
$header=[IO.File]::ReadAllText((Join-Path $CacheRoot 'nv-sdk131/include/ffnvcodec/nvEncodeAPI.h'))
$end=$header.IndexOf('*/')
if($end -lt 0){throw 'NVIDIA license comment missing'}
[IO.File]::WriteAllText((Join-Path $licenses 'NVIDIA-header-notice.txt'),$header.Substring(0,$end+2))
foreach($sdk in @('nv-sdk11','nv-sdk12','nv-sdk13')){
 $header=[IO.File]::ReadAllText((Join-Path $CacheRoot "$sdk/include/ffnvcodec/nvEncodeAPI.h"))
 $end=$header.IndexOf('*/');if($end -lt 0){throw 'NVIDIA license comment missing'}
 [IO.File]::WriteAllText((Join-Path $licenses "NVIDIA-$sdk.txt"),$header.Substring(0,$end+2))
}
$header=[IO.File]::ReadAllText((Join-Path $repo 'host/src-tauri/native/nvenc/nvEncodeAPI.h'))
$end=$header.IndexOf('*/');if($end -lt 0){throw 'Native NVENC license comment missing'}
[IO.File]::WriteAllText((Join-Path $licenses 'NVIDIA-native-12.1.txt'),$header.Substring(0,$end+2))
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'encoder-deps.lock.json') -Destination (Join-Path $licenses 'sources.json') -Force
$env:SECPAR_FFMPEG_DIR=$prefix
Write-Output "SECPAR_FFMPEG_DIR=$prefix"
