//! loftyを使った音楽タグの読み書き。読み込みはGUI表示用に曲名などを
//! 取得するだけだが、書き込みは「音声ファイルへタグを書き込む」操作から
//! 明示的に呼ばれ、初回書込み時にバックアップを作成し、対応形式では
//! 歌詞（LRCテキスト）もタグへ埋め込む。

use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use lofty::{
    config::WriteOptions,
    file::{AudioFile, FileType, TaggedFileExt},
    picture::{Picture, PictureType},
    tag::{Accessor, ItemKey, Tag},
};

use crate::domain::project::MusicMetadata;

/// 音声ファイルから読み取った曲名・アーティスト・その他メタデータ・
/// カバーアート画像データ。プロジェクトを新規作成する際の初期値として使う。
#[derive(Debug, Default)]
pub struct AudioMetadata {
    pub title: String,
    pub artist: String,
    pub details: MusicMetadata,
    pub artwork: Option<Vec<u8>>,
}

/// `path`のタグから曲名・アーティスト・メタデータ・カバーアートを読み取る。
/// タグが存在しない場合は空の[`AudioMetadata`]を返す（エラーにはしない）。
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

/// `path`のタグへ曲名・アーティスト・メタデータ・（対応形式なら）歌詞を
/// 書き込み、必要ならカバーアートも埋め込む。書込み前に、そのファイルへの
/// 初回書込みであれば`.tag-backup`（[`backup_path`]）を作成する。
/// 成功時はバックアップファイルのパスを返す。
pub fn write(
    path: &Path,
    title: &str,
    artist: &str,
    metadata: &MusicMetadata,
    lyrics: Option<&str>,
) -> Result<PathBuf, String> {
    let mut tagged = lofty::read_from_path(path)
        .map_err(|error| format!("音楽情報を読み込めません: {error}"))?;
    let file_type = tagged.file_type();
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
    // `ItemKey::Lyrics`はVorbis Comment（FLAC）とMP4の`©lyr`アトムでは
    // 実際のタグ項目に対応しており、どちらもLRC形式のテキストを
    // 問題なく格納できる。ID3v2（MP3）にはこれに相当する汎用フィールドが
    // 無く、同期歌詞には専用のSYLTフレームが必要（本関数では書かない）
    // ため、MP3ではここをスキップし、従来どおり別ファイルの`.lrc`に頼る。
    if matches!(file_type, FileType::Flac | FileType::Mp4) {
        match lyrics {
            Some(text) if !text.trim().is_empty() => set_item(tag, ItemKey::Lyrics, text),
            _ => tag.remove_key(ItemKey::Lyrics),
        }
    }
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

/// 値が空（トリム後）でなければタグ項目を設定し、空なら削除する。
/// 空文字列を書き込んで意味のないタグ項目を残さないようにする。
fn set_item(tag: &mut Tag, key: ItemKey, value: &str) {
    tag.remove_key(key);
    if !value.trim().is_empty() {
        tag.insert_text(key, value.trim().to_owned());
    }
}

/// 元ファイルと同じ場所・同じ拡張子ベースに`.tag-backup`を付けたパスを返す
/// （例：`song.mp3` → `song.mp3.tag-backup`）。
fn backup_path(path: &Path) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("audio");
    path.with_extension(format!("{extension}.tag-backup"))
}
