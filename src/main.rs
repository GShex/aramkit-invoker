// Hide the console window in release builds; this is a tray application.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icon;
mod theme;
mod tray;

use std::sync::Arc;

fn main() -> eframe::Result {
    let window_icon = Arc::new(icon::egui_icon(true));

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ARAMKit Invoker")
            .with_inner_size([560.0, 640.0])
            .with_min_inner_size([480.0, 520.0])
            .with_icon(window_icon),
        ..Default::default()
    };

    eframe::run_native(
        "ARAMKit Invoker",
        native_options,
        Box::new(|creation_context| Ok(Box::new(app::InvokerApp::new(creation_context)))),
    )
}
