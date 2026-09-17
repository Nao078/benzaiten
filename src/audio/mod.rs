//! 音声再生と、Forced Alignmentに必要なffmpeg前処理。

/// GUIの再生操作（再生・一時停止・シーク）を担うrodioベースの再生処理。
pub mod player;
/// 任意の入力音声を、Wav2Vec2モデルが要求する16kHzモノラルPCM16 WAVへ変換する。
pub mod preprocess;
