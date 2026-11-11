use std::fmt::Write as _;

use super::constants::{
    COLOR_ACCENT_ORANGE, COLOR_ACCENT_RED, COLOR_BG, COLOR_BTN_BLUE, COLOR_CARD,
    COLOR_GRAD_BLUE_BOTTOM, COLOR_GRAD_GRAY_BOTTOM, COLOR_GRAD_ORANGE_BOTTOM, COLOR_LABEL_BLUE,
    COLOR_PANEL, COLOR_PREVIEW_BG, COLOR_PRIMARY_GLOW, COLOR_PRIMARY_TRANSLUCENT, COLOR_RAIL_HOVER,
    COLOR_RAIL_SEPARATOR, COLOR_TAB_ACTIVE, COLOR_TEXT, COLOR_TEXT_SECONDARY, LEFT_PANEL_WIDTH,
};
use super::state::{
    compile_batch_pattern, AppState, BatchFilter, BatchMultiStatus, BatchReplaceRule, PreviewState,
};
use super::widgets::{gradient_button, paint_checkerboard, paint_texture_fitted};
use eframe::egui;
use egui::{Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, Sense, TextureHandle, Vec2};
use rfd::FileDialog;

pub(super) fn draw(state: &mut AppState, ctx: &egui::Context) {
    let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
    if !dropped_files.is_empty() {
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for file in dropped_files {
            if let Some(path) = file.path {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if ext == "textures" || ext == "streamtex" {
                    paths.push(path);
                }
            }
        }

        if paths.len() == 1 {
            state.queue_file_load(paths.remove(0));
        } else if paths.len() > 1 {
            state.multi_open_paths = paths;
            state.multi_open_shared_view = true;
            state.multi_open_include_multi_texture = true;
            state.show_multi_open_dialog = true;
        }
    }

    let mut do_close_tab = false;
    let mut do_save = false;
    let mut do_next_tab = false;

    ctx.input_mut(|i| {
        if i.consume_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::W,
        )) || i.consume_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::CTRL,
            egui::Key::W,
        )) {
            do_close_tab = true;
        }
        if i.consume_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::S,
        )) || i.consume_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::CTRL,
            egui::Key::S,
        )) {
            do_save = true;
        }
        if i.consume_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::CTRL,
            egui::Key::Tab,
        )) || i.consume_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::Tab,
        )) {
            do_next_tab = true;
        }
    });

    let grid_visible = !state.show_about
        && !state.show_logs
        && !state.show_tweaks
        && state.active_tab().is_some_and(|tab| !tab.show_full_preview);
    if grid_visible {
        super::search::handle_shortcut(state, ctx);
    }

    if do_close_tab {
        if let Some(tab_id) = state.active_tab_id {
            state.close_tab(tab_id);
        }
    }
    if do_save && state.can_save() {
        state.save_changes();
    }
    if do_next_tab && !state.tabs.is_empty() {
        if let Some(current_id) = state.active_tab_id {
            if let Some(pos) = state.tabs.iter().position(|t| t.id == current_id) {
                let next_pos = (pos + 1) % state.tabs.len();
                state.active_tab_id = Some(state.tabs[next_pos].id);
            }
        } else {
            state.active_tab_id = state.tabs.first().map(|t| t.id);
        }
    }

    draw_close_dialog(state, ctx);
    draw_exit_dialog(state, ctx);
    draw_error_dialog(state, ctx);
    draw_open_folder_options(state, ctx);
    draw_multi_open_dialog(state, ctx);
    draw_batch_replace_dialog(state, ctx);

    draw_top_bar(state, ctx);
    draw_left_panel(state, ctx);
    draw_status_bar(state, ctx);
    super::inspector::draw(state, ctx);
}

fn draw_close_dialog(state: &mut AppState, ctx: &egui::Context) {
    let Some(tab_id) = state.show_close_dialog else {
        return;
    };
    let mut close_dialog = true;
    egui::Window::new("Unsaved Changes")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .open(&mut close_dialog)
        .show(ctx, |ui| {
            ui.label("You have unsaved changes. Are you sure you want to close this tab?");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Yes, Close").clicked() {
                    state.force_close_tab(tab_id);
                    state.show_close_dialog = None;
                }
                if ui.button("Cancel").clicked() {
                    state.show_close_dialog = None;
                }
            });
        });
    if !close_dialog {
        state.show_close_dialog = None;
    }
}

fn draw_exit_dialog(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_exit_dialog {
        return;
    }
    let titles = state.unsaved_tab_titles();
    if titles.is_empty() {
        state.show_exit_dialog = false;
        return;
    }

    let mut open = true;
    let mut save_and_quit = false;
    let mut quit_without_saving = false;
    let mut cancel = false;
    egui::Window::new("Unsaved Changes")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .open(&mut open)
        .show(ctx, |ui| {
            let count = titles.len();
            ui.label(if count == 1 {
                "1 tab has unsaved changes. Save it before quitting?".to_string()
            } else {
                format!("{count} tabs have unsaved changes. Save them before quitting?")
            });
            ui.add_space(6.0);
            for title in titles.iter().take(6) {
                ui.label(
                    egui::RichText::new(format!("  {title}"))
                        .color(COLOR_TEXT_SECONDARY)
                        .size(12.0),
                );
            }
            if titles.len() > 6 {
                ui.label(
                    egui::RichText::new(format!("  ... and {} more", titles.len() - 6))
                        .color(COLOR_TEXT_SECONDARY)
                        .size(12.0),
                );
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Save and quit").clicked() {
                    save_and_quit = true;
                }
                if ui.button("Quit without saving").clicked() {
                    quit_without_saving = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

    if save_and_quit {
        state.log("Saving all changes before quitting...");
        match state.save_all_unsaved() {
            Ok(_) => {
                state.allow_app_close = true;
                state.show_exit_dialog = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(err) => {
                state.show_exit_dialog = false;
                state.report_error(format!("Error saving: {err}"));
            }
        }
    } else if quit_without_saving {
        state.allow_app_close = true;
        state.show_exit_dialog = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    } else if cancel || !open {
        state.show_exit_dialog = false;
    }
}

fn draw_error_dialog(state: &mut AppState, ctx: &egui::Context) {
    let Some(err) = state.error_message.clone() else {
        return;
    };
    let mut show_err = true;
    let window = egui::Window::new("Error")
        .collapsible(false)
        .resizable(true)
        .default_width(500.0)
        .default_height(300.0)
        .min_width(400.0)
        .min_height(200.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .open(&mut show_err);

    window.show(ctx, |ui| {
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new(&err).color(COLOR_TEXT).monospace());
            });

        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("OK").color(Color32::WHITE))
                        .fill(COLOR_BTN_BLUE),
                )
                .clicked()
            {
                state.error_message = None;
            }
        });
    });
    if !show_err {
        state.error_message = None;
    }
}

fn draw_open_folder_options(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_open_folder_options {
        return;
    }

    let mut show = true;
    egui::Window::new("open_folder_view_dialog")
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .default_width(360.0)
        .min_width(360.0)
        .max_width(360.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .frame(
            egui::Frame::none()
                .fill(COLOR_PANEL)
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(16.0, 16.0)),
        )
        .open(&mut show)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Open folder view")
                        .color(COLOR_TEXT)
                        .size(16.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close_resp = ui.add_sized(
                        Vec2::new(22.0, 22.0),
                        egui::Button::new(egui::RichText::new("x").color(COLOR_TEXT_SECONDARY))
                            .fill(COLOR_CARD),
                    );
                    let close_resp = close_resp.on_hover_cursor(CursorIcon::PointingHand);
                    if close_resp.clicked() {
                        state.show_open_folder_options = false;
                    }
                });
            });

            ui.add_space(12.0);
            ui.label(egui::RichText::new("Folder view options").color(COLOR_TEXT));
            ui.add_space(8.0);
            ui.radio_value(
                &mut state.folder_include_multi_texture,
                false,
                "Only single-texture files",
            );
            ui.radio_value(
                &mut state.folder_include_multi_texture,
                true,
                "Include multi-texture files",
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let total_w = ui.available_width();
                let cancel_w = 84.0;
                let choose_w = (total_w - 8.0 - cancel_w).max(0.0);

                if gradient_button(
                    ui,
                    true,
                    "Choose folder...",
                    Vec2::new(choose_w, 26.0),
                    COLOR_GRAD_BLUE_BOTTOM,
                )
                .clicked()
                {
                    if let Some(folder_path) = FileDialog::new().pick_folder() {
                        let include = state.folder_include_multi_texture;
                        state.load_folder_gallery(folder_path, include);
                        state.show_open_folder_options = false;
                    }
                }
                if gradient_button(
                    ui,
                    true,
                    "Cancel",
                    Vec2::new(cancel_w, 26.0),
                    COLOR_GRAD_GRAY_BOTTOM,
                )
                .clicked()
                {
                    state.show_open_folder_options = false;
                }
            });
        });
    if !show {
        state.show_open_folder_options = false;
    }
}

#[allow(clippy::too_many_lines)]
fn draw_multi_open_dialog(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_multi_open_dialog {
        return;
    }

    let mut show = true;
    egui::Window::new("multi_open_dialog")
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .default_width(420.0)
        .min_width(420.0)
        .max_width(420.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .frame(
            egui::Frame::none()
                .fill(COLOR_PANEL)
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(16.0, 16.0)),
        )
        .open(&mut show)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Open {} files", state.multi_open_paths.len()))
                        .color(COLOR_TEXT)
                        .size(16.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close_resp = ui.add_sized(
                        Vec2::new(22.0, 22.0),
                        egui::Button::new(egui::RichText::new("x").color(COLOR_TEXT_SECONDARY))
                            .fill(COLOR_CARD),
                    );
                    let close_resp = close_resp.on_hover_cursor(CursorIcon::PointingHand);
                    if close_resp.clicked() {
                        state.show_multi_open_dialog = false;
                        state.multi_open_paths.clear();
                    }
                });
            });

            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("How do you want to display them?")
                    .color(COLOR_TEXT)
                    .size(13.0),
            );
            ui.add_space(8.0);
            ui.radio_value(
                &mut state.multi_open_shared_view,
                true,
                "Shared view (recommended)",
            );
            ui.radio_value(&mut state.multi_open_shared_view, false, "Separate tabs");

            if state.multi_open_shared_view {
                ui.add_space(12.0);
                ui.label(egui::RichText::new("Shared view options").color(COLOR_TEXT));
                ui.add_space(8.0);
                ui.radio_value(
                    &mut state.multi_open_include_multi_texture,
                    false,
                    "Only single-texture files",
                );
                ui.radio_value(
                    &mut state.multi_open_include_multi_texture,
                    true,
                    "Include multi-texture files",
                );
            }

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let total_w = ui.available_width();
                let cancel_w = 84.0;
                let open_w = (total_w - 8.0 - cancel_w).max(0.0);

                if gradient_button(
                    ui,
                    true,
                    "Open",
                    Vec2::new(open_w, 26.0),
                    COLOR_GRAD_BLUE_BOTTOM,
                )
                .clicked()
                {
                    let paths = std::mem::take(&mut state.multi_open_paths);
                    if state.multi_open_shared_view {
                        let include = state.multi_open_include_multi_texture;
                        state.load_multi_file_gallery(paths, include);
                    } else {
                        for path in paths {
                            state.queue_file_load(path);
                        }
                    }
                    state.show_multi_open_dialog = false;
                }

                if gradient_button(
                    ui,
                    true,
                    "Cancel",
                    Vec2::new(cancel_w, 26.0),
                    COLOR_GRAD_GRAY_BOTTOM,
                )
                .clicked()
                {
                    state.show_multi_open_dialog = false;
                    state.multi_open_paths.clear();
                }
            });
        });

    if !show {
        state.show_multi_open_dialog = false;
        state.multi_open_paths.clear();
    }
}

/// Deferred action produced while rendering a batch replace row. The rows are
/// drawn from an immutable borrow of `AppState`, so every interaction is
/// recorded here and applied once the loop is finished.
enum BatchRowAction {
    Focus(usize),
    Toggle(usize, bool),
    Clear(usize),
    Assign(usize),
    AssignPath(usize, std::path::PathBuf),
}

/// Compact toolbar button used inside the batch replace dialog.
fn small_tool_button(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    let button = egui::Button::new(
        egui::RichText::new(text)
            .color(if enabled {
                COLOR_TEXT
            } else {
                COLOR_TEXT_SECONDARY
            })
            .size(12.0),
    )
    .fill(COLOR_CARD)
    .stroke(egui::Stroke::new(1.0_f32, COLOR_RAIL_SEPARATOR))
    .rounding(egui::Rounding::same(4.0));
    ui.add_enabled(enabled, button)
        .on_hover_cursor(CursorIcon::PointingHand)
}

/// Totals used by the multi-file Apply button: rows ready to apply across all
/// groups, and the total number of rows across all groups.
fn multi_group_totals(groups: &[super::state::BatchMultiGroup]) -> (usize, usize) {
    let mut ready = 0usize;
    let mut total = 0usize;
    for group in groups {
        total += group.entries.len();
        ready += group
            .entries
            .iter()
            .filter(|entry| entry.is_applicable())
            .count();
    }
    (ready, total)
}

/// Draws the per-file tree shown in multi-file mode: one collapsible group per
/// open file, with its rows and a per-file success/error status.
fn draw_multi_file_groups(state: &mut AppState, ui: &mut egui::Ui) {
    if state.batch_replace_groups.is_empty() {
        ui.label(
            egui::RichText::new(
                "Scan the folder to map every open file, then Apply All to replace and save each file.",
            )
            .color(COLOR_TEXT_SECONDARY),
        );
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let group_count = state.batch_replace_groups.len();
            for group_index in 0..group_count {
                let tab_id = state.batch_replace_groups[group_index].tab_id;
                let tab_name = state.batch_replace_groups[group_index].tab_name.clone();
                let (status_text, status_color) =
                    match &state.batch_replace_groups[group_index].status {
                        BatchMultiStatus::Pending => ("Pending".to_string(), COLOR_TEXT_SECONDARY),
                        BatchMultiStatus::Applied { count } => {
                            (format!("Saved {count} replacement(s)"), COLOR_LABEL_BLUE)
                        }
                        BatchMultiStatus::Error(message) => {
                            (format!("Error: {message}"), COLOR_ACCENT_ORANGE)
                        }
                    };
                let row_count = state.batch_replace_groups[group_index].entries.len();

                egui::CollapsingHeader::new(format!("{tab_name} ({row_count})"))
                    .id_salt(("batch_multi_group", tab_id))
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(status_text)
                                .color(status_color)
                                .size(11.0),
                        );
                        ui.add_space(4.0);

                        let entries = &mut state.batch_replace_groups[group_index].entries;
                        for entry in entries.iter_mut() {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut entry.included, "")
                                    .on_hover_text("Include this row in Apply All.");
                                entry.included = entry.included && entry.source_path.is_some();
                                ui.label(
                                    egui::RichText::new(&entry.target_name)
                                        .color(COLOR_TEXT)
                                        .size(12.0),
                                );
                                let source =
                                    entry.source_name.clone().unwrap_or_else(|| "—".to_string());
                                let source_color = if entry.source_path.is_some() {
                                    COLOR_LABEL_BLUE
                                } else {
                                    COLOR_TEXT_SECONDARY
                                };
                                ui.label(
                                    egui::RichText::new(format!("← {source}"))
                                        .color(source_color)
                                        .size(11.0),
                                );
                                ui.label(
                                    egui::RichText::new(entry.status_label())
                                        .color(COLOR_TEXT_SECONDARY)
                                        .size(11.0),
                                );
                            });
                        }
                    });
                ui.add_space(4.0);
            }
        });
}

#[allow(clippy::too_many_lines)]
fn draw_batch_replace_dialog(state: &mut AppState, ctx: &egui::Context) {
    if !state.show_batch_replace_dialog {
        return;
    }

    let Some(tab_id) = state.batch_replace_tab_id else {
        state.show_batch_replace_dialog = false;
        return;
    };
    let Some(target_tab) = state.tab_by_id(tab_id) else {
        state.show_batch_replace_dialog = false;
        state.batch_replace_tab_id = None;
        return;
    };

    let scope_label = if target_tab.folder_gallery.is_some() {
        "Current folder view"
    } else {
        "Current file"
    };
    let ready_count = state
        .batch_replace_entries
        .iter()
        .filter(|entry| entry.is_ready())
        .count();
    let missing_count = state
        .batch_replace_entries
        .iter()
        .filter(|entry| entry.is_missing())
        .count();
    let issue_count = state
        .batch_replace_entries
        .len()
        .saturating_sub(ready_count + missing_count);
    let included_count = state
        .batch_replace_entries
        .iter()
        .filter(|entry| entry.included)
        .count();
    let applicable_count = state
        .batch_replace_entries
        .iter()
        .filter(|entry| entry.is_applicable())
        .count();
    let duplicate_count = state
        .batch_replace_entries
        .iter()
        .filter(|entry| entry.is_duplicate())
        .count();

    let mut show = true;
    egui::Window::new("batch_replace_dialog")
        .collapsible(false)
        .resizable(true)
        .title_bar(false)
        .default_width(560.0)
        .default_height(600.0)
        .min_width(480.0)
        .min_height(360.0)
        .max_width(620.0)
        .max_height(840.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .frame(
            egui::Frame::none()
                .fill(COLOR_PANEL)
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(16.0, 16.0)),
        )
        .open(&mut show)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Batch Replace DDS")
                            .color(COLOR_TEXT)
                            .size(16.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("Scope: {scope_label}"))
                            .color(COLOR_TEXT_SECONDARY)
                            .size(12.0),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close_resp = ui.add_sized(
                        Vec2::new(22.0, 22.0),
                        egui::Button::new(egui::RichText::new("x").color(COLOR_TEXT_SECONDARY))
                            .fill(COLOR_CARD),
                    );
                    let close_resp = close_resp.on_hover_cursor(CursorIcon::PointingHand);
                    if close_resp.clicked() {
                        state.show_batch_replace_dialog = false;
                    }
                });
            });

            ui.add_space(10.0);
            if ui
                .checkbox(
                    &mut state.batch_replace_multi_file,
                    "Multi-file (all open tabs)",
                )
                .on_hover_text(
                    "Apply the same mapping to every open file that contains textures, saving each file individually.",
                )
                .changed()
            {
                state.batch_replace_groups.clear();
                state.batch_replace_entries.clear();
                state.batch_replace_unused_sources.clear();
                state.batch_replace_applied.clear();
            }

            ui.add_space(12.0);
            ui.label(egui::RichText::new("Replacement DDS folder").color(COLOR_TEXT));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let folder_text = state
                    .batch_replace_folder
                    .as_ref()
                    .map_or_else(|| "No folder selected".to_string(), |path| {
                        path.display().to_string()
                    });
                ui.add_sized(
                    Vec2::new((ui.available_width() - 98.0).max(120.0), 26.0),
                    egui::Label::new(
                        egui::RichText::new(folder_text.as_str())
                            .color(COLOR_TEXT)
                            .monospace(),
                    )
                    .truncate()
                    .sense(Sense::hover()),
                )
                .on_hover_text(folder_text);
                if gradient_button(
                    ui,
                    true,
                    "Browse...",
                    Vec2::new(90.0, 26.0),
                    COLOR_GRAD_BLUE_BOTTOM,
                )
                .clicked()
                {
                    if let Some(folder_path) = FileDialog::new().pick_folder() {
                        state.batch_replace_folder = Some(folder_path);
                        state.batch_replace_entries.clear();
                        state.batch_replace_unused_sources.clear();
                        state.batch_replace_groups.clear();
                    }
                }
            });

            ui.add_space(8.0);
            egui::CollapsingHeader::new("Match options")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Matching rule").color(COLOR_TEXT));
                    ui.add_space(6.0);
                    let previous_rule = state.batch_replace_rule;
                    ui.radio_value(
                        &mut state.batch_replace_rule,
                        BatchReplaceRule::ExactName,
                        "Exact name (recommended)",
                    );
                    ui.radio_value(
                        &mut state.batch_replace_rule,
                        BatchReplaceRule::Index,
                        "Index (0.dds or texture_0.dds)",
                    );
                    ui.radio_value(
                        &mut state.batch_replace_rule,
                        BatchReplaceRule::Glob,
                        "Glob pattern (tyre_d.dds -> tyre.dds)",
                    );
                    ui.radio_value(
                        &mut state.batch_replace_rule,
                        BatchReplaceRule::Regex,
                        "Regular expression with groups",
                    );
                    // Changing the matching rule invalidates the previous scan: the
                    // entries would otherwise keep the matches computed with the old
                    // rule.
                    if state.batch_replace_rule != previous_rule {
                        state.batch_replace_entries.clear();
                        state.batch_replace_unused_sources.clear();
                        state.batch_replace_groups.clear();
                    }

                    if state.batch_replace_rule.is_pattern_based() {
                        ui.add_space(8.0);
                        let bullet = if state.batch_replace_rule == BatchReplaceRule::Glob {
                            "Use * for any text and ? for one character. Each * becomes a group."
                        } else {
                            "Groups are referenced with {1}, {2}, ... and {0} is the whole match."
                        };
                        ui.label(
                            egui::RichText::new(bullet)
                                .color(COLOR_TEXT_SECONDARY)
                                .size(11.0),
                        );
                        ui.add_space(6.0);
                        let field_width = (ui.available_width() - 90.0).max(120.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Pattern")
                                    .color(COLOR_TEXT_SECONDARY)
                                    .size(12.0),
                            );
                            let changed = ui
                                .add_sized(
                                    Vec2::new(field_width, 24.0),
                                    egui::TextEdit::singleline(&mut state.batch_replace_pattern)
                                        .font(egui::TextStyle::Monospace)
                                        .hint_text("*_d.dds"),
                                )
                                .changed();
                            if changed {
                                state.batch_replace_entries.clear();
                                state.batch_replace_unused_sources.clear();
                                state.batch_replace_groups.clear();
                            }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Target name")
                                    .color(COLOR_TEXT_SECONDARY)
                                    .size(12.0),
                            );
                            let changed = ui
                                .add_sized(
                                    Vec2::new(field_width, 24.0),
                                    egui::TextEdit::singleline(&mut state.batch_replace_template)
                                        .font(egui::TextStyle::Monospace)
                                        .hint_text("{1}"),
                                )
                                .changed();
                            if changed {
                                state.batch_replace_entries.clear();
                                state.batch_replace_unused_sources.clear();
                                state.batch_replace_groups.clear();
                            }
                        });
                        ui.add_space(4.0);
                        match compile_batch_pattern(
                            state.batch_replace_rule,
                            &state.batch_replace_pattern,
                        ) {
                            Ok(_) => {
                                ui.label(
                                    egui::RichText::new("Pattern is valid")
                                        .color(COLOR_LABEL_BLUE)
                                        .size(11.0),
                                );
                            }
                            Err(err) => {
                                ui.label(
                                    egui::RichText::new(format!("Invalid pattern: {err}"))
                                        .color(COLOR_ACCENT_ORANGE)
                                        .size(11.0),
                                );
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.checkbox(&mut state.batch_replace_recursive, "Include subfolders")
                        .on_hover_text("Scan every subfolder of the replacement folder for DDS files.");

                    ui.add_space(10.0);
                    ui.checkbox(
                        &mut state.patch_header_on_replace,
                        "Allow different compression (patch .textures header)",
                    )
                    .on_hover_text(
                        "When enabled, replacing a texture with a different compression format is allowed for .textures files; the catalog sizes and records are patched automatically on save. Size and mip count must still match.",
                    );
                });

            ui.add_space(14.0);
            let multi_file = state.batch_replace_multi_file;
            ui.horizontal(|ui| {
                if gradient_button(
                    ui,
                    true,
                    "Scan Folder",
                    Vec2::new(120.0, 28.0),
                    COLOR_GRAD_BLUE_BOTTOM,
                )
                .clicked()
                {
                    if multi_file {
                        state.scan_multi_file_batch();
                    } else {
                        state.scan_batch_replace();
                    }
                }
                if multi_file {
                    let (group_ready, group_total) = multi_group_totals(&state.batch_replace_groups);
                    if gradient_button(
                        ui,
                        group_ready > 0,
                        &format!("Apply All ({group_ready}/{group_total})"),
                        Vec2::new(150.0, 28.0),
                        COLOR_GRAD_ORANGE_BOTTOM,
                    )
                    .clicked()
                    {
                        state.apply_multi_file_batch();
                    }
                } else {
                    if gradient_button(
                        ui,
                        applicable_count > 0,
                        &format!("Apply ({applicable_count})"),
                        Vec2::new(130.0, 28.0),
                        COLOR_GRAD_ORANGE_BOTTOM,
                    )
                    .clicked()
                    {
                        state.apply_batch_replace();
                    }
                    let undo_enabled = !state.batch_replace_applied.is_empty();
                    if small_tool_button(ui, undo_enabled, "Undo last batch").clicked() && undo_enabled
                    {
                        state.undo_last_batch_replace();
                    }
                }
            });

            ui.add_space(12.0);
            if multi_file {
                draw_multi_file_groups(state, ui);
                return;
            }

            if state.batch_replace_entries.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "Choose a DDS folder, then scan it to preview which textures will be replaced.",
                    )
                    .color(COLOR_TEXT_SECONDARY),
                );
                return;
            }

            ui.horizontal_wrapped(|ui| {
                if small_tool_button(ui, true, "Include all").clicked() {
                    for entry in &mut state.batch_replace_entries {
                        if entry.source_path.is_some() {
                            entry.included = true;
                        }
                    }
                }
                if small_tool_button(ui, true, "Include none").clicked() {
                    for entry in &mut state.batch_replace_entries {
                        entry.included = false;
                    }
                }
                if small_tool_button(
                    ui,
                    true,
                    &format!("Resolve duplicates ({duplicate_count})"),
                )
                .clicked()
                {
                    state.auto_fix_batch_duplicates();
                }
                if small_tool_button(ui, true, "Clear all sources").clicked() {
                    let indices: Vec<usize> = state
                        .batch_replace_entries
                        .iter()
                        .map(|entry| entry.target_index)
                        .collect();
                    for index in indices {
                        state.clear_batch_source(index);
                    }
                }
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Included: {included_count}"))
                        .color(COLOR_TEXT)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("Ready: {ready_count}"))
                        .color(COLOR_LABEL_BLUE)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("Not found: {missing_count}"))
                        .color(COLOR_TEXT_SECONDARY),
                );
                ui.label(
                    egui::RichText::new(format!("Issues: {issue_count}"))
                        .color(COLOR_ACCENT_ORANGE),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Unused DDS: {}",
                        state.batch_replace_unused_sources.len()
                    ))
                    .color(COLOR_TEXT_SECONDARY),
                );
            });

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let mut next_filter = state.batch_replace_filter;
                for filter in [
                    BatchFilter::All,
                    BatchFilter::Ready,
                    BatchFilter::Missing,
                    BatchFilter::Conflicts,
                    BatchFilter::Excluded,
                    BatchFilter::Unused,
                ] {
                    let count = if filter == BatchFilter::Unused {
                        state.batch_replace_unused_sources.len()
                    } else {
                        filter.count(&state.batch_replace_entries)
                    };
                    let selected = state.batch_replace_filter == filter;
                    let label = format!("{} ({count})", filter.label());
                    let text = egui::RichText::new(label)
                        .size(11.0)
                        .color(if selected { COLOR_TEXT } else { COLOR_TEXT_SECONDARY });
                    let button = egui::Button::new(text)
                        .fill(if selected {
                            COLOR_TAB_ACTIVE
                        } else {
                            COLOR_PANEL
                        })
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            if selected {
                                COLOR_LABEL_BLUE
                            } else {
                                COLOR_RAIL_SEPARATOR
                            },
                        ))
                        .rounding(egui::Rounding::same(10.0));
                    if ui.add(button).clicked() {
                        next_filter = filter;
                    }
                }
                state.batch_replace_filter = next_filter;

                ui.add_space(6.0);
                ui.add_sized(
                    Vec2::new(160.0, 22.0),
                    egui::TextEdit::singleline(&mut state.batch_replace_search)
                        .hint_text("Search name")
                        .font(egui::TextStyle::Small),
                );
            });

            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if small_tool_button(ui, true, "Save preset...").clicked() {
                    if let Some(path) = FileDialog::new()
                        .add_filter("Batch preset", &["txt"])
                        .set_file_name("batch-mapping.txt")
                        .save_file()
                    {
                        state.save_batch_preset(&path);
                    }
                }
                if small_tool_button(ui, true, "Load preset...").clicked() {
                    if let Some(path) = FileDialog::new()
                        .add_filter("Batch preset", &["txt"])
                        .pick_file()
                    {
                        state.load_batch_preset(&path);
                    }
                }
                ui.label(
                    egui::RichText::new(
                        "Tick a row to include it, use Change to pick a different DDS, or drop a file onto a row.",
                    )
                    .color(COLOR_TEXT_SECONDARY)
                    .size(11.0),
                );
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);

            let dropped_path = ctx.input(|input| {
                input.raw.dropped_files.iter().find_map(|file| {
                    let path = file.path.as_ref()?;
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("dds"))
                        .then(|| path.clone())
                })
            });
            let is_dragging_file = ctx.input(|input| !input.raw.hovered_files.is_empty());

            let mut row_action = None;
            let mut hovered_row = None;
            let mut rows_to_enqueue: Vec<usize> = Vec::new();
            let show_rows = state.batch_replace_filter != BatchFilter::Unused;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in &state.batch_replace_entries {
                    if !show_rows
                        || !state.batch_replace_filter.shows_row(entry)
                        || !entry.matches_search(&state.batch_replace_search)
                    {
                        continue;
                    }
                    let source_label = entry.source_name.as_deref().unwrap_or("-");
                    let source_size_label = entry
                        .source_size
                        .as_deref()
                        .map_or_else(String::new, |size| format!(" ({size})"));
                    let status_color = if entry.is_ready() {
                        COLOR_LABEL_BLUE
                    } else if entry.is_missing() {
                        COLOR_TEXT_SECONDARY
                    } else {
                        COLOR_ACCENT_ORANGE
                    };

                    let frame = egui::Frame::none()
                        .fill(COLOR_CARD)
                        .rounding(egui::Rounding::same(4.0))
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            let row_width = ui.available_width();
                            const THUMB: f32 = 46.0;
                            let control_column_width = 190.0;
                            let text_column_width =
                                (row_width - control_column_width - THUMB - 40.0).max(100.0);

                            let response = ui
                                .horizontal(|ui| {
                                    ui.set_width(row_width);
                                    let mut included = entry.included;
                                    if ui
                                        .checkbox(&mut included, "")
                                        .on_hover_text("Include this row when applying")
                                        .changed()
                                    {
                                        row_action = Some(BatchRowAction::Toggle(
                                            entry.target_index,
                                            included,
                                        ));
                                    }

                                    let (thumb_rect, _) = ui.allocate_exact_size(
                                        Vec2::splat(THUMB),
                                        Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        thumb_rect,
                                        egui::Rounding::same(4.0),
                                        COLOR_PREVIEW_BG,
                                    );
                                    if ui.is_rect_visible(thumb_rect) {
                                        rows_to_enqueue.push(entry.target_index);
                                        let content = thumb_rect.shrink(2.0);
                                        if state.show_transparency_bg {
                                            paint_checkerboard(
                                                ui,
                                                content,
                                                8.0,
                                                Color32::from_rgb(38, 38, 42),
                                                Color32::from_rgb(26, 26, 30),
                                            );
                                        }
                                        if let Some(PreviewState::Ready(texture)) = state
                                            .previews
                                            .get(&(tab_id, entry.target_index))
                                        {
                                            paint_texture_fitted(ui, content, texture);
                                        }
                                    }

                                    ui.allocate_ui_with_layout(
                                        Vec2::new(text_column_width, 0.0),
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            ui.set_width(text_column_width);
                                            ui.spacing_mut().item_spacing.y = 2.0;
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(format!(
                                                        "#{}  {}  ·  {}",
                                                        entry.target_index,
                                                        entry.target_name,
                                                        entry.target_size
                                                    ))
                                                    .color(COLOR_TEXT)
                                                    .strong(),
                                                )
                                                .truncate(),
                                            );
                                            if let Some(target_source) = &entry.target_source {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(target_source)
                                                            .color(COLOR_TEXT_SECONDARY)
                                                            .size(11.0),
                                                    )
                                                    .truncate(),
                                                );
                                            }
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(format!(
                                                        "Source DDS: {source_label}{source_size_label}"
                                                    ))
                                                    .color(COLOR_TEXT_SECONDARY)
                                                    .size(11.0),
                                                )
                                                .truncate(),
                                            );
                                            if let Some(note) = entry
                                                .prepared
                                                .as_ref()
                                                .and_then(|prepared| prepared.note.as_ref())
                                            {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(note)
                                                            .color(COLOR_ACCENT_ORANGE)
                                                            .size(11.0),
                                                    )
                                                    .truncate(),
                                                );
                                            }
                                        },
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let status = ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(entry.status_label())
                                                        .color(status_color)
                                                        .size(12.0)
                                                        .strong(),
                                                )
                                                .truncate()
                                                .sense(Sense::click()),
                                            );
                                            if let Some(path) = &entry.source_path {
                                                status.on_hover_text(path.display().to_string());
                                            } else {
                                                status.on_hover_text(
                                                    "Click the row to focus it in the grid",
                                                );
                                            }
                                            if entry.source_path.is_some()
                                                && small_tool_button(ui, true, "Clear").clicked()
                                            {
                                                row_action = Some(BatchRowAction::Clear(
                                                    entry.target_index,
                                                ));
                                            }
                                            if small_tool_button(ui, true, "Change")
                                                .on_hover_text("Choose a different DDS file")
                                                .clicked()
                                            {
                                                row_action =
                                                    Some(BatchRowAction::Assign(entry.target_index));
                                            }
                                            if entry.candidates.len() > 1 {
                                                let selected_label =
                                                    format!("Pick ({})", entry.candidates.len());
                                                egui::ComboBox::from_id_salt((
                                                    "batch_pick",
                                                    entry.target_index,
                                                ))
                                                .selected_text(
                                                    egui::RichText::new(selected_label)
                                                        .size(12.0),
                                                )
                                                .width(120.0)
                                                .show_ui(ui, |ui| {
                                                    for candidate in &entry.candidates {
                                                        let label = candidate
                                                            .file_name()
                                                            .and_then(|name| name.to_str())
                                                            .map_or_else(
                                                                || candidate.display().to_string(),
                                                                str::to_string,
                                                            );
                                                        let selected = entry.source_path.as_ref()
                                                            == Some(candidate);
                                                        if ui
                                                            .selectable_label(selected, label)
                                                            .clicked()
                                                        {
                                                            row_action =
                                                                Some(BatchRowAction::AssignPath(
                                                                    entry.target_index,
                                                                    candidate.clone(),
                                                                ));
                                                        }
                                                    }
                                                });
                                            }
                                        },
                                    );
                                })
                                .response
                                .interact(Sense::click());
                            (response, entry.target_index)
                        })
                        .inner;
                    let (response, target_index) = frame;
                    if response.hovered() {
                        hovered_row = Some(target_index);
                    }
                    if response.clicked() {
                        row_action = Some(BatchRowAction::Focus(target_index));
                    }
                    if is_dragging_file {
                        ui.painter().rect_stroke(
                            response.rect,
                            egui::Rounding::same(4.0),
                            egui::Stroke::new(2.0_f32, COLOR_LABEL_BLUE),
                        );
                    }
                    ui.add_space(4.0);
                }

                let list_unused = matches!(
                    state.batch_replace_filter,
                    BatchFilter::All | BatchFilter::Unused
                );
                if list_unused && !state.batch_replace_unused_sources.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Unused DDS files")
                            .color(COLOR_TEXT)
                            .strong(),
                    );
                    for file_name in &state.batch_replace_unused_sources {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("- {file_name}"))
                                    .color(COLOR_TEXT_SECONDARY)
                                    .size(12.0),
                            )
                            .truncate(),
                        );
                    }
                }
            });

            // Only the rows that were actually on screen requested a thumbnail,
            // so a long list never floods the preview queue.
            for index in rows_to_enqueue {
                state.enqueue_preview_load(tab_id, index);
            }

            if let Some(path) = dropped_path {
                if let Some(index) = hovered_row {
                    state.assign_batch_source(index, path);
                }
            }

            match row_action {
                Some(BatchRowAction::Focus(index)) => {
                    state.active_tab_id = Some(tab_id);
                    if let Some(tab) = state.tab_mut_by_id(tab_id) {
                        tab.selected_index = Some(index);
                    }
                }
                Some(BatchRowAction::Toggle(index, value)) => {
                    if let Some(entry) = state
                        .batch_replace_entries
                        .iter_mut()
                        .find(|entry| entry.target_index == index)
                    {
                        entry.included = value;
                    }
                }
                Some(BatchRowAction::Clear(index)) => state.clear_batch_source(index),
                Some(BatchRowAction::Assign(index)) => {
                    if let Some(path) = FileDialog::new()
                        .add_filter("DDS texture", &["dds"])
                        .pick_file()
                    {
                        state.assign_batch_source(index, path);
                    }
                }
                Some(BatchRowAction::AssignPath(index, path)) => {
                    state.assign_batch_source(index, path);
                }
                None => {}
            }
        });

    if !show {
        state.show_batch_replace_dialog = false;
    }
}

#[allow(clippy::too_many_lines)]
fn draw_top_bar(state: &mut AppState, ctx: &egui::Context) {
    let active_changed = state.previous_active_tab_id != state.active_tab_id;
    if active_changed {
        state.previous_active_tab_id = state.active_tab_id;
    }

    egui::TopBottomPanel::top("top_bar")
        .frame(
            egui::Frame::none()
                .fill(COLOR_BG)
                .inner_margin(egui::Margin::symmetric(0.0, 0.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;

                let tabs_out = egui::ScrollArea::horizontal()
                    .id_salt("tabs_scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .horizontal_scroll_offset(state.tabs_scroll_offset)
                    .max_width(ui.available_width())
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut tab_to_close = None;
                            let mut tab_to_select = None;
                            let mut tooltip_candidate: Option<(egui::Id, &'static str)> = None;

                            for tab in &state.tabs {
                                let is_active = state.active_tab_id == Some(tab.id);
                                let is_unsaved = tab.has_unsaved_changes;
                                let title = tab
                                    .file
                                    .path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("Untitled");

                                let title_galley = ui.painter().layout_no_wrap(
                                    title.to_string(),
                                    egui::FontId::proportional(13.0),
                                    COLOR_TEXT,
                                );
                                let close_galley = ui.painter().layout_no_wrap(
                                    "x".to_string(),
                                    egui::FontId::proportional(14.0),
                                    COLOR_TEXT,
                                );

                                let pad_x = 15.0;
                                let gap = 10.0;
                                let close_hit = 18.0;
                                let width = pad_x * 2.0
                                    + title_galley.size().x
                                    + gap
                                    + close_galley.size().x.max(close_hit);
                                let (rect, response) =
                                    ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click());
                                let response = response.on_hover_cursor(CursorIcon::PointingHand);

                                let close_center = Pos2::new(
                                    rect.right() - pad_x - close_galley.size().x / 2.0,
                                    rect.center().y,
                                );
                                let close_rect =
                                    Rect::from_center_size(close_center, Vec2::splat(close_hit));
                                let close_response = ui
                                    .interact(
                                        close_rect,
                                        egui::Id::new(format!("close_{}", tab.id)),
                                        Sense::click(),
                                    )
                                    .on_hover_cursor(CursorIcon::PointingHand);

                                let rounding = egui::Rounding::ZERO;
                                let hovering = response.hovered();
                                let title_color = if is_unsaved && !is_active {
                                    COLOR_ACCENT_ORANGE
                                } else if is_active || hovering {
                                    COLOR_TEXT
                                } else {
                                    COLOR_TEXT_SECONDARY
                                };
                                let close_color = if close_response.hovered() || is_active {
                                    COLOR_TEXT
                                } else {
                                    COLOR_TEXT_SECONDARY
                                };

                                {
                                    let painter = ui.painter();
                                    if is_active {
                                        painter.rect_filled(rect, rounding, COLOR_TAB_ACTIVE);
                                        painter.rect_filled(
                                            Rect::from_min_size(
                                                Pos2::new(rect.left(), rect.bottom() - 2.0),
                                                Vec2::new(rect.width(), 2.0),
                                            ),
                                            egui::Rounding::ZERO,
                                            COLOR_BTN_BLUE,
                                        );
                                    } else if hovering {
                                        painter.rect_filled(
                                            rect,
                                            rounding,
                                            Color32::from_rgba_unmultiplied(255, 255, 255, 13),
                                        );
                                    }

                                    let text_pos = Pos2::new(
                                        rect.left() + pad_x,
                                        rect.center().y - title_galley.size().y / 2.0,
                                    );
                                    painter.galley(text_pos, title_galley.clone(), title_color);

                                    if is_unsaved {
                                        painter.circle_filled(
                                            Pos2::new(
                                                text_pos.x + title_galley.size().x + 7.0,
                                                rect.center().y,
                                            ),
                                            3.5,
                                            COLOR_ACCENT_RED,
                                        );
                                    }

                                    if close_response.hovered() {
                                        painter.circle_filled(
                                            close_center,
                                            9.0,
                                            Color32::from_rgba_unmultiplied(255, 255, 255, 26),
                                        );
                                    }
                                    painter.galley(
                                        Pos2::new(
                                            close_center.x - close_galley.size().x / 2.0,
                                            close_center.y - close_galley.size().y / 2.0,
                                        ),
                                        close_galley.clone(),
                                        close_color,
                                    );
                                }

                                if active_changed && is_active {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if close_response.hovered() {
                                    tooltip_candidate = Some((
                                        egui::Id::new(format!("close_tt_{}", tab.id)),
                                        "Close tab",
                                    ));
                                }
                                if close_response.clicked() {
                                    tab_to_close = Some(tab.id);
                                } else if response.clicked() {
                                    tab_to_select = Some(tab.id);
                                }
                            }

                            if let Some(id) = tab_to_select {
                                state.active_tab_id = Some(id);
                            }
                            if let Some(id) = tab_to_close {
                                state.close_tab(id);
                            }
                            if let Some((id, text)) = tooltip_candidate {
                                state.offer_tooltip(id, text);
                            }
                        });
                    });

                state.tabs_scroll_offset = tabs_out.state.offset.x;
                let tabs_hovered = ctx.input(|i| {
                    i.pointer
                        .hover_pos()
                        .is_some_and(|p| tabs_out.inner_rect.contains(p))
                });
                if tabs_hovered {
                    let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
                    if wheel != 0.0 {
                        let max_offset =
                            (tabs_out.content_size.x - tabs_out.inner_rect.width()).max(0.0);
                        state.tabs_scroll_offset =
                            (state.tabs_scroll_offset - wheel).clamp(0.0, max_offset);
                        ctx.input_mut(|i| {
                            i.smooth_scroll_delta.y = 0.0;
                            i.raw_scroll_delta.y = 0.0;
                        });
                    }
                }
            });
        });
}

#[allow(clippy::too_many_lines)]
fn draw_left_panel(state: &mut AppState, ctx: &egui::Context) {
    egui::SidePanel::left("left_bar")
        .resizable(false)
        .exact_width(LEFT_PANEL_WIDTH)
        .frame(egui::Frame::none().fill(COLOR_BG))
        .show(ctx, |ui| {
            ui.add_space(20.0);

            let draw_side_btn = |ui: &mut egui::Ui,
                                 id: &str,
                                 is_active: bool,
                                 icon: Option<&TextureHandle>,
                                 text: Option<&'static str>|
             -> egui::Response {
                let slot_rect = ui.available_rect_before_wrap();
                let slot_rect =
                    Rect::from_min_size(slot_rect.min, Vec2::new(LEFT_PANEL_WIDTH, 50.0));

                let btn_inset = 6.0;
                let btn_w = LEFT_PANEL_WIDTH - 2.0 * btn_inset;
                let btn_h = 38.0;
                let btn_rect = Rect::from_center_size(slot_rect.center(), Vec2::new(btn_w, btn_h));

                let response = ui
                    .interact(btn_rect, egui::Id::new(id), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand);
                let painter = ui.painter();

                let rounding = egui::Rounding::same(10.0);

                let fill = if is_active {
                    COLOR_PRIMARY_TRANSLUCENT
                } else if response.hovered() {
                    COLOR_RAIL_HOVER
                } else {
                    Color32::TRANSPARENT
                };
                if fill.a() > 0 {
                    painter.rect_filled(btn_rect, rounding, fill);
                }

                // Active state: soft primary glow halo.
                if is_active {
                    painter.rect_stroke(
                        btn_rect,
                        rounding,
                        egui::Stroke::new(1.0_f32, COLOR_PRIMARY_GLOW),
                    );
                    // Left indicator stripe (mirrors Submarine's `w-0.5 h-1/2`).
                    let stripe_w = 2.5;
                    let stripe_h = btn_h * 0.5;
                    let stripe_rect = Rect::from_center_size(
                        Pos2::new(btn_rect.left() + 1.5, btn_rect.center().y),
                        Vec2::new(stripe_w, stripe_h),
                    );
                    painter.rect_filled(stripe_rect, egui::Rounding::same(2.0), COLOR_BTN_BLUE);
                }

                let icon_color = if is_active {
                    COLOR_BTN_BLUE
                } else if response.hovered() {
                    COLOR_TEXT
                } else {
                    COLOR_TEXT_SECONDARY
                };

                if let Some(tex) = icon {
                    let icon_size = Vec2::splat(26.0);
                    let icon_rect = Rect::from_center_size(btn_rect.center(), icon_size);
                    painter.image(
                        tex.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        icon_color,
                    );
                } else if let Some(text) = text {
                    painter.text(
                        btn_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        text,
                        FontId::new(16.0, FontFamily::Proportional),
                        icon_color,
                    );
                }

                ui.allocate_space(Vec2::new(LEFT_PANEL_WIDTH, 50.0));
                response
            };

            let open_btn = draw_side_btn(
                ui,
                "open_file_btn",
                false,
                state.icon_open_file.as_ref(),
                None,
            );
            if open_btn.clicked() {
                state.open_any_file();
            }

            ui.add_space(8.0);

            let folder_text = if state.icon_open_folder.is_some() {
                None
            } else {
                Some("F")
            };
            let open_folder_btn = draw_side_btn(
                ui,
                "open_folder_btn",
                false,
                state.icon_open_folder.as_ref(),
                folder_text,
            );
            if open_folder_btn.clicked() {
                state.show_open_folder_options = true;
            }

            ui.add_space(8.0);
            draw_rail_separator(ui);
            ui.add_space(8.0);

            let logs_btn = draw_side_btn(
                ui,
                "logs_btn",
                state.show_logs,
                state.icon_logs.as_ref(),
                None,
            );
            if logs_btn.clicked() {
                state.show_logs = !state.show_logs;
                if state.show_logs {
                    state.show_about = false;
                    state.show_tweaks = false;
                }
                if state.show_logs {
                    if let Some(tab) = state.active_tab_mut() {
                        tab.show_full_preview = false;
                    }
                }
            }

            ui.add_space(8.0);

            let about_btn = draw_side_btn(
                ui,
                "about_btn",
                state.show_about,
                state.icon_about.as_ref(),
                None,
            );
            if about_btn.clicked() {
                state.show_about = !state.show_about;
                if state.show_about {
                    state.show_logs = false;
                    state.show_tweaks = false;
                }
                if state.show_about {
                    if let Some(tab) = state.active_tab_mut() {
                        tab.show_full_preview = false;
                    }
                }
            }

            ui.add_space(8.0);

            let tweaks_btn = draw_side_btn(
                ui,
                "tweaks_btn",
                state.show_tweaks,
                state.icon_tweaks.as_ref(),
                Some("T"),
            );
            if tweaks_btn.clicked() {
                state.show_tweaks = !state.show_tweaks;
                if state.show_tweaks {
                    state.show_logs = false;
                    state.show_about = false;
                    if let Some(tab) = state.active_tab_mut() {
                        tab.show_full_preview = false;
                    }
                }
            }

            if open_btn.hovered() {
                state.offer_tooltip(egui::Id::new("open_file_tt"), "Open files");
            } else if open_folder_btn.hovered() {
                state.offer_tooltip(egui::Id::new("open_folder_tt"), "Open folder view");
            } else if logs_btn.hovered() {
                state.offer_tooltip(egui::Id::new("logs_tt"), "Logs");
            } else if about_btn.hovered() {
                state.offer_tooltip(egui::Id::new("about_tt"), "About");
            } else if tweaks_btn.hovered() {
                state.offer_tooltip(egui::Id::new("tweaks_tt"), "Tweaks");
            }
        });
}

fn draw_rail_separator(ui: &mut egui::Ui) {
    let available = ui.available_rect_before_wrap();
    let w = available.width() - 16.0;
    let x = available.min.x + (available.width() - w) / 2.0;
    let y = available.min.y;
    let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 1.0));
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(1.0), COLOR_RAIL_SEPARATOR);
    ui.allocate_space(Vec2::new(LEFT_PANEL_WIDTH, 1.0));
}

fn draw_status_bar(state: &AppState, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .frame(
            egui::Frame::none()
                .fill(COLOR_BG)
                .inner_margin(egui::Margin::symmetric(16.0, 6.0)),
        )
        .show(ctx, |ui| {
            let pending = state.loading.pending_count();
            if pending > 0 {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(format!("Loading {pending} request(s)..."))
                            .color(COLOR_TEXT_SECONDARY)
                            .size(12.0),
                    );
                });
            }
            let Some(tab) = state.active_tab() else {
                return;
            };

            ui.horizontal(|ui| {
                if tab
                    .folder_gallery
                    .as_ref()
                    .is_some_and(|g| !g.include_multi_texture)
                {
                    if let Some(idx) = tab.selected_index {
                        if let Some(original_filename) =
                            AppState::original_filename_in_tab(tab, idx)
                        {
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new("Source:")
                                                .color(COLOR_ACCENT_ORANGE)
                                                .size(12.0),
                                        )
                                        .sense(Sense::hover()),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&original_filename)
                                                .color(COLOR_TEXT)
                                                .size(12.0),
                                        )
                                        .sense(Sense::hover()),
                                    );
                                })
                                .response;
                            resp.on_hover_text(original_filename);
                        }
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let textures = AppState::total_textures_in_tab(tab);
                    let mut text = format!("Textures: {textures}");
                    if let Some(gallery) = &tab.folder_gallery {
                        let label = if gallery.is_folder {
                            "Folder"
                        } else {
                            "Shared"
                        };
                        if gallery.include_multi_texture {
                            write!(text, " | {label} (multi)")
                                .expect("writing to a string should never fail");
                        } else {
                            write!(text, " | {label}")
                                .expect("writing to a string should never fail");
                        }
                    }
                    ui.label(
                        egui::RichText::new(text)
                            .color(COLOR_TEXT_SECONDARY)
                            .size(12.0),
                    );
                });
            });
        });
}
