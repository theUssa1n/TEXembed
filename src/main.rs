#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;

use std::env;
use std::net::TcpListener;
use std::path::PathBuf;

use eframe::egui;

use app::constants::{
    APP_ID, APP_NAME, MAIN_WINDOW_HEIGHT, MAIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
};
use app::icons::load_icon;
use app::ipc::{send_path_to_running_instance, start_ipc_listener, IPC_PORT};
use app::state::create_app;

fn main() -> Result<(), eframe::Error> {
    let args: Vec<String> = env::args().collect();
    let file_to_open = args.get(1).map(PathBuf::from);

    if let Ok(listener) = TcpListener::bind(("127.0.0.1", IPC_PORT)) {
        let (tx, rx) = std::sync::mpsc::channel();
        let icon = load_icon();
        let _start_maximized = file_to_open.is_some();

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT])
                .with_min_inner_size([MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT])
                .with_title(APP_NAME)
                .with_icon(icon.unwrap_or_default()),
            ..Default::default()
        };

        eframe::run_native(
            APP_ID,
            options,
            Box::new(move |cc| {
                start_ipc_listener(listener, tx, &cc.egui_ctx);
                Ok(create_app(rx, file_to_open))
            }),
        )
    } else {
        if let Some(path) = file_to_open {
            send_path_to_running_instance(&path);
        }
        Ok(())
    }
}
