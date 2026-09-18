//! 歌詞テキストを、音響モデルの語彙に対応したトークンID列（transcript）へ
//! 変換する処理。英語はモデルに焼き込まれた固定語彙を、日本語は外部の
//! tokenizer.jsonから読み込んだ語彙を使う。

use crate::domain::lyrics::LyricLine;
use std::{collections::HashMap, path::Path};
use unicode_normalization::UnicodeNormalization;

/// CTCのblank（空白）トークンID。英語モデル・日本語モデルとも通常は0。
pub const BLANK_ID: usize = 0;
/// `facebook/wav2vec2-base-960h`の語彙における単語区切り（`|`）トークンID。
pub const WORD_DELIMITER_ID: usize = 4;

/// Forced Alignmentに使う言語。GUIの`Auto/en/jp`選択に対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentLanguage {
    English,
    Japanese,
}

impl AlignmentLanguage {
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Japanese => "jp",
        }
    }
}

/// トークン化された歌詞全体。`token_ids[i]`がどの歌詞行に属するかを
/// `line_indices[i]`が示し、[`resolver`](super::resolver)がこれを使って
/// トークンごとのアラインメント結果を行単位に集約する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub token_ids: Vec<usize>,
    pub line_indices: Vec<usize>,
    /// 単語区切りトークンのID（あれば）。`resolver`はこのIDのトークンを
    /// 実際の歌詞内容ではなく区切りとして扱い、行の時刻集計から除外する。
    pub delimiter_id: Option<usize>,
}

/// 日本語Forced Alignment用の、外部ファイルから読み込んだ文字→トークンID
/// の対応表。
#[derive(Debug, Clone)]
pub struct Vocabulary {
    tokens: HashMap<char, usize>,
    pub blank_id: usize,
    delimiter_id: Option<usize>,
}

impl Vocabulary {
    /// Hugging Faceの`vocab.json`または`tokenizer.json`のどちらの形式も
    /// 読み込む。`tokenizer.json`は`model.vocab`、`vocab.json`は
    /// トップレベルの`vocab`（または直下）に語彙マップを持つため、
    /// その両方を順に探す。1文字のトークンだけを文字→IDの対応として
    /// 採用する（サブワード等の複数文字トークンは無視する）。
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not read vocabulary {}: {error}", path.display()))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid vocabulary JSON {}: {error}", path.display()))?;
        let vocabulary = value
            .get("model")
            .and_then(|model| model.get("vocab"))
            .or_else(|| value.get("vocab"))
            .unwrap_or(&value)
            .as_object()
            .ok_or_else(|| format!("vocabulary map was not found in {}", path.display()))?;

        let blank_id = ["<pad>", "[PAD]", "<blank>"]
            .iter()
            .find_map(|token| vocabulary.get(*token)?.as_u64())
            .unwrap_or(BLANK_ID as u64) as usize;
        let delimiter_id = vocabulary
            .get("|")
            .and_then(serde_json::Value::as_u64)
            .map(|id| id as usize);
        let tokens = vocabulary
            .iter()
            .filter_map(|(token, id)| {
                let mut characters = token.chars();
                let character = characters.next()?;
                let id = id.as_u64()? as usize;
                (characters.next().is_none()).then_some((character, id))
            })
            .collect::<HashMap<_, _>>();
        if tokens.is_empty() {
            return Err(format!("{} contains no character tokens", path.display()));
        }
        Ok(Self {
            tokens,
            blank_id,
            delimiter_id,
        })
    }
}

/// GUI/CLIの言語モード文字列（`auto`/`en`/`jp`/`ja`）を[`AlignmentLanguage`]
/// へ解決する。`auto`の場合は歌詞の文字種から自動判定する。
pub fn resolve_language(mode: &str, lyrics: &[LyricLine]) -> Result<AlignmentLanguage, String> {
    match mode.to_ascii_lowercase().as_str() {
        "auto" => Ok(detect_language(lyrics)),
        "en" => Ok(AlignmentLanguage::English),
        "jp" | "ja" => Ok(AlignmentLanguage::Japanese),
        _ => Err(format!(
            "unsupported lyric language mode {mode:?}; choose Auto, en, or jp"
        )),
    }
}

/// 歌詞にひらがな・カタカナ・漢字が1文字でも含まれていれば日本語、
/// それ以外は英語と判定する。
pub fn detect_language(lyrics: &[LyricLine]) -> AlignmentLanguage {
    if lyrics
        .iter()
        .flat_map(|line| line.original_text.chars())
        .any(is_japanese_character)
    {
        AlignmentLanguage::Japanese
    } else {
        AlignmentLanguage::English
    }
}

/// 英語モデル（`facebook/wav2vec2-base-960h`）の固定語彙で歌詞をトークン化する。
pub fn tokenize_english(lyrics: &[LyricLine]) -> Result<Transcript, String> {
    tokenize_with(
        lyrics,
        english_token_id,
        Some(WORD_DELIMITER_ID),
        "lyrics contain no English characters that can be aligned",
        false,
    )
}

/// 後方互換のための、英語トークナイザへのエイリアス。
pub fn tokenize(lyrics: &[LyricLine]) -> Result<Transcript, String> {
    tokenize_english(lyrics)
}

/// 日本語モデル用に、外部から読み込んだ[`Vocabulary`]で歌詞をトークン化
/// する。トークン化後、語彙に無いアライメント対象文字（ひらがな・カタカナ・
/// 漢字やASCII英数字など）が残っていれば、モデルが扱えない文字として
/// エラーにする（珍しい漢字などをユーザーに気付かせるため）。
pub fn tokenize_japanese(
    lyrics: &[LyricLine],
    vocabulary: &Vocabulary,
) -> Result<Transcript, String> {
    let transcript = tokenize_with(
        lyrics,
        |character| vocabulary.tokens.get(&character).copied(),
        vocabulary.delimiter_id,
        "lyrics contain no Japanese characters that can be aligned",
        true,
    )?;
    let mut unsupported = lyrics
        .iter()
        .flat_map(|line| {
            normalize_japanese(&line.original_text)
                .chars()
                .collect::<Vec<_>>()
        })
        .filter(|character| {
            is_alignable_character(*character) && !vocabulary.tokens.contains_key(character)
        })
        .collect::<Vec<_>>();
    unsupported.sort_unstable();
    unsupported.dedup();
    if !unsupported.is_empty() {
        let characters = unsupported.iter().take(12).collect::<String>();
        return Err(format!(
            "日本語モデルの語彙にない文字があります: {characters}。ひらがな・カタカナへ置き換えてください"
        ));
    }
    Ok(transcript)
}

/// 英語・日本語共通のトークン化処理本体。行ごとにテキストを正規化し、
/// 語彙に存在する文字だけをトークンへ変換する（語彙にない文字は単に
/// スキップする。日本語の場合は呼び出し元の`tokenize_japanese`が事後的に
/// 検出してエラーにする）。
///
/// スペース（単語の切れ目）を見つけるたびに`pending_delimiter`を立て、
/// 次に有効なトークンが来た時点で（連続する空白やスキップされた文字を
/// 挟んでいても）区切りトークンを1つだけ挿入する。これにより、
/// 「単語の先頭」や「行の先頭」で余分な区切りが重複しないようにしている。
fn tokenize_with(
    lyrics: &[LyricLine],
    token_id: impl Fn(char) -> Option<usize>,
    delimiter_id: Option<usize>,
    empty_error: &str,
    japanese: bool,
) -> Result<Transcript, String> {
    let mut token_ids = Vec::new();
    let mut line_indices = Vec::new();
    for (line_index, line) in lyrics.iter().enumerate() {
        let normalized = if japanese {
            normalize_japanese(&line.original_text)
        } else {
            normalize_english(&line.original_text)
        };
        // 直前の行との間、および行内の最初の単語の前にも区切りを入れる
        // （最初の行の最初の単語の前だけは入れない）。
        let mut pending_delimiter = !token_ids.is_empty();
        for character in normalized.chars() {
            if character == ' ' {
                pending_delimiter = !token_ids.is_empty();
                continue;
            }
            let Some(token) = token_id(character) else {
                continue;
            };
            if pending_delimiter {
                if let Some(delimiter) = delimiter_id {
                    token_ids.push(delimiter);
                    line_indices.push(line_index);
                }
                pending_delimiter = false;
            }
            token_ids.push(token);
            line_indices.push(line_index);
        }
    }
    if token_ids.is_empty() {
        return Err(empty_error.to_owned());
    }
    Ok(Transcript {
        token_ids,
        line_indices,
        delimiter_id,
    })
}

/// 英語テキストの正規化：曲がった引用符をストレートに、アルファベットを
/// 大文字に統一し、空白類とハイフンは半角スペースへ、それ以外の
/// 記号は削除する。
fn normalize_english(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '’' | '‘' | '`' => '\'',
            character if character.is_ascii_alphabetic() => character.to_ascii_uppercase(),
            character if character.is_whitespace() || character == '-' => ' ',
            _ => '\0',
        })
        .filter(|character| *character != '\0')
        .collect()
}

/// 日本語テキストの正規化：NFKC正規化（全角英数字の半角化、互換文字の
/// 統一など）を行い、空白類を半角スペースへ、ASCIIアルファベットは
/// 大文字へ統一する。それ以外の文字（ひらがな・カタカナ・漢字など）は
/// そのまま残す。
fn normalize_japanese(text: &str) -> String {
    text.nfkc()
        .map(|character| {
            if character.is_whitespace() {
                ' '
            } else if character.is_ascii_alphabetic() {
                character.to_ascii_uppercase()
            } else {
                character
            }
        })
        .collect()
}

/// ひらがな・カタカナ・CJK統合漢字（拡張Aを含む）の範囲かどうか。
fn is_japanese_character(character: char) -> bool {
    matches!(character as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff)
}

/// 日本語アライメントにおいて「本来揃えるべき」文字かどうか。
/// 語彙に存在しないこの種の文字が残っていれば、日本語モデルが
/// 扱えない文字としてエラー対象にする（長音記号「ー」も含む）。
fn is_alignable_character(character: char) -> bool {
    is_japanese_character(character) || character.is_ascii_alphanumeric() || character == 'ー'
}

/// `facebook/wav2vec2-base-960h`の`vocab.json`に固定された、
/// アルファベット→トークンIDの対応表。このモデル専用の並び順であり、
/// 他の英語Wav2Vec2モデルでは異なる可能性がある点に注意。
fn english_token_id(character: char) -> Option<usize> {
    Some(match character {
        '|' => WORD_DELIMITER_ID,
        'E' => 5,
        'T' => 6,
        'A' => 7,
        'O' => 8,
        'N' => 9,
        'I' => 10,
        'H' => 11,
        'S' => 12,
        'R' => 13,
        'D' => 14,
        'L' => 15,
        'U' => 16,
        'M' => 17,
        'W' => 18,
        'C' => 19,
        'F' => 20,
        'G' => 21,
        'Y' => 22,
        'P' => 23,
        'B' => 24,
        'V' => 25,
        'K' => 26,
        '\'' => 27,
        'X' => 28,
        'J' => 29,
        'Q' => 30,
        'Z' => 31,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        detect_language, tokenize_english, tokenize_japanese, AlignmentLanguage, Vocabulary,
    };
    use crate::domain::lyrics::parse_lyrics;
    use std::collections::HashMap;

    #[test]
    fn preserves_line_ownership_while_normalizing_english() {
        let lyrics = parse_lyrics("Hello, world!\nDon't-stop");
        let transcript = tokenize_english(&lyrics).unwrap();
        assert_eq!(transcript.delimiter_id, Some(4));
        assert_eq!(transcript.line_indices[0], 0);
        assert_eq!(*transcript.line_indices.last().unwrap(), 1);
        assert_eq!(transcript.token_ids.len(), transcript.line_indices.len());
    }

    #[test]
    fn auto_detects_japanese_scripts() {
        assert_eq!(
            detect_language(&parse_lyrics("君と歌う")),
            AlignmentLanguage::Japanese
        );
        assert_eq!(
            detect_language(&parse_lyrics("We sing")),
            AlignmentLanguage::English
        );
    }

    #[test]
    fn tokenizes_japanese_with_external_vocabulary() {
        let vocabulary = Vocabulary {
            tokens: HashMap::from([('君', 5), ('と', 6), ('歌', 7), ('う', 8), ('|', 4)]),
            blank_id: 0,
            delimiter_id: Some(4),
        };
        let transcript = tokenize_japanese(&parse_lyrics("君と\n歌う"), &vocabulary).unwrap();
        assert_eq!(transcript.token_ids, vec![5, 6, 4, 7, 8]);
        assert_eq!(transcript.line_indices, vec![0, 0, 1, 1, 1]);
    }

    #[test]
    fn loads_hugging_face_tokenizer_json() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            r#"{"model":{"type":"WordLevel","vocab":{"<pad>":0,"|":4,"君":5}}}"#,
        )
        .unwrap();
        let vocabulary = Vocabulary::load(file.path()).unwrap();
        assert_eq!(vocabulary.blank_id, 0);
        assert_eq!(vocabulary.delimiter_id, Some(4));
        assert_eq!(vocabulary.tokens.get(&'君'), Some(&5));
    }
}
