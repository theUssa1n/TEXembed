use std::time::Duration;

use eframe::egui;
use egui::{Color32, FontFamily};

use super::constants::{COLOR_BG, COLOR_BTN_BLUE, COLOR_BTN_GRAY, COLOR_PANEL, COLOR_TEXT};
use super::icons::load_png_icon;
use super::state::{AppState, FolderGallery, FolderLoadResult, Tab};

use super::{panel_central, panels_shell};

impl eframe::App for AppState {
    #[allow(clippy::too_many_lines)]
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_tooltip = None;
        let mut visuals = egui::Visuals::dark();
        let subtle_border_color = Color32::from_rgb(22, 22, 24);
        let panel_border = egui::Stroke::new(1.0_f32, subtle_border_color);
        visuals.window_stroke = panel_border;
        visuals.widgets.noninteractive.bg_stroke = panel_border;
        ctx.set_visuals(visuals);

        if self.ipc_restore_was_minimized.is_none() {
            self.ipc_restore_was_minimized = Some(false);
            self.ipc_restore_was_maximized = Some(false);
        }

        if let Some(rx) = &self.ipc_receiver {
            while let Ok(path_str) = rx.try_recv() {
                self.file_load_queue
                    .push_back(std::path::PathBuf::from(path_str));

                match (
                    self.ipc_restore_was_minimized,
                    self.ipc_restore_was_maximized,
                ) {
                    (Some(true), _) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    }
                    (Some(false), Some(true)) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    }
                    _ => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    }
                }

                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }

        let folder_load_result =
            self.folder_load_receiver
                .as_ref()
                .and_then(|rx| match rx.try_recv() {
                    Ok(r) => Some(r),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        Some(FolderLoadResult::Err("Folder loading failed.".to_string()))
                    }
                });

        if let Some(result) = folder_load_result {
            self.folder_load_receiver = None;
            match result {
                FolderLoadResult::Ok(build) => {
                    let tab_id = self.next_tab_id;
                    self.next_tab_id += 1;

                    let file = texembed::TextureFile {
                        file_type: texembed::FileType::Textures,
                        path: build.folder_path,
                        dds_list: Vec::new(),
                        original_bytes: Vec::new(),
                    };

                    let tab = Tab {
                        id: tab_id,
                        file,
                        selected_index: Some(0),
                        has_unsaved_changes: false,
                        show_full_preview: false,
                        full_preview_zoom: 1.0,
                        full_preview_pan: egui::Vec2::ZERO,
                        replace_backups: std::collections::HashMap::new(),
                        folder_gallery: Some(FolderGallery {
                            files: build.files,
                            index_map: build.index_map,
                            file_global_ranges: build.file_global_ranges,
                            file_expanded: build.file_expanded,
                            include_multi_texture: build.include_multi_texture,
                        }),
                    };

                    self.tabs.push(tab);
                    self.active_tab_id = Some(tab_id);
                    self.enqueue_preview_load(tab_id, 0);
                    self.log("Loaded folder gallery!");
                    self.error_message = None;
                    ctx.request_repaint();
                }
                FolderLoadResult::Err(err) => {
                    self.error_message = Some(err);
                }
            }
        }

        if !self.file_load_queue.is_empty() {
            let budget = Duration::from_millis(6);
            let queue_len = self.file_load_queue.len();
            let max_this_frame = match queue_len {
                0..=1 => 1,
                2..=5 => 2,
                6..=20 => 3,
                _ => 4,
            }
            .min(6);

            let start = std::time::Instant::now();
            let mut loaded = 0usize;

            while loaded < max_this_frame && start.elapsed() < budget {
                let Some(path) = self.file_load_queue.pop_front() else {
                    break;
                };
                self.try_load_path(&path);
                loaded += 1;
            }

            if loaded > 0 {
                ctx.request_repaint();
            }
        }

        self.process_preview_results(ctx);
        self.dispatch_preview_jobs(ctx);

        if !self.style_initialized {
            let mut style = (*ctx.style()).clone();

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
        }

        panels_shell::draw(self, ctx);
        panel_central::draw(self, ctx);
        self.delayed_tooltip(ctx, self.frame_tooltip);
    }
}
