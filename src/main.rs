#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod app;
mod detect;
mod notify;
mod session;
mod sidebar;
mod status;
mod terminal_pane;

use app::AgentMuxApp;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0])
            .with_title("agentmux"),
        ..Default::default()
    };

    eframe::run_native(
        "agentmux",
        native_options,
        Box::new(|cc| Ok(Box::new(AgentMuxApp::new(cc)))),
    )
}
