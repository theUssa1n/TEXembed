use egui::{Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, Sense, TextureHandle, Vec2};

use super::constants::{COLOR_INSPECTOR_BOX, COLOR_LABEL_BLUE};

pub(super) fn property_box(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(
            egui::RichText::new(label)
                .color(COLOR_LABEL_BLUE)
                .size(13.0)
                .strong(),
        );
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(2.0), COLOR_INSPECTOR_BOX);
        let inner = rect.shrink2(Vec2::new(8.0, 4.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(value).color(Color32::WHITE).size(13.0))
                        .selectable(true),
                );
            });
        });
    });
}

pub(super) fn guide_card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(COLOR_INSPECTOR_BOX)
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                egui::RichText::new(title)
                    .color(COLOR_LABEL_BLUE)
                    .strong()
                    .size(15.0),
            );
            ui.add_space(6.0);
            body(ui);
        });
    ui.add_space(10.0);
}

pub(super) fn dds_export_name(name: &str, index: usize) -> String {
    let base_name = if name.trim().is_empty() {
        format!("texture_{index}")
    } else {
        name.chars()
            .map(|ch| match ch {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                _ => ch,
            })
            .collect::<String>()
    };

    if base_name.to_ascii_lowercase().ends_with(".dds") {
        base_name
    } else {
        format!("{base_name}.dds")
    }
}

pub(super) fn paint_texture_fitted(ui: &egui::Ui, rect: Rect, texture: &TextureHandle) {
    let painter = ui.painter();
    let tex_size = texture.size_vec2();
    if tex_size.x <= 0.0 || tex_size.y <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let scale = (rect.width() / tex_size.x).min(rect.height() / tex_size.y);
    let draw_rect = Rect::from_center_size(rect.center(), tex_size * scale);
    painter.image(
        texture.id(),
        draw_rect,
        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub(super) fn paint_checkerboard(
    ui: &egui::Ui,
    rect: Rect,
    cell_size: f32,
    c1: Color32,
    c2: Color32,
) {
    let painter = ui.painter();
    let w = rect.width();
    let h = rect.height();
    if w <= 0.0 || h <= 0.0 || cell_size <= 1.0 {
        return;
    }
    let cols = (w / cell_size).ceil() as i32;
    let rows = (h / cell_size).ceil() as i32;
    for y in 0..rows {
        for x in 0..cols {
            let color = if (x + y) % 2 == 0 { c1 } else { c2 };
            let x0 = (x as f32).mul_add(cell_size, rect.left());
            let y0 = (y as f32).mul_add(cell_size, rect.top());
            let tile = Rect::from_min_size(Pos2::new(x0, y0), Vec2::new(cell_size, cell_size));
            let tile = tile.intersect(rect);
            if tile.is_positive() {
                painter.rect_filled(tile, egui::Rounding::ZERO, color);
            }
        }
    }
}

pub(super) fn gradient_button(
    ui: &mut egui::Ui,
    enabled: bool,
    text: &str,
    size: Vec2,
    mut bottom: Color32,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let painter = ui.painter();
    let rounding = egui::Rounding::same(4.0);

    if !enabled {
        bottom = dim_color(bottom, 0.45);
    } else if response.is_pointer_button_down_on() {
        bottom = dim_color(bottom, 0.85);
    } else if response.hovered() {
        bottom = brighten_color(bottom, 1.18);
    }

    painter.rect_filled(rect, rounding, bottom);

    painter.rect_stroke(
        rect,
        rounding,
        egui::Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
    );

    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::new(13.0, FontFamily::Proportional),
        if enabled {
            Color32::WHITE
        } else {
            Color32::from_rgb(210, 210, 220)
        },
    );

    let mut out = response;
    if enabled {
        out = out.on_hover_cursor(CursorIcon::PointingHand);
    }
    out
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn brighten_color(c: Color32, factor: f32) -> Color32 {
    let r = (f32::from(c.r()) * factor).round().clamp(0.0, 255.0) as u8;
    let g = (f32::from(c.g()) * factor).round().clamp(0.0, 255.0) as u8;
    let b = (f32::from(c.b()) * factor).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, c.a())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn dim_color(c: Color32, factor: f32) -> Color32 {
    let r = (f32::from(c.r()) * factor).round().clamp(0.0, 255.0) as u8;
    let g = (f32::from(c.g()) * factor).round().clamp(0.0, 255.0) as u8;
    let b = (f32::from(c.b()) * factor).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, c.a())
}
