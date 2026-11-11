use std::io::Cursor;

use eframe::egui;
use egui::{ColorImage, TextureOptions};
use image_dds::{ddsfile::Caps2, ddsfile::Dds, ImageFormat, Surface};

pub(super) const THUMBNAIL_MAX_SIDE: usize = 256;
pub(super) const MAX_PREVIEW_RGBA_BYTES: usize = 64 * 1024 * 1024;
pub(super) type DecodedPreview = ([usize; 2], Vec<u8>);

const DECODE_STRIP_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
enum DecodeTarget {
    Thumbnail,
    FullPreview,
    FullResolution,
}

#[derive(Clone, Copy)]
struct SourceMip {
    width: u32,
    height: u32,
    depth: u32,
    offset: usize,
    slice_bytes: usize,
}

struct PreviewSource<'a> {
    payload: &'a [u8],
    format: ImageFormat,
    mipmaps: Vec<SourceMip>,
    layer_stride: usize,
    cubemap: bool,
}

impl<'a> PreviewSource<'a> {
    fn read(dds_bytes: &'a [u8]) -> Result<Self, String> {
        let header_len = if dds_bytes.get(84..88) == Some(b"DX10") {
            148
        } else {
            128
        };
        let source_header = dds_bytes
            .get(..header_len)
            .ok_or_else(|| "DDS header is truncated".to_string())?;
        let mut header = [0u8; 148];
        header[..header_len].copy_from_slice(source_header);
        if &header[84..88] == b"DXT4" {
            header[87] = b'5';
        } else if &header[84..88] == b"DXT2" {
            header[87] = b'3';
        }

        let dds = Dds::read(Cursor::new(&header[..header_len])).map_err(|err| err.to_string())?;
        let width = dds.get_width();
        let height = dds.get_height();
        let depth = if dds.header.caps2.contains(Caps2::VOLUME) {
            u32::from_le_bytes(header[24..28].try_into().unwrap())
        } else {
            dds.get_depth()
        };
        if width == 0 || height == 0 || depth == 0 {
            return Err("DDS dimensions must be nonzero".to_string());
        }
        let mipmap_count = dds.get_num_mipmap_levels().max(1);
        let max_mipmap_count = u32::BITS - width.max(height).max(depth).leading_zeros();
        if mipmap_count > max_mipmap_count {
            return Err("DDS mipmap count exceeds its dimensions".to_string());
        }

        let cubemap = is_cubemap(&dds);
        if cubemap && depth != 1 {
            return Err("DDS cannot be both a cubemap and a volume".to_string());
        }
        let format = image_dds::dds_image_format(&dds)
            .or_else(|err| {
                let flags = u32::from_le_bytes(header[80..84].try_into().unwrap());
                let bits = u32::from_le_bytes(header[88..92].try_into().unwrap());
                if flags & 0x40 != 0 && bits == 32 {
                    Ok(ImageFormat::Bgra8Unorm)
                } else {
                    Err(err)
                }
            })
            .map_err(|err| format!("Unsupported DDS format: {err:?}"))?;

        let (block_width, block_height, block_bytes) = image_format_layout(format)?;
        let mut mipmaps = Vec::with_capacity(mipmap_count as usize);
        let mut layer_stride = 0usize;
        for level in 0..mipmap_count {
            let mip_width = (width >> level).max(1);
            let mip_height = (height >> level).max(1);
            let mip_depth = (depth >> level).max(1);
            let slice_bytes = (mip_width.div_ceil(block_width) as usize)
                .checked_mul(mip_height.div_ceil(block_height) as usize)
                .and_then(|blocks| blocks.checked_mul(block_bytes))
                .ok_or_else(|| "DDS slice dimensions overflow".to_string())?;
            mipmaps.push(SourceMip {
                width: mip_width,
                height: mip_height,
                depth: mip_depth,
                offset: layer_stride,
                slice_bytes,
            });
            layer_stride = slice_bytes
                .checked_mul(mip_depth as usize)
                .and_then(|bytes| layer_stride.checked_add(bytes))
                .ok_or_else(|| "DDS mipmap dimensions overflow".to_string())?;
        }

        Ok(Self {
            payload: &dds_bytes[header_len..],
            format,
            mipmaps,
            layer_stride,
            cubemap,
        })
    }

    fn display_dimensions(&self, level: usize) -> Result<[usize; 2], String> {
        let mip = self.mipmaps[level];
        let width = mip.width as usize;
        let height = mip.height as usize;
        let dimensions = if self.cubemap {
            width.checked_mul(4).zip(height.checked_mul(3))
        } else {
            Some(width).zip(height.checked_mul(mip.depth as usize))
        };
        dimensions
            .map(|(width, height)| [width, height])
            .ok_or_else(|| "DDS preview dimensions overflow".to_string())
    }

    fn plane_data(&self, level: usize, plane: usize) -> Result<&[u8], String> {
        let mip = self.mipmaps[level];
        let stride = if self.cubemap {
            self.layer_stride
        } else {
            mip.slice_bytes
        };
        let start = plane
            .checked_mul(stride)
            .and_then(|offset| offset.checked_add(mip.offset))
            .ok_or_else(|| "DDS plane offset overflows".to_string())?;
        let end = start
            .checked_add(mip.slice_bytes)
            .ok_or_else(|| "DDS plane length overflows".to_string())?;
        self.payload
            .get(start..end)
            .ok_or_else(|| "DDS preview data is truncated".to_string())
    }

    fn thumbnail_mip(&self, target: [usize; 2]) -> Result<usize, String> {
        if self.mipmaps[0].depth > 1 {
            return Ok(0);
        }
        let mut selected = 0;
        for level in 1..self.mipmaps.len() {
            let size = self.display_dimensions(level)?;
            if size[0] < target[0] || size[1] < target[1] {
                break;
            }
            let last_plane = if self.cubemap { 5 } else { 0 };
            if self.plane_data(level, last_plane).is_ok() {
                selected = level;
            }
        }
        Ok(selected)
    }

    fn decode_plane(&self, level: usize, plane: usize) -> Result<Vec<u8>, String> {
        let mip = self.mipmaps[level];
        let width = mip.width as usize;
        let height = mip.height as usize;
        let byte_count = rgba_len([width, height])?;
        let data = self.plane_data(level, plane)?;
        let (block_width, block_height, block_bytes) = image_format_layout(self.format)?;
        let block_height = block_height as usize;
        let encoded_row_bytes = width.div_ceil(block_width as usize) * block_bytes;
        let rgba_row_bytes = width * 4;
        let strip_height =
            (DECODE_STRIP_BYTES / rgba_row_bytes / block_height).max(1) * block_height;
        let mut pixels = allocate_rgba(byte_count)?;
        for top in (0..height).step_by(strip_height) {
            let bottom = top.saturating_add(strip_height).min(height);
            let start = top / block_height * encoded_row_bytes;
            let end = bottom.div_ceil(block_height) * encoded_row_bytes;
            let decoded = Surface {
                width: mip.width,
                height: (bottom - top) as u32,
                depth: 1,
                layers: 1,
                mipmaps: 1,
                image_format: self.format,
                data: &data[start..end],
            }
            .decode_rgba8()
            .map_err(|err| err.to_string())?;
            let output = &mut pixels[top * rgba_row_bytes..bottom * rgba_row_bytes];
            if decoded.data.len() != output.len() {
                return Err("Decoded image has an invalid pixel count".to_string());
            }
            output.copy_from_slice(&decoded.data);
        }
        Ok(pixels)
    }
}

fn image_format_layout(format: ImageFormat) -> Result<(u32, u32, usize), String> {
    use ImageFormat as F;

    match format {
        F::BC1RgbaUnorm | F::BC1RgbaUnormSrgb | F::BC4RUnorm | F::BC4RSnorm => Ok((4, 4, 8)),
        F::BC2RgbaUnorm
        | F::BC2RgbaUnormSrgb
        | F::BC3RgbaUnorm
        | F::BC3RgbaUnormSrgb
        | F::BC5RgUnorm
        | F::BC5RgSnorm
        | F::BC6hRgbUfloat
        | F::BC6hRgbSfloat
        | F::BC7RgbaUnorm
        | F::BC7RgbaUnormSrgb => Ok((4, 4, 16)),
        F::R8Unorm | F::R8Snorm => Ok((1, 1, 1)),
        F::Rg8Unorm
        | F::Rg8Snorm
        | F::R16Unorm
        | F::R16Snorm
        | F::R16Float
        | F::Bgra4Unorm
        | F::Bgr5A1Unorm => Ok((1, 1, 2)),
        F::Bgr8Unorm => Ok((1, 1, 3)),
        F::Rgba8Unorm
        | F::Rgba8UnormSrgb
        | F::Rgba8Snorm
        | F::Bgra8Unorm
        | F::Bgra8UnormSrgb
        | F::Rg16Unorm
        | F::Rg16Snorm
        | F::Rg16Float
        | F::R32Float => Ok((1, 1, 4)),
        F::Rgba16Unorm | F::Rgba16Snorm | F::Rgba16Float | F::Rg32Float => Ok((1, 1, 8)),
        F::Rgb32Float => Ok((1, 1, 12)),
        F::Rgba32Float => Ok((1, 1, 16)),
        _ => Err(format!("Unsupported DDS preview layout: {format:?}")),
    }
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

fn rgba_len(size: [usize; 2]) -> Result<usize, String> {
    if size.contains(&0) {
        return Err("Image dimensions must be nonzero".to_string());
    }
    size[0]
        .checked_mul(size[1])
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .ok_or_else(|| "Image dimensions overflow".to_string())
}

fn allocate_rgba(bytes: usize) -> Result<Vec<u8>, String> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(bytes)
        .map_err(|err| format!("Cannot allocate decoded image: {err}"))?;
    pixels.resize(bytes, 0);
    Ok(pixels)
}

pub(super) fn preview_rgba_len(size: [usize; 2]) -> Result<usize, String> {
    let bytes = rgba_len(size)?;
    if bytes > MAX_PREVIEW_RGBA_BYTES {
        return Err("Preview exceeds the 64 MiB decoded image limit".to_string());
    }
    Ok(bytes)
}

fn thumbnail_dimensions(size: [usize; 2]) -> Result<[usize; 2], String> {
    thumbnail_dimensions_with_limit(size, THUMBNAIL_MAX_SIDE)
}

fn thumbnail_dimensions_with_limit(
    size: [usize; 2],
    max_side: usize,
) -> Result<[usize; 2], String> {
    if size.contains(&0) || max_side == 0 {
        return Err("Preview dimensions must be nonzero".to_string());
    }
    let max_side = max_side.min(THUMBNAIL_MAX_SIDE);
    let longest = size[0].max(size[1]);
    if longest <= max_side {
        return Ok(size);
    }
    Ok(size.map(|dimension| {
        let scaled = (dimension as u128 * max_side as u128 + longest as u128 / 2) / longest as u128;
        (scaled as usize).clamp(1, max_side)
    }))
}

pub(super) fn decode_dds_preview_image(dds_bytes: &[u8]) -> Result<DecodedPreview, String> {
    decode_preview(dds_bytes, DecodeTarget::Thumbnail)
}

pub(super) fn decode_dds_full_preview(dds_bytes: &[u8]) -> Result<DecodedPreview, String> {
    decode_preview(dds_bytes, DecodeTarget::FullPreview)
}

pub(super) fn decode_dds_rgba(dds_bytes: &[u8]) -> Result<DecodedPreview, String> {
    decode_preview(dds_bytes, DecodeTarget::FullResolution)
}

fn decode_preview(dds_bytes: &[u8], target: DecodeTarget) -> Result<DecodedPreview, String> {
    let source = PreviewSource::read(dds_bytes)?;
    let original_size = source.display_dimensions(0)?;
    let thumbnail_size = if matches!(target, DecodeTarget::Thumbnail) {
        Some(thumbnail_dimensions(original_size)?)
    } else {
        None
    };
    let level = if let Some(size) = thumbnail_size {
        source.thumbnail_mip(size)?
    } else {
        0
    };
    let size = source.display_dimensions(level)?;
    let byte_count = match target {
        DecodeTarget::FullResolution => rgba_len(size)?,
        DecodeTarget::Thumbnail | DecodeTarget::FullPreview => preview_rgba_len(size)?,
    };
    let mip = source.mipmaps[level];
    let pixels = if !source.cubemap && mip.depth == 1 {
        source.decode_plane(level, 0)?
    } else {
        let plane_count = if source.cubemap {
            6
        } else {
            mip.depth as usize
        };
        source.plane_data(level, plane_count - 1)?;
        let mut pixels = allocate_rgba(byte_count)?;
        let width = mip.width as usize;
        let height = mip.height as usize;
        let tiles: Vec<(usize, usize)> = if source.cubemap {
            [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)].to_vec()
        } else {
            (0..plane_count).map(|plane| (0, plane)).collect()
        };
        for (plane, &(tile_x, tile_y)) in tiles.iter().enumerate() {
            let decoded = source.decode_plane(level, plane)?;
            for y in 0..height {
                let src_start = y * width * 4;
                let dst_start = ((tile_y * height + y) * size[0] + tile_x * width) * 4;
                pixels[dst_start..dst_start + width * 4]
                    .copy_from_slice(&decoded[src_start..src_start + width * 4]);
            }
        }
        pixels
    };

    if pixels.len() != byte_count {
        return Err("Decoded preview has an invalid pixel count".to_string());
    }
    if let Some(target) = thumbnail_size {
        resize_rgba((size, pixels), target)
    } else {
        Ok((size, pixels))
    }
}

pub(super) fn resize_thumbnail(
    decoded: DecodedPreview,
    max_side: usize,
) -> Result<DecodedPreview, String> {
    let target = thumbnail_dimensions_with_limit(decoded.0, max_side)?;
    resize_rgba(decoded, target)
}

fn resize_rgba(
    (size, pixels): DecodedPreview,
    target: [usize; 2],
) -> Result<DecodedPreview, String> {
    if pixels.len() != preview_rgba_len(size)? {
        return Err("Decoded preview has an invalid pixel count".to_string());
    }
    if size == target {
        return Ok((size, pixels));
    }
    let mut resized = allocate_rgba(preview_rgba_len(target)?)?;
    let scale_x = size[0] as f64 / target[0] as f64;
    let scale_y = size[1] as f64 / target[1] as f64;
    for y in 0..target[1] {
        let top = y as f64 * scale_y;
        let bottom = (y + 1) as f64 * scale_y;
        for x in 0..target[0] {
            let left = x as f64 * scale_x;
            let right = (x + 1) as f64 * scale_x;
            let mut sum = [0.0f64; 4];
            for source_y in top.floor() as usize..(bottom.ceil() as usize).min(size[1]) {
                let weight_y = bottom.min((source_y + 1) as f64) - top.max(source_y as f64);
                for source_x in left.floor() as usize..(right.ceil() as usize).min(size[0]) {
                    let weight_x = right.min((source_x + 1) as f64) - left.max(source_x as f64);
                    let start = (source_y * size[0] + source_x) * 4;
                    let pixel = &pixels[start..start + 4];
                    let alpha = f64::from(pixel[3]) * weight_x * weight_y;
                    for channel in 0..3 {
                        sum[channel] += f64::from(pixel[channel]) * alpha;
                    }
                    sum[3] += alpha;
                }
            }
            let start = (y * target[0] + x) * 4;
            let output = &mut resized[start..start + 4];
            output[3] = (sum[3] / (scale_x * scale_y)).round().clamp(0.0, 255.0) as u8;
            if output[3] != 0 && sum[3] > 0.0 {
                for channel in 0..3 {
                    output[channel] = (sum[channel] / sum[3]).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    Ok((target, resized))
}

pub(super) fn upload_decoded_preview(
    ctx: &egui::Context,
    texture_name: &str,
    size: [usize; 2],
    pixels: &[u8],
) -> Result<egui::TextureHandle, String> {
    let expected = preview_rgba_len(size)?;
    if pixels.len() != expected {
        return Err("Decoded preview has an invalid pixel count".to_string());
    }
    let max_side = ctx.input(|input| input.max_texture_side);
    if size[0] > max_side || size[1] > max_side {
        return Err(format!(
            "Preview exceeds the GPU texture limit of {max_side} pixels"
        ));
    }
    let image = ColorImage::from_rgba_unmultiplied(size, pixels);
    // egui-wgpu only creates single-mip textures and its sampler ignores the
    // mip filter, so all scaling is plain bilinear (dithering is disabled
    // globally in main.rs to keep the preview clean).
    let options = TextureOptions::LINEAR;
    Ok(ctx.load_texture(texture_name, image, options))
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

#[cfg(test)]
mod tests {
    use image_dds::ddsfile::{D3DFormat, Header};

    use super::*;

    fn bgra_header(width: u32, height: u32, depth: u32, mipmaps: u32, cubemap: bool) -> Vec<u8> {
        let caps = if cubemap {
            Some(Caps2::CUBEMAP | Caps2::CUBEMAP_ALLFACES)
        } else if depth > 1 {
            Some(Caps2::VOLUME)
        } else {
            None
        };
        let header = Header::new_d3d(
            height,
            width,
            (depth > 1).then_some(depth),
            D3DFormat::A8R8G8B8,
            (mipmaps > 1).then_some(mipmaps),
            caps,
        )
        .unwrap();
        let dds = Dds {
            header,
            header10: None,
            data: Vec::new(),
        };
        let mut bytes = Vec::new();
        dds.write(&mut bytes).unwrap();
        bytes
    }

    fn append_color(bytes: &mut Vec<u8>, color: [u8; 4], count: usize) {
        for _ in 0..count {
            bytes.extend_from_slice(&color);
        }
    }

    fn pixel(image: &DecodedPreview, x: usize, y: usize) -> &[u8] {
        let start = (y * image.0[0] + x) * 4;
        &image.1[start..start + 4]
    }

    fn png_round_trip((size, pixels): &DecodedPreview) -> image::RgbaImage {
        let mut png = Cursor::new(Vec::new());
        image::write_buffer_with_format(
            &mut png,
            pixels,
            u32::try_from(size[0]).unwrap(),
            u32::try_from(size[1]).unwrap(),
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        image::load_from_memory_with_format(png.get_ref(), image::ImageFormat::Png)
            .unwrap()
            .to_rgba8()
    }

    #[test]
    fn format_layouts_match_public_surface_ranges() {
        use ImageFormat as F;

        let data = [0u8; 2048];
        for format in [
            F::R8Unorm,
            F::R8Snorm,
            F::Rg8Unorm,
            F::Rg8Snorm,
            F::Rgba8Unorm,
            F::Rgba8UnormSrgb,
            F::Rgba16Float,
            F::Rgba32Float,
            F::Bgr8Unorm,
            F::Bgra8Unorm,
            F::Bgra8UnormSrgb,
            F::Bgra4Unorm,
            F::BC1RgbaUnorm,
            F::BC1RgbaUnormSrgb,
            F::BC2RgbaUnorm,
            F::BC2RgbaUnormSrgb,
            F::BC3RgbaUnorm,
            F::BC3RgbaUnormSrgb,
            F::BC4RUnorm,
            F::BC4RSnorm,
            F::BC5RgUnorm,
            F::BC5RgSnorm,
            F::BC6hRgbUfloat,
            F::BC6hRgbSfloat,
            F::BC7RgbaUnorm,
            F::BC7RgbaUnormSrgb,
            F::Rgba8Snorm,
            F::R16Unorm,
            F::R16Snorm,
            F::Rg16Unorm,
            F::Rg16Snorm,
            F::Rgba16Unorm,
            F::Rgba16Snorm,
            F::R16Float,
            F::Rg16Float,
            F::R32Float,
            F::Rg32Float,
            F::Rgb32Float,
            F::Bgr5A1Unorm,
        ] {
            let (block_width, block_height, block_bytes) = image_format_layout(format).unwrap();
            for (width, height) in [(1u32, 1u32), (3, 7), (5, 9), (8, 8)] {
                let surface = Surface {
                    width,
                    height,
                    depth: 1,
                    layers: 1,
                    mipmaps: 1,
                    image_format: format,
                    data: &data[..],
                };
                let bytes = width.div_ceil(block_width) as usize
                    * height.div_ceil(block_height) as usize
                    * block_bytes;
                assert_eq!(
                    surface.get(0, 0, 0).unwrap().len(),
                    bytes,
                    "{format:?} at {width}x{height}"
                );
            }
        }
    }

    #[test]
    fn compressed_mips_preserve_block_padding_and_cubemap_face_offsets() {
        let blocks_per_mip = [9, 1, 1, 1];
        for (format, block_bytes) in [
            (D3DFormat::DXT1, 8),
            (D3DFormat::DXT3, 16),
            (D3DFormat::DXT5, 16),
        ] {
            let header = Header::new_d3d(
                9,
                9,
                None,
                format,
                Some(4),
                Some(Caps2::CUBEMAP | Caps2::CUBEMAP_ALLFACES),
            )
            .unwrap();
            let mut data = Vec::new();
            for face in 0..6u8 {
                for (level, blocks) in blocks_per_mip.into_iter().enumerate() {
                    let marker = face * 4 + level as u8 + 1;
                    data.resize(data.len() + blocks * block_bytes, marker);
                }
            }
            let dds = Dds {
                header,
                header10: None,
                data,
            };
            let mut bytes = Vec::new();
            dds.write(&mut bytes).unwrap();
            let source = PreviewSource::read(&bytes).unwrap();
            assert_eq!(source.layer_stride, 12 * block_bytes);
            for face in 0..6 {
                for (level, blocks) in blocks_per_mip.into_iter().enumerate() {
                    let plane = source.plane_data(level, face).unwrap();
                    let marker = (face * 4 + level + 1) as u8;
                    assert_eq!(plane.len(), blocks * block_bytes);
                    assert!(plane.iter().all(|byte| *byte == marker));
                }
            }
            let truncated = PreviewSource::read(&bytes[..bytes.len() - 1]).unwrap();
            assert!(truncated.plane_data(3, 5).is_err());
        }
    }

    #[test]
    fn encoded_slice_overflow_is_rejected_before_allocation() {
        let mut bytes = bgra_header(1, 1, 1, 1, false);
        bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_dds_rgba(&bytes)
            .unwrap_err()
            .contains("DDS slice dimensions overflow"));
    }

    #[test]
    fn thumbnails_preserve_aspect_ratio_and_do_not_upscale() {
        for (source, expected) in [
            ([4096, 2048], [256, 128]),
            ([2048, 4096], [128, 256]),
            ([512, 512], [256, 256]),
            ([512, 384], [256, 192]),
            ([64, 512], [32, 256]),
            ([300, 75], [256, 64]),
            ([63, 31], [63, 31]),
            ([1, 4096], [1, 256]),
            ([usize::MAX, usize::MAX], [256, 256]),
        ] {
            assert_eq!(thumbnail_dimensions(source).unwrap(), expected);
        }
        assert!(thumbnail_dimensions([0, 16]).is_err());
    }

    #[test]
    fn thumbnail_uses_a_source_mip_while_full_preview_uses_mip_zero() {
        let mut bytes = bgra_header(512, 256, 1, 3, false);
        append_color(&mut bytes, [10, 20, 200, 255], 512 * 256);
        append_color(&mut bytes, [220, 30, 40, 255], 256 * 128);
        append_color(&mut bytes, [0, 255, 0, 255], 128 * 64);

        let thumbnail = decode_dds_preview_image(&bytes).unwrap();
        assert_eq!(thumbnail.0, [256, 128]);
        assert_eq!(pixel(&thumbnail, 0, 0), [40, 30, 220, 255]);
        let full = decode_dds_rgba(&bytes).unwrap();
        assert_eq!(full.0, [512, 256]);
        assert_eq!(pixel(&full, 0, 0), [200, 20, 10, 255]);
    }

    #[test]
    fn png_export_preserves_full_resolution_mip_zero_pixels_and_alpha() {
        let mut bytes = bgra_header(513, 257, 1, 2, false);
        let mut expected = Vec::new();
        for y in 0..257u32 {
            for x in 0..513u32 {
                let rgba = [x as u8, y as u8, (x ^ y) as u8, (x + y) as u8];
                bytes.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
                expected.extend_from_slice(&rgba);
            }
        }
        append_color(&mut bytes, [200, 0, 0, 255], 256 * 128);

        let thumbnail = decode_dds_preview_image(&bytes).unwrap();
        assert_eq!(thumbnail.0, [256, 128]);
        assert_eq!(pixel(&thumbnail, 0, 0), [0, 0, 200, 255]);
        let full = decode_dds_rgba(&bytes).unwrap();
        assert_eq!(full.0, [513, 257]);
        assert_eq!(full.1, expected);
        let exported = png_round_trip(&full);
        assert_eq!(exported.dimensions(), (513, 257));
        assert_eq!(exported.as_raw(), &expected);
    }

    #[test]
    fn png_export_preserves_full_cubemap_cross_and_volume_strip() {
        let mut cube = bgra_header(128, 128, 1, 2, true);
        for face in 0..6u8 {
            append_color(&mut cube, [0, 0, face + 1, 255], 128 * 128);
            append_color(&mut cube, [0, face + 10, 0, 255], 64 * 64);
        }
        let full = decode_dds_rgba(&cube).unwrap();
        let exported = png_round_trip(&full);
        assert_eq!(exported.dimensions(), (512, 384));
        assert_eq!(exported.as_raw(), &full.1);
        assert_eq!(exported.get_pixel(256, 128).0, [1, 0, 0, 255]);
        assert_eq!(exported.get_pixel(384, 128).0, [6, 0, 0, 255]);
        assert_eq!(exported.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(decode_dds_preview_image(&cube).unwrap().0, [256, 192]);

        let mut volume = bgra_header(64, 64, 8, 2, false);
        for slice in 0..8u8 {
            append_color(&mut volume, [0, slice + 1, 0, 255], 64 * 64);
        }
        append_color(&mut volume, [255, 0, 0, 255], 32 * 32 * 4);
        let full = decode_dds_rgba(&volume).unwrap();
        let exported = png_round_trip(&full);
        assert_eq!(exported.dimensions(), (64, 512));
        assert_eq!(exported.as_raw(), &full.1);
        for slice in 0..8 {
            assert_eq!(
                exported.get_pixel(0, slice * 64).0,
                [0, slice as u8 + 1, 0, 255]
            );
        }
        assert_eq!(decode_dds_preview_image(&volume).unwrap().0, [32, 256]);
    }

    #[test]
    fn missing_lower_mip_falls_back_to_available_source_data() {
        let mut bytes = bgra_header(512, 128, 1, 2, false);
        append_color(&mut bytes, [10, 20, 30, 255], 512 * 128);
        let thumbnail = decode_dds_preview_image(&bytes).unwrap();
        assert_eq!(thumbnail.0, [256, 64]);
        assert_eq!(pixel(&thumbnail, 0, 0), [30, 20, 10, 255]);
    }

    #[test]
    fn thumbnail_resizes_textures_without_mipmaps() {
        let mut bytes = bgra_header(300, 75, 1, 1, false);
        append_color(&mut bytes, [10, 20, 30, 255], 300 * 75);
        let thumbnail = decode_dds_preview_image(&bytes).unwrap();
        assert_eq!(thumbnail.0, [256, 64]);
        assert_eq!(thumbnail.1.len(), 256 * 64 * 4);
        assert_eq!(pixel(&thumbnail, 0, 0), [30, 20, 10, 255]);
    }

    #[test]
    fn cubemap_cross_preserves_face_placements_and_transparent_corners() {
        let mut bytes = bgra_header(2, 2, 1, 1, true);
        for face in 0..6u8 {
            append_color(&mut bytes, [0, 0, face + 1, 255], 4);
        }
        let full = decode_dds_rgba(&bytes).unwrap();
        assert_eq!(full.0, [8, 6]);
        for (face, (x, y)) in [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(pixel(&full, x * 2, y * 2), [face as u8 + 1, 0, 0, 255]);
        }
        assert_eq!(pixel(&full, 0, 0), [0, 0, 0, 0]);
        assert_eq!(decode_dds_preview_image(&bytes).unwrap(), full);
    }

    #[test]
    fn cubemap_thumbnail_accounts_for_cross_dimensions_when_selecting_mips() {
        let mut bytes = bgra_header(128, 128, 1, 2, true);
        for face in 0..6u8 {
            append_color(&mut bytes, [0, 0, face + 1, 255], 128 * 128);
            append_color(&mut bytes, [0, face + 10, 0, 255], 64 * 64);
        }
        let thumbnail = decode_dds_preview_image(&bytes).unwrap();
        assert_eq!(thumbnail.0, [256, 192]);
        assert_eq!(pixel(&thumbnail, 128, 64), [0, 10, 0, 255]);
        assert_eq!(pixel(&thumbnail, 192, 64), [0, 15, 0, 255]);
    }

    #[test]
    fn volume_preview_preserves_all_slices_and_strip_aspect_ratio() {
        let mut bytes = bgra_header(2, 2, 3, 2, false);
        for slice in 0..3u8 {
            append_color(&mut bytes, [0, slice + 1, 0, 255], 4);
        }
        append_color(&mut bytes, [255, 0, 0, 255], 1);
        let full = decode_dds_rgba(&bytes).unwrap();
        assert_eq!(full.0, [2, 6]);
        for slice in 0..3 {
            assert_eq!(pixel(&full, 0, slice * 2), [0, slice as u8 + 1, 0, 255]);
        }
        assert_eq!(decode_dds_preview_image(&bytes).unwrap(), full);

        let mut large = bgra_header(64, 64, 8, 1, false);
        append_color(&mut large, [0, 50, 0, 255], 64 * 64 * 8);
        assert_eq!(decode_dds_preview_image(&large).unwrap().0, [32, 256]);
    }

    #[test]
    fn legacy_volume_caps_preserve_depth_without_the_header_depth_flag() {
        let mut bytes = bgra_header(2, 2, 2, 1, false);
        let flags = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) & !0x0080_0000;
        bytes[8..12].copy_from_slice(&flags.to_le_bytes());
        append_color(&mut bytes, [0, 10, 0, 255], 4);
        append_color(&mut bytes, [0, 20, 0, 255], 4);
        let full = decode_dds_rgba(&bytes).unwrap();
        assert_eq!(full.0, [2, 4]);
        assert_eq!(pixel(&full, 0, 2), [0, 20, 0, 255]);
    }

    #[test]
    fn truncated_or_oversized_preview_is_rejected_before_decoding() {
        assert!(decode_dds_rgba(b"DDS ").is_err());
        let bytes = bgra_header(8, 8, 1, 1, false);
        assert!(decode_dds_preview_image(&bytes).is_err());

        let bytes = bgra_header(8192, 8192, 1, 1, false);
        assert!(decode_dds_full_preview(&bytes)
            .unwrap_err()
            .contains("64 MiB"));
        assert!(decode_dds_preview_image(&bytes)
            .unwrap_err()
            .contains("64 MiB"));
        assert!(preview_rgba_len([usize::MAX, usize::MAX]).is_err());
        assert!(preview_rgba_len([0, 1]).is_err());
    }

    #[test]
    fn full_resolution_policy_allows_large_exports_without_preview_limits() {
        assert_eq!(rgba_len([8192, 8192]).unwrap(), 256 * 1024 * 1024);
        assert!(preview_rgba_len([8192, 8192]).is_err());
        assert!(rgba_len([usize::MAX, 1]).is_err());
        assert!(rgba_len([0, 8192]).is_err());
        assert!(allocate_rgba(usize::MAX).is_err());

        for bytes in [
            bgra_header(8192, 8192, 1, 1, false),
            bgra_header(2048, 2048, 1, 1, true),
            bgra_header(2048, 2048, 8, 1, false),
        ] {
            assert!(decode_dds_rgba(&bytes).unwrap_err().contains("truncated"));
            assert!(decode_dds_full_preview(&bytes)
                .unwrap_err()
                .contains("64 MiB"));
        }
    }

    #[test]
    fn strip_decoding_preserves_compressed_blocks_and_partial_final_rows() {
        for format in [
            D3DFormat::A8R8G8B8,
            D3DFormat::DXT1,
            D3DFormat::DXT3,
            D3DFormat::DXT5,
        ] {
            let header = Header::new_d3d(517, 513, None, format, None, None).unwrap();
            let mut dds = Dds {
                header,
                header10: None,
                data: Vec::new(),
            };
            let format = image_dds::dds_image_format(&dds).unwrap();
            let (block_width, block_height, block_bytes) = image_format_layout(format).unwrap();
            let count = 513u32.div_ceil(block_width) as usize
                * 517u32.div_ceil(block_height) as usize
                * block_bytes;
            dds.data = (0..count).map(|index| (index % 251) as u8).collect();
            let expected = Surface {
                width: 513,
                height: 517,
                depth: 1,
                layers: 1,
                mipmaps: 1,
                image_format: format,
                data: dds.data.as_slice(),
            }
            .decode_rgba8()
            .unwrap();
            let mut bytes = Vec::new();
            dds.write(&mut bytes).unwrap();
            let full = decode_dds_rgba(&bytes).unwrap();
            assert_eq!(full.0, [513, 517]);
            assert_eq!(full.1, expected.data, "{format:?}");
            assert_eq!(decode_dds_full_preview(&bytes).unwrap(), full);
        }
    }

    #[test]
    fn thumbnail_alpha_edges_do_not_bleed_invisible_rgb() {
        let mut bytes = bgra_header(512, 1, 1, 1, false);
        for _ in 0..256 {
            bytes.extend_from_slice(&[0, 0, 255, 255, 255, 0, 0, 0]);
        }
        let thumbnail = decode_dds_preview_image(&bytes).unwrap();
        assert_eq!(thumbnail.0, [256, 1]);
        assert!(thumbnail
            .1
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 128]));
        let full = decode_dds_rgba(&bytes).unwrap();
        assert_eq!(pixel(&full, 1, 0), [0, 0, 255, 0]);
    }

    #[test]
    fn thumbnail_resampling_preserves_fractional_coverage_and_low_alpha_colors() {
        let fractional = resize_thumbnail(
            ([3, 1], vec![255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 255]),
            2,
        )
        .unwrap();
        assert_eq!(fractional, ([2, 1], vec![255, 0, 0, 170, 0, 255, 0, 170]));
        let faint = resize_thumbnail(([2, 1], vec![64, 0, 0, 1, 0, 0, 255, 0]), 1).unwrap();
        assert_eq!(faint, ([1, 1], vec![64, 0, 0, 1]));
    }

    #[test]
    fn fully_transparent_thumbnails_have_zero_rgb_after_resizing() {
        let resized = resize_thumbnail(([2, 2], [10, 90, 250, 0].repeat(4)), 1).unwrap();
        assert_eq!(resized, ([1, 1], vec![0; 4]));
        assert!(resize_thumbnail(([2, 2], vec![0; 4]), 1).is_err());
        assert!(resize_thumbnail(([1, 1], vec![0; 4]), 0).is_err());
    }

    #[test]
    fn invalid_mipmap_count_is_rejected_without_shifting_or_allocating() {
        let mut bytes = bgra_header(4, 4, 1, 2, false);
        bytes[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_dds_preview_image(&bytes)
            .unwrap_err()
            .contains("mipmap count"));
    }

    #[test]
    fn legacy_premultiplied_fourcc_keeps_preview_compatibility() {
        for (format, fourcc) in [(D3DFormat::DXT3, b"DXT2"), (D3DFormat::DXT5, b"DXT4")] {
            let header = Header::new_d3d(4, 4, None, format, None, None).unwrap();
            let dds = Dds {
                header,
                header10: None,
                data: vec![0; 16],
            };
            let mut bytes = Vec::new();
            dds.write(&mut bytes).unwrap();
            let expected = decode_dds_rgba(&bytes).unwrap();
            bytes[84..88].copy_from_slice(fourcc);
            assert_eq!(decode_dds_rgba(&bytes).unwrap(), expected);
        }
    }

    #[test]
    fn upload_validates_pixel_count_and_gpu_dimensions() {
        let ctx = egui::Context::default();
        assert!(upload_decoded_preview(&ctx, "invalid", [2, 2], &[0; 4]).is_err());
        let max_side = ctx.input(|input| input.max_texture_side);
        let pixels = vec![0; (max_side + 1) * 4];
        assert!(upload_decoded_preview(&ctx, "too wide", [max_side + 1, 1], &pixels).is_err());
    }
}
