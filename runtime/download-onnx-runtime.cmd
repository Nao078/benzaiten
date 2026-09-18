@echo off
setlocal

set "VERSION=1.25.1"
set "ARCHIVE=%TEMP%\onnxruntime-win-x64-%VERSION%.zip"
set "EXTRACT_DIR=%TEMP%\benzaiten-onnxruntime-%VERSION%"
set "URL=https://github.com/microsoft/onnxruntime/releases/download/v%VERSION%/onnxruntime-win-x64-%VERSION%.zip"
set "DEST=%~dp0"

echo Downloading ONNX Runtime %VERSION%...
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference='Stop'; Invoke-WebRequest -Uri '%URL%' -OutFile '%ARCHIVE%'; Remove-Item -Recurse -Force '%EXTRACT_DIR%' -ErrorAction SilentlyContinue; Expand-Archive -Path '%ARCHIVE%' -DestinationPath '%EXTRACT_DIR%'; Copy-Item '%EXTRACT_DIR%\onnxruntime-win-x64-%VERSION%\lib\onnxruntime.dll' '%DEST%onnxruntime.dll' -Force; Copy-Item '%EXTRACT_DIR%\onnxruntime-win-x64-%VERSION%\lib\onnxruntime_providers_shared.dll' '%DEST%onnxruntime_providers_shared.dll' -Force"

if errorlevel 1 (
  echo Failed to download ONNX Runtime.
  exit /b 1
)

echo Runtime DLLs were saved in %DEST%
echo Copy both DLLs next to benzaiten.exe before running it.

