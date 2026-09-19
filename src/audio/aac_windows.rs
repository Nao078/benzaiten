//! Windows Media Foundation標準搭載のAACエンコーダを使ったM4A書き出し。
//!
//! ffmpegのようなサードパーティ製バイナリを一切必要としない
//! （OS自体が提供するコーデックを`IMFSinkWriter`経由で呼び出すだけ）ため、
//! 追加のライセンス条件も配布物も発生しない。実用的な純Rust AACエンコーダ
//! が存在しないため、Windows専用のこの実装で対応している。
//!
//! 参照: <https://learn.microsoft.com/windows/win32/medfound/tutorial--using-the-sink-writer-to-encode-video>、
//! <https://learn.microsoft.com/windows/win32/medfound/aac-encoder>

use std::path::Path;

use windows::{
    core::HSTRING,
    Win32::{
        Media::MediaFoundation::{
            IMFAttributes, IMFMediaBuffer, IMFSinkWriter, MFAudioFormat_AAC, MFAudioFormat_PCM,
            MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
            MFCreateSinkWriterFromURL, MFMediaType_Audio, MFShutdown, MFStartup,
            MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
            MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE,
            MF_MT_SUBTYPE, MF_SINK_WRITER_DISABLE_THROTTLING, MF_VERSION,
        },
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    },
};

use super::preprocess::{decode, resample_channels, resample_interleaved};

/// AACエンコーダが受け付けるサンプルレート（`docs/design.md`参照）。
const SUPPORTED_SAMPLE_RATES: [u32; 2] = [44_100, 48_000];
/// 1回にSinkWriterへ渡すフレーム数（AACの1フレームは1024サンプルなので、
/// その整数倍にしておくと無駄がない）。
const CHUNK_FRAMES: usize = 1024 * 4;

/// 可逆音源をM4A（AAC-LC）へ再エンコードし、`output`へ書き出す。
pub fn to_m4a(input: &Path, output: &Path) -> Result<(), String> {
    let decoded = decode(input, None)?;
    let (channels, sample_rate) = negotiate_format(decoded.channels, decoded.sample_rate);
    let interleaved = if channels == decoded.channels {
        decoded.interleaved
    } else {
        resample_channels(&decoded.interleaved, decoded.channels, channels)
    };
    let interleaved =
        resample_interleaved(&interleaved, channels, decoded.sample_rate, sample_rate)?;
    let samples: Vec<i16> = interleaved
        .iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
        .collect();

    if output.exists() {
        return Err(format!(
            "変換先ファイルが既に存在するため中止しました: {}",
            output.display()
        ));
    }

    // SAFETY: Media Foundation・COMはいずれもこの関数内で初期化・
    // 後始末まで完結させており、呼び出し中は他スレッドと共有しない。
    unsafe { encode(output, &samples, channels, sample_rate) }
}

/// AACエンコーダが対応するチャンネル数（1/2/6）・サンプルレート
/// （44100/48000Hz）へ丸める。デコード結果がこれ以外の場合、
/// 一般的な音楽ファイルではまず起こらないが、安全のため近い値へ寄せる。
fn negotiate_format(channels: u16, sample_rate: u32) -> (u16, u32) {
    let channels = match channels {
        1 => 1,
        6 => 6,
        _ => 2,
    };
    let sample_rate = if SUPPORTED_SAMPLE_RATES.contains(&sample_rate) {
        sample_rate
    } else {
        44_100
    };
    (channels, sample_rate)
}

unsafe fn encode(
    output: &Path,
    samples: &[i16],
    channels: u16,
    sample_rate: u32,
) -> Result<(), String> {
    let com_initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
    let result = encode_with_media_foundation(output, samples, channels, sample_rate);
    if com_initialized {
        CoUninitialize();
    }
    result
}

unsafe fn encode_with_media_foundation(
    output: &Path,
    samples: &[i16],
    channels: u16,
    sample_rate: u32,
) -> Result<(), String> {
    MFStartup(MF_VERSION, 0).map_err(|error| format!("MFStartupに失敗しました: {error}"))?;
    let result = write_m4a(output, samples, channels, sample_rate);
    let _ = MFShutdown();
    result
}

/// 実際にSinkWriterを組み立ててAACへエンコードし、M4Aとして書き出す。
unsafe fn write_m4a(
    output: &Path,
    samples: &[i16],
    channels: u16,
    sample_rate: u32,
) -> Result<(), String> {
    let block_align = u32::from(channels) * 2;
    let bytes_per_second = sample_rate * block_align;
    // ドキュメント記載のAACエンコーダ対応ビットレート（モノ/ステレオ用）。
    // 5.1chの場合は6倍になるが、このアプリでは実質モノ/ステレオのみ扱う。
    let aac_bytes_per_second: u32 = if channels >= 6 { 24_000 * 6 } else { 20_000 };

    // バッチ変換なので、リアルタイム再生に合わせた書込み速度の抑制
    // （スロットリング）は無効化しておく。
    let mut writer_attributes: Option<IMFAttributes> = None;
    MFCreateAttributes(&mut writer_attributes, 1)
        .map_err(|error| format!("MFCreateAttributesに失敗しました: {error}"))?;
    let writer_attributes =
        writer_attributes.ok_or_else(|| "MFCreateAttributesが空でした".to_owned())?;
    writer_attributes
        .SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)
        .map_err(|error| format!("SinkWriter属性の設定に失敗しました: {error}"))?;

    let output_url = HSTRING::from(output.as_os_str());
    let sink_writer: IMFSinkWriter =
        MFCreateSinkWriterFromURL(&output_url, None, &writer_attributes)
            .map_err(|error| format!("出力ファイルを作成できません: {error}"))?;

    let output_type =
        MFCreateMediaType().map_err(|error| format!("MFCreateMediaTypeに失敗しました: {error}"))?;
    output_type
        .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
        .and_then(|()| output_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC))
        .and_then(|()| output_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16))
        .and_then(|()| output_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate))
        .and_then(|()| output_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(channels)))
        .and_then(|()| {
            output_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, aac_bytes_per_second)
        })
        .map_err(|error| format!("AAC出力形式の設定に失敗しました: {error}"))?;
    let stream_index = sink_writer
        .AddStream(&output_type)
        .map_err(|error| format!("AACストリームの追加に失敗しました: {error}"))?;

    let input_type =
        MFCreateMediaType().map_err(|error| format!("MFCreateMediaTypeに失敗しました: {error}"))?;
    input_type
        .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
        .and_then(|()| input_type.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM))
        .and_then(|()| input_type.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16))
        .and_then(|()| input_type.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate))
        .and_then(|()| input_type.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(channels)))
        .and_then(|()| input_type.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_second))
        .map_err(|error| format!("PCM入力形式の設定に失敗しました: {error}"))?;
    sink_writer
        .SetInputMediaType(stream_index, &input_type, None)
        .map_err(|error| format!("PCM入力形式の登録に失敗しました: {error}"))?;

    sink_writer
        .BeginWriting()
        .map_err(|error| format!("BeginWritingに失敗しました: {error}"))?;

    let total_frames = samples.len() / usize::from(channels);
    let mut frame = 0_usize;
    let mut sample_time_100ns: i64 = 0;
    while frame < total_frames {
        let chunk_frames = CHUNK_FRAMES.min(total_frames - frame);
        let start = frame * usize::from(channels);
        let end = start + chunk_frames * usize::from(channels);
        let chunk = &samples[start..end];
        let chunk_bytes: &[u8] =
            std::slice::from_raw_parts(chunk.as_ptr().cast::<u8>(), std::mem::size_of_val(chunk));

        let media_buffer: IMFMediaBuffer = MFCreateMemoryBuffer(chunk_bytes.len() as u32)
            .map_err(|error| format!("MFCreateMemoryBufferに失敗しました: {error}"))?;
        let mut buffer_ptr = std::ptr::null_mut();
        media_buffer
            .Lock(&mut buffer_ptr, None, None)
            .map_err(|error| format!("バッファのロックに失敗しました: {error}"))?;
        std::ptr::copy_nonoverlapping(chunk_bytes.as_ptr(), buffer_ptr, chunk_bytes.len());
        media_buffer
            .Unlock()
            .map_err(|error| format!("バッファの解放に失敗しました: {error}"))?;
        media_buffer
            .SetCurrentLength(chunk_bytes.len() as u32)
            .map_err(|error| format!("バッファ長の設定に失敗しました: {error}"))?;

        let media_sample =
            MFCreateSample().map_err(|error| format!("MFCreateSampleに失敗しました: {error}"))?;
        media_sample
            .AddBuffer(&media_buffer)
            .map_err(|error| format!("サンプルへのバッファ追加に失敗しました: {error}"))?;
        let duration_100ns = (chunk_frames as i64 * 10_000_000) / i64::from(sample_rate);
        media_sample
            .SetSampleTime(sample_time_100ns)
            .and_then(|()| media_sample.SetSampleDuration(duration_100ns))
            .map_err(|error| format!("サンプル時刻の設定に失敗しました: {error}"))?;

        sink_writer
            .WriteSample(stream_index, &media_sample)
            .map_err(|error| format!("AACエンコードに失敗しました: {error}"))?;

        sample_time_100ns += duration_100ns;
        frame += chunk_frames;
    }

    sink_writer
        .Finalize()
        .map_err(|error| format!("M4Aファイルの確定に失敗しました: {error}"))?;
    Ok(())
}
