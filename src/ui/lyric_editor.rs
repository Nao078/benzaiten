//! タイムラインや時刻補正パネルから呼ばれる、歌詞行の開始・終了時刻を
//! 操作する純粋関数群。GUI状態を持たず、`&mut LyricLine`（またはその
//! スライス）だけを扱う。

use crate::domain::lyrics::{LyricLine, TimestampSource};

/// 行の開始時刻を設定する。`end_ms`が新しい開始時刻より前になる場合は
/// 矛盾を避けるためクリアする。手動操作の結果なので、由来を`Manual`にし、
/// 古いconfidenceは意味を失うためクリアする。
pub fn set_start(line: &mut LyricLine, ms: u64) {
    line.start_ms = Some(ms);
    if line.end_ms.is_some_and(|end| end < ms) {
        line.end_ms = None;
    }
    line.timestamp_source = Some(TimestampSource::Manual);
    line.confidence = None;
}

/// 行の終了時刻を明示的に設定する（タイムラインのブロック右端ハンドルを
/// ドラッグした場合など）。
pub fn set_end(line: &mut LyricLine, ms: u64) {
    line.end_ms = Some(ms);
    line.timestamp_source = Some(TimestampSource::Manual);
    line.confidence = None;
}

/// 行の開始時刻を相対量`delta`だけ動かす。`duration`（音声の長さ）が
/// あれば、それを超えないようクランプする。
pub fn shift_start(line: &mut LyricLine, delta: i64, duration: Option<u64>) {
    if let Some(ms) = line.start_ms {
        let shifted = ms.saturating_add_signed(delta);
        set_start(line, duration.map_or(shifted, |end| shifted.min(end)));
    }
}

/// スライス内の位置（`index`）で指定した1行の開始時刻を設定する。
///
/// 対象の行だけを変更する。前の行がすでに持っていた終了時刻は、
/// この結果ギャップやオーバーラップが生じたとしてもそのままにする
/// ——ある行の境界を動かしたことで、隣の行の境界まで黙って
/// 書き換えられるべきではない、という考え方による。
pub fn set_line_start(lines: &mut [LyricLine], index: usize, ms: u64) {
    if let Some(line) = lines.get_mut(index) {
        set_start(line, ms);
    }
}

/// `index`番目の行の開始時刻を相対量`delta`だけ動かす
/// （[`set_line_start`]を介して行うので、前の行には影響しない）。
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

/// `index`以降のすべての時刻を、同じ実効delta分だけまとめて動かす。
///
/// 時刻がゼロや音声の長さを超えないよう、必要に応じてdeltaを縮小する。
/// 全行に共通の量だけ動かすことで、`index..`内での行同士の間隔は
/// 保たれる。`index`より前の行は[`set_line_start`]と同様に一切
/// 変更しない。
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

/// 歌詞ブロックを、その長さ（開始〜終了の幅）を保ったまま絶対位置
/// `requested_start`へ移動する（タイムライン上でブロック全体を
/// ドラッグする操作に対応）。
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
