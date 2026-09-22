use eframe::egui;
use egui::{Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, Sense, Vec2};

use super::constants::{
    COLOR_ACCENT_ORANGE, COLOR_ACCENT_RED, COLOR_BG, COLOR_CARD, COLOR_CARD_SELECTED,
    COLOR_INSPECTOR_BOX, COLOR_LABEL_BLUE, COLOR_TEXT, COLOR_TEXT_SECONDARY, THUMBNAIL_SIZE,
};
use super::state::{AboutSection, AppState, PreviewState};
use super::widgets::{brighten_color, guide_card, paint_checkerboard, paint_texture_fitted};

pub(super) fn draw(state: &mut AppState, ctx: &egui::Context) {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::none()
                .fill(COLOR_BG)
                .inner_margin(egui::Margin::symmetric(16.0, 16.0)),
        )
        .show(ctx, |ui| {
            if state.show_about {
                draw_about(state, ui);
                return;
            }

            if state.show_logs {
                draw_logs(state, ui);
                return;
            }

            if state.active_tab().is_some_and(|t| t.show_full_preview) {
                draw_full_preview(state, ctx, ui);
                return;
            }

            draw_grid(state, ctx, ui);
        });
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
            if ui.button("Back to Previews").clicked() {
                state.show_about = false;
            }
        });
    });
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        if about_tab_button(ui, "General Guide", state.about_section == AboutSection::Guide)
            .clicked()
        {
            state.about_section = AboutSection::Guide;
        }
        if about_tab_button(
            ui,
            "Split/Second: ARGB Body Paint",
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
            .fill(COLOR_CARD)
            .rounding(4.0)
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
    let fill = if active {
        COLOR_ACCENT_RED
    } else {
        COLOR_INSPECTOR_BOX
    };
    let text_color = if active {
        COLOR_TEXT
    } else {
        COLOR_TEXT_SECONDARY
    };
    let inner = egui::Frame::none()
        .fill(fill)
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(14.0, 6.0))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(label)
                        .color(text_color)
                        .strong()
                        .size(14.0),
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
        egui::RichText::new("Split/Second: ARGB Body Paint")
            .color(COLOR_LABEL_BLUE)
            .size(18.0)
            .strong(),
    );
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("The front-end car-select menu draws each vehicle's paint from a pair of files: <car>.streamtex (the DDS records) and <car>.textures (the catalog that locates them). The stock paints are BC1 (DXT1), which dither and band on gradients. Replacing the paint record with an uncompressed A8R8G8B8 (32-bit) DDS gives a clean, dither-free gradient. This tab covers the technical background and the exact workflow that TEXembed automates.")
                .color(COLOR_TEXT)
                .size(14.0),
        )
        .wrap(),
    );

    guide_card(ui, "File structure (the knowledge)", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("• .streamtex: a plain sequence of records, each [u32 length][full DDS].")
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

    guide_card(ui, "The required exe patch (outside this tool)", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("The game streams the whole .streamtex into a fixed 6 MB memory pool. An A8R8G8B8 paint record is ~8 MB, which overflows the pool and crashes the game (access violation). Apply patch_menu_argb.py (included in the repository) to SplitSecond.exe: it redirects the streaming destination buffer from the 6 MB pool to the regular heap. The patch is harmless with the stock files.")
                    .color(COLOR_TEXT)
                    .size(14.0),
            );
        });
    });

    guide_card(ui, "How TEXembed automates this", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("• Allow different compression & size: lets a .streamtex record be replaced with any format or size and rewrites the record length prefix on save. Uncompressed (A8R8G8B8) replacements are automatically reduced to mip 0 (mips=1).")
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
    });

    guide_card(
        ui,
        "Related: changing the compression of an embedded .textures texture",
        |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("By default, replacing a texture requires an identical compression format (for example BC1 with BC1). The \"Allow different compression (patch .textures header)\" checkbox — found in the Inspector under the Replace button and in the Batch Replace dialog — relaxes that rule for .textures files: it lets you swap in a different format (for example BC1 → BC3), and on save the catalog sizes and the static records are patched automatically so the game loads the new format. It is off by default.")
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
                    egui::RichText::new("Psst — this option only applies to .textures files; the Streamtex Options (above) handle .streamtex records instead.")
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
                egui::RichText::new("1. Apply the exe patch once (see above).").color(COLOR_TEXT).size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("2. Open <car>.streamtex, e.g. Deferred/Vehicles/Frontend/Bodies/Musclecar_09/Musclecar_09.streamtex.").color(COLOR_TEXT).size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("3. Select the body-paint record (record 0) in the grid.").color(COLOR_TEXT).size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("4. In the Inspector, tick both checkboxes under \"Streamtex Options\".").color(COLOR_TEXT).size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("5. Replace record 0 with your A8R8G8B8 DDS (any mip count — it is auto-reduced to mip 0) using Replace or drag & drop.").color(COLOR_TEXT).size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("6. Save with Ctrl+S. The log reports the mip-0 reduction and the paired-.textures patch.").color(COLOR_TEXT).size(14.0),
            );
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("7. Launch the game and check the car-select menu.").color(COLOR_TEXT).size(14.0),
            );
        });
    });

    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("Psst — keep the same width/height as the original record (1024×2048 for Musclecar_09); only the format and mip count change in the tested workflow.")
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
            if ui.button("Clear").clicked() {
                state.logs.clear();
            }
            if ui.button("Back to Previews").clicked() {
                state.show_logs = false;
            }
        });
    });
    ui.add_space(12.0);

    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .show(ui, |ui| {
            egui::Frame::none()
                .fill(COLOR_CARD)
                .rounding(4.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    for line in &state.logs {
                        ui.label(egui::RichText::new(line).color(COLOR_TEXT).monospace());
                    }
                });
        });
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
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(card_size), Sense::click());

    // Cards scrolled out of view still need their layout slot (for correct
    // scrollbar/row geometry and click hit-testing once visible again), but
    // skip every paint call for them. In large multi-texture folders every
    // file section is expanded by default and drawn unconditionally, so
    // without this guard hundreds/thousands of off-screen cards would still
    // pay for background fills, texture blits, and stroke tessellation on
    // every single frame, which is what causes the stutter right after
    // opening a folder.
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
        brighten_color(COLOR_CARD, 1.16)
    } else if is_selected {
        brighten_color(COLOR_CARD, 1.10)
    } else {
        COLOR_CARD
    };
    painter.rect_filled(draw_rect, rounding, bg);

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
            egui::Stroke::new(2.0_f32, COLOR_CARD_SELECTED),
        );
    } else if is_hovered {
        painter.rect_stroke(
            draw_rect,
            rounding,
            egui::Stroke::new(1.5_f32, COLOR_ACCENT_ORANGE),
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

    state.enqueue_preview_load(tab_id, index);

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Full Preview")
                .color(COLOR_TEXT)
                .strong()
                .size(18.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Back").clicked() {
                show_full_preview = false;
            }
            if ui.button("Reset").clicked() {
                full_preview_zoom = 1.0;
                full_preview_pan = Vec2::ZERO;
            }
            if ui.button("−").clicked() {
                full_preview_zoom = (full_preview_zoom / 1.15).clamp(0.1, 20.0);
            }
            if ui.button("+").clicked() {
                full_preview_zoom = (full_preview_zoom * 1.15).clamp(0.1, 20.0);
            }
        });
    });
    ui.add_space(12.0);

    let avail = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(avail, Sense::drag());
    let response = response.on_hover_cursor(CursorIcon::Grab);

    ui.painter()
        .rect_filled(rect, egui::Rounding::same(6.0), COLOR_CARD);
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
                    if let Some(PreviewState::Ready(texture)) = state.previews.get(&(tab_id, index))
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
                    if let Some(PreviewState::Ready(texture)) = state.previews.get(&(tab_id, index))
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

    let preview_state = state.previews.get(&(tab_id, index));
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
        _ => {
            ui.painter().text(
                content_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Preview\nnot\navailable",
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
}

#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn draw_grid(state: &mut AppState, _ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(tab) = state.active_tab() else {
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
    let selected_idx = tab.selected_index;
    let total = tab.file.dds_list.len();

    let available_width = ui.available_width();
    let card_size = THUMBNAIL_SIZE;
    let spacing = 5.0;
    let num_columns = ((available_width + spacing) / (card_size + spacing)).floor() as usize;
    let num_columns = num_columns.max(1);

    let mut to_enqueue: Vec<usize> = Vec::new();
    let mut clicked_index: Option<usize> = None;
    let mut toggle_files: Vec<usize> = Vec::new();

    {
        let gallery = tab.folder_gallery.as_ref();

        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if let Some(gallery) = gallery {
                if gallery.include_multi_texture {
                    egui::ScrollArea::vertical()
                        .id_salt(("textures_scroll", tab_id))
                        .show(ui, |ui| {
                            for (file_idx, file) in gallery.files.iter().enumerate() {
                                let expanded =
                                    gallery.file_expanded.get(file_idx).copied().unwrap_or(true);
                                let file_name = file
                                    .path
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("file");

                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let arrow = if expanded { "v" } else { ">" };
                                    let arrow_btn = ui
                                        .add_sized(Vec2::new(22.0, 22.0), egui::Button::new(arrow))
                                        .on_hover_cursor(CursorIcon::PointingHand);
                                    if arrow_btn.clicked() {
                                        toggle_files.push(file_idx);
                                    }
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(file_name)
                                                .color(COLOR_TEXT)
                                                .size(13.0),
                                        )
                                        .sense(Sense::hover()),
                                    )
                                    .on_hover_cursor(CursorIcon::PointingHand);
                                });

                                let (line_rect, _) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 1.0),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(line_rect, 0.0, COLOR_LABEL_BLUE);
                                ui.add_space(spacing);

                                if expanded {
                                    let range = gallery
                                        .file_global_ranges
                                        .get(file_idx)
                                        .cloned()
                                        .unwrap_or(0..0);
                                    let count = range.end.saturating_sub(range.start);
                                    let rows = count.div_ceil(num_columns);

                                    for row in 0..rows {
                                        let start = range.start + row * num_columns;
                                        let end = (start + num_columns).min(range.end);

                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = spacing;
                                            for index in start..end {
                                                let is_selected = selected_idx == Some(index);
                                                let is_unsaved =
                                                    AppState::is_index_unsaved(tab, index);
                                                let response = draw_texture_card(
                                                    ui,
                                                    state,
                                                    tab_id,
                                                    index,
                                                    card_size,
                                                    is_selected,
                                                    is_unsaved,
                                                );
                                                if ui.clip_rect().intersects(response.rect) {
                                                    to_enqueue.push(index);
                                                }

                                                if response.clicked() {
                                                    clicked_index = Some(index);
                                                }
                                            }
                                        });
                                        ui.add_space(spacing);
                                    }
                                }

                                ui.add_space(spacing);
                            }
                        });
                } else {
                    let total = gallery.index_map.len();
                    let row_count = total.div_ceil(num_columns);
                    let row_height = card_size + spacing;

                    egui::ScrollArea::vertical()
                        .id_salt(("textures_scroll", tab_id))
                        .show_rows(ui, row_height, row_count, |ui, row_range| {
                            for row in row_range {
                                let start = row * num_columns;
                                let end = (start + num_columns).min(total);

                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = spacing;
                                    for index in start..end {
                                        let is_selected = selected_idx == Some(index);
                                        let is_unsaved = AppState::is_index_unsaved(tab, index);
                                        let response = draw_texture_card(
                                            ui,
                                            state,
                                            tab_id,
                                            index,
                                            card_size,
                                            is_selected,
                                            is_unsaved,
                                        );
                                        to_enqueue.push(index);

                                        if response.clicked() {
                                            clicked_index = Some(index);
                                        }
                                    }
                                });
                                ui.add_space(spacing);
                            }
                        });
                }
            } else {
                let row_count = total.div_ceil(num_columns);
                let row_height = card_size + spacing;

                egui::ScrollArea::vertical()
                    .id_salt(("textures_scroll", tab_id))
                    .show_rows(ui, row_height, row_count, |ui, row_range| {
                        for row in row_range {
                            let start = row * num_columns;
                            let end = (start + num_columns).min(total);

                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = spacing;
                                for index in start..end {
                                    let is_selected = selected_idx == Some(index);
                                    let is_unsaved = AppState::is_index_unsaved(tab, index);
                                    let response = draw_texture_card(
                                        ui,
                                        state,
                                        tab_id,
                                        index,
                                        card_size,
                                        is_selected,
                                        is_unsaved,
                                    );
                                    to_enqueue.push(index);

                                    if response.clicked() {
                                        clicked_index = Some(index);
                                    }
                                }
                            });
                            ui.add_space(spacing);
                        }
                    });
            }
        });
    }

    if let Some(t) = state.active_tab_mut() {
        if let Some(gallery) = t.folder_gallery.as_mut() {
            for file_idx in toggle_files {
                if let Some(v) = gallery.file_expanded.get_mut(file_idx) {
                    *v = !*v;
                }
            }
        }
    }

    for index in to_enqueue {
        state.enqueue_preview_load(tab_id, index);
    }
    if let Some(index) = clicked_index {
        if let Some(t) = state.active_tab_mut() {
            t.selected_index = Some(index);
        }
    }
}