//! Forced Alignmentが要求する固定フォーマットへの、ffmpegによる変換処理。

use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

/// 入力ファイルを、Wav2Vec2が期待する16kHzモノラルPCM WAVへ変換する。
///
/// `-y`は`output`を無条件に上書きし、`-vn`は映像・アートワークの
/// ストリームを除外する（ffmpegがカバーアート画像トラックまで
/// 変換しようとしないようにするため）。
pub fn to_pcm16_mono_16khz(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    if !crate::process::is_bare_command(ffmpeg) && !ffmpeg.is_file() {
        return Err(format!(
            "ffmpeg executable was not found: {}",
            ffmpeg.display()
        ));
    }
    if !input.is_file() {
        return Err(format!("audio file was not found: {}", input.display()));
    }

    let arguments = [
        Path::new("-y"),
        Path::new("-i"),
        input,
        Path::new("-vn"),
        Path::new("-ar"),
        Path::new("16000"),
        Path::new("-ac"),
        Path::new("1"),
        Path::new("-c:a"),
        Path::new("pcm_s16le"),
        output,
    ];
    crate::process::run(ffmpeg, &arguments, cancel).map(|_| ())
}
