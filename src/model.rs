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
    pub modified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileType {
    Textures,
    Streamtex,
}

#[derive(Clone, Debug)]
pub struct StreamtexRecord {
    pub(crate) length_prefix: [u8; 4],
    pub(crate) bytes: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub(crate) struct StreamtexBinding {
    pub(crate) record_index: usize,
    pub(crate) catalog_index: usize,
    pub(crate) entry_index: usize,
    pub(crate) record_id: u32,
}

#[derive(Clone, Debug)]
pub struct StreamtexLayout {
    pub(crate) records: Vec<StreamtexRecord>,
    pub(crate) trailing_bytes: Arc<[u8]>,
    pub(crate) sidecar_bindings: Vec<StreamtexBinding>,
}

#[derive(Clone, Debug)]
pub struct TextureFile {
    pub file_type: FileType,
    pub path: std::path::PathBuf,
    pub dds_list: Vec<DdsInfo>,
    pub original_bytes: Vec<u8>,
    pub streamtex_layout: Option<StreamtexLayout>,
}
