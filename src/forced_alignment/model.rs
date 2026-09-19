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

/// 推論窓（20秒）どうしを重ねる長さ。窓の端はTransformerが前後の文脈を
/// 参照できず認識精度が落ちるため、両隣の窓と5秒（前後2.5秒ずつ）重ねて
/// 推論し、各窓の端2.5秒分は捨てて中央部分だけを採用する。窓を重ねずに
/// 独立推論すると、20秒おきの継ぎ目で精度が落ちた区間が曲中に散らばり、
/// 全体を1本のViterbi経路で解く強制アラインメントでは先頭・末尾が経路の
/// 自由度不足でほぼ固定される一方、自由度の大きい中間部でこの継ぎ目誤差が
/// 蓄積・吸収されて「最初と最後は合うが中間がずれる」結果になっていた。
const WINDOW_SECONDS: usize = 20;
const STRIDE_SECONDS: usize = 15;

/// 推論窓の並びを表す。`start..end`がモデルに渡すサンプル範囲、
/// `left_trim`/`right_trim`は結果のフレーム列のうち文脈不足で捨てるべき
/// 端の長さ（サンプル数、後でフレーム数へ換算する）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    start: usize,
    end: usize,
    left_trim: usize,
    right_trim: usize,
}

/// `total_samples`を`window_samples`長の窓へ`stride_samples`間隔で
/// スライドさせながら分割する計画を立てる。隣接窓と`context_samples`ずつ
/// 重なるようにし、各窓の対応する`left_trim`/`right_trim`を引いた残りが
/// 隙間なくちょうど`0..total_samples`を敷き詰めるようにする
/// （窓を跨いだ文脈が使える中央部分だけを採用するため）。先頭の窓は
/// 左に、末尾の窓は右に捨てるべき文脈がそもそも無いので、その側は
/// 捨てない。
fn plan_windows(
    total_samples: usize,
    window_samples: usize,
    stride_samples: usize,
    context_samples: usize,
) -> Vec<Window> {
    if total_samples == 0 {
        return Vec::new();
    }
    let mut windows = Vec::new();
    let mut start = 0_usize;
    loop {
        let end = (start + window_samples).min(total_samples);
        let is_first = start == 0;
        let is_last = end == total_samples;
        let left_trim = if is_first {
            0
        } else {
            context_samples.min(end - start)
        };
        let right_trim = if is_last {
            0
        } else {
            context_samples.min(end - start - left_trim)
        };
        windows.push(Window {
            start,
            end,
            left_trim,
            right_trim,
        });
        if is_last {
            break;
        }
        start += stride_samples;
    }
    windows
}

impl AcousticModel for OnnxWav2Vec2 {
    /// 音声を20秒の窓に区切り、隣の窓と5秒重ねて（[`plan_windows`]）
    /// 順に推論し、各窓の中央部分（文脈が十分な範囲）だけを結果のlogits
    /// フレームとして連結する。窓ごとに`cancel`を確認できるようにしている
    /// （キャンセル時は即座に打ち切る）。`progress`には処理済みサンプル数
    /// に基づくおおよその進捗率（0〜100）を通知する。
    ///
    /// 正規化（平均0・分散1への標準化）は窓ごとではなく音声全体で1回だけ
    /// 計算した統計量を使う。静かな間奏と大音量のサビのように窓によって
    /// 音量特性が異なる曲でも、モデルが学習時に想定する発話全体での
    /// 正規化に近い入力分布を保つため。
    fn infer(
        &mut self,
        wav: &Path,
        cancel: Arc<AtomicBool>,
        progress: &mut dyn FnMut(u8),
    ) -> Result<Emissions, String> {
        let samples = read_pcm16_mono(wav)?;
        if samples.is_empty() {
            return Err("Forced Alignment model received empty audio".to_owned());
        }
        const SAMPLE_RATE: usize = 16_000;
        const WINDOW_SAMPLES: usize = WINDOW_SECONDS * SAMPLE_RATE;
        const STRIDE_SAMPLES: usize = STRIDE_SECONDS * SAMPLE_RATE;
        const CONTEXT_SAMPLES: usize = (WINDOW_SAMPLES - STRIDE_SAMPLES) / 2;

        let (mean, scale) = normalization_stats(&samples);
        let total_samples = samples.len();
        let windows = plan_windows(
            total_samples,
            WINDOW_SAMPLES,
            STRIDE_SAMPLES,
            CONTEXT_SAMPLES,
        );
        let mut frames = Vec::new();

        for window in windows {
            if cancel.load(Ordering::Acquire) {
                return Err("processing cancelled".to_owned());
            }
            let chunk = &samples[window.start..window.end];
            let input_values = normalize_samples(chunk, mean, scale);
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
            let window_frames: Vec<Vec<f32>> = flat
                .chunks(vocabulary)
                .map(|frame| frame.to_vec())
                .collect();

            // サンプル単位で決めた前後の切り捨て幅を、この窓の実際の
            // フレームレート（モデルのダウンサンプル率）に合わせてフレーム
            // 単位へ換算する。
            let samples_per_frame = chunk.len() as f64 / window_frames.len().max(1) as f64;
            let left_trim_frames = ((window.left_trim as f64 / samples_per_frame).round() as usize)
                .min(window_frames.len());
            let right_trim_frames = ((window.right_trim as f64 / samples_per_frame).round()
                as usize)
                .min(window_frames.len() - left_trim_frames);
            let keep = window_frames.len() - left_trim_frames - right_trim_frames;
            frames.extend(window_frames.into_iter().skip(left_trim_frames).take(keep));

            progress(((window.end * 100) / total_samples) as u8);
        }

        if frames.is_empty() {
            return Err("Forced Alignment model returned no acoustic frames".to_owned());
        }
        let duration_ms = total_samples as f64 * 1000.0 / SAMPLE_RATE as f64;
        Ok(Emissions {
            frame_duration_ms: duration_ms / frames.len() as f64,
            frames,
        })
    }
}

/// PCM16サンプル全体（`[-1.0, 1.0]`へ正規化した後）の平均と標準偏差を
/// 求める。窓ごとではなく音声全体で1回だけ計算し、[`normalize_samples`]
/// へ渡すことで、窓によって音量特性が異なっていても一貫した標準化に
/// なるようにする（Wav2Vec2の学習時の入力前処理に合わせるため）。
fn normalization_stats(samples: &[i16]) -> (f32, f32) {
    let values: Vec<f32> = samples
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    let variance = values
        .iter()
        .map(|value| (*value - mean).powi(2))
        .sum::<f32>()
        / values.len().max(1) as f32;
    (mean, (variance + 1e-7).sqrt())
}

/// PCM16サンプルを`[-1.0, 1.0]`へ正規化した上で、[`normalization_stats`]
/// が音声全体から求めた平均・標準偏差で標準化する。
fn normalize_samples(samples: &[i16], mean: f32, scale: f32) -> Vec<f32> {
    samples
        .iter()
        .map(|sample| (f32::from(*sample) / 32768.0 - mean) / scale)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{plan_windows, Window};

    const SAMPLE_RATE: usize = 16_000;
    const WINDOW_SAMPLES: usize = 20 * SAMPLE_RATE;
    const STRIDE_SAMPLES: usize = 15 * SAMPLE_RATE;
    const CONTEXT_SAMPLES: usize = (WINDOW_SAMPLES - STRIDE_SAMPLES) / 2;

    /// 各窓から`left_trim`/`right_trim`を除いた残りを繋げると、隙間も
    /// 重なりもなくちょうど`0..total_samples`を敷き詰めることを確認する
    /// （文脈が十分にある中央部分だけを採用しても取りこぼしが出ないこと）。
    fn assert_tiles_without_gaps(total_samples: usize, windows: &[Window]) {
        let mut cursor = 0_usize;
        for window in windows {
            let kept_start = window.start + window.left_trim;
            let kept_end = window.end - window.right_trim;
            assert_eq!(
                kept_start, cursor,
                "gap or overlap before window {window:?}"
            );
            assert!(kept_end >= kept_start);
            cursor = kept_end;
        }
        assert_eq!(cursor, total_samples, "coverage did not reach the end");
    }

    #[test]
    fn single_short_window_is_not_trimmed() {
        let windows = plan_windows(1_000, WINDOW_SAMPLES, STRIDE_SAMPLES, CONTEXT_SAMPLES);
        assert_eq!(
            windows,
            vec![Window {
                start: 0,
                end: 1_000,
                left_trim: 0,
                right_trim: 0,
            }]
        );
    }

    #[test]
    fn empty_audio_has_no_windows() {
        assert!(plan_windows(0, WINDOW_SAMPLES, STRIDE_SAMPLES, CONTEXT_SAMPLES).is_empty());
    }

    #[test]
    fn overlapping_windows_tile_a_long_track_without_gaps() {
        for total_samples in [
            WINDOW_SAMPLES - 1,
            WINDOW_SAMPLES,
            WINDOW_SAMPLES + 1,
            WINDOW_SAMPLES * 2,
            WINDOW_SAMPLES * 2 + STRIDE_SAMPLES / 3,
            3 * 60 * SAMPLE_RATE + 37 * SAMPLE_RATE, // ~3:37の曲相当
        ] {
            let windows = plan_windows(
                total_samples,
                WINDOW_SAMPLES,
                STRIDE_SAMPLES,
                CONTEXT_SAMPLES,
            );
            assert!(!windows.is_empty());
            assert_tiles_without_gaps(total_samples, &windows);
        }
    }

    #[test]
    fn only_the_first_and_last_window_keep_an_untrimmed_edge() {
        let total_samples = 3 * WINDOW_SAMPLES;
        let windows = plan_windows(
            total_samples,
            WINDOW_SAMPLES,
            STRIDE_SAMPLES,
            CONTEXT_SAMPLES,
        );
        assert_eq!(windows.first().unwrap().left_trim, 0);
        assert_eq!(windows.last().unwrap().right_trim, 0);
        for window in &windows[1..windows.len() - 1] {
            assert_eq!(window.left_trim, CONTEXT_SAMPLES);
            assert_eq!(window.right_trim, CONTEXT_SAMPLES);
        }
    }
}
