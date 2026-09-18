# 設計

## 構成

標準的な Cargo 構成を使います。ライブラリは `src/lib.rs`、GUI は `src/main.rs`、CLI は `src/bin/benzaiten-cli.rs`、テストは `tests/`、文書は `docs/` に置きます。

`domain`はGUIや音響モデルに依存しません。`LyricLine`は正本の原文、任意のカタカナ発音ガイド、開始・終了時刻（ミリ秒）、confidence、各値の由来を保持します。`Project`はJSON保存の単位です。

```text
GUI / CLI
  └ jobs
      ├ audio::preprocess (Symphonia + rubato、純Rust)
      ├ forced_alignment::model (Wav2Vec2 ONNX)
      ├ forced_alignment::trellis (CTC Viterbi)
      ├ forced_alignment::resolver (token → lyric line)
      └ lrc::writer
```

## データと保存

- 歌詞は UTF-8 として読み、先頭 BOM を除去して行順を保持します。空行（貼り付け時の段落区切りなど）は同期対象にならないため破棄し、残った行のIDを詰め直します。
- 内部時刻はミリ秒です。`ForcedAlignment`、`Manual`などで由来を区別します。
- JSONは`schema_version: 2`を含み、v1の`text/source`を読み込み時に自動移行します。
- `original_text`だけを同期と標準LRCへ使用し、`reading_text`は表示・練習用の派生情報として分離します。
- 英語発音は同梱CMUdictからARPAbet音素を取得し、弱形、母音間フラップ、`t/d + y`、句読点境界を考慮して歌唱練習向けカタカナへ変換します。辞書外語は文字名読みへフォールバックし、ユーザー確認対象として通知します。
- 自動生成は`ReadingSource::Generated`、直接編集およびTXT取込は`ReadingSource::Manual`として保持し、再生成時も手修正を保護します。
- schema v3では音楽情報と任意のアルバムアートパスをプロジェクトへ保存します。音声タグの変更は通常のプロジェクト保存と分離し、確認後の明示操作でのみ実行します。
- 音声タグ書込みは`MP3 + LRC` / `FLAC` / `M4A`の3種類から選びます（メニューの音声形式は拡張子から判定し、実ファイルと一致しない選択肢は無効化）。実際の書込み前に`metadata::write` (`src/metadata.rs`)が`lofty`で実ファイル種別（`FileType`）を再判定するため、ラベルと中身が食い違う心配はありません。FLAC/M4Aは`ItemKey::Lyrics`（Vorbis CommentのLYRICS欄・MP4の`©lyr`アトム）へLRCテキストをそのまま埋め込みます。ID3v2（MP3）には同期歌詞の汎用フィールドが無い（正式には専用のSYLTフレームが必要）ため埋め込みは行わず、従来どおり同じ場所へ`.lrc`を書き出します。歌詞（LRC）の生成に失敗する場合はタグ書込み全体を中止します。
- 読み込み中の音声がWAV・FLACなど可逆形式（`is_lossless_audio_extension`）の場合のみ、選択形式と異なる音声への変換を許可します。変換は`convert_audio_for_tag_format`が`crate::audio::convert`を呼び、同じ場所に新しい拡張子のファイルを作成します（既存ファイルがあれば中止、元ファイルは変更しません）。FLACは`flacenc`、MP3はLAME（`mp3lame-encoder`、LGPL）、M4A（AAC-LC）はWindows Media Foundation標準搭載のエンコーダを`IMFSinkWriter`経由で直接呼び出す実装（`src/audio/aac_windows.rs`、`windows`クレート、`cfg(windows)`限定）で、いずれも外部プロセスや追加バイナリの配布は不要です。AACエンコーダの対応制約（サンプルレート44100/48000Hz、チャンネル数1/2/6）に合わせるため、それ以外の値は`negotiate_format`で近い値へ寄せてからリサンプリング・ダウンミックスします。変換後は`Project::audio_path`を新ファイルへ差し替えますが、同一音源の再エンコードなので歌詞の時刻はそのまま維持します。MP3・M4Aのような非可逆形式を読み込んでいる場合は二重の非可逆圧縮を避けるため変換を提供せず、一致する項目のみ有効にします。
- 再生位置は`start_ms <= position < end_ms`で現在行へ変換し、`end_ms`がない場合は次の設定済み`start_ms`を境界にします。
- 読込時、相対音声パスはプロジェクトファイルの親ディレクトリから解決します。音声が移動済みでも JSON 自体は開けます。

## LRC

`[ti:]` と `[ar:]` を出力し、非空行には `[mm:ss.cc]` を付けます。時刻は 10 ms 単位へ切り捨てます。非空行の開始時刻欠損、行の逆順、改行を含む入力は出力を拒否します。

## 外部ツール

`audio::preprocess::decode`（Symphonia）が入力（MP3/WAV/FLAC/OGG/M4A）をデコードし、モノラルへダウンミックスした上で`rubato`が16 kHzへリサンプリングし、`hound`でPCM16 WAVへ書き出します。外部プロセスは一切起動しません。Wav2Vec2 ONNXモデルを20秒単位で実行してCTC logitsを連結し、正解歌詞をViterbi整列します。自由文の文字起こしは行いません。現在のモデルは英語専用です。

GUI の日本語表示は OS にある日本語フォントを順に探し、見つからない場合は egui 標準フォントへフォールバックします。依存する API 系列は eframe 0.31.1 と rodio 0.21.1 で、解決結果は `Cargo.lock` に固定します。

参照: [rodio 0.21.1](https://docs.rs/rodio/0.21.1/rodio/)、[Wav2Vec2 Base 960h](https://huggingface.co/facebook/wav2vec2-base-960h)、[ONNX Runtime](https://onnxruntime.ai/)、[Symphonia](https://github.com/pdeljanov/Symphonia)、[rubato](https://docs.rs/rubato/)
