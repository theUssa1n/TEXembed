#pragma MIME application/split-second-streamtex
#pragma description Split/Second .streamtex (PC, little-endian) - length-prefixed DDS records

// ---------------------------------------------------------------------------
// Complete ImHex pattern for the PC flavour of Split/Second `.streamtex` files.
//
// Derived from the in-tree implementation:
//   src/formats/streamtex.rs   (read_streamtex_records / load_streamtex_file)
//   src/dds.rs                 (DDS header field offsets)
//
// A `.streamtex` file has no header and no table: it is a flat sequence of
// records, each one `[u32 little-endian length][DDS file]`, running to EOF.
// `length` counts only the DDS bytes that follow the prefix (it excludes the
// prefix itself). The sibling `<name>.textures` carries the per-record names
// and shapes: its *streamed* catalog (`texture_data_size == 0`) lists one
// entry per record here, and each entry's `data_offset` is the record's
// length-prefix position inside this file.
//
// NOTE: this covers the PC (`DDS`-based) layout only. The PS3 `.streamtex` is
// plain raw texture data (128-byte aligned slots, no DDS headers and no length
// prefixes), so there is nothing structured to parse there.
//
// `length` is not always trustworthy in retail data: a few records store a
// value that is a handful of bytes off, which would desynchronise every record
// that follows. The DDS header fully describes its own payload, so the size is
// recomputed from the header and preferred whenever it is plausible, exactly
// like the loader does.
// ---------------------------------------------------------------------------

#include <std/core.pat>
#include <std/io.pat>

// PC builds are little-endian.
std::core::set_endian(std::mem::Endian::Little);

// ---------------------------------------------------------------------------
// DDS pixel format block (32 bytes), embedded at +0x4C of the DDS header.
//   four_cc        block-compressed FourCC ("DXT1", "DXT5", ...) or 0 for
//                  uncompressed data, which is then described by the masks.
//   rgb_bit_count  bits per pixel for uncompressed data.
// ---------------------------------------------------------------------------
struct DdsPixelFormat {
    u32 size;
    u32 flags;
    char four_cc[4]      [[comment("e.g. \"DXT1\"/\"DXT5\"; 0 when uncompressed")]];
    u32 rgb_bit_count    [[comment("bits per pixel for uncompressed data")]];
    u32 red_bit_mask;
    u32 green_bit_mask;
    u32 blue_bit_mask;
    u32 alpha_bit_mask;
};

// ---------------------------------------------------------------------------
// DDS header, 128 bytes, at the start of every record's payload.
// ---------------------------------------------------------------------------
struct DdsHeader {
    char magic[4]              [[comment("expected \"DDS \"")]];
    u32 size;
    u32 flags;
    u32 height;
    u32 width;
    u32 pitch_or_linear_size;
    u32 depth                  [[comment("slice count for volume textures")]];
    u32 mipmap_count;
    u32 reserved1[11];
    DdsPixelFormat pixel_format;
    u32 caps;
    u32 caps2                  [[comment("0x200 = cubemap, 0x200000 = volume")]];
    u32 caps3;
    u32 caps4;
    u32 reserved2;
};

// ---------------------------------------------------------------------------
// Bytes per 4x4 block for a block-compressed format; 0 when the format is
// uncompressed (masks/bit count apply) or a DX10 extension we do not parse.
// ---------------------------------------------------------------------------
fn block_bytes(DdsPixelFormat pf) {
    if (pf.four_cc == "DXT1" || pf.four_cc == "ATI1" || pf.four_cc == "BC4U" || pf.four_cc == "BC4S") {
        return 8;
    }
    if (pf.four_cc == "DXT2" || pf.four_cc == "DXT3" || pf.four_cc == "DXT4" || pf.four_cc == "DXT5") {
        return 16;
    }
    return 0;
};

// ---------------------------------------------------------------------------
// Payload size recomputed from the DDS header: the fixed 128-byte header plus
// the mip chain multiplied by the face and slice counts. Returns 0 when the
// format cannot be described (DX10), so the caller can fall back on `length`.
// ---------------------------------------------------------------------------
fn payload_len(DdsHeader h) {
    u32 edge = 4;
    u32 bb = block_bytes(h.pixel_format);
    if (bb == 0) {
        edge = 1;
        bb = h.pixel_format.rgb_bit_count / 8;
    }
    if (bb == 0) {
        return 0;
    }

    u32 w = h.width;
    if (w == 0) { w = 1; }
    u32 ht = h.height;
    if (ht == 0) { ht = 1; }
    u32 mips = h.mipmap_count;
    if (mips == 0) { mips = 1; }
    u32 sides = 1;
    if ((h.caps2 & 0x00000200) != 0) { sides = 6; }
    u32 depth = 1;
    if ((h.caps2 & 0x00200000) != 0) {
        depth = h.depth;
        if (depth == 0) { depth = 1; }
    }

    u32 chain = 0;
    for (u32 i = 0, i < mips, i += 1) {
        chain += ((w + edge - 1) / edge) * ((ht + edge - 1) / edge) * bb;
        w = w / 2;
        if (w == 0) { w = 1; }
        ht = ht / 2;
        if (ht == 0) { ht = 1; }
    }
    return 128 + chain * sides * depth;
};

// ---------------------------------------------------------------------------
// Size the record actually occupies (payload only): the header-derived size
// when it is valid, otherwise the stored `length`. Clamped to 128 so the walk
// always makes progress even on a corrupt entry.
// ---------------------------------------------------------------------------
fn record_payload_len(u32 declared, DdsHeader h) {
    u32 computed = payload_len(h);
    if (computed >= 128) {
        return computed;
    }
    if (declared >= 128) {
        return declared;
    }
    return 128;
};

// ---------------------------------------------------------------------------
// One record: `[u32 length][128-byte DDS header][pixel data]`.
// ---------------------------------------------------------------------------
struct StreamtexRecord {
    u32 declared_length [[comment("length prefix; counts the DDS bytes that follow")]];
    DdsHeader header;
    u8 body[record_payload_len(declared_length, header) - 128]
        [[comment("DDS pixel data: mip chain x faces x slices")]];
};

// ===========================================================================
// Root
// ===========================================================================

// A `.textures` file starts with a magic, not with a length; catch that case
// explicitly instead of mis-parsing it as one giant record.
char magic[4] @ 0;
if (magic == "SXET") {
    std::error("This is a PC .textures (\"SXET\") file; the .streamtex pattern expects a length-prefixed DDS record list.");
}
if (magic == "TEXS") {
    std::error("This is a PS3 .textures (\"TEXS\") file; not a .streamtex file.");
}

// Records run back to back until EOF.
StreamtexRecord records[while($ + 132 <= sizeof($))] @ 0;
