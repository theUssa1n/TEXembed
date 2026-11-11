use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use texembed::{load_streamtex_file, load_textures_file, TextureFile};

const MAX_LOAD_WORKERS: usize = 2;
const MAX_LOAD_REQUESTS: usize = 256;
const MAX_GALLERY_WARNINGS: usize = 32;

pub(super) enum LoadRequest {
    File(PathBuf),
    Folder {
        path: PathBuf,
        include_multi_texture: bool,
    },
    Shared {
        paths: Vec<PathBuf>,
        include_multi_texture: bool,
    },
}

#[derive(Debug)]
pub(super) struct GalleryBuild {
    pub(super) path: PathBuf,
    pub(super) is_folder: bool,
    pub(super) include_multi_texture: bool,
    pub(super) files: Vec<TextureFile>,
    pub(super) index_map: Vec<(usize, usize)>,
    pub(super) file_global_ranges: Vec<std::ops::Range<usize>>,
    pub(super) file_expanded: Vec<bool>,
    pub(super) warnings: Vec<String>,
}

#[derive(Debug)]
pub(super) enum LoadedContent {
    File(TextureFile),
    ExistingFile(PathBuf),
    Gallery(GalleryBuild),
}

type LoadResult = Result<LoadedContent, String>;

struct LoadJob {
    description: String,
    receiver: Option<Receiver<LoadResult>>,
    result: Option<LoadResult>,
}

#[derive(Default)]
pub(super) struct LoadingManager {
    queued: VecDeque<LoadRequest>,
    active: VecDeque<LoadJob>,
    loaded_paths: Arc<HashSet<PathBuf>>,
    retry: Option<PathBuf>,
}

impl LoadRequest {
    fn description(&self) -> String {
        match self {
            Self::File(path) => format!("Open {}", path.display()),
            Self::Folder { path, .. } => format!("Open folder {}", path.display()),
            Self::Shared { paths, .. } => format!("Open shared view of {} files", paths.len()),
        }
    }

    fn load(self, loaded_paths: &HashSet<PathBuf>) -> LoadResult {
        match self {
            Self::File(path) => {
                let path = canonical_path(&path)?;
                if loaded_paths.contains(&path) {
                    Ok(LoadedContent::ExistingFile(path))
                } else {
                    load_canonical_file(&path).map(LoadedContent::File)
                }
            }
            Self::Folder {
                path,
                include_multi_texture,
            } => load_folder(&path, include_multi_texture).map(LoadedContent::Gallery),
            Self::Shared {
                paths,
                include_multi_texture,
            } => build_gallery(paths, None, include_multi_texture, Vec::new())
                .map(LoadedContent::Gallery),
        }
    }
}

impl LoadingManager {
    pub(super) fn set_loaded_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let paths = paths.into_iter().collect();
        if *self.loaded_paths != paths {
            self.loaded_paths = Arc::new(paths);
        }
    }

    pub(super) fn enqueue(&mut self, request: LoadRequest) -> Result<(), String> {
        if self.pending_count() >= MAX_LOAD_REQUESTS {
            return Err(format!(
                "The loading queue is full ({MAX_LOAD_REQUESTS} requests). Wait for a load to finish and try again."
            ));
        }
        self.queued.push_back(request);
        Ok(())
    }

    pub(super) fn pending_count(&self) -> usize {
        self.queued.len() + self.active.len() + usize::from(self.retry.is_some())
    }

    pub(super) fn retry_file(&mut self, path: PathBuf) {
        debug_assert!(self.retry.is_none());
        debug_assert!(self.pending_count() < MAX_LOAD_REQUESTS);
        self.retry = Some(path);
    }

    pub(super) fn dispatch_jobs(&mut self, ctx: &egui::Context) {
        if self.active.len() < MAX_LOAD_WORKERS {
            if let Some(path) = self.retry.take() {
                self.active.push_front(spawn_job(
                    format!("Open {}", path.display()),
                    move || load_texture_file(&path).map(LoadedContent::File),
                    ctx,
                ));
            }
        }
        while self.active.len() < MAX_LOAD_WORKERS {
            let Some(request) = self.queued.pop_front() else {
                break;
            };
            let description = request.description();
            if let LoadRequest::File(path) = &request {
                if self.loaded_paths.contains(path) {
                    self.active.push_back(LoadJob {
                        description,
                        receiver: None,
                        result: Some(Ok(LoadedContent::ExistingFile(path.clone()))),
                    });
                    continue;
                }
            }
            let loaded_paths = Arc::clone(&self.loaded_paths);
            self.active.push_back(spawn_job(
                description,
                move || request.load(&loaded_paths),
                ctx,
            ));
        }
        if !self.active.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub(super) fn process_results(&mut self) -> Vec<LoadResult> {
        if self.retry.is_some() {
            return Vec::new();
        }
        for job in &mut self.active {
            if job.result.is_some() {
                continue;
            }
            job.result = match job.receiver.as_ref().map(Receiver::try_recv) {
                Some(Ok(result)) => Some(result),
                Some(Err(TryRecvError::Empty)) => None,
                Some(Err(TryRecvError::Disconnected)) | None => Some(Err(
                    "The loading worker disconnected before completing.".to_string(),
                )),
            };
        }

        let mut results = Vec::new();
        while self.active.front().is_some_and(|job| job.result.is_some()) {
            if let Some(mut job) = self.active.pop_front() {
                if let Some(result) = job.result.take() {
                    let existing_file = matches!(&result, Ok(LoadedContent::ExistingFile(_)));
                    results.push(result.map_err(|error| format!("{}: {error}", job.description)));
                    if existing_file {
                        break;
                    }
                }
            }
        }
        results
    }
}

fn spawn_job(
    description: String,
    operation: impl FnOnce() -> LoadResult + Send + 'static,
    ctx: &egui::Context,
) -> LoadJob {
    let (sender, receiver) = mpsc::sync_channel(1);
    let repaint_ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("texembed-loader".to_string())
        .spawn(move || {
            let result = crate::runtime::run_guarded(operation);
            let _ = sender.send(result);
            repaint_ctx.request_repaint();
        });
    let result = spawned
        .err()
        .map(|error| Err(format!("Could not start a loading worker: {error}")));
    LoadJob {
        description,
        receiver: Some(receiver),
        result,
    }
}

fn canonical_path(path: &Path) -> Result<PathBuf, String> {
    crate::runtime::canonicalize_path(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn load_texture_file(path: &Path) -> Result<TextureFile, String> {
    load_canonical_file(&canonical_path(path)?)
}

fn load_canonical_file(path: &Path) -> Result<TextureFile, String> {
    let mut source = fs::File::open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let mut signature = [0_u8; 4];
    source
        .read_exact(&mut signature)
        .map_err(|error| format!("Could not read the header of {}: {error}", path.display()))?;

    if &signature == b"TEXS" {
        return Err(format!(
            "{} is a console TEXS container. Only PC SXET .textures and DDS .streamtex containers are supported.",
            path.display()
        ));
    }
    if &signature == b"SXET" {
        return load_textures_file(path.to_path_buf())
            .map_err(|error| format!("Invalid .textures container {}: {error}", path.display()));
    }

    load_streamtex_file(path.to_path_buf())
        .map_err(|error| format!("Invalid .streamtex container {}: {error}", path.display()))
}

fn load_folder(path: &Path, include_multi_texture: bool) -> Result<GalleryBuild, String> {
    let folder = canonical_path(path)?;
    let entries = fs::read_dir(&folder)
        .map_err(|error| format!("Could not read folder {}: {error}", folder.display()))?;
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if !path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("textures")
                            || extension.eq_ignore_ascii_case("streamtex")
                    })
                {
                    continue;
                }
                match entry.file_type() {
                    Ok(file_type) if file_type.is_dir() => {}
                    Ok(_) => paths.push(path),
                    Err(error) => push_warning(
                        &mut warnings,
                        format!("Could not inspect {}: {error}", path.display()),
                    ),
                }
            }
            Err(error) => push_warning(
                &mut warnings,
                format!("Could not read a folder entry: {error}"),
            ),
        }
    }
    paths.sort();
    build_gallery(paths, Some(folder), include_multi_texture, warnings)
}

fn push_warning(warnings: &mut Vec<String>, warning: String) {
    if warnings.len() < MAX_GALLERY_WARNINGS {
        warnings.push(warning);
    } else if warnings.len() == MAX_GALLERY_WARNINGS {
        warnings.push("Additional loading errors were omitted.".to_string());
    }
}

fn build_gallery(
    paths: Vec<PathBuf>,
    folder: Option<PathBuf>,
    include_multi_texture: bool,
    mut warnings: Vec<String>,
) -> Result<GalleryBuild, String> {
    let mut files = Vec::new();
    let mut index_map = Vec::new();
    let mut file_global_ranges = Vec::new();
    let mut file_expanded = Vec::new();
    let mut seen = HashSet::new();

    for path in paths {
        let path = match canonical_path(&path) {
            Ok(path) => path,
            Err(error) => {
                push_warning(&mut warnings, error);
                continue;
            }
        };
        if !seen.insert(path.clone()) || path.is_dir() {
            continue;
        }
        let mut file = match crate::runtime::run_guarded(|| load_canonical_file(&path)) {
            Ok(file) => file,
            Err(error) => {
                push_warning(&mut warnings, format!("{}: {error}", path.display()));
                continue;
            }
        };
        if file.dds_list.is_empty() || (!include_multi_texture && file.dds_list.len() != 1) {
            continue;
        }

        let file_index = files.len();
        let texture_count = file.dds_list.len();
        let start = index_map.len();
        for (local_index, dds) in file.dds_list.iter_mut().enumerate() {
            if dds.name.trim().is_empty() {
                if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                    dds.name = if texture_count == 1 {
                        format!("{stem}.dds")
                    } else {
                        format!("{stem}_{local_index}.dds")
                    };
                }
            }
            index_map.push((file_index, local_index));
        }
        file_global_ranges.push(start..index_map.len());
        file_expanded.push(true);
        files.push(file);
    }

    if index_map.is_empty() {
        let kind = if include_multi_texture {
            ""
        } else {
            "single-texture "
        };
        let mut error = format!("No supported {kind}.textures/.streamtex files were found.");
        for warning in warnings {
            error.push('\n');
            error.push_str(&warning);
        }
        return Err(error);
    }

    Ok(GalleryBuild {
        is_folder: folder.is_some(),
        path: folder.unwrap_or_else(|| PathBuf::from(format!("Shared view ({})", files.len()))),
        include_multi_texture,
        files,
        index_map,
        file_global_ranges,
        file_expanded,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use super::*;

    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "texembed_loading_{}_{}_{}",
                std::process::id(),
                stamp,
                id
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn streamtex_bytes(records: usize) -> Vec<u8> {
        let mut dds = vec![0_u8; 136];
        dds[..4].copy_from_slice(b"DDS ");
        dds[4..8].copy_from_slice(&124_u32.to_le_bytes());
        dds[8..12].copy_from_slice(&0x0008_1007_u32.to_le_bytes());
        dds[12..16].copy_from_slice(&4_u32.to_le_bytes());
        dds[16..20].copy_from_slice(&4_u32.to_le_bytes());
        dds[20..24].copy_from_slice(&8_u32.to_le_bytes());
        dds[28..32].copy_from_slice(&1_u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32_u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4_u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        dds[108..112].copy_from_slice(&0x1000_u32.to_le_bytes());
        let mut bytes = Vec::new();
        for _ in 0..records {
            bytes.extend_from_slice(&136_u32.to_le_bytes());
            bytes.extend_from_slice(&dds);
        }
        bytes
    }

    fn collect_results(manager: &mut LoadingManager) -> Vec<LoadResult> {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut results = Vec::new();
        while manager.pending_count() > 0 {
            manager.dispatch_jobs(&ctx);
            results.extend(manager.process_results());
            assert!(Instant::now() < deadline, "loading workers did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
        results
    }

    #[test]
    fn detects_content_instead_of_relying_on_extension() {
        let folder = TempFolder::new();
        let path = folder.write(" leading texture_\u{0628}.textures", &streamtex_bytes(1));
        let file = load_texture_file(&path).unwrap();
        assert!(matches!(file.file_type, texembed::FileType::Streamtex));
        assert_eq!(file.path, crate::runtime::canonicalize_path(&path).unwrap());
        assert_eq!(file.dds_list.len(), 1);
    }

    #[test]
    fn background_file_folder_and_shared_loads_preserve_leading_opaque_records() {
        for leading in [b"opaque record".as_slice(), b""] {
            let folder = TempFolder::new();
            let mut bytes = u32::try_from(leading.len()).unwrap().to_le_bytes().to_vec();
            bytes.extend_from_slice(leading);
            bytes.extend_from_slice(&streamtex_bytes(1));
            let path = folder.write("leading.streamtex", &bytes);
            let canonical = crate::runtime::canonicalize_path(&path).unwrap();
            let mut manager = LoadingManager::default();
            manager.enqueue(LoadRequest::File(path.clone())).unwrap();
            manager
                .enqueue(LoadRequest::Folder {
                    path: folder.0.clone(),
                    include_multi_texture: false,
                })
                .unwrap();
            manager
                .enqueue(LoadRequest::Shared {
                    paths: vec![path],
                    include_multi_texture: false,
                })
                .unwrap();
            let results = collect_results(&mut manager);
            assert_eq!(results.len(), 3);
            for (index, result) in results.into_iter().enumerate() {
                let file = match result.unwrap() {
                    LoadedContent::File(file) => {
                        assert_eq!(index, 0);
                        file
                    }
                    LoadedContent::Gallery(mut gallery) => {
                        assert_eq!(gallery.is_folder, index == 1);
                        assert_eq!(gallery.files.len(), 1);
                        assert_eq!(gallery.index_map, vec![(0, 0)]);
                        assert_eq!(gallery.file_global_ranges, vec![0..1]);
                        assert!(gallery.warnings.is_empty());
                        gallery.files.remove(0)
                    }
                    LoadedContent::ExistingFile(_) => panic!("unexpected existing file"),
                };
                assert_eq!(file.path, canonical);
                assert_eq!(file.dds_list.len(), 1);
                assert_eq!(file.dds_list[0].entry_index, 1);
                assert_eq!(&*file.dds_list[0].bytes, &streamtex_bytes(1)[4..]);
                let output = folder.0.join(format!("saved_{index}.streamtex"));
                texembed::save_streamtex_file(&file, &output).unwrap();
                assert_eq!(fs::read(output).unwrap(), bytes);
            }
        }
    }

    #[test]
    fn rejects_opaque_streams_without_recognizable_dds_records() {
        let folder = TempFolder::new();
        for leading in [b"opaque record".as_slice(), b""] {
            let mut bytes = u32::try_from(leading.len()).unwrap().to_le_bytes().to_vec();
            bytes.extend_from_slice(leading);
            let path = folder.write("opaque.streamtex", &bytes);
            let error = load_texture_file(&path).unwrap_err();
            assert!(error.contains("Invalid .streamtex"));
            assert!(error.contains("No recognizable DDS records"));
        }
    }

    #[test]
    fn rejects_console_and_malformed_containers() {
        let folder = TempFolder::new();
        for (index, bytes) in [b"TEXS\0\0\0\0".as_slice(), b"garbage!", b"\0\0\0\0DDS "]
            .into_iter()
            .enumerate()
        {
            let path = folder.write(&format!("invalid_{index}.streamtex"), bytes);
            assert!(load_texture_file(&path).is_err());
        }
        let path = folder.write("console.textures", b"TEXS");
        assert!(load_texture_file(&path)
            .unwrap_err()
            .contains("console TEXS"));
        let path = folder.write("truncated.textures", b"SXET");
        assert!(load_texture_file(&path)
            .unwrap_err()
            .contains("Invalid .textures"));
    }

    #[test]
    fn accepts_valid_empty_pc_textures_without_falling_back_to_streamtex() {
        let folder = TempFolder::new();
        let mut bytes = vec![0_u8; 40];
        bytes[..4].copy_from_slice(b"SXET");
        bytes[12..16].copy_from_slice(&24_u32.to_le_bytes());
        let path = folder.write("empty.streamtex", &bytes);
        let file = load_texture_file(&path).unwrap();
        assert!(matches!(file.file_type, texembed::FileType::Textures));
        assert!(file.dds_list.is_empty());
    }

    #[test]
    fn shared_gallery_preserves_order_and_deduplicates_canonical_paths() {
        let folder = TempFolder::new();
        let a = folder.write("a.streamtex", &streamtex_bytes(1));
        let b = folder.write("b.streamtex", &streamtex_bytes(2));
        let alias = folder.0.join(".").join("b.streamtex");
        let gallery =
            build_gallery(vec![b.clone(), a.clone(), alias], None, true, Vec::new()).unwrap();
        assert_eq!(gallery.files.len(), 2);
        assert_eq!(
            gallery.files[0].path,
            crate::runtime::canonicalize_path(&b).unwrap()
        );
        assert_eq!(
            gallery.files[1].path,
            crate::runtime::canonicalize_path(&a).unwrap()
        );
        assert_eq!(gallery.index_map, vec![(0, 0), (0, 1), (1, 0)]);
        assert_eq!(gallery.file_global_ranges, vec![0..2, 2..3]);
        assert_eq!(gallery.file_expanded, vec![true, true]);
        assert!(!gallery.is_folder);
    }

    #[test]
    fn folder_gallery_sorts_filters_and_reports_skipped_failures() {
        let folder = TempFolder::new();
        let a = folder.write("a.streamtex", &streamtex_bytes(1));
        folder.write("b.streamtex", &streamtex_bytes(2));
        folder.write("bad.textures", b"TEXS");
        let c = folder.write("c.streamtex", &streamtex_bytes(1));
        folder.write("ignored.bin", b"not a texture");
        let gallery = load_folder(&folder.0, false).unwrap();
        assert!(gallery.is_folder);
        assert!(!gallery.include_multi_texture);
        assert_eq!(gallery.files.len(), 2);
        assert_eq!(
            gallery.files[0].path,
            crate::runtime::canonicalize_path(&a).unwrap()
        );
        assert_eq!(
            gallery.files[1].path,
            crate::runtime::canonicalize_path(&c).unwrap()
        );
        assert_eq!(gallery.warnings.len(), 1);
        assert!(gallery.warnings[0].contains("console TEXS"));
    }

    #[test]
    fn exact_loaded_paths_return_ready_results_without_filesystem_access() {
        let folder = TempFolder::new();
        let path = folder.0.join("not-on-disk.streamtex");
        let mut manager = LoadingManager::default();
        manager.set_loaded_paths([path.clone()]);
        manager.enqueue(LoadRequest::File(path.clone())).unwrap();
        manager.dispatch_jobs(&egui::Context::default());
        assert_eq!(manager.active.len(), 1);
        assert!(manager.active[0].receiver.is_none());
        let results = manager.process_results();
        assert!(matches!(
            &results[..],
            [Ok(LoadedContent::ExistingFile(existing))] if existing == &path
        ));
        assert_eq!(manager.pending_count(), 0);
    }

    #[test]
    fn worker_resolves_loaded_aliases_before_parsing_invalid_disk_contents() {
        let folder = TempFolder::new();
        let path = folder.write("open.streamtex", b"TEXS");
        let canonical = crate::runtime::canonicalize_path(&path).unwrap();
        let alias_dir = folder.0.join("alias");
        fs::create_dir(&alias_dir).unwrap();
        let alias = alias_dir.join("..").join("open.streamtex");
        assert_ne!(alias, canonical);
        let mut manager = LoadingManager::default();
        manager.set_loaded_paths([canonical.clone()]);
        manager.enqueue(LoadRequest::File(alias)).unwrap();
        manager.dispatch_jobs(&egui::Context::default());
        assert!(manager.active[0].receiver.is_some());
        let results = collect_results(&mut manager);
        assert!(matches!(
            &results[..],
            [Ok(LoadedContent::ExistingFile(existing))] if existing == &canonical
        ));
    }

    #[test]
    fn queued_requests_use_the_latest_loaded_path_snapshot() {
        let folder = TempFolder::new();
        let path = folder.0.join("not-on-disk.streamtex");
        let mut manager = LoadingManager::default();
        manager.enqueue(LoadRequest::File(path.clone())).unwrap();
        manager.set_loaded_paths([path.clone()]);
        let results = collect_results(&mut manager);
        assert!(matches!(
            &results[..],
            [Ok(LoadedContent::ExistingFile(existing))] if existing == &path
        ));
        manager.enqueue(LoadRequest::File(path)).unwrap();
        manager.set_loaded_paths([]);
        let results = collect_results(&mut manager);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[test]
    fn existing_file_retry_stays_ahead_of_later_ready_results() {
        let folder = TempFolder::new();
        let path = folder.write("retry.streamtex", &streamtex_bytes(1));
        let path = crate::runtime::canonicalize_path(&path).unwrap();
        let later = folder.0.join("later.streamtex");
        let mut manager = LoadingManager::default();
        manager.set_loaded_paths([path.clone(), later.clone()]);
        manager.enqueue(LoadRequest::File(path.clone())).unwrap();
        manager.enqueue(LoadRequest::File(later.clone())).unwrap();
        let ctx = egui::Context::default();
        manager.dispatch_jobs(&ctx);
        let results = manager.process_results();
        assert!(matches!(
            &results[..],
            [Ok(LoadedContent::ExistingFile(existing))] if existing == &path
        ));
        manager.retry_file(path.clone());
        assert_eq!(manager.pending_count(), 2);
        assert!(manager.process_results().is_empty());
        manager.dispatch_jobs(&ctx);
        assert_eq!(manager.active.len(), MAX_LOAD_WORKERS);
        let results = collect_results(&mut manager);
        assert!(matches!(
            &results[..],
            [Ok(LoadedContent::File(file)), Ok(LoadedContent::ExistingFile(existing))]
                if file.path == path && existing == &later
        ));
    }

    #[test]
    fn retry_reuses_the_completed_requests_queue_capacity() {
        let folder = TempFolder::new();
        let path = folder.0.join("open.streamtex");
        let mut manager = LoadingManager::default();
        manager.set_loaded_paths([path.clone()]);
        for _ in 0..MAX_LOAD_REQUESTS {
            manager.enqueue(LoadRequest::File(path.clone())).unwrap();
        }
        manager.dispatch_jobs(&egui::Context::default());
        assert_eq!(manager.process_results().len(), 1);
        manager.retry_file(path.clone());
        assert_eq!(manager.pending_count(), MAX_LOAD_REQUESTS);
        assert!(manager.enqueue(LoadRequest::File(path)).is_err());
        assert!(manager.process_results().is_empty());
    }

    #[test]
    fn completed_later_request_waits_for_the_earlier_request() {
        let (first_sender, first_receiver) = mpsc::sync_channel(1);
        let (second_sender, second_receiver) = mpsc::sync_channel(1);
        let mut manager = LoadingManager::default();
        for (description, receiver) in [("first", first_receiver), ("second", second_receiver)] {
            manager.active.push_back(LoadJob {
                description: description.to_string(),
                receiver: Some(receiver),
                result: None,
            });
        }
        second_sender
            .send(Err("second result".to_string()))
            .unwrap();
        assert!(manager.process_results().is_empty());
        assert_eq!(manager.pending_count(), 2);
        first_sender.send(Err("first result".to_string())).unwrap();
        let errors: Vec<_> = manager
            .process_results()
            .into_iter()
            .map(Result::unwrap_err)
            .collect();
        assert_eq!(errors, ["first: first result", "second: second result"]);
        assert_eq!(manager.pending_count(), 0);
    }

    #[test]
    fn disconnected_worker_reports_failure_and_releases_capacity() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let mut manager = LoadingManager::default();
        manager.active.push_back(LoadJob {
            description: "broken worker".to_string(),
            receiver: Some(receiver),
            result: None,
        });
        let results = manager.process_results();
        assert_eq!(results.len(), 1);
        assert!(results[0].as_ref().unwrap_err().contains("disconnected"));
        assert_eq!(manager.pending_count(), 0);
    }

    #[test]
    fn panic_and_read_failure_do_not_wedge_later_loads() {
        let folder = TempFolder::new();
        let valid = folder.write("valid.streamtex", &streamtex_bytes(1));
        let mut manager = LoadingManager::default();
        manager.active.push_back(spawn_job(
            "panic test".to_string(),
            || panic!("injected loading panic"),
            &egui::Context::default(),
        ));
        manager
            .enqueue(LoadRequest::File(folder.0.join("missing.streamtex")))
            .unwrap();
        manager.enqueue(LoadRequest::File(valid)).unwrap();
        let results = collect_results(&mut manager);
        assert_eq!(results.len(), 3);
        assert!(results[0].as_ref().unwrap_err().contains("panic"));
        assert!(results[1]
            .as_ref()
            .unwrap_err()
            .contains("missing.streamtex"));
        assert!(matches!(results[2], Ok(LoadedContent::File(_))));
    }

    #[test]
    fn bounds_workers_and_rejects_new_requests_without_dropping_queued_work() {
        let mut manager = LoadingManager::default();
        let folder = TempFolder::new();
        for index in 0..MAX_LOAD_REQUESTS {
            manager
                .enqueue(LoadRequest::File(folder.0.join(index.to_string())))
                .unwrap();
        }
        assert!(manager
            .enqueue(LoadRequest::File(folder.0.clone()))
            .is_err());
        let ctx = egui::Context::default();
        manager.dispatch_jobs(&ctx);
        manager.dispatch_jobs(&ctx);
        assert_eq!(manager.active.len(), MAX_LOAD_WORKERS);
        assert_eq!(manager.pending_count(), MAX_LOAD_REQUESTS);
        assert_eq!(manager.queued.len(), MAX_LOAD_REQUESTS - MAX_LOAD_WORKERS);
    }
}
