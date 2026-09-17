//! GUIのエントリポイント。CLI版は`src/bin/benzaiten-cli.rs`、
//! ウィンドウ・パネルの実装本体は`src/app.rs`を参照。

// コンソールウィンドウを抑制するのはリリースビルドのみ。
// デバッグビルドでは標準出力・標準エラーを調査用に残す。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    eframe::run_native(
        "Benzaiten",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1440.0, 850.0])
                .with_min_inner_size([1100.0, 700.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(benzaiten::app::BenzaitenApp::new(cc)))),
    )
}
