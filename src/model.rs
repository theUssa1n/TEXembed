use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct DdsInfo {
    pub name: String,
    pub bytes: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub mipmap_count: u32,
    pub pixel_format: u32,
    pub catalog_index: usize,
    pub entry_index: usize,
}

#[derive(Clone, Debug)]
pub enum FileType {
    Textures,
    Streamtex,
}

#[derive(Clone, Debug)]
pub struct TextureFile {
    pub file_type: FileType,
    pub path: std::path::PathBuf,
    pub dds_list: Vec<DdsInfo>,
    pub original_bytes: Vec<u8>,
}
