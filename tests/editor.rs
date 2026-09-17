use benzaiten::{
    domain::lyrics::{parse_lyrics, TimestampSource},
    ui::lyric_editor::{
        move_line_to, set_line_start, set_start, shift_from, shift_line_start, shift_start,
    },
};

#[test]
fn correction_clears_stale_confidence_and_invalid_end() {
    let mut line = parse_lyrics("歌詞").remove(0);
    line.end_ms = Some(200);
    line.confidence = Some(0.9);
    set_start(&mut line, 500);
    assert_eq!(line.start_ms, Some(500));
    assert_eq!(line.end_ms, None);
    assert_eq!(line.confidence, None);
    assert_eq!(line.timestamp_source, Some(TimestampSource::Manual));
    shift_start(&mut line, -1000, Some(1000));
    assert_eq!(line.start_ms, Some(0));
    shift_start(&mut line, 5000, Some(1000));
    assert_eq!(line.start_ms, Some(1000));
}

#[test]
fn changing_a_line_start_does_not_touch_the_previous_line() {
    let mut lines = parse_lyrics("one\ntwo");
    lines[0].start_ms = Some(100);
    lines[0].end_ms = Some(500);
    lines[1].start_ms = Some(500);

    set_line_start(&mut lines, 1, 650);
    assert_eq!(lines[0].end_ms, Some(500));
    assert_eq!(lines[1].start_ms, Some(650));

    shift_line_start(&mut lines, 1, -50, Some(1_000));
    assert_eq!(lines[0].end_ms, Some(500));
    assert_eq!(lines[1].start_ms, Some(600));
}

#[test]
fn bulk_shift_preserves_spacing_and_clamps_as_one_group() {
    let mut lines = parse_lyrics("one\ntwo\nthree");
    for (line, start) in lines.iter_mut().zip([100, 400, 800]) {
        line.start_ms = Some(start);
        line.end_ms = Some(start + 100);
        line.confidence = Some(0.8);
    }

    assert_eq!(shift_from(&mut lines, 1, 500, Some(1_000)), 100);
    assert_eq!(lines[0].start_ms, Some(100));
    assert_eq!(lines[0].end_ms, Some(200));
    assert_eq!(lines[1].start_ms, Some(500));
    assert_eq!(lines[2].start_ms, Some(900));
    assert_eq!(lines[2].end_ms, Some(1_000));
    assert_eq!(lines[1].confidence, None);
    assert_eq!(lines[2].timestamp_source, Some(TimestampSource::Manual));

    assert_eq!(shift_from(&mut lines, 1, -1_000, Some(1_000)), -500);
    assert_eq!(lines[1].start_ms, Some(0));
    assert_eq!(lines[2].start_ms, Some(400));
}

#[test]
fn timeline_move_preserves_length_and_clamps_to_audio() {
    let mut lines = parse_lyrics("one\ntwo");
    lines[0].start_ms = Some(1_000);
    lines[0].end_ms = Some(2_500);
    lines[0].confidence = Some(0.8);

    move_line_to(&mut lines, 0, 9_000, Some(1_500), Some(10_000));

    assert_eq!(lines[0].start_ms, Some(8_500));
    assert_eq!(lines[0].end_ms, Some(10_000));
    assert_eq!(lines[0].confidence, None);
    assert_eq!(lines[0].timestamp_source, Some(TimestampSource::Manual));
}
