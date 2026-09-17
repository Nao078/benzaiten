use std::{io::Write, path::Path};

use tempfile::NamedTempFile;

use crate::domain::project::Project;

/// Renders standard LRC. Timing is rounded down to centiseconds as required by
/// the format's conventional `[mm:ss.cc]` representation.
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

fn validate_metadata(field: &str, value: &str) -> Result<(), String> {
    if value.contains('\n') || value.contains('\r') {
        return Err(format!("{field} must not contain a newline"));
    }
    Ok(())
}

fn format_timestamp(milliseconds: u64) -> String {
    let centiseconds = milliseconds / 10;
    let minutes = centiseconds / 6_000;
    let seconds = (centiseconds % 6_000) / 100;
    let remainder = centiseconds % 100;
    format!("[{minutes:02}:{seconds:02}.{remainder:02}]")
}
