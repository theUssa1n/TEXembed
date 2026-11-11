//! Texture name search: a VS Code style find box that filters the grid live.

use eframe::egui;
use egui::{Color32, Key, Modifiers, Rect};

use super::constants::{
    COLOR_ACCENT_ORANGE, COLOR_BORDER_MEDIUM, COLOR_CARD_SURFACE, COLOR_TEXT, COLOR_TEXT_SECONDARY,
};
use super::state::{AppState, Tab};

const BAR_WIDTH: f32 = 330.0;

/// A parsed search query. Every whitespace separated term must occur in the
/// texture name (case-insensitive), so `hud msg` matches `HUD_Message.dds`.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SearchQuery {
    terms: Vec<String>,
}

impl SearchQuery {
    /// Returns `None` for an empty or whitespace-only query.
    pub(super) fn parse(text: &str) -> Option<Self> {
        let terms: Vec<String> = text
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect();
        if terms.is_empty() {
            None
        } else {
            Some(Self { terms })
        }
    }

    pub(super) fn matches(&self, name: &str) -> bool {
        self.terms
            .iter()
            .all(|term| contains_ignore_ascii_case(name, term))
    }
}

/// ASCII case-insensitive substring test that does not allocate.
/// `needle` must already be lowercase.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Global texture indices of `tab` whose name passes `query`, ascending.
pub(super) fn matching_indices(tab: &Tab, query: &SearchQuery) -> Vec<usize> {
    let total = AppState::total_textures_in_tab(tab);
    (0..total)
        .filter(|&index| {
            AppState::dds_by_index(tab, index).is_some_and(|dds| query.matches(&dds.name))
        })
        .collect()
}

/// Pressing Ctrl+F opens the search box (or re-focuses it and selects the
/// current text).
pub(super) fn handle_shortcut(state: &mut AppState, ctx: &egui::Context) {
    let pressed = ctx.input_mut(|input| {
        input.consume_shortcut(&egui::KeyboardShortcut::new(Modifiers::CTRL, Key::F))
            || input.consume_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::F))
    });
    if pressed && !state.tabs.is_empty() {
        state.open_search();
    }
}

/// Draws the floating search box in the top right corner of `panel_rect`.
/// `matches` is the number of textures that pass the filter and `total` the
/// number of textures in the tab.
pub(super) fn draw_bar(
    state: &mut AppState,
    ctx: &egui::Context,
    panel_rect: Rect,
    matches: usize,
    total: usize,
) {
    if !state.search_open {
        return;
    }

    let escape = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    if escape {
        state.close_search();
        return;
    }

    let pos = egui::pos2(
        panel_rect.right() - BAR_WIDTH - 22.0,
        panel_rect.top() + 6.0,
    );
    let mut close = false;
    let mut select_first = false;
    let edit_id = egui::Id::new("texture_search_input");

    egui::Area::new(egui::Id::new("texture_search_bar"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(COLOR_CARD_SURFACE)
                .stroke(egui::Stroke::new(1.0_f32, COLOR_BORDER_MEDIUM))
                .rounding(egui::Rounding::same(8.0))
                .shadow(egui::epaint::Shadow {
                    offset: egui::vec2(0.0, 4.0),
                    blur: 14.0,
                    spread: 0.0,
                    color: Color32::from_black_alpha(140),
                })
                .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(BAR_WIDTH - 20.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;

                        let (icon_rect, _) =
                            ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        paint_magnifier(ui, icon_rect, COLOR_TEXT_SECONDARY);

                        let edit = egui::TextEdit::singleline(&mut state.search_query)
                            .id(edit_id)
                            .hint_text("Find texture")
                            .frame(false)
                            .desired_width(BAR_WIDTH - 20.0 - 14.0 - 8.0 * 3.0 - 96.0)
                            .text_color(COLOR_TEXT);
                        let response = ui.add(edit);

                        if state.search_focus_pending {
                            state.search_focus_pending = false;
                            response.request_focus();
                            select_all(ctx, edit_id, &state.search_query);
                        }
                        if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
                        {
                            select_first = true;
                            response.request_focus();
                        }

                        let has_query = SearchQuery::parse(&state.search_query).is_some();
                        if has_query {
                            let (text, color) = if matches == 0 {
                                ("No results".to_string(), COLOR_ACCENT_ORANGE)
                            } else {
                                (format!("{matches} / {total}"), COLOR_TEXT_SECONDARY)
                            };
                            ui.label(egui::RichText::new(text).color(color).size(11.5));
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if close_button(ui).clicked() {
                                close = true;
                            }
                        });
                    });
                });
        });

    if close {
        state.close_search();
        return;
    }
    if select_first {
        select_first_match(state);
    }
}

fn select_all(ctx: &egui::Context, id: egui::Id, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    if let Some(mut edit_state) = egui::TextEdit::load_state(ctx, id) {
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(0),
            CCursor::new(text.chars().count()),
        )));
        edit_state.store(ctx, id);
    }
}

fn select_first_match(state: &mut AppState) {
    let Some(query) = SearchQuery::parse(&state.search_query) else {
        return;
    };
    let Some(tab) = state.active_tab() else {
        return;
    };
    let first = matching_indices(tab, &query).first().copied();
    if let Some(index) = first {
        if let Some(tab) = state.active_tab_mut() {
            tab.selected_index = Some(index);
        }
    }
}

fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter()
            .circle_filled(rect.center(), 9.0, Color32::from_white_alpha(22));
    }
    let color = if hovered {
        COLOR_TEXT
    } else {
        COLOR_TEXT_SECONDARY
    };
    let stroke = egui::Stroke::new(1.5_f32, color);
    let c = rect.center();
    let d = 3.5;
    ui.painter()
        .line_segment([c + egui::vec2(-d, -d), c + egui::vec2(d, d)], stroke);
    ui.painter()
        .line_segment([c + egui::vec2(-d, d), c + egui::vec2(d, -d)], stroke);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Close (Esc)")
}

fn paint_magnifier(ui: &egui::Ui, rect: Rect, color: Color32) {
    let stroke = egui::Stroke::new(1.5_f32, color);
    let center = rect.center() + egui::vec2(-1.5, -1.5);
    ui.painter().circle_stroke(center, 4.5, stroke);
    let from = center + egui::vec2(3.2, 3.2);
    ui.painter()
        .line_segment([from, from + egui::vec2(3.4, 3.4)], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_queries_do_not_filter() {
        assert_eq!(SearchQuery::parse(""), None);
        assert_eq!(SearchQuery::parse("   \t "), None);
    }

    #[test]
    fn matching_is_case_insensitive_substring() {
        let query = SearchQuery::parse("MESS").unwrap();
        assert!(query.matches("hud_message.dds"));
        assert!(query.matches("HUD_MESSAGE.DDS"));
        assert!(!query.matches("hud_map.dds"));
    }

    #[test]
    fn every_term_must_match_in_any_order() {
        let query = SearchQuery::parse("  msg   hud ").unwrap();
        assert!(query.matches("hud_msg.dds"));
        assert!(query.matches("msg_hud_icon.dds"));
        assert!(!query.matches("hud_icon.dds"));
    }

    #[test]
    fn needle_longer_than_name_never_matches() {
        let query = SearchQuery::parse("very_long_search_term").unwrap();
        assert!(!query.matches("a.dds"));
    }

    #[test]
    fn non_ascii_names_do_not_panic() {
        let query = SearchQuery::parse("tex").unwrap();
        assert!(query.matches("тест_tex.dds"));
        assert!(!query.matches("тест.dds"));
    }

    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use texembed::{DdsInfo, FileType, TextureFile};

    use super::super::state::FolderGallery;

    fn dds(name: &str, entry_index: usize) -> DdsInfo {
        DdsInfo {
            name: name.to_string(),
            bytes: Arc::from(vec![0_u8; 4].into_boxed_slice()),
            width: 4,
            height: 4,
            mipmap_count: 1,
            pixel_format: 0,
            catalog_index: 0,
            entry_index,
            modified: false,
        }
    }

    fn file(path: &str, names: &[&str]) -> TextureFile {
        TextureFile {
            file_type: FileType::Textures,
            path: PathBuf::from(path),
            dds_list: names
                .iter()
                .enumerate()
                .map(|(index, name)| dds(name, index))
                .collect(),
            original_bytes: Vec::new(),
            streamtex_layout: None,
        }
    }

    fn plain_tab(names: &[&str]) -> Tab {
        Tab {
            id: 1,
            file: file("one.textures", names),
            selected_index: None,
            has_unsaved_changes: false,
            show_full_preview: false,
            full_preview_zoom: 1.0,
            full_preview_pan: egui::Vec2::ZERO,
            replace_backups: HashMap::new(),
            folder_gallery: None,
        }
    }

    fn gallery_tab(include_multi_texture: bool) -> Tab {
        let files = vec![
            file(
                "a.textures",
                &["hud_message.dds", "hud_icon.dds", "map.dds"],
            ),
            file("b.textures", &["sky.dds", "cloud.dds"]),
            file("c.textures", &["HUD_Timer.dds"]),
        ];
        let mut index_map = Vec::new();
        let mut ranges = Vec::new();
        for (file_index, file) in files.iter().enumerate() {
            let start = index_map.len();
            for local in 0..file.dds_list.len() {
                index_map.push((file_index, local));
            }
            ranges.push(start..index_map.len());
        }
        let mut tab = plain_tab(&[]);
        tab.folder_gallery = Some(FolderGallery {
            file_expanded: vec![true; files.len()],
            files,
            index_map,
            file_global_ranges: ranges,
            include_multi_texture,
            is_folder: true,
        });
        tab
    }

    fn state_with(tab: Tab) -> AppState {
        AppState {
            active_tab_id: Some(tab.id),
            tabs: vec![tab],
            ..Default::default()
        }
    }

    fn run_frame(ctx: &egui::Context, state: &mut AppState) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            super::super::panels_shell::draw(state, ctx);
            super::super::panel_central::draw(state, ctx);
        });
    }

    #[test]
    fn matching_indices_cover_single_files_and_galleries() {
        let tab = plain_tab(&["hud_message.dds", "map.dds", "hud_icon.dds"]);
        let query = SearchQuery::parse("hud").unwrap();
        assert_eq!(matching_indices(&tab, &query), vec![0, 2]);

        let tab = gallery_tab(true);
        let query = SearchQuery::parse("hud").unwrap();
        // global indices: a = 0..3, b = 3..5, c = 5..6
        assert_eq!(matching_indices(&tab, &query), vec![0, 1, 5]);
        let none = SearchQuery::parse("zzz").unwrap();
        assert!(matching_indices(&tab, &none).is_empty());
    }

    #[test]
    fn enter_selects_the_first_match() {
        let mut state = state_with(gallery_tab(true));
        state.search_query = "cloud".to_string();
        select_first_match(&mut state);
        assert_eq!(state.active_tab().unwrap().selected_index, Some(4));
    }

    #[test]
    fn filtered_grids_render_without_panicking() {
        for tab in [
            plain_tab(&["hud_a.dds", "b.dds", "hud_c.dds"]),
            gallery_tab(true),
            gallery_tab(false),
        ] {
            let mut state = state_with(tab);
            let ctx = egui::Context::default();
            run_frame(&ctx, &mut state);
            for query in ["", "hud", "sky cloud", "cloud sky", "zzz-no-match"] {
                state.open_search();
                state.search_query = query.to_string();
                run_frame(&ctx, &mut state);
                run_frame(&ctx, &mut state);
            }
            state.close_search();
            assert!(!state.search_open);
            assert!(state.search_query.is_empty());
            run_frame(&ctx, &mut state);
        }
    }

    #[test]
    fn closing_the_search_clears_the_filter() {
        let mut state = state_with(plain_tab(&["a.dds"]));
        state.open_search();
        state.search_query = "a".to_string();
        assert!(state.search_focus_pending);
        state.close_search();
        assert!(!state.search_open && !state.search_focus_pending);
        assert!(state.search_query.is_empty());
    }
}
