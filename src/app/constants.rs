use egui::Color32;

pub const THUMBNAIL_SIZE: f32 = 120.0;
pub const INSPECTOR_WIDTH: f32 = 225.0;
pub const INSPECTOR_CONTENT_WIDTH: f32 = 195.0;
pub const LEFT_PANEL_WIDTH: f32 = 56.0;

pub const MAIN_WINDOW_WIDTH: f32 = 1185.0;
pub const MAIN_WINDOW_HEIGHT: f32 = 760.0;
pub const MIN_WINDOW_WIDTH: f32 = 960.0;
pub const MIN_WINDOW_HEIGHT: f32 = 680.0;
pub const APP_ID: &str = "texembed";
pub const APP_NAME: &str = "TEXembed";

pub const COLOR_BG: Color32 = Color32::from_rgb(10, 10, 12);
pub const COLOR_PANEL: Color32 = Color32::from_rgb(18, 18, 20);
pub const COLOR_CARD: Color32 = Color32::from_rgb(45, 45, 48);
pub const COLOR_BTN_BLUE: Color32 = Color32::from_rgb(96, 165, 250);
pub const COLOR_BTN_GRAY: Color32 = Color32::from_rgb(72, 72, 78);
pub const COLOR_TAB_ACTIVE: Color32 = Color32::from_rgb(45, 45, 48);
pub const COLOR_TAB_INACTIVE: Color32 = Color32::from_rgb(28, 28, 30);
pub const COLOR_TEXT: Color32 = Color32::from_rgb(240, 240, 245);
pub const COLOR_TEXT_SECONDARY: Color32 = Color32::from_rgb(161, 161, 170);
pub const COLOR_LABEL_BLUE: Color32 = Color32::from_rgb(120, 170, 250);
pub const COLOR_ACCENT_RED: Color32 = Color32::from_rgb(96, 165, 250);
pub const COLOR_ACCENT_ORANGE: Color32 = Color32::from_rgb(242, 146, 72);
pub const COLOR_GRAD_BLUE_BOTTOM: Color32 = Color32::from_rgb(66, 133, 244);
pub const COLOR_GRAD_ORANGE_BOTTOM: Color32 = Color32::from_rgb(220, 112, 40);
pub const COLOR_GRAD_GRAY_BOTTOM: Color32 = Color32::from_rgb(62, 62, 70);
pub const COLOR_INSPECTOR_BOX: Color32 = Color32::from_rgb(36, 36, 40);

// Submarine-style left rail visuals.
// NOTE: these are premultiplied (rgb already scaled by alpha). Using the raw
// (96, 165, 250) with a small alpha is invalid and renders as opaque blue.
pub const COLOR_PRIMARY_TRANSLUCENT: Color32 = Color32::from_rgba_premultiplied(10, 17, 25, 26);
pub const COLOR_PRIMARY_GLOW: Color32 = Color32::from_rgba_premultiplied(15, 26, 39, 40);
pub const COLOR_RAIL_HOVER: Color32 = Color32::from_rgba_premultiplied(13, 13, 13, 13);
pub const COLOR_RAIL_SEPARATOR: Color32 = Color32::from_rgba_premultiplied(16, 16, 16, 16);

// Submarine-style card / border / hover tokens.
pub const COLOR_CARD_SURFACE: Color32 = Color32::from_rgb(18, 18, 21); // bg-[#121215]
pub const COLOR_BORDER_SUBTLE: Color32 = Color32::from_rgba_premultiplied(13, 13, 13, 13); // white/5
pub const COLOR_BORDER_MEDIUM: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26); // white/10
pub const COLOR_HOVER_OVERLAY: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20); // white/[0.08]
pub const COLOR_PRIMARY_BORDER: Color32 = Color32::from_rgba_premultiplied(19, 33, 50, 51); // primary/20
