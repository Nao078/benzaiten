//! 歌唱練習ガイド用の、文脈を考慮した米語発音処理。

use std::{collections::HashMap, sync::OnceLock};

use super::{katakana, phoneme::Phoneme, PronunciationEngine, PronunciationResult};

/// 同梱のCMU英語発音辞書（固定コミット、詳細はREADME参照）。
const CMUDICT: &str = include_str!("../../assets/cmudict/cmudict.dict");

/// CMUdict＋弱形・連結規則にもとづく英語発音エンジン。状態を持たないため
/// ゼロサイズのマーカー型で、[`dictionary`]内部の静的辞書を共有する。
#[derive(Debug, Default, Clone, Copy)]
pub struct EnglishPronunciationEngine;

/// 1単語分の発音情報。`link_to_next`は、次の単語と連結発音（フラップ・
/// yod融合など）の対象になり得るかどうかを表す（句読点の直前ではfalse）。
#[derive(Debug)]
struct WordPronunciation {
    word: String,
    phones: Vec<String>,
    link_to_next: bool,
}

/// トークン化直後の単語（辞書引き前）。構造は[`WordPronunciation`]と同じ
/// 役割の`link_to_next`を持つが、まだ発音情報がない点が異なる。
#[derive(Debug)]
struct WordToken {
    word: String,
    link_to_next: bool,
}

impl PronunciationEngine for EnglishPronunciationEngine {
    fn generate(&self, original: &str) -> Result<PronunciationResult, String> {
        let words = tokenize(original);
        if words.is_empty() {
            return Ok(PronunciationResult {
                reading: String::new(),
                phonemes: Some(Vec::new()),
                unknown_words: Vec::new(),
            });
        }
        let dictionary = dictionary();
        let mut unknown_words = Vec::new();
        // 辞書にある単語はその音素列を、辞書に無い単語（固有名詞など）は
        // フォールバック（綴り読み）を使い、未知語として記録しておく。
        let mut pronunciations: Vec<WordPronunciation> = words
            .into_iter()
            .map(|token| {
                let phones = dictionary
                    .get(token.word.as_str())
                    .cloned()
                    .unwrap_or_else(|| {
                        unknown_words.push(token.word.clone());
                        fallback_pronunciation(&token.word)
                    });
                WordPronunciation {
                    word: token.word,
                    phones,
                    link_to_next: token.link_to_next,
                }
            })
            .collect();
        apply_weak_forms(&mut pronunciations);
        apply_connected_speech(&mut pronunciations);

        // 単語境界には`|`マーカーを挟んでおき、カタカナ変換側が単語の
        // 切れ目を認識できるようにする。
        let mut phonemes = Vec::new();
        for (index, word) in pronunciations.into_iter().enumerate() {
            if index > 0 {
                phonemes.push(Phoneme::new("|"));
            }
            phonemes.extend(word.phones.into_iter().map(Phoneme::new));
        }
        let reading = katakana::from_phonemes(&phonemes)?;
        Ok(PronunciationResult {
            reading,
            phonemes: Some(phonemes),
            unknown_words,
        })
    }
}

/// CMUdictをパースした単語→ARPAbet音素列の辞書。初回アクセス時に一度だけ
/// 構築し、以降は使い回す（`OnceLock`によるプロセス内キャッシュ）。
fn dictionary() -> &'static HashMap<&'static str, Vec<String>> {
    static DICTIONARY: OnceLock<HashMap<&'static str, Vec<String>>> = OnceLock::new();
    DICTIONARY.get_or_init(|| {
        let mut entries = HashMap::with_capacity(135_000);
        for line in CMUDICT.lines() {
            let Some((entry, pronunciation)) = line.split_once(' ') else {
                continue;
            };
            // `WORD(1)`のような異形（第2発音以降）と重複エントリは無視し、
            // 各単語の最初（＝最も一般的）の発音だけを採用する。
            if entry.ends_with(')') || entries.contains_key(entry) {
                continue;
            }
            entries.insert(
                entry,
                pronunciation
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
            );
        }
        entries
    })
}

/// 原文を英単語トークンへ分割する。アポストロフィは単語の内側でのみ
/// 許容し（`don't`など）、句読点の直前の単語は`link_to_next = false`にして
/// 連結発音の対象から外す。
fn tokenize(input: &str) -> Vec<WordToken> {
    let normalized = input.replace(['’', '‘'], "'").to_ascii_lowercase();
    let mut words = Vec::new();
    let mut current = String::new();
    for character in normalized.chars() {
        if character.is_ascii_alphabetic() || (character == '\'' && !current.is_empty()) {
            current.push(character);
        } else {
            if !current.is_empty() {
                words.push(WordToken {
                    word: std::mem::take(&mut current),
                    link_to_next: true,
                });
            }
            if matches!(
                character,
                '.' | ',' | '!' | '?' | ';' | ':' | '(' | ')' | '[' | ']'
            ) {
                if let Some(previous) = words.last_mut() {
                    previous.link_to_next = false;
                }
            }
        }
    }
    if !current.is_empty() {
        words.push(WordToken {
            word: current,
            link_to_next: false,
        });
    } else if let Some(previous) = words.last_mut() {
        previous.link_to_next = false;
    }
    words
}

/// 機能語（冠詞・前置詞など）を、辞書の強形ではなく実際の発話でよく
/// 使われる弱形の音素列へ置き換える（例："a"→強形"EY1"ではなく弱形
/// "AH0"、"the"は次が母音始まりかどうかで読みが変わる）。歌詞テキストの
/// `link_to_next`情報だけで判定するため完全ではないが、素の辞書読みより
/// 自然な歌唱ガイドになる。
fn apply_weak_forms(words: &mut [WordPronunciation]) {
    for index in 0..words.len() {
        let next_starts_with_vowel = words
            .get(index + 1)
            .and_then(|word| word.phones.first())
            .is_some_and(|phone| is_vowel(phone));
        let replacement: Option<&[&str]> = match words[index].word.as_str() {
            "a" => Some(&["AH0"]),
            "an" => Some(&["AH0", "N"]),
            "and" if index + 1 < words.len() => Some(&["AH0", "N"]),
            "of" => Some(&["AH0", "V"]),
            "for" if index + 1 < words.len() => Some(&["F", "ER0"]),
            "the" if next_starts_with_vowel => Some(&["DH", "IY0"]),
            "the" => Some(&["DH", "AH0"]),
            "to" if !next_starts_with_vowel && index > 0 => Some(&["T", "AH0"]),
            _ => None,
        };
        if let Some(replacement) = replacement {
            words[index].phones = replacement
                .iter()
                .map(|phone| (*phone).to_owned())
                .collect();
        }
        // "our/their/your lives"のような所有格の後の"lives"は
        // 名詞（人生・複数形、/laɪvz/）として読む特例。
        // （動詞"live"の三人称単数形/lɪvz/と発音が異なるため）
        if words[index].word == "lives"
            && index > 0
            && matches!(words[index - 1].word.as_str(), "our" | "their" | "your")
        {
            words[index].phones = ["L", "AY1", "V", "Z"]
                .into_iter()
                .map(str::to_owned)
                .collect();
        }
    }
}

/// 単語をまたぐ連結発音（フラップ化・yod融合）を音素列へ反映する。
/// 対象は`link_to_next`な（＝句読点で区切られていない）隣接語のみ。
///
/// - "T"/"D" + "Y"（例："got you"）→ "CH"/"JH"へ融合（yod coalescence）。
/// - 母音に挟まれた"T"/"D"（前の単語末が母音、次の単語頭も母音）→
///   アメリカ英語の弾き音"DX"に置き換える（例："get it"→ ゲリ）。
fn apply_connected_speech(words: &mut [WordPronunciation]) {
    for index in 0..words.len().saturating_sub(1) {
        if !words[index].link_to_next {
            continue;
        }
        let (left, right) = words.split_at_mut(index + 1);
        let previous = &mut left[index].phones;
        let next = &mut right[0].phones;
        let Some(last) = previous.last_mut() else {
            continue;
        };
        let Some(first) = next.first_mut() else {
            continue;
        };
        if matches!(last.as_str(), "T" | "D") && first == "Y" {
            *first = if last == "T" { "CH" } else { "JH" }.to_owned();
            previous.pop();
            previous.append(next);
        } else if matches!(last.as_str(), "T" | "D")
            && is_vowel(first)
            && previous
                .get(previous.len().saturating_sub(2))
                .is_some_and(|phone| is_vowel(phone))
        {
            previous.pop();
            next.insert(0, "DX".to_owned());
            previous.append(next);
        }
    }
}

fn is_vowel(phone: &str) -> bool {
    Phoneme::new(phone).is_vowel()
}

/// CMUdictに無い単語（固有名詞・造語など）向けのフォールバック発音。
/// 各アルファベットを「その文字名の読み」（B→ビー、など）に変換して
/// 綴り読みにする。近似でしかないため、生成後にGUI上で未知語として
/// 通知しユーザーに確認してもらう前提。
fn fallback_pronunciation(word: &str) -> Vec<String> {
    let mut phones = Vec::new();
    for character in word.chars().filter(char::is_ascii_alphabetic) {
        let spelled: &[&str] = match character {
            'a' => &["EY1"],
            'b' => &["B", "IY1"],
            'c' => &["S", "IY1"],
            'd' => &["D", "IY1"],
            'e' => &["IY1"],
            'f' => &["EH1", "F"],
            'g' => &["JH", "IY1"],
            'h' => &["EY1", "CH"],
            'i' => &["AY1"],
            'j' => &["JH", "EY1"],
            'k' => &["K", "EY1"],
            'l' => &["EH1", "L"],
            'm' => &["EH1", "M"],
            'n' => &["EH1", "N"],
            'o' => &["OW1"],
            'p' => &["P", "IY1"],
            'q' => &["K", "Y", "UW1"],
            'r' => &["AA1", "R"],
            's' => &["EH1", "S"],
            't' => &["T", "IY1"],
            'u' => &["Y", "UW1"],
            'v' => &["V", "IY1"],
            'w' => &["D", "AH1", "B", "AH0", "L", "Y", "UW0"],
            'x' => &["EH1", "K", "S"],
            'y' => &["W", "AY1"],
            'z' => &["Z", "IY1"],
            _ => &[],
        };
        phones.extend(spelled.iter().map(|phone| (*phone).to_owned()));
    }
    phones
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_and_context_create_singing_guide() {
        let result = EnglishPronunciationEngine
            .generate("We are living our lives")
            .unwrap();
        assert_eq!(result.reading, "ウィー アー リヴィング アウアー ライヴズ");
        assert!(result.unknown_words.is_empty());
    }

    #[test]
    fn applies_flap_and_yod_coalescence_between_words() {
        let get_it = EnglishPronunciationEngine.generate("get it").unwrap();
        let did_you = EnglishPronunciationEngine.generate("did you").unwrap();
        assert!(get_it.reading.starts_with("ゲリ"));
        assert!(did_you.reading.contains("ジュ"));
    }

    #[test]
    fn does_not_connect_across_punctuation_or_after_a_consonant_cluster() {
        let punctuation = EnglishPronunciationEngine.generate("out! Yes").unwrap();
        let world_is = EnglishPronunciationEngine.generate("world is").unwrap();
        assert_eq!(punctuation.reading, "アウト イェス");
        assert_eq!(world_is.reading, "ワールド イズ");
    }

    #[test]
    fn renders_common_singing_clusters_without_extra_gemination() {
        let result = EnglishPronunciationEngine
            .generate("That's sacrifice excited forced you")
            .unwrap();
        assert_eq!(
            result.reading,
            "ザッツ サクラファイス イクサイタド フォースチュー"
        );
    }

    #[test]
    fn reports_unknown_words_without_failing_the_line() {
        let result = EnglishPronunciationEngine.generate("qzxqzx").unwrap();
        assert!(!result.reading.is_empty());
        assert_eq!(result.unknown_words, ["qzxqzx"]);
    }
}
