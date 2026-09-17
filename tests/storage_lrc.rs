use std::path::PathBuf;

use benzaiten::{
    domain::{
        lyrics::parse_lyrics,
        project::{LyricLine, Project, ReadingSource, TimestampSource, CURRENT_SCHEMA_VERSION},
    },
    lrc::writer,
    project::storage,
};

fn line(id: usize, text: &str, start_ms: Option<u64>) -> LyricLine {
    LyricLine {
        id,
        original_text: text.into(),
        reading_text: None,
        reading_source: None,
        start_ms,
        end_ms: None,
        confidence: Some(0.8),
        timestamp_source: Some(TimestampSource::LegacyAuto),
    }
}

#[test]
fn lyrics_parser_preserves_blank_and_trailing_rows() {
    let lines = parse_lyrics("\u{feff}first\r\n\nlast\n");
    assert_eq!(
        lines
            .iter()
            .map(|line| line.original_text.as_str())
            .collect::<Vec<_>>(),
        ["first", "", "last", ""]
    );
    assert_eq!(
        lines.iter().map(|line| line.id).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
}

#[test]
fn storage_round_trip_resolves_relative_audio_path() {
    let directory = tempfile::tempdir().unwrap();
    let project_path = directory.path().join("song.benzaiten.json");
    let project = Project {
        title: "Song".into(),
        artist: "Artist".into(),
        audio_path: PathBuf::from("audio/song.wav"),
        lyrics: vec![line(8, "one", Some(1_250))],
        ..Project::default()
    };

    storage::save(&project_path, &project).unwrap();
    let loaded = storage::load(&project_path).unwrap();
    assert_eq!(loaded.title, "Song");
    assert_eq!(loaded.lyrics, project.lyrics);
    assert_eq!(loaded.audio_path, directory.path().join("audio/song.wav"));
}

#[test]
fn storage_round_trip_preserves_pronunciation_guide() {
    let directory = tempfile::tempdir().unwrap();
    let project_path = directory.path().join("reading.json");
    let mut lyric = line(1, "We are", Some(100));
    lyric.reading_text = Some("ウィー アー".into());
    lyric.reading_source = Some(ReadingSource::Manual);
    let project = Project {
        lyrics: vec![lyric],
        ..Project::default()
    };
    storage::save(&project_path, &project).unwrap();
    assert_eq!(storage::load(&project_path).unwrap(), project);
}

#[test]
fn schema_v1_project_is_migrated_without_losing_timing() {
    let directory = tempfile::tempdir().unwrap();
    let project_path = directory.path().join("legacy.json");
    std::fs::write(
        &project_path,
        r#"{
          "schema_version": 1,
          "title": "Legacy",
          "artist": "Artist",
          "audio_path": "song.mp3",
          "lyrics": [{
            "id": 4,
            "text": "Hello",
            "start_ms": 1200,
            "end_ms": 1800,
            "confidence": 0.8,
            "source": "Auto"
          }]
        }"#,
    )
    .unwrap();

    let project = storage::load(&project_path).unwrap();
    assert_eq!(project.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(project.lyrics[0].original_text, "Hello");
    assert_eq!(project.lyrics[0].reading_text, None);
    assert_eq!(
        project.lyrics[0].timestamp_source,
        Some(TimestampSource::LegacyAuto)
    );
}

#[test]
fn schema_v2_project_is_migrated_with_empty_music_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let project_path = directory.path().join("v2.json");
    std::fs::write(
        &project_path,
        r#"{
          "schema_version": 2,
          "title": "Song",
          "artist": "Artist",
          "audio_path": "song.mp3",
          "lyrics": []
        }"#,
    )
    .unwrap();

    let project = storage::load(&project_path).unwrap();
    assert_eq!(project.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(project.metadata, Default::default());
}

#[test]
fn storage_rejects_invalid_timing() {
    let directory = tempfile::tempdir().unwrap();
    let project = Project {
        lyrics: vec![LyricLine {
            end_ms: Some(9),
            ..line(0, "one", Some(10))
        }],
        ..Project::default()
    };
    assert!(storage::save(&directory.path().join("bad.json"), &project).is_err());
}

#[test]
fn lrc_uses_floor_centiseconds_and_skips_blank_rows() {
    let project = Project {
        title: "Song".into(),
        artist: "Artist".into(),
        lyrics: vec![
            line(0, "", None),
            line(1, "first", Some(60_019)),
            line(2, "second", Some(60_020)),
        ],
        ..Project::default()
    };
    assert_eq!(
        writer::render(&project).unwrap(),
        "[ti:Song]\n[ar:Artist]\n[01:00.01]first\n[01:00.02]second\n"
    );
}

#[test]
fn lrc_rejects_missing_or_reverse_timestamps_and_metadata_injection() {
    let mut project = Project {
        lyrics: vec![line(0, "one", Some(20)), line(1, "two", Some(10))],
        ..Project::default()
    };
    assert!(writer::render(&project).unwrap_err().contains("earlier"));
    project.lyrics[1].start_ms = None;
    assert!(writer::render(&project)
        .unwrap_err()
        .contains("no start timestamp"));
    project.title = "title\n[ar:injected]".into();
    assert!(writer::render(&project).unwrap_err().contains("newline"));
}
