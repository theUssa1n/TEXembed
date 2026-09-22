use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use byteorder::{LittleEndian, ReadBytesExt};

use super::textures::load_textures_name_list;
use crate::dds::parse_dds_header;
use crate::model::{DdsInfo, FileType, TextureFile};

fn load_streamtex_sidecar_names(path: &Path) -> Option<Vec<String>> {
    let textures_path = path.with_extension("textures");
    if !textures_path.is_file() {
        return None;
    }

    load_textures_name_list(&textures_path)
        .ok()
        .filter(|names| !names.is_empty())
}

/// Loads a `.streamtex` file and extracts embedded DDS payloads.
///
/// # Errors
///
/// Returns an error when the file cannot be read or a texture entry cannot be
/// parsed from the binary stream.
pub fn load_streamtex_file(path: PathBuf) -> Result<TextureFile> {
    let original_bytes = fs::read(&path)?;
    let mut cursor = Cursor::new(&original_bytes);
    let mut textures = Vec::new();
    let sidecar_names = load_streamtex_sidecar_names(&path);

    loop {
        let current_offset = cursor.position();
        let texture_size = match cursor.read_u32::<LittleEndian>() {
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => {
                return Err(anyhow!(
                    "[Streamtex file] Error reading texture size at offset 0x{current_offset:08X}: {err}"
                ))
            }
            Ok(s) => s,
        };

        let mut texture_bytes = vec![
            0;
            usize::try_from(texture_size).map_err(|_| anyhow!(
                "[Streamtex file] Invalid texture size {texture_size} at offset 0x{current_offset:08X}"
            ))?
        ];
        let read_offset = cursor.position();
        cursor.read_exact(&mut texture_bytes).map_err(|err| {
            anyhow!(
                "[Streamtex file] Error reading {texture_size} bytes of texture data at offset 0x{read_offset:08X}: {err}"
            )
        })?;
        textures.push(texture_bytes);
    }

    let mut dds_list = Vec::new();

    for (i, bytes) in textures.into_iter().enumerate() {
        if let Ok((width, height, mipmap_count, pixel_format)) = parse_dds_header(&bytes) {
            let name = sidecar_names
                .as_deref()
                .and_then(|names| names.get(i))
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("{i}.dds"));
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
        let mut bytes = vec![0_u8; 128];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&mipmap_count.to_le_bytes());
        bytes[84..88].copy_from_slice(&pixel_format.to_le_bytes());
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

    fn make_test_textures(names: &[&str]) -> Vec<u8> {
        const TEXTURES_MAGIC: u32 = 0x5445_5853;
        const NAME_TABLE_MARKER: u32 = 0x4332_4E4D;
        const BASE_OFFSET: usize = 0x10;
        const CAT_OFF: u32 = 0x18;

        let base_offset = u32::try_from(BASE_OFFSET).unwrap();
        let entry_count = u32::try_from(names.len()).unwrap();
        let entries_offset = CAT_OFF + 16;
        let records_offset = entries_offset + entry_count * 12;
        let table_size = records_offset + entry_count * 8;
        let mut bytes = vec![0_u8; BASE_OFFSET + usize::try_from(table_size).unwrap()];
        let mut cursor = Cursor::new(&mut bytes);

        cursor.write_u32::<LittleEndian>(TEXTURES_MAGIC).unwrap();
        cursor.set_position(0xC);
        cursor.write_u32::<LittleEndian>(table_size).unwrap();
        cursor.set_position(u64::try_from(BASE_OFFSET).unwrap());
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(1).unwrap();
        cursor.write_u32::<LittleEndian>(CAT_OFF).unwrap();

        cursor.set_position(u64::from(base_offset + CAT_OFF));
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(entry_count).unwrap();
        cursor.write_u32::<LittleEndian>(entries_offset).unwrap();

        for (index, _) in names.iter().enumerate() {
            let index = u32::try_from(index).unwrap();
            let rec_off = records_offset + index * 8;
            cursor.set_position(u64::from(base_offset + entries_offset + index * 12));
            cursor.write_u32::<LittleEndian>(rec_off).unwrap();
            cursor.write_u32::<LittleEndian>(0).unwrap();
            cursor.write_u32::<LittleEndian>(0).unwrap();

            cursor.set_position(u64::from(base_offset + rec_off + 4));
            cursor.write_u32::<LittleEndian>(index + 1).unwrap();
        }

        bytes.extend_from_slice(&NAME_TABLE_MARKER.to_le_bytes());
        for (index, name) in names.iter().enumerate() {
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

    #[test]
    fn load_streamtex_applies_sidecar_names_by_entry_index() {
        let dir = make_temp_dir();
        let streamtex_path = dir.join("frontend.streamtex");
        let textures_path = dir.join("frontend.textures");
        let first = make_test_dds(64, 64, 1, 0x3154_5844);
        let invalid = vec![0_u8; 32];
        let third = make_test_dds(128, 128, 1, 0x3554_5844);

        fs::write(
            &streamtex_path,
            make_test_streamtex(&[first, invalid, third]),
        )
        .unwrap();
        fs::write(
            &textures_path,
            make_test_textures(&["Body_Front", "Skipped", "Body_Rear"]),
        )
        .unwrap();

        let file = load_streamtex_file(streamtex_path).unwrap();

        assert_eq!(file.dds_list.len(), 2);
        assert_eq!(file.dds_list[0].entry_index, 0);
        assert_eq!(file.dds_list[0].name, "Body_Front.dds");
        assert_eq!(file.dds_list[1].entry_index, 2);
        assert_eq!(file.dds_list[1].name, "Body_Rear.dds");

        fs::remove_dir_all(dir).unwrap();
    }
}
