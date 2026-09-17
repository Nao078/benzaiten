//! GUIの再生・一時停止・シーク操作のための、薄いrodioラッパー。

use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::Duration,
};

/// 開いている音声ファイルとその再生状態。1つの`Sink`が1つのデコード済み
/// ストリームを再生する。rodioのsinkはその場での巻き戻しができないため、
/// sinkが空になった後の`seek`は、`path`から透過的に再デコードし直す。
pub struct AudioPlayer {
    sink: Sink,
    stream: OutputStream,
    path: PathBuf,
    duration: Option<Duration>,
}

impl AudioPlayer {
    /// `path`を開き、総再生時間を読み取れる程度にデコードして、
    /// 位置0で一時停止した状態にする。
    pub fn open(path: &Path) -> Result<Self, String> {
        let decoder = Decoder::try_from(File::open(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let duration = decoder.total_duration();
        let stream = OutputStreamBuilder::open_default_stream().map_err(|e| e.to_string())?;
        let sink = Sink::connect_new(stream.mixer());
        sink.pause();
        sink.append(decoder);
        Ok(Self {
            sink,
            stream,
            path: path.to_owned(),
            duration,
        })
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.duration.map(|d| d.as_millis() as u64)
    }

    /// 現在の再生位置。sinkが再生し切って空になった後は、
    /// 不定な古い位置ではなく曲の長さ（不明なら0）を返す。
    pub fn position_ms(&self) -> u64 {
        if self.sink.empty() {
            self.duration_ms().unwrap_or(0)
        } else {
            self.sink.get_pos().as_millis() as u64
        }
    }

    pub fn is_playing(&self) -> bool {
        !self.sink.is_paused() && !self.sink.empty()
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    /// 再生を再開する。sinkが空になっていた（終端に達していた）場合は、
    /// 先に位置0へシークし直してから再生するため、再生ボタンを押しても
    /// 何も起きない、という状態を避けて曲を最初から再生し直す。
    pub fn play(&mut self) -> Result<(), String> {
        if self.sink.empty() {
            self.seek(0)?;
        }
        self.sink.play();
        Ok(())
    }

    /// `ms`（曲の長さでクランプ済み）へシークする。sinkが空になっている
    /// 場合は、rodioが使い切ったソースの内部でシークできないため、
    /// ファイルを最初から再デコードして新しいsinkへ積み直す。
    pub fn seek(&mut self, ms: u64) -> Result<(), String> {
        let ms = self.duration_ms().map_or(ms, |duration| ms.min(duration));
        if self.sink.empty() {
            let decoder = Decoder::try_from(File::open(&self.path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            self.sink = Sink::connect_new(self.stream.mixer());
            self.sink.pause();
            self.sink.append(decoder);
        }
        self.sink
            .try_seek(Duration::from_millis(ms))
            .map_err(|e| e.to_string())
    }
}
