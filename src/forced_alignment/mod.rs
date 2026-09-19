//! 正解歌詞と音響モデルの出力（emissions）をCTC Forced Alignmentで照合する。
//!
//! パイプラインは3段階：[`tokenizer`]が歌詞をモデルのトークンID列へ変換し、
//! [`trellis`]がCTC Viterbiアルゴリズムでフレーム単位の対応を求め、
//! [`resolver`]がその結果を歌詞行ごとの開始・終了時刻へ集約する。

pub mod model;
pub mod resolver;
pub mod tokenizer;
pub mod trellis;

use crate::domain::lyrics::LyricLine;

pub use model::AcousticModel;

/// 英語Wav2Vec2モデルの出力に対してForced Alignmentを実行する。
pub fn align(
    lyrics: &[LyricLine],
    emissions: &[Vec<f32>],
    frame_duration_ms: f64,
) -> Result<Vec<LyricLine>, String> {
    let transcript = tokenizer::tokenize_english(lyrics)?;
    let aligned = trellis::force_align(
        emissions,
        &transcript.token_ids,
        tokenizer::BLANK_ID,
        frame_duration_ms,
    )?;
    resolver::resolve_lines(lyrics, &transcript, &aligned, frame_duration_ms)
}

/// 日本語Wav2Vec2モデルの出力に対してForced Alignmentを実行する。
/// 語彙（トークン→ID対応）は外部のtokenizer.jsonから読み込んだものを使う。
pub fn align_japanese(
    lyrics: &[LyricLine],
    emissions: &[Vec<f32>],
    frame_duration_ms: f64,
    vocabulary: &tokenizer::Vocabulary,
) -> Result<Vec<LyricLine>, String> {
    let transcript = tokenizer::tokenize_japanese(lyrics, vocabulary)?;
    let aligned = trellis::force_align(
        emissions,
        &transcript.token_ids,
        vocabulary.blank_id,
        frame_duration_ms,
    )?;
    resolver::resolve_lines(lyrics, &transcript, &aligned, frame_duration_ms)
}
