//! 可逆音源（WAV/FLAC）を、タグ埋め込み用の別形式へ再エンコードする処理。
//! `preprocess::decode`でデコードし、チャンネル数・サンプルレートは
//! そのまま保って再エンコードする（Forced Alignment前処理とは異なり、
//! モノラル化・16kHz化は行わない）。
//!
//! M4A（AAC）への変換は実用的な純Rustエンコーダが無いため、ここでは
//! 未対応（別タスクでWindows Media Foundation連携として実装予定）。

use std::path::Path;

use flacenc::{component::BitRepr, error::Verify};
use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, InterleavedPcm, Quality};

use super::preprocess::decode;

/// 可逆音源をFLACへ再エンコードし、`output`へ書き出す。
pub fn to_flac(input: &Path, output: &Path) -> Result<(), String> {
    let decoded = decode(input, None)?;
    let channels = usize::from(decoded.channels);
    let mut samples: Vec<i32> = decoded
        .interleaved
        .iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i32)
        .collect();

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, error)| format!("invalid FLAC encoder configuration: {error:?}"))?;
    let block_size = config.block_size;
    // `encode_with_fixed_block_size`が末尾に端数フレーム（`block_size`未満の
    // 最終ブロック）を残すと、そのFLACをSymphoniaで開いた際に
    // "end of stream"で失敗する（flacenc側の既知の癖）。総フレーム数を
    // block_sizeの倍数に揃えるため、無音（最大でも1ブロック未満、数十ms
    // 程度）を末尾に足しておく。
    let frames = samples.len() / channels;
    let pad_frames = (block_size - frames % block_size) % block_size;
    samples.resize(samples.len() + pad_frames * channels, 0);

    let source = flacenc::source::MemSource::from_samples(
        &samples,
        channels,
        16,
        decoded.sample_rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, block_size)
        .map_err(|error| format!("FLAC encoding failed: {error:?}"))?;

    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|error| format!("could not serialize FLAC stream: {error:?}"))?;
    std::fs::write(output, sink.as_slice())
        .map_err(|error| format!("could not write {}: {error}", output.display()))
}

/// 可逆音源をMP3へ再エンコードし、`output`へ書き出す。LAME
/// （LGPL）へのバインディングを使う。ffmpeg（GPL）よりも再配布時の
/// 条件が軽いため採用している。
pub fn to_mp3(input: &Path, output: &Path) -> Result<(), String> {
    let decoded = decode(input, None)?;
    let samples: Vec<i16> = decoded
        .interleaved
        .iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
        .collect();

    let mut builder =
        Builder::new().ok_or_else(|| "could not initialize LAME encoder".to_owned())?;
    builder
        .set_num_channels(decoded.channels as u8)
        .map_err(|error| format!("could not set channel count: {error:?}"))?;
    builder
        .set_sample_rate(decoded.sample_rate)
        .map_err(|error| format!("could not set sample rate: {error:?}"))?;
    builder
        .set_brate(Bitrate::Kbps192)
        .map_err(|error| format!("could not set bitrate: {error:?}"))?;
    builder
        .set_quality(Quality::Best)
        .map_err(|error| format!("could not set encoder quality: {error:?}"))?;
    let mut encoder = builder
        .build()
        .map_err(|error| format!("could not build LAME encoder: {error:?}"))?;

    let input_pcm = InterleavedPcm(&samples);
    let mut mp3_out = Vec::with_capacity(mp3lame_encoder::max_required_buffer_size(samples.len()));
    let encoded_size = encoder
        .encode(input_pcm, mp3_out.spare_capacity_mut())
        .map_err(|error| format!("MP3 encoding failed: {error:?}"))?;
    // SAFETY: `encode` just initialized exactly `encoded_size` bytes at the
    // front of the spare capacity we handed it.
    unsafe { mp3_out.set_len(mp3_out.len() + encoded_size) };

    let flush_size = encoder
        .flush::<FlushNoGap>(mp3_out.spare_capacity_mut())
        .map_err(|error| format!("MP3 encoder flush failed: {error:?}"))?;
    // SAFETY: same reasoning as above, for the bytes `flush` just wrote.
    unsafe { mp3_out.set_len(mp3_out.len() + flush_size) };

    std::fs::write(output, mp3_out)
        .map_err(|error| format!("could not write {}: {error}", output.display()))
}
