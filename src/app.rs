//! GUI本体：`BenzaitenApp`（eframe::Appの実装）、3ペイン＋下部タイムラインの
//! 描画、メニューバーのコマンド処理、音声再生・タグ書込み・LRC出力などの
//! 操作を1ファイルに集約している。

use crate::{
    audio::player::AudioPlayer,
    domain::{
        lyrics::{apply_readings, parse_lyrics},
        project::{Project, ReadingSource},
    },
    jobs::{Job, JobEvent, ToolSettings},
    lrc::writer,
    project::storage,
    pronunciation::{english::EnglishPronunciationEngine, PronunciationEngine},
    ui::{
        lyric_editor,
        player::format_time,
        timeline::{self, DragAnchor, ResizeAnchor, ResizeEdge, TimelineAction},
    },
};

/// 右ペインの歌詞リストに何を表示するか（原文のみ／カタカナのみ／両方）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LyricsDisplayMode {
    Original,
    Reading,
    OriginalAndReading,
}

/// メニューバー・キーボードショートカットから発行される、遅延実行の
/// コマンド。フレーム内で一旦`Option<AppCommand>`に集約し、
/// `execute_command`でまとめて処理する。
#[derive(Debug, Clone, Copy)]
enum AppCommand {
    OpenProject,
    SaveProject,
    SaveProjectAs,
    ExportLrc,
    OpenAudio,
    OpenOriginalLyrics,
    OpenReadingLyrics,
    ShowExternalTools,
    WriteAudioTags(AudioTagFormat),
}

/// 「音声ファイルへタグを書き込む」サブメニューで選ぶ、タグ書込み先の
/// 音声コンテナ形式。FLACとM4Aだけがloftyで書き込める汎用の歌詞タグ
/// （Vorbis Comment／`©lyr`）を持つ。MP3のID3v2には専用のSYLTフレームが
/// 無いと同等のことができないため、従来どおり別ファイルの`.lrc`を使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioTagFormat {
    Mp3WithLrc,
    Flac,
    M4a,
}

impl AudioTagFormat {
    /// 変換先ファイルに使う拡張子。
    fn extension(self) -> &'static str {
        match self {
            AudioTagFormat::Mp3WithLrc => "mp3",
            AudioTagFormat::Flac => "flac",
            AudioTagFormat::M4a => "m4a",
        }
    }

    /// 確認ダイアログなどの表示に使うラベル。
    fn label(self) -> &'static str {
        match self {
            AudioTagFormat::Mp3WithLrc => "MP3",
            AudioTagFormat::Flac => "FLAC",
            AudioTagFormat::M4a => "M4A",
        }
    }

    /// 可逆音源からこの形式へ変換するときだけ使うffmpegコーデック引数。
    /// 1回限りの非可逆エンコードとなるため、十分な品質・ビットレートを
    /// 指定している。
    fn ffmpeg_codec_args(self) -> &'static [&'static str] {
        match self {
            AudioTagFormat::Mp3WithLrc => &["-c:a", "libmp3lame", "-q:a", "2"],
            AudioTagFormat::Flac => &["-c:a", "flac"],
            AudioTagFormat::M4a => &["-c:a", "aac", "-b:a", "256k"],
        }
    }
}

/// `path`の拡張子から、対応する[`AudioTagFormat`]を推測する。サブメニューで
/// 読み込み中の音声と一致しない項目をグレーアウトするために使う。実際の
/// 書込み時にはloftyが実ファイルの種別を改めて判定するため、拡張子を
/// 偽装されても誤った種類のタグに歌詞が書き込まれることはない。
fn detected_audio_tag_format(path: &Path) -> Option<AudioTagFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "mp3" => Some(AudioTagFormat::Mp3WithLrc),
        "flac" => Some(AudioTagFormat::Flac),
        "m4a" | "m4b" | "mp4" => Some(AudioTagFormat::M4a),
        _ => None,
    }
}

/// このアプリが「可逆」として扱う拡張子。可逆音源であれば、非可逆圧縮を
/// 重ねることなく3つのタグ埋め込み先すべてへ自由に変換できる。*非可逆*な
/// 音源（MP3、あるいはM4A内のAAC）を変換すると、別の非可逆形式への
/// 二重圧縮になるか、劣化済みの音声を可逆コンテナへ包むだけで意味がない
/// ため、これらは自分自身と一致するメニュー項目だけを有効にする。
///
/// `.m4a`は（圧倒的に多いケースである）非可逆のAACを保持していると
/// 仮定している。M4Aコンテナ内の可逆ALAC音声を検出する処理は行わない。
fn is_lossless_audio_extension(extension: &str) -> bool {
    matches!(extension, "wav" | "flac")
}
use eframe::egui;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

/// 開発時のカレントディレクトリ、または実行ファイルの祖先ディレクトリ
/// から`relative`（例：`models/wav2vec2-base-960h.onnx`）を探す。
/// `cargo run`実行時と、ビルド済みexeを別の場所から実行する場合の
/// 両方で既定パスを解決できるようにするための処理。
fn find_project_file(relative: &Path) -> Option<PathBuf> {
    if let Ok(current_dir) = std::env::current_dir() {
        let candidate = current_dir.join(relative);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let executable = std::env::current_exe().ok()?;
    for directory in executable.parent()?.ancestors() {
        let candidate = directory.join(relative);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// `position_ms`時点でアクティブな（ハイライト対象の）歌詞行のインデックス
/// を返す。行は`start_ms <= position < end_ms`で判定し、`end_ms`が無い
/// 行は次に時刻が設定されている行の`start_ms`を境界として扱う。
fn active_lyric_index(
    lyrics: &[crate::domain::project::LyricLine],
    position_ms: u64,
) -> Option<usize> {
    lyrics.iter().enumerate().find_map(|(index, line)| {
        let start = line.start_ms?;
        let end = line.end_ms.or_else(|| {
            lyrics[index + 1..]
                .iter()
                .find_map(|following| following.start_ms)
        });
        (position_ms >= start && end.is_none_or(|end| position_ms < end)).then_some(index)
    })
}

/// 原文入力欄のテキストを再パースしつつ、変更されていない行については
/// 既存の時刻・カタカナ読みなどのデータを引き継ぐ。
///
/// まず新しいテキストを行ごとにパースし、各新規行を`original_text`が
/// 一致する未使用の旧行と対応付ける（同じ位置を優先し、無ければ他の
/// 位置から探す）。これにより、歌詞の一部を編集しても、変更していない
/// 行のアライメント結果や手動補正が失われないようにしている。
fn merge_original_lyrics(
    previous: Vec<crate::domain::project::LyricLine>,
    input: &str,
) -> Vec<crate::domain::project::LyricLine> {
    let mut parsed = if input.is_empty() {
        Vec::new()
    } else {
        parse_lyrics(input)
    };
    let mut used = vec![false; previous.len()];
    let mut sources = vec![None; parsed.len()];
    for (index, line) in parsed.iter().enumerate() {
        let source = previous
            .get(index)
            .filter(|old| !used[index] && old.original_text == line.original_text)
            .map(|_| index)
            .or_else(|| {
                previous.iter().enumerate().position(|(old_index, old)| {
                    !used[old_index] && old.original_text == line.original_text
                })
            });
        if let Some(source) = source {
            sources[index] = Some(source);
            used[source] = true;
        }
    }
    for (index, (line, source)) in parsed.iter_mut().zip(sources).enumerate() {
        if let Some(source) = source {
            let old = &previous[source];
            line.reading_text.clone_from(&old.reading_text);
            line.reading_source = old.reading_source;
            line.start_ms = old.start_ms;
            line.end_ms = old.end_ms;
            line.confidence = old.confidence;
            line.timestamp_source = old.timestamp_source;
        }
        line.id = index;
    }
    parsed
}

/// アプリ全体の状態。`eframe::App`を実装し、`update`が毎フレーム呼ばれる。
pub struct BenzaitenApp {
    project: Project,
    player: Option<AudioPlayer>,
    /// タイムライン・時刻補正パネルで選択中の歌詞行インデックス。
    selected: Option<usize>,
    lyrics_display_mode: LyricsDisplayMode,
    job: Option<Job>,
    job_started_at: Option<Instant>,
    /// 歌詞やプロジェクトが変更されるたびに増える世代カウンタ。
    generation: u64,
    /// ジョブ開始時点の`generation`のスナップショット。ジョブ完了時に
    /// `generation`と食い違っていれば、開始後に入力が変わったということ
    /// なので、結果を古いものとして破棄する（`poll_job`参照）。
    job_generation: u64,
    status: String,
    ffmpeg: String,
    model: String,
    japanese_model: String,
    japanese_vocabulary: String,
    language: String,
    alignment_threads: usize,
    project_path: Option<PathBuf>,
    /// 保存されていない変更があるかどうか。ウィンドウを閉じる際などの
    /// 破棄確認に使う。
    dirty: bool,
    /// 現在のアートワークの生データ（画像デコード前）。
    artwork_bytes: Option<Vec<u8>>,
    /// `artwork_bytes`をGPUテクスチャへデコードしたもの。`None`のときは
    /// 次フレームで`ensure_artwork_texture`が再デコードする。
    artwork_texture: Option<egui::TextureHandle>,
    auto_scroll: bool,
    /// 直前フレームでハイライトしていた行。自動スクロールを行変化時
    /// だけに起動するための比較に使う。
    last_highlighted: Option<usize>,
    workspace_root: PathBuf,
    /// 時刻補正パネルの「調整幅」（±ボタンの移動量）。
    bulk_shift_ms: i64,
    show_external_tools: bool,
    timeline_zoom: f32,
    timeline_drag: Option<DragAnchor>,
    timeline_resize: Option<ResizeAnchor>,
    timeline_link_tail_movement: bool,
    /// 左ペインの原文テキストエリアの生テキスト（`project.lyrics`とは
    /// 別に保持し、パース失敗時も入力内容を保持できるようにしている）。
    original_lyrics_input: String,
    reading_lyrics_input: String,
}

impl BenzaitenApp {
    /// eframeのウィンドウ生成時に一度だけ呼ばれる初期化処理。日本語フォント
    /// の探索と、外部ツール（ffmpeg・モデル・語彙）の既定パス解決を行う。
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        // 独自の日本語フォントを同梱・再配布せず、OSに入っている
        // フォントを順に探して使う。
        let candidates = [
            "C:/Windows/Fonts/meiryo.ttc",
            "C:/Windows/Fonts/YuGothR.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        ];
        for path in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("japanese".into(), egui::FontData::from_owned(bytes).into());
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(0, "japanese".into());
                break;
            }
        }
        cc.egui_ctx.set_fonts(fonts);
        let model = find_project_file(Path::new("models/wav2vec2-base-960h.onnx"))
            .unwrap_or_else(|| PathBuf::from("models/wav2vec2-base-960h.onnx"));
        let japanese_model =
            find_project_file(Path::new("models/wav2vec2-large-xlsr-53-japanese.onnx"))
                .unwrap_or_else(|| PathBuf::from("models/wav2vec2-large-xlsr-53-japanese.onnx"));
        let japanese_vocabulary = find_project_file(Path::new(
            "models/wav2vec2-large-xlsr-53-japanese-tokenizer.json",
        ))
        .unwrap_or_else(|| PathBuf::from("models/wav2vec2-large-xlsr-53-japanese-tokenizer.json"));
        let ffmpeg = if cfg!(windows) {
            find_project_file(Path::new("ffmpeg/bin/ffmpeg.exe"))
                .unwrap_or_else(|| PathBuf::from("ffmpeg.exe"))
        } else {
            PathBuf::from("ffmpeg")
        };
        Self {
            project: Project::default(),
            player: None,
            selected: None,
            lyrics_display_mode: LyricsDisplayMode::Original,
            job: None,
            job_started_at: None,
            generation: 0,
            job_generation: 0,
            status: "音声と歌詞を開いてください".into(),
            ffmpeg: ffmpeg.to_string_lossy().into_owned(),
            model: model.to_string_lossy().into_owned(),
            japanese_model: japanese_model.to_string_lossy().into_owned(),
            japanese_vocabulary: japanese_vocabulary.to_string_lossy().into_owned(),
            language: "auto".into(),
            alignment_threads: 8,
            project_path: None,
            dirty: false,
            artwork_bytes: None,
            artwork_texture: None,
            auto_scroll: true,
            last_highlighted: None,
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            bulk_shift_ms: 100,
            show_external_tools: false,
            timeline_zoom: 80.0,
            timeline_drag: None,
            timeline_resize: None,
            timeline_link_tail_movement: false,
            original_lyrics_input: String::new(),
            reading_lyrics_input: String::new(),
        }
    }

    /// プロジェクトが変更されたことを記録する。`generation`を進めて
    /// 実行中のジョブの結果を無効化できるようにし、`dirty`を立てて
    /// 未保存インジケータを表示する。歌詞・タグ・音声パスなどを
    /// 変更する箇所は必ずこれを呼ぶ。
    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.dirty = true;
    }
    /// 処理結果をステータスバー用の文字列に変換して設定する。
    fn report(&mut self, result: Result<(), String>, success: &str) {
        self.status = match result {
            Ok(()) => success.into(),
            Err(e) => format!("エラー: {e}"),
        };
    }
    /// 未保存の変更を破棄してよいか確認する。変更が無ければ確認なしで
    /// 続行を許可し、あればダイアログでユーザーに確認する。
    fn allow_discard(&self) -> bool {
        !self.dirty
            || rfd::MessageDialog::new()
                .set_title("未保存の変更")
                .set_description("未保存の変更を破棄しますか？")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }

    /// `project.lyrics`の内容から、左ペインの原文・カタカナ入力欄の
    /// テキストを再構築する。プロジェクトを開いた直後や、Forced
    /// Alignment完了後など、`project.lyrics`側が正になった後に呼ぶ。
    fn sync_lyric_inputs_from_project(&mut self) {
        self.original_lyrics_input = self
            .project
            .lyrics
            .iter()
            .map(|line| line.original_text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.reading_lyrics_input = self
            .project
            .lyrics
            .iter()
            .map(|line| line.reading_text.as_deref().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
    }

    /// 原文テキストエリアの編集を`project.lyrics`へ反映する。編集のたびに
    /// 呼ばれるため、[`merge_original_lyrics`]で変更されていない行の
    /// 時刻・読みを保ったまま再構築する。行数が変わることがあるので、
    /// カタカナ入力欄も合わせて再同期し、選択中インデックスが範囲外に
    /// なっていれば解除する。
    fn apply_original_lyrics_input(&mut self) {
        let previous = std::mem::take(&mut self.project.lyrics);
        self.project.lyrics = merge_original_lyrics(previous, &self.original_lyrics_input);
        self.selected = self
            .selected
            .filter(|index| *index < self.project.lyrics.len());
        self.reading_lyrics_input = self
            .project
            .lyrics
            .iter()
            .map(|line| line.reading_text.as_deref().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        self.changed();
    }

    /// カタカナテキストエリアの編集を`project.lyrics`へ反映する。原文より
    /// 行数が多い場合はエラーにする（少ない場合は残りの行の読みを未設定
    /// のままにする）。
    fn apply_reading_lyrics_input(&mut self) {
        let rows = self
            .reading_lyrics_input
            .split('\n')
            .map(|row| row.strip_suffix('\r').unwrap_or(row).trim())
            .collect::<Vec<_>>();
        if rows.len() > self.project.lyrics.len() {
            self.status = format!(
                "カタカナ歌詞が原文より多くなっています（原文{}行 / カタカナ{}行）",
                self.project.lyrics.len(),
                rows.len()
            );
            return;
        }
        let mut count = 0;
        for (index, line) in self.project.lyrics.iter_mut().enumerate() {
            let reading = rows.get(index).copied().unwrap_or_default();
            if reading.is_empty() {
                line.reading_text = None;
                line.reading_source = None;
            } else {
                line.reading_text = Some(reading.to_owned());
                line.reading_source = Some(ReadingSource::Manual);
                count += 1;
            }
        }
        self.changed();
        self.status = format!("カタカナ歌詞を{count}行反映しました");
    }

    /// 「プロジェクトを開く」ファイルダイアログを表示する。
    fn open_project_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Project", &["json"])
            .pick_file()
        {
            self.open_project_file(path);
        }
    }

    /// プロジェクトを保存する。`save_as`が`false`かつ既存の保存先が
    /// あればそこへ上書き保存し、それ以外は保存先ダイアログを表示する
    /// （既存パスがあればその場所・ファイル名を初期値にする）。
    fn save_project(&mut self, save_as: bool) {
        let path = if !save_as {
            self.project_path.clone()
        } else {
            None
        }
        .or_else(|| {
            let mut dialog = rfd::FileDialog::new()
                .add_filter("Project", &["json"])
                .set_file_name("project.json");
            if let Some(path) = &self.project_path {
                if let Some(parent) = path.parent() {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(name) = path.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
            }
            dialog.save_file()
        });
        let Some(path) = path else { return };
        let result = storage::save(&path, &self.project);
        if result.is_ok() {
            self.dirty = false;
            if let Some(parent) = path.parent() {
                self.workspace_root = parent.to_owned();
            }
            self.project_path = Some(path);
        }
        self.report(result, "プロジェクトを保存しました");
    }

    /// 「LRC出力」の保存先ダイアログを表示し、選択されたパスへ書き出す。
    fn export_lrc_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("LRC", &["lrc"])
            .set_file_name("lyrics.lrc")
            .save_file()
        {
            let result = writer::export(&path, &self.project);
            self.report(result, "LRCを保存しました");
        }
    }

    /// 「音声を開く」ファイルダイアログを表示する。
    fn open_audio_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio", &["mp3", "wav", "flac", "ogg", "m4a"])
            .pick_file()
        {
            self.set_audio_file(path);
        }
    }

    /// 「原文歌詞を開く」ファイルダイアログを表示する（未保存の変更が
    /// あれば先に破棄確認を行う）。
    fn open_original_lyrics_dialog(&mut self) {
        if self.allow_discard() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("UTF-8 lyrics", &["txt"])
                .pick_file()
            {
                self.load_original_file(&path);
            }
        }
    }

    /// 「カタカナ歌詞を開く」ファイルダイアログを表示する
    /// （原文が未読込の場合は先にそちらを促す）。
    fn open_reading_lyrics_dialog(&mut self) {
        if self.project.lyrics.is_empty() {
            self.status = "先に原文歌詞を読み込んでください".into();
        } else if let Some(path) = rfd::FileDialog::new()
            .add_filter("UTF-8 katakana lyrics", &["txt"])
            .pick_file()
        {
            self.load_reading_file(&path);
        }
    }

    /// フレーム内で集約された[`AppCommand`]を実際の処理へディスパッチする。
    fn execute_command(&mut self, command: AppCommand) {
        match command {
            AppCommand::OpenProject => self.open_project_dialog(),
            AppCommand::SaveProject => self.save_project(false),
            AppCommand::SaveProjectAs => self.save_project(true),
            AppCommand::ExportLrc => self.export_lrc_dialog(),
            AppCommand::OpenAudio => self.open_audio_dialog(),
            AppCommand::OpenOriginalLyrics => self.open_original_lyrics_dialog(),
            AppCommand::OpenReadingLyrics => self.open_reading_lyrics_dialog(),
            AppCommand::ShowExternalTools => self.show_external_tools = true,
            AppCommand::WriteAudioTags(format) => self.confirm_and_write_audio_tags(format),
        }
    }

    /// 「外部ツール設定」ダイアログの中身：歌詞言語の選択、ffmpeg・各種
    /// モデルのパス設定、CPUスレッド数、配布元へのリンク。
    fn show_external_tools_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("自動同期にはffmpegと選択言語のWav2Vec2 ONNXモデルが必要です。");
        let detected = crate::forced_alignment::tokenizer::detect_language(&self.project.lyrics);
        ui.horizontal(|ui| {
            ui.label("歌詞言語");
            egui::ComboBox::from_id_salt("alignment-language")
                .selected_text(match self.language.as_str() {
                    "en" => "en（英語）",
                    "jp" => "jp（日本語）",
                    _ => "Auto",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.language, "auto".into(), "Auto");
                    ui.selectable_value(&mut self.language, "en".into(), "en（英語）");
                    ui.selectable_value(&mut self.language, "jp".into(), "jp（日本語）");
                });
            if self.language == "auto" {
                ui.label(format!("判定: {}", detected.code()));
            }
        });
        for (label, value) in [
            ("ffmpeg", &mut self.ffmpeg),
            ("英語ONNX", &mut self.model),
            ("日本語ONNX", &mut self.japanese_model),
            ("日本語tokenizer", &mut self.japanese_vocabulary),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(value).desired_width(390.0));
                if ui.button("参照").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        *value = path.to_string_lossy().into_owned();
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("CPUスレッド");
            ui.add(egui::DragValue::new(&mut self.alignment_threads).range(1..=32));
        });
        ui.separator();
        ui.hyperlink_to(
            "英語モデル配布元",
            "https://huggingface.co/facebook/wav2vec2-base-960h/tree/main/onnx",
        );
        ui.hyperlink_to(
            "日本語モデル配布元",
            "https://huggingface.co/FinDIT-Studio/wav2vec2-large-xlsr-53-japanese-onnx",
        );
    }

    /// `timeline::show`が返した[`TimelineAction`]群を処理し、選択状態・
    /// 歌詞の時刻・再生位置・追従移動トグルなどへ反映する。
    fn handle_timeline_actions(&mut self, actions: Vec<TimelineAction>, duration: Option<u64>) {
        for action in actions {
            match action {
                TimelineAction::Select(index) => self.selected = Some(index),
                TimelineAction::Seek(milliseconds) => {
                    if let Some(player) = &mut self.player {
                        if let Err(error) = player.seek(milliseconds) {
                            self.status = error;
                        }
                    }
                }
                TimelineAction::BeginDrag(index) => {
                    if let Some(line) = self.project.lyrics.get(index) {
                        if let Some(start_ms) = line.start_ms {
                            let effective_end = line.end_ms.or_else(|| {
                                self.project.lyrics[index + 1..]
                                    .iter()
                                    .find_map(|following| following.start_ms)
                            });
                            self.selected = Some(index);
                            self.timeline_drag = Some(DragAnchor {
                                index,
                                start_ms,
                                length_ms: effective_end.map(|end| end.saturating_sub(start_ms)),
                            });
                        }
                    }
                }
                TimelineAction::Drag { index, delta_ms } => {
                    if let Some(anchor) = self.timeline_drag.filter(|anchor| anchor.index == index)
                    {
                        if self.timeline_link_tail_movement {
                            // `delta_ms`はすでに「このフレームでの」増分な
                            // ので、現在の（前フレームまでにすでに移動済みの）
                            // 開始時刻へそのまま加算していけばフレームを
                            // 重ねるごとに正しく積み上がる。アンカーの
                            // 記録は不要。
                            lyric_editor::shift_from(
                                &mut self.project.lyrics,
                                index,
                                delta_ms,
                                duration,
                            );
                        } else {
                            let start = anchor.start_ms.saturating_add_signed(delta_ms);
                            lyric_editor::move_line_to(
                                &mut self.project.lyrics,
                                index,
                                start,
                                anchor.length_ms,
                                duration,
                            );
                            // `delta_ms`はドラッグ開始からの累計ではなく、
                            // 前フレームからのポインタ移動量（eguiの
                            // `drag_delta`）。これをアンカーへ反映しておく
                            // ことで、次フレームのdeltaが新しい位置を基準に
                            // 積み上がっていき、ドラッグ開始位置へ
                            // 戻ってしまうことがなくなる。
                            self.timeline_drag = Some(DragAnchor {
                                index,
                                start_ms: start,
                                length_ms: anchor.length_ms,
                            });
                        }
                        self.changed();
                    }
                }
                TimelineAction::EndDrag => {
                    self.timeline_drag = None;
                    self.status = "タイムライン上の歌詞時刻を変更しました".into();
                }
                TimelineAction::SetLinkedTailMovement(enabled) => {
                    self.timeline_link_tail_movement = enabled;
                }
                TimelineAction::BeginResize { index, edge } => {
                    if let Some(line) = self.project.lyrics.get(index) {
                        let ms = match edge {
                            ResizeEdge::Start => line.start_ms,
                            ResizeEdge::End => line
                                .end_ms
                                .or_else(|| {
                                    self.project.lyrics[index + 1..]
                                        .iter()
                                        .find_map(|following| following.start_ms)
                                })
                                .or_else(|| line.start_ms.map(|start| start.saturating_add(2_000))),
                        };
                        if let Some(ms) = ms {
                            self.selected = Some(index);
                            self.timeline_resize = Some(ResizeAnchor { index, edge, ms });
                        }
                    }
                }
                TimelineAction::Resize {
                    index,
                    edge,
                    delta_ms,
                } => {
                    const MIN_LENGTH_MS: u64 = 50;
                    if let Some(anchor) = self
                        .timeline_resize
                        .filter(|anchor| anchor.index == index && anchor.edge == edge)
                    {
                        let requested = anchor.ms.saturating_add_signed(delta_ms);
                        // 開始・終了の間を最低MIN_LENGTH_MSだけ空けておく。
                        // 速いドラッグで片方のハンドルがもう一方を
                        // 追い越してしまわないようにするため。
                        let applied = self.project.lyrics.get_mut(index).map(|line| match edge {
                            ResizeEdge::Start => {
                                let max_start = line
                                    .end_ms
                                    .map_or(u64::MAX, |end| end.saturating_sub(MIN_LENGTH_MS));
                                let start = requested.min(max_start);
                                lyric_editor::set_start(line, start);
                                start
                            }
                            ResizeEdge::End => {
                                let min_end = line
                                    .start_ms
                                    .map_or(0, |start| start.saturating_add(MIN_LENGTH_MS));
                                let end = requested.max(min_end).min(duration.unwrap_or(u64::MAX));
                                lyric_editor::set_end(line, end);
                                end
                            }
                        });
                        if let Some(applied) = applied {
                            self.timeline_resize = Some(ResizeAnchor {
                                index,
                                edge,
                                ms: applied,
                            });
                            self.changed();
                        }
                    }
                }
                TimelineAction::EndResize => {
                    self.timeline_resize = None;
                    self.status = "タイムライン上で開始・終了時刻を調整しました".into();
                }
            }
        }
    }
    /// 音声ファイルを切り替える。すでに時刻が設定された行がある状態で
    /// 別の音声へ切り替える場合は、その時刻がもう意味を持たなくなるため
    /// 確認の上ですべて解除する。
    fn set_audio_file(&mut self, path: PathBuf) {
        if self.project.audio_path != path
            && self
                .project
                .lyrics
                .iter()
                .any(|line| line.start_ms.is_some())
            && rfd::MessageDialog::new()
                .set_title("音声の変更")
                .set_description("音声を変更し、すべての行の時刻を解除しますか？")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                != rfd::MessageDialogResult::Yes
        {
            return;
        }
        if self.project.audio_path != path {
            for line in &mut self.project.lyrics {
                line.start_ms = None;
                line.end_ms = None;
                line.confidence = None;
                line.timestamp_source = None;
            }
        }
        self.project.audio_path = path;
        self.changed();
        self.load_audio(true);
    }

    /// ファイルから原文歌詞を読み込み、プロジェクトを丸ごと置き換える
    /// （テキストエリア編集時の[`merge_original_lyrics`]とは異なり、
    /// 既存の時刻・読みは引き継がない）。
    fn load_original_file(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.project.lyrics = parse_lyrics(&text);
                self.selected = None;
                self.sync_lyric_inputs_from_project();
                self.changed();
                self.status = "原文歌詞を読み込みました".into();
            }
            Err(error) => self.status = format!("歌詞読込エラー: {error}"),
        }
    }

    /// ファイルからカタカナ読みを読み込み、[`apply_readings`]で適用する。
    fn load_reading_file(&mut self, path: &Path) {
        if self.project.lyrics.is_empty() {
            self.status = "先に原文歌詞を読み込んでください".into();
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => match apply_readings(&mut self.project.lyrics, &text) {
                Ok(count) => {
                    self.sync_lyric_inputs_from_project();
                    self.changed();
                    self.status = format!("カタカナ歌詞を{count}行読み込みました");
                }
                Err(error) => self.status = format!("カタカナ歌詞読込エラー: {error}"),
            },
            Err(error) => self.status = format!("カタカナ歌詞読込エラー: {error}"),
        }
    }

    /// プロジェクトJSONを開き、現在の状態を丸ごと置き換える
    /// （未保存の変更があれば先に破棄確認を行う）。
    fn open_project_file(&mut self, path: PathBuf) {
        if !self.allow_discard() {
            return;
        }
        match storage::load(&path) {
            Ok(project) => {
                self.project = project;
                self.sync_lyric_inputs_from_project();
                self.workspace_root = path.parent().unwrap_or(Path::new(".")).to_owned();
                self.project_path = Some(path);
                self.selected = None;
                self.changed();
                self.dirty = false;
                self.load_audio(false);
            }
            Err(error) => self.status = error,
        }
    }

    /// Windows Explorerからのドラッグ＆ドロップなど、拡張子だけを
    /// 手掛かりにファイルの種類を判定して適切な読込処理へ振り分ける。
    fn open_file_default(&mut self, path: PathBuf) {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "mp3" | "wav" | "flac" | "ogg" | "m4a" => self.set_audio_file(path),
            "txt" => self.load_original_file(&path),
            "json" => self.open_project_file(path),
            "jpg" | "jpeg" | "png" | "gif" | "webp" => match std::fs::read(&path) {
                Ok(bytes) => {
                    self.project.metadata.artwork_path = Some(path);
                    self.artwork_bytes = Some(bytes);
                    self.artwork_texture = None;
                    self.changed();
                }
                Err(error) => self.status = format!("画像読込エラー: {error}"),
            },
            _ => self.status = format!("未対応のファイルです: {}", path.display()),
        }
    }
    /// `project.audio_path`から`AudioPlayer`を開き直す。`import_metadata`が
    /// `true`のときだけ、音声タグから曲名・アーティスト・メタデータを
    /// プロジェクトへ取り込む（既存プロジェクトを開いた場合は上書きしない）。
    /// アートワークは`import_metadata`に関わらず毎回読み直す。
    fn load_audio(&mut self, import_metadata: bool) {
        match AudioPlayer::open(&self.project.audio_path) {
            Ok(player) => {
                self.player = Some(player);
                self.status = "音声を読み込みました".into();
                if let Ok(metadata) = crate::metadata::read(&self.project.audio_path) {
                    if import_metadata {
                        if !metadata.title.is_empty() {
                            self.project.title = metadata.title;
                        }
                        if !metadata.artist.is_empty() {
                            self.project.artist = metadata.artist;
                        }
                        self.project.metadata = metadata.details;
                    }
                    self.artwork_bytes = self
                        .project
                        .metadata
                        .artwork_path
                        .as_deref()
                        .and_then(|path| std::fs::read(path).ok())
                        .or(metadata.artwork);
                    self.artwork_texture = None;
                }
            }
            Err(e) => {
                self.player = None;
                self.status = format!("音声を再指定してください: {e}");
            }
        }
    }

    /// `artwork_bytes`がまだGPUテクスチャ化されていなければデコードする。
    /// 毎フレーム呼んでも、すでにテクスチャがあれば何もしない。
    fn ensure_artwork_texture(&mut self, ctx: &egui::Context) {
        if self.artwork_texture.is_some() {
            return;
        }
        let Some(bytes) = self.artwork_bytes.as_deref() else {
            return;
        };
        if let Ok(decoded) = image::load_from_memory(bytes) {
            let rgba = decoded.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            self.artwork_texture = Some(ctx.load_texture(
                "album-art",
                egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                egui::TextureOptions::LINEAR,
            ));
        }
    }

    /// タグ書込みの前に確認ダイアログを出す。変換が必要かどうかで
    /// 文言を変える（変換ありなら「新しいファイルを作成する」旨を明示）。
    fn confirm_and_write_audio_tags(&mut self, format: AudioTagFormat) {
        if self.project.audio_path.as_os_str().is_empty() {
            return;
        }
        let description = if detected_audio_tag_format(&self.project.audio_path) == Some(format) {
            match format {
                AudioTagFormat::Mp3WithLrc => {
                    "音声ファイルへタグを書き込み、同じ場所にLRCファイルを保存します。\
                     初回は音声ファイルの同じ場所にバックアップを作成します。続行しますか？"
                        .to_owned()
                }
                AudioTagFormat::Flac | AudioTagFormat::M4a => {
                    "音声ファイルへタグを書き込み、歌詞を埋め込みます。\
                     初回は同じ場所にバックアップを作成します。続行しますか？"
                        .to_owned()
                }
            }
        } else {
            format!(
                "音声を{}へ変換した新しいファイルを同じ場所に作成し、そちらへタグと歌詞を書き込みます。\
                 元の音声ファイルは変更しません。続行しますか？",
                format.label()
            )
        };
        if rfd::MessageDialog::new()
            .set_title("音声タグの書込み")
            .set_description(description)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
        {
            self.write_audio_tags(format);
        }
    }

    /// 現在の音声をffmpegで`format`へ変換し、元ファイルはそのまま残して
    /// 同じ場所に新しいファイルを作成し、そのパスを返す。可逆音源に対して
    /// のみ呼ばれる想定なので、対象が非可逆形式でも1回限りのエンコードで
    /// 済み、非可逆圧縮が重なることはない。
    fn convert_audio_for_tag_format(&self, format: AudioTagFormat) -> Result<PathBuf, String> {
        let target_path = self.project.audio_path.with_extension(format.extension());
        if target_path.exists() {
            return Err(format!(
                "変換先ファイルが既に存在するため中止しました: {}",
                target_path.display()
            ));
        }
        let ffmpeg = PathBuf::from(&self.ffmpeg);
        let mut arguments: Vec<&Path> =
            vec![Path::new("-y"), Path::new("-i"), &self.project.audio_path];
        arguments.extend(format.ffmpeg_codec_args().iter().map(Path::new));
        arguments.push(&target_path);
        crate::process::run(&ffmpeg, &arguments, Arc::new(AtomicBool::new(false)))
            .map(|_| target_path)
    }

    /// 実際にタグを書き込む。必要なら先にffmpeg変換を行い、その後
    /// タグ・（対応形式なら）歌詞の埋め込み、（MP3なら）`.lrc`出力を行う。
    fn write_audio_tags(&mut self, format: AudioTagFormat) {
        // 最初に一度だけレンダリングし、歌詞が出力可能な状態でない
        // （時刻未設定・時刻の逆転など）場合は、音声ファイルに一切
        // 触れずに中止する。通常のLRC出力と同じ検証を適用している。
        // 埋め込み・別ファイル出力のどちらの経路でもこのテキストを使う。
        let lyrics = match writer::render(&self.project) {
            Ok(text) => text,
            Err(error) => {
                self.status = format!("歌詞(LRC)の生成に失敗したため書込みを中止しました: {error}");
                return;
            }
        };

        let converted = detected_audio_tag_format(&self.project.audio_path) != Some(format);
        if converted {
            match self.convert_audio_for_tag_format(format) {
                Ok(converted_path) => {
                    // 音声の中身自体は同じで再エンコードしただけなので、
                    // 既存の行の時刻はそのまま有効——別の音声へ本当に
                    // 切り替える場合とは異なり、リセットは不要。
                    self.project.audio_path = converted_path;
                    self.changed();
                }
                Err(error) => {
                    self.status = format!("音声の変換に失敗しました: {error}");
                    return;
                }
            }
        }

        let position = self
            .player
            .as_ref()
            .map(AudioPlayer::position_ms)
            .unwrap_or(0);
        let was_playing = self.player.as_ref().is_some_and(AudioPlayer::is_playing);
        self.player = None;
        let embed_lyrics = matches!(format, AudioTagFormat::Flac | AudioTagFormat::M4a);
        match crate::metadata::write(
            &self.project.audio_path,
            &self.project.title,
            &self.project.artist,
            &self.project.metadata,
            embed_lyrics.then_some(lyrics.as_str()),
        ) {
            Ok(backup) => {
                self.load_audio(false);
                if let Some(player) = &mut self.player {
                    let _ = player.seek(position);
                    if was_playing {
                        let _ = player.play();
                    }
                }
                let converted_prefix = if converted {
                    format!(
                        "{}へ変換したファイルを作成し、",
                        self.project.audio_path.display()
                    )
                } else {
                    String::new()
                };
                if format == AudioTagFormat::Mp3WithLrc {
                    let lrc_path = self.project.audio_path.with_extension("lrc");
                    match writer::export(&lrc_path, &self.project) {
                        Ok(()) => {
                            self.status = format!(
                                "{converted_prefix}音声タグとLRCを書き込みました（バックアップ: {}、LRC: {}）",
                                backup.display(),
                                lrc_path.display()
                            );
                        }
                        Err(error) => {
                            self.status = format!(
                                "{converted_prefix}音声タグは書き込みましたが、LRC出力に失敗しました（バックアップ: {}）: {error}",
                                backup.display()
                            );
                        }
                    }
                } else {
                    self.status = format!(
                        "{converted_prefix}音声タグと歌詞を書き込みました（バックアップ: {}）",
                        backup.display()
                    );
                }
            }
            Err(error) => {
                self.load_audio(false);
                self.status = format!("タグ書込エラー: {error}");
            }
        }
    }

    /// 「歌唱向けカタカナ自動生成」を実行する。手動入力・手修正済み
    /// （`ReadingSource::Manual`）の行はスキップして上書きしない。
    /// 辞書に無かった単語はまとめてステータスバーに表示する。
    fn generate_readings(&mut self) {
        let engine = EnglishPronunciationEngine;
        let mut generated = 0;
        let mut preserved = 0;
        let mut unknown = BTreeSet::new();
        for line in &mut self.project.lyrics {
            if line.original_text.trim().is_empty() {
                continue;
            }
            if line.reading_source == Some(ReadingSource::Manual) {
                preserved += 1;
                continue;
            }
            match engine.generate(&line.original_text) {
                Ok(result) => {
                    unknown.extend(result.unknown_words);
                    line.reading_text = (!result.reading.is_empty()).then_some(result.reading);
                    line.reading_source =
                        line.reading_text.as_ref().map(|_| ReadingSource::Generated);
                    generated += 1;
                }
                Err(error) => {
                    self.status = format!("カタカナ生成エラー: {error}");
                    return;
                }
            }
        }
        self.changed();
        self.sync_lyric_inputs_from_project();
        self.lyrics_display_mode = LyricsDisplayMode::OriginalAndReading;
        self.status = if unknown.is_empty() {
            format!("カタカナを{generated}行生成しました（手修正{preserved}行を保持）")
        } else {
            let examples = unknown
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "カタカナを{generated}行生成しました。辞書外{}語は要確認: {examples}",
                unknown.len()
            )
        };
    }
    /// 実行中の[`Job`]からのイベントを毎フレーム確認する。`Stage`は
    /// ステータス表示のみ、`Finished`ではジョブ開始後に入力が変更されて
    /// いないか（`job_generation`と`generation`の比較）を確認してから
    /// 結果を反映する。
    fn poll_job(&mut self) {
        let event = self.job.as_ref().map(|j| j.try_recv());
        match event {
            Some(Ok(JobEvent::Stage(stage))) => self.status = stage,
            Some(Ok(JobEvent::Finished(result))) => {
                self.job = None;
                self.job_started_at = None;
                if self.job_generation != self.generation {
                    self.status = "入力が変更されたため同期結果を破棄しました".into();
                    return;
                }
                match result {
                    Ok(lyrics) => {
                        self.project.lyrics = lyrics;
                        self.sync_lyric_inputs_from_project();
                        self.changed();
                        self.status = "同期完了。低信頼・未設定の行を確認してください".into();
                    }
                    Err(e) => self.status = format!("同期終了: {e}"),
                }
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.job = None;
                self.job_started_at = None;
                self.status = "処理スレッドが終了しました".into();
            }
            _ => {}
        }
    }
}

impl eframe::App for BenzaitenApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(job) = &mut self.job {
            job.shutdown();
        }
    }

    /// eguiが毎フレーム呼ぶメインループ。ジョブのポーリング、ドロップ
    /// ファイルの処理、キーボードショートカット、メニューバー、左右の
    /// パネル、下部タイムライン、中央パネルを順に描画・処理する。
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_job();
        self.ensure_artwork_texture(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_discard() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = file.path {
                self.open_file_default(path);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(50));

        let mut command = None;
        let mut toggle_playback = false;
        if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            ))
        }) {
            command = Some(AppCommand::SaveProjectAs);
        } else if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::S,
            ))
        }) {
            command = Some(AppCommand::SaveProject);
        } else if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::O,
            ))
        }) {
            command = Some(AppCommand::OpenProject);
        } else if ctx.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::E,
            ))
        }) {
            command = Some(AppCommand::ExportLrc);
        }

        egui::TopBottomPanel::top("main-menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("ファイル", |ui| {
                    for (label, action) in [
                        ("プロジェクトを開く    Ctrl+O", AppCommand::OpenProject),
                        ("プロジェクト保存      Ctrl+S", AppCommand::SaveProject),
                        ("名前を付けて保存  Ctrl+Shift+S", AppCommand::SaveProjectAs),
                        ("LRC出力                 Ctrl+E", AppCommand::ExportLrc),
                    ] {
                        if ui.button(label).clicked() {
                            command = Some(action);
                        }
                    }
                    ui.separator();
                    if ui.button("音声を開く").clicked() {
                        command = Some(AppCommand::OpenAudio);
                    }
                    if ui.button("原文歌詞を開く").clicked() {
                        command = Some(AppCommand::OpenOriginalLyrics);
                    }
                    if ui.button("カタカナ歌詞を開く").clicked() {
                        command = Some(AppCommand::OpenReadingLyrics);
                    }
                    ui.separator();
                    ui.menu_button("音声ファイルへタグを書き込む", |ui| {
                        let detected = detected_audio_tag_format(&self.project.audio_path);
                        // 可逆音源（WAV/FLAC）は好きな形式へ自由に変換できる
                        // が、非可逆音源（MP3、M4A）は一致する項目しか
                        // 選べないようにし、非可逆圧縮を重ねたり、劣化済み
                        // 音声を無意味に包んだりしないようにする。
                        let lossless_source = self
                            .project
                            .audio_path
                            .extension()
                            .and_then(|value| value.to_str())
                            .map(|value| value.to_ascii_lowercase())
                            .is_some_and(|extension| is_lossless_audio_extension(&extension));
                        for (label, format) in [
                            ("MP3 + LRC", AudioTagFormat::Mp3WithLrc),
                            ("FLAC", AudioTagFormat::Flac),
                            ("M4A", AudioTagFormat::M4a),
                        ] {
                            let enabled = detected == Some(format) || lossless_source;
                            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                                command = Some(AppCommand::WriteAudioTags(format));
                            }
                        }
                    });
                });
                ui.menu_button("編集", |ui| {
                    if ui.button("歌唱向けカタカナ自動生成").clicked() {
                        self.generate_readings();
                    }
                    ui.label("Ctrl+A はテキスト全選択");
                });
                ui.menu_button("再生", |ui| {
                    if ui.button("再生 / 一時停止    Space").clicked() {
                        toggle_playback = true;
                    }
                    if ui.button("先頭へ移動").clicked() {
                        if let Some(player) = &mut self.player {
                            let _ = player.seek(0);
                        }
                    }
                });
                ui.menu_button("設定", |ui| {
                    if ui.button("外部ツール設定…").clicked() {
                        command = Some(AppCommand::ShowExternalTools);
                    }
                });
                ui.separator();
                ui.label(
                    self.project_path
                        .as_ref()
                        .and_then(|path| path.file_name())
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_else(|| "未保存プロジェクト".into()),
                );
                if self.dirty {
                    ui.colored_label(egui::Color32::YELLOW, "● 未保存");
                }
            });
        });
        if !ctx.wants_keyboard_input()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            toggle_playback = true;
        }
        if toggle_playback {
            if let Some(player) = &mut self.player {
                if player.is_playing() {
                    player.pause();
                } else if let Err(error) = player.play() {
                    self.status = error;
                }
            }
        }
        if let Some(command) = command {
            self.execute_command(command);
        }

        let mut external_tools_open = self.show_external_tools;
        egui::Window::new("外部ツール設定")
            .open(&mut external_tools_open)
            .resizable(true)
            .default_width(620.0)
            .show(ctx, |ui| self.show_external_tools_ui(ui));
        self.show_external_tools = external_tools_open;

        egui::TopBottomPanel::bottom("status-bar")
            .exact_height(28.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(started_at) = self.job_started_at.filter(|_| self.job.is_some()) {
                        let elapsed = started_at.elapsed().as_secs();
                        ui.label(format!(
                            "{}（経過 {:02}:{:02}）",
                            self.status,
                            elapsed / 60,
                            elapsed % 60
                        ));
                    } else {
                        ui.label(&self.status);
                    }
                });
            });

        // --- 下部タイムラインパネル ---
        let timeline_position = self
            .player
            .as_ref()
            .map(AudioPlayer::position_ms)
            .unwrap_or(0);
        let timeline_duration = self.player.as_ref().and_then(AudioPlayer::duration_ms);
        let mut timeline_actions = Vec::new();
        egui::TopBottomPanel::bottom("lyrics-timeline")
            .default_height(150.0)
            .min_height(112.0)
            .max_height(340.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("タイムライン");
                    ui.label(format!("ズーム {:.0}px/秒", self.timeline_zoom));
                    if ui.small_button("－").clicked() {
                        self.timeline_zoom = (self.timeline_zoom / 1.25).max(timeline::MIN_ZOOM);
                    }
                    if ui.small_button("＋").clicked() {
                        self.timeline_zoom = (self.timeline_zoom * 1.25).min(timeline::MAX_ZOOM);
                    }
                    ui.separator();
                    let current_position = self.player.as_ref().map(AudioPlayer::position_ms);
                    if ui
                        .add_enabled(
                            self.selected.is_some() && current_position.is_some(),
                            egui::Button::new("現在の再生位置を開始時刻に設定"),
                        )
                        .clicked()
                    {
                        if let (Some(index), Some(position)) = (self.selected, current_position) {
                            lyric_editor::set_line_start(&mut self.project.lyrics, index, position);
                            self.changed();
                            self.status = "現在の再生位置を開始時刻に設定しました".into();
                        }
                    }
                    ui.label(
                        "Ctrl+ホイールで拡大縮小 / ドラッグで移動 / 端をドラッグして開始・終了を調整 / 右クリックで追従移動の設定",
                    );
                });
                timeline_actions = timeline::show(
                    ui,
                    &self.project.lyrics,
                    timeline::PlaybackState {
                        position_ms: timeline_position,
                        duration_ms: timeline_duration,
                        is_playing: self.player.as_ref().is_some_and(AudioPlayer::is_playing),
                    },
                    self.selected,
                    &mut self.timeline_zoom,
                    timeline::InteractionState {
                        drag_anchor: self.timeline_drag,
                        resize_anchor: self.timeline_resize,
                        linked_tail_movement: self.timeline_link_tail_movement,
                    },
                );
            });
        self.handle_timeline_actions(timeline_actions, timeline_duration);

        // --- 左ペイン：音声選択、原文・カタカナ入力 ---
        let mut original_input_changed = false;
        let mut reading_input_changed = false;
        egui::SidePanel::left("lyrics-input-panel")
            .default_width(300.0)
            .min_width(220.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("入力");
                if ui.button("🎵 音声を開く").clicked() {
                    self.open_audio_dialog();
                }
                if self.project.audio_path.as_os_str().is_empty() {
                    ui.small("音声ファイルが選択されていません");
                } else {
                    ui.small(self.project.audio_path.to_string_lossy());
                }
                ui.separator();
                ui.strong("原文歌詞");
                ui.small("Forced Alignmentで使用する正解歌詞を入力します");
                // 入力欄の高さを固定し、長い歌詞は内部でスクロールさせる。
                // TextEditが伸び続けてカタカナ欄（とその下）を画面外へ
                // 押し出してしまわないようにするため。
                let original_area_height = ui.available_height() * 0.55;
                original_input_changed = egui::ScrollArea::vertical()
                    .id_salt("original-lyrics-scroll")
                    .max_height(original_area_height)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.original_lyrics_input)
                                .desired_width(f32::INFINITY)
                                .desired_rows(14)
                                .hint_text("歌詞を1行ずつ入力してください"),
                        )
                    })
                    .inner
                    .changed();
                ui.separator();
                ui.strong("カタカナ歌詞（任意）");
                ui.small("同期判定には使用されません");
                reading_input_changed = egui::ScrollArea::vertical()
                    .id_salt("reading-lyrics-scroll")
                    .max_height(ui.available_height())
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.reading_lyrics_input)
                                .desired_width(f32::INFINITY)
                                .desired_rows(10)
                                .hint_text("原文と同じ行構成で入力してください"),
                        )
                    })
                    .inner
                    .changed();
            });
        if original_input_changed {
            self.apply_original_lyrics_input();
        }
        if reading_input_changed {
            self.apply_reading_lyrics_input();
        }

        let active_index = self
            .player
            .as_ref()
            .and_then(|player| active_lyric_index(&self.project.lyrics, player.position_ms()));
        let right_scroll = self.auto_scroll && active_index != self.last_highlighted;
        // --- 右ペイン：アートワーク・曲情報・歌詞位置リスト ---
        egui::SidePanel::right("player-panel")
            .default_width(360.0)
            .min_width(260.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Player");
                if let Some(texture) = &self.artwork_texture {
                    let width = ui.available_width().min(180.0);
                    ui.vertical_centered(|ui| {
                        ui.image((texture.id(), egui::vec2(width, width)));
                    });
                }
                ui.strong(if self.project.title.is_empty() {
                    "タイトル未設定"
                } else {
                    &self.project.title
                });
                ui.label(&self.project.artist);
                ui.checkbox(&mut self.auto_scroll, "歌詞を自動スクロール");
                ui.separator();
                ui.heading("Lyrics");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (index, line) in self.project.lyrics.iter().enumerate() {
                        let highlighted = active_index == Some(index);
                        let text = match self.lyrics_display_mode {
                            LyricsDisplayMode::Original => line.original_text.clone(),
                            LyricsDisplayMode::Reading => line
                                .reading_text
                                .clone()
                                .unwrap_or_else(|| "（未生成）".into()),
                            LyricsDisplayMode::OriginalAndReading => format!(
                                "{}\n{}",
                                line.original_text,
                                line.reading_text.as_deref().unwrap_or("（未生成）")
                            ),
                        };
                        let response = ui.add(
                            egui::Label::new(if highlighted {
                                egui::RichText::new(text)
                                    .size(17.0)
                                    .color(egui::Color32::WHITE)
                                    .background_color(egui::Color32::from_rgb(42, 92, 138))
                                    .strong()
                            } else {
                                egui::RichText::new(text)
                            })
                            .sense(egui::Sense::click()),
                        );
                        if highlighted && right_scroll {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
                            self.selected = Some(index);
                            if let (Some(start), Some(player)) = (line.start_ms, &mut self.player) {
                                let _ = player.seek(start).and_then(|()| player.play());
                            }
                        }
                        ui.add_space(4.0);
                    }
                });
            });
        self.last_highlighted = active_index;
        // --- 中央パネル：曲情報、歌詞表示切替、時刻補正、自動同期、
        //     下部固定の再生バー ---
        egui::CentralPanel::default().show(ctx, |ui| {
            // 再生バーの領域を先に中央エリア下部へ確保しておくことで、
            // 上のスクロール可能な部分がどれだけ縦に伸びても
            // （長い歌詞行など）、再生バーの位置は常に固定される。
            egui::TopBottomPanel::bottom("central-playback-bar")
                .show_separator_line(false)
                .show_inside(ui, |ui| {
                    ui.separator();
                    if let Some(player) = &mut self.player {
                        ui.horizontal(|ui| {
                            if ui
                                .button(if player.is_playing() {
                                    "一時停止"
                                } else {
                                    "▶ 再生"
                                })
                                .clicked()
                            {
                                if player.is_playing() {
                                    player.pause();
                                } else if let Err(error) = player.play() {
                                    self.status = error;
                                }
                            }
                            let mut position = player.position_ms();
                            ui.label(format_time(position));
                            if let Some(duration) = player.duration_ms() {
                                ui.spacing_mut().slider_width =
                                    (ui.available_width() - 60.0).max(80.0);
                                if ui
                                    .add(
                                        egui::Slider::new(&mut position, 0..=duration)
                                            .show_value(false),
                                    )
                                    .changed()
                                {
                                    let _ = player.seek(position);
                                }
                                ui.label(format_time(duration));
                            }
                        });
                    } else {
                        ui.label("音声を読み込んでください");
                    }
                });
            egui::ScrollArea::vertical()
                .id_salt("central-content-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.heading(if self.dirty {
                        "Benzaiten *"
                    } else {
                        "Benzaiten"
                    });
                    let mut metadata_changed = false;
                    egui::CollapsingHeader::new("音楽情報・アルバムアート")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    if let Some(texture) = &self.artwork_texture {
                                        ui.image((texture.id(), egui::vec2(128.0, 128.0)));
                                    } else {
                                        ui.allocate_ui(egui::vec2(128.0, 128.0), |ui| {
                                            ui.centered_and_justified(|ui| ui.label("No Artwork"));
                                        });
                                    }
                                    if ui.button("画像を選択").clicked() {
                                        if let Some(path) = rfd::FileDialog::new()
                                            .add_filter(
                                                "Image",
                                                &["jpg", "jpeg", "png", "gif", "webp"],
                                            )
                                            .pick_file()
                                        {
                                            match std::fs::read(&path) {
                                                Ok(bytes) => {
                                                    self.project.metadata.artwork_path = Some(path);
                                                    self.artwork_bytes = Some(bytes);
                                                    self.artwork_texture = None;
                                                    metadata_changed = true;
                                                }
                                                Err(error) => {
                                                    self.status = format!("画像読込エラー: {error}")
                                                }
                                            }
                                        }
                                    }
                                });
                                ui.vertical(|ui| {
                                    for (label, value) in [
                                        ("曲名", &mut self.project.title),
                                        ("アーティスト", &mut self.project.artist),
                                        ("アルバム", &mut self.project.metadata.album),
                                        (
                                            "アルバムアーティスト",
                                            &mut self.project.metadata.album_artist,
                                        ),
                                        ("ジャンル", &mut self.project.metadata.genre),
                                    ] {
                                        ui.horizontal(|ui| {
                                            ui.label(label);
                                            metadata_changed |=
                                                ui.text_edit_singleline(value).changed();
                                        });
                                    }
                                    ui.horizontal(|ui| {
                                        let mut year = self.project.metadata.year.unwrap_or(0);
                                        let mut track =
                                            self.project.metadata.track_number.unwrap_or(0);
                                        let mut disc =
                                            self.project.metadata.disc_number.unwrap_or(0);
                                        ui.label("年");
                                        if ui
                                            .add(egui::DragValue::new(&mut year).range(0..=9999))
                                            .changed()
                                        {
                                            self.project.metadata.year =
                                                (year != 0).then_some(year);
                                            metadata_changed = true;
                                        }
                                        ui.label("トラック");
                                        if ui
                                            .add(egui::DragValue::new(&mut track).range(0..=999))
                                            .changed()
                                        {
                                            self.project.metadata.track_number =
                                                (track != 0).then_some(track);
                                            metadata_changed = true;
                                        }
                                        ui.label("ディスク");
                                        if ui
                                            .add(egui::DragValue::new(&mut disc).range(0..=99))
                                            .changed()
                                        {
                                            self.project.metadata.disc_number =
                                                (disc != 0).then_some(disc);
                                            metadata_changed = true;
                                        }
                                    });
                                });
                            });
                        });
                    if metadata_changed {
                        self.changed();
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("歌詞表示");
                        ui.selectable_value(
                            &mut self.lyrics_display_mode,
                            LyricsDisplayMode::Original,
                            "原文のみ",
                        );
                        ui.selectable_value(
                            &mut self.lyrics_display_mode,
                            LyricsDisplayMode::Reading,
                            "カタカナのみ",
                        );
                        ui.selectable_value(
                            &mut self.lyrics_display_mode,
                            LyricsDisplayMode::OriginalAndReading,
                            "原文 + カタカナ",
                        );
                        if ui
                            .button("歌唱向けカタカナ自動生成")
                            .on_hover_text("手動入力・カタカナTXTから読み込んだ行は上書きしません")
                            .clicked()
                        {
                            self.generate_readings();
                        }
                    });
                    let active_index = self.player.as_ref().and_then(|player| {
                        active_lyric_index(&self.project.lyrics, player.position_ms())
                    });
                    if self.player.as_ref().is_some_and(AudioPlayer::is_playing) {
                        self.selected = active_index.or(self.selected);
                    }
                    let position = self.player.as_ref().map(|p| p.position_ms());
                    let duration = self.player.as_ref().and_then(|p| p.duration_ms());
                    let mut edited = false;
                    let selected = self
                        .selected
                        .or(active_index)
                        .filter(|index| *index < self.project.lyrics.len());
                    let mut set_to = None;
                    let mut shift_by = None;
                    let mut clear_time = false;
                    let mut seek_selected = false;
                    let mut navigate_to = None;
                    let mut reading_edit = None;
                    // 「現在の歌詞・時刻補正」パネル。選択中の行のカタカナ・
                    // 開始時刻・調整幅などをまとめて表示・編集する。各操作は
                    // いったんローカル変数（`set_to`/`shift_by`など）へ集約し、
                    // パネルを閉じた後にまとめて`project.lyrics`へ反映する。
                    egui::Frame::group(ui.style())
                        .fill(egui::Color32::from_rgb(29, 34, 42))
                        .inner_margin(16.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.heading("現在の歌詞・時刻補正");
                                if let Some(index) = selected {
                                    ui.label(format!(
                                        "行 {} / {}",
                                        index + 1,
                                        self.project.lyrics.len()
                                    ));
                                    if ui
                                        .add_enabled(index > 0, egui::Button::new("◀ 前の行"))
                                        .clicked()
                                    {
                                        navigate_to = Some(index - 1);
                                    }
                                    if ui
                                        .add_enabled(
                                            index + 1 < self.project.lyrics.len(),
                                            egui::Button::new("次の行 ▶"),
                                        )
                                        .clicked()
                                    {
                                        navigate_to = Some(index + 1);
                                    }
                                }
                            });
                            if let Some(index) = selected {
                                let line = &self.project.lyrics[index];
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(&line.original_text)
                                        .size(24.0)
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                );
                                let mut reading = line.reading_text.clone().unwrap_or_default();
                                ui.horizontal(|ui| {
                                    ui.label("カタカナ");
                                    if ui
                                        .add(
                                            egui::TextEdit::singleline(&mut reading)
                                                .desired_width(ui.available_width()),
                                        )
                                        .changed()
                                    {
                                        reading_edit =
                                            Some((!reading.is_empty()).then_some(reading));
                                    }
                                });
                                ui.separator();
                                ui.horizontal_wrapped(|ui| {
                                    let mut start =
                                        line.start_ms.unwrap_or_else(|| position.unwrap_or(0));
                                    ui.label("開始時刻");
                                    if ui
                                        .add(
                                            egui::DragValue::new(&mut start)
                                                .range(0..=duration.unwrap_or(u64::MAX))
                                                .speed(10.0)
                                                .suffix(" ms"),
                                        )
                                        .changed()
                                    {
                                        set_to = Some(start);
                                    }
                                    ui.strong(format_time(start));
                                    ui.separator();
                                    ui.label("調整幅");
                                    ui.add(
                                        egui::DragValue::new(&mut self.bulk_shift_ms)
                                            .range(1..=60_000)
                                            .speed(10.0)
                                            .suffix(" ms"),
                                    );
                                    if ui
                                        .add_enabled(
                                            line.start_ms.is_some(),
                                            egui::Button::new(format!(
                                                "−{} ms",
                                                self.bulk_shift_ms
                                            )),
                                        )
                                        .clicked()
                                    {
                                        shift_by = Some(-self.bulk_shift_ms);
                                    }
                                    if ui
                                        .add_enabled(
                                            line.start_ms.is_some(),
                                            egui::Button::new(format!(
                                                "＋{} ms",
                                                self.bulk_shift_ms
                                            )),
                                        )
                                        .clicked()
                                    {
                                        shift_by = Some(self.bulk_shift_ms);
                                    }
                                });
                                ui.horizontal_wrapped(|ui| {
                                    if ui.button("この行から再生").clicked() {
                                        seek_selected = true;
                                    }
                                    if ui.button("時刻を解除").clicked() {
                                        clear_time = true;
                                    }
                                });
                                let score = line
                                    .confidence
                                    .map(|value| format!("{value:.2}"))
                                    .unwrap_or_else(|| "—".into());
                                ui.small(format!(
                                    "照合指標: {score} / 時刻の由来: {:?}",
                                    line.timestamp_source
                                ));
                                ui.small(
                                    "ショートカット: Ctrl+←/→ = 10 ms、Ctrl+Shift+←/→ = 100 ms",
                                );
                            } else {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(24.0);
                                    ui.label("タイムラインの歌詞ブロックを選択してください");
                                    ui.add_space(24.0);
                                });
                            }
                        });

                    if let Some(index) = navigate_to {
                        self.selected = Some(index);
                        if let (Some(start), Some(player)) =
                            (self.project.lyrics[index].start_ms, &mut self.player)
                        {
                            let _ = player.seek(start);
                        }
                    }

                    // Ctrl+←/→ = 10ms、Ctrl+Shift+←/→ = 100msで選択行を
                    // 微調整する。テキスト入力中（`wants_keyboard_input`）は
                    // 矢印キーが文字移動に使われるべきなので無効化する。
                    if selected.is_some() && !ctx.wants_keyboard_input() {
                        let keyboard_delta = ctx.input(|input| {
                            if !input.modifiers.ctrl {
                                None
                            } else if input.key_pressed(egui::Key::ArrowLeft) {
                                Some(if input.modifiers.shift { -100 } else { -10 })
                            } else if input.key_pressed(egui::Key::ArrowRight) {
                                Some(if input.modifiers.shift { 100 } else { 10 })
                            } else {
                                None
                            }
                        });
                        shift_by = shift_by.or(keyboard_delta);
                    }
                    if let Some(index) = selected {
                        if let Some(reading) = reading_edit {
                            let line = &mut self.project.lyrics[index];
                            line.reading_text = reading;
                            line.reading_source =
                                line.reading_text.as_ref().map(|_| ReadingSource::Manual);
                            edited = true;
                        }
                        if let Some(ms) = set_to {
                            lyric_editor::set_line_start(&mut self.project.lyrics, index, ms);
                            edited = true;
                        }
                        if let Some(delta) = shift_by {
                            lyric_editor::shift_line_start(
                                &mut self.project.lyrics,
                                index,
                                delta,
                                duration,
                            );
                            edited = true;
                        }
                        if clear_time {
                            let line = &mut self.project.lyrics[index];
                            line.start_ms = None;
                            line.end_ms = None;
                            line.confidence = None;
                            line.timestamp_source = None;
                            edited = true;
                        }
                        if seek_selected {
                            if let (Some(ms), Some(player)) =
                                (self.project.lyrics[index].start_ms, &mut self.player)
                            {
                                if let Err(error) = player.seek(ms) {
                                    self.status = error;
                                }
                            }
                        }
                    }
                    if edited {
                        self.changed();
                        self.sync_lyric_inputs_from_project();
                    }
                    // 「自動同期」ボタン：歌詞言語を解決し、対応するモデル・
                    // 語彙の設定を検証した上で、Forced Alignmentパイプライン
                    // を非同期ジョブとして開始する。開始時点の`generation`を
                    // `job_generation`に記録しておき、完了時に入力が変わって
                    // いないか確認できるようにする（`poll_job`参照）。
                    ui.horizontal(|ui| {
                        if self.job.is_some() {
                            if ui.button("同期をキャンセル").clicked() {
                                if let Some(job) = &self.job {
                                    job.cancel();
                                    self.generation = self.generation.wrapping_add(1);
                                }
                            }
                        } else if ui
                            .add_enabled(
                                !self.project.audio_path.as_os_str().is_empty()
                                    && !self.project.lyrics.is_empty(),
                                egui::Button::new("自動同期"),
                            )
                            .clicked()
                        {
                            let language = crate::forced_alignment::tokenizer::resolve_language(
                                &self.language,
                                &self.project.lyrics,
                            );
                            let Ok(language) = language else {
                                self.status = language.unwrap_err();
                                return;
                            };
                            let japanese = language
                                == crate::forced_alignment::tokenizer::AlignmentLanguage::Japanese;
                            let settings = ToolSettings {
                                ffmpeg: self.ffmpeg.clone().into(),
                                model: if japanese {
                                    self.japanese_model.clone().into()
                                } else {
                                    self.model.clone().into()
                                },
                                vocabulary: japanese
                                    .then(|| self.japanese_vocabulary.clone().into()),
                                language: language.code().into(),
                                threads: self.alignment_threads,
                            };
                            match settings.validate_model(&self.project.lyrics) {
                                Err(error) => self.status = error,
                                Ok(()) => {
                                    self.job_generation = self.generation;
                                    self.job = Some(Job::start(
                                        self.project.audio_path.clone(),
                                        self.project.lyrics.clone(),
                                        settings,
                                    ));
                                    self.job_started_at = Some(Instant::now());
                                }
                            }
                        }
                    });
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{active_lyric_index, merge_original_lyrics};
    use crate::domain::{
        lyrics::parse_lyrics,
        project::{ReadingSource, TimestampSource},
    };

    #[test]
    fn highlight_uses_end_or_next_line_start() {
        let mut lyrics = parse_lyrics("one\ntwo\nthree");
        lyrics[0].start_ms = Some(1_000);
        lyrics[0].end_ms = Some(2_000);
        lyrics[1].start_ms = Some(3_000);
        lyrics[2].start_ms = Some(5_000);

        assert_eq!(active_lyric_index(&lyrics, 999), None);
        assert_eq!(active_lyric_index(&lyrics, 1_500), Some(0));
        assert_eq!(active_lyric_index(&lyrics, 2_500), None);
        assert_eq!(active_lyric_index(&lyrics, 4_000), Some(1));
        assert_eq!(active_lyric_index(&lyrics, 6_000), Some(2));
    }

    #[test]
    fn lyric_text_edit_preserves_matching_line_data_after_insertion() {
        let mut lyrics = parse_lyrics("one\ntwo");
        lyrics[0].start_ms = Some(1_000);
        lyrics[0].reading_text = Some("ワン".into());
        lyrics[0].reading_source = Some(ReadingSource::Manual);
        lyrics[1].start_ms = Some(2_000);
        lyrics[1].timestamp_source = Some(TimestampSource::ForcedAlignment);

        let merged = merge_original_lyrics(lyrics, "intro\none\ntwo");

        assert_eq!(merged[0].start_ms, None);
        assert_eq!(merged[1].start_ms, Some(1_000));
        assert_eq!(merged[1].reading_text.as_deref(), Some("ワン"));
        assert_eq!(merged[2].start_ms, Some(2_000));
        assert_eq!(
            merged[2].timestamp_source,
            Some(TimestampSource::ForcedAlignment)
        );

        let changed = merge_original_lyrics(merged, "intro\none\nchanged");
        assert_eq!(changed[1].start_ms, Some(1_000));
        assert_eq!(changed[2].start_ms, None);
    }
}
