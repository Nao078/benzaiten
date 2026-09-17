# 弁才天 (Benzaiten)

ローカル音声と正解歌詞をCTC Forced Alignmentで直接照合し、行ごとの開始時刻を持つLRCを作るRustデスクトップアプリです。MP3/WAVをffmpegで16 kHz mono PCM WAVに変換し、Wav2Vec2 ONNXモデルの音響フレームへ正解歌詞を割り当てます。

## 実行

通常の Windows 開発環境では Rust の MSVC ツールチェーン、Visual Studio Build Tools、Windows SDK を用意してから実行します。同梱のGNUツールチェーンを使う場合は、後述のONNX Runtime DLLも必要です。

```powershell
cargo run --bin benzaiten
```

一般的なCargo構成として、`src/lib.rs` に処理モジュール、`src/main.rs` にGUI起動、`src/bin/` にCLI、`tests/` に統合テスト、`docs/` に設計資料を配置しています。

GUIでは音声・UTF-8歌詞・ffmpeg・Forced Alignmentモデルを選択できます。正解歌詞は必須です。歌詞言語は`Auto / en / jp`から選択でき、Autoはひらがな・カタカナ・漢字を含む歌詞を日本語、それ以外を英語として扱います。外部ツールのパスは現在の起動中だけ保持し、プロジェクトJSONには保存しません。

単曲プレイヤーとして、再生中の歌詞行ハイライト、自動スクロール、歌詞行クリックによるシークに対応しています。音声タグから曲名・アーティスト・アルバム・ジャンル・年・トラック番号・ディスク番号・アルバムアートを読み込み、編集内容はプロジェクトJSONへ保存できます。上部メニューバーの「ファイル」→「音声ファイルへタグを書き込む」から`MP3 + LRC` / `FLAC` / `M4A`を選んだ場合だけ音声ファイルを更新し、初回書込み前に同じ場所へ`.tag-backup`ファイルを作成します。読み込み中の音声がWAV・FLACなど可逆形式の場合は3項目とも選択でき、選んだ形式と一致しない場合はffmpegで変換した新しいファイルを同じ場所に作成してから書き込みます（元ファイルは変更しません）。MP3・M4Aなど非可逆形式を読み込んでいる場合は、二重の非可逆圧縮を避けるため一致する項目のみ選択できます。FLAC・M4Aは同期歌詞（LRC形式のテキスト）をタグへ直接埋め込みます（対応プレイヤーで再生時に同期歌詞を表示可能）。MP3のID3v2には同等の汎用フィールドがないため、従来どおり同じ場所に`.lrc`ファイルを書き出します。

画面は3ペインと下部タイムラインの構成です。左ペインには音声選択、正解歌詞の入力欄、任意のカタカナ歌詞入力欄を配置しています。中央は選択中または再生中の歌詞を大きく表示し、開始時刻、調整幅、前後移動、再生位置の採用、後続行の一括移動を編集します。右はアルバムアート、再生操作、現在歌詞と同期歌詞表示を担当します。Windows Explorerからアプリへファイルをドロップすることもできます。

プロジェクト操作、LRC出力、外部ツール設定は画面上部のメニューバーから行います。`Ctrl+O`でプロジェクトを開く、`Ctrl+S`で保存、`Ctrl+Shift+S`で名前を付けて保存、`Ctrl+E`でLRC出力、`Space`で再生・一時停止ができます。`Ctrl+A`は歌詞や設定欄での標準的な全選択として予約しています。

下部タイムラインには原文とカタカナを別トラックで表示します。時刻付き歌詞ブロックを左右へドラッグすると開始・終了時刻を保ったまま移動し、ダブルクリックでその位置へシークします。タイムライン上で`Ctrl+マウスホイール`を操作すると時間軸を拡大・縮小できます。

タイムラインまたは右側の歌詞から行を選択すると、中央の補正パネルで開始時刻の直接入力、任意の調整幅による前後移動、再生位置の採用、選択行以降の一括移動ができます。`Ctrl+←/→`は10ms、`Ctrl+Shift+←/→`は100ms単位で選択行を調整します。補正後はプロジェクトJSONを保存し、LRCを再出力してください。

カタカナ歌詞は任意です。原文を読み込んだ後に「カタカナ歌詞を開く（任意）」からUTF-8テキストを指定してください。原文と同じ行構成、または空行を除いた歌詞行数が同じファイルを利用できます。カタカナは表示・編集・保存専用で、Forced Alignmentには使用されません。

「歌唱向けカタカナ自動生成」は、同梱のCMU英語発音辞書から音素を取得し、弱形・母音間のt/d・`t/d + y`などの連結規則を適用して発音ガイドを生成します。手動入力またはTXTから読み込んだ行は上書きしません。辞書外の固有名詞などはGUIに表示されるため、生成後に確認・修正してください。

### 初回のForced Alignmentモデル設定

英語Wav2Vec2モデルがない場合は、Windowsで`models\download-forced-alignment-model.cmd`を実行し、「外部ツール設定」から`models\wav2vec2-base-960h.onnx`を選択してください。

日本語同期を使う場合は`models\download-japanese-alignment-model.cmd`を実行してください。[日本語Wav2Vec2 ONNX変換](https://huggingface.co/FinDIT-Studio/wav2vec2-large-xlsr-53-japanese-onnx)から約1.27GBのモデルとtokenizerを取得し、SHA-256を検証します。GUIは既定パスを自動検出します。日本語モデルの語彙にない珍しい漢字は、歌詞をひらがな・カタカナへ置き換えてください。モデルは音声認識用データで学習されているため、歌唱・伴奏条件によって同期精度は変わります。

GNU版Rustでビルドする場合は`runtime\download-onnx-runtime.cmd`を一度実行し、生成された`onnxruntime.dll`と`onnxruntime_providers_shared.dll`を`benzaiten.exe`と同じディレクトリへ置いてください。このプロジェクトで生成済みのWindows版にはDLLも配置済みです。

CLI は次の形式です。

```powershell
cargo run --bin benzaiten-cli -- <audio> <lyrics.txt> <ffmpeg> <wav2vec2.onnx> <output.lrc>
```

任意のカタカナ歌詞もプロジェクトJSONへ格納する場合は、末尾に`--reading <katakana.txt>`を指定します。
自動生成する場合は、代わりに`--generate-reading`を指定します。

日本語同期では日本語モデルを第4引数へ渡し、`--language jp --vocabulary models\wav2vec2-large-xlsr-53-japanese-tokenizer.json`を追加します。

CLI は LRC を出力する前に `<output>.json` のプロジェクトを保存します。未設定時刻などで LRC 出力に失敗しても、手動補正用の JSON は残ります。

## 注意

- LRC は空行を除外し、時刻はミリ秒から百分の一秒へ切り捨てます。
- 未設定時刻・時刻の逆転・改行を含むメタ情報は LRC 出力時にエラーになります。
- 歌唱・伴奏条件により同期精度は変化するため、低confidence行は手動確認してください。
- 自動カタカナは英語辞書と一般的な連結規則による近似です。歌い手固有の母音伸長、訛り、意図的な崩し方は生成後に手動調整してください。
- タグ書込み中は音声を一時的に閉じ、完了後に同じ再生位置へ戻します。バックアップを削除するまでは元の音声タグへ戻せます。

CMUdictはCarnegie Mellon University Speech Groupの辞書（固定コミット`74790861f652b15e4ac49015a90074ad62a27690`）を使用しています。ライセンスは`assets/cmudict/LICENSE`を参照してください。

依存APIは`Cargo.lock`に固定されています。主要な外部APIは[rodio 0.21.1](https://docs.rs/rodio/0.21.1/rodio/)、[ONNX Runtime](https://onnxruntime.ai/)、[Wav2Vec2 Base 960h](https://huggingface.co/facebook/wav2vec2-base-960h)を参照してください。

設計は [docs/design.md](docs/design.md)、実装状況は [docs/implementation-status.md](docs/implementation-status.md) に記載しています。
