use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use super::textures::{
    dds_record_pixel_format, load_streamed_texture_entries, StreamedTextureEntry,
};
use crate::dds::parse_dds_header;
use crate::model::{DdsInfo, FileType, TextureFile};

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
    for _ in 0..mipmap_count {
        let blocks_w = usize::try_from(current_width.div_ceil(edge)).ok()?;
        let blocks_h = usize::try_from(current_height.div_ceil(edge)).ok()?;
        chain = chain.checked_add(blocks_w.checked_mul(blocks_h)?.checked_mul(block_bytes)?)?;
        current_width = (current_width / 2).max(1);
        current_height = (current_height / 2).max(1);
    }

    DDS_HEADER_LEN.checked_add(chain.checked_mul(sides)?.checked_mul(depth)?)
}

/// Whether `at` is a position where a new record can begin: the start of the
/// next `[len]["DDS "]` pair, or the end of the file.
fn record_boundary(bytes: &[u8], at: usize) -> bool {
    at == bytes.len() || (at + 8 <= bytes.len() && &bytes[at + 4..at + 8] == b"DDS ")
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

/// Splits a `.streamtex` blob into its payloads.
///
/// A record is `[u32 little-endian length][DDS payload]`, but a handful of
/// retail records store a length that is a few bytes off. Advancing by such a
/// value desynchronises every record that follows, so each boundary is
/// validated against the DDS magic of the record that should start there: the
/// candidate size that actually lands on a record boundary wins, and because
/// the DDS header fully describes its payload, that recomputed size is
/// preferred whenever both are plausible.
fn read_streamtex_records(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut records = Vec::new();
    let mut position = 0usize;

    while position + 4 <= bytes.len() {
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
        let declared_len =
            (declared <= available && record_boundary(bytes, payload_start + declared))
                .then_some(declared);

        let payload_len = match (computed_len, declared_len) {
            (Some(len), _) => len,
            (None, Some(len)) => len,
            // Neither candidate lands on a boundary: fall back to the most
            // trustworthy size known, or stop when nothing is usable.
            (None, None) => match computed.or((declared <= available).then_some(declared)) {
                Some(len) => len,
                None => break,
            },
        };

        records.push(bytes[payload_start..payload_start + payload_len].to_vec());
        position = payload_start + payload_len;
    }

    records
}

/// Loads a `.streamtex` file and extracts embedded DDS payloads.
///
/// # Errors
///
/// Returns an error when the file cannot be read.
pub fn load_streamtex_file(path: PathBuf) -> Result<TextureFile> {
    let original_bytes = fs::read(&path)?;
    let textures = read_streamtex_records(&original_bytes);
    let sidecar_entries = load_streamtex_sidecar_entries(&path);

    let headers: Vec<Option<(u32, u32, u32, u32)>> = textures
        .iter()
        .map(|bytes| parse_dds_header(bytes).ok())
        .collect();
    let names = sidecar_entries.as_deref().map(|entries| {
        let records: Vec<RecordKey> = textures
            .iter()
            .zip(&headers)
            .filter_map(|(bytes, header)| header.map(|header| dds_record_key(bytes, header)))
            .collect();
        map_sidecar_names(&records, entries)
    });

    let mut dds_list = Vec::new();
    let mut name_index = 0;

    for (i, bytes) in textures.into_iter().enumerate() {
        if let Some((width, height, mipmap_count, pixel_format)) = headers[i] {
            let name = names
                .as_ref()
                .and_then(|names| names.get(name_index))
                .and_then(|name| name.as_ref())
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("{i}.dds"));
            name_index += 1;
            dds_list.push(DdsInfo {
                name,
                bytes: bytes.into(),
                width,
                height,
                mipmap_count,
                pixel_format,
                catalog_index: 0,
                entry_index: i,
            });
        }
    }

    Ok(TextureFile {
        file_type: FileType::Streamtex,
        path,
        dds_list,
        original_bytes,
    })
}

/// Saves a `.streamtex` file from the in-memory DDS list.
///
/// # Errors
///
/// Returns an error when a DDS payload length does not fit in `u32` or the
/// output file cannot be written.
pub fn save_streamtex_file(texture_file: &TextureFile, path: &PathBuf) -> Result<()> {
    let mut result_bytes = Vec::new();
    for dds_info in &texture_file.dds_list {
        let length = u32::try_from(dds_info.bytes.len()).map_err(|_| {
            anyhow!(
                "[Save Streamtex] DDS payload is too large to store: {} bytes",
                dds_info.bytes.len()
            )
        })?;
        result_bytes.extend_from_slice(&length.to_le_bytes());
        result_bytes.extend_from_slice(dds_info.bytes.as_ref());
    }
    fs::write(path, &result_bytes).map_err(|e| {
        anyhow!(
            "[Save Streamtex] Failed to write file to {}: {}",
            path.display(),
            e
        )
    })?;
    Ok(())
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
