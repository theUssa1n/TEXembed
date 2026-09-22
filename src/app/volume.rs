use std::io::Cursor;

use image_dds::{ddsfile::Dds, ImageFormat, Mipmaps, Quality, SurfaceRgba8};

/// `DDSCAPS2_VOLUME` — marks a DDS as a 3D (volume) texture.
const CAPS2_VOLUME: u32 = 0x0020_0000;
/// `DDSD_DEPTH`
const DDSD_DEPTH: u32 = 0x0080_0000;
/// `DDSD_MIPMAPCOUNT`
const DDSD_MIPMAPCOUNT: u32 = 0x0002_0000;
/// `DDSCAPS_MIPMAP`
const DDSCAPS_MIPMAP: u32 = 0x0040_0000;
/// `DDSCAPS_COMPLEX`
const DDSCAPS_COMPLEX: u32 = 0x0000_0008;
/// `DDSCAPS_TEXTURE`
const DDSCAPS_TEXTURE: u32 = 0x0000_1000;
/// `DDPF_RGB`
const DDPF_RGB: u32 = 0x0000_0040;

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

fn write_u32_at(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Returns the slice count when `dds_bytes` is a volume (3D) texture, else `0`.
///
/// Split/Second stores particle effects that animate through a depth slab
/// (smoke puffs, speed lines) as 3D textures. Their record keeps the slice
/// count in the `sides` field and the DDS header carries `DDSCAPS2_VOLUME`.
pub(super) fn volume_depth(dds_bytes: &[u8]) -> u32 {
    if dds_bytes.len() < 128 || &dds_bytes[0..4] != b"DDS " {
        return 0;
    }
    let depth = u32_at(dds_bytes, 24);
    if u32_at(dds_bytes, 112) & CAPS2_VOLUME != 0 && depth > 1 {
        depth
    } else {
        0
    }
}

/// Block (or pixel) layout of the payload: `(block_width, block_height, block_bytes)`.
fn block_layout(dds_bytes: &[u8]) -> Option<(usize, usize, usize)> {
    match &dds_bytes[84..88] {
        b"DXT1" | b"ATI1" | b"BC4U" => Some((4, 4, 8)),
        b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5" | b"ATI2" | b"BC5U" => Some((4, 4, 16)),
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
            let bytes = usize::try_from(bits / 8).ok()?;
            Some((1, 1, bytes))
        }
    }
}

/// Bytes of one mip 0 slice, padded to whole blocks exactly like a 2D mip.
fn slice_size(width: u32, height: u32, layout: (usize, usize, usize)) -> Option<usize> {
    let (block_w, block_h, block_bytes) = layout;
    usize::try_from(width)
        .ok()?
        .div_ceil(block_w)
        .checked_mul(usize::try_from(height).ok()?.div_ceil(block_h))?
        .checked_mul(block_bytes)
}

/// Dimensions of the flat 2D strip [`flatten_volume_to_atlas`] produces:
/// `(width, height * depth)`. Returns `None` for non-volume textures.
pub(super) fn flattened_dimensions(dds_bytes: &[u8]) -> Option<(u32, u32)> {
    let depth = volume_depth(dds_bytes);
    if depth < 2 {
        return None;
    }
    let height = u32_at(dds_bytes, 12);
    let width = u32_at(dds_bytes, 16);
    height.checked_mul(depth).map(|flat| (width, flat))
}

/// Converts a volume texture into a flat 2D DDS that image editors can open.
///
/// The slices are stacked vertically, which for block aligned widths is a plain
/// concatenation of the existing bytes: the pixels are copied exactly, with no
/// decoding or recompression. Only mip 0 is kept, because a 3D mip chain has no
/// equivalent in a 2D image; the remaining levels are rebuilt when the edited
/// strip is imported back (see [`rebuild_volume_from_strip`]).
///
/// Returns `None` when the input is not a volume texture or its payload is not
/// shaped the way this conversion expects.
pub(super) fn flatten_volume_to_atlas(dds_bytes: &[u8]) -> Option<Vec<u8>> {
    let depth = volume_depth(dds_bytes);
    if depth < 2 {
        return None;
    }
    let layout = block_layout(dds_bytes)?;
    let width = u32_at(dds_bytes, 16);
    let height = u32_at(dds_bytes, 12);
    // Stacking rows only preserves the block order when a slice spans a whole
    // number of blocks; otherwise every row would need repadding.
    if layout.0 > 1 && !width.is_multiple_of(4) {
        return None;
    }

    let slice = slice_size(width, height, layout)?;
    let take = slice.checked_mul(usize::try_from(depth).ok()?)?;
    if dds_bytes.len() < 128 + take {
        return None;
    }
    let flat_height = height.checked_mul(depth)?;

    let mut out = dds_bytes[..128].to_vec();
    write_u32_at(&mut out, 12, flat_height);
    write_u32_at(&mut out, 24, 0);
    write_u32_at(&mut out, 28, 1);
    let flags = u32_at(dds_bytes, 8) & !DDSD_DEPTH & !DDSD_MIPMAPCOUNT;
    write_u32_at(&mut out, 8, flags);
    let caps = u32_at(dds_bytes, 108) & !DDSCAPS_MIPMAP & !DDSCAPS_COMPLEX | DDSCAPS_TEXTURE;
    write_u32_at(&mut out, 108, caps);
    write_u32_at(&mut out, 112, 0);
    // `pitchOrLinearSize`: block formats store the top level's total byte count,
    // uncompressed formats store the row pitch.
    let pitch_or_linear_size = if layout.0 == 1 {
        width.checked_mul(u32::try_from(layout.2).ok()?)?
    } else {
        u32::try_from(take).ok()?
    };
    write_u32_at(&mut out, 20, pitch_or_linear_size);

    out.extend_from_slice(&dds_bytes[128..128 + take]);
    Some(out)
}

/// Map a record's DDS fourCC to the format used to re-encode a rebuilt volume.
pub(super) fn volume_image_format(dds_bytes: &[u8]) -> Option<ImageFormat> {
    match &dds_bytes[84..88] {
        b"DXT1" | b"ATI1" | b"BC4U" => Some(ImageFormat::BC1RgbaUnorm),
        b"DXT2" | b"DXT3" => Some(ImageFormat::BC2RgbaUnorm),
        b"DXT4" | b"DXT5" | b"ATI2" | b"BC5U" => Some(ImageFormat::BC3RgbaUnorm),
        _ => None,
    }
}

/// Rebuilds a volume texture from the flat strip produced by
/// [`flatten_volume_to_atlas`].
///
/// The strip's pixels are split back into slices and the mip chain is
/// regenerated, then the original volume header is reused so the record keeps
/// its `sides`/dimension fields and the game's D3D9 style fourCC.
///
/// # Errors
///
/// Returns a message when the strip cannot be decoded, its size does not match
/// the slice stack, or the encoded payload does not fit the expected volume.
pub(super) fn rebuild_volume_from_strip(
    strip_bytes: &[u8],
    original_dds: &[u8],
    width: u32,
    height: u32,
    depth: u32,
    mipmap_count: u32,
    format: ImageFormat,
) -> Result<Vec<u8>, String> {
    if original_dds.len() < 128 {
        return Err("Original volume header is missing".to_string());
    }

    let dds = Dds::read(&mut Cursor::new(strip_bytes))
        .map_err(|err| format!("Cannot read the flat volume strip: {err}"))?;
    let rgba = SurfaceRgba8::decode_dds(&dds)
        .map_err(|err| format!("Cannot decode the flat volume strip: {err}"))?;

    let expected_height = height
        .checked_mul(depth)
        .ok_or_else(|| "Volume dimensions overflow".to_string())?;
    if rgba.width != width || rgba.height != expected_height || rgba.layers != 1 {
        return Err(format!(
            "Expected a {width}x{expected_height} strip, received {}x{}",
            rgba.width, rgba.height
        ));
    }
    let expected_pixels = usize::try_from(width)
        .ok()
        .and_then(|w| {
            usize::try_from(expected_height)
                .ok()
                .and_then(|h| w.checked_mul(h))
        })
        .ok_or_else(|| "Volume dimensions overflow".to_string())?;
    if rgba.data.len() < expected_pixels * 4 {
        return Err("The flat volume strip is truncated".to_string());
    }

    let surface = SurfaceRgba8 {
        width,
        height,
        depth,
        layers: 1,
        mipmaps: 1,
        data: rgba.data,
    };
    let mipmaps = if mipmap_count > 1 {
        Mipmaps::GeneratedExact(mipmap_count)
    } else {
        Mipmaps::Disabled
    };
    let encoded = surface
        .encode(format, Quality::Normal, mipmaps)
        .map_err(|err| format!("Cannot re-encode the volume texture: {err}"))?;

    let mut out = original_dds[..128].to_vec();
    write_u32_at(&mut out, 12, height);
    write_u32_at(&mut out, 16, width);
    write_u32_at(&mut out, 24, depth);
    write_u32_at(&mut out, 28, mipmap_count.max(1));
    out.extend_from_slice(&encoded.data);
    Ok(out)
}
