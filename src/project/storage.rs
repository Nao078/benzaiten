//! プロジェクトJSONのアトミックな保存・読込、スキーマ移行、内容検証。

use std::{collections::HashSet, fs, io::Write, path::Path};

use serde::Deserialize;
use tempfile::NamedTempFile;

use crate::domain::project::{
    LyricLine, MusicMetadata, Project, TimestampSource, CURRENT_SCHEMA_VERSION,
};

/// プロジェクトをJSONとして保存する。一時ファイルは保存先と同じ
/// ディレクトリに作成するため、リネームによる置き換えがファイル
/// システムをまたがずに済む（アトミックな書き込みになる）。
pub fn save(path: &Path, project: &Project) -> Result<(), String> {
    validate(project)?;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        return Err(format!(
            "project directory does not exist: {}",
            parent.display()
        ));
    }

    let bytes = serde_json::to_vec_pretty(project)
        .map_err(|error| format!("could not serialize project: {error}"))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("could not create temporary project file: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.write_all(b"\n"))
        .and_then(|_| temporary.flush())
        .map_err(|error| format!("could not write project file: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("could not sync project file: {error}"))?;

    replace_file(temporary, path)
}

/// プロジェクトを読み込み、相対音声パスをプロジェクトファイルのある
/// ディレクトリから解決する。音声ファイル自体の存在はあえて必須と
/// しない。プロジェクトが移動された後、音声を再リンクする前でも
/// 開けるようにするためである。
pub fn load(path: &Path) -> Result<Project, String> {
    let contents = fs::read(path)
        .map_err(|error| format!("could not read project file {}: {error}", path.display()))?;
    let document: serde_json::Value = serde_json::from_slice(&contents)
        .map_err(|error| format!("could not parse project file {}: {error}", path.display()))?;
    let schema_version = document
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("project file {} has no schema version", path.display()))?;
    // スキーマバージョンごとに、まず旧バージョンの構造体としてパースして
    // から現行の`Project`へ移行する。現行バージョンはそのまま直接パースする。
    let mut project = match schema_version {
        1 => migrate_v1(serde_json::from_value(document).map_err(|error| {
            format!(
                "could not parse v1 project file {}: {error}",
                path.display()
            )
        })?),
        2 => {
            // v2はv3（本メタデータ追加）とフィールド互換なので、
            // パース自体はそのまま行い、バージョン番号だけ更新する。
            let mut project: Project = serde_json::from_value(document).map_err(|error| {
                format!(
                    "could not parse v2 project file {}: {error}",
                    path.display()
                )
            })?;
            project.schema_version = CURRENT_SCHEMA_VERSION;
            project
        }
        version if version == u64::from(CURRENT_SCHEMA_VERSION) => serde_json::from_value(document)
            .map_err(|error| format!("could not parse project file {}: {error}", path.display()))?,
        version => {
            return Err(format!(
                "unsupported project schema version {version}; expected {CURRENT_SCHEMA_VERSION}"
            ));
        }
    };
    validate(&project)?;

    if !project.audio_path.as_os_str().is_empty() && project.audio_path.is_relative() {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = if parent.is_absolute() {
            parent.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|error| format!("could not determine project directory: {error}"))?
                .join(parent)
        };
        project.audio_path = parent.join(&project.audio_path);
        // アートワークパスも音声パスと同じ規則で相対解決する。
        if project
            .metadata
            .artwork_path
            .as_ref()
            .is_some_and(|artwork| artwork.is_relative())
        {
            project.metadata.artwork_path = project
                .metadata
                .artwork_path
                .as_ref()
                .map(|artwork| parent.join(artwork));
        }
    }
    Ok(project)
}

/// プロジェクトの整合性を検証する：現行スキーマバージョンであること、
/// 行idの重複がないこと、`end_ms`が`start_ms`より前でないこと、
/// confidenceが有効な範囲(0.0〜1.0)であること、原文・読みに改行が
/// 含まれていないこと（LRC出力を壊さないようにするための制約）。
fn validate(project: &Project) -> Result<(), String> {
    if project.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported project schema version {}; expected {CURRENT_SCHEMA_VERSION}",
            project.schema_version
        ));
    }

    let mut ids = HashSet::new();
    for line in &project.lyrics {
        if !ids.insert(line.id) {
            return Err(format!("duplicate lyric line id: {}", line.id));
        }
        if let (Some(start), Some(end)) = (line.start_ms, line.end_ms) {
            if end < start {
                return Err(format!("lyric line {} ends before it starts", line.id));
            }
        }
        if let Some(confidence) = line.confidence {
            if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
                return Err(format!("lyric line {} has an invalid confidence", line.id));
            }
        }
        if line.original_text.contains('\n') || line.original_text.contains('\r') {
            return Err(format!("lyric line {} contains a newline", line.id));
        }
        if line
            .reading_text
            .as_ref()
            .is_some_and(|reading| reading.contains('\n') || reading.contains('\r'))
        {
            return Err(format!("lyric line {} reading contains a newline", line.id));
        }
    }
    Ok(())
}

/// 旧Whisperパイプライン時代（schema v1）のプロジェクト構造。
/// `text`/`source`が現行の`original_text`/`timestamp_source`に対応する。
#[derive(Deserialize)]
struct ProjectV1 {
    title: String,
    artist: String,
    audio_path: std::path::PathBuf,
    lyrics: Vec<LyricLineV1>,
}

#[derive(Deserialize)]
struct LyricLineV1 {
    id: usize,
    text: String,
    start_ms: Option<u64>,
    end_ms: Option<u64>,
    confidence: Option<f32>,
    source: Option<TimestampSourceV1>,
}

/// v1時代の`TimestampSource`。`Auto`は現行の`LegacyAuto`に対応する
/// （当時は現在のForced Alignmentではなく、Whisperによる自動認識だったため）。
#[derive(Deserialize)]
enum TimestampSourceV1 {
    Auto,
    Manual,
    Interpolated,
}

/// v1プロジェクトを現行の`Project`へ移行する。カタカナ読み関連の
/// フィールド（v1には存在しない）は`None`で初期化する。
fn migrate_v1(project: ProjectV1) -> Project {
    Project {
        schema_version: CURRENT_SCHEMA_VERSION,
        title: project.title,
        artist: project.artist,
        metadata: MusicMetadata::default(),
        audio_path: project.audio_path,
        lyrics: project
            .lyrics
            .into_iter()
            .map(|line| LyricLine {
                id: line.id,
                original_text: line.text,
                reading_text: None,
                reading_source: None,
                start_ms: line.start_ms,
                end_ms: line.end_ms,
                confidence: line.confidence,
                timestamp_source: line.source.map(|source| match source {
                    TimestampSourceV1::Auto => TimestampSource::LegacyAuto,
                    TimestampSourceV1::Manual => TimestampSource::Manual,
                    TimestampSourceV1::Interpolated => TimestampSource::Interpolated,
                }),
            })
            .collect(),
    }
}

/// 一時ファイルを`destination`へ配置（リネーム）する。
fn replace_file(temporary: NamedTempFile, destination: &Path) -> Result<(), String> {
    temporary.persist(destination).map(|_| ()).map_err(|error| {
        format!(
            "could not replace project file {}: {}",
            destination.display(),
            error.error
        )
    })
}
