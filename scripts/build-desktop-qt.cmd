@echo off
rem Build the Secpar desktop app on Windows with MSVC and Qt.
rem Usage: scripts\build-desktop-qt.cmd <Qt prefix, e.g. D:\Qt\6.12.0> <build dir>
rem Needs Visual Studio Build Tools 2022 (C++), CMake and Ninja.
setlocal
if "%~2"=="" (echo Usage: %~nx0 ^<Qt prefix^> ^<build dir^> & exit /b 2)
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat" -no_logo -arch=x64 -host_arch=x64
if errorlevel 1 exit /b %errorlevel%
cmake -S "%~dp0..\desktop-qt" -B "%~2" -G Ninja -DCMAKE_BUILD_TYPE=Release "-DCMAKE_PREFIX_PATH=%~1"
if errorlevel 1 exit /b %errorlevel%
cmake --build "%~2"
