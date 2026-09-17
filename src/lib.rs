//! Benzaitenのライブラリクレート。
//!
//! GUI（`src/main.rs`）とCLI（`src/bin/benzaiten-cli.rs`）の両方が共有する
//! 処理モジュール（音声前処理、Forced Alignment、歌詞・プロジェクトデータ、
//! LRC出力、音声タグの読み書き）をまとめている。全体のパイプラインは
//! `docs/design.md`を参照。

/// eframe/eguiによるデスクトップアプリ本体（ウィンドウ、各パネル、操作処理）。
pub mod app;
/// 音声再生（rodio）と、ffmpegによる16kHzモノラルWAVへの前処理。
pub mod audio;
/// GUIや音響モデルに依存しない、歌詞・プロジェクトのデータ型。
pub mod domain;
/// Wav2Vec2 ONNX推論、CTCトークン化、Viterbiアライメント。
pub mod forced_alignment;
/// アライメントパイプライン全体をキャンセル可能に実行する非同期ジョブ。
pub mod jobs;
/// 標準LRCの生成とファイル出力。
pub mod lrc;
/// loftyを使った音楽タグの読み書き（FLAC/M4Aでは歌詞の埋め込みも行う）。
pub mod metadata;
/// 外部コマンドラインツールをキャンセル可能に実行する小さなラッパー。
pub mod process;
/// プロジェクトJSONの読み書き、スキーマ移行、相対パス解決。
pub mod project;
/// 英語歌詞から歌唱向けカタカナ発音ガイドを生成する処理。
pub mod pronunciation;
/// GUIが使う再利用可能なeguiウィジェットと、純粋な編集ヘルパー関数。
pub mod ui;
