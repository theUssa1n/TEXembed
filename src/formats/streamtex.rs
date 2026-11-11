use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};

use super::textures::{
    dds_record_pixel_format, load_streamed_texture_entries, StreamedTextureEntry,
};
use crate::dds::parse_dds_header;
use crate::model::{
    DdsInfo, FileType, StreamtexBinding, StreamtexLayout, StreamtexRecord, TextureFile,
};

/// `DDSCAPS2_CUBEMAP`: set in the DDS header when a texture has six faces.
const DDSCAPS2_CUBEMAP: u32 = 0x200;
/// `DDSCAPS2_VOLUME`: set in the DDS header when a texture has depth slabs.
const DDSCAPS2_VOLUME: u32 = 0x0020_0000;
/// Size of the fixed DDS header that precedes every payload.
const DDS_HEADER_LEN: usize = 128;
/// `DDPF_RGB`: the pixel format section describes uncompressed colour data.
const DDPF_RGB: u32 = 0x0040;

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Block layout of a DDS payload as `(block edge, bytes per block)`.
fn dds_block_layout(dds_bytes: &[u8]) -> Option<(u32, usize)> {
    match &dds_bytes[84..88] {
        b"DXT1" | b"ATI1" | b"BC4U" | b"BC4S" => Some((4, 8)),
        b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5" | b"ATI2" | b"BC5U" | b"BC5S" => Some((4, 16)),
        // DX10 keeps its format in an extra header this module does not parse.
        b"DX10" => None,
        _ => {
            if u32_at(dds_bytes, 80) & DDPF_RGB == 0 {
                return None;
            }
            let bits = u32_at(dds_bytes, 88);
            if bits == 0 || !bits.is_multiple_of(8) {
                return None;
            }
            Some((1, usize::try_from(bits / 8).ok()?))
        }
    }
}

/// Exact payload size of a DDS blob, derived from its own header.
///
/// The length prefix of a `.streamtex` record is not always trustworthy: a few
/// retail records store a value that is a handful of bytes off, which
/// desynchronises every record that follows. The DDS header fully describes the
/// payload, so the size is recomputed here: the fixed header counted once, plus
/// the mip chain multiplied by the face (and slice) count.
fn dds_payload_len(dds_bytes: &[u8]) -> Option<usize> {
    if dds_bytes.len() < DDS_HEADER_LEN || &dds_bytes[0..4] != b"DDS " {
        return None;
    }
    let (edge, block_bytes) = dds_block_layout(dds_bytes)?;
    let width = u32_at(dds_bytes, 16).max(1);
    let height = u32_at(dds_bytes, 12).max(1);
    let mipmap_count = u32_at(dds_bytes, 28).max(1);
    if mipmap_count > 32 {
        return None;
    }
    let caps2 = u32_at(dds_bytes, 112);
    let sides = usize::try_from(dds_sides(dds_bytes)).ok()?;
    let depth = if caps2 & DDSCAPS2_VOLUME != 0 {
        usize::try_from(u32_at(dds_bytes, 24).max(1)).ok()?
    } else {
        1
    };

    let mut chain = 0usize;
    let mut current_width = width;
    let mut current_height = height;
    let mut current_depth = depth;
    for _ in 0..mipmap_count {
        let blocks_w = usize::try_from(current_width.div_ceil(edge)).ok()?;
        let blocks_h = usize::try_from(current_height.div_ceil(edge)).ok()?;
        let level = blocks_w
            .checked_mul(blocks_h)?
            .checked_mul(block_bytes)?
            .checked_mul(current_depth)?;
        chain = chain.checked_add(level)?;
        current_width = (current_width / 2).max(1);
        current_height = (current_height / 2).max(1);
        current_depth = (current_depth / 2).max(1);
    }

    DDS_HEADER_LEN.checked_add(chain.checked_mul(sides)?)
}

/// Whether `at` is a position where a new record can begin: the start of the
/// next `[len]["DDS "]` pair, or the end of the file.
fn record_boundary(bytes: &[u8], at: usize) -> bool {
    at == bytes.len()
        || at
            .checked_add(8)
            .and_then(|end| bytes.get(at + 4..end))
            .is_some_and(|magic| magic == b"DDS ")
}

/// Shape of a `.streamtex` record: `sides`, `width`, `height`, `mipmap_count`,
/// game pixel-format code. Both sides of the pairing describe the same
/// payload, so this is the field-based key used to line them up.
type RecordKey = (u32, u32, u32, u32, u32);

fn load_streamtex_sidecar_entries(path: &Path) -> Option<Vec<StreamedTextureEntry>> {
    let textures_path = path.with_extension("textures");
    if !textures_path.is_file() {
        return None;
    }

    load_streamed_texture_entries(&textures_path)
        .ok()
        .filter(|entries| !entries.is_empty())
}

/// Reads the number of faces from a DDS header (`1` for 2D, `6` for cubemaps).
fn dds_sides(dds_bytes: &[u8]) -> u32 {
    const CAPS2_OFFSET: usize = 112;
    if dds_bytes.len() < CAPS2_OFFSET + 4 {
        return 1;
    }
    let caps2 = u32::from_le_bytes([
        dds_bytes[CAPS2_OFFSET],
        dds_bytes[CAPS2_OFFSET + 1],
        dds_bytes[CAPS2_OFFSET + 2],
        dds_bytes[CAPS2_OFFSET + 3],
    ]);
    if (caps2 & DDSCAPS2_CUBEMAP) != 0 {
        6
    } else {
        1
    }
}

/// Builds the shape key of a `.streamtex` record from its DDS payload.
fn dds_record_key(dds_bytes: &[u8], header: (u32, u32, u32, u32)) -> RecordKey {
    let (width, height, mipmap_count, _) = header;
    (
        dds_sides(dds_bytes),
        width,
        height,
        mipmap_count,
        dds_record_pixel_format(dds_bytes),
    )
}

/// Builds the shape key of a sidecar entry.
fn entry_record_key(entry: &StreamedTextureEntry) -> RecordKey {
    (
        entry.sides,
        entry.width,
        entry.height,
        entry.mipmap_count,
        entry.pixel_format,
    )
}

/// Finds the global entry offset that best lines the records up with the
/// sidecar entries.
///
/// The sidecar lists streamed entries in file order but may also contain
/// streamed entries whose payload is stored elsewhere, so the records of one
/// `.streamtex` file can start a fixed number of entries into that list.
/// Both sides describe the same payload, so the offset whose keys mismatch
/// least is the right one. Shape fields decide the winner; the pixel format is
/// only a tie-breaker because it cannot always be mapped back from the DDS
/// header.
fn best_entry_shift(records: &[RecordKey], entries: &[RecordKey]) -> usize {
    let max_shift = entries.len().saturating_sub(records.len());
    let mut best_shift = 0;
    let mut best_score: Option<(usize, usize, usize)> = None;

    for shift in 0..=max_shift {
        let mut shape_mismatches = 0;
        let mut format_mismatches = 0;

        for (record, entry) in records.iter().zip(&entries[shift..]) {
            if record.0 != entry.0
                || record.1 != entry.1
                || record.2 != entry.2
                || record.3 != entry.3
            {
                shape_mismatches += 1;
            } else if record.4 != entry.4 {
                format_mismatches += 1;
            }
        }

        let score = (shape_mismatches, format_mismatches, shift);
        if best_score.is_none_or(|best| score < best) {
            best_score = Some(score);
            best_shift = shift;
        }
    }

    best_shift
}

/// Pairs every `.streamtex` record with its sidecar entry name.
///
/// The `.textures` sidecar interleaves embedded and streamed entries, so entry
/// order alone does not reliably line up with the `.streamtex` records. Both
/// sides, however, describe the same payload (`sides`, `width`, `height`,
/// `mipmap_count`), so the records are shifted against the entry list by the
/// offset that matches best.
fn map_sidecar_names(
    records: &[RecordKey],
    entries: &[StreamedTextureEntry],
) -> Vec<Option<String>> {
    let entry_keys: Vec<RecordKey> = entries.iter().map(entry_record_key).collect();
    let shift = best_entry_shift(records, &entry_keys);

    (0..records.len())
        .map(|index| entries.get(index + shift).map(|entry| entry.name.clone()))
        .collect()
}

fn matching_shape(record: RecordKey, entry: RecordKey) -> bool {
    record.0 == entry.0
        && record.1 == entry.1
        && record.2 == entry.2
        && record.3 == entry.3
        && (record.4 == 0 || entry.4 == 0 || record.4 == entry.4)
}

fn bind_sidecar(
    layout: &StreamtexLayout,
    entries: &[StreamedTextureEntry],
) -> Vec<StreamtexBinding> {
    let mut offset = 0usize;
    let records: Vec<_> = layout
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let current = offset;
            offset += 4 + record.bytes.len();
            let key = parse_dds_header(&record.bytes)
                .ok()
                .map(|header| dds_record_key(&record.bytes, header));
            (index, current, key)
        })
        .collect();
    let mut bindings = HashMap::new();
    let mut used = std::collections::HashSet::new();
    for (record_index, record_offset, key) in &records {
        let mut candidates = entries.iter().enumerate().filter(|(_, entry)| {
            entry.data_offset as usize == *record_offset
                && key.is_none_or(|key| matching_shape(key, entry_record_key(entry)))
        });
        if let Some((entry_index, _)) = candidates.next() {
            if candidates.next().is_none() && used.insert(entry_index) {
                bindings.insert(*record_index, entry_index);
            }
        }
    }
    let valid: Vec<_> = records
        .iter()
        .filter_map(|(index, _, key)| key.map(|key| (*index, key)))
        .collect();
    if entries.len() >= valid.len() && !valid.is_empty() {
        let mut shifts = (0..=entries.len() - valid.len()).filter(|shift| {
            valid.iter().enumerate().all(|(position, (index, key))| {
                let entry_index = position + shift;
                matching_shape(*key, entry_record_key(&entries[entry_index]))
                    && bindings
                        .get(index)
                        .is_none_or(|bound| *bound == entry_index)
                    && (!used.contains(&entry_index) || bindings.get(index) == Some(&entry_index))
            })
        });
        if let Some(shift) = shifts.next() {
            if shifts.next().is_none() {
                for (position, (index, _)) in valid.iter().enumerate() {
                    bindings.insert(*index, position + shift);
                }
            }
        }
    }
    let mut result: Vec<_> = bindings
        .into_iter()
        .map(|(record_index, entry)| StreamtexBinding {
            record_index,
            catalog_index: entries[entry].catalog_index,
            entry_index: entries[entry].entry_index,
            record_id: entries[entry].record_id,
        })
        .collect();
    result.sort_by_key(|binding| binding.record_index);
    result
}

/// Splits a `.streamtex` blob into its payloads.
///
/// A record is `[u32 little-endian length][DDS payload]`, but a handful of
/// retail records store a length that is a few bytes off. Advancing by such a
/// value desynchronises every record that follows, so each boundary is
/// validated against the DDS magic of the record that should start there: the
/// candidate size that actually lands on a record boundary wins, and because
/// the DDS header fully describes its payload, that recomputed size is
/// preferred whenever both are plausible.
fn read_streamtex_records(bytes: &[u8]) -> StreamtexLayout {
    let mut records = Vec::new();
    let mut position = 0usize;

    while bytes.len() - position >= 4 {
        let payload_start = position + 4;
        let available = bytes.len() - payload_start;
        if available == 0 {
            break;
        }

        let declared = u32_at(bytes, position) as usize;
        let computed = if bytes[payload_start..].starts_with(b"DDS ") {
            dds_payload_len(&bytes[payload_start..]).filter(|len| *len <= available)
        } else {
            None
        };

        let computed_len = computed.filter(|len| record_boundary(bytes, payload_start + len));
        let declared_len = (declared <= available
            && record_boundary(bytes, payload_start + declared))
        .then_some(declared);

        let payload_len = match (computed_len, declared_len) {
            (Some(len), _) => len,
            (None, Some(len)) => len,
            // Neither candidate lands on a boundary: fall back to the most
            // trustworthy size known, or stop when nothing is usable.
            (None, None) => match (declared <= available).then_some(declared).or(computed) {
                Some(len) => len,
                None => break,
            },
        };

        records.push(StreamtexRecord {
            length_prefix: u32_at(bytes, position).to_le_bytes(),
            bytes: Arc::from(&bytes[payload_start..payload_start + payload_len]),
        });
        position = payload_start + payload_len;
    }

    StreamtexLayout {
        records,
        trailing_bytes: Arc::from(&bytes[position..]),
        sidecar_bindings: Vec::new(),
    }
}

/// Loads a `.streamtex` file and extracts embedded DDS payloads.
///
/// # Errors
///
/// Returns an error when the file cannot be read.
pub fn load_streamtex_file(path: PathBuf) -> Result<TextureFile> {
    let original_bytes = fs::read(&path)?;
    let mut layout = read_streamtex_records(&original_bytes);
    let sidecar_entries = load_streamtex_sidecar_entries(&path);
    if let Some(entries) = &sidecar_entries {
        layout.sidecar_bindings = bind_sidecar(&layout, entries);
    }

    let headers: Vec<Option<(u32, u32, u32, u32)>> = layout
        .records
        .iter()
        .map(|record| parse_dds_header(&record.bytes).ok())
        .collect();
    let names = sidecar_entries.as_deref().map(|entries| {
        let records: Vec<RecordKey> = layout
            .records
            .iter()
            .zip(&headers)
            .filter_map(|(record, header)| {
                header.map(|header| dds_record_key(&record.bytes, header))
            })
            .collect();
        map_sidecar_names(&records, entries)
    });

    let mut dds_list = Vec::new();
    let mut name_index = 0;

    for (i, record) in layout.records.iter().enumerate() {
        if let Some((width, height, mipmap_count, pixel_format)) = headers[i] {
            let bound_name = layout
                .sidecar_bindings
                .iter()
                .find(|binding| binding.record_index == i)
                .and_then(|binding| {
                    sidecar_entries.as_ref()?.iter().find(|entry| {
                        entry.catalog_index == binding.catalog_index
                            && entry.entry_index == binding.entry_index
                    })
                })
                .map(|entry| &entry.name);
            let name = bound_name
                .or_else(|| {
                    names
                        .as_ref()
                        .and_then(|names| names.get(name_index))
                        .and_then(|name| name.as_ref())
                })
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("{i}.dds"));
            name_index += 1;
            dds_list.push(DdsInfo {
                name,
                bytes: Arc::clone(&record.bytes),
                width,
                height,
                mipmap_count,
                pixel_format,
                catalog_index: 0,
                entry_index: i,
                modified: false,
            });
        }
    }

    if dds_list.is_empty() {
        return Err(anyhow!("[Streamtex file] No recognizable DDS records; the file may be unsupported or malformed"));
    }

    Ok(TextureFile {
        file_type: FileType::Streamtex,
        path,
        dds_list,
        original_bytes: Vec::new(),
        streamtex_layout: Some(layout),
    })
}

/// Saves a `.streamtex` file from the in-memory DDS list.
///
/// # Errors
///
/// Returns an error when a DDS payload length does not fit in `u32` or the
/// output file cannot be written.
pub fn save_streamtex_file(texture_file: &TextureFile, path: &Path) -> Result<()> {
    let result_bytes = serialize_streamtex(texture_file)?;
    super::storage::atomic_write(path, &result_bytes)
}

pub fn save_streamtex_with_sidecar(
    texture_file: &TextureFile,
    path: &Path,
    sidecar: &Path,
) -> Result<()> {
    let stream_bytes = serialize_streamtex(texture_file)?;
    let sidecar_bytes = fs::read(sidecar).map_err(|error| {
        anyhow!(
            "[Save Streamtex] Cannot read sidecar {}: {error}; no files were saved",
            sidecar.display()
        )
    })?;
    let patched_sidecar = super::textures::serialize_bound_sidecar(texture_file, &sidecar_bytes)?;
    super::storage::write_pair((path, &stream_bytes), (sidecar, &patched_sidecar))
}

pub(super) fn streamtex_replacements(
    texture_file: &TextureFile,
) -> Result<HashMap<usize, &DdsInfo>> {
    let layout = texture_file.streamtex_layout.as_ref().ok_or_else(|| {
        anyhow!("[Save Streamtex] Original record layout is missing; reopen the file before saving")
    })?;
    let mut entries = HashMap::new();
    for dds in &texture_file.dds_list {
        if dds.catalog_index != 0 || dds.entry_index >= layout.records.len() {
            return Err(anyhow!(
                "[Save Streamtex] Invalid record index {}",
                dds.entry_index
            ));
        }
        if entries.insert(dds.entry_index, dds).is_some() {
            return Err(anyhow!(
                "[Save Streamtex] Duplicate record index {}",
                dds.entry_index
            ));
        }
        if dds.modified {
            parse_dds_header(&dds.bytes)?;
        }
    }
    Ok(entries)
}

pub(super) fn serialize_streamtex(texture_file: &TextureFile) -> Result<Vec<u8>> {
    if texture_file.file_type != FileType::Streamtex {
        return Err(anyhow!("[Save Streamtex] Expected a .streamtex document"));
    }
    let replacements = streamtex_replacements(texture_file)?;
    let layout = texture_file
        .streamtex_layout
        .as_ref()
        .ok_or_else(|| anyhow!("Missing stream layout"))?;
    let mut result_bytes = Vec::new();
    for (index, record) in layout.records.iter().enumerate() {
        match replacements
            .get(&index)
            .filter(|dds| dds.modified && dds.bytes != record.bytes)
        {
            Some(dds) => {
                let length = u32::try_from(dds.bytes.len()).map_err(|_| {
                    anyhow!("[Save Streamtex] DDS payload exceeds the u32 size limit")
                })?;
                result_bytes.extend_from_slice(&length.to_le_bytes());
                result_bytes.extend_from_slice(&dds.bytes);
            }
            None => {
                result_bytes.extend_from_slice(&record.length_prefix);
                result_bytes.extend_from_slice(&record.bytes);
            }
        }
    }
    result_bytes.extend_from_slice(&layout.trailing_bytes);
    Ok(result_bytes)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    use byteorder::{LittleEndian, WriteBytesExt};

    use super::load_streamtex_file;

    fn make_test_dds(width: u32, height: u32, mipmap_count: u32, pixel_format: u32) -> Vec<u8> {
        make_test_dds_with_sides(width, height, mipmap_count, pixel_format, 1)
    }

    fn make_test_dds_with_sides(
        width: u32,
        height: u32,
        mipmap_count: u32,
        pixel_format: u32,
        sides: u32,
    ) -> Vec<u8> {
        let mut bytes = vec![0_u8; 128];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&mipmap_count.to_le_bytes());
        bytes[84..88].copy_from_slice(&pixel_format.to_le_bytes());
        if sides == 6 {
            bytes[112..116].copy_from_slice(&0x200_u32.to_le_bytes());
        }
        bytes
    }

    fn make_test_streamtex(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for entry in entries {
            bytes.extend_from_slice(&u32::try_from(entry.len()).unwrap().to_le_bytes());
            bytes.extend_from_slice(entry);
        }
        bytes
    }

    /// Builds a `.textures` file whose streamed catalog (`tex_data_size == 0`)
    /// holds one 0x40 static record per
    /// `(name, sides, width, height, mipmap_count, pixel_format)` entry.
    fn make_test_textures(entries: &[(&str, u32, u32, u32, u32, u32)]) -> Vec<u8> {
        const TEXTURES_MAGIC: u32 = 0x5445_5853;
        const NAME_TABLE_MARKER: u32 = 0x4332_4E4D;
        const BASE_OFFSET: usize = 0x10;
        const CAT_OFF: u32 = 0x18;
        const RECORD_SIZE: u32 = 0x40;

        let base_offset = u32::try_from(BASE_OFFSET).unwrap();
        let entry_count = u32::try_from(entries.len()).unwrap();
        let entries_offset = CAT_OFF + 16;
        let records_offset = entries_offset + entry_count * 12;
        let table_size = records_offset + entry_count * RECORD_SIZE;
        let mut bytes = vec![0_u8; BASE_OFFSET + usize::try_from(table_size).unwrap()];
        let mut cursor = Cursor::new(&mut bytes);

        cursor.write_u32::<LittleEndian>(TEXTURES_MAGIC).unwrap();
        cursor.set_position(0xC);
        cursor.write_u32::<LittleEndian>(table_size).unwrap();
        cursor.set_position(u64::try_from(BASE_OFFSET).unwrap());
        cursor.write_u32::<LittleEndian>(entry_count).unwrap(); // static texture count
        cursor.write_u32::<LittleEndian>(CAT_OFF).unwrap(); // static texture offset
        cursor.write_u32::<LittleEndian>(entry_count).unwrap(); // streamed texture count
        cursor.write_u32::<LittleEndian>(CAT_OFF).unwrap();
        cursor.write_u32::<LittleEndian>(1).unwrap(); // catalog count
        cursor.write_u32::<LittleEndian>(CAT_OFF).unwrap(); // catalog offset

        // Streamed catalog header: tex_data_size = 0 marks external payloads.
        cursor.set_position(u64::from(base_offset + CAT_OFF));
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(entry_count).unwrap();
        cursor.write_u32::<LittleEndian>(entries_offset).unwrap();

        for (index, (_, sides, width, height, mipmap_count, pixel_format)) in
            entries.iter().enumerate()
        {
            let index = u32::try_from(index).unwrap();
            let rec_off = records_offset + index * RECORD_SIZE;
            cursor.set_position(u64::from(base_offset + entries_offset + index * 12));
            cursor.write_u32::<LittleEndian>(rec_off).unwrap();
            cursor.write_u32::<LittleEndian>(0).unwrap();
            cursor.write_u32::<LittleEndian>(0).unwrap();

            cursor.set_position(u64::from(base_offset + rec_off + 4));
            cursor.write_u32::<LittleEndian>(index + 1).unwrap(); // record id
            cursor.set_position(u64::from(base_offset + rec_off + 12));
            cursor.write_u32::<LittleEndian>(*pixel_format).unwrap();
            cursor.set_position(u64::from(base_offset + rec_off + 16));
            cursor.write_u32::<LittleEndian>(*sides).unwrap();
            cursor.set_position(u64::from(base_offset + rec_off + 20));
            cursor.write_u32::<LittleEndian>(*mipmap_count).unwrap();
            cursor.set_position(u64::from(base_offset + rec_off + 24));
            cursor.write_u32::<LittleEndian>(*width).unwrap();
            cursor.write_u32::<LittleEndian>(*height).unwrap();
        }

        bytes.extend_from_slice(&NAME_TABLE_MARKER.to_le_bytes());
        for (index, (name, _, _, _, _, _)) in entries.iter().enumerate() {
            bytes.extend_from_slice(&(u32::try_from(index).unwrap() + 1).to_le_bytes());
            bytes.extend_from_slice(&u32::try_from(name.len()).unwrap().to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
        }

        bytes
    }

    fn make_temp_dir() -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "texembed_streamtex_test_{}_{}",
            std::process::id(),
            stamp
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// A sidecar can hold more streamed entries than the `.streamtex` file
    /// stores (e.g. `end_sequence` keeps a leading cubemap elsewhere). Names
    /// must still line up by payload shape rather than by entry order.
    #[test]
    fn load_streamtex_skips_extra_leading_sidecar_entry() {
        let dir = make_temp_dir();
        let streamtex_path = dir.join("level.streamtex");
        let textures_path = dir.join("level.textures");

        fs::write(
            &streamtex_path,
            make_test_streamtex(&[
                make_test_dds(256, 256, 9, 0x3154_5844),
                make_test_dds(1024, 1024, 11, 0x3154_5844),
            ]),
        )
        .unwrap();
        fs::write(
            &textures_path,
            make_test_textures(&[
                ("cubemap_Generic_swap", 6, 256, 256, 9, 0x21),
                ("A_Pavement_A", 1, 256, 256, 9, 0x21),
                ("Generic_Signage01", 1, 1024, 1024, 11, 0x21),
            ]),
        )
        .unwrap();

        let file = load_streamtex_file(streamtex_path).unwrap();

        assert_eq!(file.dds_list.len(), 2);
        assert_eq!(file.dds_list[0].name, "A_Pavement_A.dds");
        assert_eq!(file.dds_list[1].name, "Generic_Signage01.dds");

        fs::remove_dir_all(dir).unwrap();
    }

    /// Identical shapes must be consumed one-to-one, in file order.
    #[test]
    fn load_streamtex_consumes_duplicate_shapes_one_to_one() {
        let dir = make_temp_dir();
        let streamtex_path = dir.join("dupes.streamtex");
        let textures_path = dir.join("dupes.textures");

        fs::write(
            &streamtex_path,
            make_test_streamtex(&[
                make_test_dds(64, 64, 1, 0x3154_5844),
                make_test_dds(64, 64, 1, 0x3154_5844),
            ]),
        )
        .unwrap();
        fs::write(
            &textures_path,
            make_test_textures(&[
                ("First", 1, 64, 64, 1, 0x21),
                ("Second", 1, 64, 64, 1, 0x21),
            ]),
        )
        .unwrap();

        let file = load_streamtex_file(streamtex_path).unwrap();

        assert_eq!(file.dds_list.len(), 2);
        assert_eq!(file.dds_list[0].name, "First.dds");
        assert_eq!(file.dds_list[1].name, "Second.dds");

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_streamtex_falls_back_to_index_names_without_sidecar() {
        let dir = make_temp_dir();
        let streamtex_path = dir.join("orphan.streamtex");

        fs::write(
            &streamtex_path,
            make_test_streamtex(&[
                make_test_dds(64, 64, 1, 0x3154_5844),
                vec![0_u8; 32],
                make_test_dds(128, 128, 1, 0x3554_5844),
            ]),
        )
        .unwrap();

        let file = load_streamtex_file(streamtex_path).unwrap();

        assert_eq!(file.dds_list.len(), 2);
        assert_eq!(file.dds_list[0].entry_index, 0);
        assert_eq!(file.dds_list[0].name, "0.dds");
        assert_eq!(file.dds_list[1].entry_index, 2);
        assert_eq!(file.dds_list[1].name, "2.dds");

        fs::remove_dir_all(dir).unwrap();
    }
}
