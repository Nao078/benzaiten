//! 音声再生、Forced Alignmentに必要な前処理、タグ埋め込み用の再エンコード。

/// 可逆音源をタグ埋め込み用の別形式（FLAC/MP3/M4A）へ再エンコードする処理。
pub mod convert;
/// GUIの再生操作（再生・一時停止・シーク）を担うrodioベースの再生処理。
pub mod player;
/// 任意の入力音声を、Wav2Vec2モデルが要求する16kHzモノラルPCM16 WAVへ変換する。
pub mod preprocess;
/// Forced Alignmentの前処理として、伴奏からボーカルを分離する（任意機能）。
pub mod vocal_separation;

/// Windows Media Foundation標準搭載のAACエンコーダを使ったM4A書き出し
/// （`convert::to_m4a`から使う）。このアプリはWindows専用なので、
/// 非Windowsターゲットではコンパイル対象から外す。
#[cfg(windows)]
mod aac_windows;
