# Benzaiten

ローカル音声と正解歌詞をCTC Forced Alignmentで直接照合し、行ごとの開始時刻を持つLRCを作るRustデスクトップアプリです。MP3/WAV/FLAC/OGG/M4Aを純Rust実装（Symphonia＋rubato）で16 kHz mono PCM WAVに変換し、Wav2Vec2 ONNXモデルの音響フレームへ正解歌詞を割り当てます。外部のffmpegバイナリには依存しません。

詳しい設計は[docs/design.md](docs/design.md)、実装状況は[docs/implementation-status.md](docs/implementation-status.md)を参照してください。

## クイックスタート（配布zip）

1. [GitHub Releases](../../releases)から`benzaiten-<version>-windows-x86_64.zip`をダウンロードし展開する。
2. `models\download-forced-alignment-model.cmd`（英語）または`models\download-japanese-alignment-model.cmd`（日本語）を実行してForced Alignmentモデルを取得する。
3. `benzaiten.exe`を起動し、「外部ツール設定」からダウンロードしたモデルファイルを選択する。

zipにはexe・ONNX Runtime DLL・ライセンス表記一式（`licenses/`、`THIRD_PARTY_NOTICES.txt`）を含みますが、サイズとライセンスの都合上Wav2Vec2 ONNXモデルは含みません。zipは`.github/workflows/release.yml`によりタグpush時（`v*`）に自動生成されます。

## 主な機能

- **Forced Alignment**: 歌詞言語は`Auto / en / jp`から選択（Autoはひらがな・カタカナ・漢字を含む歌詞を日本語、それ以外を英語として扱う）。
- **単曲プレイヤー**: 再生中の歌詞行ハイライト、自動スクロール、歌詞行クリックによるシーク。
- **3ペイン編集画面**: 左＝音声・原文・カタカナ入力、中央＝選択行の時刻編集、右＝アルバムアート・再生操作・同期歌詞表示。下部タイムラインは原文・カタカナを別トラック表示し、ドラッグで開始・終了時刻を移動、`Ctrl+マウスホイール`で拡大縮小。
- **音声タグの読み書き**: 曲名・アーティスト・アルバム・ジャンル・年・トラック番号・ディスク番号・アルバムアートを読み込み編集し、プロジェクトJSONへ保存可能。メニューの「ファイル」→「音声ファイルへタグを書き込む」から`MP3 + LRC` / `FLAC` / `M4A`を選んだ場合のみ音声ファイルを更新する（詳細は下記）。
- **歌唱向けカタカナ自動生成**: 同梱のCMU英語発音辞書からARPAbet音素を取得し、弱形・母音間のt/d・`t/d + y`などの連結規則を適用して発音ガイドを生成（手動入力・TXT取込済みの行は上書きしない）。

### 音声タグ書込みの詳細

- 書込み前に同じ場所へ`.tag-backup`ファイルを作成する（削除するまで元のタグへ戻せる）。
- FLAC・M4Aは同期歌詞（LRC形式のテキスト）をタグへ直接埋め込む（対応プレイヤーで再生時に表示可能）。MP3のID3v2には同等の汎用フィールドが無いため、従来どおり同じ場所に`.lrc`ファイルを書き出す。
- 読み込み中の音声がWAV・FLACなど可逆形式の場合はMP3・FLAC・M4Aいずれへも選択できる（MP3はLAME、FLACはflacenc、M4AはWindows Media Foundation標準搭載のAACエンコーダでエンコードし、いずれも追加バイナリの配布は不要）。選んだ形式と一致しない場合は変換した新しいファイルを同じ場所に作成してから書き込む（元ファイルは変更しない）。
- MP3・M4Aなど非可逆形式を読み込んでいる場合は、二重の非可逆圧縮を避けるため一致する項目のみ選択できる。

### カタカナ歌詞（任意）

原文を読み込んだ後に「カタカナ歌詞を開く（任意）」からUTF-8テキストを指定します。原文と同じ行構成、または空行を除いた歌詞行数が同じファイルを利用できます。カタカナは表示・編集・保存専用で、Forced Alignmentには使用されません。辞書外の固有名詞などは生成後にGUIへ表示されるため、確認・修正してください。

## キーボード操作

| キー | 動作 |
| --- | --- |
| `Ctrl+O` | プロジェクトを開く |
| `Ctrl+S` | 保存 |
| `Ctrl+Shift+S` | 名前を付けて保存 |
| `Ctrl+E` | LRC出力 |
| `Space` | 再生・一時停止 |
| `Ctrl+←/→` | 選択行を10ms単位で調整 |
| `Ctrl+Shift+←/→` | 選択行を100ms単位で調整 |
| `Ctrl+A` | （歌詞・設定欄での）標準的な全選択 |

補正後はプロジェクトJSONを保存し、LRCを再出力してください。

## Forced Alignmentモデルの準備

- **英語**: `models\download-forced-alignment-model.cmd`を実行し、「外部ツール設定」から`models\wav2vec2-base-960h.onnx`を選択します。
- **日本語**: `models\download-japanese-alignment-model.cmd`を実行します。[日本語Wav2Vec2 ONNX変換](https://huggingface.co/FinDIT-Studio/wav2vec2-large-xlsr-53-japanese-onnx)から約1.27GBのモデルとtokenizerを取得し、SHA-256を検証します。GUIは既定パスを自動検出します。語彙にない珍しい漢字は歌詞をひらがな・カタカナへ置き換えてください。モデルは音声認識用データで学習されているため、歌唱・伴奏条件によって同期精度は変わります。
- **ボーカル分離（任意）**: `models\download-vocal-separation-model.cmd`を実行して[HT-Demucs ONNXモデル](https://huggingface.co/StemSplitio/htdemucs-ft-vocals-onnx)（約166MB、SHA-256検証あり）を取得すると、「外部ツール設定」でパスが自動検出され、「歌詞に時刻を割り当てる（高精度）」ボタンが使えるようになります。伴奏の強い曲・激しくミックスされた曲では、Wav2Vec2（読み上げ音声で学習）がそのままだとconfidenceが全編にわたって低くなることがあり、先にボーカルだけを分離してから渡すことで大幅に改善する場合があります。CPUのみで動作しますが、曲の長さに応じて数十秒〜数分程度の処理時間が通常の割り当てより追加でかかります。
- **GNU版Rustでビルドする場合**: `runtime\download-onnx-runtime.cmd`を一度実行し、生成された`onnxruntime.dll`と`onnxruntime_providers_shared.dll`を`benzaiten.exe`と同じディレクトリへ置いてください（配布zip・MSVCビルドには同梱済み）。

## 開発環境から実行する

通常のWindows開発環境ではRustのMSVCツールチェーン、Visual Studio Build Tools、Windows SDKを用意してから実行します。同梱のGNUツールチェーンを使う場合は上記のONNX Runtime DLLも必要です。

```powershell
cargo run --bin benzaiten
```

一般的なCargo構成として、`src/lib.rs`に処理モジュール、`src/main.rs`にGUI起動、`src/bin/`にCLI、`tests/`に統合テスト、`docs/`に設計資料を配置しています。外部ツールのパスは現在の起動中だけ保持し、プロジェクトJSONには保存しません。

## CLI

```powershell
cargo run --bin benzaiten-cli -- <audio> <lyrics.txt> <wav2vec2.onnx> <output.lrc>
```

- `--reading <katakana.txt>`: 任意のカタカナ歌詞もプロジェクトJSONへ格納する。
- `--generate-reading`: カタカナ歌詞を自動生成する（`--reading`の代わりに指定）。
- 日本語同期では日本語モデルを第3引数へ渡し、`--language jp --vocabulary models\wav2vec2-large-xlsr-53-japanese-tokenizer.json`を追加する。
- `--vocal-separator <htdemucs.onnx>`: 伴奏の強い曲でForced Alignmentの精度が低い場合、先にボーカルを分離してから整列する（任意、処理時間が増える）。

CLIはLRCを出力する前に`<output>.json`のプロジェクトを保存します。未設定時刻などでLRC出力に失敗しても、手動補正用のJSONは残ります。

## 注意事項

- LRCは空行を除外し、時刻はミリ秒から百分の一秒へ切り捨てます。
- 未設定時刻・時刻の逆転・改行を含むメタ情報はLRC出力時にエラーになります。
- 歌唱・伴奏条件により同期精度は変化するため、低confidence行は手動確認してください。
- 自動カタカナは英語辞書と一般的な連結規則による近似です。歌い手固有の母音伸長、訛り、意図的な崩し方は生成後に手動調整してください。
- タグ書込み中は音声を一時的に閉じ、完了後に同じ再生位置へ戻します。

## ライセンス・依存関係

本体は[LICENSE](LICENSE)（MIT）です。依存クレート・DLL・CMUdictのライセンス一覧は[THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt)にまとめています。依存APIのバージョンは`Cargo.lock`に固定されています。
