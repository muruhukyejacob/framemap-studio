@echo off
REM Builds FrameMap Studio for Windows and (if NSIS is installed) the installer.
REM Needs: Rust from https://rustup.rs (with Visual Studio C++ Build Tools), optional NSIS from https://nsis.sourceforge.io
where cargo >nul 2>nul || (echo Rust is not installed. Get it from https://rustup.rs & exit /b 1)
cargo build --release || exit /b 1
if not exist dist mkdir dist
copy /Y target\release\framemap.exe "dist\FrameMap Studio.exe" >nul
echo Portable app: dist\FrameMap Studio.exe
set MAKENSIS=makensis
where makensis >nul 2>nul || set MAKENSIS="C:\Program Files (x86)\NSIS\makensis.exe"
for /f "tokens=3 delims= " %%v in ('findstr /b "version" Cargo.toml') do set VER=%%~v
%MAKENSIS% /DVERSION=%VER% packaging\windows\installer.nsi && echo Installer: dist\FrameMap-Studio-Setup.exe
