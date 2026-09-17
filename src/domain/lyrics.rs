//! 自由入力の歌詞・読みテキストを[`LyricLine`]へ変換する処理。
//! テキストの出所（GUIへの貼り付けか、ファイル読み込みか）には依存しない。

pub use super::project::{LyricLine, ReadingSource, TimestampSource};

/// 編集用の歌詞テキストを歌詞行の配列へパースする。
///
/// 先頭のUTF-8 BOMは許容し、CRLFはLFと同じテキストとして扱う。
/// 空行（貼り付け・編集時の見やすさのため、段落区切りなどに使われる）は
/// 空の歌詞行にはせず破棄する。これにより、残る行はすべて実際の
/// アライメント対象となり、idも詰めた値（0, 1, 2, ...）になる。
pub fn parse_lyrics(input: &str) -> Vec<LyricLine> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);

    input
        .split('\n')
        .map(|row| row.strip_suffix('\r').unwrap_or(row).to_owned())
        .filter(|row| !row.trim().is_empty())
        .enumerate()
        .map(|(id, original_text)| LyricLine {
            id,
            original_text,
            reading_text: None,
            reading_source: None,
            start_ms: None,
            end_ms: None,
            confidence: None,
            timestamp_source: None,
        })
        .collect()
}

/// ユーザーが用意した任意の発音ファイルを適用する。正本の歌詞テキストは
/// 変更せず、`reading_text`/`reading_source`のみを書き換える。
///
/// 読みファイルの行に対して、以下の2通りの対応付けを順に試す。
/// 1. `lyrics`の行数（渡された配列そのまま）と完全一致する場合の位置対応。
///    呼び出し側が直接構築した空の`original_text`行も含めて対応させる
///    （`parse_lyrics`自体はもう空行を生成しないが、読みファイル側に
///    空の段落区切り行があり、それが1:1で並んでいるケースにはまだ対応する）。
/// 2. 上記が一致しない場合、両側とも空行を除いた行数で対応付けるフォールバック。
///    これにより、空の区切り行を省略した読みファイルでも正しく対応する。
///
/// 読みが設定できた行数を返す。どちらの方式でも行数が一致しない場合はエラー。
pub fn apply_readings(lyrics: &mut [LyricLine], input: &str) -> Result<usize, String> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut rows: Vec<String> = input
        .split('\n')
        .map(|row| row.strip_suffix('\r').unwrap_or(row).trim().to_owned())
        .collect();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }

    let mut lyric_len = lyrics.len();
    while lyric_len > 0 && lyrics[lyric_len - 1].original_text.trim().is_empty() {
        lyric_len -= 1;
    }

    if rows.len() == lyric_len {
        // 方式1: 空行も含めた位置対応。
        for (line, reading) in lyrics.iter_mut().zip(rows) {
            set_reading(line, reading);
        }
    } else {
        // 方式2: 両側とも空行を除いた行同士を対応させる。
        // 歌詞側の空行の読みは常にクリアする。
        let non_empty_rows: Vec<String> = rows.into_iter().filter(|row| !row.is_empty()).collect();
        let non_empty_lyrics = lyrics
            .iter()
            .filter(|line| !line.original_text.trim().is_empty())
            .count();
        if non_empty_rows.len() != non_empty_lyrics {
            return Err(format!(
                "カタカナ歌詞の行数が一致しません（原文の歌詞行: {non_empty_lyrics}、カタカナ行: {}）",
                non_empty_rows.len()
            ));
        }
        let mut readings = non_empty_rows.into_iter();
        for line in lyrics.iter_mut() {
            if line.original_text.trim().is_empty() {
                set_reading(line, String::new());
            } else if let Some(reading) = readings.next() {
                set_reading(line, reading);
            }
        }
    }

    Ok(lyrics
        .iter()
        .filter(|line| line.reading_text.is_some())
        .count())
}

/// 1行分の読みを設定またはクリアする。ユーザー入力のテキストなので、
/// 空でない読みには[`ReadingSource::Manual`]を付与する。
fn set_reading(line: &mut LyricLine, reading: String) {
    if reading.is_empty() {
        line.reading_text = None;
        line.reading_source = None;
    } else {
        line.reading_text = Some(reading);
        line.reading_source = Some(ReadingSource::Manual);
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_readings, parse_lyrics, LyricLine, ReadingSource};

    /// `parse_lyrics`（空行を破棄する）を経由せず、歌詞行を直接組み立てる。
    /// これにより`apply_readings`の空行対応ロジックのテストを維持する。
    fn lines(rows: &[&str]) -> Vec<LyricLine> {
        rows.iter()
            .enumerate()
            .map(|(id, text)| LyricLine {
                id,
                original_text: (*text).to_owned(),
                reading_text: None,
                reading_source: None,
                start_ms: None,
                end_ms: None,
                confidence: None,
                timestamp_source: None,
            })
            .collect()
    }

    #[test]
    fn parse_lyrics_drops_blank_rows() {
        let lyrics = parse_lyrics("\u{feff}first\r\n\n   \nlast\n");

        assert_eq!(
            lyrics
                .iter()
                .map(|line| line.original_text.as_str())
                .collect::<Vec<_>>(),
            ["first", "last"]
        );
        assert_eq!(
            lyrics.iter().map(|line| line.id).collect::<Vec<_>>(),
            [0, 1]
        );
    }

    #[test]
    fn applies_readings_with_matching_blank_rows() {
        let mut lyrics = lines(&["Hello", "", "World"]);
        let count = apply_readings(&mut lyrics, "ハロー\n\nワールド").unwrap();

        assert_eq!(count, 2);
        assert_eq!(lyrics[0].reading_text.as_deref(), Some("ハロー"));
        assert_eq!(lyrics[1].reading_text, None);
        assert_eq!(lyrics[2].reading_text.as_deref(), Some("ワールド"));
        assert_eq!(lyrics[2].reading_source, Some(ReadingSource::Manual));
    }

    #[test]
    fn allows_omitting_blank_stanza_rows() {
        let mut lyrics = lines(&["Hello", "", "World"]);
        apply_readings(&mut lyrics, "ハロー\nワールド").unwrap();

        assert_eq!(lyrics[0].reading_text.as_deref(), Some("ハロー"));
        assert_eq!(lyrics[1].reading_text, None);
        assert_eq!(lyrics[2].reading_text.as_deref(), Some("ワールド"));
    }

    #[test]
    fn rejects_readings_with_a_different_line_count() {
        let mut lyrics = parse_lyrics("Hello\nWorld");
        let error = apply_readings(&mut lyrics, "ハロー").unwrap_err();

        assert!(error.contains("行数が一致しません"));
        assert!(lyrics.iter().all(|line| line.reading_text.is_none()));
    }
}
