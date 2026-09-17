//! [`Project`]：プロジェクト`.json`ファイルとして保存される単位と、
//! その中にネストされる行・メタデータ型。読み書き・スキーマ移行は
//! `src/project/storage.rs`を参照。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// このビルドがディスクへ書き出す現在のスキーマバージョン。
/// `storage::load`は読込時に古いバージョン（v1: Whisper時代、
/// v2: 音楽メタデータ追加前）をこのバージョンまで自動移行する。
pub const CURRENT_SCHEMA_VERSION: u32 = 3;

/// 行の`start_ms`/`end_ms`がどこから来たかを表す。GUIが低信頼度の行や
/// 手動補正済みの行を区別して表示するために保持している。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimestampSource {
    /// 現行のWav2Vec2 CTC Forced Alignmentパイプラインによる結果。
    ForcedAlignment,
    /// 旧Whisperパイプラインで作成されたschema v1プロジェクトのために残している値。
    LegacyAuto,
    /// ユーザーが設定・調整した値（タイムラインのドラッグ、DragValue、キーボード微調整など）。
    Manual,
    /// 将来の「時刻を補間する」機能のために予約している値。現時点では生成されない。
    Interpolated,
}

/// 行の`reading_text`（カタカナ発音ガイド）がどこから来たかを表す。
/// これにより、読みの再生成がユーザーの手修正を上書きしないようにする。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingSource {
    /// `pronunciation::english::EnglishPronunciationEngine`による自動生成。
    Generated,
    /// 直接入力、またはカタカナ`.txt`ファイルからの読み込み。
    Manual,
}

/// 歌詞の1行分。正本となる原文、任意のカタカナ読み、割り当てられた
/// 時刻・信頼度（あれば）を保持する。
///
/// `start_ms: None`の行はまだ整列・手動設定されておらず、LRC出力や
/// タイムラインの対象から外れる。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    /// パース時点での歌詞リスト内の位置。原文が再パースされるたびに
    /// 振り直される（`domain::lyrics::parse_lyrics`参照）ため、
    /// 恒久的な識別子ではなく「位置」を表す値である点に注意。
    pub id: usize,
    /// 実際に歌われる・入力された原文。Forced Alignmentと標準LRC出力が
    /// 使用するのはこのテキストのみ。
    pub original_text: String,
    /// 任意のカタカナ発音ガイド。表示・練習用であり、
    /// Forced Alignmentには一切使用されない。
    #[serde(default)]
    pub reading_text: Option<String>,
    #[serde(default)]
    pub reading_source: Option<ReadingSource>,
    pub start_ms: Option<u64>,
    /// この行がアクティブとみなされる区間の終端（この時刻を含まない）。
    /// `None`の場合は「次の行の`start_ms`まで」を意味する
    /// （`app::active_lyric_index`を参照）。
    pub end_ms: Option<u64>,
    /// Forced Alignmentの信頼度スコア（あれば）。行の時刻を手動で
    /// 設定・調整するたびにクリアされる。
    pub confidence: Option<f32>,
    pub timestamp_source: Option<TimestampSource>,
}

/// 曲名・アーティスト以外の音楽タグ項目と、任意のアートワーク画像パス。
/// schema v3から[`Project`]に保存されるようになった。実際の音声ファイルへの
/// 書込みはこれとは別の、明示的な操作として行う（`metadata::write`参照）。
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
    /// カバーアートとして埋め込む画像ファイルのパス。音声ファイルに
    /// *現在* 埋め込まれているアートワークとは別物で、そちらは別途
    /// 読み込んで`BenzaitenApp::artwork_bytes`にキャッシュされる。
    #[serde(default)]
    pub artwork_path: Option<PathBuf>,
}

/// プロジェクト`.json`ファイルへ保存・そこから読み込まれる単位。
/// 作業中の同期・編集セッションを再開するために必要な情報一式を持つ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub metadata: MusicMetadata,
    /// 音声ファイルへの絶対パス、またはプロジェクトファイルからの
    /// 相対パスとして解決されたパス。相対パス解決のルールは
    /// `project::storage`を参照。
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
