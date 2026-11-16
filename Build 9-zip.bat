@echo off
rem Builds 9-zip for Windows with pnpm and puts the results in the Release folder.
rem Double-click this file. Needs Rust (MSVC), Node.js and pnpm.
setlocal
cd /d "%~dp0"

echo Installing packages...
call pnpm install || goto :failed

echo Building 9-zip (this takes a few minutes)...
call pnpm tauri build --bundles nsis || goto :failed

if not exist "Release" mkdir "Release"
copy /y "src-tauri\target\release\9-zip.exe" "Release\9-zip.exe" >nul || goto :failed
copy /y "src-tauri\target\release\bundle\nsis\*-setup.exe" "Release\" >nul || goto :failed

echo.
echo Done. The files are in the Release folder:
dir /b "Release"
pause
exit /b 0

:failed
echo.
echo The build failed. Please read the messages above.
pause
exit /b 1
