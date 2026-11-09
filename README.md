<div align="center">

<img src="appicon.png" alt="TEXembed logo" width="140">

# TEXembed

**A fast, native texture viewer, extractor and replacer for the PC version of *Split/Second: Velocity*.**

Open `.textures` and `.streamtex` containers, preview every DDS inside them, export textures, and put your own back in.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platform: Windows x64](https://img.shields.io/badge/platform-Windows%20x64-0078D6)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange)
![UI: egui](https://img.shields.io/badge/UI-egui%20%2F%20eframe-8A63D2)
[![Discord](https://img.shields.io/badge/Discord-Join%20Server-5865F2?logo=discord&logoColor=white)](https://discord.gg/EEvCkD8fA3)

</div>

> **Unofficial fan tool.** TEXembed is not affiliated with, endorsed by, or connected to the developers or publisher of Split/Second. It contains no game assets. You need your own copy of the game.

<img width="1920" height="2098" alt="Screenshot (338)" src="https://github.com/user-attachments/assets/111f3841-e9ba-42b0-90ff-a282fd0501b8" />

---

## Table of contents

- [Features](#features)
- [Supported files](#supported-files)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Replacing textures](#replacing-textures)
- [Batch Replace](#batch-replace)
- [Tweaks panel](#tweaks-panel)
- [Special texture types](#special-texture-types)
- [Known limits and safety notes](#known-limits-and-safety-notes)
- [Format documentation (ImHex patterns)](#format-documentation-imhex-patterns)
- [Building from source](#building-from-source)
- [Project layout](#project-layout)
- [Contributing](#contributing)
- [Credits](#credits)
- [License](#license)

---

## Features

- **Open `.textures` and `.streamtex` files** by drag and drop, from the left rail buttons, or by double-clicking them in Explorer once the installer has registered the extensions.
- **Texture grid with live previews** of every DDS inside the container, with mipmapped thumbnails so large textures stay clean at small sizes.
- **Inspector panel** showing the properties of the selected texture (name, size, mipmap count, pixel format and more).
- **Full Preview mode** with mouse-wheel zoom, drag-to-pan, a Reset button, and a transparency background toggle for checking alpha channels.
- **Export** the selected texture as DDS (optionally PNG too), or **Export all as DDS** into a folder.
- **Replace** a texture with your own DDS through a file dialog, or just **drag a DDS onto the preview thumbnail** in the Inspector. Changes stay in memory with **Undo** until you save.
- **Batch Replace**: match a whole folder of DDS files against the open container (by name or by index) and replace everything in one step.
- **Open folder view** and **Shared view**: browse many files at once in a single combined gallery, or open each file in its own tab.
- **Tabs and shortcuts**: `Ctrl + Tab` switches files, `Ctrl + W` closes the current one, `Ctrl + S` saves.
- **Cubemaps and 3D (volume) textures** are previewed as a cross layout and a flat slice atlas respectively. See [Special texture types](#special-texture-types).
- **Legacy DDS compatibility**: obsolete `DXT2`/`DXT4` textures are normalised to `DXT3`/`DXT5`, and uncompressed `B8G8R8A8` data is decoded correctly.
- **Optional Tweaks** that relax the strict replacement rules (different compression, size, mip count) and automatically patch the file headers, records and paired files on save.
- **Single instance**: opening another file while TEXembed is running forwards it to the existing window.
- **Dark, compact UI** built with [egui](https://github.com/emilk/egui). No installers for runtimes, no web view, no background services.

## Supported files

| Extension | Container | What TEXembed does |
|-----------|-----------|--------------------|
| `.textures` | `SXET` container: header, catalog table, 12-byte catalog entries, static texture records, optional `MN2C` name table | Loads embedded DDS textures, reads names, previews, exports, replaces and saves. Streamed catalogs point at records in the paired `.streamtex`. |
| `.streamtex` | Flat sequence of `[u32 length][DDS file]` records, no header | Loads each record as a DDS, with the same preview, export, replace and save workflow. |
| `.dds` | Standard DDS | Used for replacement and export. |

Scope notes:

- Only the **PC (little-endian, DDS-based)** layout is supported. The console `.streamtex` layout stores raw texture data without DDS headers and is not handled.
- `.textures` and `.streamtex` files are usually **paired** by name. Keep the two files together in their original folder structure when you resize records (see [Tweaks panel](#tweaks-panel)).

## Installation

### Option 1: Windows installer (recommended)

1. Download `TEXembed_win-x64.exe` from the [Releases](https://github.com/theUssa1n/TEXembed/releases) page.
2. Run it. The installer needs administrator rights because it installs to `Program Files` and registers the file types machine-wide.
3. Optionally tick **Create a desktop shortcut**.

The installer:

- Installs `texembed.exe` to `C:\Program Files\TEXembed` (64-bit only, Windows 10 or 11).
- Associates **`.textures`** and **`.streamtex`** with TEXembed and gives them the TEXembed icon.
- Cleans up leftover per-user registrations from older 32-bit versions.
- Uninstalls cleanly and removes the associations again.

### Option 2: Build it yourself

See [Building from source](#building-from-source).

## Quick start

1. **Open files.** Drag `.textures` / `.streamtex` files into the window, or use the buttons on the left rail:
   - **Open files**: pick one or more files.
   - **Open folder view**: scan a whole folder into one gallery.
2. **Browse.** Click any texture in the grid to see its properties in the Inspector on the right.
3. **Inspect.** Click the preview in the Inspector for Full Preview. Scroll to zoom, drag to pan, **Reset** to restore. Toggle the transparency background to check alpha.
4. **Export.** Use the Inspector to export the selected texture as DDS (and optionally PNG), or **Export all as DDS** for the whole tab.
5. **Replace.** Select a texture, click **Replace** (or drop a DDS on the preview) and choose your edited file.
6. **Save.** Press `Ctrl + S` or click **Save changes** to write the modified container to disk. Use **Undo** if you change your mind before saving.

When you drop several files at once, TEXembed asks whether to open them in **Shared view** (one combined gallery, recommended) or **Separate tabs**. Both modes offer an **Include multi-texture files** option.

### Keyboard shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl + Tab` | Switch between open files |
| `Ctrl + W` | Close the current file |
| `Ctrl + S` | Save changes to disk |

## Replacing textures

By default, a replacement DDS must match the original on **all three** of these:

| Requirement | Rule |
|-------------|------|
| **Dimensions** | Width and height must match the original exactly. |
| **Mipmaps** | The mipmap count must be identical. |
| **Compression format** | The pixel format must match (for example BC1 with BC1, BC3 with BC3). Obsolete `DXT4` replacements are accepted and normalised to `DXT5`. |

Mismatched files are rejected with a message in the log instead of silently corrupting the container. The three rules can be relaxed through the [Tweaks panel](#tweaks-panel) when you know what you are doing.

Replacing keeps the change **in memory**. Nothing touches the original file until you save, and an **Undo** button appears as soon as there is something to undo.

> **Tip:** Dithering and banding on gradients are usually caused by BC1/DXT1 compression. Replacing such a texture with an uncompressed `A8R8G8B8` (ARGB) DDS removes it. This needs the *Allow different compression* option, see below.

## Batch Replace

For many textures at once, open a file and click **Batch Replace...** in the **Tools** section of the Inspector.

1. **Replacement DDS folder**: click **Browse...** and choose the folder containing your DDS files.
2. **Matching rule**:
   - **Exact name (recommended)**: matches by file name.
   - **Index (`0.dds` or `texture_0.dds`)**: matches by the texture's position in the file.
3. **Allow different compression (patch .textures header)**: optional checkbox, same as the [Tweaks](#tweaks-panel) option of the same name.
4. **Scan Folder**: previews the result as counters: **Ready**, **Not found**, **Issues** and **Unused DDS**, with the size of each file.
5. **Apply Matched (N)**: replaces every ready texture in one step, still in memory. The usual Undo and Save workflow applies afterwards.

Details:

- Several entries can share the same name. **Export all as DDS** suffixes those with the entry index (for example `name_12.dds`), and Batch Replace routes such files back to their exact entry.
- The same technical requirements apply to every file in the batch. Mismatched files are **skipped and reported**, not applied.

## Tweaks panel

The Tweaks panel (the last button on the left rail) holds advanced switches. **All of them are off by default**, which is the safest setting. Turn one on only when you understand what it does.

| Option | What it does | When to use it |
|--------|--------------|----------------|
| **Allow different compression for `.textures`** (patch header on save) | Lets a replacement use another compression, for example BC1 to BC3 or an uncompressed ARGB texture. On save, the record before the DDS and the catalog sizes are patched so the game loads the new format. Width, height and mip count must still match. | A replacement keeps being skipped for a compression mismatch. |
| **Allow a different size and mip count on replace** | Accepts a replacement with another resolution and mip chain (for example 2048x2048 with a full chain instead of 1024x1024 single mip). On save, the `.textures` record and catalog sizes (or the `.streamtex` length prefix) are rewritten. | You genuinely need a different resolution. Not recommended for 3D (volume) textures, whose depth cannot be resized. |
| **Allow different compression and size for `.streamtex` records** | Same idea for records inside a `.streamtex`. The new image is stored as-is and the record's length value is rewritten on save. ARGB replacements are reduced to mip 0 because the game's menu loader only accepts that form. | Optional and mainly meant for testing. |
| **Auto-patch the paired `.textures` when saving a `.streamtex`** | After saving a `.streamtex`, the sibling `.textures` is updated: streamed-catalog offsets are recomputed and each record's format, mips, width and height are synced from the new DDS headers. | Keep it ON together with the option above whenever a record changes size. |

Important points:

- The auto-patch works **only when the same-named `.textures` sits next to the `.streamtex`** in its proper folder. Do not resize records of a standalone `.streamtex`: without the paired file, the record locations cannot be fixed and the game will read broken data.
- A log note tells you whenever a header is patched.

## Special texture types

- **Cubemaps** are shown as a horizontal-cross layout in the preview, assembled from all six faces.
- **Volume (3D) textures** cannot be shown as a flat image, so TEXembed previews and exports them as a **vertical strip of depth slices** (an atlas). When you re-import an edited strip, the slices are split again and the mip chain is rebuilt, keeping the original record's header fields. Depth itself cannot be changed.
- **Array textures** with several layers or slices preview the first layer and slice.
- **Obsolete formats**: `DXT2` and `DXT4` are treated as `DXT3` and `DXT5`; plain 32-bit `A8R8G8B8` data is decoded correctly.

## Known limits and safety notes

- **Keep backups.** Always work on a copy of the game's files, or keep the originals somewhere safe. Saving overwrites the container.
- **Offsets matter.** The engine locates records by absolute offsets and does not walk files sequentially. If one `.streamtex` record changes size, every later record shifts, so the paired `.textures` must be updated (the auto-patch option does this). Stale offsets show up as wrong or black textures.
- **Menu / frontend streamed loader.** It only accepts uncompressed records with a single mip level. A full mip chain can stop a vehicle from rendering at all, with no crash.
- **About 6 MB cap for frontend and UI `.streamtex` files.** The game sets a fixed limit. A larger file cannot be handled and will crash the game.
- **Body paint with ARGB.** For `bodypaint.textures`, use a game build that supports it (the in-app guide recommends the *SSDC v2* build). Then enable the `.textures` compression option, replace with your `A8R8G8B8` DDS, keep the same width, height and mip count, and save.
- The tool targets the **PC** version of the game only.

The app also ships its own **About** panel with the General Guide and a Dithering document, plus a format reference. Open it from the left rail.

## Format documentation (ImHex patterns)

The [`hex_pattern`](hex_pattern) folder contains [ImHex](https://imhex.werwolv.net/) pattern files that describe both formats, derived directly from this tool's implementation:

| File | Describes |
|------|-----------|
| [`textures.hexpat`](hex_pattern/textures.hexpat) | `.textures` (`SXET`) container: header, object table, 0x40-byte static texture records, catalog entries, embedded payloads, optional `MN2C` name table. |
| [`streamtex.hexpat`](hex_pattern/streamtex.hexpat) | `.streamtex` length-prefixed DDS records, including the recomputed-size logic used when a stored length is slightly off. |

Load one in ImHex on top of a real file to explore the structure byte by byte.

## Building from source

**Requirements**

- A recent stable [Rust toolchain](https://rustup.rs/) (the code uses recent standard library APIs such as `slice::as_chunks` and `is_multiple_of`, so keep `rustup` up to date).
- Windows for the full experience (icon embedding with `winres`, file associations, installer). The core format code is platform independent.

**Build**

```bash
git clone https://github.com/theUssa1n/TEXembed.git
cd TEXembed
cargo build --release
```

The executable is written to `target/release/texembed.exe`. The release profile uses fat LTO, a single codegen unit, `panic = "abort"` and stripped symbols for a small, fast binary. `build.rs` embeds `icon.ico` into the executable on Windows.

**Run**

```bash
cargo run --release -- "path\to\file.textures"
```

**Tests**

```bash
cargo test
```

**Build the installer**

1. Install [Inno Setup](https://jrsoftware.org/isinfo.php) 6.4 or newer.
2. Build the release executable (see above).
3. Open `texembed_installer.iss` and compile it. The output is `TEXembed_win-x64.exe`.

## Project layout

```text
TEXembed/
├── src/
│   ├── main.rs              # entry point, single-instance IPC, window setup
│   ├── lib.rs               # public API of the format library
│   ├── model.rs             # TextureFile / DdsInfo data model
│   ├── dds.rs               # DDS header parsing
│   ├── formats/
│   │   ├── textures.rs      # .textures (SXET) load / save / patching
│   │   └── streamtex.rs     # .streamtex load / save
│   └── app/
│       ├── state.rs         # application state, open/export/replace/save logic
│       ├── ui.rs            # eframe update loop, theme and style
│       ├── panels_shell.rs  # left rail, inspector, dialogs
│       ├── panel_central.rs # grid, full preview, About and Tweaks panels
│       ├── preview.rs       # DDS decoding for previews, legacy format handling
│       ├── volume.rs        # 3D texture flattening / rebuilding
│       ├── widgets.rs       # custom buttons, cards, checkerboard
│       ├── constants.rs     # colours and layout constants
│       ├── icons.rs         # icon loading
│       └── ipc.rs           # local single-instance forwarding
├── hex_pattern/             # ImHex patterns for .textures and .streamtex
├── appicon.png, icon.ico    # application icon
├── *.png                    # left rail icons
├── texembed_installer.iss   # Inno Setup script
├── build.rs                 # Windows resource (icon) embedding
└── Cargo.toml
```

Main dependencies: [`eframe` / `egui`](https://github.com/emilk/egui) for the UI, [`image_dds`](https://crates.io/crates/image_dds) and `ddsfile` for DDS decoding, [`image`](https://crates.io/crates/image) for PNG, [`rfd`](https://crates.io/crates/rfd) for native file dialogs, plus `anyhow`, `byteorder` and `chrono`.

## Contributing

Bug reports, format findings and pull requests are welcome.

- When reporting a bug, include the file name and type (`.textures` or `.streamtex`), what you did, and the **Logs** panel output.
- If a file fails to load, mention the game version it came from.
- Please run `cargo fmt` and `cargo clippy` before opening a pull request, and add tests for format-level changes where you can (the format modules already include unit tests).

## Credits

- **Ussa1n**: author and maintainer.
- **[Lahvuun](https://github.com/Lahvuun)**: released the base source for texture analysis and extraction (MIT licensed).
- **Selter**: tool testing and many suggestions for improvement and optimization.

## License

Released under the [MIT License](LICENSE). © 2026 Ussa1n.

*Split/Second and related names are trademarks of their respective owners. TEXembed is a community tool made for modding and research purposes.*
