use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const CURRENT_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimestampSource {
    ForcedAlignment,
    /// Retained for schema-v1 projects created by the former Whisper pipeline.
    LegacyAuto,
    Manual,
    Interpolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingSource {
    Generated,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    pub id: usize,
    pub original_text: String,
    #[serde(default)]
    pub reading_text: Option<String>,
    #[serde(default)]
    pub reading_source: Option<ReadingSource>,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub confidence: Option<f32>,
    pub timestamp_source: Option<TimestampSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MusicMetadata {
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_artist: String,
    #[serde(default)]
    pub genre: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub track_number: Option<u32>,
    #[serde(default)]
    pub disc_number: Option<u32>,
    #[serde(default)]
    pub artwork_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub metadata: MusicMetadata,
    pub audio_path: PathBuf,
    pub lyrics: Vec<LyricLine>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            title: String::new(),
            artist: String::new(),
            metadata: MusicMetadata::default(),
            audio_path: PathBuf::new(),
            lyrics: Vec::new(),
        }
    }
}
