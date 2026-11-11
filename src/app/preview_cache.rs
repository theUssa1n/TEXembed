use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{mpsc, Arc};

use eframe::egui;

use super::preview::{
    decode_dds_full_preview, decode_dds_preview_image, preview_rgba_len, resize_thumbnail,
    upload_decoded_preview, DecodedPreview, THUMBNAIL_MAX_SIDE,
};

const THUMBNAIL_CACHE_BYTES: usize = 32 * 1024 * 1024;
const THUMBNAIL_CACHE_ENTRIES: usize = 1024;
const MAX_QUEUED_JOBS: usize = 256;
const MAX_RESULTS_PER_FRAME: usize = 4;

type PreviewKey = (usize, usize);

pub(super) enum PreviewState {
    Ready(egui::TextureHandle),
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PreviewKind {
    Thumbnail,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct JobKey {
    entry: PreviewKey,
    kind: PreviewKind,
}

struct DecodeJob {
    key: JobKey,
    generation: u64,
    bytes: Arc<[u8]>,
}

struct DecodeResult {
    key: JobKey,
    generation: u64,
    decoded: Result<DecodedPreview, String>,
}

struct CacheEntry {
    state: PreviewState,
    bytes: usize,
    source_side: usize,
    last_used: Cell<u64>,
}

struct ThumbnailCache {
    entries: HashMap<PreviewKey, CacheEntry>,
    bytes: usize,
    budget: usize,
    capacity: usize,
    clock: Cell<u64>,
    visible: Option<HashMap<PreviewKey, usize>>,
}

impl ThumbnailCache {
    fn new(budget: usize, capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            budget,
            capacity,
            clock: Cell::new(0),
            visible: None,
        }
    }

    fn set_visible(
        &mut self,
        keys: impl IntoIterator<Item = PreviewKey>,
        selected: Option<PreviewKey>,
    ) {
        let capacity = self.capacity.min(self.budget / 4);
        let mut visible = HashMap::new();
        if let Some(key) = selected.filter(|_| capacity > 0) {
            visible.insert(key, 0);
        }
        for key in keys {
            if visible.len() >= capacity {
                break;
            }
            visible.insert(key, 0);
        }
        let selected = selected.filter(|key| visible.contains_key(key));
        let others = visible.len() - usize::from(selected.is_some());
        let selected_side = if selected.is_some() {
            thumbnail_side_for_budget(self.budget - others * 4)
        } else {
            0
        };
        let available = self.budget - selected_side * selected_side * 4;
        let grid_side = thumbnail_side_for_budget(available / others.max(1));
        for (key, side) in &mut visible {
            *side = if Some(*key) == selected {
                selected_side
            } else {
                grid_side
            };
        }
        if self.visible.as_ref() == Some(&visible) {
            return;
        }
        self.entries.retain(|key, entry| {
            let keep = visible.get(key).is_none_or(|side| match &entry.state {
                PreviewState::Ready(texture) => {
                    let size = texture.size();
                    size[0].max(size[1]) == entry.source_side.min(*side)
                }
                PreviewState::Unsupported => true,
            });
            if !keep {
                self.bytes -= entry.bytes;
            }
            keep
        });
        self.visible = Some(visible);
    }

    fn requested_side(&self, key: &PreviewKey) -> Option<usize> {
        match &self.visible {
            Some(visible) => visible.get(key).copied(),
            None => Some(THUMBNAIL_MAX_SIDE),
        }
    }

    fn tick(&self) -> u64 {
        let tick = self.clock.get().saturating_add(1);
        self.clock.set(tick);
        tick
    }

    fn get(&self, key: &PreviewKey) -> Option<&PreviewState> {
        self.entries.get(key).map(|entry| {
            entry.last_used.set(self.tick());
            &entry.state
        })
    }

    fn remove(&mut self, key: &PreviewKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes -= entry.bytes;
        }
    }

    fn clear_tab(&mut self, tab_id: usize) {
        if let Some(visible) = &mut self.visible {
            visible.retain(|(id, _), _| *id != tab_id);
        }
        self.entries.retain(|(id, _), entry| {
            if *id == tab_id {
                self.bytes -= entry.bytes;
                false
            } else {
                true
            }
        });
    }

    fn reserve(&mut self, bytes: usize) -> Result<(), String> {
        if bytes > self.budget || self.capacity == 0 {
            return Err("Thumbnail exceeds the GPU cache budget".to_string());
        }
        while self.bytes > self.budget - bytes || self.entries.len() >= self.capacity {
            let Some(key) = self
                .entries
                .iter()
                .filter(|(key, _)| {
                    !self
                        .visible
                        .as_ref()
                        .is_some_and(|visible| visible.contains_key(key))
                })
                .min_by_key(|(key, entry)| (entry.last_used.get(), **key))
                .map(|(key, _)| *key)
            else {
                return Err("Visible thumbnails exhaust the GPU cache budget".to_string());
            };
            self.remove(&key);
        }
        Ok(())
    }

    fn insert(
        &mut self,
        key: PreviewKey,
        texture: egui::TextureHandle,
        bytes: usize,
        source_side: usize,
    ) {
        self.remove(&key);
        let last_used = Cell::new(self.tick());
        self.entries.insert(
            key,
            CacheEntry {
                state: PreviewState::Ready(texture),
                bytes,
                source_side,
                last_used,
            },
        );
        self.bytes += bytes;
    }
}

fn thumbnail_side_for_budget(bytes: usize) -> usize {
    if bytes < 4 {
        return 0;
    }
    let mut side = THUMBNAIL_MAX_SIDE;
    while side * side * 4 > bytes {
        side /= 2;
    }
    side
}

pub(super) struct PreviewManager {
    thumbnails: ThumbnailCache,
    full: Option<(PreviewKey, PreviewState)>,
    active_full: Option<PreviewKey>,
    unsupported: PreviewState,
    failures: HashSet<JobKey>,
    queue: VecDeque<DecodeJob>,
    current: HashMap<JobKey, u64>,
    in_flight: HashMap<u64, JobKey>,
    next_generation: u64,
    max_jobs: usize,
    max_queued: usize,
    sender: mpsc::SyncSender<DecodeResult>,
    receiver: mpsc::Receiver<DecodeResult>,
    launch_failures: VecDeque<DecodeResult>,
    repaint: Option<egui::Context>,
}

impl Default for PreviewManager {
    fn default() -> Self {
        let max_jobs = std::thread::available_parallelism().map_or(2, |parallelism| {
            parallelism.get().saturating_sub(1).clamp(1, 2)
        });
        Self::with_limits(
            THUMBNAIL_CACHE_BYTES,
            THUMBNAIL_CACHE_ENTRIES,
            max_jobs,
            MAX_QUEUED_JOBS,
        )
    }
}

impl PreviewManager {
    fn with_limits(budget: usize, capacity: usize, max_jobs: usize, max_queued: usize) -> Self {
        let max_jobs = max_jobs.max(1);
        let (sender, receiver) = mpsc::sync_channel(max_jobs);
        Self {
            thumbnails: ThumbnailCache::new(budget, capacity),
            full: None,
            active_full: None,
            unsupported: PreviewState::Unsupported,
            failures: HashSet::new(),
            queue: VecDeque::new(),
            current: HashMap::new(),
            in_flight: HashMap::new(),
            next_generation: 0,
            max_jobs,
            max_queued: max_queued.max(1),
            sender,
            receiver,
            launch_failures: VecDeque::new(),
            repaint: None,
        }
    }

    pub(super) fn get(&self, key: &PreviewKey) -> Option<&PreviewState> {
        self.thumbnails.get(key).or_else(|| {
            self.failures
                .contains(&JobKey {
                    entry: *key,
                    kind: PreviewKind::Thumbnail,
                })
                .then_some(&self.unsupported)
        })
    }

    pub(super) fn set_visible_thumbnails(
        &mut self,
        keys: impl IntoIterator<Item = PreviewKey>,
        selected: Option<PreviewKey>,
    ) {
        self.thumbnails.set_visible(keys, selected);
        let visible = self.thumbnails.visible.as_ref().unwrap();
        self.current
            .retain(|key, _| key.kind == PreviewKind::Full || visible.contains_key(&key.entry));
        self.queue.retain(|job| {
            job.key.kind == PreviewKind::Full || visible.contains_key(&job.key.entry)
        });
    }

    pub(super) fn get_full(&self, key: &PreviewKey) -> Option<&PreviewState> {
        if self.active_full != Some(*key) {
            return None;
        }
        self.full
            .as_ref()
            .filter(|(entry, _)| entry == key)
            .map(|(_, state)| state)
            .or_else(|| {
                self.failures
                    .contains(&JobKey {
                        entry: *key,
                        kind: PreviewKind::Full,
                    })
                    .then_some(&self.unsupported)
            })
    }

    pub(super) fn request_thumbnail(&mut self, tab_id: usize, index: usize, bytes: Arc<[u8]>) {
        let entry = (tab_id, index);
        if self.thumbnails.requested_side(&entry).is_some() && self.get(&entry).is_none() {
            self.request(
                JobKey {
                    entry,
                    kind: PreviewKind::Thumbnail,
                },
                bytes,
            );
        }
    }

    pub(super) fn request_full(&mut self, tab_id: usize, index: usize, bytes: Arc<[u8]>) {
        let entry = (tab_id, index);
        self.retain_full(Some(entry));
        if self.get_full(&entry).is_none() {
            self.request(
                JobKey {
                    entry,
                    kind: PreviewKind::Full,
                },
                bytes,
            );
        }
    }

    fn request(&mut self, key: JobKey, bytes: Arc<[u8]>) {
        if self.current.contains_key(&key) {
            return;
        }
        if self.queue.len() >= self.max_queued {
            if key.kind == PreviewKind::Thumbnail {
                return;
            }
            let Some(position) = self
                .queue
                .iter()
                .position(|job| job.key.kind == PreviewKind::Thumbnail)
            else {
                return;
            };
            if let Some(discarded) = self.queue.remove(position) {
                self.current.remove(&discarded.key);
            }
        }

        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
        self.current.insert(key, generation);
        let job = DecodeJob {
            key,
            generation,
            bytes,
        };
        if key.kind == PreviewKind::Full {
            self.queue.push_front(job);
        } else {
            self.queue.push_back(job);
        }
        if let Some(ctx) = &self.repaint {
            ctx.request_repaint();
        }
    }

    pub(super) fn invalidate(&mut self, tab_id: usize, index: usize) {
        let entry = (tab_id, index);
        self.thumbnails.remove(&entry);
        if self.full.as_ref().is_some_and(|(key, _)| *key == entry) {
            self.full = None;
        }
        self.failures.retain(|key| key.entry != entry);
        self.current.retain(|key, _| key.entry != entry);
        self.queue.retain(|job| job.key.entry != entry);
    }

    pub(super) fn clear_tab(&mut self, tab_id: usize) {
        self.thumbnails.clear_tab(tab_id);
        if self.active_full.is_some_and(|(id, _)| id == tab_id) {
            self.retain_full(None);
        }
        self.failures.retain(|key| key.entry.0 != tab_id);
        self.current.retain(|key, _| key.entry.0 != tab_id);
        self.queue.retain(|job| job.key.entry.0 != tab_id);
    }

    pub(super) fn retain_full(&mut self, key: Option<PreviewKey>) {
        if self.active_full == key {
            return;
        }
        self.active_full = key;
        self.full = None;
        self.current
            .retain(|job, _| job.kind == PreviewKind::Thumbnail || Some(job.entry) == key);
        self.queue
            .retain(|job| job.key.kind == PreviewKind::Thumbnail || Some(job.key.entry) == key);
    }

    fn next_job(&mut self) -> Option<DecodeJob> {
        if self.in_flight.len() >= self.max_jobs {
            return None;
        }
        while let Some(job) = self.queue.pop_front() {
            if self.current.get(&job.key) == Some(&job.generation) {
                self.in_flight.insert(job.generation, job.key);
                return Some(job);
            }
        }
        None
    }

    pub(super) fn dispatch_jobs(&mut self, ctx: &egui::Context) {
        self.repaint = Some(ctx.clone());
        while let Some(job) = self.next_job() {
            let key = job.key;
            let generation = job.generation;
            let sender = self.sender.clone();
            let repaint = ctx.clone();
            let spawn = std::thread::Builder::new()
                .name(format!("preview-{generation}"))
                .spawn(move || {
                    let result = complete_job(job, decode_job_image);
                    let _ = sender.send(result);
                    repaint.request_repaint();
                });
            if let Err(err) = spawn {
                self.launch_failures.push_back(DecodeResult {
                    key,
                    generation,
                    decoded: Err(format!("Cannot start preview worker: {err}")),
                });
                ctx.request_repaint();
            }
        }
    }

    pub(super) fn process_results(&mut self, ctx: &egui::Context) -> Vec<String> {
        let mut errors = Vec::new();
        for _ in 0..MAX_RESULTS_PER_FRAME {
            let result = self
                .launch_failures
                .pop_front()
                .or_else(|| self.receiver.try_recv().ok());
            let Some(result) = result else {
                break;
            };
            if self.in_flight.remove(&result.generation) != Some(result.key) {
                continue;
            }
            if self.current.get(&result.key) != Some(&result.generation) {
                continue;
            }
            self.current.remove(&result.key);

            let applied = result.decoded.and_then(|decoded| {
                crate::runtime::run_guarded(|| {
                    self.apply_decoded(ctx, result.key, result.generation, decoded)
                })
            });
            if let Err(err) = applied {
                self.failures.insert(result.key);
                errors.push(format!(
                    "{:?} preview for tab {}, texture {} failed: {err}",
                    result.key.kind, result.key.entry.0, result.key.entry.1
                ));
            }
            ctx.request_repaint();
        }
        if !self.queue.is_empty() && self.in_flight.len() < self.max_jobs {
            ctx.request_repaint();
        }
        errors
    }

    fn apply_decoded(
        &mut self,
        ctx: &egui::Context,
        key: JobKey,
        generation: u64,
        (mut size, mut pixels): DecodedPreview,
    ) -> Result<(), String> {
        let mut bytes = preview_rgba_len(size)?;
        let source_side = size[0].max(size[1]);
        if key.kind == PreviewKind::Thumbnail {
            if size[0] > THUMBNAIL_MAX_SIDE || size[1] > THUMBNAIL_MAX_SIDE {
                return Err("Thumbnail exceeds the 256 pixel upload limit".to_string());
            }
            let Some(max_side) = self.thumbnails.requested_side(&key.entry) else {
                return Ok(());
            };
            (size, pixels) = resize_thumbnail((size, pixels), max_side)?;
            bytes = preview_rgba_len(size)?;
            self.thumbnails.remove(&key.entry);
            self.thumbnails.reserve(bytes)?;
        }
        let name = format!(
            "preview::{:?}::{}::{}::{generation}",
            key.kind, key.entry.0, key.entry.1
        );
        let texture = upload_decoded_preview(ctx, &name, size, &pixels)?;
        match key.kind {
            PreviewKind::Thumbnail => {
                self.thumbnails
                    .insert(key.entry, texture, bytes, source_side);
            }
            PreviewKind::Full => self.full = Some((key.entry, PreviewState::Ready(texture))),
        }
        Ok(())
    }
}

fn decode_job_image(bytes: &[u8], kind: PreviewKind) -> Result<DecodedPreview, String> {
    match kind {
        PreviewKind::Thumbnail => decode_dds_preview_image(bytes),
        PreviewKind::Full => decode_dds_full_preview(bytes),
    }
}

fn complete_job(
    job: DecodeJob,
    operation: impl FnOnce(&[u8], PreviewKind) -> Result<DecodedPreview, String>,
) -> DecodeResult {
    DecodeResult {
        key: job.key,
        generation: job.generation,
        decoded: crate::runtime::run_guarded(|| operation(&job.bytes, job.key.kind)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Arc<[u8]> {
        Arc::from(&b"synthetic DDS"[..])
    }

    fn finish(manager: &PreviewManager, job: DecodeJob, size: [usize; 2]) {
        let result = complete_job(job, |_, _| Ok((size, vec![255; size[0] * size[1] * 4])));
        assert!(manager.sender.try_send(result).is_ok());
    }

    fn load_thumbnail(
        manager: &mut PreviewManager,
        ctx: &egui::Context,
        index: usize,
        size: [usize; 2],
    ) {
        manager.request_thumbnail(1, index, source());
        let job = manager.next_job().unwrap();
        finish(manager, job, size);
        assert!(manager.process_results(ctx).is_empty());
    }

    fn visible_frame(
        manager: &mut PreviewManager,
        ctx: &egui::Context,
        visible: &[PreviewKey],
        selected: Option<PreviewKey>,
        size: [usize; 2],
    ) -> Vec<PreviewKey> {
        manager.set_visible_thumbnails(visible.iter().copied(), selected);
        for (tab, index) in selected.into_iter().chain(visible.iter().copied()) {
            manager.request_thumbnail(tab, index, source());
        }
        let mut started = Vec::new();
        while let Some(job) = manager.next_job() {
            assert_eq!(job.key.kind, PreviewKind::Thumbnail);
            started.push(job.key.entry);
            finish(manager, job, size);
        }
        assert!(manager.process_results(ctx).is_empty());
        assert!(manager.thumbnails.bytes <= manager.thumbnails.budget);
        assert!(manager.thumbnails.entries.len() <= manager.thumbnails.capacity);
        let _ = ctx.tex_manager().write().take_delta();
        started
    }

    #[test]
    fn stationary_129_square_thumbnails_converge_with_a_sharp_offscreen_inspector() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::default();
        let visible: Vec<_> = (0..129).map(|index| (1, index)).collect();
        let selected = (1, 1000);
        let mut decodes = HashMap::new();
        for _ in 0..130 {
            for key in visible_frame(&mut manager, &ctx, &visible, Some(selected), [256, 256]) {
                *decodes.entry(key).or_insert(0) += 1;
            }
        }
        assert_eq!(decodes.len(), 130);
        assert!(decodes.values().all(|count| *count == 1));
        assert_eq!(manager.thumbnails.entries.len(), 130);
        for key in &visible {
            let Some(PreviewState::Ready(texture)) = manager.get(key) else {
                panic!("Missing visible thumbnail");
            };
            assert_eq!(texture.size(), [128, 128]);
        }
        let Some(PreviewState::Ready(texture)) = manager.get(&selected) else {
            panic!("Missing inspector thumbnail");
        };
        assert_eq!(texture.size(), [256, 256]);
        assert_eq!(manager.thumbnails.bytes, (129 * 128 * 128 + 256 * 256) * 4);
        for _ in 0..8 {
            assert!(
                visible_frame(&mut manager, &ctx, &visible, Some(selected), [256, 256]).is_empty()
            );
        }
        assert!(manager.queue.is_empty());
        assert!(manager.current.is_empty());
    }

    #[test]
    fn visible_work_larger_than_the_queue_converges_without_request_starvation() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(4096, 512, 2, 16);
        let visible: Vec<_> = (0..300).map(|index| (1, index)).collect();
        let mut started = Vec::new();
        for _ in 0..160 {
            started.extend(visible_frame(&mut manager, &ctx, &visible, None, [2, 2]));
            assert!(manager.queue.len() <= 16);
        }
        assert_eq!(started, visible);
        assert_eq!(manager.thumbnails.entries.len(), 300);
        assert_eq!(manager.thumbnails.bytes, 300 * 4);
        assert!(visible_frame(&mut manager, &ctx, &visible, None, [2, 2]).is_empty());
    }

    #[test]
    fn capacity_overflow_uses_stable_admission_and_prioritizes_the_inspector() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(512, 3, 2, 4);
        let visible: Vec<_> = (0..200).map(|index| (1, index)).collect();
        let selected = Some((1, 199));
        let mut started = Vec::new();
        for _ in 0..12 {
            started.extend(visible_frame(
                &mut manager,
                &ctx,
                &visible,
                selected,
                [8, 8],
            ));
        }
        assert_eq!(started, [(1, 199), (1, 0), (1, 1)]);
        assert_eq!(manager.thumbnails.visible.as_ref().unwrap().len(), 3);
        assert_eq!(manager.thumbnails.bytes, 384);
        assert!(manager.failures.is_empty());
        assert!(manager.get(&(1, 2)).is_none());
        assert!(manager.queue.is_empty());

        manager.set_visible_thumbnails([(1, 2)], Some((1, 199)));
        load_thumbnail(&mut manager, &ctx, 2, [8, 8]);
        assert!(manager.get(&(1, 2)).is_some());
        assert!(manager.get(&(1, 199)).is_some());
    }

    #[test]
    fn resizing_the_working_set_reloads_changed_sizes_once() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(512, 8, 2, 4);
        let selected = Some((1, 0));
        assert_eq!(
            visible_frame(&mut manager, &ctx, &[(1, 0), (1, 1)], selected, [8, 8]),
            [(1, 0), (1, 1)]
        );
        let visible = [(1, 0), (1, 1), (1, 2)];
        assert_eq!(
            visible_frame(&mut manager, &ctx, &visible, selected, [8, 8]),
            [(1, 1), (1, 2)]
        );
        assert!(visible_frame(&mut manager, &ctx, &visible, selected, [8, 8]).is_empty());
        let Some(PreviewState::Ready(texture)) = manager.get(&(1, 1)) else {
            panic!("Missing resized thumbnail");
        };
        assert_eq!(texture.size(), [4, 4]);
        assert_eq!(
            visible_frame(&mut manager, &ctx, &visible, Some((1, 1)), [8, 8]),
            [(1, 1), (1, 0)]
        );
        assert!(visible_frame(&mut manager, &ctx, &visible, Some((1, 1)), [8, 8]).is_empty());
        let Some(PreviewState::Ready(texture)) = manager.get(&(1, 1)) else {
            panic!("Missing upgraded inspector thumbnail");
        };
        assert_eq!(texture.size(), [8, 8]);
    }

    #[test]
    fn visible_residency_evicts_offscreen_entries_instead_of_visible_ones() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(32, 4, 1, 4);
        load_thumbnail(&mut manager, &ctx, 0, [2, 2]);
        load_thumbnail(&mut manager, &ctx, 1, [2, 2]);
        manager.set_visible_thumbnails([(1, 0), (1, 2)], None);
        assert!(manager.get(&(1, 1)).is_some());
        load_thumbnail(&mut manager, &ctx, 2, [2, 2]);
        assert!(manager.get(&(1, 0)).is_some());
        assert!(manager.get(&(1, 1)).is_none());
        assert!(manager.get(&(1, 2)).is_some());
        assert_eq!(manager.thumbnails.bytes, 32);
    }

    #[test]
    fn leaving_the_working_set_discards_stale_thumbnails_without_affecting_full_preview() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(1024, 8, 2, 4);
        manager.set_visible_thumbnails([(1, 0), (1, 1)], None);
        manager.request_thumbnail(1, 0, source());
        let old_job = manager.next_job().unwrap();
        manager.request_thumbnail(1, 1, source());
        manager.request_full(1, 0, source());
        manager.set_visible_thumbnails([(1, 2)], None);
        assert_eq!(manager.queue.len(), 1);
        let full_job = manager.next_job().unwrap();
        assert_eq!(full_job.key.kind, PreviewKind::Full);
        finish(&manager, old_job, [2, 2]);
        finish(&manager, full_job, [8, 8]);
        assert!(manager.process_results(&ctx).is_empty());
        assert!(manager.get(&(1, 0)).is_none());
        assert!(matches!(
            manager.get_full(&(1, 0)),
            Some(PreviewState::Ready(_))
        ));
        load_thumbnail(&mut manager, &ctx, 2, [2, 2]);
        manager.set_visible_thumbnails([], None);
        assert!(matches!(
            manager.get_full(&(1, 0)),
            Some(PreviewState::Ready(_))
        ));
        manager.clear_tab(1);
        assert!(manager.thumbnails.visible.as_ref().unwrap().is_empty());
    }

    #[test]
    fn demand_policy_fits_tiny_budgets_and_unbounded_input_ranges() {
        for budget in [0, 3, 4, 7, 64, 1024] {
            for capacity in [0, 1, 2, 16] {
                let mut cache = ThumbnailCache::new(budget, capacity);
                cache.set_visible((0..usize::MAX).map(|index| (1, index)), Some((2, 0)));
                let visible = cache.visible.as_ref().unwrap();
                let bytes: usize = visible.values().map(|side| side * side * 4).sum();
                assert!(bytes <= budget);
                assert_eq!(visible.len(), capacity.min(budget / 4));
                assert!(visible.values().all(|side| *side > 0));
            }
        }
    }

    #[test]
    fn cache_downsizing_keeps_the_unmultiplied_upload_contract() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(4, 1, 1, 1);
        manager.set_visible_thumbnails([(1, 0)], None);
        manager.request_thumbnail(1, 0, source());
        let job = manager.next_job().unwrap();
        let result = complete_job(job, |_, _| Ok(([2, 1], vec![255, 0, 0, 255, 0, 0, 255, 0])));
        assert!(manager.sender.try_send(result).is_ok());
        assert!(manager.process_results(&ctx).is_empty());
        let delta = ctx.tex_manager().write().take_delta();
        let egui::ImageData::Color(image) = &delta.set.last().unwrap().1.image else {
            panic!("Missing thumbnail upload");
        };
        assert_eq!(image.size, [1, 1]);
        assert_eq!(
            image.pixels,
            [egui::Color32::from_rgba_unmultiplied(255, 0, 0, 128)]
        );
    }

    #[test]
    fn full_preview_jobs_keep_the_explicit_decode_budget() {
        use image_dds::ddsfile::{D3DFormat, Dds, Header};

        let dds = Dds {
            header: Header::new_d3d(8192, 8192, None, D3DFormat::A8R8G8B8, None, None).unwrap(),
            header10: None,
            data: Vec::new(),
        };
        let mut bytes = Vec::new();
        dds.write(&mut bytes).unwrap();
        let mut manager = PreviewManager::default();
        manager.request_full(1, 0, Arc::from(bytes));
        let result = complete_job(manager.next_job().unwrap(), decode_job_image);
        assert!(result.decoded.as_ref().unwrap_err().contains("64 MiB"));
        assert!(manager.sender.try_send(result).is_ok());
        assert_eq!(manager.process_results(&egui::Context::default()).len(), 1);
        assert!(matches!(
            manager.get_full(&(1, 0)),
            Some(PreviewState::Unsupported)
        ));
    }

    #[test]
    fn thumbnail_and_full_resolution_caches_are_separate() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::default();
        load_thumbnail(&mut manager, &ctx, 0, [256, 128]);
        assert!(manager.get_full(&(1, 0)).is_none());

        manager.request_full(1, 0, source());
        let job = manager.next_job().unwrap();
        finish(&manager, job, [512, 256]);
        assert!(manager.process_results(&ctx).is_empty());

        let Some(PreviewState::Ready(thumbnail)) = manager.get(&(1, 0)) else {
            panic!("Missing thumbnail");
        };
        assert_eq!(thumbnail.size(), [256, 128]);
        let Some(PreviewState::Ready(full)) = manager.get_full(&(1, 0)) else {
            panic!("Missing full preview");
        };
        assert_eq!(full.size(), [512, 256]);

        manager.retain_full(None);
        assert!(manager.get_full(&(1, 0)).is_none());
        assert!(manager.get(&(1, 0)).is_some());
    }

    #[test]
    fn thumbnail_budget_evicts_least_recently_used() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(32, 10, 1, 4);
        load_thumbnail(&mut manager, &ctx, 0, [2, 2]);
        load_thumbnail(&mut manager, &ctx, 1, [2, 2]);
        assert!(manager.get(&(1, 0)).is_some());
        load_thumbnail(&mut manager, &ctx, 2, [2, 2]);
        assert!(manager.get(&(1, 0)).is_some());
        assert!(manager.get(&(1, 1)).is_none());
        assert!(manager.get(&(1, 2)).is_some());
        assert_eq!(manager.thumbnails.bytes, 32);

        manager.clear_tab(1);
        assert_eq!(manager.thumbnails.bytes, 0);
        assert!(manager.thumbnails.entries.is_empty());
    }

    #[test]
    fn tiny_thumbnails_have_a_metadata_capacity() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(1024, 2, 1, 4);
        for index in 0..6 {
            load_thumbnail(&mut manager, &ctx, index, [1, 1]);
        }
        assert_eq!(manager.thumbnails.entries.len(), 2);
        assert_eq!(manager.thumbnails.bytes, 8);
    }

    #[test]
    fn queue_and_duplicate_requests_are_bounded() {
        let mut manager = PreviewManager::with_limits(1024, 8, 1, 4);
        for _ in 0..100 {
            manager.request_thumbnail(1, 0, source());
        }
        assert_eq!(manager.queue.len(), 1);
        for index in 1..100 {
            manager.request_thumbnail(1, index, source());
        }
        assert_eq!(manager.queue.len(), 4);
        assert_eq!(manager.current.len(), 4);

        manager.request_full(1, 200, source());
        for index in 200..300 {
            manager.request_thumbnail(1, index, source());
        }
        assert_eq!(manager.queue.len(), 4);
        assert_eq!(manager.next_job().unwrap().key.kind, PreviewKind::Full);
    }

    #[test]
    fn invalidated_jobs_keep_capacity_until_their_stale_results_arrive() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(1024, 8, 1, 4);
        manager.request_thumbnail(1, 0, source());
        let old_job = manager.next_job().unwrap();
        manager.invalidate(1, 0);
        manager.request_thumbnail(1, 0, source());

        assert_eq!(manager.in_flight.len(), 1);
        assert!(manager.next_job().is_none());
        finish(&manager, old_job, [1, 1]);
        assert_eq!(manager.in_flight.len(), 1);
        assert!(manager.process_results(&ctx).is_empty());
        assert!(manager.get(&(1, 0)).is_none());

        let new_job = manager.next_job().unwrap();
        finish(&manager, new_job, [2, 2]);
        assert!(manager.process_results(&ctx).is_empty());
        assert_eq!(manager.thumbnails.bytes, 16);
    }

    #[test]
    fn clearing_and_reusing_a_tab_rejects_old_generations() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(1024, 8, 1, 4);
        manager.request_thumbnail(1, 0, source());
        let old_job = manager.next_job().unwrap();
        manager.clear_tab(1);
        manager.request_thumbnail(1, 0, source());
        assert!(manager.next_job().is_none());

        finish(&manager, old_job, [1, 1]);
        assert!(manager.process_results(&ctx).is_empty());
        assert!(manager.get(&(1, 0)).is_none());
        assert!(manager.next_job().is_some());
    }

    #[test]
    fn stale_full_preview_cannot_replace_a_new_selection() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(1024, 8, 2, 4);
        manager.request_full(1, 0, source());
        let old_job = manager.next_job().unwrap();
        manager.request_full(1, 1, source());
        let new_job = manager.next_job().unwrap();
        finish(&manager, new_job, [4, 4]);
        finish(&manager, old_job, [8, 8]);

        assert!(manager.process_results(&ctx).is_empty());
        assert!(manager.get_full(&(1, 0)).is_none());
        let Some(PreviewState::Ready(texture)) = manager.get_full(&(1, 1)) else {
            panic!("Missing current full preview");
        };
        assert_eq!(texture.size(), [4, 4]);
        assert!(manager.in_flight.is_empty());
    }

    #[test]
    fn leaving_and_reentering_full_preview_rejects_previous_work() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(1024, 8, 1, 4);
        manager.request_full(1, 0, source());
        let old_job = manager.next_job().unwrap();
        manager.retain_full(None);
        manager.request_full(1, 0, source());
        assert!(manager.next_job().is_none());
        finish(&manager, old_job, [4, 4]);
        assert!(manager.process_results(&ctx).is_empty());
        assert!(manager.get_full(&(1, 0)).is_none());
        assert!(manager.next_job().is_some());
    }

    #[test]
    fn unsupported_previews_are_not_retried_until_invalidated() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::default();
        manager.request_thumbnail(1, 0, source());
        let job = manager.next_job().unwrap();
        let result = complete_job(job, |_, _| Err("Unsupported fixture".to_string()));
        assert!(manager.sender.try_send(result).is_ok());
        assert_eq!(manager.process_results(&ctx).len(), 1);
        for _ in 0..100 {
            manager.request_thumbnail(1, 0, source());
        }
        assert!(manager.queue.is_empty());
        assert!(matches!(
            manager.get(&(1, 0)),
            Some(PreviewState::Unsupported)
        ));

        manager.invalidate(1, 0);
        manager.request_thumbnail(1, 0, source());
        assert_eq!(manager.queue.len(), 1);
    }

    #[test]
    fn panic_completion_releases_worker_capacity_and_reports_error() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(1024, 8, 1, 4);
        manager.request_thumbnail(1, 0, source());
        let job = manager.next_job().unwrap();
        let result = complete_job(job, |_, _| panic!("injected preview panic"));
        assert!(result.decoded.is_err());
        assert!(manager.sender.try_send(result).is_ok());
        let errors = manager.process_results(&ctx);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("injected preview panic"));
        assert!(manager.in_flight.is_empty());

        manager.request_thumbnail(1, 1, source());
        assert!(manager.next_job().is_some());
    }

    #[test]
    fn oversized_thumbnail_upload_is_rejected_and_memoized() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::default();
        manager.request_thumbnail(1, 0, source());
        let job = manager.next_job().unwrap();
        finish(&manager, job, [THUMBNAIL_MAX_SIDE + 1, 1]);
        assert_eq!(manager.process_results(&ctx).len(), 1);
        assert_eq!(manager.thumbnails.bytes, 0);
        manager.request_thumbnail(1, 0, source());
        assert!(manager.queue.is_empty());
    }

    #[test]
    fn completed_results_still_count_against_worker_capacity() {
        let ctx = egui::Context::default();
        let mut manager = PreviewManager::with_limits(1024, 8, 2, 4);
        for index in 0..3 {
            manager.request_thumbnail(1, index, source());
        }
        for _ in 0..2 {
            let job = manager.next_job().unwrap();
            finish(&manager, job, [1, 1]);
        }
        assert!(manager.next_job().is_none());
        assert!(manager.process_results(&ctx).is_empty());
        assert!(manager.next_job().is_some());
    }
}
