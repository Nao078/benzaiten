use crate::domain::{lyrics::LyricLine, project::TimestampSource};

use super::{tokenizer::Transcript, trellis::TokenSpan};

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
