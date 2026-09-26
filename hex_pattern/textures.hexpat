#pragma MIME application/split-second-textures
#pragma description Split/Second .textures (PC, little-endian) - complete structure

// ---------------------------------------------------------------------------
// Complete ImHex pattern for the PC flavour of Split/Second `.textures` files.
//
// Derived from the in-tree implementation:
//   src/formats/textures.rs   (PC/`SXET`, little-endian)
//   src/dds.rs                (DDS header field offsets)
//
// Layout summary (all offsets are absolute in the file):
//   0x00  char   magic[4]                 "SXET" (PC)
//   0x04  u32    version                  format version, expected 0x0C
//   0x08  u32    unused                   present but ignored by the loader
//   0x0C  u32    table_size               size of the record/table region
//   0x10  struct TextureObjectTable       begins at the fixed BASE_OFFSET (0x10)
//   ...   u8[]   embedded payloads        per-catalog blobs, `[u32 len][DDS ]`
//   ...   struct NameTable                optional, prefixed by "MN2C"
// ---------------------------------------------------------------------------

#include <std/core.pat>
#include <std/io.pat>
#include <std/ptr.pat>

// The loader always treats offset 0x10 as the start of the object table, so all
// table pointers inside the file are relative to this fixed base, not to 0.
#define BASE_OFFSET 0x10

// Pointer-base helper: all table pointers are stored relative to BASE_OFFSET,
// not to their parent object. (std::ptr::relative_to_parent is not usable here.)
fn relative_to_base(auto ptr) {
    return BASE_OFFSET;
};

// ---------------------------------------------------------------------------
// Static texture record (`cTextureStatic`), 0x40 bytes.
//
// One record per texture: it describes the *shape and format* of the payload,
// whether that payload is embedded in this file or lives in a `.streamtex`
// sibling. `id` links the record to its entry in the optional name table.
//
// Known `pixel_format` codes (game-side enum, not the DDS FourCC):
//   0x21 = DXT1   0x22 = DXT2   0x23 = DXT3   0x24 = DXT4/DXT5 (BC3)
//   0x02 = A8R8G8B8 (uncompressed 32-bit)     (others follow the 0x20+family)
// ---------------------------------------------------------------------------
struct TextureRecord {
    u32 vftable_pointer;    // +0x00 runtime vtable, always 0 on disk
    u32 id;                 // +0x04 texture id, key into the name table
    u16 runtime_ffff;       // +0x08 initialised 0xFFFF at runtime
    u16 runtime_zero;       // +0x0A initialised 0 at runtime
    u32 pixel_format;       // +0x0C game format code (see table above)
    u32 sides;              // +0x10 1 = 2D, 6 = cubemap, other = volume depth
    u32 mipmap_count;       // +0x14 number of mip levels in the payload
    u32 width;              // +0x18 base width in pixels
    u32 height;             // +0x1C base height in pixels
    u32 sampler_state_value;// +0x20 DWORD for IDirect3DDevice9::SetSamplerState
    u32 id3d_texture;       // +0x24 runtime IDirect3DBaseTexture9*, 0 on disk
    u8  unknown[24];        // +0x28 unused/padding to fill the 0x40 record
};

// ---------------------------------------------------------------------------
// Proxy texture record (`cTextureProxy`), 8 bytes. Points at another texture.
// ---------------------------------------------------------------------------
struct TextureProxy {
    u32 vftable_pointer;    // +0x00 runtime vtable, always 0 on disk
    u32 runtime_zero;       // +0x04 initialised 0 at runtime
};

// ---------------------------------------------------------------------------
// Catalog entry, 12 bytes. Lives inside the entry array of a catalog.
//
//   records_offset  relative to BASE_OFFSET -> the TextureRecord for this slot
//   unknown_offset  purpose unknown, preserved verbatim on save
//   data_offset     if the catalog has inline data, this is the offset of the
//                   `[u32 len][DDS ]` blob within that catalog's data region
//                   (relative to the catalog data start). For streamed
//                   catalogs (`texture_data_size == 0`) it is an offset into
//                   the sibling `.streamtex` file instead.
// ---------------------------------------------------------------------------
struct CatalogEntry {
    u32 records_offset [[comment("relative to 0x10 base -> TextureRecord")]];
    u32 unknown_offset;
    u32 data_offset    [[comment("catalog-relative (embedded) or streamtex offset")]];
};

// ---------------------------------------------------------------------------
// Catalog header, 16 bytes, immediately followed by `entry_count` entries.
//
//   texture_data_size  0 => this catalog is *streamed*: its pixel data is not
//                      stored inline but in the sibling `<name>.streamtex`.
//                      Non-zero => the size in bytes of the inline data region
//                      that follows the table region for this catalog.
//   entry_count        number of 12-byte entries in the array.
//   entries_offset     offset of the entry array, relative to the 0x10 base.
//                      Modelled as a pointer array so the entries are expanded.
// ---------------------------------------------------------------------------
struct Catalog {
    u32 texture_data_size [[comment("0 = streamed (.streamtex sidecar)")]];
    u32 unknown;
    u32 entry_count;
    // `entries_offset` (relative to the 0x10 base) is stored right here. It is
    // declared as a pointer array so the `entry_count` entries are expanded in
    // place: only the 4-byte offset itself is consumed, keeping the catalog
    // stride at 16 bytes. A plain `u32` would leave the entries unparsed.
    CatalogEntry *entries[entry_count] : u32
        [[pointer_base("relative_to_base")]];
};

// ---------------------------------------------------------------------------
// Texture object table at BASE_OFFSET. Each of the three (count, offset) pairs
// is optional; a zero count (and/or zero offset) means that section is absent.
// ---------------------------------------------------------------------------
struct TextureObjectTable {
    u32 static_texture_count;
    TextureRecord *static_textures[static_texture_count] : u32
        [[pointer_base("relative_to_base")]];

    u32 proxy_texture_count;
    TextureProxy *proxy_textures[proxy_texture_count] : u32
        [[pointer_base("relative_to_base")]];

    u32 catalog_count;
    Catalog *catalogs[catalog_count] : u32
        [[pointer_base("relative_to_base")]];
};

// ---------------------------------------------------------------------------
// A single embedded payload: `[u32 length][DDS file]`. `data_length` counts
// only the DDS bytes that follow it (it excludes the 4-byte length prefix).
// ---------------------------------------------------------------------------
struct EmbeddedDds {
    u32 data_length [[comment("byte count of the DDS blob that follows")]];
    u8  dds[data_length] [[comment("begins with \"DDS \" then the 128-byte header")]];
};

// ---------------------------------------------------------------------------
// Name table records, optional. Preceded by the 4-byte marker "MN2C" and then
// repeated `[u32 id][u32 size][char bytes[size]]` entries mapping an id to a
// human-readable name. Files without a name table simply end after the data.
// ---------------------------------------------------------------------------
struct Name {
    u32 id;
    u32 size;
    char value[size];
};

// ===========================================================================
// Root
// ===========================================================================

char magic[4] @ $;
if (magic == "SXET") {
    // PC builds are little-endian.
    std::core::set_endian(std::mem::Endian::Little);
} else if (magic == "TEXS") {
    // PS3 builds are big-endian; this pattern targets the PC layout only, so
    // flag it instead of silently mis-parsing every following field.
    std::core::set_endian(std::mem::Endian::Big);
    std::error("This is a PS3 (\"TEXS\", big-endian) file; this pattern covers the PC \"SXET\" layout.");
} else {
    std::error("Magic is neither \"SXET\" nor \"TEXS\"; not a .textures file?");
}

u32 version @ $ [[comment("expected 0x0C; the loader errors on any other value")]];
u32 unused  @ $ [[comment("present in the header but ignored by the loader")]];
u32 table_size @ $ [[comment("size of the record/table region that follows")]];

// The object table starts at the fixed BASE_OFFSET (0x10).
TextureObjectTable table @ BASE_OFFSET;

// Walk past the whole table region. The loader reads `table_size` bytes and
// jumps over them, so we mirror that here.
u32 name_count = 0;
$ = addressof(table) + table_size;

// The loader advances past each catalog's inline data region; the running sum
// of `texture_data_size` lands exactly on the name-table marker.
for (u32 i = 0, i < table.catalog_count, i += 1) {
    name_count = table.catalogs[i].entry_count;
    $ += table.catalogs[i].texture_data_size;
}

// The name table is optional: only present when the marker sits before EOF.
struct NameTable {
    if ($ < sizeof($)) {
        char name_table_marker[4] [[comment("expected \"MN2C\"")]];
        Name names[name_count];
    }
};

NameTable name_table @ $;
