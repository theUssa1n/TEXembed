use std::io::Cursor;

use anyhow::{anyhow, Result};
use byteorder::{LittleEndian, ReadBytesExt};

/// Parses the core DDS header fields needed by the app.
///
/// # Errors
///
/// Returns an error when the byte slice is shorter than a DDS header, the
/// magic bytes do not match `DDS `, or the header cannot be read.
pub fn parse_dds_header(dds_bytes: &[u8]) -> Result<(u32, u32, u32, u32)> {
    if dds_bytes.len() < 128 {
        return Err(anyhow!(
            "[DDS Parser] File is too small to be a DDS file (got {} bytes, needs at least 128)",
            dds_bytes.len()
        ));
    }
    let magic = &dds_bytes[0..4];
    if magic != b"DDS " {
        return Err(anyhow!(
            "[DDS Parser] Invalid DDS header magic number: expected 'DDS ', got '{magic:02X?}'"
        ));
    }
    let mut cursor = Cursor::new(dds_bytes);
    cursor.set_position(12);
    let height = cursor.read_u32::<LittleEndian>()?;
    let width = cursor.read_u32::<LittleEndian>()?;
    cursor.set_position(28);
    let mipmap_count = cursor.read_u32::<LittleEndian>()?;
    cursor.set_position(84);
    let pixel_format = cursor.read_u32::<LittleEndian>()?;
    Ok((width, height, mipmap_count, pixel_format))
}
