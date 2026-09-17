#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phoneme {
    pub symbol: String,
}

impl Phoneme {
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
        }
    }

    pub fn base(&self) -> &str {
        self.symbol.trim_end_matches(['0', '1', '2'])
    }

    pub fn is_vowel(&self) -> bool {
        matches!(
            self.base(),
            "AA" | "AE"
                | "AH"
                | "AO"
                | "AW"
                | "AY"
                | "EH"
                | "ER"
                | "EY"
                | "IH"
                | "IY"
                | "OW"
                | "OY"
                | "UH"
                | "UW"
        )
    }

    pub fn is_word_boundary(&self) -> bool {
        self.symbol == "|"
    }
}
