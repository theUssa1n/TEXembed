use eframe::egui;
use egui::{ColorImage, TextureHandle, TextureOptions};

pub(super) fn load_png_icon(
    ctx: &egui::Context,
    name: &str,
    bytes: &[u8],
) -> Option<TextureHandle> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
    let rgba = image.to_rgba8();
    let size = [rgba.width() as _, rgba.height() as _];
    let pixels = rgba.into_raw();
    let color_image = ColorImage::from_rgba_unmultiplied(size, &pixels);
    Some(ctx.load_texture(name, color_image, TextureOptions::default()))
}

pub fn load_icon() -> Option<egui::IconData> {
    let icon_bytes = include_bytes!("../../appicon.png");
    let image = image::load_from_memory_with_format(icon_bytes, image::ImageFormat::Png).ok()?;
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}
