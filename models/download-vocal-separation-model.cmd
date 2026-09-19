@echo off
setlocal
set "MODEL_DIR=%~dp0"
set "REV=2ef0d757d3e226d0da85fb8c71514f464fcabdd0"
set "BASE=https://huggingface.co/StemSplitio/htdemucs-ft-vocals-onnx/resolve/%REV%"
set "MODEL=%MODEL_DIR%htdemucs-ft-vocals.onnx"

echo Downloading vocal separation ONNX model (about 166 MB, fp16 weights)...
curl.exe -L --fail --retry 3 --output "%MODEL%" "%BASE%/htdemucs_ft_vocals_fp16weights.onnx"
if errorlevel 1 goto :error

echo Verifying model SHA-256...
for /f "usebackq delims=" %%H in (`PowerShell -NoProfile -Command "(Get-FileHash -Algorithm SHA256 -LiteralPath '%MODEL%').Hash.ToLowerInvariant()"`) do set "ACTUAL=%%H"
if /i not "%ACTUAL%"=="0cbe651f535415c9d26a7bb614f7d322dd5a080fa0298f2e50f478030a994dce" (
  echo Model checksum mismatch: %ACTUAL%
  goto :error
)

echo Vocal separation model is ready.
exit /b 0

:error
echo Vocal separation model download failed.
exit /b 1
