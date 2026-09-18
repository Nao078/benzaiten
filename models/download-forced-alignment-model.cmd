@echo off
setlocal

set "script_dir=%~dp0"
set "destination=%script_dir%wav2vec2-base-960h.onnx"
set "source=https://huggingface.co/facebook/wav2vec2-base-960h/resolve/6d2b9ffaac8aabc45934584ee608c5fb5ee34a4e/onnx/model.onnx"

if exist "%destination%" (
  echo Model already exists: %destination%
  exit /b 0
)

echo Downloading Wav2Vec2 Base 960h ONNX model...
PowerShell -NoProfile -ExecutionPolicy Bypass -Command ^
  "Start-BitsTransfer -Source '%source%' -Destination '%destination%'"
if errorlevel 1 (
  echo Failed to download Forced Alignment model.
  exit /b 1
)

echo Done: %destination%
