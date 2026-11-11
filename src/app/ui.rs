use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;
use egui::{Color32, FontFamily};

use super::constants::{COLOR_BG, COLOR_BTN_BLUE, COLOR_BTN_GRAY, COLOR_PANEL, COLOR_TEXT};
use super::icons::load_png_icon;
use super::state::AppState;

use super::{panel_central, panels_shell};

const MAX_IPC_MESSAGES_PER_FRAME: usize = 16;

fn drain_ipc_messages(receiver: &Receiver<String>) -> (Vec<String>, bool) {
    let mut messages = Vec::new();
    let mut disconnected = false;
    for _ in 0..MAX_IPC_MESSAGES_PER_FRAME {
        match receiver.try_recv() {
            Ok(message) => messages.push(message),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                disconnected = true;
                break;
            }
        }
    }
    (messages, disconnected)
}

impl AppState {
    /// Closing the window (X button, Alt+F4, taskbar) must not silently drop
    /// unsaved edits: cancel the close and ask first.
    pub(super) fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested())
            && !self.allow_app_close
            && self.tabs.iter().any(|tab| tab.has_unsaved_changes)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_exit_dialog = true;
        }
    }
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_tooltip = None;

        self.handle_close_request(ctx);
        self.process_load_results(ctx);
        self.process_ipc_messages(ctx);
        self.initialize_style(ctx);

        let full_preview = self.active_full_preview_key();
        self.previews.retain_full(full_preview);
        self.process_preview_results(ctx);
        panels_shell::draw(self, ctx);
        let full_preview = self.active_full_preview_key();
        self.previews.retain_full(full_preview);
        panel_central::draw(self, ctx);
        let full_preview = self.active_full_preview_key();
        self.previews.retain_full(full_preview);
        self.dispatch_preview_jobs(ctx);
        self.loading.dispatch_jobs(ctx);
        self.delayed_tooltip(ctx, self.frame_tooltip);
    }
}

impl AppState {
    fn initialize_style(&mut self, ctx: &egui::Context) {
        if !self.style_initialized {
            let mut style = (*ctx.style()).clone();
            style.visuals = egui::Visuals::dark();

            for font_id in style.text_styles.values_mut() {
                font_id.family = FontFamily::Proportional;
            }

            style.visuals.window_fill = COLOR_BG;
            style.visuals.panel_fill = COLOR_BG;
            style.visuals.extreme_bg_color = COLOR_PANEL;
            style.visuals.widgets.noninteractive.bg_stroke =
                egui::Stroke::new(1.0_f32, Color32::from_rgb(24, 24, 28));

            style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, COLOR_TEXT);
            style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, COLOR_TEXT);
            style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, COLOR_TEXT);
            style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, COLOR_TEXT);
            style.visuals.widgets.inactive.bg_fill = COLOR_BTN_GRAY;
            style.visuals.widgets.hovered.bg_fill = COLOR_BTN_BLUE;
            style.visuals.widgets.active.bg_fill = COLOR_BTN_BLUE;
            style.visuals.selection.bg_fill = COLOR_BTN_BLUE;
            style.visuals.window_stroke = egui::Stroke::NONE;
            style.visuals.widgets.noninteractive.rounding = egui::Rounding::same(2.0);
            style.visuals.widgets.inactive.rounding = egui::Rounding::same(2.0);
            style.visuals.widgets.hovered.rounding = egui::Rounding::same(2.0);
            style.visuals.widgets.active.rounding = egui::Rounding::same(2.0);
            style.spacing.item_spacing = egui::vec2(12.0, 12.0);
            style.spacing.scroll = egui::style::ScrollStyle::solid();

            ctx.set_style(style);
            self.style_initialized = true;

            self.icon_open_file =
                load_png_icon(ctx, "open_file_icon", include_bytes!("../../open-file.png"));
            self.icon_open_folder = load_png_icon(
                ctx,
                "open_folder_icon",
                include_bytes!("../../open-folder.png"),
            );
            self.icon_logs = load_png_icon(ctx, "logs_icon", include_bytes!("../../logs.png"));
            self.icon_about = load_png_icon(ctx, "about_icon", include_bytes!("../../about.png"));
            self.icon_tweaks =
                load_png_icon(ctx, "tweaks_icon", include_bytes!("../../tweaks.png"));
        }
    }

    fn process_ipc_messages(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.ipc_receiver else {
            return;
        };
        let (messages, disconnected) = drain_ipc_messages(receiver);
        if messages.len() == MAX_IPC_MESSAGES_PER_FRAME {
            ctx.request_repaint();
        }
        if !messages.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        for path in messages {
            if !path.is_empty() {
                self.queue_file_load(std::path::PathBuf::from(path));
            }
        }
        if disconnected {
            self.ipc_receiver = None;
            self.log(
                "IPC listener disconnected; opening files from this window is still available.",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn ipc_draining_is_bounded_and_keeps_message_order() {
        let (sender, receiver) = mpsc::channel();
        for index in 0..MAX_IPC_MESSAGES_PER_FRAME + 3 {
            sender.send(index.to_string()).unwrap();
        }
        let (first, disconnected) = drain_ipc_messages(&receiver);
        assert_eq!(first.len(), MAX_IPC_MESSAGES_PER_FRAME);
        assert_eq!(first[0], "0");
        assert!(!disconnected);
        drop(sender);
        let (second, disconnected) = drain_ipc_messages(&receiver);
        assert_eq!(second.len(), 3);
        assert_eq!(second[0], MAX_IPC_MESSAGES_PER_FRAME.to_string());
        assert!(disconnected);
    }

    #[test]
    fn activation_does_not_enqueue_an_empty_path() {
        let (sender, receiver) = mpsc::channel();
        sender.send(String::new()).unwrap();
        let mut state = AppState {
            ipc_receiver: Some(receiver),
            ..Default::default()
        };
        state.process_ipc_messages(&egui::Context::default());
        assert_eq!(state.loading.pending_count(), 0);
        assert!(state.error_message.is_none());
    }

    #[test]
    fn ipc_paths_preserve_unicode_and_whitespace() {
        let (sender, receiver) = mpsc::channel();
        let path = "C:\\textures\\ leading_\u{0628} .streamtex ";
        sender.send(path.to_string()).unwrap();
        let (messages, disconnected) = drain_ipc_messages(&receiver);
        assert_eq!(messages, [path]);
        assert!(!disconnected);
    }

    #[test]
    fn style_initialization_does_not_reset_customized_visuals() {
        let mut state = AppState::default();
        let ctx = egui::Context::default();
        state.initialize_style(&ctx);
        assert!(state.style_initialized);
        let mut style = (*ctx.style()).clone();
        let custom = Color32::from_rgb(12, 34, 56);
        style.visuals.window_fill = custom;
        style.visuals.widgets.inactive.bg_fill = custom;
        ctx.set_style(style);
        state.initialize_style(&ctx);
        assert_eq!(ctx.style().visuals.window_fill, custom);
        assert_eq!(ctx.style().visuals.widgets.inactive.bg_fill, custom);
    }

    fn run_with_close_request(state: &mut AppState) -> Vec<egui::ViewportCommand> {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                events: vec![egui::ViewportEvent::Close],
                ..Default::default()
            },
        );
        let output = ctx.run(raw, |ctx| state.handle_close_request(ctx));
        output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| viewport.commands.clone())
            .unwrap_or_default()
    }

    fn tab_with_unsaved(id: usize, unsaved: bool) -> super::super::state::Tab {
        use std::collections::HashMap;
        use std::path::PathBuf;
        super::super::state::Tab {
            id,
            file: texembed::TextureFile {
                file_type: texembed::FileType::Textures,
                path: PathBuf::from(format!("file_{id}.textures")),
                dds_list: Vec::new(),
                original_bytes: Vec::new(),
                streamtex_layout: None,
            },
            selected_index: None,
            has_unsaved_changes: unsaved,
            show_full_preview: false,
            full_preview_zoom: 1.0,
            full_preview_pan: egui::Vec2::ZERO,
            replace_backups: HashMap::new(),
            folder_gallery: None,
        }
    }

    #[test]
    fn closing_with_unsaved_tabs_is_cancelled_and_asks_first() {
        let mut state = AppState::default();
        state.tabs.push(tab_with_unsaved(1, false));
        state.tabs.push(tab_with_unsaved(2, true));
        let commands = run_with_close_request(&mut state);
        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(state.show_exit_dialog);
        assert_eq!(
            state.unsaved_tab_titles(),
            vec!["file_2.textures".to_string()]
        );
    }

    #[test]
    fn closing_without_unsaved_tabs_is_not_blocked() {
        let mut state = AppState::default();
        state.tabs.push(tab_with_unsaved(1, false));
        let commands = run_with_close_request(&mut state);
        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!state.show_exit_dialog);
    }

    #[test]
    fn confirmed_quit_is_not_blocked_again() {
        let mut state = AppState::default();
        state.tabs.push(tab_with_unsaved(1, true));
        state.allow_app_close = true;
        let commands = run_with_close_request(&mut state);
        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!state.show_exit_dialog);
    }

    #[test]
    fn save_all_with_nothing_unsaved_is_a_no_op() {
        let mut state = AppState::default();
        state.tabs.push(tab_with_unsaved(1, false));
        assert_eq!(state.save_all_unsaved(), Ok(0));
    }
}
