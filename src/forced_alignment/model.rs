//! ONNX Runtime経由でWav2Vec2モデルを実行し、CTCのlogits（emissions）を得る。

use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};

/// モデルが出力したフレームごとのlogits列と、フレーム1つあたりの
/// 実時間（ミリ秒）。`resolver`がこれを使ってフレーム番号を実時間へ変換する。
pub struct Emissions {
    pub frames: Vec<Vec<f32>>,
    pub frame_duration_ms: f64,
}

/// 準備済みWAVからCTC emissionsを推論する音響モデルの共通インターフェース。
pub trait AcousticModel {
    fn infer(
        &mut self,
        wav: &Path,
        cancel: Arc<AtomicBool>,
        progress: &mut dyn FnMut(u8),
    ) -> Result<Emissions, String>;
}

/// ONNX Runtimeで読み込んだWav2Vec2セッション。
pub struct OnnxWav2Vec2 {
    session: Session,
}

impl OnnxWav2Vec2 {
    /// `path`のONNXモデルを読み込む。`threads`はONNX Runtimeの
    /// イントラオペレータ並列数（GUIの「スレッド数」設定に対応）。
    pub fn load(path: &Path, threads: usize) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!(
                "Forced Alignment model was not found: {}",
                path.display()
            ));
        }
        let session = Session::builder()
            .map_err(|error| format!("could not initialize ONNX Runtime: {error}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|error| format!("could not configure ONNX Runtime: {error}"))?
            .with_intra_threads(threads.max(1))
            .map_err(|error| format!("could not configure ONNX threads: {error}"))?
            .commit_from_file(path)
            .map_err(|error| format!("could not load ONNX model {}: {error}", path.display()))?;
        Ok(Self { session })
    }
}

impl AcousticModel for OnnxWav2Vec2 {
    /// 音声を20秒ごとのチャンクに分けて順に推論し、結果のlogitsフレームを
    /// 連結する。長い音声を1度に流すとメモリ使用量・推論時間が大きくなる
    /// ため、チャンクごとに`cancel`を確認できるようにしている
    /// （キャンセル時は即座に打ち切る）。`progress`にはチャンク単位の
    /// おおよその進捗率（0〜100）を通知する。
    fn infer(
        &mut self,
        wav: &Path,
        cancel: Arc<AtomicBool>,
        progress: &mut dyn FnMut(u8),
    ) -> Result<Emissions, String> {
        let samples = read_pcm16_mono(wav)?;
        const SAMPLE_RATE: usize = 16_000;
        const CHUNK_SAMPLES: usize = 20 * SAMPLE_RATE;
        let chunk_count = samples.len().div_ceil(CHUNK_SAMPLES);
        let mut frames = Vec::new();

        for (chunk_index, chunk) in samples.chunks(CHUNK_SAMPLES).enumerate() {
            if cancel.load(Ordering::Acquire) {
                return Err("processing cancelled".to_owned());
            }
            let input_values = normalize_samples(chunk);
            let input = Tensor::from_array(([1_usize, chunk.len()], input_values))
                .map_err(|error| format!("could not create ONNX input tensor: {error}"))?;
            let outputs = self
                .session
                .run(ort::inputs![input])
                .map_err(|error| format!("Forced Alignment model inference failed: {error}"))?;
            let logits = outputs[0]
                .try_extract_array::<f32>()
                .map_err(|error| format!("ONNX model did not return float logits: {error}"))?;
            let shape = logits.shape();
            if shape.len() != 3 || shape[0] != 1 || shape[2] < 2 {
                return Err(format!(
                    "unexpected ONNX output shape {shape:?}; expected [1, frames, vocabulary]"
                ));
            }
            let vocabulary = shape[2];
            let flat: Vec<f32> = logits.iter().copied().collect();
            frames.extend(flat.chunks(vocabulary).map(|frame| frame.to_vec()));
            progress((((chunk_index + 1) * 100) / chunk_count) as u8);
        }

        if frames.is_empty() {
            return Err("Forced Alignment model returned no acoustic frames".to_owned());
        }
        let duration_ms = samples.len() as f64 * 1000.0 / SAMPLE_RATE as f64;
        Ok(Emissions {
            frame_duration_ms: duration_ms / frames.len() as f64,
            frames,
        })
    }
}

/// PCM16サンプルを`[-1.0, 1.0]`へ正規化した上で、平均0・分散1に
/// 標準化する（Wav2Vec2の学習時の入力前処理に合わせるため）。
fn normalize_samples(samples: &[i16]) -> Vec<f32> {
    let mut values: Vec<f32> = samples
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    let variance = values
        .iter()
        .map(|value| (*value - mean).powi(2))
        .sum::<f32>()
        / values.len().max(1) as f32;
    let scale = (variance + 1e-7).sqrt();
    for value in &mut values {
        *value = (*value - mean) / scale;
    }
    values
}

/// 前処理済みWAV（16kHzモノラルPCM16であることが前提）を読み込み、
/// サンプル列を返す。フォーマットが想定と異なる場合はエラーにする。
fn read_pcm16_mono(path: &Path) -> Result<Vec<i16>, String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|error| format!("could not read prepared WAV {}: {error}", path.display()))?;
    let specification = reader.spec();
    if specification.channels != 1
        || specification.sample_rate != 16_000
        || specification.bits_per_sample != 16
        || specification.sample_format != hound::SampleFormat::Int
    {
        return Err("prepared WAV must be 16 kHz mono PCM16".to_owned());
    }
    reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not decode prepared WAV: {error}"))
}
