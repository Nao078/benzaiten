//! 発音ガイド（カタカナ読み）は歌詞から派生する表示・練習用データであり、
//! Forced Alignmentには一切関与しない。

/// CMUdict＋ARPAbet音素をもとにした英語→歌唱向けカタカナ変換エンジン。
pub mod english;
/// 生成済みカタカナへ、歌唱向けの音の連結規則を適用する処理。
pub mod katakana;
/// ARPAbet音素（CMUdict由来）を表す型と、母音判定などの補助関数。
pub mod phoneme;

use phoneme::Phoneme;

/// 発音ガイド生成の結果。カタカナ読み本体に加え、音素列（取得できた場合）と、
/// 辞書に無くフォールバック読みになった単語の一覧を持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct PronunciationResult {
    pub reading: String,
    pub phonemes: Option<Vec<Phoneme>>,
    pub unknown_words: Vec<String>,
}

/// 原文の歌詞行から発音ガイドを生成するエンジンの共通インターフェース。
/// 現状の実装は[`english::EnglishPronunciationEngine`]のみ。
pub trait PronunciationEngine {
    fn generate(&self, original: &str) -> Result<PronunciationResult, String>;
}
