use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::Duration,
};

pub struct AudioPlayer {
    sink: Sink,
    stream: OutputStream,
    path: PathBuf,
    duration: Option<Duration>,
}

impl AudioPlayer {
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
    pub fn play(&mut self) -> Result<(), String> {
        if self.sink.empty() {
            self.seek(0)?;
        }
        self.sink.play();
        Ok(())
    }
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
