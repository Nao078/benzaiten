//! ARPAbet音素列を、読みやすい日本語（カタカナ）近似へ変換する処理。
//! 厳密な言語学的転写ではなく、歌唱練習で使いやすいことをあえて優先している。
//!
//! 全体の流れ：1単語分の音素列を先頭から走査し、母音に出会うたびに
//! 「直前の子音クラスタ＋その母音」をまとめて1つのカタカナ音節へ変換する
//! （[`onset`]／[`palatal_onset`]）。子音が単語末や別の子音の前で終わる
//! （母音が続かない）場合は、日本語の「〜ン」「〜ック」のような
//! 母音抜きの表記（[`coda`]）を個別に当てる。3つ以上の子音が連続する
//! 場合は、末尾以外の子音に短い母音を補って発音可能にする
//! （[`medial_prefix`]／[`cluster_prefix`]）。

use super::phoneme::Phoneme;

/// 単語境界マーカーで区切られた音素列全体を、単語ごとにカタカナへ変換し
/// 半角スペースで連結する。
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

/// 1単語分の音素列をカタカナへ変換する。
///
/// 先頭から走査し、母音が現れるたびに、その直前にたまっている子音の
/// 並びと合わせて1音節分のカタカナを出力する。子音が3つ以上連続する
/// 場合は、末尾の子音（＋直後が"Y"なら拗音として扱う）だけを本来の
/// 頭子音とし、それより前の子音は[`medial_prefix`]で個別に母音を
/// 補いながら出力する。音素列が子音のまま終わる場合（後ろに母音が
/// 続かない）は[`coda`]で母音なしの表記にする。
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
            // 音素列が子音のまま終わった（単語末の子音クラスタ）。
            // 母音を補わずcodaの表記をそのまま並べる。
            for phone in &phones[start..index] {
                output.push_str(coda(phone.base(), previous_vowel, previous_consonant));
                previous_consonant = Some(phone.base());
            }
            break;
        }
        let consonants = &phones[start..index];
        // 子音クラスタの末尾が"Y"（例："-ty"）なら拗音（キャ行など）として扱う。
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
    // 稀に生じる不自然な連続（"ットス"→"ッツ"、例："-ts"クラスタ）を
    // 事後的に補正する。
    Ok(output.replace("ットス", "ッツ"))
}

/// 母音単体（先行子音なし）のカタカナ表記。長音・二重母音の表記はここで
/// 決まる。`"IY"`/`"UW"`はストレス表記が"0"（無強勢）かどうかで
/// 長音「イー」「ウー」を出すか短く「イ」「ウ」にするかを分けている。
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

/// 子音1つ＋母音1つの音節（頭子音＋母音、五十音の行×段のようなもの）を
/// カタカナへ変換する。`slot`で母音の系統（ア段/イ段/ウ段/エ段/オ段相当）
/// を選び、二重母音・長母音の場合は末尾に伸ばし棒や「イ」「ウ」を補う。
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

/// 子音クラスタの先頭側（末尾以外）の子音1つに、促音・短母音を補って
/// 発音可能な形にする（例："SK"の"S"部分→「ス」）。`coda`を流用できない
/// 子音（有声・無声破裂音や摩擦音など）だけ個別の表記を持つ。
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

/// 3子音以上が連続するときの、末尾以外の子音向け表記。基本は
/// [`cluster_prefix`]に委譲し、それ以外の子音は直前の母音を考慮した
/// [`coda`]表記にフォールバックする。
fn medial_prefix(phone: &str, previous_vowel: Option<&str>) -> &'static str {
    match phone {
        "B" | "D" | "F" | "G" | "K" | "P" | "S" | "SH" | "T" | "TH" | "V" => cluster_prefix(phone),
        _ => coda(phone, previous_vowel, None),
    }
}

/// 拗音（子音＋"Y"＋母音、例："-ty"→チュ系）の音節をカタカナへ変換する。
/// [`onset`]と同様の考え方だが、行・段の組み合わせが拗音用のため
/// 別テーブルを持つ。
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

/// 母音を伴わない子音（単語末や、別の子音の直前）の表記。日本語には
/// 存在しない末子音を、促音「ッ」や撥音「ン」、短い母音付きの表記で
/// 近似する。`previous_vowel`/`previous_consonant`は、直前の音の種類に
/// よって"K"/"T"/"R"の表記を変える（長母音の後か、子音が続くかなど）
/// ために参照する。
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

/// 長母音・二重母音かどうか。直後の子音の促音化・表記選択の判定に使う。
fn is_long_vowel(vowel: Option<&str>) -> bool {
    matches!(
        vowel,
        Some("AO" | "AW" | "AY" | "ER" | "EY" | "IY" | "OW" | "OY" | "UW")
    )
}
