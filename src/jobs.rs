//! Forced Alignmentパイプライン全体（音声前処理→ONNX推論→整列）を、
//! GUIからはキャンセル可能な非同期ジョブとして、CLIからは同期関数
//! として実行できるようにする。

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

/// パイプライン実行に必要な外部ツール・モデルの設定一式。
/// GUIでは「外部ツール設定」ダイアログの内容がこれになる。
#[derive(Debug, Clone)]
pub struct ToolSettings {
    pub model: PathBuf,
    pub vocabulary: Option<PathBuf>,
    pub language: String,
    pub threads: usize,
}

impl ToolSettings {
    /// 実行前に、選択言語に応じて必要なモデル・語彙ファイルが揃っているか
    /// 検証する。日本語の場合は`vocabulary`（tokenizer.json）が必須。
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
            model: PathBuf::from("models/wav2vec2-base-960h.onnx"),
            vocabulary: None,
            language: "en".to_owned(),
            threads: 8,
        }
    }
}

/// バックグラウンドジョブからGUIへ送られるイベント。`Stage`は
/// ステータスバーに表示する進捗メッセージ、`Finished`は最終結果。
#[derive(Debug)]
pub enum JobEvent {
    Stage(String),
    Finished(Result<Vec<LyricLine>, String>),
}

/// 1回分のバックグラウンドアライメントパイプライン。受信は
/// ノンブロッキングなので、GUIのイベントループから毎フレームpollできる。
pub struct Job {
    receiver: Receiver<JobEvent>,
    cancel: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Job {
    /// 別スレッドでパイプラインを開始する。進捗・結果は`try_recv`で
    /// 取得する。
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

    /// キャンセルフラグを立てる。ワーカースレッドの終了は待たない
    /// （最終的な`Finished`イベントをGUI側でまだ表示できるようにするため）。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    /// ワーカーを停止させ、子プロセスが完全に後始末されるまで待つ。
    /// アプリ終了時に呼ぶこと。通常のGUI操作によるキャンセルは、
    /// 最終イベントをまだ表示できるよう[`Self::cancel`]を使うこと。
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

/// CLIエントリポイント向けに、同じパイプラインを同期的に実行する。
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

/// パイプライン本体：入力検証 → 音声前処理 → モデル読込 → 音響推論 →
/// 言語判定 → Forced Alignment、の順に実行する。各段階の間で
/// `check_cancelled`を挟み、キャンセルされていれば早期に打ち切る。
/// `stage`コールバックには各段階の進捗メッセージを渡す（GUIではこれを
/// ステータスバーに表示する）。
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
    crate::audio::preprocess::to_pcm16_mono_16khz(audio, &wav, Arc::clone(&cancel))?;
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

/// パイプライン実行前の前提条件（音声ファイル・モデル・語彙・スレッド数）
/// を検証する。
fn validate(audio: &Path, lyrics: &[LyricLine], settings: &ToolSettings) -> Result<(), String> {
    if !audio.is_file() {
        return Err(format!("audio file was not found: {}", audio.display()));
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
