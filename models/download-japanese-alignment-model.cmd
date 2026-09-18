@echo off
setlocal
set "MODEL_DIR=%~dp0"
set "REV=b0a09e7298ff9b62072308467fe00936dd0d9a1b"
set "BASE=https://huggingface.co/FinDIT-Studio/wav2vec2-large-xlsr-53-japanese-onnx/resolve/%REV%"
set "MODEL=%MODEL_DIR%wav2vec2-large-xlsr-53-japanese.onnx"
set "TOKENIZER=%MODEL_DIR%wav2vec2-large-xlsr-53-japanese-tokenizer.json"

echo Downloading Japanese Wav2Vec2 ONNX model (about 1.27 GB)...
curl.exe -L --fail --retry 3 --output "%MODEL%" "%BASE%/model.onnx"
if errorlevel 1 goto :error

echo Verifying model SHA-256...
for /f "usebackq delims=" %%H in (`PowerShell -NoProfile -Command "(Get-FileHash -Algorithm SHA256 -LiteralPath '%MODEL%').Hash.ToLowerInvariant()"`) do set "ACTUAL=%%H"
if /i not "%ACTUAL%"=="1157d2e1078392f6469e87993d879e3af569fb9754a443c539dd5886cfbd4c5e" (
  echo Model checksum mismatch: %ACTUAL%
  goto :error
)

echo Downloading Japanese tokenizer...
curl.exe -L --fail --retry 3 --output "%TOKENIZER%" "%BASE%/tokenizer.json"
if errorlevel 1 goto :error

echo Japanese alignment model is ready.
exit /b 0

:error
echo Japanese alignment model download failed.
exit /b 1
