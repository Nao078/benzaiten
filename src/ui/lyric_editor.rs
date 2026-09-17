use crate::domain::lyrics::{LyricLine, TimestampSource};

pub fn set_start(line: &mut LyricLine, ms: u64) {
    line.start_ms = Some(ms);
    if line.end_ms.is_some_and(|end| end < ms) {
        line.end_ms = None;
    }
    line.timestamp_source = Some(TimestampSource::Manual);
    line.confidence = None;
}

/// Set a line's end time explicitly, e.g. from dragging the timeline block's
/// right-edge handle.
pub fn set_end(line: &mut LyricLine, ms: u64) {
    line.end_ms = Some(ms);
    line.timestamp_source = Some(TimestampSource::Manual);
    line.confidence = None;
}

pub fn shift_start(line: &mut LyricLine, delta: i64, duration: Option<u64>) {
    if let Some(ms) = line.start_ms {
        let shifted = ms.saturating_add_signed(delta);
        set_start(line, duration.map_or(shifted, |end| shifted.min(end)));
    }
}

/// Set one line's start, identified by index within a slice.
///
/// This only touches the targeted line. Earlier lines keep whatever end time
/// they already had, even if that now leaves a gap or overlap next to this
/// line — moving one line's boundary should not silently reshape its
/// neighbor's.
pub fn set_line_start(lines: &mut [LyricLine], index: usize, ms: u64) {
    if let Some(line) = lines.get_mut(index) {
        set_start(line, ms);
    }
}

pub fn shift_line_start(lines: &mut [LyricLine], index: usize, delta: i64, duration: Option<u64>) {
    let Some(current) = lines.get(index).and_then(|line| line.start_ms) else {
        return;
    };
    let shifted = current.saturating_add_signed(delta);
    set_line_start(
        lines,
        index,
        duration.map_or(shifted, |end| shifted.min(end)),
    );
}

/// Shift every timestamp from `index` by the same effective delta.
///
/// The delta is reduced when necessary so no timestamp crosses zero or the
/// audio duration. Keeping a common delta preserves spacing between lines in
/// `index..`. The line before `index` is left untouched, same as
/// `set_line_start`.
pub fn shift_from(
    lines: &mut [LyricLine],
    index: usize,
    requested_delta: i64,
    duration: Option<u64>,
) -> i64 {
    let (minimum, maximum) = {
        let Some(tail) = lines.get(index..) else {
            return 0;
        };
        let timestamps = tail
            .iter()
            .flat_map(|line| [line.start_ms, line.end_ms])
            .flatten();
        let Some(minimum) = timestamps.clone().min() else {
            return 0;
        };
        (minimum, timestamps.max().unwrap_or(minimum))
    };
    let lower_bound = -(minimum.min(i64::MAX as u64) as i64);
    let upper_bound = duration
        .map(|end| end.saturating_sub(maximum).min(i64::MAX as u64) as i64)
        .unwrap_or(i64::MAX);
    let delta = requested_delta.clamp(lower_bound, upper_bound);
    if delta == 0 {
        return 0;
    }

    for line in &mut lines[index..] {
        line.start_ms = line
            .start_ms
            .map(|value| value.saturating_add_signed(delta));
        line.end_ms = line.end_ms.map(|value| value.saturating_add_signed(delta));
        if line.start_ms.is_some() {
            line.timestamp_source = Some(TimestampSource::Manual);
            line.confidence = None;
        }
    }
    delta
}

/// Move a lyric block to an absolute start while preserving its captured length.
pub fn move_line_to(
    lines: &mut [LyricLine],
    index: usize,
    requested_start: u64,
    block_length: Option<u64>,
    audio_duration: Option<u64>,
) {
    if index >= lines.len() {
        return;
    }
    let maximum_start = match (audio_duration, block_length) {
        (Some(duration), Some(length)) => duration.saturating_sub(length),
        (Some(duration), None) => duration,
        (None, _) => u64::MAX,
    };
    let start = requested_start.min(maximum_start);
    set_line_start(lines, index, start);
    if let Some(length) = block_length {
        lines[index].end_ms = Some(start.saturating_add(length));
    }
}
