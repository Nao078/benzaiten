//! [`Project`]を標準LRCテキストへレンダリングし、アトミックに書き出す。

use std::{io::Write, path::Path};

use tempfile::NamedTempFile;

use crate::domain::project::Project;

/// 標準LRCを生成する。フォーマットの慣例である`[mm:ss.cc]`表記に合わせ、
/// 時刻はセンチ秒へ切り捨てる。
///
/// 空の歌詞行は完全にスキップする（LRCには空行という概念がない）。
/// 空でない行にはすべて`start_ms`が必要で、時刻は単調非減少でなければ
/// ならない。また曲名・アーティスト・歌詞テキストに改行が含まれていては
/// いけない（含まれていると、出力に任意の`[tag:]`行を注入できてしまう
/// ため）。いずれかの違反があれば、不完全な出力を出さずレンダリング
/// 全体をエラーにする。
pub fn render(project: &Project) -> Result<String, String> {
    validate_metadata("title", &project.title)?;
    validate_metadata("artist", &project.artist)?;

    let mut output = format!("[ti:{}]\n[ar:{}]\n", project.title, project.artist);
    let mut previous_timestamp = None;
    for line in &project.lyrics {
        if line.original_text.trim().is_empty() {
            continue;
        }
        if line.original_text.contains('\n') || line.original_text.contains('\r') {
            return Err(format!("lyric line {} contains a newline", line.id));
        }
        let timestamp = line
            .start_ms
            .ok_or_else(|| format!("lyric line {} has text but no start timestamp", line.id))?;
        if previous_timestamp.is_some_and(|previous| timestamp < previous) {
            return Err(format!(
                "lyric line {} is earlier than the preceding line",
                line.id
            ));
        }
        previous_timestamp = Some(timestamp);
        output.push_str(&format_timestamp(timestamp));
        output.push_str(&line.original_text);
        output.push('\n');
    }
    Ok(output)
}

/// `project`をレンダリングして`path`へ書き出す。書き込みはアトミックに
/// 行う（同じディレクトリ内の一時ファイルへまず書き込み、それを
/// リネームして配置する）ため、クラッシュや同時読み込みが発生しても
/// 書きかけの`.lrc`ファイルが見えることはない。
pub fn export(path: &Path, project: &Project) -> Result<(), String> {
    let rendered = render(project)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        return Err(format!(
            "LRC directory does not exist: {}",
            parent.display()
        ));
    }
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("could not create temporary LRC file: {error}"))?;
    temporary
        .write_all(rendered.as_bytes())
        .and_then(|_| temporary.flush())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("could not write LRC file {}: {error}", path.display()))?;
    temporary.persist(path).map(|_| ()).map_err(|error| {
        format!(
            "could not replace LRC file {}: {}",
            path.display(),
            error.error
        )
    })
}

/// メタデータ項目に改行が含まれていないか検証する。改行が入っていると、
/// LRC出力に余計な`[tag:]`風の行を注入できてしまうため拒否する。
fn validate_metadata(field: &str, value: &str) -> Result<(), String> {
    if value.contains('\n') || value.contains('\r') {
        return Err(format!("{field} must not contain a newline"));
    }
    Ok(())
}

/// ミリ秒の時刻をLRCの`[mm:ss.cc]`形式にフォーマットする。フォーマットの
/// 慣例に従い、センチ秒未満は四捨五入せず切り捨てる。
fn format_timestamp(milliseconds: u64) -> String {
    let centiseconds = milliseconds / 10;
    let minutes = centiseconds / 6_000;
    let seconds = (centiseconds % 6_000) / 100;
    let remainder = centiseconds % 100;
    format!("[{minutes:02}:{seconds:02}.{remainder:02}]")
}
