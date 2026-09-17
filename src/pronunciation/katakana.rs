//! ARPAbet to a readable Japanese approximation. The output intentionally
//! favors singing practice over strict linguistic transcription.

use super::phoneme::Phoneme;

pub fn from_phonemes(phonemes: &[Phoneme]) -> Result<String, String> {
    let mut words = Vec::new();
    let mut current = Vec::new();
    for phoneme in phonemes {
        if phoneme.is_word_boundary() {
            if !current.is_empty() {
                words.push(render_word(&current)?);
                current.clear();
            }
        } else {
            current.push(phoneme.clone());
        }
    }
    if !current.is_empty() {
        words.push(render_word(&current)?);
    }
    Ok(words.join(" "))
}

fn render_word(phones: &[Phoneme]) -> Result<String, String> {
    let mut output = String::new();
    let mut index = 0;
    let mut previous_vowel: Option<&str> = None;
    let mut previous_consonant: Option<&str> = None;
    while index < phones.len() {
        if phones[index].is_vowel() {
            output.push_str(vowel(&phones[index]));
            previous_vowel = Some(phones[index].base());
            previous_consonant = None;
            index += 1;
            continue;
        }
        let start = index;
        while index < phones.len() && !phones[index].is_vowel() {
            index += 1;
        }
        if index == phones.len() {
            for phone in &phones[start..index] {
                output.push_str(coda(phone.base(), previous_vowel, previous_consonant));
                previous_consonant = Some(phone.base());
            }
            break;
        }
        let consonants = &phones[start..index];
        let palatal =
            consonants.len() >= 2 && consonants.last().is_some_and(|phone| phone.base() == "Y");
        if consonants.len() > 1 {
            let prefix_end = if palatal {
                consonants.len() - 2
            } else {
                consonants.len() - 1
            };
            for phone in &consonants[..prefix_end] {
                output.push_str(medial_prefix(phone.base(), previous_vowel));
            }
        }
        if palatal {
            output.push_str(&palatal_onset(
                consonants[consonants.len() - 2].base(),
                &phones[index],
            )?);
        } else {
            output.push_str(&onset(
                consonants.last().expect("non-empty consonant run").base(),
                &phones[index],
            )?);
        }
        previous_vowel = Some(phones[index].base());
        previous_consonant = None;
        index += 1;
    }
    Ok(output.replace("ットス", "ッツ"))
}

fn vowel(phone: &Phoneme) -> &'static str {
    match phone.base() {
        "AA" | "AE" | "AH" => "ア",
        "AO" => "オー",
        "AW" => "アウ",
        "AY" => "アイ",
        "EH" => "エ",
        "ER" => "アー",
        "EY" => "エイ",
        "IH" => "イ",
        "IY" if phone.symbol.ends_with('0') => "イ",
        "IY" => "イー",
        "OW" => "オウ",
        "OY" => "オイ",
        "UH" => "ウ",
        "UW" if phone.symbol.ends_with('0') => "ウ",
        "UW" => "ウー",
        _ => "",
    }
}

fn onset(consonant: &str, vowel_phone: &Phoneme) -> Result<String, String> {
    let slot = match vowel_phone.base() {
        "AA" | "AE" | "AH" | "AW" | "AY" => 0,
        "IH" | "IY" => 1,
        "UH" | "UW" => 2,
        "EH" | "EY" => 3,
        "ER" => 0,
        "AO" | "OW" | "OY" => 4,
        other => return Err(format!("unsupported ARPAbet vowel: {other}")),
    };
    let row: [&str; 5] = match consonant {
        "B" => ["バ", "ビ", "ブ", "ベ", "ボ"],
        "CH" => ["チャ", "チ", "チュ", "チェ", "チョ"],
        "D" => ["ダ", "ディ", "ドゥ", "デ", "ド"],
        "DH" => ["ザ", "ジ", "ズ", "ゼ", "ゾ"],
        "DX" => ["ラ", "リ", "ル", "レ", "ロ"],
        "F" => ["ファ", "フィ", "フ", "フェ", "フォ"],
        "G" => ["ガ", "ギ", "グ", "ゲ", "ゴ"],
        "HH" => ["ハ", "ヒ", "フ", "ヘ", "ホ"],
        "JH" => ["ジャ", "ジ", "ジュ", "ジェ", "ジョ"],
        "K" => ["カ", "キ", "ク", "ケ", "コ"],
        "L" | "R" => ["ラ", "リ", "ル", "レ", "ロ"],
        "M" => ["マ", "ミ", "ム", "メ", "モ"],
        "N" => ["ナ", "ニ", "ヌ", "ネ", "ノ"],
        "NG" => ["ンガ", "ンギ", "ング", "ンゲ", "ンゴ"],
        "P" => ["パ", "ピ", "プ", "ペ", "ポ"],
        "S" => ["サ", "シ", "ス", "セ", "ソ"],
        "SH" => ["シャ", "シ", "シュ", "シェ", "ショ"],
        "T" => ["タ", "ティ", "トゥ", "テ", "ト"],
        "TH" => ["サ", "シ", "ス", "セ", "ソ"],
        "V" => ["ヴァ", "ヴィ", "ヴ", "ヴェ", "ヴォ"],
        "W" => ["ワ", "ウィ", "ウ", "ウェ", "ウォ"],
        "Y" => ["ヤ", "イ", "ユ", "イェ", "ヨ"],
        "Z" => ["ザ", "ジ", "ズ", "ゼ", "ゾ"],
        "ZH" => ["ジャ", "ジ", "ジュ", "ジェ", "ジョ"],
        other => return Err(format!("unsupported ARPAbet consonant: {other}")),
    };
    let suffix = match vowel_phone.base() {
        "AW" => "ウ",
        "AY" | "EY" | "OY" => "イ",
        "OW" => "ウ",
        "IY" if !vowel_phone.symbol.ends_with('0') => "ー",
        "UW" if !vowel_phone.symbol.ends_with('0') => "ー",
        "AO" => "ー",
        "ER" => "ー",
        _ => "",
    };
    Ok(format!("{}{}", row[slot], suffix))
}

fn cluster_prefix(phone: &str) -> &'static str {
    match phone {
        "B" => "ブ",
        "D" => "ド",
        "F" => "フ",
        "G" => "グ",
        "K" => "ク",
        "P" => "プ",
        "S" | "SH" => "ス",
        "T" | "TH" => "ト",
        "V" => "ヴ",
        _ => coda(phone, None, None),
    }
}

fn medial_prefix(phone: &str, previous_vowel: Option<&str>) -> &'static str {
    match phone {
        "B" | "D" | "F" | "G" | "K" | "P" | "S" | "SH" | "T" | "TH" | "V" => cluster_prefix(phone),
        _ => coda(phone, previous_vowel, None),
    }
}

fn palatal_onset(consonant: &str, vowel_phone: &Phoneme) -> Result<String, String> {
    let row = match consonant {
        "B" => ["ビャ", "ビュ", "ビョ"],
        "D" => ["ヂャ", "ヂュ", "ヂョ"],
        "F" => ["フャ", "フュ", "フョ"],
        "G" => ["ギャ", "ギュ", "ギョ"],
        "K" => ["キャ", "キュ", "キョ"],
        "M" => ["ミャ", "ミュ", "ミョ"],
        "N" => ["ニャ", "ニュ", "ニョ"],
        "P" => ["ピャ", "ピュ", "ピョ"],
        "R" => ["リャ", "リュ", "リョ"],
        "S" => ["シャ", "シュ", "ショ"],
        "T" => ["チャ", "チュ", "チョ"],
        "V" => ["ヴャ", "ヴュ", "ヴョ"],
        "Z" => ["ジャ", "ジュ", "ジョ"],
        other => return Err(format!("unsupported palatal ARPAbet onset: {other} Y")),
    };
    let index = match vowel_phone.base() {
        "AA" | "AE" | "AH" | "AW" | "AY" => 0,
        "IH" | "IY" | "UH" | "UW" | "EH" | "ER" | "EY" => 1,
        "AO" | "OW" | "OY" => 2,
        other => return Err(format!("unsupported palatal ARPAbet vowel: {other}")),
    };
    let suffix = match vowel_phone.base() {
        "AW" => "ウ",
        "AY" | "EY" | "OY" => "イ",
        "OW" => "ウ",
        "IY" | "UW" if !vowel_phone.symbol.ends_with('0') => "ー",
        "AO" | "ER" => "ー",
        _ => "",
    };
    Ok(format!("{}{}", row[index], suffix))
}

fn coda(
    phone: &str,
    previous_vowel: Option<&str>,
    previous_consonant: Option<&str>,
) -> &'static str {
    match phone {
        "B" => "ブ",
        "CH" => "チ",
        "D" | "DH" => "ド",
        "DX" => "ル",
        "F" => "フ",
        "G" => "グ",
        "HH" => "",
        "JH" => "ジ",
        "K" if is_long_vowel(previous_vowel) => "ク",
        "K" => "ック",
        "L" => "ル",
        "M" => "ム",
        "N" => "ン",
        "NG" => "ング",
        "P" => "ップ",
        "R" if matches!(previous_vowel, Some("AO" | "ER")) => "",
        "R" if previous_vowel == Some("AA") => "ー",
        "R" => "ア",
        "S" => "ス",
        "SH" => "シュ",
        "T" if previous_consonant.is_some() || is_long_vowel(previous_vowel) => "ト",
        "T" => "ット",
        "TH" => "ス",
        "V" => "ヴ",
        "W" | "Y" => "",
        "Z" => "ズ",
        "ZH" => "ジュ",
        _ => "",
    }
}

fn is_long_vowel(vowel: Option<&str>) -> bool {
    matches!(
        vowel,
        Some("AO" | "AW" | "AY" | "ER" | "EY" | "IY" | "OW" | "OY" | "UW")
    )
}
