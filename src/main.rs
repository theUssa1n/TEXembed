#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod runtime;

use std::env;
use std::ffi::OsString;
use std::net::TcpListener;
use std::path::PathBuf;

use eframe::egui;

use app::constants::{
    APP_ID, APP_NAME, MAIN_WINDOW_HEIGHT, MAIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
};
use app::icons::load_icon;
use app::ipc::{send_path_to_running_instance, start_ipc_listener, IPC_PORT};
use app::state::create_app;

fn main() -> std::process::ExitCode {
    runtime::install_panic_hook();
    match runtime::run_guarded(run) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            show_startup_message(rfd::MessageLevel::Error, &error);
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let file_to_open = startup_path(env::args_os().nth(1))?;
    let listener = match TcpListener::bind(("127.0.0.1", IPC_PORT)) {
        Ok(listener) => Some(listener),
        Err(bind_error) => match send_path_to_running_instance(file_to_open.as_deref()) {
            Ok(()) => return Ok(()),
            Err(forward_error) => {
                show_startup_message(
                    rfd::MessageLevel::Warning,
                    &format!(
                        "Could not reserve 127.0.0.1:{IPC_PORT}: {bind_error}\n\nCould not verify delivery to a compatible TEXembed instance: {forward_error}\n\nTEXembed will continue as a standalone window. Single-instance forwarding is unavailable for this window.",
                    ),
                );
                None
            }
        },
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let ipc_listener = listener.and_then(|listener| match start_ipc_listener(listener, tx) {
        Ok(listener) => Some(listener),
        Err(error) => {
            show_startup_message(
                rfd::MessageLevel::Warning,
                &format!("{error}\n\nTEXembed will continue without single-instance forwarding."),
            );
            None
        }
    });
    let icon = load_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT])
            .with_min_inner_size([MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT])
            .with_title(APP_NAME)
            .with_icon(icon.unwrap_or_default()),
        // eframe dithers every interpolated color by default, which shows up
        // as grain on smooth texture previews; disabling it matches the
        // clean scaling Paint.NET uses.
        dithering: false,
        ..Default::default()
    };

    eframe::run_native(
        APP_ID,
        options,
        Box::new(|cc| {
            if let Some(listener) = &ipc_listener {
                listener.set_context(&cc.egui_ctx);
            }
            Ok(create_app(rx, file_to_open))
        }),
    )
    .map_err(|error| format!("TEXembed could not run the application: {error}"))
}

fn startup_path(argument: Option<OsString>) -> Result<Option<PathBuf>, String> {
    let Some(argument) = argument else {
        return Ok(None);
    };
    if argument.is_empty() {
        return Err("The requested startup path is empty.".to_string());
    }
    let path = PathBuf::from(argument);
    runtime::canonicalize_path(&path)
        .map(Some)
        .map_err(|error| {
            format!(
                "Could not resolve the requested startup path {}: {error}",
                path.display()
            )
        })
}

fn show_startup_message(level: rfd::MessageLevel, message: &str) {
    let log_status = match runtime::log_error(message) {
        Ok(path) => format!("Details were saved to {}.", path.display()),
        Err(error) => format!("Could not save crash.log: {error}"),
    };
    rfd::MessageDialog::new()
        .set_level(level)
        .set_title(APP_NAME)
        .set_description(format!("{message}\n\n{log_status}"))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "texembed-startup-test-{}-{unique}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(runtime::canonicalize_path(&path).unwrap())
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn absent_startup_argument_means_activation() {
        assert_eq!(startup_path(None), Ok(None));
    }

    #[test]
    fn empty_startup_argument_is_not_activation() {
        let error = startup_path(Some(OsString::new())).unwrap_err();
        assert!(error.contains("empty"));
    }

    #[test]
    fn missing_startup_file_preserves_the_error() {
        let directory = TestDirectory::new();
        let path = directory.0.join("missing.textures");
        let error = startup_path(Some(path.into_os_string())).unwrap_err();
        assert!(error.contains("Could not resolve the requested startup path"));
        assert!(error.contains("missing.textures"));
    }

    #[test]
    fn startup_paths_are_canonical_and_preserve_unicode_and_spaces() {
        let directory = TestDirectory::new();
        let path = directory.0.join("  texture_世界 .textures ");
        fs::write(&path, b"synthetic texture").unwrap();
        let canonical = runtime::canonicalize_path(&path).unwrap();
        let result = startup_path(Some(path.into_os_string())).unwrap().unwrap();
        assert!(result.is_absolute());
        assert_eq!(result, canonical);
        assert_eq!(result.file_name().unwrap(), "  texture_世界 .textures ");
    }
}
