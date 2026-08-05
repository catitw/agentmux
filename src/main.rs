#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod app;
mod detect;
mod fonts;
mod hooks;
mod notify;
mod session;
mod sidebar;
mod status;
mod terminal_pane;

use app::AgentMuxApp;

fn main() -> eframe::Result {
    // CLI-only paths run without starting the GUI.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--install-hooks") => return run_cli(hooks::install::install),
        Some("--uninstall-hooks") => return run_cli(hooks::install::uninstall),
        _ => {}
    }
    run_gui()
}

/// Run an installer CLI command; exits the process with its result.
fn run_cli(f: impl FnOnce() -> std::io::Result<()>) -> eframe::Result {
    match f() {
        Ok(()) => std::process::exit(0),
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

fn run_gui() -> eframe::Result {
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
