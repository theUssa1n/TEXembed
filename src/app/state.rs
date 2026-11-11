use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Instant;

use egui::{TextureHandle, Vec2};
use rfd::FileDialog;

use texembed::{
    parse_dds_header, save_streamtex_file, save_streamtex_with_sidecar, save_textures_file,
    DdsInfo, FileType, TextureFile,
};

use super::loading::{GalleryBuild, LoadRequest, LoadedContent, LoadingManager};
use super::preview::{
    decode_dds_rgba, format_pixel_format_u32, normalize_legacy_pixel_format, patch_legacy_fourcc,
};
use super::preview_cache::PreviewManager;
pub(super) use super::preview_cache::PreviewState;
use super::widgets::dds_export_name;

pub(super) struct FolderGallery {
    pub(super) files: Vec<TextureFile>,
    pub(super) index_map: Vec<(usize, usize)>,
    pub(super) file_global_ranges: Vec<std::ops::Range<usize>>,
    pub(super) file_expanded: Vec<bool>,
    pub(super) include_multi_texture: bool,
    pub(super) is_folder: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BatchReplaceRule {
    ExactName,
    Index,
}

pub(super) struct BatchReplaceEntry {
    pub(super) target_index: usize,
    pub(super) target_name: String,
    pub(super) target_size: String,
    pub(super) target_source: Option<String>,
    pub(super) source_name: Option<String>,
    pub(super) source_size: Option<String>,
    pub(super) source_path: Option<std::path::PathBuf>,
    pub(super) status: BatchReplaceStatus,
    pub(super) prepared: Option<PreparedReplacement>,
}

pub(super) enum BatchReplaceStatus {
    Ready,
    Missing,
    DuplicateSource { count: usize },
    ReadError(String),
    ValidationError(String),
}

pub(super) struct Tab {
    pub(super) id: usize,
    pub(super) file: TextureFile,
    pub(super) selected_index: Option<usize>,
    pub(super) has_unsaved_changes: bool,
    pub(super) show_full_preview: bool,
    pub(super) full_preview_zoom: f32,
    pub(super) full_preview_pan: Vec2,
    pub(super) replace_backups: HashMap<usize, DdsInfo>,
    pub(super) folder_gallery: Option<FolderGallery>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AboutSection {
    Guide,
    Argb,
}

#[allow(clippy::struct_excessive_bools)]
pub(super) struct AppState {
    pub(super) tabs: Vec<Tab>,
    pub(super) active_tab_id: Option<usize>,
    pub(super) next_tab_id: usize,
    pub(super) previews: PreviewManager,
    pub(super) error_message: Option<String>,
    pub(super) logs: Vec<String>,
    pub(super) show_logs: bool,
    pub(super) show_about: bool,
    pub(super) show_tweaks: bool,
    pub(super) about_section: AboutSection,
    pub(super) style_initialized: bool,
    pub(super) icon_open_file: Option<TextureHandle>,
    pub(super) icon_open_folder: Option<TextureHandle>,
    pub(super) icon_logs: Option<TextureHandle>,
    pub(super) icon_about: Option<TextureHandle>,
    pub(super) icon_tweaks: Option<TextureHandle>,
    pub(super) show_open_folder_options: bool,
    pub(super) folder_include_multi_texture: bool,
    pub(super) patch_header_on_replace: bool,
    pub(super) streamtex_allow_resize: bool,
    pub(super) streamtex_patch_sidecar: bool,
    pub(super) resize_on_replace: bool,
    pub(super) show_multi_open_dialog: bool,
    pub(super) show_batch_replace_dialog: bool,
    pub(super) batch_replace_tab_id: Option<usize>,
    pub(super) batch_replace_rule: BatchReplaceRule,
    pub(super) batch_replace_folder: Option<std::path::PathBuf>,
    pub(super) batch_replace_entries: Vec<BatchReplaceEntry>,
    pub(super) batch_replace_unused_sources: Vec<String>,
    pub(super) multi_open_paths: Vec<std::path::PathBuf>,
    pub(super) multi_open_shared_view: bool,
    pub(super) multi_open_include_multi_texture: bool,
    pub(super) show_close_dialog: Option<usize>,
    pub(super) loading: LoadingManager,
    pub(super) previous_active_tab_id: Option<usize>,
    pub(super) ipc_receiver: Option<std::sync::mpsc::Receiver<String>>,
    pub(super) tabs_scroll_offset: f32,
    pub(super) tooltip_id: Option<egui::Id>,
    pub(super) tooltip_since: Option<Instant>,
    pub(super) frame_tooltip: Option<(egui::Id, &'static str)>,
    pub(super) show_transparency_bg: bool,
}

#[derive(Clone)]
pub(super) struct PreparedReplacement {
    pub(super) bytes: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) mipmap_count: u32,
    pub(super) pixel_format: u32,
    pub(super) note: Option<String>,
}

const DDS_HEADER_SIZE: usize = 128;

fn normalize_streamtex_replacement(
    original_bytes: &[u8],
    replacement_bytes: &[u8],
) -> Result<(Vec<u8>, Option<String>), String> {
    if original_bytes.len() < DDS_HEADER_SIZE || replacement_bytes.len() < DDS_HEADER_SIZE {
        return Err("DDS file is too small to normalize for .streamtex.".to_string());
    }

    let original_data_len = original_bytes.len() - DDS_HEADER_SIZE;
    let replacement_data_len = replacement_bytes.len() - DDS_HEADER_SIZE;

    if replacement_data_len < original_data_len {
        return Err(format!(
            "DDS payload is too small for this .streamtex entry: expected at least {original_data_len} bytes of image data, got {replacement_data_len}."
        ));
    }

    let mut normalized = Vec::with_capacity(original_bytes.len());
    normalized.extend_from_slice(&original_bytes[..DDS_HEADER_SIZE]);
    normalized.extend_from_slice(
        &replacement_bytes[DDS_HEADER_SIZE..DDS_HEADER_SIZE + original_data_len],
    );

    let note = (replacement_data_len != original_data_len
        || original_bytes[..DDS_HEADER_SIZE] != replacement_bytes[..DDS_HEADER_SIZE])
        .then(|| {
            format!(
                "Normalized .streamtex DDS layout to match the original entry (header preserved, image data {replacement_data_len} -> {original_data_len} bytes)."
            )
        });

    Ok((normalized, note))
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab_id: None,
            next_tab_id: 0,
            previews: PreviewManager::default(),
            error_message: None,
            logs: Vec::new(),
            show_logs: false,
            show_about: false,
            show_tweaks: false,
            about_section: AboutSection::Guide,
            style_initialized: false,
            icon_open_file: None,
            icon_open_folder: None,
            icon_logs: None,
            icon_about: None,
            icon_tweaks: None,
            show_open_folder_options: false,
            folder_include_multi_texture: true,
            patch_header_on_replace: false,
            streamtex_allow_resize: false,
            streamtex_patch_sidecar: false,
            resize_on_replace: false,
            show_multi_open_dialog: false,
            show_batch_replace_dialog: false,
            batch_replace_tab_id: None,
            batch_replace_rule: BatchReplaceRule::ExactName,
            batch_replace_folder: None,
            batch_replace_entries: Vec::new(),
            batch_replace_unused_sources: Vec::new(),
            multi_open_paths: Vec::new(),
            multi_open_shared_view: false,
            multi_open_include_multi_texture: true,
            show_close_dialog: None,
            loading: LoadingManager::default(),
            previous_active_tab_id: None,
            ipc_receiver: None,
            tabs_scroll_offset: 0.0,
            tooltip_id: None,
            tooltip_since: None,
            frame_tooltip: None,
            show_transparency_bg: true,
        }
    }
}

pub fn create_app(
    rx: std::sync::mpsc::Receiver<String>,
    file_to_open: Option<std::path::PathBuf>,
) -> Box<dyn eframe::App> {
    let mut app_state = AppState {
        ipc_receiver: Some(rx),
        ..Default::default()
    };
    if let Some(path) = file_to_open {
        app_state.queue_file_load(path);
    }
    Box::new(app_state)
}

fn normalize_batch_name(name: &str) -> String {
    std::path::Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name)
        .to_ascii_lowercase()
}

fn batch_index_key(name: &str) -> Option<usize> {
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(name);

    stem.parse::<usize>().ok().or_else(|| {
        stem.strip_prefix("texture_")
            .and_then(|suffix| suffix.parse::<usize>().ok())
    })
}

/// Splits an export-suffixed stem (`name_12`) into its base name and the target
/// index it was suffixed with by Export All for duplicate texture names.
/// Returns `None` when the stem does not end in `_<digits>`.
fn split_batch_hint(stem: &str) -> Option<(&str, usize)> {
    let pos = stem.rfind('_')?;
    let (base, suffix) = stem.split_at(pos);
    let digits = &suffix[1..];
    if base.is_empty() || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let index = digits.parse().ok()?;
    Some((base, index))
}

/// Produces a unique export file name. Duplicate texture names are common (the
/// same image can back several entries), so the first entry keeps the plain
/// name and later ones are suffixed with their global index. The suffix lets
/// Batch Replace route the file back to the exact entry it came from instead of
/// every same-named entry.
fn unique_export_name(base_name: &str, index: usize, used_names: &mut HashSet<String>) -> String {
    if !used_names.contains(base_name) {
        used_names.insert(base_name.to_string());
        return base_name.to_string();
    }

    let stem = base_name.strip_suffix(".dds").unwrap_or(base_name);
    let mut candidate = format!("{stem}_{index}.dds");
    let mut attempt = 0;
    while used_names.contains(&candidate) {
        attempt += 1;
        candidate = format!("{stem}_{index}_{attempt}.dds");
    }
    used_names.insert(candidate.clone());
    candidate
}

impl BatchReplaceEntry {
    pub(super) const fn is_ready(&self) -> bool {
        matches!(self.status, BatchReplaceStatus::Ready)
    }

    pub(super) const fn is_missing(&self) -> bool {
        matches!(self.status, BatchReplaceStatus::Missing)
    }

    pub(super) fn status_label(&self) -> String {
        match &self.status {
            BatchReplaceStatus::Ready => "Ready".to_string(),
            BatchReplaceStatus::Missing => "Not found".to_string(),
            BatchReplaceStatus::DuplicateSource { count } => {
                format!("Multiple matches ({count})")
            }
            BatchReplaceStatus::ReadError(message) => format!("Read error: {message}"),
            BatchReplaceStatus::ValidationError(message) => message.replace('\n', " | "),
        }
    }
}

impl AppState {
    pub(super) fn tab_by_id(&self, id: usize) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub(super) fn tab_mut_by_id(&mut self, id: usize) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub(super) fn prepare_replacement_dds(
        file_type: &FileType,
        original: &DdsInfo,
        new_dds_bytes: &[u8],
        patch_header: bool,
        streamtex_allow_resize: bool,
        resize_on_replace: bool,
    ) -> Result<PreparedReplacement, String> {
        let mut patched_bytes = patch_legacy_fourcc(new_dds_bytes);
        let (mut width, mut height, mut mipmap_count, mut pixel_format) =
            parse_dds_header(&patched_bytes).map_err(|err| err.to_string())?;
        let expected_pf = normalize_legacy_pixel_format(original.pixel_format);
        // Volume (3D) textures are exported as a flat 2D strip because image
        // editors cannot open `DDSCAPS2_VOLUME` files. When the incoming file
        // matches that strip shape, rebuild the 3D layout before validating, so
        // exporting a volume and importing it back stays a lossless round trip.
        let mut volume_note = None;
        let depth = super::volume::volume_depth(original.bytes.as_ref());
        if depth >= 2 {
            if let Some((flat_w, flat_h)) =
                super::volume::flattened_dimensions(original.bytes.as_ref())
            {
                if width == flat_w && height == flat_h {
                    let format = super::volume::volume_image_format(original.bytes.as_ref())
                        .ok_or_else(|| {
                            "Unsupported volume texture format for re-import".to_string()
                        })?;
                    patched_bytes = super::volume::rebuild_volume_from_strip(
                        &patched_bytes,
                        original.bytes.as_ref(),
                        original.width,
                        original.height,
                        depth,
                        original.mipmap_count,
                        format,
                    )?;
                    // The rebuilt header is copied from the original record, so
                    // legacy fourCCs (DXT4/DXT2) need the same normalization the
                    // rest of the tool applies.
                    patched_bytes = patch_legacy_fourcc(&patched_bytes);
                    (width, height, mipmap_count, pixel_format) =
                        parse_dds_header(&patched_bytes).map_err(|err| err.to_string())?;
                    volume_note = Some(format!(
                        "Volume texture rebuilt: the {flat_w}x{flat_h} strip was split into {depth} slices of {}x{} and its mip chain regenerated.",
                        original.width, original.height
                    ));
                } else {
                    // A plain 2D image would leave the record's `sides` field
                    // pointing at a mip chain that no longer exists, so the game
                    // would sample garbage. Only the exported strip is accepted.
                    return Err(format!(
                        "Volume texture: expected the {flat_w}x{flat_h} strip produced by Export ({} slices of {}x{} stacked vertically), received {width}x{height}.",
                        depth, original.width, original.height
                    ));
                }
            }
        }
        // For .textures files the catalog sizes and records are patched on save,
        // so a different compression format can be allowed as an opt-in. The
        // resize option implies a format change may accompany the new size.
        let allow_compression_change =
            (patch_header || resize_on_replace) && matches!(file_type, FileType::Textures);
        // A record can be replaced with a different size/format: for .streamtex
        // through the Streamtex Options, and for both file types through the
        // resize option. The record fields (pf/mips/width/height) and the size
        // prefixes are patched on save to match the new DDS header.
        let allow_resize = (streamtex_allow_resize && matches!(file_type, FileType::Streamtex))
            || resize_on_replace;
        let mut reasons = Vec::new();

        if !allow_resize {
            if width != original.width || height != original.height {
                reasons.push(format!(
                    "Size mismatch: Expected {}x{}, Received {}x{}",
                    original.width, original.height, width, height
                ));
            }
            if mipmap_count != original.mipmap_count {
                reasons.push(format!(
                    "Mip count differs: Expected {}, Received {}",
                    original.mipmap_count, mipmap_count
                ));
            }
            if !allow_compression_change {
                if super::preview::is_b8g8r8a8(original.bytes.as_ref())
                    && !super::preview::is_b8g8r8a8(&patched_bytes)
                {
                    reasons.push(format!(
                        "Format mismatch: Expected {}, Received {}",
                        super::preview::format_pixel_format(original.bytes.as_ref()),
                        super::preview::format_pixel_format(&patched_bytes)
                    ));
                }
                if pixel_format != expected_pf {
                    reasons.push(format!(
                        "Format mismatch: Expected {}, Received {}",
                        format_pixel_format_u32(expected_pf),
                        format_pixel_format_u32(pixel_format)
                    ));
                }
            }
        }

        if !reasons.is_empty() {
            return Err(reasons.join("\n"));
        }

        let (bytes, mipmap_count_final, note) = match file_type {
            FileType::Streamtex => {
                if allow_resize {
                    let mut bytes = patched_bytes;
                    let mut final_mips = mipmap_count;
                    let mut strip_note = None;
                    // The game's frontend loader only accepts uncompressed
                    // (A8R8G8B8) streamed records with a single mip level: a
                    // full mip chain makes the car not render at all. Strip
                    // every level past mip 0 and fix the header (pitch + mips).
                    let is_uncompressed_rgba =
                        pixel_format == 0 && super::preview::is_b8g8r8a8(&bytes);
                    if is_uncompressed_rgba && mipmap_count > 1 {
                        let mip0_len = (width as usize) * (height as usize) * 4;
                        if bytes.len() >= 128 + mip0_len {
                            let mut hdr = bytes[..128].to_vec();
                            hdr[20..24].copy_from_slice(&(width * 4).to_le_bytes()); // pitch
                            hdr[28..32].copy_from_slice(&1_u32.to_le_bytes()); // mips = 1
                            let mut stripped = hdr;
                            stripped.extend_from_slice(&bytes[128..128 + mip0_len]);
                            bytes = stripped;
                            final_mips = 1;
                            strip_note = Some(format!(
                                "Uncompressed record reduced to mip 0 (mips=1): the game's menu loader only supports A8R8G8B8 with a single mip level (got {mipmap_count})."
                            ));
                        }
                    }
                    let differs =
                        bytes.len() != original.bytes.len() || pixel_format != expected_pf;
                    let note = match (differs, strip_note) {
                        (true, Some(strip)) => Some(format!(
                            "Replaced with a different size or compression and reduced to mip 0. The .streamtex record and its length prefix will be rewritten on save. Keep \"Auto-patch paired .textures on save\" enabled so the game reads the updated offsets.\n{strip}"
                        )),
                        (true, None) => Some(
                            "Replaced with a different size or compression. The .streamtex record and its length prefix will be rewritten on save. Keep \"Auto-patch paired .textures on save\" enabled so the game reads the updated offsets."
                                .to_string(),
                        ),
                        (false, Some(strip)) => Some(strip),
                        (false, None) => None,
                    };
                    (bytes, final_mips, note)
                } else {
                    let (norm_bytes, norm_note) =
                        normalize_streamtex_replacement(original.bytes.as_ref(), &patched_bytes)?;
                    (norm_bytes, mipmap_count, norm_note)
                }
            }
            FileType::Textures => {
                let mut notes: Vec<String> = Vec::new();
                if allow_compression_change && pixel_format != expected_pf {
                    notes.push(format!(
                        "Different compression detected: {} -> {}. The .textures header will be patched on save.",
                        super::preview::format_pixel_format(original.bytes.as_ref()),
                        super::preview::format_pixel_format(&patched_bytes)
                    ));
                }
                if resize_on_replace
                    && (width != original.width
                        || height != original.height
                        || mipmap_count != original.mipmap_count)
                {
                    notes.push(format!(
                        "Different size or mip count: {}x{} m{} -> {}x{} m{}. The .textures record and catalog sizes will be patched on save.",
                        original.width,
                        original.height,
                        original.mipmap_count,
                        width,
                        height,
                        mipmap_count
                    ));
                }
                let note = if notes.is_empty() {
                    None
                } else {
                    Some(notes.join("\n"))
                };
                (patched_bytes, mipmap_count, note)
            }
        };

        let note = match (volume_note, note) {
            (Some(volume), Some(note)) => Some(format!("{volume}\n{note}")),
            (Some(volume), None) => Some(volume),
            (None, note) => note,
        };

        Ok(PreparedReplacement {
            bytes,
            width,
            height,
            mipmap_count: mipmap_count_final,
            pixel_format,
            note,
        })
    }

    pub(super) fn log(&mut self, message: &str) {
        let timestamp = crate::runtime::log_timestamp();
        self.logs.push(format!("[{timestamp}] {message}"));
        if self.logs.len() > 500 {
            self.logs.remove(0);
        }
    }

    fn report_error(&mut self, message: String) {
        self.log(&message);
        if let Some(existing) = &mut self.error_message {
            if *existing != message {
                existing.push_str("\n\n");
                existing.push_str(&message);
            }
        } else {
            self.error_message = Some(message);
        }
    }

    pub(super) fn active_tab(&self) -> Option<&Tab> {
        self.active_tab_id
            .and_then(|id| self.tabs.iter().find(|t| t.id == id))
    }

    pub(super) fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let id = self.active_tab_id?;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub(super) fn can_save(&self) -> bool {
        self.active_tab().is_some_and(|t| t.has_unsaved_changes)
    }

    pub(super) fn can_batch_replace(&self) -> bool {
        self.active_tab()
            .is_some_and(|tab| Self::total_textures_in_tab(tab) > 0)
    }

    pub(super) fn can_export_all(&self) -> bool {
        self.active_tab().is_some()
    }

    pub(super) fn open_batch_replace_dialog(&mut self) {
        let Some(tab_id) = self.active_tab_id else {
            return;
        };
        self.show_batch_replace_dialog = true;
        self.batch_replace_tab_id = Some(tab_id);
        self.batch_replace_entries.clear();
        self.batch_replace_unused_sources.clear();
    }

    pub(super) fn total_textures_in_tab(tab: &Tab) -> usize {
        tab.folder_gallery
            .as_ref()
            .map_or(tab.file.dds_list.len(), |gallery| gallery.index_map.len())
    }

    pub(super) fn original_filename_in_tab(tab: &Tab, index: usize) -> Option<String> {
        let gallery = tab.folder_gallery.as_ref()?;
        let (file_idx, _) = gallery.index_map.get(index).copied()?;
        let file = gallery.files.get(file_idx)?;
        Some(
            file.path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("file")
                .to_string(),
        )
    }

    pub(super) fn dds_by_index(tab: &Tab, index: usize) -> Option<&DdsInfo> {
        if let Some(gallery) = &tab.folder_gallery {
            let (file_idx, local_idx) = gallery.index_map.get(index).copied()?;
            gallery.files.get(file_idx)?.dds_list.get(local_idx)
        } else {
            tab.file.dds_list.get(index)
        }
    }

    pub(super) fn dds_by_index_mut(tab: &mut Tab, index: usize) -> Option<&mut DdsInfo> {
        if let Some(gallery) = tab.folder_gallery.as_mut() {
            let (file_idx, local_idx) = gallery.index_map.get(index).copied()?;
            gallery.files.get_mut(file_idx)?.dds_list.get_mut(local_idx)
        } else {
            tab.file.dds_list.get_mut(index)
        }
    }

    pub(super) fn is_index_unsaved(tab: &Tab, index: usize) -> bool {
        tab.replace_backups.contains_key(&index)
    }

    pub(super) fn file_type_by_index(tab: &Tab, index: usize) -> Option<&FileType> {
        if let Some(gallery) = &tab.folder_gallery {
            let (file_idx, _) = gallery.index_map.get(index).copied()?;
            Some(&gallery.files.get(file_idx)?.file_type)
        } else {
            Some(&tab.file.file_type)
        }
    }

    pub(super) fn file_type_by_index_cloned(tab: &Tab, index: usize) -> Option<FileType> {
        Self::file_type_by_index(tab, index).cloned()
    }

    fn apply_prepared_replacement_to_index(
        tab: &mut Tab,
        index: usize,
        original: &DdsInfo,
        prepared: PreparedReplacement,
    ) -> Option<(String, Option<String>)> {
        Self::dds_by_index(tab, index)?;
        let PreparedReplacement {
            bytes,
            width,
            height,
            mipmap_count,
            pixel_format,
            note,
        } = prepared;

        tab.replace_backups
            .entry(index)
            .or_insert_with(|| original.clone());

        let dds_info = Self::dds_by_index_mut(tab, index)?;
        dds_info.bytes = bytes.into();
        dds_info.width = width;
        dds_info.height = height;
        dds_info.mipmap_count = mipmap_count;
        dds_info.pixel_format = pixel_format;
        dds_info.modified = true;
        let name = dds_info.name.clone();
        tab.has_unsaved_changes = true;
        Some((name, note))
    }

    pub(super) fn replace_from_path(
        &mut self,
        tab_id: usize,
        index: usize,
        path: &std::path::Path,
    ) {
        let Some((original, file_type)) = self.tab_by_id(tab_id).and_then(|tab| {
            Some((
                Self::dds_by_index(tab, index)?.clone(),
                Self::file_type_by_index_cloned(tab, index)?,
            ))
        }) else {
            self.report_error("The replacement target is no longer available.".to_string());
            return;
        };
        let prepared = crate::runtime::run_guarded(|| {
            let bytes = fs::read(path)
                .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
            Self::prepare_replacement_dds(
                &file_type,
                &original,
                &bytes,
                self.patch_header_on_replace,
                self.streamtex_allow_resize,
                self.resize_on_replace,
            )
        });
        match prepared {
            Ok(prepared) => {
                let replacement = self.tab_mut_by_id(tab_id).and_then(|tab| {
                    Self::apply_prepared_replacement_to_index(tab, index, &original, prepared)
                });
                if let Some((name, note)) = replacement {
                    self.invalidate_preview(tab_id, index);
                    if let Some(note) = note {
                        self.log(&note);
                    }
                    self.log(&format!("Replaced texture: {name}"));
                }
            }
            Err(error) => self.report_error(error),
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn scan_batch_replace(&mut self) {
        let Some(tab_id) = self.batch_replace_tab_id else {
            self.error_message = Some("No target tab is selected for batch replace.".to_string());
            return;
        };
        let Some(folder_path) = self.batch_replace_folder.clone() else {
            self.error_message = Some("Please choose a DDS folder first.".to_string());
            return;
        };
        let Some(tab) = self.tab_by_id(tab_id) else {
            self.error_message = Some("The target tab is no longer available.".to_string());
            return;
        };

        let read_dir = match fs::read_dir(&folder_path) {
            Ok(read_dir) => read_dir,
            Err(err) => {
                self.error_message = Some(err.to_string());
                return;
            }
        };

        let mut dds_paths: Vec<std::path::PathBuf> = read_dir
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("dds"))
            })
            .collect();
        dds_paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));

        if dds_paths.is_empty() {
            self.error_message =
                Some("No DDS files were found in the selected replacement folder.".to_string());
            return;
        }

        let total = Self::total_textures_in_tab(tab);

        // Collect the normalized key of every entry up front so index-hinted
        // source files (Export All suffixes duplicate names with "_<index>")
        // can be routed back to the exact entry they were exported from.
        let target_keys: Vec<(usize, String)> = (0..total)
            .filter_map(|index| {
                let dds = Self::dds_by_index(tab, index)?;
                let target_name = dds_export_name(&dds.name, index);
                let key = match self.batch_replace_rule {
                    BatchReplaceRule::ExactName => normalize_batch_name(&target_name),
                    BatchReplaceRule::Index => index.to_string(),
                };
                Some((index, key))
            })
            .collect();

        let mut grouped_paths: HashMap<String, Vec<std::path::PathBuf>> = HashMap::new();
        let mut hinted_paths: HashMap<usize, Vec<std::path::PathBuf>> = HashMap::new();
        for path in dds_paths {
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            match self.batch_replace_rule {
                BatchReplaceRule::ExactName => {
                    let stem = normalize_batch_name(file_name);
                    if target_keys.iter().any(|(_, key)| *key == stem) {
                        grouped_paths.entry(stem).or_default().push(path);
                    } else if let Some((base, claimed_index)) = split_batch_hint(&stem) {
                        let hint_matches = target_keys
                            .iter()
                            .any(|(index, key)| *index == claimed_index && *key == base);
                        if hint_matches {
                            hinted_paths.entry(claimed_index).or_default().push(path);
                        } else {
                            grouped_paths.entry(stem).or_default().push(path);
                        }
                    } else {
                        grouped_paths.entry(stem).or_default().push(path);
                    }
                }
                BatchReplaceRule::Index => {
                    let maybe_key = batch_index_key(file_name).map(|value| value.to_string());
                    let Some(key) = maybe_key else {
                        continue;
                    };
                    grouped_paths.entry(key).or_default().push(path);
                }
            }
        }

        let mut used_sources: HashSet<std::path::PathBuf> = HashSet::new();
        let mut entries = Vec::with_capacity(total);

        for index in 0..total {
            let Some(dds) = Self::dds_by_index(tab, index) else {
                continue;
            };
            let Some(file_type) = Self::file_type_by_index(tab, index) else {
                continue;
            };

            let target_name = dds_export_name(&dds.name, index);
            let target_source = Self::original_filename_in_tab(tab, index);
            let key = match self.batch_replace_rule {
                BatchReplaceRule::ExactName => normalize_batch_name(&target_name),
                BatchReplaceRule::Index => index.to_string(),
            };

            // Index-hinted files take priority over plain name matches so that
            // duplicate texture names stay unambiguous: a file exported as
            // "name_<index>.dds" only ever replaces the entry with that index.
            let candidates: Vec<std::path::PathBuf> = match self.batch_replace_rule {
                BatchReplaceRule::ExactName => {
                    hinted_paths.get(&index).map_or_else(Vec::new, Clone::clone)
                }
                BatchReplaceRule::Index => Vec::new(),
            };
            let candidates = if candidates.is_empty() {
                grouped_paths.get(&key).map_or_else(Vec::new, Clone::clone)
            } else {
                candidates
            };

            let mut entry = BatchReplaceEntry {
                target_index: index,
                target_size: format!("{}x{}", dds.width, dds.height),
                target_name,
                target_source,
                source_name: None,
                source_size: None,
                source_path: None,
                status: BatchReplaceStatus::Missing,
                prepared: None,
            };

            if candidates.len() > 1 {
                entry.status = BatchReplaceStatus::DuplicateSource {
                    count: candidates.len(),
                };
            } else if let Some(path) = candidates.first() {
                used_sources.insert(path.clone());
                entry.source_name = Some(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map_or_else(|| path.display().to_string(), str::to_string),
                );
                entry.source_path = Some(path.clone());

                match fs::read(path) {
                    Ok(bytes) => {
                        entry.source_size = parse_dds_header(&bytes)
                            .ok()
                            .map(|(width, height, _, _)| format!("{width}x{height}"));
                        match Self::prepare_replacement_dds(
                            file_type,
                            dds,
                            &bytes,
                            self.patch_header_on_replace,
                            self.streamtex_allow_resize,
                            self.resize_on_replace,
                        ) {
                            Ok(prepared) => {
                                entry.status = BatchReplaceStatus::Ready;
                                entry.prepared = Some(prepared);
                            }
                            Err(err) => {
                                entry.status = BatchReplaceStatus::ValidationError(err);
                            }
                        }
                    }
                    Err(err) => {
                        entry.status = BatchReplaceStatus::ReadError(err.to_string());
                    }
                }
            }

            entries.push(entry);
        }

        let mut unused_sources: Vec<String> = grouped_paths
            .values()
            .chain(hinted_paths.values())
            .flat_map(|paths| paths.iter())
            .filter(|path| !used_sources.contains(*path))
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            })
            .collect();
        unused_sources.sort();

        self.batch_replace_entries = entries;
        self.batch_replace_unused_sources = unused_sources;
        self.error_message = None;
        self.log("Scanned batch replace folder.");
    }

    pub(super) fn apply_batch_replace(&mut self) {
        let Some(tab_id) = self.batch_replace_tab_id else {
            self.error_message = Some("No target tab is selected for batch replace.".to_string());
            return;
        };

        // The replace options (allow different compression, streamtex resize,
        // resize on replace) can be toggled after scanning. Re-read and
        // re-validate each source here instead of trusting the `prepared`
        // results cached by the scan, otherwise Apply would silently ignore the
        // current settings.
        let sources: Vec<(usize, std::path::PathBuf)> = self
            .batch_replace_entries
            .iter()
            .filter_map(|entry| {
                entry
                    .source_path
                    .clone()
                    .map(|path| (entry.target_index, path))
            })
            .collect();

        if sources.is_empty() {
            self.error_message =
                Some("There are no valid batch replacements to apply.".to_string());
            return;
        }

        let patch_header = self.patch_header_on_replace;
        let streamtex_allow_resize = self.streamtex_allow_resize;
        let resize_on_replace = self.resize_on_replace;

        let mut applied_names: Vec<String> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        let mut refreshed_indices: Vec<usize> = Vec::new();

        {
            let Some(tab) = self.tab_mut_by_id(tab_id) else {
                self.error_message = Some("The target tab is no longer available.".to_string());
                return;
            };

            for (index, path) in sources {
                let Some(original) = Self::dds_by_index(tab, index).cloned() else {
                    continue;
                };
                let Some(file_type) = Self::file_type_by_index_cloned(tab, index) else {
                    continue;
                };
                let bytes = match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        failures.push(format!("{}: {err}", original.name));
                        continue;
                    }
                };
                let prepared = match Self::prepare_replacement_dds(
                    &file_type,
                    &original,
                    &bytes,
                    patch_header,
                    streamtex_allow_resize,
                    resize_on_replace,
                ) {
                    Ok(prepared) => prepared,
                    Err(err) => {
                        failures.push(format!("{}: {}", original.name, err.replace('\n', " | ")));
                        continue;
                    }
                };
                if let Some((name, note)) =
                    Self::apply_prepared_replacement_to_index(tab, index, &original, prepared)
                {
                    applied_names.push(name);
                    if let Some(note) = note {
                        notes.push(note);
                    }
                    refreshed_indices.push(index);
                }
            }

            if !applied_names.is_empty() {
                tab.has_unsaved_changes = true;
            }
        }

        for index in refreshed_indices {
            self.invalidate_preview(tab_id, index);
        }

        for note in notes {
            self.log(&note);
        }

        if applied_names.is_empty() {
            self.error_message = Some(if failures.is_empty() {
                "No batch replacements were applied.".to_string()
            } else {
                format!(
                    "No batch replacements were applied. {}",
                    failures.join(" | ")
                )
            });
            return;
        }

        if !failures.is_empty() {
            self.error_message = Some(format!(
                "{} textures failed and were skipped: {}",
                failures.len(),
                failures.join(" | ")
            ));
        }

        self.log(&format!(
            "Applied batch replace to {} textures.",
            applied_names.len()
        ));
        self.show_batch_replace_dialog = false;
    }

    pub(super) fn invalidate_preview(&mut self, tab_id: usize, index: usize) {
        self.previews.invalidate(tab_id, index);
    }

    pub(super) fn clear_tab_previews(&mut self, tab_id: usize) {
        self.previews.clear_tab(tab_id);
    }

    pub(super) fn enqueue_preview_load(&mut self, tab_id: usize, index: usize) {
        if let Some(bytes) = self
            .tab_by_id(tab_id)
            .and_then(|tab| Self::dds_by_index(tab, index))
            .map(|dds| dds.bytes.clone())
        {
            self.previews.request_thumbnail(tab_id, index, bytes);
        }
    }

    pub(super) fn enqueue_full_preview_load(&mut self, tab_id: usize, index: usize) {
        if let Some(bytes) = self
            .tab_by_id(tab_id)
            .and_then(|tab| Self::dds_by_index(tab, index))
            .map(|dds| dds.bytes.clone())
        {
            self.previews.request_full(tab_id, index, bytes);
        }
    }

    pub(super) fn process_preview_results(&mut self, ctx: &egui::Context) {
        for error in self.previews.process_results(ctx) {
            self.log(&error);
        }
    }

    pub(super) fn dispatch_preview_jobs(&mut self, ctx: &egui::Context) {
        self.previews.dispatch_jobs(ctx);
    }

    pub(super) fn active_full_preview_key(&self) -> Option<(usize, usize)> {
        if self.show_logs || self.show_about || self.show_tweaks {
            return None;
        }
        let tab = self.active_tab().filter(|tab| tab.show_full_preview)?;
        let index = tab.selected_index?;
        Self::dds_by_index(tab, index)?;
        Some((tab.id, index))
    }

    pub(super) const fn offer_tooltip(&mut self, id: egui::Id, text: &'static str) {
        self.frame_tooltip = Some((id, text));
    }

    pub(super) fn delayed_tooltip(
        &mut self,
        ctx: &egui::Context,
        candidate: Option<(egui::Id, &'static str)>,
    ) {
        let Some((id, text)) = candidate else {
            self.tooltip_id = None;
            self.tooltip_since = None;
            return;
        };

        match (self.tooltip_id, self.tooltip_since) {
            (Some(current), Some(since)) if current == id => {
                if since.elapsed() >= std::time::Duration::from_millis(900) {
                    let layer_id = egui::LayerId::new(
                        egui::Order::Tooltip,
                        egui::Id::new("delayed_tooltip_layer"),
                    );
                    egui::show_tooltip_at_pointer(ctx, layer_id, id, |ui| {
                        ui.label(egui::RichText::new(text));
                    });
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
            _ => {
                self.tooltip_id = Some(id);
                self.tooltip_since = Some(Instant::now());
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
    }

    fn sync_loaded_paths(&mut self) {
        self.loading.set_loaded_paths(
            self.tabs
                .iter()
                .filter(|tab| tab.folder_gallery.is_none())
                .map(|tab| tab.file.path.clone()),
        );
    }

    fn enqueue_load(&mut self, request: LoadRequest) {
        self.sync_loaded_paths();
        if let Err(error) = self.loading.enqueue(request) {
            self.report_error(error);
        }
    }

    pub(super) fn queue_file_load(&mut self, path: std::path::PathBuf) {
        if self.loading.pending_count() == 0 && self.activate_loaded_file(&path) {
            return;
        }
        self.enqueue_load(LoadRequest::File(path));
    }

    pub(super) fn process_load_results(&mut self, ctx: &egui::Context) {
        let results = self.loading.process_results();
        if !results.is_empty() {
            ctx.request_repaint();
        }
        for result in results {
            self.integrate_load_result(result);
        }
        if self.loading.pending_count() > 0 {
            self.sync_loaded_paths();
        }
    }

    fn integrate_load_result(&mut self, result: Result<LoadedContent, String>) {
        match result {
            Ok(LoadedContent::File(file)) => self.load_file(file),
            Ok(LoadedContent::ExistingFile(path)) => {
                if !self.activate_loaded_file(&path) {
                    self.loading.retry_file(path);
                }
            }
            Ok(LoadedContent::Gallery(build)) => self.load_gallery(build),
            Err(error) => self.report_error(error),
        }
    }

    fn activate_loaded_file(&mut self, path: &std::path::Path) -> bool {
        if let Some(existing_tab) = self
            .tabs
            .iter()
            .find(|tab| tab.folder_gallery.is_none() && tab.file.path == path)
        {
            self.active_tab_id = Some(existing_tab.id);
            self.log("This file is already loaded.");
            true
        } else {
            false
        }
    }

    pub(super) fn load_file(&mut self, file: TextureFile) {
        if self.activate_loaded_file(&file.path) {
            return;
        }
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;

        let dds_count = file.dds_list.len();

        let tab = Tab {
            id: tab_id,
            file,
            selected_index: (dds_count > 0).then_some(0),
            has_unsaved_changes: false,
            show_full_preview: false,
            full_preview_zoom: 1.0,
            full_preview_pan: Vec2::ZERO,
            replace_backups: HashMap::new(),
            folder_gallery: None,
        };

        self.tabs.push(tab);
        self.active_tab_id = Some(tab_id);

        if dds_count > 0 {
            self.enqueue_preview_load(tab_id, 0);
        }
        self.log(&format!("Loaded file with {dds_count} textures!"));
    }

    pub(super) fn load_folder_gallery(
        &mut self,
        folder_path: std::path::PathBuf,
        include_multi_texture: bool,
    ) {
        self.enqueue_load(LoadRequest::Folder {
            path: folder_path,
            include_multi_texture,
        });
    }

    pub(super) fn load_multi_file_gallery(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        include_multi_texture: bool,
    ) {
        self.enqueue_load(LoadRequest::Shared {
            paths,
            include_multi_texture,
        });
    }

    fn load_gallery(&mut self, build: GalleryBuild) {
        let GalleryBuild {
            path,
            is_folder,
            include_multi_texture,
            files,
            index_map,
            file_global_ranges,
            file_expanded,
            warnings,
        } = build;
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        let selected_index = (!index_map.is_empty()).then_some(0);
        let file = TextureFile {
            file_type: FileType::Textures,
            path,
            dds_list: Vec::new(),
            original_bytes: Vec::new(),
            streamtex_layout: None,
        };

        let tab = Tab {
            id: tab_id,
            file,
            selected_index,
            has_unsaved_changes: false,
            show_full_preview: false,
            full_preview_zoom: 1.0,
            full_preview_pan: Vec2::ZERO,
            replace_backups: HashMap::new(),
            folder_gallery: Some(FolderGallery {
                files,
                index_map,
                file_global_ranges,
                file_expanded,
                include_multi_texture,
                is_folder,
            }),
        };

        self.tabs.push(tab);
        self.active_tab_id = Some(tab_id);
        if let Some(index) = selected_index {
            self.enqueue_preview_load(tab_id, index);
        }
        self.log(if is_folder {
            "Loaded folder gallery!"
        } else {
            "Loaded shared gallery!"
        });
        if !warnings.is_empty() {
            self.report_error(format!(
                "Some gallery files could not be loaded:\n{}",
                warnings.join("\n")
            ));
        }
    }

    pub(super) fn close_tab(&mut self, tab_id: usize) {
        if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
            if tab.has_unsaved_changes {
                self.show_close_dialog = Some(tab_id);
                return;
            }
        }
        self.force_close_tab(tab_id);
    }

    pub(super) fn force_close_tab(&mut self, tab_id: usize) {
        let pos = self.tabs.iter().position(|t| t.id == tab_id);
        self.tabs.retain(|t| t.id != tab_id);
        self.clear_tab_previews(tab_id);
        if self.active_tab_id == Some(tab_id) {
            if let Some(pos) = pos {
                if self.tabs.is_empty() {
                    self.active_tab_id = None;
                } else if pos < self.tabs.len() {
                    self.active_tab_id = Some(self.tabs[pos].id);
                } else {
                    self.active_tab_id = Some(self.tabs[pos - 1].id);
                }
            } else {
                self.active_tab_id = self.tabs.last().map(|t| t.id);
            }
        }
        if self.batch_replace_tab_id == Some(tab_id) {
            self.batch_replace_tab_id = None;
            self.show_batch_replace_dialog = false;
            self.batch_replace_entries.clear();
            self.batch_replace_unused_sources.clear();
        }
    }

    pub(super) fn open_any_file(&mut self) {
        if let Some(paths) = FileDialog::new()
            .add_filter("Supported Files", &["textures", "streamtex"])
            .add_filter("All Files", &["*"])
            .pick_files()
        {
            for path in paths {
                self.queue_file_load(path);
            }
        }
    }

    pub(super) fn save_changes(&mut self) {
        let Some(tab_id) = self.active_tab_id else {
            return;
        };
        self.log("Saving changes...");

        let (result, sidecar_logs) = {
            let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
                self.log("Error: Could not find active tab during save.");
                return;
            };
            let patch_sidecar = self.streamtex_patch_sidecar;
            let mut sidecar_logs = Vec::new();
            let result = crate::runtime::run_guarded(|| {
                if let Some(gallery) = &tab.folder_gallery {
                    let mut changed_files = Vec::new();
                    for index in tab.replace_backups.keys() {
                        let (file_index, _) = gallery.index_map.get(*index).ok_or_else(|| {
                            format!("Cannot save missing gallery texture {index}.")
                        })?;
                        changed_files.push(*file_index);
                    }
                    changed_files.sort_unstable();
                    changed_files.dedup();
                    for file_index in changed_files {
                        let file = gallery.files.get(file_index).ok_or_else(|| {
                            format!("Cannot save missing gallery file {file_index}.")
                        })?;
                        if let Some(message) = Self::save_file_with_sidecar(file, patch_sidecar)? {
                            sidecar_logs.push(message);
                        }
                    }
                } else if let Some(message) =
                    Self::save_file_with_sidecar(&tab.file, patch_sidecar)?
                {
                    sidecar_logs.push(message);
                }
                Ok(())
            });
            (result, sidecar_logs)
        };

        for message in &sidecar_logs {
            self.log(message);
        }

        match result {
            Ok(()) => {
                if let Some(tab) = self.tab_mut_by_id(tab_id) {
                    tab.has_unsaved_changes = false;
                    tab.replace_backups.clear();
                    self.log("Saved changes successfully!");
                }
            }
            Err(err) => {
                self.report_error(format!("Error saving: {err}"));
            }
        }
    }

    fn save_file_with_sidecar(
        file: &TextureFile,
        patch_sidecar: bool,
    ) -> Result<Option<String>, String> {
        if patch_sidecar && matches!(file.file_type, FileType::Streamtex) {
            let sidecar = file.path.with_extension("textures");
            save_streamtex_with_sidecar(file, &file.path, &sidecar).map_err(|error| {
                format!(
                    "Could not save stream and sidecar {}: {error}",
                    sidecar.display()
                )
            })?;
            return Ok(Some(format!(
                "Saved paired .textures offsets: {}",
                sidecar.display()
            )));
        }
        match file.file_type {
            FileType::Textures => save_textures_file(file, &file.path),
            FileType::Streamtex => save_streamtex_file(file, &file.path),
        }
        .map_err(|error| format!("{}: {error}", file.path.display()))?;
        Ok(None)
    }

    pub(super) fn undo_selected_replace(&mut self) {
        let tab_id = self.active_tab_id;
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Some(index) = tab.selected_index else {
            return;
        };
        if Self::dds_by_index(tab, index).is_none() {
            return;
        }
        let Some(backup) = tab.replace_backups.remove(&index) else {
            return;
        };

        let name = backup.name.clone();
        if let Some(entry) = Self::dds_by_index_mut(tab, index) {
            *entry = backup;
        }
        tab.has_unsaved_changes = !tab.replace_backups.is_empty();
        if let Some(id) = tab_id {
            self.invalidate_preview(id, index);
        }
        if name.is_empty() {
            self.log("Undo replace");
        } else {
            self.log(&format!("Undo replace: {name}"));
        }
        self.error_message = None;
    }

    pub(super) fn selected_item(&self) -> Option<(usize, &DdsInfo)> {
        let tab = self.active_tab()?;
        let index = tab.selected_index?;
        Self::dds_by_index(tab, index).map(|dds| (index, dds))
    }

    /// Bytes written for an exported texture.
    ///
    /// Volume (3D) textures are flattened into a tall 2D strip, because image
    /// editors cannot open a DDS that carries `DDSCAPS2_VOLUME`; the layout is
    /// restored on Replace (see [`Self::prepare_replacement_dds`]).
    pub(super) fn export_bytes(dds: &DdsInfo) -> Vec<u8> {
        let flattened = super::volume::flatten_volume_to_atlas(dds.bytes.as_ref());
        let source = flattened.as_deref().unwrap_or(dds.bytes.as_ref());
        patch_legacy_fourcc(source)
    }

    pub(super) fn export_selected(&mut self) {
        let Some((index, dds)) = self
            .selected_item()
            .map(|(index, dds)| (index, dds.clone()))
        else {
            return;
        };

        if let Some(save_path) = FileDialog::new()
            .set_file_name(dds_export_name(&dds.name, index))
            .add_filter("DDS File", &["dds"])
            .save_file()
        {
            match fs::write(&save_path, Self::export_bytes(&dds)) {
                Ok(()) => self.log(&format!("Exported texture to {}", save_path.display())),
                Err(err) => self.error_message = Some(err.to_string()),
            }
        }
    }

    pub(super) fn export_selected_png(&mut self) {
        let Some((index, dds)) = self
            .selected_item()
            .map(|(index, dds)| (index, dds.clone()))
        else {
            return;
        };

        let file_name = if dds.name.is_empty() {
            format!("texture_{index}.png")
        } else {
            let stem = std::path::Path::new(&dds.name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&dds.name);
            format!("{stem}.png")
        };

        if let Some(save_path) = FileDialog::new()
            .set_file_name(&file_name)
            .add_filter("PNG Image", &["png"])
            .save_file()
        {
            match decode_dds_rgba(dds.bytes.as_ref()) {
                Ok((size, pixels)) => {
                    let Ok(w) = u32::try_from(size[0]) else {
                        self.error_message = Some("PNG width is too large.".to_string());
                        return;
                    };
                    let Ok(h) = u32::try_from(size[1]) else {
                        self.error_message = Some("PNG height is too large.".to_string());
                        return;
                    };
                    match image::save_buffer_with_format(
                        &save_path,
                        &pixels,
                        w,
                        h,
                        image::ColorType::Rgba8,
                        image::ImageFormat::Png,
                    ) {
                        Ok(()) => self.log(&format!("Exported PNG to {}", save_path.display())),
                        Err(err) => self.error_message = Some(err.to_string()),
                    }
                }
                Err(err) => self.error_message = Some(err),
            }
        }
    }

    pub(super) fn export_all_dds(&mut self) {
        let Some(tab) = self.active_tab() else { return };

        let Some(folder_path) = FileDialog::new().pick_folder() else {
            return;
        };

        let mut exported_count = 0usize;
        let mut used_names: HashSet<String> = HashSet::new();
        let total = Self::total_textures_in_tab(tab);
        for index in 0..total {
            let Some(dds) = Self::dds_by_index(tab, index) else {
                continue;
            };
            // Several entries can share one texture name; suffixing duplicates
            // with their global index keeps every export on disk and lets Batch
            // Replace route each file back to its exact entry.
            let base_name = dds_export_name(&dds.name, index);
            let file_name = unique_export_name(&base_name, index, &mut used_names);
            let save_path = folder_path.join(&file_name);
            match fs::write(&save_path, Self::export_bytes(dds)) {
                Ok(()) => exported_count += 1,
                Err(err) => {
                    self.error_message = Some(err.to_string());
                    self.log(&format!(
                        "Failed to export {}: {}",
                        save_path.display(),
                        err
                    ));
                    return;
                }
            }
        }

        self.log(&format!(
            "Exported {} DDS files to {}",
            exported_count,
            folder_path.display()
        ));
    }

    pub(super) fn replace_selected(&mut self) {
        let Some((tab_id, selected_index, name)) = self.active_tab().and_then(|tab| {
            let index = tab.selected_index?;
            let dds = Self::dds_by_index(tab, index)?;
            Some((tab.id, index, dds.name.clone()))
        }) else {
            return;
        };

        if let Some(file) = FileDialog::new()
            .set_file_name(&name)
            .add_filter("DDS File", &["dds"])
            .pick_file()
        {
            self.replace_from_path(tab_id, selected_index, &file);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn split_batch_hint_parses_index_suffix() {
        assert_eq!(split_batch_hint("foo_827"), Some(("foo", 827)));
        assert_eq!(split_batch_hint("texture_3"), Some(("texture", 3)));
        assert_eq!(split_batch_hint("banner_02"), Some(("banner", 2)));
        assert_eq!(split_batch_hint("foo"), None);
        assert_eq!(split_batch_hint("foo_"), None);
        assert_eq!(split_batch_hint("_827"), None);
        assert_eq!(split_batch_hint("foo_bar"), None);
        assert_eq!(split_batch_hint("foo_8x"), None);
        assert_eq!(split_batch_hint("foo_-1"), None);
    }

    #[test]
    fn unique_export_name_suffixes_duplicates() {
        let mut used: HashSet<String> = HashSet::new();

        assert_eq!(unique_export_name("foo.dds", 5, &mut used), "foo.dds");
        // Same name exported again (another entry sharing the name).
        assert_eq!(unique_export_name("foo.dds", 9, &mut used), "foo_9.dds");
        // A third collision, also colliding with an already used suffixed name.
        assert_eq!(unique_export_name("foo.dds", 9, &mut used), "foo_9_1.dds");
        // Distinct names are untouched.
        assert_eq!(unique_export_name("bar.dds", 2, &mut used), "bar.dds");
    }

    fn make_argb_dds(width: u32, height: u32, mips: u32) -> Vec<u8> {
        let mut b = vec![0_u8; 128];
        b[0..4].copy_from_slice(b"DDS ");
        b[4..8].copy_from_slice(&124_u32.to_le_bytes());
        b[8..12].copy_from_slice(&0x0000_100F_u32.to_le_bytes());
        b[12..16].copy_from_slice(&height.to_le_bytes());
        b[16..20].copy_from_slice(&width.to_le_bytes());
        b[20..24].copy_from_slice(&(width * 4).to_le_bytes());
        b[28..32].copy_from_slice(&mips.to_le_bytes());
        b[76..80].copy_from_slice(&32_u32.to_le_bytes());
        b[80..84].copy_from_slice(&0x41_u32.to_le_bytes());
        b[84..88].copy_from_slice(&0_u32.to_le_bytes());
        b[88..92].copy_from_slice(&32_u32.to_le_bytes());
        b[92..96].copy_from_slice(&0x00FF_0000_u32.to_le_bytes());
        b[96..100].copy_from_slice(&0x0000_FF00_u32.to_le_bytes());
        b[100..104].copy_from_slice(&0x0000_00FF_u32.to_le_bytes());
        b[104..108].copy_from_slice(&0xFF00_0000_u32.to_le_bytes());
        b[108..112].copy_from_slice(&0x1000_u32.to_le_bytes());
        b.resize(128 + (width as usize) * (height as usize) * 4 + 4096, 0xAA);
        b
    }

    fn make_dxt1_dds(width: u32, height: u32, mips: u32) -> Vec<u8> {
        let mut b = vec![0_u8; 128];
        b[0..4].copy_from_slice(b"DDS ");
        b[4..8].copy_from_slice(&124_u32.to_le_bytes());
        b[8..12].copy_from_slice(&0x0008_1007_u32.to_le_bytes());
        b[12..16].copy_from_slice(&height.to_le_bytes());
        b[16..20].copy_from_slice(&width.to_le_bytes());
        b[20..24].copy_from_slice(&(width.div_ceil(4) * height.div_ceil(4) * 8).to_le_bytes());
        b[28..32].copy_from_slice(&mips.to_le_bytes());
        b[76..80].copy_from_slice(&32_u32.to_le_bytes());
        b[80..84].copy_from_slice(&4_u32.to_le_bytes());
        b[84..88].copy_from_slice(b"DXT1");
        b[108..112].copy_from_slice(&0x1000_u32.to_le_bytes());
        b.resize(128 + (width as usize) * (height as usize) / 2 + 1024, 0xBB);
        b
    }

    fn orig_dds_info() -> DdsInfo {
        DdsInfo {
            name: "orig.dds".into(),
            bytes: make_dxt1_dds(8, 8, 1).into(),
            width: 8,
            height: 8,
            mipmap_count: 1,
            pixel_format: u32::from_le_bytes(*b"DXT1"),
            catalog_index: 0,
            entry_index: 0,
            modified: false,
        }
    }

    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "texembed_ui_test_{}_{}_{}",
                std::process::id(),
                stamp,
                id
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_file(path: &str, count: usize) -> TextureFile {
        TextureFile {
            file_type: FileType::Textures,
            path: PathBuf::from(path),
            dds_list: (0..count)
                .map(|index| DdsInfo {
                    entry_index: index,
                    ..orig_dds_info()
                })
                .collect(),
            original_bytes: b"load-time snapshot".to_vec(),
            streamtex_layout: None,
        }
    }

    fn prepared_for(original: &DdsInfo) -> PreparedReplacement {
        let mut bytes = original.bytes.to_vec();
        bytes[128] ^= 0xFF;
        PreparedReplacement {
            bytes,
            width: original.width,
            height: original.height,
            mipmap_count: original.mipmap_count,
            pixel_format: original.pixel_format,
            note: None,
        }
    }

    fn stream_fixture(path: &Path) -> TextureFile {
        let mut dds = make_dxt1_dds(8, 8, 1);
        dds.truncate(160);
        dds[4..8].copy_from_slice(&124_u32.to_le_bytes());
        dds[8..12].copy_from_slice(&0x0008_1007_u32.to_le_bytes());
        dds[20..24].copy_from_slice(&32_u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32_u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4_u32.to_le_bytes());
        dds[108..112].copy_from_slice(&0x1000_u32.to_le_bytes());
        let mut bytes = 160_u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&dds);
        fs::write(path, bytes).unwrap();
        texembed::load_streamtex_file(path.to_path_buf()).unwrap()
    }

    fn finish_loading(state: &mut AppState) {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.loading.pending_count() > 0 {
            state.loading.dispatch_jobs(&ctx);
            state.process_load_results(&ctx);
            assert!(Instant::now() < deadline, "UI loading did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn queued_load_results(state: &mut AppState) -> Vec<Result<LoadedContent, String>> {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            state.loading.dispatch_jobs(&ctx);
            let results = state.loading.process_results();
            if !results.is_empty() {
                return results;
            }
            assert!(
                Instant::now() < deadline,
                "UI loading worker did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn file_alias(path: &Path) -> PathBuf {
        let directory = path.parent().unwrap().join("alias");
        fs::create_dir(&directory).unwrap();
        directory.join("..").join(path.file_name().unwrap())
    }

    #[test]
    fn opening_requests_are_deferred_and_do_not_replace_earlier_requests() {
        let mut state = AppState::default();
        state.queue_file_load(PathBuf::from("missing.streamtex"));
        state.load_folder_gallery(PathBuf::from("missing-folder"), false);
        state.load_multi_file_gallery(vec![PathBuf::from("also-missing.textures")], true);
        state.process_load_results(&egui::Context::default());
        assert_eq!(state.loading.pending_count(), 3);
        assert!(state.tabs.is_empty());
        assert!(state.error_message.is_none());
    }

    #[test]
    fn worker_results_integrate_in_request_order_and_activate_duplicates() {
        let folder = TempFolder::new();
        let first = folder.0.join("first.streamtex");
        let second = folder.0.join("second.streamtex");
        stream_fixture(&first);
        stream_fixture(&second);
        let mut state = AppState::default();
        for path in [
            first.clone(),
            second.clone(),
            folder.0.join(".").join("first.streamtex"),
        ] {
            state.queue_file_load(path);
        }
        finish_loading(&mut state);
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.tabs[0].file.path, fs::canonicalize(first).unwrap());
        assert_eq!(state.tabs[1].file.path, fs::canonicalize(second).unwrap());
        assert_eq!(state.active_tab_id, Some(state.tabs[0].id));
        assert_eq!(state.tabs[0].selected_index, Some(0));
        assert!(state.error_message.is_none());
    }

    #[test]
    fn reopening_invalidated_exact_path_activates_without_io_and_preserves_edits() {
        let folder = TempFolder::new();
        let path = folder.0.join("open.streamtex");
        stream_fixture(&path);
        let mut state = AppState::default();
        state.queue_file_load(path.clone());
        finish_loading(&mut state);
        let target = state.active_tab_id.unwrap();
        let canonical = state.active_tab().unwrap().file.path.clone();
        let original = state.active_tab().unwrap().file.dds_list[0].clone();
        AppState::apply_prepared_replacement_to_index(
            state.active_tab_mut().unwrap(),
            0,
            &original,
            prepared_for(&original),
        )
        .unwrap();
        let edited = state.active_tab().unwrap().file.dds_list[0].bytes.clone();
        state.load_file(test_file("other.textures", 0));
        fs::write(&path, b"TEXS").unwrap();
        state.queue_file_load(canonical.clone());
        assert_eq!(state.loading.pending_count(), 0);
        assert_eq!(state.active_tab_id, Some(target));
        assert_eq!(state.tabs.len(), 2);
        let tab = state.active_tab().unwrap();
        assert_eq!(tab.selected_index, Some(0));
        assert!(tab.has_unsaved_changes);
        assert!(tab.file.dds_list[0].modified);
        assert!(std::sync::Arc::ptr_eq(&tab.file.dds_list[0].bytes, &edited));
        assert!(tab.replace_backups.contains_key(&0));
        fs::remove_file(&path).unwrap();
        state.active_tab_id = Some(state.tabs[1].id);
        state.queue_file_load(canonical);
        assert_eq!(state.active_tab_id, Some(target));
        assert_eq!(state.loading.pending_count(), 0);
        assert!(state.error_message.is_none());
    }

    #[test]
    fn reopening_invalidated_alias_activates_existing_tab_in_background() {
        let folder = TempFolder::new();
        let path = folder.0.join("open.streamtex");
        stream_fixture(&path);
        let alias = file_alias(&path);
        let mut state = AppState::default();
        state.queue_file_load(path.clone());
        finish_loading(&mut state);
        let target = state.active_tab_id.unwrap();
        let bytes = state.active_tab().unwrap().file.dds_list[0].bytes.clone();
        state.load_file(test_file("other.textures", 0));
        let other = state.active_tab_id;
        fs::write(path, b"TEXS").unwrap();
        state.queue_file_load(alias);
        assert_eq!(state.loading.pending_count(), 1);
        assert_eq!(state.active_tab_id, other);
        finish_loading(&mut state);
        assert_eq!(state.active_tab_id, Some(target));
        assert_eq!(state.tabs.len(), 2);
        assert!(std::sync::Arc::ptr_eq(
            &state.active_tab().unwrap().file.dds_list[0].bytes,
            &bytes
        ));
        assert!(state.error_message.is_none());
    }

    #[test]
    fn exact_duplicate_activation_waits_for_earlier_queued_requests() {
        let folder = TempFolder::new();
        let first = folder.0.join("first.streamtex");
        let second = folder.0.join("second.streamtex");
        stream_fixture(&first);
        stream_fixture(&second);
        let mut state = AppState::default();
        state.queue_file_load(first.clone());
        finish_loading(&mut state);
        let target = state.active_tab_id.unwrap();
        let canonical = state.active_tab().unwrap().file.path.clone();
        state.load_file(test_file("other.textures", 0));
        let other = state.active_tab_id;
        fs::write(first, b"TEXS").unwrap();
        state.queue_file_load(second.clone());
        state.queue_file_load(canonical);
        assert_eq!(state.active_tab_id, other);
        assert_eq!(state.loading.pending_count(), 2);
        finish_loading(&mut state);
        assert_eq!(state.tabs.len(), 3);
        assert_eq!(state.tabs[2].file.path, fs::canonicalize(second).unwrap());
        assert_eq!(state.active_tab_id, Some(target));
        assert!(state.error_message.is_none());
    }

    #[test]
    fn closing_tab_before_queued_alias_dispatch_reloads_in_request_order() {
        let folder = TempFolder::new();
        let first = folder.0.join("first.streamtex");
        let second = folder.0.join("second.streamtex");
        stream_fixture(&first);
        stream_fixture(&second);
        let alias = file_alias(&first);
        let mut state = AppState::default();
        state.queue_file_load(first.clone());
        finish_loading(&mut state);
        let closed = state.active_tab_id.unwrap();
        state.queue_file_load(alias);
        state.queue_file_load(second.clone());
        state.force_close_tab(closed);
        finish_loading(&mut state);
        assert_eq!(state.tabs.len(), 2);
        assert_ne!(state.tabs[0].id, closed);
        assert_eq!(state.tabs[0].file.path, fs::canonicalize(first).unwrap());
        assert_eq!(state.tabs[1].file.path, fs::canonicalize(second).unwrap());
        assert_eq!(state.active_tab_id, Some(state.tabs[1].id));
        assert!(state.error_message.is_none());
    }

    #[test]
    fn closing_tab_after_alias_resolution_retries_and_reports_invalid_disk_contents() {
        let folder = TempFolder::new();
        let first = folder.0.join("first.streamtex");
        let second = folder.0.join("second.streamtex");
        stream_fixture(&first);
        stream_fixture(&second);
        let alias = file_alias(&first);
        let mut state = AppState::default();
        state.queue_file_load(first.clone());
        finish_loading(&mut state);
        let closed = state.active_tab_id.unwrap();
        let canonical = state.active_tab().unwrap().file.path.clone();
        fs::write(&first, b"TEXS").unwrap();
        state.queue_file_load(alias);
        state.queue_file_load(second.clone());
        let results = queued_load_results(&mut state);
        assert!(matches!(
            &results[..],
            [Ok(LoadedContent::ExistingFile(path))] if path == &canonical
        ));
        state.force_close_tab(closed);
        for result in results {
            state.integrate_load_result(result);
        }
        assert_eq!(state.loading.pending_count(), 2);
        state.process_load_results(&egui::Context::default());
        assert!(state.tabs.is_empty());
        finish_loading(&mut state);
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.tabs[0].file.path, fs::canonicalize(second).unwrap());
        assert_eq!(state.active_tab_id, Some(state.tabs[0].id));
        let error = state.error_message.as_deref().unwrap();
        assert!(error.contains("first.streamtex"));
        assert!(error.contains("console TEXS"));
    }

    #[test]
    fn gallery_files_do_not_count_as_standalone_loaded_paths() {
        let folder = TempFolder::new();
        let path = folder.0.join("file.streamtex");
        stream_fixture(&path);
        let mut state = AppState::default();
        state.load_multi_file_gallery(vec![path.clone()], true);
        finish_loading(&mut state);
        let gallery = state.active_tab_id.unwrap();
        state.queue_file_load(fs::canonicalize(&path).unwrap());
        assert_eq!(state.loading.pending_count(), 1);
        finish_loading(&mut state);
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.tabs[0].id, gallery);
        assert!(state.tabs[0].folder_gallery.is_some());
        assert!(state.tabs[1].folder_gallery.is_none());
        assert_eq!(state.active_tab_id, Some(state.tabs[1].id));
        assert!(state.error_message.is_none());
    }

    #[test]
    fn successful_loads_do_not_hide_other_load_failures() {
        let mut state = AppState::default();
        state.integrate_load_result(Err("first load failed".to_string()));
        state.integrate_load_result(Ok(LoadedContent::File(test_file("loaded.textures", 0))));
        state.integrate_load_result(Err("another load failed".to_string()));
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.tabs[0].selected_index, None);
        let error = state.error_message.as_deref().unwrap();
        assert!(error.contains("first load failed"));
        assert!(error.contains("another load failed"));
    }

    #[test]
    fn duplicate_load_keeps_selection_and_unsaved_edits() {
        let mut state = AppState::default();
        let file = test_file("same.textures", 2);
        state.load_file(file.clone());
        let tab = state.active_tab_mut().unwrap();
        tab.selected_index = Some(1);
        let original = AppState::dds_by_index(tab, 1).unwrap().clone();
        AppState::apply_prepared_replacement_to_index(tab, 1, &original, prepared_for(&original))
            .unwrap();
        state.load_file(file);
        assert_eq!(state.tabs.len(), 1);
        let tab = state.active_tab().unwrap();
        assert_eq!(tab.selected_index, Some(1));
        assert!(tab.has_unsaved_changes);
        assert!(AppState::dds_by_index(tab, 1).unwrap().modified);
        assert!(tab.replace_backups.contains_key(&1));
    }

    #[test]
    fn gallery_integration_preserves_global_indices_and_collapsed_sections() {
        let mut state = AppState::default();
        let build = GalleryBuild {
            path: PathBuf::from("synthetic-folder"),
            is_folder: true,
            include_multi_texture: true,
            files: vec![
                test_file("first.textures", 2),
                test_file("second.textures", 1),
            ],
            index_map: vec![(0, 0), (0, 1), (1, 0)],
            file_global_ranges: vec![0..2, 2..3],
            file_expanded: vec![false, true],
            warnings: vec!["skipped invalid file".to_string()],
        };
        state.integrate_load_result(Ok(LoadedContent::Gallery(build)));
        let tab = state.active_tab().unwrap();
        assert_eq!(tab.selected_index, Some(0));
        assert_eq!(AppState::total_textures_in_tab(tab), 3);
        assert_eq!(
            AppState::original_filename_in_tab(tab, 2).as_deref(),
            Some("second.textures")
        );
        assert!(tab.file.streamtex_layout.is_none());
        let gallery = tab.folder_gallery.as_ref().unwrap();
        assert_eq!(gallery.file_global_ranges, vec![0..2, 2..3]);
        assert_eq!(gallery.file_expanded, vec![false, true]);
        assert!(gallery.is_folder);
        assert!(state
            .error_message
            .as_deref()
            .unwrap()
            .contains("skipped invalid file"));
    }

    #[test]
    fn replacement_and_undo_restore_the_previous_modified_flag() {
        for modified in [false, true] {
            let mut state = AppState::default();
            let mut file = test_file("undo.textures", 1);
            file.dds_list[0].modified = modified;
            let original = file.dds_list[0].clone();
            state.load_file(file);
            let tab = state.active_tab_mut().unwrap();
            AppState::apply_prepared_replacement_to_index(
                tab,
                0,
                &original,
                prepared_for(&original),
            )
            .unwrap();
            assert!(tab.file.dds_list[0].modified);
            assert!(tab.has_unsaved_changes);
            assert_eq!(tab.replace_backups[&0].modified, modified);
            state.undo_selected_replace();
            let tab = state.active_tab().unwrap();
            assert_eq!(tab.file.dds_list[0].modified, modified);
            assert!(std::sync::Arc::ptr_eq(
                &tab.file.dds_list[0].bytes,
                &original.bytes
            ));
            assert!(!tab.has_unsaved_changes);
            assert!(tab.replace_backups.is_empty());
        }
    }

    #[test]
    fn invalid_replacement_index_does_not_create_a_backup_or_mark_dirty() {
        let mut state = AppState::default();
        state.load_file(test_file("invalid-index.textures", 1));
        let tab = state.active_tab_mut().unwrap();
        let original = tab.file.dds_list[0].clone();
        assert!(AppState::apply_prepared_replacement_to_index(
            tab,
            5,
            &original,
            prepared_for(&original),
        )
        .is_none());
        assert!(!tab.has_unsaved_changes);
        assert!(tab.replace_backups.is_empty());
    }

    #[test]
    fn dialog_and_drop_replacement_helper_targets_the_requested_tab() {
        let folder = TempFolder::new();
        let mut state = AppState::default();
        state.load_file(test_file("target.textures", 1));
        let target = state.active_tab_id.unwrap();
        let original = state.active_tab().unwrap().file.dds_list[0].clone();
        let replacement = folder.0.join("replacement.dds");
        fs::write(&replacement, prepared_for(&original).bytes).unwrap();
        state.load_file(test_file("other.textures", 1));
        state.replace_from_path(target, 0, &replacement);
        assert!(state.tab_by_id(target).unwrap().file.dds_list[0].modified);
        assert!(state.tab_by_id(target).unwrap().has_unsaved_changes);
        assert!(!state.active_tab().unwrap().has_unsaved_changes);
        assert!(!state.active_tab().unwrap().file.dds_list[0].modified);
    }

    #[test]
    fn batch_apply_revalidates_current_options_and_marks_successful_entries_modified() {
        let folder = TempFolder::new();
        let source = folder.0.join("orig.dds");
        fs::write(&source, make_dxt1_dds(16, 8, 1)).unwrap();
        let mut state = AppState {
            resize_on_replace: true,
            batch_replace_folder: Some(folder.0.clone()),
            ..Default::default()
        };
        state.load_file(test_file("batch.textures", 1));
        state.batch_replace_tab_id = state.active_tab_id;
        state.scan_batch_replace();
        assert!(state.batch_replace_entries[0].is_ready());
        state.resize_on_replace = false;
        state.apply_batch_replace();
        assert!(!state.active_tab().unwrap().has_unsaved_changes);
        assert!(!state.active_tab().unwrap().file.dds_list[0].modified);
        state.resize_on_replace = true;
        let mut latest = make_dxt1_dds(16, 8, 1);
        latest[128] = 0x12;
        fs::write(&source, &latest).unwrap();
        state.apply_batch_replace();
        let tab = state.active_tab().unwrap();
        assert!(tab.has_unsaved_changes);
        assert!(tab.file.dds_list[0].modified);
        assert_eq!(tab.file.dds_list[0].width, 16);
        assert_eq!(tab.file.dds_list[0].bytes[128], 0x12);
    }

    #[test]
    fn sidecar_save_failure_keeps_dirty_state_backups_and_snapshot() {
        let folder = TempFolder::new();
        let path = folder.0.join("paired.streamtex");
        let file = stream_fixture(&path);
        fs::write(path.with_extension("textures"), b"TEXS").unwrap();
        let snapshot = file.original_bytes.clone();
        let original = file.dds_list[0].clone();
        let mut state = AppState {
            streamtex_patch_sidecar: true,
            ..Default::default()
        };
        state.load_file(file);
        AppState::apply_prepared_replacement_to_index(
            state.active_tab_mut().unwrap(),
            0,
            &original,
            prepared_for(&original),
        )
        .unwrap();
        state.save_changes();
        let tab = state.active_tab().unwrap();
        assert!(tab.has_unsaved_changes);
        assert!(tab.replace_backups.contains_key(&0));
        assert!(tab.file.dds_list[0].modified);
        assert_eq!(tab.file.original_bytes, snapshot);
        assert!(state.error_message.as_deref().unwrap().contains("sidecar"));
    }

    #[test]
    fn successful_save_keeps_modified_flags_and_next_undo_restores_saved_entry() {
        let folder = TempFolder::new();
        let file = stream_fixture(&folder.0.join("save.streamtex"));
        let snapshot = file.original_bytes.clone();
        let original = file.dds_list[0].clone();
        let mut state = AppState::default();
        state.load_file(file);
        AppState::apply_prepared_replacement_to_index(
            state.active_tab_mut().unwrap(),
            0,
            &original,
            prepared_for(&original),
        )
        .unwrap();
        state.save_changes();
        assert!(state.error_message.is_none());
        let tab = state.active_tab().unwrap();
        assert!(!tab.has_unsaved_changes);
        assert!(tab.replace_backups.is_empty());
        assert!(tab.file.dds_list[0].modified);
        assert_eq!(tab.file.original_bytes, snapshot);
        let saved = tab.file.dds_list[0].clone();
        AppState::apply_prepared_replacement_to_index(
            state.active_tab_mut().unwrap(),
            0,
            &saved,
            prepared_for(&saved),
        )
        .unwrap();
        state.undo_selected_replace();
        let tab = state.active_tab().unwrap();
        assert!(tab.file.dds_list[0].modified);
        assert!(std::sync::Arc::ptr_eq(
            &tab.file.dds_list[0].bytes,
            &saved.bytes
        ));
        assert!(!tab.has_unsaved_changes);
    }

    #[test]
    fn full_preview_key_exists_only_for_the_visible_valid_selection() {
        let mut state = AppState::default();
        state.load_file(test_file("preview.textures", 1));
        let id = state.active_tab_id.unwrap();
        assert_eq!(state.active_full_preview_key(), None);
        state.active_tab_mut().unwrap().show_full_preview = true;
        assert_eq!(state.active_full_preview_key(), Some((id, 0)));
        state.show_logs = true;
        assert_eq!(state.active_full_preview_key(), None);
        state.show_logs = false;
        state.show_about = true;
        assert_eq!(state.active_full_preview_key(), None);
        state.show_about = false;
        state.show_tweaks = true;
        assert_eq!(state.active_full_preview_key(), None);
        state.show_tweaks = false;
        state.active_tab_mut().unwrap().selected_index = Some(10);
        assert_eq!(state.active_full_preview_key(), None);
        state.force_close_tab(id);
        assert_eq!(state.active_full_preview_key(), None);
    }

    #[test]
    fn streamtex_argb_full_mips_is_reduced_to_mip0() {
        let argb = make_argb_dds(8, 8, 3);
        let prep = AppState::prepare_replacement_dds(
            &FileType::Streamtex,
            &orig_dds_info(),
            &argb,
            false,
            true,
            false,
        )
        .unwrap();

        assert_eq!(prep.mipmap_count, 1);
        assert_eq!(prep.bytes.len(), 128 + 8 * 8 * 4);
        let mips = u32::from_le_bytes(prep.bytes[28..32].try_into().unwrap());
        let pitch = u32::from_le_bytes(prep.bytes[20..24].try_into().unwrap());
        assert_eq!(mips, 1);
        assert_eq!(pitch, 8 * 4);
        let note = prep.note.as_ref().expect("strip note");
        assert!(note.contains("mip 0"), "note: {note}");
    }

    #[test]
    fn streamtex_argb_mip0_passthrough_is_unchanged() {
        let argb = make_argb_dds(8, 8, 1);
        let prep = AppState::prepare_replacement_dds(
            &FileType::Streamtex,
            &orig_dds_info(),
            &argb,
            false,
            true,
            false,
        )
        .unwrap();

        // A mip-0-only ARGB DDS must pass through byte-for-byte (no strip).
        assert_eq!(prep.mipmap_count, 1);
        assert_eq!(prep.bytes.len(), argb.len());
        let mips = u32::from_le_bytes(prep.bytes[28..32].try_into().unwrap());
        assert_eq!(mips, 1);
        if let Some(note) = &prep.note {
            assert!(!note.contains("mip 0"), "unexpected strip note: {note}");
        }
    }

    #[test]
    fn streamtex_dxt_full_mips_is_kept() {
        let dxt = make_dxt1_dds(8, 8, 3);
        let prep = AppState::prepare_replacement_dds(
            &FileType::Streamtex,
            &orig_dds_info(),
            &dxt,
            false,
            true,
            false,
        )
        .unwrap();

        assert_eq!(prep.mipmap_count, 3);
        assert_eq!(prep.bytes.len(), dxt.len());
        let mips = u32::from_le_bytes(prep.bytes[28..32].try_into().unwrap());
        assert_eq!(mips, 3);
    }
}
