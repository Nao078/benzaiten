//! コアとなるデータ型と、純粋なパース・マージ処理。`domain`配下はGUIや
//! 音響モデルに依存しないため、単体テストしやすくGUI・CLI双方から
//! 再利用できる。

/// 自由入力の歌詞テキストを[`project::LyricLine`]へ変換し、
/// 別ファイルのカタカナ読みを適用する処理。
pub mod lyrics;
/// [`project::Project`]・[`project::LyricLine`]と、それらが保存される
/// スキーマバージョン。
pub mod project;
