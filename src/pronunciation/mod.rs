//! Pronunciation guides are derived data and never participate in alignment.

pub mod english;
pub mod katakana;
pub mod phoneme;

use phoneme::Phoneme;

#[derive(Debug, Clone, PartialEq)]
pub struct PronunciationResult {
    pub reading: String,
    pub phonemes: Option<Vec<Phoneme>>,
    pub unknown_words: Vec<String>,
}

pub trait PronunciationEngine {
    fn generate(&self, original: &str) -> Result<PronunciationResult, String>;
}
