pub use super::project::{LyricLine, ReadingSource, TimestampSource};

/// Parses the editable lyric text without changing its line boundaries.
///
/// A UTF-8 BOM is tolerated at the very beginning of the input and CRLF input
/// is represented with the same text as LF input.
pub fn parse_lyrics(input: &str) -> Vec<LyricLine> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);

    input
        .split('\n')
        .enumerate()
        .map(|(id, row)| LyricLine {
            id,
            original_text: row.strip_suffix('\r').unwrap_or(row).to_owned(),
            reading_text: None,
            reading_source: None,
            start_ms: None,
            end_ms: None,
            confidence: None,
            timestamp_source: None,
        })
        .collect()
}

/// Applies an optional, user-supplied pronunciation file without changing the
/// canonical lyrics. Blank stanza rows may either be present or omitted.
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
        for (line, reading) in lyrics.iter_mut().zip(rows) {
            set_reading(line, reading);
        }
    } else {
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
    use super::{apply_readings, parse_lyrics, ReadingSource};

    #[test]
    fn applies_readings_with_matching_blank_rows() {
        let mut lyrics = parse_lyrics("Hello\n\nWorld\n");
        let count = apply_readings(&mut lyrics, "ハロー\n\nワールド").unwrap();

        assert_eq!(count, 2);
        assert_eq!(lyrics[0].reading_text.as_deref(), Some("ハロー"));
        assert_eq!(lyrics[1].reading_text, None);
        assert_eq!(lyrics[2].reading_text.as_deref(), Some("ワールド"));
        assert_eq!(lyrics[2].reading_source, Some(ReadingSource::Manual));
    }

    #[test]
    fn allows_omitting_blank_stanza_rows() {
        let mut lyrics = parse_lyrics("Hello\n\nWorld");
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
