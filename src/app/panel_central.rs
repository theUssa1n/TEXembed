use eframe::egui;
use egui::{Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, Sense, Vec2};

use super::constants::{
    COLOR_ACCENT_ORANGE, COLOR_BG, COLOR_BORDER_MEDIUM, COLOR_BORDER_SUBTLE, COLOR_BTN_BLUE,
    COLOR_CARD_SURFACE, COLOR_LABEL_BLUE, COLOR_PREVIEW_BG, COLOR_PRIMARY_BORDER,
    COLOR_PRIMARY_TRANSLUCENT, COLOR_TEXT, COLOR_TEXT_SECONDARY, THUMBNAIL_SIZE,
};
use super::grid_layout::{GridMetrics, GridRowKind, GroupedGridLayout};
use super::search::SearchQuery;
use super::state::{AboutSection, AppState, FolderGallery, PreviewState, Tab};
use super::widgets::{
    brighten_color, ghost_button, guide_card, paint_checkerboard, paint_texture_fitted,
};

pub(super) fn draw(state: &mut AppState, ctx: &egui::Context) {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::none()
                .fill(COLOR_BG)
                .inner_margin(egui::Margin::symmetric(16.0, 16.0)),
        )
        .show(ctx, |ui| {
            if state.show_about {
                set_thumbnail_demand(state, &[]);
                draw_about(state, ui);
                return;
            }

            if state.show_logs {
                set_thumbnail_demand(state, &[]);
                draw_logs(state, ui);
                return;
            }

            if state.show_tweaks {
                set_thumbnail_demand(state, &[]);
                draw_tweaks(state, ui);
                return;
            }

            if state.active_tab().is_some_and(|t| t.show_full_preview) {
                set_thumbnail_demand(state, &[]);
                draw_full_preview(state, ctx, ui);
                return;
            }

            draw_grid(state, ctx, ui);
        });
}

fn set_thumbnail_demand(state: &mut AppState, indices: &[usize]) {
    let tab_id = state.active_tab_id;
    let selected = if state.show_about || state.show_logs || state.show_tweaks {
        None
    } else {
        state.active_tab().and_then(|tab| {
            let index = tab.selected_index?;
            AppState::dds_by_index(tab, index)?;
            Some((tab.id, index))
        })
    };
    state.previews.set_visible_thumbnails(
        indices
            .iter()
            .filter_map(|index| tab_id.map(|id| (id, *index))),
        selected,
    );
    if let Some((tab_id, index)) = selected {
        state.enqueue_preview_load(tab_id, index);
    }
}

#[allow(clippy::too_many_lines)]
fn draw_about(state: &mut AppState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("About TEXembed")
                .color(COLOR_TEXT)
                .strong()
                .size(18.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, "BACK TO PREVIEWS", Vec2::new(132.0, 28.0)).clicked() {
                state.show_about = false;
            }
        });
    });
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        if about_tab_button(
            ui,
            "General Guide",
            state.about_section == AboutSection::Guide,
        )
        .clicked()
        {
            state.about_section = AboutSection::Guide;
        }
        if about_tab_button(
            ui,
            "Dithering doc",
            state.about_section == AboutSection::Argb,
        )
        .clicked()
        {
            state.about_section = AboutSection::Argb;
        }
    });
    ui.add_space(8.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Frame::none()
            .fill(COLOR_CARD_SURFACE)
            .stroke(egui::Stroke::new(1.0_f32, COLOR_BORDER_SUBTLE))
            .rounding(10.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());

                if state.about_section == AboutSection::Argb {
                    draw_about_argb(ui);
                    return;
                }

                ui.label(egui::RichText::new("TEXembed is an unofficial tool for the PC version of Split/Second - Velocity, designed to manage and extract .textures and .streamtex files. It loads these two file extensions, provides a preview of the DDS files inside them, and allows you to export them or replace them with new ones.").color(COLOR_TEXT).size(14.0));

                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new("Guide & Rules:")
                        .color(COLOR_LABEL_BLUE)
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(8.0);

                guide_card(ui, "1. Getting Started", |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Open your files by dragging and dropping them into the window, or use the Open buttons on the left panel. To view multiple files at once, use the Open folder view feature. When dragging multiple files, you can choose to open them in Shared view (combined) or Separate tabs.")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        )
                        .wrap(),
                    );
                });

                guide_card(ui, "2. Navigating Textures", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new("Click any texture in the grid to view its properties in the right Inspector panel. You can switch between open files using")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        );
                        ui.label(egui::RichText::new("Ctrl + Tab").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(
                            egui::RichText::new("and close the current one with")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        );
                        ui.label(egui::RichText::new("Ctrl + W").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new(".").color(COLOR_TEXT).size(14.0));
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new("Press")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        );
                        ui.label(egui::RichText::new("Ctrl + F").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(
                            egui::RichText::new("to search textures by name. The grid filters as you type, Enter selects the first match, and Esc closes the search.")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        );
                    });
                });

                guide_card(ui, "3. Inspecting Details", |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Click the preview image in the Inspector to open the Full Preview mode. Here, you can use your mouse wheel to zoom, drag to pan, and click Reset to restore the view. To check alpha channels, toggle the Transparency background checkbox.")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        )
                        .wrap(),
                    );
                });

                guide_card(ui, "4. Exporting", |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("You can export the selected texture as a DDS (and optionally PNG) from the Inspector, or use Export all as DDS to extract every texture in the current tab into a folder.")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        )
                        .wrap(),
                    );
                });

                guide_card(ui, "5. Replacing Textures (Strict Rules)", |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("To modify the game's textures, select an item and click Replace. Your new DDS file MUST meet these technical requirements:")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        )
                        .wrap(),
                    );

                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new("• Dimensions:").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new("The width and height must match the original texture exactly.").color(COLOR_TEXT).size(14.0));
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new("• Mipmaps:").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new("The number of mipmaps must be identical.").color(COLOR_TEXT).size(14.0));
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new("• Compression Format:").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new("The pixel format must match the original (e.g., BC1, BC3). Note: Obsolete DXT4 textures can be replaced directly with DXT5 (the tool will automatically normalize the format).").color(COLOR_TEXT).size(14.0));
                    });

                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Psst — you don't have to go through the file dialog every time. Just drag a DDS file straight onto the preview thumbnail in the Inspector and drop it there; it replaces the texture instantly, same as clicking Replace.")
                                .color(COLOR_TEXT_SECONDARY)
                                .italics()
                                .size(13.0),
                        )
                        .wrap(),
                    );
                });

                guide_card(ui, "6. Saving Your Work", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("Replacing a texture keeps the change in memory and reveals an Undo button if you make a mistake. Once you are satisfied, press").color(COLOR_TEXT).size(14.0));
                        ui.label(egui::RichText::new("Ctrl + S").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new("or click Save changes to write the modified container file to your disk.").color(COLOR_TEXT).size(14.0));
                    });
                });

                guide_card(ui, "7. Batch Replace", |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Instead of replacing textures one by one, open a file and use the Batch Replace... button in the Tools section of the Inspector panel. It matches every texture in the current file against a folder of DDS files and replaces them all at once.")
                                .color(COLOR_TEXT)
                                .size(14.0),
                        )
                        .wrap(),
                    );

                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new("• Replacement folder:").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new("Click Browse... and pick the folder that contains your replacement DDS files.").color(COLOR_TEXT).size(14.0));
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new("• Matching rule:").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new("Exact name matches by filename (recommended). Index matches by the texture's position, using files named 0.dds, 1.dds, and so on. Several entries can share one name; Export All suffixes those with _<index> (name_12.dds) and Batch Replace routes such files back to their exact entry.").color(COLOR_TEXT).size(14.0));
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new("• Scan Folder:").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new("Previews the matches before anything changes. Each row shows the entry's index, name and dimensions, plus the source file it matched with, so same-named entries stay distinguishable. Every texture shows whether it's ready, mismatched (wrong dimensions/mipmaps/format), or has no matching file, and any leftover files are listed as unused.").color(COLOR_TEXT).size(14.0));
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new("• Apply Matched:").color(COLOR_TEXT).strong().size(14.0));
                        ui.label(egui::RichText::new("Replaces every ready texture in one step, still in memory. The same Undo and Save changes workflow from Section 6 applies afterwards.").color(COLOR_TEXT).size(14.0));
                    });

                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Psst — the same technical requirements from Section 5 (matching dimensions, mipmaps, and compression format) still apply to every file in the batch; mismatched ones are simply skipped and reported instead of applied.")
                                .color(COLOR_TEXT_SECONDARY)
                                .italics()
                                .size(13.0),
                        )
                        .wrap(),
                    );
                });

                ui.add_space(6.0);
                ui.label(egui::RichText::new("Need help? Check the Logs panel for detailed operations and error messages.").color(COLOR_TEXT_SECONDARY).size(13.0));

                ui.add_space(24.0);
                ui.label(
                    egui::RichText::new("CREDITS")
                        .color(Color32::from_rgb(255, 215, 0))
                        .size(18.0)
                        .strong(),
                );
                ui.label(egui::RichText::new("SPECIAL THANKS TO :").color(COLOR_TEXT_SECONDARY).size(14.0));

                ui.add_space(10.0);

                ui.label(egui::RichText::new("Lahvuun").color(COLOR_TEXT).size(16.0).strong());
                let (name_line_rect, _) =
                    ui.allocate_exact_size(Vec2::new(160.0, 6.0), Sense::hover());
                ui.painter().line_segment(
                    [
                        Pos2::new(name_line_rect.left(), name_line_rect.center().y),
                        Pos2::new(name_line_rect.right(), name_line_rect.center().y),
                    ],
                    egui::Stroke::new(2.0_f32, COLOR_LABEL_BLUE),
                );
                ui.label(
                    egui::RichText::new("Released the base source for texture analysis and extraction.")
                        .color(COLOR_TEXT_SECONDARY)
                        .size(13.0),
                );

                ui.add_space(12.0);

                ui.label(egui::RichText::new("Selter").color(COLOR_TEXT).size(16.0).strong());
                let (name_line_rect, _) =
                    ui.allocate_exact_size(Vec2::new(160.0, 6.0), Sense::hover());
                ui.painter().line_segment(
                    [
                        Pos2::new(name_line_rect.left(), name_line_rect.center().y),
                        Pos2::new(name_line_rect.right(), name_line_rect.center().y),
                    ],
                    egui::Stroke::new(2.0_f32, COLOR_ACCENT_ORANGE),
                );
                ui.label(
                    egui::RichText::new("Tool testing and lot of suggestions for improvement and optimization.")
                        .color(COLOR_TEXT_SECONDARY)
                        .size(13.0),
                );
            });
    });
}

fn about_tab_button(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let (fill, stroke, text_color) = if active {
        (
            COLOR_PRIMARY_TRANSLUCENT,
            egui::Stroke::new(1.0_f32, COLOR_PRIMARY_BORDER),
            COLOR_TEXT,
        )
    } else {
        (
            Color32::from_white_alpha(10),
            egui::Stroke::new(1.0_f32, COLOR_BORDER_MEDIUM),
            COLOR_TEXT_SECONDARY,
        )
    };
    let inner = egui::Frame::none()
        .fill(fill)
        .stroke(stroke)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(12.0, 5.0))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(label)
                        .color(text_color)
                        .strong()
                        .size(12.0),
                )
                .selectable(false),
            );
        });
    // The frame itself does not sense clicks, so register a clickable region
    // over the exact rectangle it occupies.
    ui.interact(
        inner.response.rect,
        ui.id().with(("about_tab", label)),
        egui::Sense::click(),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
}

#[allow(clippy::too_many_lines)]
fn draw_about_argb(ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new("Dithering doc")
            .color(COLOR_LABEL_BLUE)
            .size(18.0)
            .strong(),
    );
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("Some color schemes look grainy or banded on gradients, because they are stored with a compressed format (the main one is BC1 / DXT1). Using a different format — uncompressed A8R8G8B8 (ARGB) — removes that dithering, so the gradients show up clear and smooth.")
                .color(COLOR_TEXT)
                .size(14.0),
        )
        .wrap(),
    );
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.add_space(16.0);
        ui.label(
            egui::RichText::new("• To fix it, turn on the .textures option in the Tweaks panel (the last button on the left). It lets you put your own DDS design into that .textures texture even if its compression is different.")
                .color(COLOR_TEXT)
                .size(14.0),
        );
    });
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.add_space(16.0);
        ui.label(
            egui::RichText::new("• Keep in mind this only applies to .textures files. For .streamtex there are separate checkboxes, explained in the same Tweaks panel.")
                .color(COLOR_TEXT)
                .size(14.0),
        );
    });

    guide_card(ui, "File structure (the knowledge)", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "• .streamtex: a plain sequence of records, each [u32 length][full DDS].",
                )
                .color(COLOR_TEXT)
                .size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("• .textures (SXET container): header + catalog table + 12-byte catalog entries + static records. The streamed catalog (tex_data_size == 0) describes records stored in the external .streamtex file.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("• Each 12-byte entry: [record offset][pointer][data_off]. For streamed entries, data_off is the ABSOLUTE offset of the record's length prefix inside the .streamtex.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("• Static record fields: pf @ +12 (0x21 = DXT1, 0x24 = DXT4/DXT5, 0x02 = A8R8G8B8), mips @ +20, width @ +24, height @ +28.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
    });

    guide_card(ui, "The two rules that break things", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("1. Records are located by their absolute data_off — the engine does not walk the file sequentially. When one record changes size, every later record shifts; if the catalog offsets are stale, the game reads the wrong data (for example a black shadow rectangle).")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("2. The menu's streamed loader only accepts uncompressed records with a single mip level (mip 0). A full mip chain makes the vehicle fail to render entirely — no crash, the car just does not appear.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
    });

    guide_card(ui, "Use SSDC v2", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("Make sure you use the SSDC v2 build. In this build, an ARGB-compressed paint is displayed without any problem — no extra patch is needed.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("The same ~6 MB cap applies to frontend and UI .streamtex files: the game sets a fixed limit for them, and a .streamtex larger than that cannot be handled and will crash the game.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });

        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Psst — the two .streamtex options are completely optional and are mainly placed for testing. If you are not sure how to use them correctly, there is no need to enable them.")
                    .color(COLOR_TEXT_SECONDARY)
                    .italics()
                    .size(13.0),
            )
            .wrap(),
        );
    });

    guide_card(ui, "How TEXembed automates this", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("• Allow different compression & size: lets a .streamtex record be replaced with any format or size and rewrites the record length prefix on save. Uncompressed (A8R8G8B8) replacements are reduced to mip 0 (mips=1).")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("• Auto-patch paired .textures on save: after saving a .streamtex, the sibling .textures file is updated — the streamed-catalog data_off offsets are recomputed from the new record layout and the records' pf/mips/width/height are synced from the new DDS headers.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });

        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Psst — both options now live in the Tweaks panel (the last button on the left). The auto-patch only works when the same-named .textures sits in its proper folder next to the .streamtex; do not use the size-changing option on a standalone .streamtex that has no paired .textures.")
                    .color(COLOR_TEXT_SECONDARY)
                    .italics()
                    .size(13.0),
            )
            .wrap(),
        );
    });

    guide_card(
        ui,
        "Related: changing the compression of an embedded .textures texture",
        |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("By default, replacing a texture requires an identical compression format (for example BC1 with BC1). The \"Allow different compression for .textures\" option — the first card in the Tweaks panel (the last button on the left) — relaxes that rule for .textures files: it lets you swap in a different compression, such as ARGB (uncompressed) or any other format the game supports (for example BC1 to BC3), and on save the catalog sizes and the static records are patched automatically so the game loads the new format. It is off by default.")
                        .color(COLOR_TEXT)
                        .size(14.0),
                )
                .wrap(),
            );

            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(16.0);
                ui.label(egui::RichText::new("• What is relaxed:").color(COLOR_TEXT).strong().size(14.0));
                ui.label(egui::RichText::new("the compression format may differ from the original. The width, height and mip count must still match exactly.").color(COLOR_TEXT).size(14.0));
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(16.0);
                ui.label(egui::RichText::new("• How to use it:").color(COLOR_TEXT).strong().size(14.0));
                ui.label(egui::RichText::new("open a .textures file, select a texture, tick the checkbox, then Replace with the new-format DDS (or run Batch Replace with the checkbox enabled) and save with Ctrl + S. A log note tells you when the header is patched.").color(COLOR_TEXT).size(14.0));
            });

            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Psst — this option only applies to .textures files; the .streamtex options in the Tweaks panel (the last button on the left) handle .streamtex records instead.")
                        .color(COLOR_TEXT_SECONDARY)
                        .italics()
                        .size(13.0),
                )
                .wrap(),
            );
        },
    );

    guide_card(ui, "Step-by-step workflow", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("1. Make sure you use the SSDC v2 build (see above).")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("2. Open the bodypaint.textures of the car you want.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("3. In the Tweaks panel (the last button on the left), turn on the .textures option.").color(COLOR_TEXT).size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("4. Select the body-paint texture and replace it with your A8R8G8B8 (ARGB) DDS using Replace or drag & drop.").color(COLOR_TEXT).size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "5. Save with Ctrl+S. The log tells you when the header is patched.",
                )
                .color(COLOR_TEXT)
                .size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("6. Launch the game and check the car.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
    });

    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("Psst — keep the same width, height and mip count as the original texture; only the compression format may differ in this workflow.")
                .color(COLOR_TEXT_SECONDARY)
                .italics()
                .size(13.0),
        )
        .wrap(),
    );
}

fn draw_logs(state: &mut AppState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Logs")
                .color(COLOR_TEXT)
                .strong()
                .size(18.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, "CLEAR", Vec2::new(70.0, 28.0)).clicked() {
                state.logs.clear();
            }
            if ghost_button(ui, "BACK TO PREVIEWS", Vec2::new(132.0, 28.0)).clicked() {
                state.show_logs = false;
            }
        });
    });
    ui.add_space(12.0);

    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .show(ui, |ui| {
            egui::Frame::none()
                .fill(COLOR_CARD_SURFACE)
                .stroke(egui::Stroke::new(1.0_f32, COLOR_BORDER_SUBTLE))
                .rounding(10.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    for line in &state.logs {
                        ui.label(egui::RichText::new(line).color(COLOR_TEXT).monospace());
                    }
                });
        });
}

fn draw_tweaks(state: &mut AppState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Tweaks")
                .color(COLOR_TEXT)
                .strong()
                .size(18.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, "BACK TO PREVIEWS", Vec2::new(132.0, 28.0)).clicked() {
                state.show_tweaks = false;
            }
        });
    });
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(
                "These are advanced switches that change how TEXembed checks and rewrites textures when you replace and save. They are all OFF by default — leaving them off is the safest choice. Turn one on only when you actually need it and you understand what it does.",
            )
            .color(COLOR_TEXT_SECONDARY)
            .size(13.0),
        )
        .wrap(),
    );
    ui.add_space(12.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Frame::none()
            .fill(COLOR_CARD_SURFACE)
            .stroke(egui::Stroke::new(1.0_f32, COLOR_BORDER_SUBTLE))
            .rounding(10.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());

                tweak_card(
                    ui,
                    &mut state.patch_header_on_replace,
                    "Allow different compression for .textures (patch header on save)",
                    "Normally, when you replace a texture inside a .textures file, the new DDS must use exactly the same compression as the original (for example BC1 stays BC1). Turn this on to allow a different compression for the replacement — such as ARGB (uncompressed) or any other format the game supports, for example BC1 to BC3. TEXembed then rewrites the file's internal header and records on save so the game still reads it correctly.",
                    "Leave this OFF unless a replacement keeps getting skipped for a compression mismatch. The width, height and mip count must still match the original exactly — this only relaxes the compression format.",
                );

                tweak_card(
                    ui,
                    &mut state.resize_on_replace,
                    "Allow a different size and mip count on replace (patch the record on save)",
                    "Normally a replacement must have exactly the same width, height and mip count as the texture you are replacing. Turn this on to allow a completely different size and mip count instead (for example a 2048x2048 with a full mip chain in place of a 1024x1024 single mip). This applies to the items inside the .textures you opened, and also to .streamtex records. On save TEXembed rewrites the file's own header — for a .textures that means the record that sits before the DDS (format, mips, width, height) and the catalog sizes; for a .streamtex it means the record's length prefix. The DDS header is stored as-is, so whatever format and dimensions you bring are used.",
                    "Leave this OFF for normal edits and keep the original size and mips. Use it only when you really need a different resolution, and keep the paired-file options in mind for .streamtex: a standalone .streamtex with no same-named .textures next to it should not be resized, because the record locations cannot be fixed. If you are not sure you need this, keep it off. Important: this option is not recommended at all for volumetric (3D) textures — the ones that have depth, not just width and height. Their depth cannot be resized, so a mismatched replacement is simply rejected.",
                );

                tweak_card(
                    ui,
                    &mut state.streamtex_allow_resize,
                    "Allow different compression and size for .streamtex records",
                    "The same idea as the .textures option above, but for records inside a .streamtex: it lets you swap in a different compression (for example DXT1 to A8R8G8B8) and also a different size. The new image is stored as-is and the record's length value is rewritten when you save. When the paired .textures file sits next to it with the same name, that file is updated automatically too, so the two stay in sync. Uncompressed (ARGB) replacements are reduced to mip 0 (a single mip level), because the game's menu only accepts that form.",
                    "Keep this OFF unless you deliberately need it — it is fully optional and mainly meant for testing. Do not use it on a standalone .streamtex whose same-named .textures is not in its proper folder next to it: without that file the record locations cannot be fixed and the game will read broken data. If you are not sure how to use it correctly, there is no need to enable it.",
                );

                tweak_card(
                    ui,
                    &mut state.streamtex_patch_sidecar,
                    "Auto-patch the paired .textures when saving a .streamtex",
                    "A .streamtex is paired with a sibling .textures that stores where each record lives. When you change a record's size, every later record shifts, so those stored locations become stale and the game reads the wrong data. Turn this on and, after saving the .streamtex, TEXembed updates the paired .textures too — it recomputes the record offsets and syncs each record's format, mips, width and height from the new DDS headers.",
                    "Keep this ON together with the option above whenever you change a record's size, otherwise the game may show wrong or broken textures. If you only replace records with the identical size, this is not needed.",
                );

                ui.add_space(4.0);
                egui::Frame::none()
                    .fill(COLOR_CARD_SURFACE)
                    .stroke(egui::Stroke::new(1.0_f32, COLOR_BORDER_SUBTLE))
                    .rounding(egui::Rounding::same(10.0))
                    .inner_margin(egui::Margin::symmetric(14.0, 12.0))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("Note — frontend / UI .streamtex")
                                .color(COLOR_LABEL_BLUE)
                                .strong()
                                .size(14.0),
                        );
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "The game sets a fixed cap for these: a frontend or UI .streamtex larger than about 6 MB cannot be handled and will crash the game. Both .streamtex options above are completely optional and are mainly here for testing — if you are not sure how to use them correctly, simply leave them off.",
                                )
                                .color(COLOR_TEXT_SECONDARY)
                                .size(13.0),
                            )
                            .wrap(),
                        );
                    });
            });
    });
}

#[allow(clippy::too_many_arguments)]
fn tweak_card(ui: &mut egui::Ui, value: &mut bool, title: &str, plain: &str, safe: &str) {
    egui::Frame::none()
        .fill(COLOR_CARD_SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, COLOR_BORDER_SUBTLE))
        .rounding(egui::Rounding::same(10.0))
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.checkbox(
                    value,
                    egui::RichText::new(title)
                        .color(COLOR_TEXT)
                        .strong()
                        .size(15.0),
                );
            });
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(egui::RichText::new(plain).color(COLOR_TEXT).size(14.0)).wrap(),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Safest use:")
                        .color(COLOR_LABEL_BLUE)
                        .strong()
                        .size(13.0),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(safe)
                            .color(COLOR_TEXT_SECONDARY)
                            .size(13.0),
                    )
                    .wrap(),
                );
            });
        });
    ui.add_space(12.0);
}

fn paint_unsaved_badge(ui: &egui::Ui, rect: Rect) {
    let painter = ui.painter();
    let center = Pos2::new(rect.right() - 11.0, rect.top() + 11.0);

    painter.circle_filled(center, 7.0, Color32::from_black_alpha(150));
    painter.circle_filled(center, 4.5, COLOR_ACCENT_ORANGE);
}

fn draw_texture_card(
    ui: &mut egui::Ui,
    state: &AppState,
    tab_id: usize,
    index: usize,
    card_size: f32,
    is_selected: bool,
    is_unsaved: bool,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(card_size), Sense::hover());
    let response = ui.interact(
        rect,
        ui.id().with(("texture_card", tab_id, index)),
        Sense::click(),
    );

    if !ui.clip_rect().intersects(rect) {
        return response.on_hover_cursor(CursorIcon::PointingHand);
    }

    let mut response = response.on_hover_cursor(CursorIcon::PointingHand);
    if is_unsaved {
        response = response.on_hover_text("Modified - not saved yet");
    }

    let painter = ui.painter();
    let is_hovered = response.hovered();
    let draw_rect = rect;
    let rounding = egui::Rounding::same(4.0);

    if is_hovered {
        let shadow_rect = draw_rect.translate(Vec2::new(0.0, 3.0));
        painter.rect_filled(shadow_rect, rounding, Color32::from_black_alpha(90));
    }

    let bg = if is_hovered {
        brighten_color(COLOR_PREVIEW_BG, 1.14)
    } else if is_selected {
        brighten_color(COLOR_PREVIEW_BG, 1.08)
    } else {
        COLOR_PREVIEW_BG
    };
    painter.rect_filled(draw_rect, rounding, bg);
    painter.rect_stroke(
        draw_rect,
        rounding,
        egui::Stroke::new(1.0_f32, COLOR_BORDER_SUBTLE),
    );

    if state.show_transparency_bg {
        paint_checkerboard(
            ui,
            draw_rect.shrink(4.0),
            10.0,
            Color32::from_rgb(38, 38, 42),
            Color32::from_rgb(26, 26, 30),
        );
    }
    if let Some(PreviewState::Ready(texture)) = state.previews.get(&(tab_id, index)) {
        paint_texture_fitted(ui, draw_rect.shrink(4.0), texture);
    }

    if is_unsaved {
        painter.rect_stroke(
            draw_rect.shrink(1.0),
            rounding,
            egui::Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(
                    COLOR_ACCENT_ORANGE.r(),
                    COLOR_ACCENT_ORANGE.g(),
                    COLOR_ACCENT_ORANGE.b(),
                    170,
                ),
            ),
        );
        paint_unsaved_badge(ui, draw_rect);
    }

    if is_selected {
        painter.rect_stroke(
            draw_rect,
            rounding,
            egui::Stroke::new(2.0_f32, COLOR_BTN_BLUE),
        );
    } else if is_hovered {
        painter.rect_stroke(
            draw_rect,
            rounding,
            egui::Stroke::new(1.5_f32, COLOR_PRIMARY_BORDER),
        );
    }

    response
}

#[allow(clippy::too_many_lines, clippy::float_cmp)]
fn draw_full_preview(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(tab) = state.active_tab() else {
        return;
    };
    let tab_id = tab.id;
    let Some(index) = tab.selected_index else {
        if let Some(tab) = state.active_tab_mut() {
            tab.show_full_preview = false;
        }
        return;
    };

    let mut show_full_preview = tab.show_full_preview;
    let mut full_preview_zoom = tab.full_preview_zoom;
    let mut full_preview_pan = tab.full_preview_pan;

    state.enqueue_full_preview_load(tab_id, index);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Full Preview")
                .color(COLOR_TEXT)
                .strong()
                .size(18.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ghost_button(ui, "BACK", Vec2::new(64.0, 28.0)).clicked() {
                show_full_preview = false;
            }
            if ghost_button(ui, "RESET", Vec2::new(76.0, 28.0)).clicked() {
                full_preview_zoom = 1.0;
                full_preview_pan = Vec2::ZERO;
            }
            if ghost_button(ui, "−", Vec2::new(34.0, 28.0)).clicked() {
                full_preview_zoom = (full_preview_zoom / 1.15).clamp(0.1, 20.0);
            }
            if ghost_button(ui, "+", Vec2::new(34.0, 28.0)).clicked() {
                full_preview_zoom = (full_preview_zoom * 1.15).clamp(0.1, 20.0);
            }
        });
    });
    ui.add_space(12.0);

    let avail = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(avail, Sense::drag());
    let response = response.on_hover_cursor(CursorIcon::Grab);

    ui.painter()
        .rect_filled(rect, egui::Rounding::same(10.0), COLOR_PREVIEW_BG);
    ui.painter().rect_stroke(
        rect,
        egui::Rounding::same(10.0),
        egui::Stroke::new(1.0_f32, COLOR_BORDER_SUBTLE),
    );
    let content_rect = rect.shrink(8.0);
    if state.show_transparency_bg {
        paint_checkerboard(
            ui,
            content_rect,
            16.0,
            Color32::from_rgb(38, 38, 42),
            Color32::from_rgb(26, 26, 30),
        );
    }

    if response.dragged() {
        let delta = ctx.input(|i| i.pointer.delta());
        full_preview_pan += delta;
        ctx.request_repaint();
    }

    if response.hovered() {
        let zoom_delta = ctx.input(egui::InputState::zoom_delta);
        if zoom_delta != 1.0 {
            if let Some(mouse_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                let content_center = content_rect.center();
                let z_old = full_preview_zoom;
                let z_new = (z_old * zoom_delta).clamp(0.1, 20.0);

                if z_old != 0.0 && (content_rect.width() > 0.0 && content_rect.height() > 0.0) {
                    if let Some(PreviewState::Ready(texture)) =
                        state.previews.get_full(&(tab_id, index))
                    {
                        let tex_size = texture.size_vec2();
                        if tex_size.x > 0.0 && tex_size.y > 0.0 {
                            let base_scale = (content_rect.width() / tex_size.x)
                                .min(content_rect.height() / tex_size.y);
                            if base_scale > 0.0 {
                                let screen_point = mouse_pos;
                                let pan_old = full_preview_pan;
                                let tex_rel = (screen_point - content_center - pan_old)
                                    / (base_scale * z_old);
                                let pan_new =
                                    screen_point - content_center - tex_rel * base_scale * z_new;
                                full_preview_zoom = z_new;
                                full_preview_pan = pan_new;
                            } else {
                                full_preview_zoom = z_new;
                            }
                        } else {
                            full_preview_zoom = z_new;
                        }
                    } else {
                        full_preview_zoom = z_new;
                    }

                    ctx.request_repaint();
                } else {
                    full_preview_zoom = z_new;
                    ctx.request_repaint();
                }
            }
        }

        let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            if let Some(mouse_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                let content_center = content_rect.center();
                let z_old = full_preview_zoom;
                let factor = (1.0 + (wheel / 400.0)).max(0.1);
                let z_new = (z_old * factor).clamp(0.1, 20.0);

                if z_old != 0.0 {
                    if let Some(PreviewState::Ready(texture)) =
                        state.previews.get_full(&(tab_id, index))
                    {
                        let tex_size = texture.size_vec2();
                        if tex_size.x > 0.0 && tex_size.y > 0.0 {
                            let base_scale = (content_rect.width() / tex_size.x)
                                .min(content_rect.height() / tex_size.y);
                            if base_scale > 0.0 {
                                let screen_point = mouse_pos;
                                let pan_old = full_preview_pan;
                                let tex_rel = (screen_point - content_center - pan_old)
                                    / (base_scale * z_old);
                                let pan_new =
                                    screen_point - content_center - tex_rel * base_scale * z_new;
                                full_preview_zoom = z_new;
                                full_preview_pan = pan_new;
                            } else {
                                full_preview_zoom = z_new;
                            }
                        } else {
                            full_preview_zoom = z_new;
                        }
                    } else {
                        full_preview_zoom = z_new;
                    }

                    ctx.input_mut(|i| {
                        i.smooth_scroll_delta.y = 0.0;
                        i.raw_scroll_delta.y = 0.0;
                    });
                    ctx.request_repaint();
                }
            }
        }
    }

    let preview_state = state.previews.get_full(&(tab_id, index));
    match preview_state {
        Some(PreviewState::Ready(texture)) => {
            let tex_size = texture.size_vec2();
            if tex_size.x > 0.0
                && tex_size.y > 0.0
                && content_rect.width() > 0.0
                && content_rect.height() > 0.0
            {
                let base_scale =
                    (content_rect.width() / tex_size.x).min(content_rect.height() / tex_size.y);
                let draw_size = tex_size * (base_scale * full_preview_zoom);
                let max_pan_x = ((draw_size.x - content_rect.width()) * 0.5).max(0.0);
                let max_pan_y = ((draw_size.y - content_rect.height()) * 0.5).max(0.0);
                full_preview_pan.x = full_preview_pan.x.clamp(-max_pan_x, max_pan_x);
                full_preview_pan.y = full_preview_pan.y.clamp(-max_pan_y, max_pan_y);
                let draw_rect =
                    Rect::from_center_size(content_rect.center() + full_preview_pan, draw_size);
                let painter = ui.painter().with_clip_rect(content_rect);
                painter.image(
                    texture.id(),
                    draw_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        preview => {
            let message = if preview.is_none() {
                "Loading preview..."
            } else {
                "Preview\nnot\navailable"
            };
            ui.painter().text(
                content_rect.center(),
                egui::Align2::CENTER_CENTER,
                message,
                FontId::new(26.0, FontFamily::Proportional),
                Color32::WHITE,
            );
        }
    }

    if let Some(tab) = state.active_tab_mut() {
        tab.show_full_preview = show_full_preview;
        tab.full_preview_zoom = full_preview_zoom;
        tab.full_preview_pan = full_preview_pan;
    }
    if !show_full_preview {
        state.previews.retain_full(None);
    }
}

#[derive(Default)]
struct GridActions {
    enqueue: Vec<usize>,
    clicked: Option<usize>,
    toggle_files: Vec<usize>,
}

fn draw_grid_cards(
    ui: &mut egui::Ui,
    state: &AppState,
    tab: &Tab,
    indices: impl IntoIterator<Item = usize>,
    actions: &mut GridActions,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for index in indices {
            let response = draw_texture_card(
                ui,
                state,
                tab.id,
                index,
                THUMBNAIL_SIZE,
                tab.selected_index == Some(index),
                AppState::is_index_unsaved(tab, index),
            );
            if ui.clip_rect().intersects(response.rect) {
                actions.enqueue.push(index);
            }
            if response.clicked() {
                actions.clicked = Some(index);
            }
        }
    });
}

fn draw_group_header(ui: &mut egui::Ui, file_name: &str, expanded: bool, height: f32) -> bool {
    let origin = ui.max_rect().min;
    let button_height = height - 6.0;
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let arrow = if expanded { "v" } else { ">" };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(22.0, button_height), Sense::hover());
        let response = ui
            .interact(rect, ui.id().with("file_toggle"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            visuals.rounding,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            arrow,
            FontId::new(13.0, FontFamily::Proportional),
            visuals.fg_stroke.color,
        );
        clicked = response.clicked();
        ui.add(
            egui::Label::new(egui::RichText::new(file_name).color(COLOR_TEXT).size(13.0))
                .truncate()
                .sense(Sense::hover()),
        )
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(file_name);
    });
    let line = Rect::from_min_size(
        origin + Vec2::new(0.0, button_height),
        Vec2::new(ui.max_rect().width(), 1.0),
    );
    ui.painter().rect_filled(line, 0.0, COLOR_BORDER_SUBTLE);
    clicked
}

fn draw_grouped_grid(
    ui: &mut egui::Ui,
    state: &AppState,
    tab: &Tab,
    gallery: &FolderGallery,
    filter: Option<&[usize]>,
    actions: &mut GridActions,
) {
    egui::ScrollArea::vertical()
        .id_salt(("textures_scroll", tab.id))
        .show_viewport(ui, |ui, viewport| {
            let spacing = 5.0;
            let width = ui.available_width();
            let columns = ((width + spacing) / (THUMBNAIL_SIZE + spacing)).floor() as usize;
            let metrics = GridMetrics {
                columns,
                card_height: THUMBNAIL_SIZE,
                row_gap: spacing,
                header_height: ui.spacing().interact_size.y.max(22.0) + 6.0,
                section_gap: spacing,
            };

            // `section_files[n]` is the file shown by layout section `n`. While a
            // search is active only files with matches get a section, their cards
            // are positions in the sorted `found` list, and all of them are
            // expanded so matches are never hidden behind a collapsed header.
            let mut section_files: Vec<usize> = Vec::new();
            let mut sections: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
            for (file_index, range) in gallery.file_global_ranges.iter().enumerate() {
                match filter {
                    None => {
                        let expanded = gallery
                            .file_expanded
                            .get(file_index)
                            .copied()
                            .unwrap_or(true);
                        section_files.push(file_index);
                        sections.push((range.clone(), expanded));
                    }
                    Some(found) => {
                        let first = found.partition_point(|&index| index < range.start);
                        let last = found.partition_point(|&index| index < range.end);
                        if last > first {
                            section_files.push(file_index);
                            sections.push((first..last, true));
                        }
                    }
                }
            }
            let layout = GroupedGridLayout::new(sections, metrics);
            ui.set_width(width);
            ui.set_height(layout.height());
            let origin = ui.max_rect().min;
            for row in layout.visible_rows(viewport.min.y..viewport.max.y) {
                let rect = Rect::from_min_size(
                    origin + Vec2::new(0.0, row.top),
                    Vec2::new(width, row.height),
                );
                let (section, row_id) = match &row.kind {
                    GridRowKind::Header { section } => (*section, None),
                    GridRowKind::Cards { section, indices } => (*section, Some(indices.start)),
                };
                let mut row_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt(("grouped_row", tab.id, section, row_id))
                        .max_rect(rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                match row.kind {
                    GridRowKind::Header { section } => {
                        let file_index = section_files[section];
                        let file = &gallery.files[file_index];
                        let file_name = file.path.file_name().unwrap_or_default().to_string_lossy();
                        let expanded = filter.is_some()
                            || gallery
                                .file_expanded
                                .get(file_index)
                                .copied()
                                .unwrap_or(true);
                        if draw_group_header(&mut row_ui, &file_name, expanded, row.height)
                            && filter.is_none()
                        {
                            actions.toggle_files.push(file_index);
                        }
                    }
                    GridRowKind::Cards { indices, .. } => {
                        let cards: Vec<usize> = match filter {
                            Some(found) => found[indices].to_vec(),
                            None => indices.collect(),
                        };
                        draw_grid_cards(&mut row_ui, state, tab, cards, actions);
                    }
                }
            }
        });
}

fn draw_grid(state: &mut AppState, ctx: &egui::Context, ui: &mut egui::Ui) {
    let panel_rect = ui.max_rect();
    let Some(tab) = state.active_tab() else {
        set_thumbnail_demand(state, &[]);
        ui.centered_and_justified(|ui| {
            let response = ui.label(
                egui::RichText::new("Open a file or drop it here to view textures.")
                    .color(COLOR_TEXT_SECONDARY)
                    .size(18.0),
            );
            response.on_hover_cursor(egui::CursorIcon::Default);
        });
        return;
    };

    let tab_id = tab.id;
    let mut actions = GridActions::default();
    let total = tab
        .folder_gallery
        .as_ref()
        .map_or(tab.file.dds_list.len(), |gallery| gallery.index_map.len());
    let query = if state.search_open {
        SearchQuery::parse(&state.search_query)
    } else {
        None
    };
    let filter: Option<Vec<usize>> = query
        .as_ref()
        .map(|query| super::search::matching_indices(tab, query));
    let match_count = filter.as_ref().map_or(total, Vec::len);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if filter.as_ref().is_some_and(Vec::is_empty) {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "No textures match \"{}\"",
                        state.search_query.trim()
                    ))
                    .color(COLOR_TEXT_SECONDARY)
                    .size(16.0),
                );
            });
            return;
        }
        if let Some(gallery) = tab
            .folder_gallery
            .as_ref()
            .filter(|gallery| gallery.include_multi_texture)
        {
            draw_grouped_grid(ui, state, tab, gallery, filter.as_deref(), &mut actions);
            return;
        }

        let shown = filter.as_ref().map_or(total, Vec::len);
        let spacing = 5.0;
        let columns = (((ui.available_width() + spacing) / (THUMBNAIL_SIZE + spacing)).floor()
            as usize)
            .max(1);
        let row_count = shown.div_ceil(columns);
        egui::ScrollArea::vertical()
            .id_salt(("textures_scroll", tab_id))
            .show_rows(ui, THUMBNAIL_SIZE + spacing, row_count, |ui, rows| {
                for row in rows {
                    let start = row * columns;
                    let end = start.saturating_add(columns).min(shown);
                    let cards: Vec<usize> = match &filter {
                        Some(found) => found[start..end].to_vec(),
                        None => (start..end).collect(),
                    };
                    draw_grid_cards(ui, state, tab, cards, &mut actions);
                    ui.add_space(spacing);
                }
            });
    });

    if let Some(t) = state.active_tab_mut() {
        if let Some(gallery) = t.folder_gallery.as_mut() {
            for file_idx in actions.toggle_files {
                if let Some(v) = gallery.file_expanded.get_mut(file_idx) {
                    *v = !*v;
                }
            }
        }
    }

    if let Some(index) = actions.clicked {
        if let Some(t) = state.active_tab_mut() {
            t.selected_index = Some(index);
        }
    }
    set_thumbnail_demand(state, &actions.enqueue);
    for index in actions.enqueue {
        state.enqueue_preview_load(tab_id, index);
    }
    super::search::draw_bar(state, ctx, panel_rect, match_count, total);
}
