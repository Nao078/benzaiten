//! CTC forced alignment of canonical lyrics against acoustic-model emissions.

pub mod model;
pub mod resolver;
pub mod tokenizer;
pub mod trellis;

use crate::domain::lyrics::LyricLine;

pub use model::AcousticModel;

pub fn align(
    lyrics: &[LyricLine],
    emissions: &[Vec<f32>],
    frame_duration_ms: f64,
) -> Result<Vec<LyricLine>, String> {
    let transcript = tokenizer::tokenize_english(lyrics)?;
    let aligned = trellis::force_align(emissions, &transcript.token_ids, tokenizer::BLANK_ID)?;
    resolver::resolve_lines(lyrics, &transcript, &aligned, frame_duration_ms)
}

pub fn align_japanese(
    lyrics: &[LyricLine],
    emissions: &[Vec<f32>],
    frame_duration_ms: f64,
    vocabulary: &tokenizer::Vocabulary,
) -> Result<Vec<LyricLine>, String> {
    let transcript = tokenizer::tokenize_japanese(lyrics, vocabulary)?;
    let aligned = trellis::force_align(emissions, &transcript.token_ids, vocabulary.blank_id)?;
    resolver::resolve_lines(lyrics, &transcript, &aligned, frame_duration_ms)
}
