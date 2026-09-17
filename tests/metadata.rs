use std::path::Path;

use benzaiten::domain::project::MusicMetadata;
use lofty::{file::TaggedFileExt, tag::Accessor};

/// Writes an embedded fixture to a temp file so `metadata::write` (which
/// mutates in place and drops a `.tag-backup` next to it) never touches the
/// checked-in copy under `tests/fixtures/`.
fn fixture(bytes: &[u8], name: &str, dir: &Path) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn flac_embeds_lrc_text_as_lyrics() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(
        include_bytes!("fixtures/tiny.flac"),
        "tiny.flac",
        dir.path(),
    );

    benzaiten::metadata::write(
        &path,
        "Title",
        "Artist",
        &MusicMetadata::default(),
        Some("[00:00.00]hello\n[00:01.00]world\n"),
    )
    .unwrap();

    let tagged = lofty::read_from_path(&path).unwrap();
    let tag = tagged.primary_tag().unwrap();
    // FLAC's Vorbis Comment `LYRICS` field has no fixed line-ending
    // convention, so a lone trailing newline being trimmed (same as every
    // other text field) doesn't lose any lyric line.
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::Lyrics).unwrap(),
        "[00:00.00]hello\n[00:01.00]world"
    );
}

#[test]
fn m4a_embeds_lrc_text_as_lyrics() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(include_bytes!("fixtures/tiny.m4a"), "tiny.m4a", dir.path());

    benzaiten::metadata::write(
        &path,
        "Title",
        "Artist",
        &MusicMetadata::default(),
        Some("[00:00.00]hello\n"),
    )
    .unwrap();

    let tagged = lofty::read_from_path(&path).unwrap();
    let tag = tagged.primary_tag().unwrap();
    assert_eq!(
        tag.get_string(lofty::tag::ItemKey::Lyrics).unwrap(),
        "[00:00.00]hello"
    );
}

#[test]
fn mp3_ignores_the_lyrics_argument() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(include_bytes!("fixtures/tiny.mp3"), "tiny.mp3", dir.path());

    benzaiten::metadata::write(
        &path,
        "Title",
        "Artist",
        &MusicMetadata::default(),
        Some("[00:00.00]hello\n"),
    )
    .unwrap();

    let tagged = lofty::read_from_path(&path).unwrap();
    let tag = tagged.primary_tag().unwrap();
    // Regular tags still get written normally...
    assert_eq!(tag.title().as_deref(), Some("Title"));
    assert_eq!(tag.artist().as_deref(), Some("Artist"));
    // ...but ID3v2 has no generic `Lyrics` field (only a dedicated SYLT
    // frame, which this app doesn't write), so nothing should land here.
    // MP3 keeps relying on a companion .lrc file instead.
    assert!(tag.get_string(lofty::tag::ItemKey::Lyrics).is_none());
}
