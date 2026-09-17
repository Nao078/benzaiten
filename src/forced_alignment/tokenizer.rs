use crate::domain::lyrics::LyricLine;
use std::{collections::HashMap, path::Path};
use unicode_normalization::UnicodeNormalization;

pub const BLANK_ID: usize = 0;
pub const WORD_DELIMITER_ID: usize = 4;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub token_ids: Vec<usize>,
    pub line_indices: Vec<usize>,
    pub delimiter_id: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Vocabulary {
    tokens: HashMap<char, usize>,
    pub blank_id: usize,
    delimiter_id: Option<usize>,
}

impl Vocabulary {
    /// Load either Hugging Face `vocab.json` or `tokenizer.json`.
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

pub fn tokenize_english(lyrics: &[LyricLine]) -> Result<Transcript, String> {
    tokenize_with(
        lyrics,
        english_token_id,
        Some(WORD_DELIMITER_ID),
        "lyrics contain no English characters that can be aligned",
        false,
    )
}

/// Backward-compatible English tokenizer entry point.
pub fn tokenize(lyrics: &[LyricLine]) -> Result<Transcript, String> {
    tokenize_english(lyrics)
}

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

fn is_japanese_character(character: char) -> bool {
    matches!(character as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff)
}

fn is_alignable_character(character: char) -> bool {
    is_japanese_character(character) || character.is_ascii_alphanumeric() || character == 'ー'
}

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
