use std::io::Cursor;

use eframe::egui;
use egui::{ColorImage, TextureOptions};
use image_dds::{ddsfile::Caps2, ddsfile::Dds, image_from_dds, SurfaceRgba8};

use super::state::PreviewState;

pub(super) fn decode_dds_preview_image(dds_bytes: &[u8]) -> Result<([usize; 2], Vec<u8>), String> {
    decode_dds_rgba(dds_bytes)
}

pub(super) fn upload_decoded_preview(
    ctx: &egui::Context,
    texture_name: &str,
    size: [usize; 2],
    pixels: &[u8],
) -> PreviewState {
    let image = ColorImage::from_rgba_unmultiplied(size, pixels);
    // Grid thumbnails draw large source textures (often 512px-4096px) into a
    // small ~120px card. Without a mipmap chain, the GPU samples the base
    // level directly at that minification ratio, which produces aliased,
    // noisy-looking results compared to the original texture. Enabling
    // mipmap_mode builds a proper mip pyramid and samples it with linear
    // (trilinear) filtering, giving a clean downsample. This only affects
    // how the texture is *sampled*, not the decoded data, so the full
    // preview (near 1:1 or zoomed in) is unaffected.
    let options = TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
    let texture = ctx.load_texture(texture_name, image, options);
    PreviewState::Ready(texture)
}

pub(super) fn decode_dds_rgba(dds_bytes: &[u8]) -> Result<([usize; 2], Vec<u8>), String> {
    // Volume (3D) textures cannot be shown as an image, so preview the flat
    // atlas form that export writes as well.
    let flattened = super::volume::flatten_volume_to_atlas(dds_bytes);
    let dds_bytes = flattened.as_deref().unwrap_or(dds_bytes);
    let mut patched_storage;
    let bytes_for_read = if dds_bytes.len() >= 88 {
        let fourcc = &dds_bytes[84..88];
        if fourcc == b"DXT4" || fourcc == b"DXT2" {
            patched_storage = dds_bytes.to_vec();
            patched_storage[87] = if fourcc == b"DXT4" { b'5' } else { b'3' };
            patched_storage.as_slice()
        } else {
            dds_bytes
        }
    } else {
        dds_bytes
    };
    let mut cursor = Cursor::new(bytes_for_read);
    let dds = Dds::read(&mut cursor).map_err(|err| err.to_string())?;

    if let Ok(decoded) = decode_dds_rgba_smart(&dds, 0) {
        return Ok(decoded);
    }

    if let Some(decoded) = decode_legacy_uncompressed(&dds, dds_bytes) {
        return Ok(decoded);
    }

    Err("Unsupported DDS format".to_string())
}

fn decode_dds_rgba_smart(dds: &Dds, mipmap: u32) -> Result<([usize; 2], Vec<u8>), String> {
    if is_cubemap(dds) {
        return decode_cubemap_cross_rgba(dds, mipmap);
    }

    let layers = dds.get_num_array_layers().max(1);
    let depth = dds.get_depth().max(1);

    if layers > 1 || depth > 1 {
        return decode_first_layer_first_slice_rgba(dds, mipmap);
    }

    let image = image_from_dds(dds, mipmap).map_err(|err| err.to_string())?;
    let size = [image.width() as usize, image.height() as usize];
    Ok((size, image.into_raw()))
}

fn is_cubemap(dds: &Dds) -> bool {
    if dds.header.caps2.contains(Caps2::CUBEMAP) {
        return true;
    }
    matches!(
        &dds.header10,
        Some(header10) if header10.misc_flag == image_dds::ddsfile::MiscFlag::TEXTURECUBE
    )
}

fn decode_first_layer_first_slice_rgba(
    dds: &Dds,
    mipmap: u32,
) -> Result<([usize; 2], Vec<u8>), String> {
    let surface = SurfaceRgba8::decode_layers_mipmaps_dds(dds, 0..1, mipmap..mipmap + 1)
        .map_err(|err| err.to_string())?;

    let width = surface.width as usize;
    let height = surface.height as usize;
    let slice = surface
        .get(0, 0, 0)
        .ok_or_else(|| "Failed to extract DDS slice".to_string())?
        .to_vec();

    Ok(([width, height], slice))
}

fn decode_cubemap_cross_rgba(dds: &Dds, mipmap: u32) -> Result<([usize; 2], Vec<u8>), String> {
    let surface = SurfaceRgba8::decode_layers_mipmaps_dds(dds, 0..6, mipmap..mipmap + 1)
        .map_err(|err| err.to_string())?;

    let face_w = surface.width as usize;
    let face_h = surface.height as usize;
    let out_w = face_w * 4;
    let out_h = face_h * 3;

    let mut out = vec![0u8; out_w * out_h * 4];
    let placements = [
        (2usize, 1usize, 0u32),
        (0usize, 1usize, 1u32),
        (1usize, 0usize, 2u32),
        (1usize, 2usize, 3u32),
        (1usize, 1usize, 4u32),
        (3usize, 1usize, 5u32),
    ];

    for (tile_x, tile_y, face) in placements {
        let face_rgba = surface
            .get(face, 0, 0)
            .ok_or_else(|| "Failed to extract DDS cubemap face".to_string())?;

        for y in 0..face_h {
            let src_start = y * face_w * 4;
            let dst_x = tile_x * face_w;
            let dst_y = tile_y * face_h + y;
            let dst_start = (dst_y * out_w + dst_x) * 4;
            out[dst_start..dst_start + face_w * 4]
                .copy_from_slice(&face_rgba[src_start..src_start + face_w * 4]);
        }
    }

    Ok(([out_w, out_h], out))
}

fn decode_legacy_uncompressed(dds: &Dds, dds_bytes: &[u8]) -> Option<([usize; 2], Vec<u8>)> {
    let width = dds.get_width() as usize;
    let height = dds.get_height() as usize;
    let data = &dds.data;

    if dds_bytes.len() >= 128 {
        let pf_flags =
            u32::from_le_bytes([dds_bytes[80], dds_bytes[81], dds_bytes[82], dds_bytes[83]]);
        let ddpf_rgb = (pf_flags & 0x40) != 0;
        if ddpf_rgb {
            let rgb_bit_count =
                u32::from_le_bytes([dds_bytes[88], dds_bytes[89], dds_bytes[90], dds_bytes[91]]);
            if rgb_bit_count == 32 && data.len() >= width * height * 4 {
                let mut rgba = Vec::with_capacity(width * height * 4);
                for chunk in data.as_chunks::<4>().0.iter().take(width * height) {
                    rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
                }
                return Some(([width, height], rgba));
            }
        }
    }
    None
}

pub(super) fn patch_legacy_fourcc(dds_bytes: &[u8]) -> Vec<u8> {
    let mut bytes = dds_bytes.to_vec();
    if bytes.len() >= 88 {
        if &bytes[84..88] == b"DXT4" {
            bytes[87] = b'5';
        } else if &bytes[84..88] == b"DXT2" {
            bytes[87] = b'3';
        }
    }
    bytes
}

pub(super) fn normalize_legacy_pixel_format(pixel_format: u32) -> u32 {
    let mut bytes = pixel_format.to_le_bytes();
    if &bytes == b"DXT4" {
        bytes[3] = b'5';
        u32::from_le_bytes(bytes)
    } else if &bytes == b"DXT2" {
        bytes[3] = b'3';
        u32::from_le_bytes(bytes)
    } else {
        pixel_format
    }
}

pub(super) fn format_pixel_format_u32(pixel_format: u32) -> String {
    if pixel_format == 0 {
        return "None".to_string();
    }
    let bytes = pixel_format.to_le_bytes();
    match &bytes {
        b"DXT1" => "BC1 (DXT1)".to_string(),
        b"DXT2" => "BC2 (DXT2)".to_string(),
        b"DXT3" => "BC2 (DXT3)".to_string(),
        b"DXT4" => "BC3 (DXT4)".to_string(),
        b"DXT5" => "BC3 (DXT5)".to_string(),
        b"DX10" => "DX10".to_string(),
        _ => "Unknown".to_string(),
    }
}

pub(super) fn format_pixel_format(dds_bytes: &[u8]) -> String {
    if dds_bytes.len() < 128 {
        return "Unknown".to_string();
    }
    let fourcc_bytes = &dds_bytes[84..88];
    match fourcc_bytes {
        b"DXT1" => return "BC1 (DXT1)".to_string(),
        b"DXT2" => return "BC2 (DXT2)".to_string(),
        b"DXT3" => return "BC2 (DXT3)".to_string(),
        b"DXT4" => return "BC3 (DXT4)".to_string(),
        b"DXT5" => return "BC3 (DXT5)".to_string(),
        _ => {}
    }

    let pf_flags = u32::from_le_bytes([dds_bytes[80], dds_bytes[81], dds_bytes[82], dds_bytes[83]]);
    let ddpf_rgb = (pf_flags & 0x40) != 0;
    if ddpf_rgb {
        let r_mask =
            u32::from_le_bytes([dds_bytes[92], dds_bytes[93], dds_bytes[94], dds_bytes[95]]);
        if r_mask == 0x00FF_0000 {
            return "B8G8R8A8 (A8R8G8B8)".to_string();
        }
    }
    "Unknown".to_string()
}

pub(super) fn is_b8g8r8a8(dds_bytes: &[u8]) -> bool {
    if dds_bytes.len() < 128 {
        return false;
    }

    let pf_flags = u32::from_le_bytes([dds_bytes[80], dds_bytes[81], dds_bytes[82], dds_bytes[83]]);
    if (pf_flags & 0x40) == 0 {
        return false;
    }

    let rgb_bit_count =
        u32::from_le_bytes([dds_bytes[88], dds_bytes[89], dds_bytes[90], dds_bytes[91]]);
    if rgb_bit_count != 32 {
        return false;
    }

    let r_mask = u32::from_le_bytes([dds_bytes[92], dds_bytes[93], dds_bytes[94], dds_bytes[95]]);
    let g_mask = u32::from_le_bytes([dds_bytes[96], dds_bytes[97], dds_bytes[98], dds_bytes[99]]);
    let b_mask = u32::from_le_bytes([
        dds_bytes[100],
        dds_bytes[101],
        dds_bytes[102],
        dds_bytes[103],
    ]);
    let a_mask = u32::from_le_bytes([
        dds_bytes[104],
        dds_bytes[105],
        dds_bytes[106],
        dds_bytes[107],
    ]);

    r_mask == 0x00FF_0000 && g_mask == 0x0000_FF00 && b_mask == 0x0000_00FF && a_mask == 0xFF00_0000
}