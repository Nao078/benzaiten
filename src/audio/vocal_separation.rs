//! ONNX Runtime経由でHT-Demucs（歌声分離）モデルを実行し、ステレオ音声から
//! ボーカルだけを抽出する。歌唱・伴奏条件が厳しい曲では、Forced Alignment
//! の音響モデル（読み上げ音声で学習されたWav2Vec2）が伴奏に埋もれた歌声を
//! うまく認識できず、confidenceが全編にわたって低くなることがある。先に
//! ボーカルを分離しておくことで、音響モデルへ渡す信号をクリーンにし、
//! 同期精度を改善する。任意（デフォルト無効）の前処理ステージ。

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

/// モデルが1回の推論で処理する固定入力長（サンプル数、44.1kHzステレオ・
/// 約7.8秒）。`StemSplitio/htdemucs-ft-vocals-onnx`の`mix`入力shape
/// `(1, 2, 343980)`に固定されている（モデル自体から実測して確認済み）。
const SEGMENT_SAMPLES: usize = 343_980;
/// 分離モデルが要求するチャンネル数（ステレオ固定）。
const SEPARATOR_CHANNELS: usize = 2;
/// モデルが出力するstem数（drums, bass, other, vocalsの4つ）。
const STEM_COUNT: usize = 4;
/// 出力stemのうちボーカルのインデックス（`[drums, bass, other, vocals]`）。
const VOCALS_STEM_INDEX: usize = 3;
/// 隣接チャンク間のオーバーラップ幅（25%）。モデル配布元のリファレンス
/// 実装（`infer.py`）に合わせた値。
const OVERLAP_SAMPLES: usize = SEGMENT_SAMPLES / 4;
/// チャンクを進める間隔（オーバーラップ分を差し引いた残り）。
const HOP_SAMPLES: usize = SEGMENT_SAMPLES - OVERLAP_SAMPLES;

/// ステレオ音声からボーカルを分離する処理の共通インターフェース。
pub trait VocalSeparator {
    /// `stereo_interleaved`（L,R,L,R,...の順にインターリーブされた
    /// 44.1kHzステレオ音声）からボーカルだけを抽出し、同じ長さ・同じ
    /// インターリーブ形式で返す。
    fn separate(
        &mut self,
        stereo_interleaved: &[f32],
        cancel: Arc<AtomicBool>,
        progress: &mut dyn FnMut(u8),
    ) -> Result<Vec<f32>, String>;
}

/// ONNX Runtimeで読み込んだHT-Demucsセッション。
pub struct OnnxHtDemucs {
    session: Session,
}

impl OnnxHtDemucs {
    /// `path`のONNXモデルを読み込む。`threads`はONNX Runtimeの
    /// イントラオペレータ並列数（Forced Alignmentモデルと同じ設定を共有）。
    pub fn load(path: &Path, threads: usize) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!(
                "ボーカル分離モデルが見つかりません: {}",
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

impl VocalSeparator for OnnxHtDemucs {
    fn separate(
        &mut self,
        stereo_interleaved: &[f32],
        cancel: Arc<AtomicBool>,
        progress: &mut dyn FnMut(u8),
    ) -> Result<Vec<f32>, String> {
        if !stereo_interleaved.len().is_multiple_of(SEPARATOR_CHANNELS) {
            return Err("vocal separation input must be interleaved stereo".to_owned());
        }
        let total_samples = stereo_interleaved.len() / SEPARATOR_CHANNELS;
        if total_samples == 0 {
            return Err("vocal separation received empty audio".to_owned());
        }

        let chunks = plan_separation_chunks(total_samples, SEGMENT_SAMPLES, HOP_SAMPLES);
        let session = &mut self.session;

        reconstruct_windowed(
            total_samples,
            &chunks,
            OVERLAP_SAMPLES,
            &cancel,
            |completed, total| progress(((completed * 100) / total) as u8),
            move |chunk| {
                let planar_input = build_planar_input(stereo_interleaved, chunk, total_samples);
                let input = Tensor::from_array((
                    [1_usize, SEPARATOR_CHANNELS, SEGMENT_SAMPLES],
                    planar_input,
                ))
                .map_err(|error| format!("could not create ONNX input tensor: {error}"))?;
                let outputs = session
                    .run(ort::inputs![input])
                    .map_err(|error| format!("vocal separation model inference failed: {error}"))?;
                let stems = outputs[0]
                    .try_extract_array::<f32>()
                    .map_err(|error| format!("ONNX model did not return float stems: {error}"))?;
                let shape = stems.shape();
                if shape.len() != 4
                    || shape[0] != 1
                    || shape[1] != STEM_COUNT
                    || shape[2] != SEPARATOR_CHANNELS
                    || shape[3] != SEGMENT_SAMPLES
                {
                    return Err(format!(
                        "unexpected ONNX output shape {shape:?}; expected [1, {STEM_COUNT}, {SEPARATOR_CHANNELS}, {SEGMENT_SAMPLES}]"
                    ));
                }
                let flat: Vec<f32> = stems.iter().copied().collect();
                let stem_offset = VOCALS_STEM_INDEX * SEPARATOR_CHANNELS * SEGMENT_SAMPLES;
                let left = &flat[stem_offset..stem_offset + SEGMENT_SAMPLES];
                let right = &flat[stem_offset + SEGMENT_SAMPLES..stem_offset + 2 * SEGMENT_SAMPLES];
                let mut interleaved = vec![0.0_f32; SEGMENT_SAMPLES * SEPARATOR_CHANNELS];
                for i in 0..SEGMENT_SAMPLES {
                    interleaved[i * SEPARATOR_CHANNELS] = left[i];
                    interleaved[i * SEPARATOR_CHANNELS + 1] = right[i];
                }
                Ok(interleaved)
            },
        )
    }
}

/// 分離モデルへ渡す1チャンク分のサンプル範囲。`start..end`は常に
/// `SEGMENT_SAMPLES`幅ちょうど（モデルの入力長が固定のため）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeparationChunk {
    start: usize,
    end: usize,
}

/// `total_samples`を`segment_samples`幅の固定長チャンクへ、`hop_samples`
/// 間隔でスライドさせながら分割する計画を立てる。最後のチャンクは
/// `total_samples`ちょうどに収まるよう引き戻す（`total_samples`が
/// `segment_samples`未満の場合のみ、1チャンクだけを返し、呼び出し側が
/// 末尾をゼロ埋めする前提とする）。隣接チャンクは意図的に重なり合い、
/// その重なり区間は[`crossfade_weights`]による加重和で再構成する
/// （CTCのフレーム整列と違い、波形の再構成にはトリム＋連結ではなく
/// クロスフェードが必要なため）。
fn plan_separation_chunks(
    total_samples: usize,
    segment_samples: usize,
    hop_samples: usize,
) -> Vec<SeparationChunk> {
    if total_samples == 0 {
        return Vec::new();
    }
    if total_samples <= segment_samples {
        return vec![SeparationChunk {
            start: 0,
            end: segment_samples,
        }];
    }
    let mut chunks = Vec::new();
    let mut start = 0_usize;
    loop {
        let end = start + segment_samples;
        if end >= total_samples {
            let start = total_samples - segment_samples;
            chunks.push(SeparationChunk {
                start,
                end: start + segment_samples,
            });
            break;
        }
        chunks.push(SeparationChunk { start, end });
        start += hop_samples;
    }
    chunks
}

/// 1チャンク分（`segment_samples`長）のクロスフェード重みを返す。先頭・
/// 末尾の`overlap_samples`分を0→1／1→0へ線形にランプし、中央は1.0で
/// 平坦（モデル配布元のリファレンス実装`infer.py`の
/// `np.linspace(0, 1, transition)`と同じ形）。隣接チャンクの重なり区間
/// では、片方の立ち上がりと隣のチャンクの立ち下がりの重みの和が常に
/// 1になるため、加重和した後に重みの合計で正規化すれば振幅が変化しない。
///
/// ただし曲全体の先頭・末尾はこの限りではない：先頭チャンクの立ち上がり
/// （`is_first`）や末尾チャンクの立ち下がり（`is_last`）は、重ねる相手の
/// チャンクがそもそも存在しないため、ランプさせず1.0のままにする
/// （そうしないと曲の一番最初・最後のサンプルの重みが0になり、
/// 正規化時にゼロ除算に近い状態で振幅が消えてしまう）。
fn crossfade_weights(
    segment_samples: usize,
    overlap_samples: usize,
    is_first: bool,
    is_last: bool,
) -> Vec<f32> {
    let overlap = overlap_samples.min(segment_samples / 2);
    let mut weights = vec![1.0_f32; segment_samples];
    if overlap == 0 {
        return weights;
    }
    for i in 0..overlap {
        let ramp = if overlap == 1 {
            0.0
        } else {
            i as f32 / (overlap - 1) as f32
        };
        if !is_first {
            weights[i] = ramp;
        }
        if !is_last {
            weights[segment_samples - overlap + i] = 1.0 - ramp;
        }
    }
    weights
}

/// チャンクの推論結果（`infer_chunk`）を[`crossfade_weights`]で加重しながら
/// 足し合わせ、最後に重みの合計で正規化して1本の波形へ再構成する、
/// ONNX Runtimeに依存しない純粋な処理。`infer_chunk`は1チャンク分
/// （`SEGMENT_SAMPLES`長）のインターリーブステレオ音声を返す。この関数を
/// ONNX推論から切り離しておくことで、実モデル無しでもフェイクの
/// `infer_chunk`を使って再構成ロジック自体を単体テストできる。
fn reconstruct_windowed(
    total_samples: usize,
    chunks: &[SeparationChunk],
    overlap_samples: usize,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
    mut infer_chunk: impl FnMut(&SeparationChunk) -> Result<Vec<f32>, String>,
) -> Result<Vec<f32>, String> {
    let mut output = vec![0.0_f32; total_samples * SEPARATOR_CHANNELS];
    let mut weight_sum = vec![0.0_f32; total_samples];

    for (index, chunk) in chunks.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            return Err("processing cancelled".to_owned());
        }
        let chunk_audio = infer_chunk(chunk)?;
        let segment_samples = chunk.end - chunk.start;
        let weights = crossfade_weights(
            segment_samples,
            overlap_samples,
            index == 0,
            index + 1 == chunks.len(),
        );
        for (i, &weight) in weights.iter().enumerate() {
            let position = chunk.start + i;
            if position >= total_samples {
                break;
            }
            weight_sum[position] += weight;
            output[position * SEPARATOR_CHANNELS] += chunk_audio[i * SEPARATOR_CHANNELS] * weight;
            output[position * SEPARATOR_CHANNELS + 1] +=
                chunk_audio[i * SEPARATOR_CHANNELS + 1] * weight;
        }
        progress(index + 1, chunks.len());
    }

    for position in 0..total_samples {
        let weight = weight_sum[position].max(1e-8);
        output[position * SEPARATOR_CHANNELS] /= weight;
        output[position * SEPARATOR_CHANNELS + 1] /= weight;
    }
    Ok(output)
}

/// `chunk`が指すサンプル範囲を`stereo_interleaved`から切り出し、モデルが
/// 要求する平面（channel-major: 左チャンネル`SEGMENT_SAMPLES`個の後に
/// 右チャンネル`SEGMENT_SAMPLES`個）形式へ変換する。曲の末尾に達した分は
/// ゼロ埋めのまま残す。
fn build_planar_input(
    stereo_interleaved: &[f32],
    chunk: &SeparationChunk,
    total_samples: usize,
) -> Vec<f32> {
    let mut planar = vec![0.0_f32; SEGMENT_SAMPLES * SEPARATOR_CHANNELS];
    let (left, right) = planar.split_at_mut(SEGMENT_SAMPLES);
    for i in 0..SEGMENT_SAMPLES {
        let position = chunk.start + i;
        if position >= total_samples {
            break;
        }
        left[i] = stereo_interleaved[position * SEPARATOR_CHANNELS];
        right[i] = stereo_interleaved[position * SEPARATOR_CHANNELS + 1];
    }
    planar
}

#[cfg(test)]
mod tests {
    use super::{
        crossfade_weights, plan_separation_chunks, reconstruct_windowed, SeparationChunk,
        HOP_SAMPLES, OVERLAP_SAMPLES, SEGMENT_SAMPLES, SEPARATOR_CHANNELS,
    };
    use std::sync::atomic::AtomicBool;

    #[test]
    fn single_short_track_yields_one_zero_padded_chunk() {
        let chunks = plan_separation_chunks(1_000, SEGMENT_SAMPLES, HOP_SAMPLES);
        assert_eq!(
            chunks,
            vec![SeparationChunk {
                start: 0,
                end: SEGMENT_SAMPLES,
            }]
        );
    }

    #[test]
    fn empty_audio_has_no_chunks() {
        assert!(plan_separation_chunks(0, SEGMENT_SAMPLES, HOP_SAMPLES).is_empty());
    }

    #[test]
    fn chunks_cover_a_long_track_without_gaps() {
        for total_samples in [
            SEGMENT_SAMPLES - 1,
            SEGMENT_SAMPLES,
            SEGMENT_SAMPLES + 1,
            SEGMENT_SAMPLES * 2,
            SEGMENT_SAMPLES * 2 + HOP_SAMPLES / 3,
            3 * 60 * 44_100 + 37 * 44_100, // ~3:37の曲相当(44.1kHz)
        ] {
            let chunks = plan_separation_chunks(total_samples, SEGMENT_SAMPLES, HOP_SAMPLES);
            assert!(
                !chunks.is_empty(),
                "no chunks for total_samples={total_samples}"
            );
            let mut covered = vec![false; total_samples];
            for chunk in &chunks {
                let end = chunk.end.min(total_samples);
                for value in covered.iter_mut().take(end).skip(chunk.start) {
                    *value = true;
                }
            }
            assert!(
                covered.iter().all(|&value| value),
                "gap found for total_samples={total_samples}"
            );
        }
    }

    #[test]
    fn middle_chunk_weights_ramp_from_zero_to_one_and_back() {
        let weights = crossfade_weights(SEGMENT_SAMPLES, OVERLAP_SAMPLES, false, false);
        assert_eq!(weights.len(), SEGMENT_SAMPLES);
        assert_eq!(weights[0], 0.0);
        assert!((weights[OVERLAP_SAMPLES - 1] - 1.0).abs() < 1e-6);
        assert_eq!(weights[SEGMENT_SAMPLES / 2], 1.0);
        assert!((weights[SEGMENT_SAMPLES - OVERLAP_SAMPLES] - 1.0).abs() < 1e-6);
        assert_eq!(*weights.last().unwrap(), 0.0);
    }

    #[test]
    fn first_and_last_chunk_do_not_ramp_down_their_outer_edge() {
        // 曲全体の先頭チャンクは、立ち上がり側に重ねる相手のチャンクが
        // 存在しないので、その端は常に1.0でなければならない
        // （さもないと曲の最初のサンプルの重みが0になってしまう）。
        let first = crossfade_weights(SEGMENT_SAMPLES, OVERLAP_SAMPLES, true, false);
        assert_eq!(first[0], 1.0);
        assert!((first[SEGMENT_SAMPLES - OVERLAP_SAMPLES] - 1.0).abs() < 1e-6);
        assert_eq!(*first.last().unwrap(), 0.0);

        let last = crossfade_weights(SEGMENT_SAMPLES, OVERLAP_SAMPLES, false, true);
        assert_eq!(last[0], 0.0);
        assert_eq!(*last.last().unwrap(), 1.0);

        let only = crossfade_weights(SEGMENT_SAMPLES, OVERLAP_SAMPLES, true, true);
        assert!(only.iter().all(|&weight| weight == 1.0));
    }

    #[test]
    fn adjacent_chunks_crossfade_weights_sum_to_one_in_the_overlap_region() {
        // 隣接する2チャンクが重なる区間では、片方の立ち上がりと
        // もう片方の立ち下がりの重みの和が常に1でなければならない
        // （そうでないと重なり区間だけ音量が変化してしまう）。
        let weights = crossfade_weights(SEGMENT_SAMPLES, OVERLAP_SAMPLES, false, false);
        for i in 0..OVERLAP_SAMPLES {
            let rising = weights[i];
            let falling = weights[SEGMENT_SAMPLES - OVERLAP_SAMPLES + i];
            assert!(
                (rising + falling - 1.0).abs() < 1e-5,
                "mismatch at {i}: {rising} + {falling}"
            );
        }
    }

    #[test]
    fn reconstruction_matches_a_known_signal_across_chunk_boundaries() {
        // 実モデルなしで、チャンク分割＋クロスフェード再構成の全体を
        // 検証する。フェイクの推論が常に一定値（左=1.0、右=-1.0）を
        // 返すとき、チャンクの境界をまたいでも再構成結果がその一定値の
        // ままであるべき（重み付き平均なので、境界で値が変化しないなら
        // 正規化後も同じ値に戻る）。
        let total_samples = SEGMENT_SAMPLES + HOP_SAMPLES * 3;
        let chunks = plan_separation_chunks(total_samples, SEGMENT_SAMPLES, HOP_SAMPLES);
        assert!(
            chunks.len() > 1,
            "test needs multiple chunks to be meaningful"
        );
        let cancel = AtomicBool::new(false);

        let result = reconstruct_windowed(
            total_samples,
            &chunks,
            OVERLAP_SAMPLES,
            &cancel,
            |_, _| {},
            |_chunk| Ok([1.0_f32, -1.0].repeat(SEGMENT_SAMPLES)),
        )
        .unwrap();

        assert_eq!(result.len(), total_samples * SEPARATOR_CHANNELS);
        for position in 0..total_samples {
            let left = result[position * SEPARATOR_CHANNELS];
            let right = result[position * SEPARATOR_CHANNELS + 1];
            assert!(
                (left - 1.0).abs() < 1e-4,
                "left drifted at {position}: {left}"
            );
            assert!(
                (right + 1.0).abs() < 1e-4,
                "right drifted at {position}: {right}"
            );
        }
    }

    #[test]
    fn cancellation_stops_before_completion() {
        let total_samples = SEGMENT_SAMPLES + HOP_SAMPLES * 3;
        let chunks = plan_separation_chunks(total_samples, SEGMENT_SAMPLES, HOP_SAMPLES);
        let cancel = AtomicBool::new(true);
        let result = reconstruct_windowed(
            total_samples,
            &chunks,
            OVERLAP_SAMPLES,
            &cancel,
            |_, _| {},
            |_chunk| Ok([0.0_f32; 2].repeat(SEGMENT_SAMPLES)),
        );
        assert!(result.is_err());
    }
}
