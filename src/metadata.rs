use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use lofty::{
    config::WriteOptions,
    file::{AudioFile, TaggedFileExt},
    picture::{Picture, PictureType},
    tag::{Accessor, ItemKey, Tag},
};

use crate::domain::project::MusicMetadata;

#[derive(Debug, Default)]
pub struct AudioMetadata {
    pub title: String,
    pub artist: String,
    pub details: MusicMetadata,
    pub artwork: Option<Vec<u8>>,
}

pub fn read(path: &Path) -> Result<AudioMetadata, String> {
    let tagged = lofty::read_from_path(path)
        .map_err(|error| format!("音楽情報を読み込めません: {error}"))?;
    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
        return Ok(AudioMetadata::default());
    };
    Ok(AudioMetadata {
        title: tag
            .title()
            .map(|value| value.into_owned())
            .unwrap_or_default(),
        artist: tag
            .artist()
            .map(|value| value.into_owned())
            .unwrap_or_default(),
        details: MusicMetadata {
            album: tag
                .album()
                .map(|value| value.into_owned())
                .unwrap_or_default(),
            album_artist: tag
                .get_string(ItemKey::AlbumArtist)
                .unwrap_or_default()
                .to_owned(),
            genre: tag
                .genre()
                .map(|value| value.into_owned())
                .unwrap_or_default(),
            year: tag.date().map(|date| u32::from(date.year)),
            track_number: tag.track(),
            disc_number: tag.disk(),
            artwork_path: None,
        },
        artwork: tag
            .pictures()
            .iter()
            .find(|picture| picture.pic_type() == PictureType::CoverFront)
            .or_else(|| tag.pictures().first())
            .map(|picture| picture.data().to_vec()),
    })
}

pub fn write(
    path: &Path,
    title: &str,
    artist: &str,
    metadata: &MusicMetadata,
) -> Result<PathBuf, String> {
    let mut tagged = lofty::read_from_path(path)
        .map_err(|error| format!("音楽情報を読み込めません: {error}"))?;
    let tag_type = tagged.primary_tag_type();
    if tagged.primary_tag().is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .ok_or_else(|| "この音声形式には書込み可能なタグがありません".to_owned())?;

    set_item(tag, ItemKey::TrackTitle, title);
    set_item(tag, ItemKey::TrackArtist, artist);
    set_item(tag, ItemKey::AlbumTitle, &metadata.album);
    set_item(tag, ItemKey::Genre, &metadata.genre);
    set_item(tag, ItemKey::AlbumArtist, &metadata.album_artist);
    if let Some(year) = metadata.year.and_then(|year| u16::try_from(year).ok()) {
        tag.set_date(lofty::tag::items::Timestamp {
            year,
            ..Default::default()
        });
    } else {
        tag.remove_date();
    }
    if let Some(value) = metadata.track_number {
        tag.set_track(value);
    } else {
        tag.remove_track();
    }
    if let Some(value) = metadata.disc_number {
        tag.set_disk(value);
    } else {
        tag.remove_disk();
    }

    if let Some(artwork_path) = metadata.artwork_path.as_deref() {
        let bytes = fs::read(artwork_path)
            .map_err(|error| format!("アルバムアートを読み込めません: {error}"))?;
        let mut picture = Picture::from_reader(&mut Cursor::new(bytes))
            .map_err(|error| format!("アルバムアート形式を認識できません: {error}"))?;
        picture.set_pic_type(PictureType::CoverFront);
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(picture);
    }

    let backup = backup_path(path);
    if !backup.exists() {
        fs::copy(path, &backup)
            .map_err(|error| format!("タグ書込み前のバックアップを作成できません: {error}"))?;
    }
    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|error| format!("音声ファイルへタグを書き込めません: {error}"))?;
    Ok(backup)
}

fn set_item(tag: &mut Tag, key: ItemKey, value: &str) {
    tag.remove_key(key);
    if !value.trim().is_empty() {
        tag.insert_text(key, value.trim().to_owned());
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("audio");
    path.with_extension(format!("{extension}.tag-backup"))
}
