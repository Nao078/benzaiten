use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
};

use crate::domain::lyrics::LyricLine;

#[derive(Debug, Clone)]
pub struct ToolSettings {
    pub ffmpeg: PathBuf,
    pub model: PathBuf,
    pub vocabulary: Option<PathBuf>,
    pub language: String,
    pub threads: usize,
}

impl ToolSettings {
    pub fn validate_model(&self, lyrics: &[LyricLine]) -> Result<(), String> {
        if self.model.as_os_str().is_empty() {
            return Err("Forced Alignmentモデルが未設定です。「外部ツール設定」→「モデル」→「参照」で、Wav2Vec2 ONNXモデルを選択してください。".into());
        }
        if !self.model.is_file() {
            return Err(format!("Forced Alignmentモデルが見つかりません: {}。「外部ツール設定」からファイルを選び直してください。", self.model.display()));
        }
        if crate::forced_alignment::tokenizer::resolve_language(&self.language, lyrics)?
            == crate::forced_alignment::tokenizer::AlignmentLanguage::Japanese
        {
            let vocabulary = self.vocabulary.as_ref().ok_or(
                "日本語tokenizerが未設定です。外部ツール設定からtokenizer.jsonを選択してください。",
            )?;
            if !vocabulary.is_file() {
                return Err(format!(
                    "日本語tokenizerが見つかりません: {}",
                    vocabulary.display()
                ));
            }
        }
        Ok(())
    }
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            ffmpeg: PathBuf::from("ffmpeg"),
            model: PathBuf::from("models/wav2vec2-base-960h.onnx"),
            vocabulary: None,
            language: "en".to_owned(),
            threads: 8,
        }
    }
}

#[derive(Debug)]
pub enum JobEvent {
    Stage(String),
    Finished(Result<Vec<LyricLine>, String>),
}

/// A single background alignment pipeline. Its receiver is non-blocking so it
/// can be polled from the GUI event loop.
pub struct Job {
    receiver: Receiver<JobEvent>,
    cancel: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Job {
    pub fn start(audio: PathBuf, lyrics: Vec<LyricLine>, settings: ToolSettings) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker = thread::spawn(move || {
            let result = run_with_events(&audio, &lyrics, &settings, worker_cancel, |stage| {
                let _ = sender.send(JobEvent::Stage(stage.to_owned()));
            });
            let _ = sender.send(JobEvent::Finished(result));
        });
        Self {
            receiver,
            cancel,
            worker: Some(worker),
        }
    }

    pub fn try_recv(&self) -> Result<JobEvent, TryRecvError> {
        self.receiver.try_recv()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    /// Stop the worker and wait until its child process has been reaped.
    /// Call this while shutting down the application; ordinary GUI cancellation
    /// should use [`Self::cancel`] so the final event can still be displayed.
    pub fn shutdown(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Run the same pipeline synchronously for the command-line entry point.
pub fn run_sync(
    audio: &Path,
    lyrics: &[LyricLine],
    settings: &ToolSettings,
) -> Result<Vec<LyricLine>, String> {
    run_with_events(
        audio,
        lyrics,
        settings,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
}

fn run_with_events(
    audio: &Path,
    lyrics: &[LyricLine],
    settings: &ToolSettings,
    cancel: Arc<AtomicBool>,
    mut stage: impl FnMut(&str),
) -> Result<Vec<LyricLine>, String> {
    validate(audio, lyrics, settings)?;
    check_cancelled(&cancel)?;

    let temporary = tempfile::Builder::new()
        .prefix("benzaiten-")
        .tempdir()
        .map_err(|error| format!("could not create temporary directory: {error}"))?;
    let wav = temporary.path().join("audio-16k-mono.wav");
    stage("音声を変換中");
    crate::audio::preprocess::to_pcm16_mono_16khz(
        &settings.ffmpeg,
        audio,
        &wav,
        Arc::clone(&cancel),
    )?;
    check_cancelled(&cancel)?;

    stage("Forced Alignmentモデルを読み込み中");
    let mut model =
        crate::forced_alignment::model::OnnxWav2Vec2::load(&settings.model, settings.threads)?;
    let mut report_progress = |percent| stage(&format!("音響特徴を解析中: {percent}%"));
    let emissions = crate::forced_alignment::AcousticModel::infer(
        &mut model,
        &wav,
        Arc::clone(&cancel),
        &mut report_progress,
    )?;
    check_cancelled(&cancel)?;

    stage("正解歌詞をForced Alignment中");
    let language =
        crate::forced_alignment::tokenizer::resolve_language(&settings.language, lyrics)?;
    let aligned = match language {
        crate::forced_alignment::tokenizer::AlignmentLanguage::English => {
            crate::forced_alignment::align(lyrics, &emissions.frames, emissions.frame_duration_ms)
        }
        crate::forced_alignment::tokenizer::AlignmentLanguage::Japanese => {
            let vocabulary = crate::forced_alignment::tokenizer::Vocabulary::load(
                settings.vocabulary.as_ref().expect("validated vocabulary"),
            )?;
            crate::forced_alignment::align_japanese(
                lyrics,
                &emissions.frames,
                emissions.frame_duration_ms,
                &vocabulary,
            )
        }
    };
    check_cancelled(&cancel)?;
    aligned
}

fn validate(audio: &Path, lyrics: &[LyricLine], settings: &ToolSettings) -> Result<(), String> {
    if !audio.is_file() {
        return Err(format!("audio file was not found: {}", audio.display()));
    }
    if !crate::process::is_bare_command(&settings.ffmpeg) && !settings.ffmpeg.is_file() {
        return Err(format!(
            "ffmpeg executable was not found: {}",
            settings.ffmpeg.display()
        ));
    }
    settings.validate_model(lyrics)?;
    if settings.threads == 0 {
        return Err("Forced Alignment thread count must be at least 1".to_owned());
    }
    Ok(())
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("processing cancelled".to_owned())
    } else {
        Ok(())
    }
}
