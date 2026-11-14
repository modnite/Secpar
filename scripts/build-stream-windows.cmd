@echo off
rem Build the pinned, patched Moonlight stream component on Windows with MSVC and Qt.
rem Usage: scripts\build-stream-windows.cmd <Qt prefix, e.g. D:\Qt\6.12.0> <source dir>
rem The source dir comes from integrations\gamestream\prepare.py moonlight-qt <dir>,
rem with Moonlight's prebuilt Windows libraries unpacked into <dir>\libs\windows
rem (moonlight-stream/moonlight-qt-deps release v19, windows-x64.zip,
rem SHA-256 65ff1bd439222e1e2750054f960dcd6fac46190027da16db7403e345c295f10f).
setlocal
if "%~2"=="" (echo Usage: %~nx0 ^<Qt prefix^> ^<source dir^> & exit /b 2)
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat" -no_logo -arch=x64 -host_arch=x64
if errorlevel 1 exit /b %errorlevel%
set "PATH=%~1\bin;%PATH%"
rem Compile several files at once; nmake itself runs one job at a time.
set CL=/MP
cd /d "%~2" || exit /b 1
qmake6 moonlight-qt.pro CONFIG+=release
if errorlevel 1 exit /b %errorlevel%
nmake /nologo release
