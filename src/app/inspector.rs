use eframe::egui;
use egui::{Color32, CursorIcon, Sense, Vec2};

use super::constants::{
    COLOR_BG, COLOR_GRAD_BLUE_BOTTOM, COLOR_GRAD_GRAY_BOTTOM, COLOR_GRAD_ORANGE_BOTTOM,
    COLOR_LABEL_BLUE, COLOR_PREVIEW_BG, COLOR_TEXT_SECONDARY, INSPECTOR_CONTENT_WIDTH,
    INSPECTOR_WIDTH,
};
use super::preview::format_pixel_format;
use super::state::{AppState, PreviewState};
use super::widgets::{gradient_button, paint_checkerboard, paint_texture_fitted, property_box};

pub(super) fn draw(state: &mut AppState, ctx: &egui::Context) {
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
                draw_save_actions(state, ui);
            });
            ui.add_space(6.0);
            ui.scope(|ui| {
                ui.style_mut().spacing.scroll.bar_width = 2.0;
                egui::ScrollArea::vertical()
                    .id_salt("inspector_scroll")
                    .show(ui, |ui| {
                        ui.set_max_width(INSPECTOR_CONTENT_WIDTH);
                        ui.spacing_mut().item_spacing.y = 12.0;
                        draw_selected_texture(state, ctx, ui);
                    });
            });
        });
}

fn draw_save_actions(state: &mut AppState, ui: &mut egui::Ui) {
    let can_undo_selected = state.active_tab().is_some_and(|tab| {
        tab.selected_index
            .is_some_and(|index| tab.replace_backups.contains_key(&index))
    });
    if state.can_save() {
        if can_undo_selected {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let undo_width = 64.0;
                let save_width = (ui.available_width() - 8.0 - undo_width).max(0.0);
                if gradient_button(
                    ui,
                    true,
                    "Save changes",
                    Vec2::new(save_width, 26.0),
                    COLOR_GRAD_BLUE_BOTTOM,
                )
                .clicked()
                {
                    state.save_changes();
                }
                if gradient_button(
                    ui,
                    true,
                    "Undo",
                    Vec2::new(undo_width, 26.0),
                    COLOR_GRAD_GRAY_BOTTOM,
                )
                .clicked()
                {
                    state.undo_selected_replace();
                }
            });
        } else if gradient_button(
            ui,
            true,
            "Save changes",
            Vec2::new(ui.available_width(), 26.0),
            COLOR_GRAD_BLUE_BOTTOM,
        )
        .clicked()
        {
            state.save_changes();
        }
    }

    egui::CollapsingHeader::new("Export")
        .default_open(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if gradient_button(
                ui,
                state.can_export_all(),
                "Export all as DDS",
                Vec2::new(ui.available_width(), 26.0),
                COLOR_GRAD_GRAY_BOTTOM,
            )
            .clicked()
            {
                state.export_all_dds();
            }
        });
}

fn draw_selected_texture(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some((index, dds)) = state
        .selected_item()
        .map(|(index, dds)| (index, dds.clone()))
    else {
        let preview_size = ui.available_width();
        let preview_rect = ui.allocate_space(Vec2::splat(preview_size)).1;
        ui.painter()
            .rect_filled(preview_rect, egui::Rounding::same(4.0), COLOR_PREVIEW_BG);
        ui.add_space(2.0);
        draw_transparency_toggle(state, ui);
        return;
    };
    let Some(tab_id) = state.active_tab_id else {
        return;
    };

    draw_thumbnail(state, ctx, ui, tab_id, index);
    ui.add_space(2.0);
    draw_transparency_toggle(state, ui);

    let name = if dds.name.is_empty() {
        format!("texture_{index}.dds")
    } else {
        dds.name.clone()
    };
    ui.label(
        egui::RichText::new(name)
            .color(Color32::WHITE)
            .size(14.0)
            .strong(),
    );
    ui.add_space(4.0);
    ui.spacing_mut().item_spacing.y = 8.0;
    property_box(ui, "Width", &dds.width.to_string());
    property_box(ui, "Height", &dds.height.to_string());
    let depth = super::volume::volume_depth(dds.bytes.as_ref());
    if depth > 1 {
        property_box(ui, "Depth (volumetric)", &format!("{depth} (3D texture)"));
    }
    property_box(ui, "Compression", &format_pixel_format(dds.bytes.as_ref()));
    property_box(ui, "Mipmaps", &dds.mipmap_count.to_string());
    ui.add_space(6.0);
    draw_texture_actions(state, ui);
}

fn draw_thumbnail(
    state: &mut AppState,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    tab_id: usize,
    index: usize,
) {
    state.enqueue_preview_load(tab_id, index);
    let preview_size = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(preview_size), Sense::click() | Sense::drag());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(4.0), COLOR_PREVIEW_BG);
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
    if let Some(PreviewState::Ready(texture)) = state.previews.get(&(tab_id, index)) {
        paint_texture_fitted(ui, content_rect, texture);
    }

    let is_dragging_file = ctx.input(|input| !input.raw.hovered_files.is_empty());
    let dropped_path = ctx.input(|input| {
        input.raw.dropped_files.iter().find_map(|file| {
            let path = file.path.as_ref()?;
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("dds"))
                .then(|| path.clone())
        })
    });
    if is_dragging_file {
        ui.painter().rect_stroke(
            rect,
            egui::Rounding::same(4.0),
            egui::Stroke::new(2.0_f32, COLOR_LABEL_BLUE),
        );
    }
    if let Some(path) = dropped_path {
        state.log(&format!("Drag & drop detected: {}", path.display()));
        state.replace_from_path(tab_id, index, &path);
    }

    if response.clicked() {
        if let Some(tab) = state.tab_mut_by_id(tab_id) {
            tab.show_full_preview = true;
            tab.full_preview_zoom = 1.0;
            tab.full_preview_pan = Vec2::ZERO;
        }
        state.show_logs = false;
        state.show_about = false;
        state.show_tweaks = false;
    }
    if response.hovered() && !is_dragging_file {
        state.offer_tooltip(
            egui::Id::new("tt_preview_drop"),
            "Drag & drop a DDS file to replace this texture",
        );
    }
}

fn draw_transparency_toggle(state: &mut AppState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let checkbox = ui
            .checkbox(&mut state.show_transparency_bg, "")
            .on_hover_cursor(CursorIcon::PointingHand);
        let label = ui
            .add(
                egui::Label::new(
                    egui::RichText::new("Transparency background")
                        .color(COLOR_TEXT_SECONDARY)
                        .size(12.0),
                )
                .sense(Sense::click()),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        if label.clicked() {
            state.show_transparency_bg = !state.show_transparency_bg;
        }
        if checkbox.hovered() || label.hovered() {
            state.offer_tooltip(
                egui::Id::new("tt_transparency_bg"),
                "Toggle checkerboard transparency background",
            );
        }
    });
}

fn draw_texture_actions(state: &mut AppState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let width = (ui.available_width() - 8.0) / 2.0;
        if gradient_button(
            ui,
            true,
            "Export DDS",
            Vec2::new(width, 26.0),
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
            Vec2::new(width, 26.0),
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
}
