//! CMUdict／ARPAbet形式の音素を表す小さな値型。

/// ARPAbet記法の1音素（例：`"AH0"`, `"T"`）。CMUdictの発音表記そのままの
/// 文字列を保持する。
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

    /// 末尾のストレス表記（0/1/2）を除いた音素本体を返す
    /// （例：`"AH0"` → `"AH"`）。
    pub fn base(&self) -> &str {
        self.symbol.trim_end_matches(['0', '1', '2'])
    }

    /// ARPAbetの母音音素かどうかを判定する。連結規則
    /// （母音間フラップ、yod融合など）の判定に使う。
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

    /// 単語境界を表す特別なマーカー（実際の音素ではない）かどうか。
    pub fn is_word_boundary(&self) -> bool {
        self.symbol == "|"
    }
}
