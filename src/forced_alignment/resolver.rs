//! `trellis`が求めたトークンごとのフレーム区間を、歌詞行ごとの
//! 開始・終了時刻とconfidenceへ集約する処理。

use crate::domain::{lyrics::LyricLine, project::TimestampSource};

use super::{tokenizer::Transcript, trellis::TokenSpan};

/// `spans`（トークンごとのフレーム区間）を`transcript.line_indices`で
/// 歌詞行ごとにグループ化し、各行の最初のトークンの開始フレームと
/// 最後のトークンの終了フレームから`start_ms`/`end_ms`を求める。
/// confidenceはその行に属するトークンの信頼度の平均値。
///
/// `TimestampSource::Manual`が付いている行（ユーザーが手動で時刻を
/// 設定・調整した行）は、再アライメントで上書きされないようスキップする。
/// 行に対応するトークンが1つも見つからない場合（例えば区切り記号のみの
/// 行）は時刻を設定しない。
pub fn resolve_lines(
    lyrics: &[LyricLine],
    transcript: &Transcript,
    spans: &[TokenSpan],
    frame_duration_ms: f64,
) -> Result<Vec<LyricLine>, String> {
    if spans.len() != transcript.token_ids.len()
        || transcript.line_indices.len() != transcript.token_ids.len()
    {
        return Err("forced-alignment token metadata is inconsistent".to_owned());
    }
    if !frame_duration_ms.is_finite() || frame_duration_ms <= 0.0 {
        return Err("forced-alignment frame duration must be positive".to_owned());
    }

    let mut result = lyrics.to_vec();
    for (line_index, line) in result.iter_mut().enumerate() {
        if matches!(line.timestamp_source, Some(TimestampSource::Manual)) {
            continue;
        }
        line.start_ms = None;
        line.end_ms = None;
        line.confidence = None;
        line.timestamp_source = None;

        // この行に属し、かつ行区切りトークン自体ではないスパンだけを集める。
        let aligned: Vec<_> = spans
            .iter()
            .zip(&transcript.token_ids)
            .zip(&transcript.line_indices)
            .filter(|((_, token), owner)| {
                **owner == line_index && transcript.delimiter_id != Some(**token)
            })
            .map(|((span, _), _)| span)
            .collect();
        let (Some(first), Some(last)) = (aligned.first(), aligned.last()) else {
            continue;
        };
        line.start_ms = Some((first.start_frame as f64 * frame_duration_ms).round() as u64);
        line.end_ms = Some((last.end_frame as f64 * frame_duration_ms).round() as u64);
        line.confidence =
            Some(aligned.iter().map(|span| span.confidence).sum::<f32>() / aligned.len() as f32);
        line.timestamp_source = Some(TimestampSource::ForcedAlignment);
    }
    Ok(result)
}
