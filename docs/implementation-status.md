# 実装状況

## 現在の状態（要約）

- GUI・CLIとも実装済み。英語・日本語Wav2Vec2によるForced Alignment、LRC出力、プロジェクトJSON保存、音声タグ読み書き（FLAC/M4Aへの歌詞埋め込み、MP3は`.lrc`併用）に対応。
- 外部ffmpegバイナリへの依存は排除済み（前処理・音声変換は純Rust実装＋Windows Media Foundation）。
- Windows向け配布zipを自動生成するGitHub Actionsワークフローを追加済みだが、**リモートでの実行は未検証**（下記「検証環境」参照）。
- テスト36件・`cargo clippy`・`cargo fmt`はすべてWindows向けローカル検証でパス済み。

タスクごとの詳細な変更履歴は下の「詳細な実装履歴」を参照してください。

## 検証環境

Windows のローカル検証は Rust GNU クロスビルドで実施しました。GNUビルドではONNX Runtime 1.25.1をDLLとして動的ロードします。通常の開発環境には Rust の MSVC ツールチェーンと Visual Studio Build Tools（Windows SDK を含む）を推奨します。

- Windows向け `cargo test`: 36件成功。
- Windows向け `cargo clippy --all-targets -- -D warnings`: 成功。
- `cargo fmt --all -- --check`: 成功。
- GUI実行ファイルを起動し、ウィンドウ生成と起動時エラーがないことを確認。
- 日本語ONNXモデルと2341語彙tokenizerを実ロードし、Windows上で推論・Forced Alignment・LRC出力を通し確認。
- Windows実機（クロスビルド、WSL2 Interop経由で実プロセスとして実行）で、ffmpeg抜きの新前処理パイプライン（Symphonia＋rubato）を含めONNX Runtime、Wav2Vec2、CTC、LRCまで通し検証済み。FLAC/MP3/M4A再エンコード（`flacenc`/LAME/Media Foundation）も実音声で往復デコード確認済み。M4Aは`ffprobe`による外部検証、および変換後のタグ・歌詞埋め込みまでの一気通貫も確認済み。
- Windows向けCI設定（`ci.yml`）、配布zip生成（`release.yml`）とも追加済みだが、リモート（GitHub Actions上）ではまだ一度も実行していない。

`.tmp` はサンプル・作業用でGit管理対象外です。他のPCでは通常のRust開発環境を用意してください。

## 担当

- GPT-6 Astra / high: Alignmentの実装、保存・LRC・GUI・再生処理の独立レビュー。
- GPT-5.6 Terra / medium: domain、JSON保存、LRC出力、ドキュメント。
- GPT-5.6 Terra / high: ffmpeg、非同期ジョブ、プロセス管理。
- 親エージェント: Cargo構成、GUI・音声再生・CLI統合、検証とレビュー修正。

提供音源で処理速度と出力範囲を確認済みですが、歌唱・伴奏条件の異なる複数曲での同期誤差評価は今後の課題です。

## 詳細な実装履歴

<details>
<summary>T01〜T25の変更内容（クリックで展開）</summary>

| タスク | 状況 | 内容 |
| --- | --- | --- |
| T01 | 設計済み | 入出力と検証方針を `.tmp/v0.1-implementation-plan.md` に整理済み。 |
| T02 | 実装済み | domain DTO、モジュール構成、エラー文字列を用意。 |
| T03 | 実装済み | UTF-8 歌詞解析、Project JSON の保存・読込、相対音声パス解決。 |
| T04 | 実装済み | ffmpeg による 16 kHz mono PCM16 WAV 前処理（T23で純Rust実装へ置換）。 |
| T05 | 削除済み | whisper.cpp転写経路。Forced Alignment移行後、未使用コードと配布物を削除。 |
| T06 | 実装済み | 英語正解歌詞のCTCトークン化。 |
| T07 | 実装済み | ONNX Wav2Vec2推論とCTC Viterbi Forced Alignment。 |
| T08 | 実装済み | 行時刻、confidence、Forced Alignment・手動補正の区別。 |
| T09 | 実装済み | Standard LRC 出力と入力検証。 |
| T10 | 実装済み | 非同期ジョブ、進捗、キャンセル、CLI パイプライン。 |
| T11 | 実装済み | egui 画面、rodio 再生、seek、行選択、手動時刻補正、保存。 |
| T12 | 実施済み | 提供音源3分36秒をCPUデバッグビルドで約14.6秒で処理し、LRC/JSONを生成。 |
| T13 | 実装済み | schema v2、原文・カタカナ分離、v1自動移行、表示切替と手動編集。 |
| T14 | 実装済み | CMUdict音素、弱形・連結規則、辞書外語通知による歌唱向けカタカナ自動生成。 |
| T15 | 実装済み | 単曲プレイヤーの現在行ハイライト、自動スクロール、クリックシーク。 |
| T16 | 実装済み | 音楽情報・アルバムアートの読込/編集/JSON保存、確認付き音声タグ書込みと初回バックアップ。 |
| T17 | 実装済み | Zed風3ペイン、プロジェクトExplorer、内部/OSドラッグ＆ドロップ、右側プレイヤー。 |
| T18 | 実装済み | 選択行の直接時刻入力、10/100/500ms微調整、再生位置採用、後続行一括シフト。 |
| T19 | 実装済み | Auto/en/jp言語切替、日本語文字検出、外部tokenizer語彙、日本語Wav2Vec2 Forced Alignment。 |
| T20 | 実装済み | 常設メニューバーと保存ショートカット、原文・カタカナ2トラックのドラッグ編集タイムライン、Ctrl+ホイール拡大縮小。 |
| T21 | 実装済み | 左ペインを音声・原文・カタカナ入力へ変更し、中央一覧を現在行に集中した可変ms時刻補正UIへ変更。 |
| T22 | 実装済み | タイムラインの左右端ドラッグによる開始・終了個別調整、再生位置追従、右クリックでの追従移動トグルなど編集UXの改善。歌詞テキストの空行破棄。音声タグ書込みをメニューの`MP3 + LRC`/`FLAC`/`M4A`サブメニューへ再編し、FLAC/M4Aへの同期歌詞埋め込み（`ItemKey::Lyrics`）とMP3の`.lrc`併用出力を追加。可逆音源（WAV/FLAC）読み込み時は他形式へ変換してから書き込む機能も追加（非可逆音源からの変換は二重圧縮を避けるため提供しない）。 |
| T23 | 実装済み | 外部ffmpegバイナリへの依存を排除。前処理を`Symphonia`（デコード）＋`rubato`（16kHzリサンプリング）による純Rust実装へ置換し、`ToolSettings`/CLI引数からffmpegパスを削除。可逆音源からのタグ埋め込み用変換もFLACは`flacenc`、MP3はLAME（`mp3lame-encoder`、LGPL）による純Rust/軽量バインディング実装へ置換。汎用サブプロセス実行モジュール（`src/process.rs`）は用途が無くなったため削除。 |
| T24 | 実装済み | M4A（AAC）への変換を実装（`src/audio/aac_windows.rs`）。Windows Media Foundation標準搭載のAACエンコーダを`windows`クレート・`IMFSinkWriter`経由で直接呼び出すため、追加バイナリの配布やライセンス条件は発生しない。「音声ファイルへタグを書き込む」メニューのM4A項目を可逆音源からの変換にも対応させ、`AudioTagFormat::supports_lossless_conversion`による無効化を撤廃。実音声での変換→デコード往復、および変換後にタグ・歌詞埋め込みまで通しての実機検証を実施。 |
| T25 | 実装済み・リモート未検証 | Windows向け配布zipを`.github/workflows/release.yml`で自動生成。タグpush（`v*`）または手動実行で`cargo build --release`後、`runtime\download-onnx-runtime.cmd`でONNX Runtime DLLを取得し、exe・DLL・モデル取得用`.cmd`・`README.md`・`LICENSE`・`THIRD_PARTY_NOTICES.txt`・CMUdictライセンスをまとめてzip化し、GitHub Releasesへ添付する。Wav2Vec2 ONNXモデルはサイズとライセンス（日本語モデルは配布元に明記なし）の都合で同梱しない。ステージング処理（ファイル配置とzip化）はWSL側のGNUクロスビルド成果物で内容を疑似検証済みだが、ワークフロー自体はGitHub Actions上でまだ一度も実行していない（`リモートCIは未実行`の状態が継続）ため、実際のタグpush前に`workflow_dispatch`での試験実行を推奨。 |

</details>
