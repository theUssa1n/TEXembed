use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::dds::parse_dds_header;
use crate::model::{DdsInfo, FileType, TextureFile};

const TEXTURES_MAGIC: u32 = 0x5445_5853;
const NAME_TABLE_MARKER: u32 = 0x4332_4E4D;
const DDS_MAGIC: u32 = 0x2053_4444;
const BASE_OFFSET: usize = 0x10;

/// Bounded forward scan (in bytes) used to locate a name-table marker that sits
/// a few bytes past its nominal position. Some externally-modded files store a
/// stale catalog size, shifting the marker by a small amount.
const NAME_TABLE_SCAN_LIMIT: u64 = 0x100;

#[derive(Clone, Copy)]
struct TexturesHeader {
    table_size: usize,
    cat_count: u32,
    cat_off: u32,
}

struct CatalogData {
    size: usize,
    unk: u32,
    entry_count: u32,
    entries_offset: u32,
    entry_info: Vec<(usize, usize)>,
}

fn read_u32_as_usize(value: u32, label: &str) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_| anyhow!("[Textures file] {label} does not fit in usize: {value}"))
}

fn usize_to_u32(value: usize, label: &str) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| anyhow!("[Textures file] {label} does not fit in u32: {value}"))
}

fn checked_range(
    start: usize,
    length: usize,
    limit: usize,
    label: &str,
) -> Result<std::ops::Range<usize>> {
    let end = start.checked_add(length).filter(|end| *end <= limit)
        .ok_or_else(|| anyhow!("[Textures file] {label} at {start:#X} with length {length} is out of range ({limit} bytes)"))?;
    Ok(start..end)
}

fn checked_table_range(
    offset: u32,
    count: u32,
    stride: usize,
    header_end: usize,
    label: &str,
) -> Result<usize> {
    let offset = read_u32_as_usize(offset, label)?;
    let length = read_u32_as_usize(count, label)?
        .checked_mul(stride)
        .ok_or_else(|| anyhow!("[Textures file] {label} size overflow"))?;
    let start = BASE_OFFSET
        .checked_add(offset)
        .ok_or_else(|| anyhow!("[Textures file] {label} offset overflow"))?;
    if count != 0 && offset < 24 {
        return Err(anyhow!("[Textures file] {label} overlaps the fixed header"));
    }
    if count != 0 {
        checked_range(start, length, header_end, label)?;
    }
    Ok(start)
}

/// Returns the absolute end offset of the header/table region, rejecting a
/// `table_size` that would reach past the end of the file. Guards the header
/// slice so a corrupt or externally-modded size cannot panic on save.
fn checked_header_end(bytes_len: usize, table_size: usize, context: &str) -> Result<usize> {
    let end = BASE_OFFSET
        .checked_add(table_size)
        .ok_or_else(|| anyhow!("[{context}] Header table size overflow"))?;
    if end > bytes_len {
        return Err(anyhow!(
            "[{context}] Header table size {table_size} reaches past the end of the file ({bytes_len} bytes)"
        ));
    }
    Ok(end)
}

fn read_textures_header(cursor: &mut Cursor<&[u8]>, context: &str) -> Result<TexturesHeader> {
    let magic = cursor.read_u32::<LittleEndian>()?;
    if magic != TEXTURES_MAGIC {
        return Err(anyhow!(
            "[{context}] Invalid magic number at offset 0x0: expected 0x{TEXTURES_MAGIC:08X} (\"SXET\"), got 0x{magic:08X}"
        ));
    }

    cursor.set_position(0xC);
    let table_size = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "table size")?;
    let header_end = checked_header_end(cursor.get_ref().len(), table_size, context)?;
    if table_size < 24 {
        return Err(anyhow!(
            "[{context}] Header table is smaller than the fixed header"
        ));
    }

    cursor.set_position(BASE_OFFSET as u64);
    let _st_count = cursor.read_u32::<LittleEndian>()?;
    let _st_off = cursor.read_u32::<LittleEndian>()?;
    let _pt_count = cursor.read_u32::<LittleEndian>()?;
    let _pt_off = cursor.read_u32::<LittleEndian>()?;
    let cat_count = cursor.read_u32::<LittleEndian>()?;
    let cat_off = cursor.read_u32::<LittleEndian>()?;
    let cat_start = checked_table_range(cat_off, cat_count, 16, header_end, "catalog table")?;
    let mut data_end = header_end;
    for index in 0..read_u32_as_usize(cat_count, "catalog count")? {
        cursor.set_position((cat_start + index * 16) as u64);
        let size = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "catalog size")?;
        let _unknown = cursor.read_u32::<LittleEndian>()?;
        let count = cursor.read_u32::<LittleEndian>()?;
        let entries_offset = cursor.read_u32::<LittleEndian>()?;
        let entries_start =
            checked_table_range(entries_offset, count, 12, header_end, "entry table")?;
        data_end = checked_range(data_end, size, cursor.get_ref().len(), "catalog data")?.end;
        for entry in 0..read_u32_as_usize(count, "entry count")? {
            cursor.set_position((entries_start + entry * 12) as u64);
            let record_offset = cursor.read_u32::<LittleEndian>()?;
            checked_table_range(record_offset, 1, 32, header_end, "texture record")?;
        }
    }

    Ok(TexturesHeader {
        table_size,
        cat_count,
        cat_off,
    })
}

fn sum_catalog_texture_sizes(cursor: &mut Cursor<&[u8]>, header: TexturesHeader) -> Result<u64> {
    let cat_off = read_u32_as_usize(header.cat_off, "catalog offset")?;
    let mut sum_tex_data = 0_u64;

    for i in 0..read_u32_as_usize(header.cat_count, "catalog count")? {
        let cat_ptr = BASE_OFFSET + cat_off + i * 16;
        cursor.set_position(cat_ptr as u64);
        let tex_data_size = cursor.read_u32::<LittleEndian>()?;
        sum_tex_data += u64::from(tex_data_size);
    }

    Ok(sum_tex_data)
}

/// Locates the `MN2C` marker that precedes the optional name table.
///
/// The nominal position is derived from the catalog `texture_data_size` fields.
/// Files whose data was replaced externally can carry a slightly stale size,
/// shifting the marker by a few bytes, so a bounded forward scan is used as a
/// fallback before giving up.
fn find_name_table_offset(
    cursor: &mut Cursor<&[u8]>,
    original_bytes: &[u8],
    header: TexturesHeader,
) -> Result<Option<u64>> {
    let bytes_len = u64::try_from(original_bytes.len())?;
    let nominal = u64::try_from(BASE_OFFSET + header.table_size)?
        .checked_add(sum_catalog_texture_sizes(cursor, header)?)
        .ok_or_else(|| anyhow!("[Textures file] name table offset overflow"))?;
    let marker = NAME_TABLE_MARKER.to_le_bytes();

    let has_marker =
        |off: u64| off + 4 <= bytes_len && original_bytes[off as usize..off as usize + 4] == marker;

    if has_marker(nominal) {
        return Ok(Some(nominal));
    }

    let scan_end = nominal
        .saturating_add(NAME_TABLE_SCAN_LIMIT)
        .min(bytes_len.saturating_sub(4));
    for off in (nominal + 1)..=scan_end {
        if has_marker(off) {
            return Ok(Some(off));
        }
    }

    Ok(None)
}

fn read_name_table(
    cursor: &mut Cursor<&[u8]>,
    original_bytes: &[u8],
    header: TexturesHeader,
) -> Result<HashMap<u32, String>> {
    let bytes_len = u64::try_from(original_bytes.len())?;
    let mut names_hashmap = HashMap::new();

    let Some(name_table_offset) = find_name_table_offset(cursor, original_bytes, header)? else {
        return Ok(names_hashmap);
    };
    if name_table_offset + 4 > bytes_len {
        return Ok(names_hashmap);
    }

    let mut pos = name_table_offset + 4;
    while pos + 8 <= bytes_len {
        cursor.set_position(pos);
        let id = cursor.read_u32::<LittleEndian>()?;
        let length = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "name length")?;
        pos += 8;

        if pos + u64::try_from(length)? > bytes_len {
            break;
        }

        let mut name_bytes = vec![0_u8; length];
        cursor.read_exact(&mut name_bytes)?;
        if let Ok(name_str) = String::from_utf8(name_bytes) {
            names_hashmap.insert(id, name_str);
        }
        pos += u64::try_from(length)?;
    }

    Ok(names_hashmap)
}

fn collect_dds_entries(
    cursor: &mut Cursor<&[u8]>,
    original_bytes: &[u8],
    header: TexturesHeader,
    names_hashmap: &HashMap<u32, String>,
) -> Result<Vec<DdsInfo>> {
    let cat_off = read_u32_as_usize(header.cat_off, "catalog offset")?;
    let cat_count = read_u32_as_usize(header.cat_count, "catalog count")?;
    let bytes_len = u64::try_from(original_bytes.len())?;
    let mut dds_list = Vec::new();
    let mut current_embedded_offset = u64::try_from(BASE_OFFSET + header.table_size)?;

    for i in 0..cat_count {
        let cat_ptr = BASE_OFFSET + cat_off + i * 16;
        cursor.set_position(cat_ptr as u64);
        let tex_data_size = cursor.read_u32::<LittleEndian>()?;
        let _unk = cursor.read_u32::<LittleEndian>()?;
        let entry_count = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "entry count")?;
        let entries_offset =
            read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "entries offset")?;

        if tex_data_size == 0 {
            continue;
        }
        let catalog_end = current_embedded_offset + u64::from(tex_data_size);
        for e in 0..entry_count {
            if let Some(dds) = read_embedded_dds(
                cursor,
                bytes_len.min(catalog_end),
                current_embedded_offset,
                entries_offset,
                i,
                e,
                names_hashmap,
            )? {
                dds_list.push(dds);
            }
        }

        current_embedded_offset += u64::from(tex_data_size);
    }

    Ok(dds_list)
}

fn read_embedded_dds(
    cursor: &mut Cursor<&[u8]>,
    bytes_len: u64,
    current_embedded_offset: u64,
    entries_offset: usize,
    catalog_index: usize,
    entry_index: usize,
    names_hashmap: &HashMap<u32, String>,
) -> Result<Option<DdsInfo>> {
    let entry_ptr = BASE_OFFSET + entries_offset + entry_index * 12;
    cursor.set_position(entry_ptr as u64);
    let rec_off = cursor.read_u32::<LittleEndian>()?;
    let _unk_off = cursor.read_u32::<LittleEndian>()?;
    let data_off = cursor.read_u32::<LittleEndian>()?;

    let abs_data_off = current_embedded_offset + u64::from(data_off);
    if abs_data_off + 4 > bytes_len {
        return Err(anyhow!("[Textures file] Catalog {catalog_index}, entry {entry_index}: data offset is outside its catalog"));
    }

    cursor.set_position(abs_data_off);
    let file_len = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "embedded DDS length")?;
    let magic_ptr = abs_data_off + 4;
    if magic_ptr + u64::try_from(file_len)? > bytes_len {
        return Err(anyhow!("[Textures file] Catalog {catalog_index}, entry {entry_index}: payload extends past its catalog"));
    }
    if file_len < 4 {
        return Ok(None);
    }

    cursor.set_position(magic_ptr);
    let dds_magic = cursor.read_u32::<LittleEndian>()?;
    if dds_magic != DDS_MAGIC || magic_ptr + u64::try_from(file_len)? > bytes_len {
        return Ok(None);
    }

    let mut dds_bytes = vec![0_u8; file_len];
    cursor.set_position(magic_ptr);
    cursor.read_exact(&mut dds_bytes)?;

    let abs_rec_off = BASE_OFFSET + read_u32_as_usize(rec_off, "record offset")?;
    cursor.set_position(abs_rec_off as u64 + 4);
    let id = cursor.read_u32::<LittleEndian>()?;
    let name = match names_hashmap.get(&id) {
        Some(name) => format!("{name}.dds"),
        None => format!("{id:08X}.dds"),
    };

    let Ok((width, height, mipmap_count, pixel_format)) = parse_dds_header(&dds_bytes) else {
        return Ok(None);
    };

    Ok(Some(DdsInfo {
        name,
        bytes: dds_bytes.into(),
        width,
        height,
        mipmap_count,
        pixel_format,
        catalog_index,
        entry_index,
        modified: false,
    }))
}

fn build_dds_map(texture_file: &TextureFile) -> Result<HashMap<(usize, usize), &DdsInfo>> {
    let mut dds_map = HashMap::new();
    for dds in &texture_file.dds_list {
        if dds_map
            .insert((dds.catalog_index, dds.entry_index), dds)
            .is_some()
        {
            return Err(anyhow!(
                "[Save Textures] Duplicate DDS entry ({}, {})",
                dds.catalog_index,
                dds.entry_index
            ));
        }
    }
    Ok(dds_map)
}

fn build_catalog_data(
    texture_file: &TextureFile,
    cursor: &mut Cursor<&[u8]>,
    original_bytes: &[u8],
    header: TexturesHeader,
) -> Result<(Vec<u8>, Vec<CatalogData>)> {
    let dds_map = build_dds_map(texture_file)?;
    let cat_off = read_u32_as_usize(header.cat_off, "catalog offset")?;
    let cat_count = read_u32_as_usize(header.cat_count, "catalog count")?;
    let mut embedded_files_bytes = Vec::new();
    let mut catalog_info = Vec::new();
    let mut old_catalog_start = BASE_OFFSET + header.table_size;
    for dds in dds_map.values() {
        if dds.catalog_index >= cat_count {
            return Err(anyhow!(
                "[Save Textures] Catalog index {} is out of range",
                dds.catalog_index
            ));
        }
        cursor.set_position((BASE_OFFSET + cat_off + dds.catalog_index * 16) as u64);
        let size = cursor.read_u32::<LittleEndian>()?;
        let _unknown = cursor.read_u32::<LittleEndian>()?;
        let count = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "entry count")?;
        if size == 0 || dds.entry_index >= count {
            return Err(anyhow!(
                "[Save Textures] DDS entry ({}, {}) does not refer to embedded data",
                dds.catalog_index,
                dds.entry_index
            ));
        }
    }

    for i in 0..cat_count {
        let cat_ptr = BASE_OFFSET + cat_off + i * 16;
        cursor.set_position(cat_ptr as u64);
        let tex_data_size = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "catalog size")?;
        let unk = cursor.read_u32::<LittleEndian>()?;
        let entry_count = cursor.read_u32::<LittleEndian>()?;
        let entries_offset = cursor.read_u32::<LittleEndian>()?;
        let entry_count_usize = read_u32_as_usize(entry_count, "entry count")?;
        let catalog_start = embedded_files_bytes.len();
        let mut entry_info = Vec::new();

        if tex_data_size != 0 {
            let range = checked_range(
                old_catalog_start,
                tex_data_size,
                original_bytes.len(),
                "catalog data",
            )?;
            let catalog_bytes = &original_bytes[range.clone()];
            let mut groups = BTreeMap::<usize, Vec<usize>>::new();
            for e in 0..entry_count_usize {
                let entry_ptr =
                    BASE_OFFSET + read_u32_as_usize(entries_offset, "entries offset")? + e * 12;
                cursor.set_position((entry_ptr + 8) as u64);
                let offset = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "data offset")?;
                groups.entry(offset).or_default().push(e);
            }
            let mut consumed = 0;
            let mut separate_replacements = Vec::new();
            for (offset, entries) in groups {
                let original = copy_original_entry(catalog_bytes, offset)?;
                if offset < consumed {
                    return Err(anyhow!("[Save Textures] Overlapping payloads in catalog {i} cannot be saved safely"));
                }
                embedded_files_bytes.extend_from_slice(&catalog_bytes[consumed..offset]);
                let new_offset = embedded_files_bytes.len() - catalog_start;
                let replacement = entries
                    .first()
                    .and_then(|e| dds_map.get(&(i, *e)).copied())
                    .filter(|dds| dds.modified);
                let shared_replacement = replacement.filter(|first| {
                    entries.iter().all(|e| {
                        dds_map
                            .get(&(i, *e))
                            .is_some_and(|dds| dds.modified && dds.bytes == first.bytes)
                    })
                });
                if let Some(dds) = shared_replacement {
                    append_dds_record(&mut embedded_files_bytes, dds)?;
                    entry_info.extend(entries.iter().map(|e| (*e, new_offset)));
                } else {
                    embedded_files_bytes.extend_from_slice(original);
                    for e in entries {
                        if let Some(dds) = dds_map
                            .get(&(i, e))
                            .filter(|dds| dds.modified && dds.bytes.as_ref() != &original[4..])
                        {
                            separate_replacements.push((e, *dds));
                        } else {
                            entry_info.push((e, new_offset));
                        }
                    }
                }
                consumed = offset + original.len();
            }
            embedded_files_bytes.extend_from_slice(&catalog_bytes[consumed..]);
            for (entry, dds) in separate_replacements {
                entry_info.push((entry, embedded_files_bytes.len() - catalog_start));
                append_dds_record(&mut embedded_files_bytes, dds)?;
            }
            old_catalog_start = range.end;
        }

        let catalog_size = embedded_files_bytes.len() - catalog_start;
        catalog_info.push(CatalogData {
            size: catalog_size,
            unk,
            entry_count,
            entries_offset,
            entry_info,
        });
    }

    Ok((embedded_files_bytes, catalog_info))
}

fn append_dds_record(bytes: &mut Vec<u8>, dds: &DdsInfo) -> Result<()> {
    parse_dds_header(&dds.bytes)?;
    let length = usize_to_u32(dds.bytes.len(), "DDS length")?;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&dds.bytes);
    Ok(())
}

fn copy_original_entry(catalog_bytes: &[u8], offset: usize) -> Result<&[u8]> {
    let prefix = checked_range(offset, 4, catalog_bytes.len(), "entry length prefix")?;
    let mut cursor = Cursor::new(&catalog_bytes[prefix.clone()]);
    let length = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "entry length")?;
    let payload = checked_range(prefix.end, length, catalog_bytes.len(), "entry payload")?;
    Ok(&catalog_bytes[offset..payload.end])
}

fn update_catalog_headers(
    header_mut_cursor: &mut Cursor<&mut [u8]>,
    header: TexturesHeader,
    catalog_info: &[CatalogData],
) -> Result<()> {
    let cat_off = read_u32_as_usize(header.cat_off, "catalog offset")?;

    for (i, catalog) in catalog_info.iter().enumerate() {
        if catalog.size == 0 {
            continue;
        }
        let cat_ptr = BASE_OFFSET + cat_off + i * 16;
        header_mut_cursor.set_position(cat_ptr as u64);
        header_mut_cursor.write_u32::<LittleEndian>(usize_to_u32(catalog.size, "catalog size")?)?;
        header_mut_cursor.write_u32::<LittleEndian>(catalog.unk)?;
        header_mut_cursor.write_u32::<LittleEndian>(catalog.entry_count)?;
        header_mut_cursor.write_u32::<LittleEndian>(catalog.entries_offset)?;

        let entries_offset = read_u32_as_usize(catalog.entries_offset, "entries offset")?;
        for (entry_index, offset_in_catalog) in &catalog.entry_info {
            let entry_ptr = BASE_OFFSET + entries_offset + entry_index * 12;
            header_mut_cursor.set_position((entry_ptr + 8) as u64);
            header_mut_cursor
                .write_u32::<LittleEndian>(usize_to_u32(*offset_in_catalog, "entry offset")?)?;
        }
    }

    Ok(())
}

fn update_record_dimensions(
    cursor: &mut Cursor<&[u8]>,
    header_mut_cursor: &mut Cursor<&mut [u8]>,
    texture_file: &TextureFile,
    header: TexturesHeader,
) -> Result<()> {
    let cat_off = read_u32_as_usize(header.cat_off, "catalog offset")?;

    for dds in texture_file.dds_list.iter().filter(|dds| dds.modified) {
        let cat_ptr = BASE_OFFSET + cat_off + dds.catalog_index * 16;
        cursor.set_position(cat_ptr as u64);
        let _tex_data_size = cursor.read_u32::<LittleEndian>()?;
        let _unk = cursor.read_u32::<LittleEndian>()?;
        let _entry_count = cursor.read_u32::<LittleEndian>()?;
        let entries_offset =
            read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "entries offset")?;
        let entry_ptr = BASE_OFFSET + entries_offset + dds.entry_index * 12;
        cursor.set_position(entry_ptr as u64);
        let rec_off = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "record offset")?;

        let (width, height, mipmap_count, _) = parse_dds_header(dds.bytes.as_ref())?;
        let rec_ptr = BASE_OFFSET + rec_off;
        // Keep the record's pixel-format code in sync with the (possibly
        // replaced) DDS header, so a format change is reflected on save.
        let pf = dds_record_pixel_format(dds.bytes.as_ref());
        if pf != 0 {
            header_mut_cursor.set_position(rec_ptr as u64 + 12);
            header_mut_cursor.write_u32::<LittleEndian>(pf)?;
        }
        header_mut_cursor.set_position(rec_ptr as u64 + 20);
        header_mut_cursor.write_u32::<LittleEndian>(mipmap_count)?;
        header_mut_cursor.write_u32::<LittleEndian>(width)?;
        header_mut_cursor.write_u32::<LittleEndian>(height)?;
    }

    Ok(())
}

fn sum_old_texture_data(cursor: &mut Cursor<&[u8]>, header: TexturesHeader) -> Result<usize> {
    let cat_off = read_u32_as_usize(header.cat_off, "catalog offset")?;
    let mut sum = 0_usize;

    for i in 0..read_u32_as_usize(header.cat_count, "catalog count")? {
        let cat_ptr = BASE_OFFSET + cat_off + i * 16;
        cursor.set_position(cat_ptr as u64);
        let j_size = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "catalog size")?;
        sum += j_size;
    }

    Ok(sum)
}

/// A texture entry whose pixel data lives in an external `.streamtex` file.
///
/// Split/Second marks such entries with a catalog `texture_data_size` of `0`;
/// the payload is not stored inline but in the sibling `<name>.streamtex`
/// file. The static record fields describe the payload shape and are what
/// allows the entry to be paired with a `.streamtex` record, since entries of
/// both kinds are interleaved in the file and a catalog index is not stable
/// across assets.
#[derive(Clone, Debug)]
pub struct StreamedTextureEntry {
    pub name: String,
    pub catalog_index: usize,
    pub entry_index: usize,
    pub record_id: u32,
    pub data_offset: u32,
    pub width: u32,
    pub height: u32,
    pub sides: u32,
    pub mipmap_count: u32,
    /// Game pixel-format code (`0x21` DXT1, `0x24` BC3, `0x02` A8R8G8B8, ...).
    pub pixel_format: u32,
}

/// Collects every streamed texture entry of a `.textures` file, in file order.
///
/// Only catalogs without inline data (`texture_data_size == 0`) are considered,
/// so the returned list is not polluted by the embedded textures that are
/// stored next to them (which is what made a flat name list unreliable).
///
/// # Errors
///
/// Returns an error when the file cannot be read or its binary layout cannot be
/// parsed.
pub fn load_streamed_texture_entries(path: &PathBuf) -> Result<Vec<StreamedTextureEntry>> {
    let original_bytes = fs::read(path)?;
    read_streamed_texture_entries(&original_bytes)
}

pub(super) fn read_streamed_texture_entries(
    original_bytes: &[u8],
) -> Result<Vec<StreamedTextureEntry>> {
    let mut cursor = Cursor::new(original_bytes);
    let header = read_textures_header(&mut cursor, "Streamed texture entries")?;
    let names_hashmap = read_name_table(&mut cursor, original_bytes, header)?;
    let cat_off = read_u32_as_usize(header.cat_off, "catalog offset")?;
    let cat_count = read_u32_as_usize(header.cat_count, "catalog count")?;
    let mut entries = Vec::new();

    for i in 0..cat_count {
        let cat_ptr = BASE_OFFSET + cat_off + i * 16;
        cursor.set_position(cat_ptr as u64);
        let tex_data_size = cursor.read_u32::<LittleEndian>()?;
        let _unk = cursor.read_u32::<LittleEndian>()?;
        let entry_count = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "entry count")?;
        let entries_offset =
            read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "entries offset")?;

        if tex_data_size != 0 {
            continue;
        }

        for e in 0..entry_count {
            let entry_ptr = BASE_OFFSET + entries_offset + e * 12;
            cursor.set_position(entry_ptr as u64);
            let rec_off = read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "record offset")?;
            let _unk_off = cursor.read_u32::<LittleEndian>()?;
            let data_offset = cursor.read_u32::<LittleEndian>()?;
            let rec_ptr = BASE_OFFSET + rec_off;
            cursor.set_position((rec_ptr + 4) as u64);
            let id = cursor.read_u32::<LittleEndian>()?;
            cursor.set_position((rec_ptr + 12) as u64);
            let pixel_format = cursor.read_u32::<LittleEndian>()?;
            cursor.set_position((rec_ptr + 16) as u64);
            let sides = cursor.read_u32::<LittleEndian>()?;
            cursor.set_position((rec_ptr + 20) as u64);
            let mipmap_count = cursor.read_u32::<LittleEndian>()?;
            cursor.set_position((rec_ptr + 24) as u64);
            let width = cursor.read_u32::<LittleEndian>()?;
            let height = cursor.read_u32::<LittleEndian>()?;
            let name = match names_hashmap.get(&id) {
                Some(name) => name.clone(),
                None => format!("{id:08X}"),
            };
            entries.push(StreamedTextureEntry {
                name: format!("{name}.dds"),
                catalog_index: i,
                entry_index: e,
                record_id: id,
                data_offset,
                width,
                height,
                sides,
                mipmap_count,
                pixel_format,
            });
        }
    }

    Ok(entries)
}

/// Loads a `.textures` file and extracts embedded DDS payloads.
///
/// # Errors
///
/// Returns an error when the file cannot be read or its binary layout cannot be
/// parsed.
pub fn load_textures_file(path: PathBuf) -> Result<TextureFile> {
    let original_bytes = fs::read(&path)?;
    let mut cursor = Cursor::new(original_bytes.as_slice());
    let header = read_textures_header(&mut cursor, "Textures file")?;
    let names_hashmap = read_name_table(&mut cursor, &original_bytes, header)?;
    let dds_list = collect_dds_entries(&mut cursor, &original_bytes, header, &names_hashmap)?;

    Ok(TextureFile {
        file_type: FileType::Textures,
        path,
        dds_list,
        original_bytes,
        streamtex_layout: None,
    })
}

/// Saves a `.textures` file while preserving the original table structure.
///
/// # Errors
///
/// Returns an error when the source file metadata is invalid, a DDS payload
/// does not fit into the binary format, or the destination file cannot be
/// written.
pub fn save_textures_file(texture_file: &TextureFile, path: &Path) -> Result<()> {
    let result_bytes = serialize_textures(texture_file)?;
    super::storage::atomic_write(path, &result_bytes)
}

pub(super) fn serialize_textures(texture_file: &TextureFile) -> Result<Vec<u8>> {
    if texture_file.file_type != FileType::Textures {
        return Err(anyhow!("[Save Textures] Expected a .textures document"));
    }
    let original_bytes = &texture_file.original_bytes;
    let mut cursor = Cursor::new(original_bytes.as_slice());
    let header = read_textures_header(&mut cursor, "Save Textures")?;
    let header_end = checked_header_end(original_bytes.len(), header.table_size, "Save Textures")?;
    let mut header_bytes = original_bytes[..header_end].to_vec();
    let mut header_mut_cursor = Cursor::new(header_bytes.as_mut_slice());
    let (embedded_files_bytes, catalog_info) =
        build_catalog_data(texture_file, &mut cursor, original_bytes, header)?;
    update_catalog_headers(&mut header_mut_cursor, header, &catalog_info)?;
    update_record_dimensions(&mut cursor, &mut header_mut_cursor, texture_file, header)?;

    let mut result_bytes = Vec::new();
    result_bytes.extend_from_slice(&header_bytes);
    result_bytes.extend_from_slice(&embedded_files_bytes);
    let suffix_offset = checked_range(
        header_end,
        sum_old_texture_data(&mut cursor, header)?,
        original_bytes.len(),
        "embedded data",
    )?
    .end;
    result_bytes.extend_from_slice(&original_bytes[suffix_offset..]);
    Ok(result_bytes)
}

/// Maps a DDS header to the game's pixel-format code stored in the static
/// `.textures` record (`pf` field @ record+12). Returns `0` for formats that
/// cannot be mapped, in which case the `pf` field is left untouched.
///
/// Known codes: `0x21` = DXT1, `0x24` = DXT4/DXT5 (BC3), `0x02` =
/// A8R8G8B8 (uncompressed 32-bit). The remaining codes follow the same
/// `0x20 + family` pattern.
pub(crate) fn dds_record_pixel_format(dds_bytes: &[u8]) -> u32 {
    if dds_bytes.len() < 128 {
        return 0;
    }
    let fourcc = u32::from_le_bytes([dds_bytes[84], dds_bytes[85], dds_bytes[86], dds_bytes[87]]);
    match &fourcc.to_le_bytes() {
        b"DXT1" => 0x21,
        b"DXT2" => 0x22,
        b"DXT3" => 0x23,
        b"DXT4" => 0x24,
        b"DXT5" => 0x24,
        _ => {
            let flags =
                u32::from_le_bytes([dds_bytes[80], dds_bytes[81], dds_bytes[82], dds_bytes[83]]);
            // `ddspf.dwRGBBitCount` sits at offset 88, right after the fourCC.
            let bitdepth =
                u32::from_le_bytes([dds_bytes[88], dds_bytes[89], dds_bytes[90], dds_bytes[91]]);
            let r_mask =
                u32::from_le_bytes([dds_bytes[92], dds_bytes[93], dds_bytes[94], dds_bytes[95]]);
            if (flags & 0x40) != 0 && bitdepth == 32 && r_mask == 0x00FF_0000 {
                0x02
            } else {
                0
            }
        }
    }
}

/// Rewrites the streamed catalog of a `.textures` file to match a rebuilt
/// `.streamtex` sibling.
///
/// Split/Second stores the streamed textures (the ones whose data lives in the
/// external `<name>.streamtex` file) in a catalog whose `tex_data_size` is
/// `0`. Each of its 12-byte entries keeps an absolute `data_off` pointing at
/// the length-prefixed record inside the `.streamtex` file. Whenever a
/// `.streamtex` record changes size (for example a DXT1 body paint replaced
/// with A8R8G8B8), every later record shifts and these offsets must be
/// recomputed, otherwise the game reads garbage at the stale offsets.
///
/// This function recomputes each `data_off` from the rebuilt record layout and
/// syncs the static record fields (`pf`/mips/width/height) from the new DDS
/// headers. The embedded catalog data and the name table are preserved.
///
/// # Errors
///
/// Returns an error when the `.textures` file cannot be read, no streamed
/// catalog matches the rebuilt record count, or the output cannot be written.
pub fn patch_streamtex_sidecar(textures_path: &PathBuf, streamtex_dds: &[DdsInfo]) -> Result<()> {
    if streamtex_dds
        .iter()
        .enumerate()
        .any(|(index, dds)| dds.entry_index != index)
    {
        return Err(anyhow!("[Patch Streamtex Sidecar] Opaque or reordered records require the original stream layout"));
    }
    let original_bytes = fs::read(textures_path)?;
    let mut cursor = Cursor::new(original_bytes.as_slice());
    let header = read_textures_header(&mut cursor, "Patch Streamtex Sidecar")?;
    let cat_off = read_u32_as_usize(header.cat_off, "catalog offset")?;
    let cat_count = read_u32_as_usize(header.cat_count, "catalog count")?;

    // Locate the streamed catalog: no embedded data and a matching entry count.
    let mut streamed_catalog: Option<(usize, u32)> = None;
    for i in 0..cat_count {
        let cat_ptr = BASE_OFFSET + cat_off + i * 16;
        cursor.set_position(cat_ptr as u64);
        let tex_data_size = cursor.read_u32::<LittleEndian>()?;
        let _unk = cursor.read_u32::<LittleEndian>()?;
        let entry_count = cursor.read_u32::<LittleEndian>()?;
        let entries_offset = cursor.read_u32::<LittleEndian>()?;
        if tex_data_size == 0
            && entry_count == u32::try_from(streamtex_dds.len()).unwrap_or(u32::MAX)
        {
            if streamed_catalog.is_some() {
                return Err(anyhow!("[Patch Streamtex Sidecar] Multiple catalogs match; the original stream binding is required"));
            }
            streamed_catalog = Some((
                read_u32_as_usize(entries_offset, "entries offset")?,
                entry_count,
            ));
        }
    }

    let Some((entries_offset, entry_count)) = streamed_catalog else {
        return Err(anyhow!(
            "[Patch Streamtex Sidecar] No streamed catalog (tex_data_size=0) with {} entries found in {}",
            streamtex_dds.len(),
            textures_path.display()
        ));
    };

    let header_end = checked_header_end(
        original_bytes.len(),
        header.table_size,
        "Patch Streamtex Sidecar",
    )?;
    let mut header_bytes = original_bytes[..header_end].to_vec();
    let mut header_mut_cursor = Cursor::new(header_bytes.as_mut_slice());

    let mut record_offset = 0_u32;
    for e in 0..read_u32_as_usize(entry_count, "entry count")? {
        let entry_ptr = BASE_OFFSET + entries_offset + e * 12;

        // data_off = absolute offset of the record's length-prefix in the new streamtex.
        header_mut_cursor.set_position((entry_ptr + 8) as u64);
        header_mut_cursor.write_u32::<LittleEndian>(record_offset)?;

        // Sync the static record fields from the new DDS header (if parseable).
        if let Some(dds) = streamtex_dds.get(e).filter(|dds| dds.modified) {
            if let Ok((width, height, mipmap_count, _)) = parse_dds_header(dds.bytes.as_ref()) {
                cursor.set_position(entry_ptr as u64);
                let rec_off =
                    read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "record offset")?;
                let rec_ptr = BASE_OFFSET + rec_off;

                let pf = dds_record_pixel_format(dds.bytes.as_ref());
                if pf != 0 {
                    header_mut_cursor.set_position((rec_ptr + 12) as u64);
                    header_mut_cursor.write_u32::<LittleEndian>(pf)?;
                }
                header_mut_cursor.set_position((rec_ptr + 20) as u64);
                header_mut_cursor.write_u32::<LittleEndian>(mipmap_count)?;
                header_mut_cursor.write_u32::<LittleEndian>(width)?;
                header_mut_cursor.write_u32::<LittleEndian>(height)?;
            }
        }

        let record_len = streamtex_dds.get(e).map_or(0, |d| d.bytes.len());
        let record_len_u32 = u32::try_from(record_len).map_err(|_| {
            anyhow!(
                "[Patch Streamtex Sidecar] DDS record is too large to store: {record_len} bytes"
            )
        })?;
        record_offset = record_offset
            .checked_add(4)
            .and_then(|value| value.checked_add(record_len_u32))
            .ok_or_else(|| anyhow!("[Patch Streamtex Sidecar] record offset overflow"))?;
    }

    // Preserve the embedded catalog data and the name table that follow the table region.
    let mut result_bytes = Vec::new();
    result_bytes.extend_from_slice(&header_bytes);
    result_bytes.extend_from_slice(&original_bytes[BASE_OFFSET + header.table_size..]);
    super::storage::atomic_write(textures_path, &result_bytes)
}

pub(super) fn serialize_bound_sidecar(
    texture_file: &TextureFile,
    original_bytes: &[u8],
) -> Result<Vec<u8>> {
    let layout = texture_file
        .streamtex_layout
        .as_ref()
        .ok_or_else(|| anyhow!("[Patch Streamtex Sidecar] Original stream layout is missing"))?;
    let replacements = super::streamtex::streamtex_replacements(texture_file)?;
    let entries = read_streamed_texture_entries(original_bytes)?;
    let mut cursor = Cursor::new(original_bytes);
    let header = read_textures_header(&mut cursor, "Patch Streamtex Sidecar")?;
    let header_end = BASE_OFFSET + header.table_size;
    let mut output = original_bytes.to_vec();
    let mut writer = Cursor::new(&mut output[..header_end]);
    let bindings: HashMap<_, _> = layout
        .sidecar_bindings
        .iter()
        .map(|binding| (binding.record_index, binding))
        .collect();
    let entries: HashMap<_, _> = entries
        .iter()
        .map(|entry| ((entry.catalog_index, entry.entry_index), entry))
        .collect();
    let mut old_offset = 0usize;
    let mut new_offset = 0usize;
    for (index, record) in layout.records.iter().enumerate() {
        let replacement = replacements
            .get(&index)
            .filter(|dds| dds.modified && dds.bytes != record.bytes);
        let length = replacement.map_or(record.bytes.len(), |dds| dds.bytes.len());
        if let Some(binding) = bindings.get(&index) {
            let entry = entries.get(&(binding.catalog_index, binding.entry_index))
                .filter(|entry| entry.record_id == binding.record_id)
                .ok_or_else(|| anyhow!("[Patch Streamtex Sidecar] Catalog binding changed for stream record {index}; reopen the pair"))?;
            let cat_ptr = BASE_OFFSET
                + read_u32_as_usize(header.cat_off, "catalog offset")?
                + entry.catalog_index * 16;
            cursor.set_position((cat_ptr + 12) as u64);
            let entries_offset =
                read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "entries offset")?;
            let entry_ptr = BASE_OFFSET + entries_offset + entry.entry_index * 12;
            cursor.set_position(entry_ptr as u64);
            let rec_ptr = BASE_OFFSET
                + read_u32_as_usize(cursor.read_u32::<LittleEndian>()?, "record offset")?;
            if old_offset != new_offset || replacement.is_some() {
                writer.set_position((entry_ptr + 8) as u64);
                writer.write_u32::<LittleEndian>(usize_to_u32(new_offset, "stream offset")?)?;
            }
            if let Some(dds) = replacement {
                let (width, height, mips, _) = parse_dds_header(&dds.bytes)?;
                let pf = dds_record_pixel_format(&dds.bytes);
                if pf != 0 {
                    writer.set_position((rec_ptr + 12) as u64);
                    writer.write_u32::<LittleEndian>(pf)?;
                }
                writer.set_position((rec_ptr + 20) as u64);
                writer.write_u32::<LittleEndian>(mips)?;
                writer.write_u32::<LittleEndian>(width)?;
                writer.write_u32::<LittleEndian>(height)?;
            }
        } else if replacement.is_some() || old_offset != new_offset {
            return Err(anyhow!("[Patch Streamtex Sidecar] Stream record {index} has no unambiguous catalog binding; no files were saved"));
        }
        old_offset = old_offset
            .checked_add(4)
            .and_then(|offset| offset.checked_add(record.bytes.len()))
            .ok_or_else(|| anyhow!("[Patch Streamtex Sidecar] Original offset overflow"))?;
        new_offset = new_offset
            .checked_add(4)
            .and_then(|offset| offset.checked_add(length))
            .ok_or_else(|| anyhow!("[Patch Streamtex Sidecar] Replacement offset overflow"))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_RECORD_ID: u32 = 0xD6C9_ED9E;

    fn make_test_dds(
        width: u32,
        height: u32,
        mipmap_count: u32,
        pixel_format: u32,
        data_len: usize,
    ) -> Vec<u8> {
        let mut bytes = vec![0_u8; 128 + data_len];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&mipmap_count.to_le_bytes());
        bytes[84..88].copy_from_slice(&pixel_format.to_le_bytes());
        bytes
    }

    /// Builds a single-catalog `.textures` file mirroring the game layout used by
    /// files such as `Unique_02_BodyPaint.textures`. The name-table marker can be
    /// placed `marker_shift` bytes past its nominal position to reproduce the
    /// stale-size layout produced by external modding tools.
    fn make_single_entry_textures(dds: &[u8], marker_shift: u64) -> Vec<u8> {
        const TABLE_SIZE: u32 = 0x74;
        const CAT_OFF: u32 = 0x58;
        const ENTRIES_OFFSET: u32 = 0x68;
        const REC_OFF: u32 = 0x18;

        let mut bytes = vec![0_u8; BASE_OFFSET + TABLE_SIZE as usize];
        bytes[0..4].copy_from_slice(b"SXET");
        bytes[4..8].copy_from_slice(&0x0C_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&TABLE_SIZE.to_le_bytes());

        bytes[0x10..0x14].copy_from_slice(&1_u32.to_le_bytes()); // static texture count
        bytes[0x14..0x18].copy_from_slice(&REC_OFF.to_le_bytes()); // static texture offset
        bytes[0x20..0x24].copy_from_slice(&1_u32.to_le_bytes()); // catalog count
        bytes[0x24..0x28].copy_from_slice(&CAT_OFF.to_le_bytes()); // catalog offset

        bytes[0x2C..0x30].copy_from_slice(&TEST_RECORD_ID.to_le_bytes()); // record id

        bytes[0x68..0x6C].copy_from_slice(&(4_u32 + dds.len() as u32).to_le_bytes()); // tex_data_size
        bytes[0x70..0x74].copy_from_slice(&1_u32.to_le_bytes()); // entry count
        bytes[0x74..0x78].copy_from_slice(&ENTRIES_OFFSET.to_le_bytes()); // entries offset
        bytes[0x78..0x7C].copy_from_slice(&REC_OFF.to_le_bytes()); // record offset
        bytes[0x80..0x84].copy_from_slice(&0_u32.to_le_bytes()); // data offset

        bytes.extend_from_slice(&(dds.len() as u32).to_le_bytes()); // length prefix
        bytes.extend_from_slice(dds);
        bytes.extend(vec![0_u8; marker_shift as usize]);
        bytes.extend_from_slice(b"MN2C");
        bytes.extend_from_slice(&TEST_RECORD_ID.to_le_bytes());
        bytes.extend_from_slice(&10_u32.to_le_bytes());
        bytes.extend_from_slice(b"BodyPaint!");
        bytes
    }

    fn make_temp_dir() -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "texembed_textures_test_{}_{}",
            std::process::id(),
            stamp
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn read_name_table_handles_stale_size_fields() {
        let dds = make_test_dds(64, 64, 1, 0x3154_5844, 64);
        let bytes = make_single_entry_textures(&dds, 4);

        let mut cursor = Cursor::new(bytes.as_slice());
        let header = read_textures_header(&mut cursor, "test").unwrap();
        let names = read_name_table(&mut cursor, &bytes, header).unwrap();

        assert_eq!(
            names.get(&TEST_RECORD_ID).map(String::as_str),
            Some("BodyPaint!")
        );
    }

    #[test]
    fn save_rebuilds_sizes_for_larger_replacement() {
        let dir = make_temp_dir();
        let path = dir.join("test.textures");

        let original_dds = make_test_dds(64, 64, 1, 0x3154_5844, 64);
        fs::write(&path, make_single_entry_textures(&original_dds, 0)).unwrap();

        let mut texture_file = load_textures_file(path.clone()).unwrap();
        assert_eq!(texture_file.dds_list.len(), 1);
        assert_eq!(texture_file.dds_list[0].bytes.len(), 192);
        assert_eq!(texture_file.dds_list[0].name, "BodyPaint!.dds");

        let replacement = make_test_dds(64, 64, 1, 0, 4096);
        texture_file.dds_list[0].bytes = replacement.into();
        texture_file.dds_list[0].pixel_format = 0;
        texture_file.dds_list[0].modified = true;
        save_textures_file(&texture_file, &path).unwrap();

        let reloaded = load_textures_file(path.clone()).unwrap();
        assert_eq!(reloaded.dds_list[0].bytes.len(), 128 + 4096);
        assert_eq!(reloaded.dds_list[0].name, "BodyPaint!.dds");

        fs::remove_dir_all(dir).unwrap();
    }

    /// Builds a `.textures` file with a single *streamed* catalog
    /// (`tex_data_size == 0`), mirroring the `Frontend\Bodies\<car>` layout:
    /// one 0x40 static record per streamed entry, a 16-byte catalog header and
    /// 12-byte entries. A tail blob is appended after the table region to make
    /// sure `patch_streamtex_sidecar` preserves everything past the header.
    fn make_streamed_textures(entry_count: usize) -> Vec<u8> {
        const ST_OFF: u32 = 0x18;
        let cat_off = ST_OFF + entry_count as u32 * 0x40;
        let entries_offset = cat_off + 0x10;
        let table_size = entries_offset + entry_count as u32 * 12;

        let mut bytes = vec![0_u8; BASE_OFFSET + table_size as usize];
        bytes[0..4].copy_from_slice(b"SXET");
        bytes[12..16].copy_from_slice(&table_size.to_le_bytes());
        bytes[0x10..0x14].copy_from_slice(&(entry_count as u32).to_le_bytes());
        bytes[0x14..0x18].copy_from_slice(&ST_OFF.to_le_bytes());
        bytes[0x20..0x24].copy_from_slice(&1_u32.to_le_bytes());
        bytes[0x24..0x28].copy_from_slice(&cat_off.to_le_bytes());

        // Static records: one per entry, with a stale pf/mips/w/h to be synced.
        for i in 0..entry_count {
            let rec = BASE_OFFSET + ST_OFF as usize + i * 0x40;
            bytes[rec + 4..rec + 8].copy_from_slice(&(1000 + i as u32).to_le_bytes());
            bytes[rec + 12..rec + 16].copy_from_slice(&0x21_u32.to_le_bytes());
            bytes[rec + 20..rec + 24].copy_from_slice(&1_u32.to_le_bytes());
            bytes[rec + 24..rec + 28].copy_from_slice(&8_u32.to_le_bytes());
            bytes[rec + 28..rec + 32].copy_from_slice(&8_u32.to_le_bytes());
        }

        // Streamed catalog header.
        let cat = BASE_OFFSET + cat_off as usize;
        bytes[cat..cat + 4].copy_from_slice(&0_u32.to_le_bytes());
        bytes[cat + 8..cat + 12].copy_from_slice(&(entry_count as u32).to_le_bytes());
        bytes[cat + 12..cat + 16].copy_from_slice(&entries_offset.to_le_bytes());

        // Entries with stale data_off = 0 (must be recomputed).
        for i in 0..entry_count {
            let entry = BASE_OFFSET + entries_offset as usize + i * 12;
            let rec_off = ST_OFF + i as u32 * 0x40;
            bytes[entry..entry + 4].copy_from_slice(&rec_off.to_le_bytes());
        }

        bytes.extend_from_slice(b"MN2C");
        bytes.extend_from_slice(&0xDEADBEEF_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn patch_streamtex_sidecar_recomputes_offsets_and_syncs_records() {
        let dir = make_temp_dir();
        let path = dir.join("test.textures");
        fs::write(&path, make_streamed_textures(3)).unwrap();

        // Record list mirroring a rebuilt .streamtex: DXT1, ARGB (32-bit) and DXT5.
        let dxt1 = make_test_dds(8, 8, 1, u32::from_le_bytes(*b"DXT1"), 100);
        let mut argb = make_test_dds(16, 32, 3, 0, 50);
        argb[76..80].copy_from_slice(&32_u32.to_le_bytes()); // ddspf.dwSize
        argb[80..84].copy_from_slice(&0x41_u32.to_le_bytes()); // DDPF_RGB | DDPF_ALPHAPIXELS
        argb[88..92].copy_from_slice(&32_u32.to_le_bytes()); // dwRGBBitCount
        argb[92..96].copy_from_slice(&0x00FF_0000_u32.to_le_bytes());
        let dxt5 = make_test_dds(8, 8, 1, u32::from_le_bytes(*b"DXT5"), 200);

        let dds_list = vec![
            DdsInfo {
                name: "rec0.dds".into(),
                bytes: dxt1.into(),
                width: 8,
                height: 8,
                mipmap_count: 1,
                pixel_format: u32::from_le_bytes(*b"DXT1"),
                catalog_index: 0,
                entry_index: 0,
                modified: true,
            },
            DdsInfo {
                name: "rec1.dds".into(),
                bytes: argb.into(),
                width: 16,
                height: 32,
                mipmap_count: 3,
                pixel_format: 0,
                catalog_index: 0,
                entry_index: 1,
                modified: true,
            },
            DdsInfo {
                name: "rec2.dds".into(),
                bytes: dxt5.into(),
                width: 8,
                height: 8,
                mipmap_count: 1,
                pixel_format: u32::from_le_bytes(*b"DXT5"),
                catalog_index: 0,
                entry_index: 2,
                modified: true,
            },
        ];

        patch_streamtex_sidecar(&path, &dds_list).unwrap();

        let patched = fs::read(&path).unwrap();
        let u32_at = |off: usize| u32::from_le_bytes(patched[off..off + 4].try_into().unwrap());
        let entries_offset = (0x18 + 3 * 0x40 + 0x10) as usize;
        let entry = |i: usize| BASE_OFFSET + entries_offset + i * 12;

        // data_off = absolute offset of each record's length-prefix.
        let len0 = (128 + 100) as u32;
        let len1 = (128 + 50) as u32;
        assert_eq!(u32_at(entry(0) + 8), 0);
        assert_eq!(u32_at(entry(1) + 8), 4 + len0);
        assert_eq!(u32_at(entry(2) + 8), 4 + len0 + 4 + len1);

        // Records synced: pf (DXT1→0x21, ARGB→0x02, DXT5→0x24), mips/w/h.
        let rec = |i: usize| BASE_OFFSET + 0x18 + i * 0x40;
        assert_eq!(u32_at(rec(0) + 12), 0x21);
        assert_eq!(u32_at(rec(0) + 20), 1);
        assert_eq!(u32_at(rec(1) + 12), 0x02);
        assert_eq!(u32_at(rec(1) + 20), 3);
        assert_eq!(u32_at(rec(1) + 24), 16);
        assert_eq!(u32_at(rec(1) + 28), 32);
        assert_eq!(u32_at(rec(2) + 12), 0x24);

        // The tail after the table region is preserved ("MN2C" + payload).
        assert!(patched.ends_with(&[0x4D, 0x4E, 0x32, 0x43, 0xEF, 0xBE, 0xAD, 0xDE]));

        fs::remove_dir_all(dir).unwrap();
    }
}
