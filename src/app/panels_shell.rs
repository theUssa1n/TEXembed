use std::fmt::Write as _;

use super::constants::{
    COLOR_ACCENT_ORANGE, COLOR_ACCENT_RED, COLOR_BG, COLOR_BTN_BLUE, COLOR_BTN_BLUE_HOVER,
    COLOR_CARD, COLOR_GRAD_BLUE_BOTTOM, COLOR_GRAD_GRAY_BOTTOM, COLOR_GRAD_ORANGE_BOTTOM,
    COLOR_INSPECTOR_BOX, COLOR_LABEL_BLUE, COLOR_PANEL, COLOR_TAB_ACTIVE, COLOR_TAB_INACTIVE,
    COLOR_TEXT, COLOR_TEXT_SECONDARY, INSPECTOR_CONTENT_WIDTH, INSPECTOR_WIDTH, LEFT_PANEL_WIDTH,
};
use super::preview::format_pixel_format;
use super::state::{AppState, BatchReplaceRule, PreviewState};
use super::widgets::{gradient_button, paint_checkerboard, paint_texture_fitted, property_box};
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
            state.file_load_queue.push_back(paths.remove(0));
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
    draw_error_dialog(state, ctx);
    draw_open_folder_options(state, ctx);
    draw_multi_open_dialog(state, ctx);
    draw_batch_replace_dialog(state, ctx);

    draw_top_bar(state, ctx);
    draw_left_panel(state, ctx);
    draw_status_bar(state, ctx);
    draw_inspector(state, ctx);
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
                            state.file_load_queue.push_back(path);
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
        .filter(|entry| entry.source_name.is_none())
        .count();
    let issue_count = state
        .batch_replace_entries
        .len()
        .saturating_sub(ready_count + missing_count);

    let mut show = true;
    egui::Window::new("batch_replace_dialog")
        .collapsible(false)
        .resizable(true)
        .title_bar(false)
        .default_width(560.0)
        .default_height(520.0)
        .min_width(480.0)
        .min_height(360.0)
        .max_width(620.0)
        .max_height(760.0)
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
                    }
                }
            });

            ui.add_space(12.0);
            ui.label(egui::RichText::new("Matching rule").color(COLOR_TEXT));
            ui.add_space(6.0);
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

            ui.add_space(10.0);
            ui.checkbox(
                &mut state.patch_header_on_replace,
                "Allow different compression (patch .textures header)",
            )
            .on_hover_text(
                "When enabled, replacing a texture with a different compression format is allowed for .textures files; the catalog sizes and records are patched automatically on save. Size and mip count must still match.",
            );

            ui.add_space(14.0);
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
                    state.scan_batch_replace();
                }
                if gradient_button(
                    ui,
                    ready_count > 0,
                    &format!("Apply Matched ({ready_count})"),
                    Vec2::new(150.0, 28.0),
                    COLOR_GRAD_ORANGE_BOTTOM,
                )
                .clicked()
                {
                    state.apply_batch_replace();
                }
            });

            ui.add_space(12.0);
            if state.batch_replace_entries.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "Choose a DDS folder, then scan it to preview which textures will be replaced.",
                    )
                    .color(COLOR_TEXT_SECONDARY),
                );
                return;
            }

            ui.horizontal(|ui| {
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

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);

            let mut clicked_target = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in &state.batch_replace_entries {
                    let source_label = entry.source_name.as_deref().unwrap_or("-");
                    let source_size_label = entry
                        .source_size
                        .as_deref()
                        .map_or_else(String::new, |size| format!(" ({size})"));
                    let status_color = if entry.is_ready() {
                        COLOR_LABEL_BLUE
                    } else if entry.source_name.is_none() {
                        COLOR_TEXT_SECONDARY
                    } else {
                        COLOR_ACCENT_ORANGE
                    };

                    egui::Frame::none()
                        .fill(COLOR_CARD)
                        .rounding(egui::Rounding::same(4.0))
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            let row_width = ui.available_width();
                            let status_column_width = 110.0;
                            let text_column_width =
                                (row_width - status_column_width).max(100.0);

                            let response = ui
                                .horizontal(|ui| {
                                    ui.set_width(row_width);
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
                                            }
                                        },
                                    );
                                })
                                .response
                                .interact(Sense::click());
                            if response.clicked() {
                                clicked_target = Some(entry.target_index);
                            }
                        });
                    ui.add_space(4.0);
                }

                if !state.batch_replace_unused_sources.is_empty() {
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

            if let Some(index) = clicked_target {
                state.active_tab_id = Some(tab_id);
                if let Some(tab) = state.tab_mut_by_id(tab_id) {
                    tab.selected_index = Some(index);
                }
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
                                let bg_color = if is_active {
                                    COLOR_TAB_ACTIVE
                                } else {
                                    COLOR_TAB_INACTIVE
                                };
                                let title = tab
                                    .file
                                    .path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("Untitled");
                                let title = if tab.has_unsaved_changes {
                                    format!("{title} *")
                                } else {
                                    title.to_string()
                                };

                                let rounding = egui::Rounding {
                                    nw: 6.0,
                                    ne: 6.0,
                                    sw: 0.0,
                                    se: 0.0,
                                };
                                let tab_frame = egui::Frame::none()
                                    .fill(bg_color)
                                    .rounding(rounding)
                                    .inner_margin(egui::Margin::symmetric(16.0, 6.0));

                                let response = tab_frame
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            let title_color =
                                                if tab.has_unsaved_changes && !is_active {
                                                    COLOR_ACCENT_ORANGE
                                                } else if is_active {
                                                    COLOR_TEXT
                                                } else {
                                                    COLOR_TEXT_SECONDARY
                                                };
                                            let resp = ui.add(
                                                egui::Button::new(
                                                    egui::RichText::new(&title)
                                                        .color(title_color)
                                                        .size(13.0)
                                                        .strong(),
                                                )
                                                .frame(false),
                                            );
                                            if tab.has_unsaved_changes {
                                                let (dot_rect, _) = ui.allocate_exact_size(
                                                    Vec2::splat(10.0),
                                                    Sense::hover(),
                                                );
                                                ui.painter().circle_filled(
                                                    dot_rect.center(),
                                                    3.5,
                                                    COLOR_ACCENT_RED,
                                                );
                                            }
                                            if resp.clicked() {
                                                tab_to_select = Some(tab.id);
                                            }
                                        });

                                        ui.add_space(8.0);
                                        let close_resp = ui.label(
                                            egui::RichText::new("x")
                                                .color(if is_active {
                                                    COLOR_TEXT
                                                } else {
                                                    COLOR_TEXT_SECONDARY
                                                })
                                                .size(14.0)
                                                .strong(),
                                        );
                                        let close_resp = ui
                                            .interact(
                                                close_resp.rect,
                                                egui::Id::new(format!("close_{}", tab.id)),
                                                Sense::click(),
                                            )
                                            .on_hover_cursor(CursorIcon::PointingHand);
                                        if close_resp.hovered() {
                                            tooltip_candidate = Some((
                                                egui::Id::new(format!("close_tt_{}", tab.id)),
                                                "Close tab",
                                            ));
                                        }
                                        if close_resp.clicked() {
                                            tab_to_close = Some(tab.id);
                                        }
                                    })
                                    .response;

                                if active_changed && is_active {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }

                                if response.hovered() && !is_active {
                                    ui.painter().rect_stroke(
                                        response.rect,
                                        rounding,
                                        egui::Stroke::new(
                                            1.0_f32,
                                            Color32::from_rgb(120, 170, 240),
                                        ),
                                    );
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
                let btn_rect = ui.available_rect_before_wrap();
                let btn_rect = Rect::from_min_size(btn_rect.min, Vec2::new(LEFT_PANEL_WIDTH, 50.0));
                let response = ui
                    .interact(btn_rect, egui::Id::new(id), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand);
                let painter = ui.painter();

                let bg_color = if response.hovered() {
                    COLOR_BTN_BLUE_HOVER
                } else if is_active {
                    COLOR_BTN_BLUE
                } else {
                    Color32::TRANSPARENT
                };

                let rounding = egui::Rounding {
                    nw: 0.0,
                    sw: 0.0,
                    ne: 6.0,
                    se: 6.0,
                };

                if response.hovered() || is_active {
                    painter.rect_filled(btn_rect, rounding, bg_color);
                    if response.hovered() {
                        painter.rect_stroke(
                            btn_rect,
                            rounding,
                            egui::Stroke::new(1.0_f32, Color32::from_rgb(120, 170, 240)),
                        );
                    }
                }

                if let Some(tex) = icon {
                    let icon_size = Vec2::splat(24.0);
                    let icon_rect = Rect::from_center_size(btn_rect.center(), icon_size);
                    painter.image(
                        tex.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        if is_active || response.hovered() {
                            Color32::WHITE
                        } else {
                            COLOR_TEXT_SECONDARY
                        },
                    );
                } else if let Some(text) = text {
                    painter.text(
                        btn_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        text,
                        FontId::new(16.0, FontFamily::Proportional),
                        if is_active || response.hovered() {
                            Color32::WHITE
                        } else {
                            COLOR_TEXT_SECONDARY
                        },
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

fn draw_status_bar(state: &AppState, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .frame(
            egui::Frame::none()
                .fill(COLOR_BG)
                .inner_margin(egui::Margin::symmetric(16.0, 6.0)),
        )
        .show(ctx, |ui| {
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
                        let label = if tab.file.path.is_dir() {
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

#[allow(clippy::too_many_lines)]
fn draw_inspector(state: &mut AppState, ctx: &egui::Context) {
    if state.active_tab().is_none() || state.show_logs || state.show_about || state.show_tweaks {
        return;
    }

    egui::SidePanel::right("inspector")
        .resizable(false)
        .exact_width(INSPECTOR_WIDTH)
        .frame(
            egui::Frame::none()
                .fill(COLOR_BG)
                .inner_margin(egui::Margin {
                    left: 16.0,
                    right: 4.0,
                    top: 16.0,
                    bottom: 16.0,
                }),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;

            ui.scope(|ui| {
                ui.set_max_width(INSPECTOR_CONTENT_WIDTH);

                let can_undo_selected = state.active_tab().is_some_and(|t| {
                    t.selected_index
                        .is_some_and(|idx| t.replace_backups.contains_key(&idx))
                });
                if state.can_save() {
                    if can_undo_selected {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let total_w = ui.available_width();
                            let undo_w = 64.0;
                            let save_w = (total_w - 8.0 - undo_w).max(0.0);

                            let save_resp = gradient_button(
                                ui,
                                true,
                                "Save changes",
                                Vec2::new(save_w, 26.0),
                                COLOR_GRAD_BLUE_BOTTOM,
                            );
                            if save_resp.clicked() {
                                state.save_changes();
                            }

                            let undo_resp = gradient_button(
                                ui,
                                true,
                                "Undo",
                                Vec2::new(undo_w, 26.0),
                                COLOR_GRAD_GRAY_BOTTOM,
                            );
                            if undo_resp.clicked() {
                                state.undo_selected_replace();
                            }
                        });
                    } else {
                        let save_resp = gradient_button(
                            ui,
                            true,
                            "Save changes",
                            Vec2::new(ui.available_width(), 26.0),
                            COLOR_GRAD_BLUE_BOTTOM,
                        );
                        if save_resp.clicked() {
                            state.save_changes();
                        }
                    }
                }

                egui::CollapsingHeader::new("Export")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        let export_all_resp = gradient_button(
                            ui,
                            state.can_export_all(),
                            "Export all as DDS",
                            Vec2::new(ui.available_width(), 26.0),
                            COLOR_GRAD_GRAY_BOTTOM,
                        );
                        if export_all_resp.clicked() {
                            state.export_all_dds();
                        }
                    });
            });

            ui.add_space(6.0);

            ui.scope(|ui| {
                ui.style_mut().spacing.scroll.bar_width = 2.0;

                egui::ScrollArea::vertical()
                    .id_salt("inspector_scroll")
                    .show(ui, |ui| {
                        ui.set_max_width(INSPECTOR_CONTENT_WIDTH);
                        ui.spacing_mut().item_spacing.y = 12.0;

                        let Some((index, dds)) = state.selected_item().map(|(i, d)| (i, d.clone()))
                        else {
                            let preview_size = ui.available_width();
                            let preview_rect =
                                ui.allocate_space(Vec2::new(preview_size, preview_size)).1;
                            ui.painter().rect_filled(
                                preview_rect,
                                egui::Rounding::same(4.0),
                                COLOR_INSPECTOR_BOX,
                            );
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let cb = ui.checkbox(&mut state.show_transparency_bg, "");
                                let cb = cb.on_hover_cursor(CursorIcon::PointingHand);
                                let label = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new("Transparency background")
                                            .color(COLOR_TEXT_SECONDARY)
                                            .size(12.0),
                                    )
                                    .sense(Sense::click()),
                                );
                                let label = label.on_hover_cursor(CursorIcon::PointingHand);
                                if label.clicked() {
                                    state.show_transparency_bg = !state.show_transparency_bg;
                                }
                                if cb.hovered() || label.hovered() {
                                    state.offer_tooltip(
                                        egui::Id::new("tt_transparency_bg"),
                                        "Toggle checkerboard transparency background",
                                    );
                                }
                            });
                            return;
                        };

                        let Some(tab_id) = state.active_tab_id else {
                            return;
                        };
                        state.enqueue_preview_load(tab_id, index);
                        let preview_state = state.previews.get(&(tab_id, index));

                        let preview_size = ui.available_width();
                        let (rect, preview_resp) = ui.allocate_exact_size(
                            Vec2::splat(preview_size),
                            Sense::click() | Sense::drag(),
                        );
                        let preview_resp = preview_resp.on_hover_cursor(CursorIcon::PointingHand);
                        ui.painter().rect_filled(
                            rect,
                            egui::Rounding::same(4.0),
                            COLOR_INSPECTOR_BOX,
                        );
                        let content_rect = rect.shrink(4.0);
                        if state.show_transparency_bg {
                            paint_checkerboard(
                                ui,
                                content_rect,
                                10.0,
                                Color32::from_rgb(38, 38, 42),
                                Color32::from_rgb(26, 26, 30),
                            );
                        }

                        if let Some(PreviewState::Ready(texture)) = preview_state {
                            paint_texture_fitted(ui, content_rect, texture);
                        }

                        let is_hovering = preview_resp.hovered();
                        let is_dragging_file = ctx.input(|i| !i.raw.hovered_files.is_empty());
                        let dropped_dds_path: Option<std::path::PathBuf> = ctx.input(|i| {
                            i.raw.dropped_files.iter().find_map(|f| {
                                f.path.as_ref().and_then(|p| {
                                    let is_dds = p
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .is_some_and(|e| e.eq_ignore_ascii_case("dds"));
                                    is_dds.then(|| p.clone())
                                })
                            })
                        });

                        if is_dragging_file {
                            ui.painter().rect_stroke(
                                rect,
                                egui::Rounding::same(4.0),
                                egui::Stroke::new(2.0_f32, COLOR_LABEL_BLUE),
                            );
                        }

                        if let Some(path) = dropped_dds_path {
                            let snapshot = state.active_tab().and_then(|tab| {
                                let dds = AppState::dds_by_index(tab, index)?.clone();
                                let file_type = AppState::file_type_by_index_cloned(tab, index)?;
                                Some((dds.clone(), dds.name.clone(), file_type))
                            });

                            if let Some((dds_clone, dds_name, file_type)) = snapshot {
                                state.log(&format!("Drag & drop detected: {}", path.display()));

                                match std::fs::read(&path) {
                                    Ok(new_dds_bytes) => {
                                        match AppState::prepare_replacement_dds(
                                            &file_type,
                                            &dds_clone,
                                            &new_dds_bytes,
                                            state.patch_header_on_replace,
                                            state.streamtex_allow_resize,
                                            state.resize_on_replace,
                                        ) {
                                            Ok(prepared) => {
                                                let super::state::PreparedReplacement {
                                                    bytes,
                                                    width,
                                                    height,
                                                    mipmap_count,
                                                    pixel_format,
                                                    note,
                                                } = prepared;
                                                let replaced_tab_id =
                                                    state.active_tab_mut().map(|tab| {
                                                        tab.replace_backups
                                                            .entry(index)
                                                            .or_insert_with(|| dds_clone.clone());

                                                        if let Some(dds_info) =
                                                            AppState::dds_by_index_mut(tab, index)
                                                        {
                                                            dds_info.bytes = bytes.into();
                                                            dds_info.width = width;
                                                            dds_info.height = height;
                                                            dds_info.mipmap_count = mipmap_count;
                                                            dds_info.pixel_format = pixel_format;
                                                        }

                                                        tab.has_unsaved_changes = true;
                                                        tab.id
                                                    });

                                                if let Some(tab_id) = replaced_tab_id {
                                                    state.invalidate_preview(tab_id, index);
                                                }
                                                if let Some(note) = note {
                                                    state.log(&note);
                                                }
                                                state.log(&format!(
                                                    "Replaced texture via drag & drop: {dds_name}"
                                                ));
                                            }
                                            Err(err) => state.error_message = Some(err),
                                        }
                                    }
                                    Err(err) => state.error_message = Some(err.to_string()),
                                }
                            }
                        }

                        if preview_resp.clicked() {
                            if let Some(tab) = state.active_tab_mut() {
                                tab.show_full_preview = true;
                                tab.full_preview_zoom = 1.0;
                                tab.full_preview_pan = Vec2::ZERO;
                            }
                            state.show_logs = false;
                            state.show_about = false;
                            state.show_tweaks = false;
                        }

                        if is_hovering && !is_dragging_file {
                            state.offer_tooltip(
                                egui::Id::new("tt_preview_drop"),
                                "Drag & drop a DDS file to replace this texture",
                            );
                        }

                        ui.add_space(2.0);

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let cb = ui.checkbox(&mut state.show_transparency_bg, "");
                            let cb = cb.on_hover_cursor(CursorIcon::PointingHand);
                            let label = ui.add(
                                egui::Label::new(
                                    egui::RichText::new("Transparency background")
                                        .color(COLOR_TEXT_SECONDARY)
                                        .size(12.0),
                                )
                                .sense(Sense::click()),
                            );
                            let label = label.on_hover_cursor(CursorIcon::PointingHand);
                            if label.clicked() {
                                state.show_transparency_bg = !state.show_transparency_bg;
                            }
                            if cb.hovered() || label.hovered() {
                                state.offer_tooltip(
                                    egui::Id::new("tt_transparency_bg"),
                                    "Toggle checkerboard transparency background",
                                );
                            }
                        });

                        let name_label = if dds.name.is_empty() {
                            format!("texture_{index}.dds")
                        } else {
                            dds.name.clone()
                        };
                        ui.label(
                            egui::RichText::new(name_label)
                                .color(Color32::WHITE)
                                .size(14.0)
                                .strong(),
                        );

                        ui.add_space(4.0);

                        ui.spacing_mut().item_spacing.y = 8.0;
                        property_box(ui, "Width", &dds.width.to_string());
                        property_box(ui, "Height", &dds.height.to_string());
                        property_box(ui, "Compression", &format_pixel_format(dds.bytes.as_ref()));
                        property_box(ui, "Mipmaps", &dds.mipmap_count.to_string());

                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let btn_width = (ui.available_width() - 8.0) / 2.0;
                            if gradient_button(
                                ui,
                                true,
                                "Export DDS",
                                Vec2::new(btn_width, 26.0),
                                COLOR_GRAD_BLUE_BOTTOM,
                            )
                            .clicked()
                            {
                                state.export_selected();
                            }
                            if gradient_button(
                                ui,
                                true,
                                "Replace",
                                Vec2::new(btn_width, 26.0),
                                COLOR_GRAD_ORANGE_BOTTOM,
                            )
                            .clicked()
                            {
                                state.replace_selected();
                            }
                        });

                        ui.add_space(6.0);

                        egui::CollapsingHeader::new("Tools")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 8.0;
                                if gradient_button(
                                    ui,
                                    state.can_batch_replace(),
                                    "Batch Replace...",
                                    Vec2::new(ui.available_width(), 26.0),
                                    COLOR_GRAD_ORANGE_BOTTOM,
                                )
                                .clicked()
                                {
                                    state.open_batch_replace_dialog();
                                }
                                if gradient_button(
                                    ui,
                                    true,
                                    "Export PNG",
                                    Vec2::new(ui.available_width(), 26.0),
                                    COLOR_GRAD_GRAY_BOTTOM,
                                )
                                .clicked()
                                {
                                    state.export_selected_png();
                                }
                            });
                    });
            });
        });
}
