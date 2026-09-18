use benzaiten::{
    domain::{lyrics::parse_lyrics, project::TimestampSource},
    forced_alignment::{self, tokenizer},
};

#[test]
fn known_transcript_is_resolved_to_line_timestamps() {
    let lyrics = parse_lyrics("A B\nC");
    let transcript = tokenizer::tokenize(&lyrics).unwrap();
    let mut emissions = Vec::new();
    for token in &transcript.token_ids {
        let mut token_frame = vec![-8.0; 32];
        token_frame[*token] = 8.0;
        emissions.push(token_frame);
        let mut blank_frame = vec![-8.0; 32];
        blank_frame[tokenizer::BLANK_ID] = 8.0;
        emissions.push(blank_frame);
    }

    let aligned = forced_alignment::align(&lyrics, &emissions, 20.0).unwrap();
    assert_eq!(aligned[0].start_ms, Some(0));
    assert!(aligned[1].start_ms.unwrap() > aligned[0].start_ms.unwrap());
    assert!(aligned.iter().all(|line| matches!(
        line.timestamp_source,
        Some(TimestampSource::ForcedAlignment)
    )));
}

#[test]
fn pronunciation_guide_never_changes_alignment_tokens() {
    let mut lyrics = parse_lyrics("HELLO");
    let original = tokenizer::tokenize(&lyrics).unwrap();
    lyrics[0].reading_text = Some("ハロー".into());
    let with_reading = tokenizer::tokenize(&lyrics).unwrap();
    assert_eq!(original, with_reading);
}
