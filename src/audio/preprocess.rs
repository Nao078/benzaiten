//! 純Rust実装（Symphonia＋rubato）による、Forced Alignmentが要求する
//! 固定フォーマットへの変換処理。外部のffmpegバイナリには依存しない。

use std::{
    fs::File,
    path::Path,
    sync::{atomic::AtomicBool, atomic::Ordering, Arc},
};

use rubato::{FftFixedIn, Resampler};
use symphonia::core::{
    codecs::{DecoderOptions, CODEC_TYPE_NULL},
    errors::Error as SymphoniaError,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

const TARGET_SAMPLE_RATE: u32 = 16_000;
/// resamplerへ一度に渡す入力フレーム数。値自体に強い意味は無く、
/// 精度と処理速度のバランスの取れた大きさであれば良い。
const RESAMPLE_CHUNK: usize = 2048;

/// デコード済みの音声：チャンネルインターリーブのf32サンプル列と、
/// 元のサンプルレート・チャンネル数。値の範囲はおおむね`[-1.0, 1.0]`。
pub struct DecodedAudio {
    pub interleaved: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// 入力ファイルを、Wav2Vec2が期待する16kHzモノラルPCM16 WAVへ変換する。
///
/// Symphoniaで入力をデコードしながらモノラルへダウンミックスし、
/// 必要なら rubato でサンプルレートを16kHzへ変換した上で、`hound`で
/// WAVとして書き出す。対応形式（MP3/WAV/FLAC/OGG/M4A）はいずれも
/// Symphoniaが純Rustでデコード可能なため、外部プロセスを一切起動しない。
pub fn to_pcm16_mono_16khz(
    input: &Path,
    output: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let decoded = decode(input, Some(&cancel))?;
    let mono = downmix_to_mono(&decoded.interleaved, decoded.channels);
    let resampled = resample_interleaved(&mono, 1, decoded.sample_rate, TARGET_SAMPLE_RATE)?;
    write_pcm16_wav(output, &resampled)
}

/// 入力ファイルを全チャンネル保持したままデコードする。`audio::convert`
/// （可逆音源からの再エンコード）と、このモジュールの前処理の両方から
/// 使う共通のデコード処理。
pub fn decode(input: &Path, cancel: Option<&AtomicBool>) -> Result<DecodedAudio, String> {
    if !input.is_file() {
        return Err(format!("audio file was not found: {}", input.display()));
    }
    let file = File::open(input)
        .map_err(|error| format!("could not open {}: {error}", input.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = input.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("could not recognize audio format: {error}"))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "audio file has no decodable track".to_owned())?
        .clone();
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| "audio track has no sample rate".to_owned())?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| format!("unsupported audio codec: {error}"))?;

    let mut channels: Option<u16> = None;
    let mut interleaved = Vec::new();
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err("processing cancelled".to_owned());
        }
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            // ストリームの終端は`IoError`のUnexpectedEofとして通知される。
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => return Err(format!("could not read audio packet: {error}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                channels.get_or_insert(spec.channels.count() as u16);
                let mut buffer = symphonia::core::audio::SampleBuffer::<f32>::new(
                    decoded.capacity() as u64,
                    spec,
                );
                buffer.copy_interleaved_ref(decoded);
                interleaved.extend_from_slice(buffer.samples());
            }
            // 個別パケットのデコードエラーはスキップし、後続の復号を試みる。
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(format!("could not decode audio: {error}")),
        }
    }
    let channels = channels.ok_or_else(|| "audio file has no decodable frames".to_owned())?;
    Ok(DecodedAudio {
        interleaved,
        sample_rate,
        channels,
    })
}

/// インターリーブされたマルチチャンネルのf32サンプル列を、フレームごとの
/// 平均を取ってモノラルへダウンミックスする。
fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels).max(1);
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// インターリーブされた（1チャンネル以上の）f32サンプル列を`from_rate`
/// から`to_rate`へリサンプリングする。`channels`個のチャンネルをまとめて
/// 同じresamplerに通すことで、チャンネル間の時間的なずれが生じないように
/// している。`from_rate == to_rate`の場合はそのまま返す。
pub fn resample_interleaved(
    interleaved: &[f32],
    channels: u16,
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<f32>, String> {
    if from_rate == to_rate {
        return Ok(interleaved.to_vec());
    }
    let channels = usize::from(channels).max(1);
    let frames = interleaved.len() / channels;
    let mut resampler = FftFixedIn::<f32>::new(
        from_rate as usize,
        to_rate as usize,
        RESAMPLE_CHUNK,
        1,
        channels,
    )
    .map_err(|error| format!("could not initialize resampler: {error}"))?;

    // チャンネルごとの連続バッファへ組み直す（rubatoはチャンネル別の
    // スライスを要求するため）。
    let mut per_channel: Vec<Vec<f32>> = vec![Vec::with_capacity(frames); channels];
    for frame in interleaved.chunks(channels) {
        for (channel, &sample) in frame.iter().enumerate() {
            per_channel[channel].push(sample);
        }
    }

    let mut resampled_channels: Vec<Vec<f32>> = vec![Vec::new(); channels];
    let mut position = 0;
    while position < frames {
        let end = (position + RESAMPLE_CHUNK).min(frames);
        let chunks: Vec<Vec<f32>> = per_channel
            .iter()
            .map(|channel| {
                let mut chunk = channel[position..end].to_vec();
                chunk.resize(RESAMPLE_CHUNK, 0.0);
                chunk
            })
            .collect();
        let produced = resampler
            .process(&chunks, None)
            .map_err(|error| format!("resampling failed: {error}"))?;
        for (output, produced) in resampled_channels.iter_mut().zip(produced) {
            output.extend_from_slice(&produced);
        }
        position = end;
    }
    // 最後のチャンクをゼロ埋めした分だけ余分な無音が末尾に付くので、
    // 想定される出力長へ切り詰める。
    let expected_frames =
        (frames as f64 * f64::from(to_rate) / f64::from(from_rate)).round() as usize;
    for channel in &mut resampled_channels {
        channel.truncate(expected_frames);
    }

    let mut result = Vec::with_capacity(expected_frames * channels);
    for frame in 0..expected_frames {
        for channel in &resampled_channels {
            result.push(channel[frame]);
        }
    }
    Ok(result)
}

/// モノラルf32サンプル列を16bit PCM WAVとして書き出す。
fn write_pcm16_wav(output: &Path, samples: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(output, spec)
        .map_err(|error| format!("could not create {}: {error}", output.display()))?;
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let quantized = (clamped * i16::MAX as f32).round() as i16;
        writer
            .write_sample(quantized)
            .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    }
    writer
        .finalize()
        .map_err(|error| format!("could not finalize {}: {error}", output.display()))
}
