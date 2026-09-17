# 実装状況

| タスク | 状況 | 内容 |
| --- | --- | --- |
| T01 | 設計済み | 入出力と検証方針を `.tmp/v0.1-implementation-plan.md` に整理済み。 |
| T02 | 実装済み | domain DTO、モジュール構成、エラー文字列を用意。 |
| T03 | 実装済み | UTF-8 歌詞解析、Project JSON の保存・読込、相対音声パス解決。 |
| T04 | 実装済み | ffmpeg による 16 kHz mono PCM16 WAV 前処理。 |
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
| T22 | 実装済み | タイムラインの左右端ドラッグによる開始・終了個別調整、再生位置追従、右クリックでの追従移動トグルなど編集UXの改善。歌詞テキストの空行破棄。音声タグ書込みをメニューの`MP3 + LRC`/`FLAC`/`M4A`サブメニューへ再編し、FLAC/M4Aへの同期歌詞埋め込み（`ItemKey::Lyrics`）とMP3の`.lrc`併用出力を追加。可逆音源（WAV/FLAC）読み込み時はffmpeg経由で他形式へ変換してから書き込む機能も追加（非可逆音源からの変換は二重圧縮を避けるため提供しない）。 |

## 検証環境

Windows のローカル検証は Rust GNU クロスビルドで実施しました。GNUビルドではONNX Runtime 1.25.1をDLLとして動的ロードします。通常の開発環境には Rust の MSVC ツールチェーンと Visual Studio Build Tools（Windows SDK を含む）を推奨します。

- Windows向け `cargo test`: 36件成功。
- Windows向け `cargo clippy --all-targets -- -D warnings`: 成功。
- `cargo fmt --all -- --check`: 成功。
- GUI実行ファイルを起動し、ウィンドウ生成と起動時エラーがないことを確認。
- 日本語ONNXモデルと2341語彙tokenizerを実ロードし、Windows上で推論・Forced Alignment・LRC出力を通し確認。
- Linux/WSLで実ffmpeg、ONNX Runtime、Wav2Vec2、CTC、LRCまで通し検証済み。
- Windows向けCI設定を追加。リモートCIは未実行。

`.tmp` はサンプル・作業用でGit管理対象外です。他のPCでは通常のRust開発環境を用意してください。

## 担当

- GPT-6 Astra / high: Alignmentの実装、保存・LRC・GUI・再生処理の独立レビュー。
- GPT-5.6 Terra / medium: domain、JSON保存、LRC出力、ドキュメント。
- GPT-5.6 Terra / high: ffmpeg、非同期ジョブ、プロセス管理。
- 親エージェント: Cargo構成、GUI・音声再生・CLI統合、検証とレビュー修正。

提供音源で処理速度と出力範囲を確認済みですが、歌唱・伴奏条件の異なる複数曲での同期誤差評価は今後の課題です。
