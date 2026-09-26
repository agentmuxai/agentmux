// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Image attachments for the agent composer: ingest from paths, process on
//! a bounded blocking pool, report progress on the event bus, and keep a
//! content-addressed store with retention by last use.
//! docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.

pub mod process;
pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::backend::mps::{self, Broker, MuxEvent};
use crate::backend::rpc_types::{
    AttachmentBatchDoneEvent, AttachmentFailedEvent, AttachmentInfo, AttachmentPending,
    AttachmentProgressEvent, AttachmentReadyEvent, AttachmentRejected, AttachmentsIngestResult,
    CommandAttachmentsIngestData,
};
use crate::backend::wconfig::ConfigState;

pub use store::{is_valid_id, Kind, Store};

pub const DEFAULT_MAX_FILES: u32 = 128;
pub const DEFAULT_MAX_TOTAL_MB: u64 = 1024;
pub const DEFAULT_SEND_MAX_EDGE: u32 = 2000;
pub const DEFAULT_RETENTION_DAYS: u64 = 30;

/// Folder drops are walked for images; stop after this many entries so a
/// drop of a home directory can't hang the server.
const MAX_WALK_ENTRIES: usize = 10_000;
/// Estimated decode memory all running jobs may hold at once, in MiB. A job
/// reserves `process::decode_cost_bytes` of it from the image header before
/// decoding; one bigger than the whole budget takes all of it and runs alone.
const DECODE_BUDGET_MIB: u32 = 2048;
/// Minimum gap between two progress events for the same item.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Extensions treated as images without reading the file. Anything else is
/// sniffed. SVG is deliberately absent: until it can be rasterized it keeps
/// the existing "copy into the working folder" behavior, where the agent can
/// at least read it as text.
const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "webp", "bmp", "dib", "tif", "tiff", "heic",
    "heif", "avif",
];

/// Limits and knobs, read from settings on every ingest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_files: u32,
    pub max_total_bytes: u64,
    pub send_max_edge: u32,
    pub retention: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_FILES,
            max_total_bytes: DEFAULT_MAX_TOTAL_MB * 1024 * 1024,
            send_max_edge: DEFAULT_SEND_MAX_EDGE,
            retention: Duration::from_secs(DEFAULT_RETENTION_DAYS * 24 * 3600),
        }
    }
}

impl Limits {
    pub fn from_settings(extra: &HashMap<String, serde_json::Value>) -> Self {
        let num = |k: &str| {
            extra
                .get(k)
                .and_then(|v| v.as_f64())
                .filter(|n| n.is_finite() && *n > 0.0)
        };
        let d = Self::default();
        Self {
            max_files: num("attachments:maxfiles")
                .map(|n| n as u32)
                .unwrap_or(d.max_files),
            max_total_bytes: num("attachments:maxtotalmb")
                .map(|n| (n * 1024.0 * 1024.0) as u64)
                .unwrap_or(d.max_total_bytes),
            send_max_edge: num("attachments:sendmaxedge")
                .map(|n| (n as u32).clamp(256, 8000))
                .unwrap_or(d.send_max_edge),
            retention: num("attachments:retentiondays")
                .map(|n| Duration::from_secs((n * 24.0 * 3600.0) as u64))
                .unwrap_or(d.retention),
        }
    }
}

pub struct Service {
    store: Store,
    broker: Arc<Broker>,
    config: Arc<ConfigState>,
    /// Bounds concurrent jobs (CPU).
    permits: Arc<tokio::sync::Semaphore>,
    /// Bounds the decode memory of concurrent jobs, in MiB.
    memory: Arc<tokio::sync::Semaphore>,
    /// Cancellation flags of batches still running.
    batches: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();

/// The process-wide service. The first call creates the store and starts
/// the retention timer; later calls return the same instance.
pub fn init(broker: Arc<Broker>, config: Arc<ConfigState>) -> Arc<Service> {
    SERVICE
        .get_or_init(|| {
            let root = crate::backend::base::get_mux_data_dir().join("attachments");
            let svc = Arc::new(Service::new(Store::new(root), broker, config));
            svc.start_sweeper();
            svc
        })
        .clone()
}

// Used by prompt delivery (next PR in the stack), which has no AppState.
#[allow(dead_code)]
pub fn get() -> Option<Arc<Service>> {
    SERVICE.get().cloned()
}

impl Service {
    pub fn new(store: Store, broker: Arc<Broker>, config: Arc<ConfigState>) -> Self {
        if let Err(e) = store.ensure_dirs() {
            tracing::warn!(error = %e, root = %store.root().display(), "attachments: cannot create store");
        }
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        Self {
            store,
            broker,
            config,
            permits: Arc::new(tokio::sync::Semaphore::new((workers / 2).max(1))),
            memory: Arc::new(tokio::sync::Semaphore::new(DECODE_BUDGET_MIB as usize)),
            batches: Mutex::new(HashMap::new()),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn limits(&self) -> Limits {
        Limits::from_settings(&self.config.get_settings().extra)
    }

    fn start_sweeper(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let svc = Arc::clone(self);
        handle.spawn(async move {
            // Let startup finish first.
            tokio::time::sleep(Duration::from_secs(60)).await;
            loop {
                let store = svc.store.clone();
                let retention = svc.limits().retention;
                let removed = tokio::task::spawn_blocking(move || store.sweep(retention))
                    .await
                    .unwrap_or(0);
                if removed > 0 {
                    tracing::info!(removed, "attachments: retention sweep");
                }
                tokio::time::sleep(SWEEP_INTERVAL).await;
            }
        });
    }

    /// Classify `paths`, apply the limits, and (unless `dry_run`) start
    /// processing the accepted images in the background. Returns at once.
    pub fn ingest(self: &Arc<Self>, req: CommandAttachmentsIngestData) -> AttachmentsIngestResult {
        let limits = self.limits();
        let plan = plan(&req.paths, &limits, req.existing_count, req.existing_bytes);
        let result = AttachmentsIngestResult {
            batch_id: req.batch_id.clone(),
            accepted: plan.accepted.clone(),
            rejected: plan.rejected,
            non_images: plan.non_images,
            total_bytes: plan.accepted.iter().map(|a| a.bytes).sum(),
            max_files: limits.max_files,
            max_total_bytes: limits.max_total_bytes,
        };
        if req.dry_run || plan.accepted.is_empty() {
            return result;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.batches
            .lock()
            .unwrap()
            .insert(req.batch_id.clone(), Arc::clone(&cancel));
        let svc = Arc::clone(self);
        let items = plan.accepted;
        tokio::spawn(async move {
            svc.run_batch(req.block_id, req.batch_id, items, limits, cancel)
                .await;
        });
        result
    }

    pub fn cancel(&self, batch_id: &str) -> bool {
        match self.batches.lock().unwrap().get(batch_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Info for each id still in the store, in request order. May decode
    /// (after a send-edge change), so call it off the async workers.
    pub fn info(&self, ids: &[String]) -> Vec<AttachmentInfo> {
        let edge = self.limits().send_max_edge;
        ids.iter()
            .filter_map(|id| {
                self.store
                    .ensure_derived(id, edge)
                    .map(|m| m.to_info(id, ""))
            })
            .collect()
    }

    /// Resolve an id to the send-copy path and MIME type for delivery,
    /// marking it used. May decode, so call it off the async workers.
    #[allow(dead_code)] // used by prompt delivery (next PR in the stack)
    pub fn send_path(&self, id: &str) -> Option<(PathBuf, String)> {
        let edge = self.limits().send_max_edge;
        self.store.ensure_derived(id, edge)?;
        let fp = store::fingerprint(edge);
        let found = self.store.file(id, &fp, Kind::Send)?;
        self.store.touch(id, &fp);
        Some(found)
    }

    /// Reserve decode memory for a `w`×`h` image. Held until dropped.
    async fn reserve_memory(&self, w: u32, h: u32) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let mib = process::decode_cost_bytes(w, h).div_ceil(1024 * 1024);
        let mib = (mib as u32).clamp(1, DECODE_BUDGET_MIB);
        self.memory.clone().acquire_many_owned(mib).await.ok()
    }

    /// Store a file already sitting in `incoming/` (an upload): place it,
    /// reserve decode memory, derive. Returns its info.
    pub async fn commit_upload(
        self: &Arc<Self>,
        tmp: PathBuf,
        id: String,
        name: String,
    ) -> Result<AttachmentInfo, process::ProcessError> {
        let edge = self.limits().send_max_edge;
        let svc = Arc::clone(self);
        let (blob, format, dims, existing) = blocking(move || {
            let (blob, format) = svc.store.place(&tmp, &id)?;
            let fp = store::fingerprint(edge);
            if let Some(meta) = svc.store.meta(&id, &fp) {
                svc.store.touch(&id, &fp);
                return Ok((blob, format, (0, 0), Some((id, meta))));
            }
            let dims = process::header_dimensions(&blob)?;
            Ok((blob, format, dims, None::<(String, store::StoredMeta)>))
        })
        .await?;
        if let Some((id, meta)) = existing {
            return Ok(meta.to_info(&id, &name));
        }
        let id = blob
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let _mem = self.reserve_memory(dims.0, dims.1).await;
        let svc = Arc::clone(self);
        blocking(move || {
            let meta = svc.store.derive_and_save(&id, &blob, format, edge)?;
            Ok(meta.to_info(&id, &name))
        })
        .await
    }

    async fn run_batch(
        self: Arc<Self>,
        block_id: String,
        batch_id: String,
        items: Vec<AttachmentPending>,
        limits: Limits,
        cancel: Arc<AtomicBool>,
    ) {
        let mut tasks = Vec::with_capacity(items.len());
        for item in items {
            let svc = Arc::clone(&self);
            let block_id = block_id.clone();
            let batch_id = batch_id.clone();
            let cancel = Arc::clone(&cancel);
            tasks.push(tokio::spawn(async move {
                let index = item.index;
                let outcome = svc
                    .run_item(&block_id, &batch_id, item, limits, &cancel)
                    .await;
                match outcome {
                    Ok(info) => svc.publish(
                        &block_id,
                        mps::EVENT_ATTACHMENT_READY,
                        &AttachmentReadyEvent {
                            batch_id,
                            index,
                            info,
                        },
                    ),
                    Err(e) => svc.publish(
                        &block_id,
                        mps::EVENT_ATTACHMENT_FAILED,
                        &AttachmentFailedEvent {
                            batch_id,
                            index,
                            code: e.code.into(),
                            error: e.message,
                        },
                    ),
                }
            }));
        }
        for t in tasks {
            let _ = t.await;
        }
        self.batches.lock().unwrap().remove(&batch_id);
        self.publish(
            &block_id,
            mps::EVENT_ATTACHMENT_BATCH_DONE,
            &AttachmentBatchDoneEvent { batch_id },
        );
    }

    /// One item: wait for a CPU slot, copy + hash + place, reserve decode
    /// memory from the header, derive. The cancel flag is checked before
    /// each step and between copy chunks; a decode already running finishes
    /// but is reported as cancelled.
    async fn run_item(
        self: &Arc<Self>,
        block_id: &str,
        batch_id: &str,
        item: AttachmentPending,
        limits: Limits,
        cancel: &Arc<AtomicBool>,
    ) -> Result<AttachmentInfo, process::ProcessError> {
        let cancelled = || process::ProcessError {
            code: "cancelled",
            message: "Cancelled.".into(),
        };
        let _cpu = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| cancelled())?;
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }

        let svc = Arc::clone(self);
        let (block, batch, it, flag) = (
            block_id.to_string(),
            batch_id.to_string(),
            item.clone(),
            Arc::clone(cancel),
        );
        let placed =
            blocking(move || svc.copy_and_place(&block, &batch, &it, limits, &flag)).await?;
        let (id, blob, format) = placed;

        let fp = store::fingerprint(limits.send_max_edge);
        if let Some(meta) = self.store.meta(&id, &fp) {
            self.store.touch(&id, &fp);
            return Ok(meta.to_info(&id, &item.name));
        }

        let blob_for_dims = blob.clone();
        let (w, h) = blocking(move || process::header_dimensions(&blob_for_dims)).await?;
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let _mem = self.reserve_memory(w, h).await;
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        self.publish(
            block_id,
            mps::EVENT_ATTACHMENT_PROGRESS,
            &AttachmentProgressEvent {
                batch_id: batch_id.to_string(),
                index: item.index,
                stage: "processing".into(),
                done_bytes: item.bytes,
                total_bytes: item.bytes,
            },
        );
        let svc = Arc::clone(self);
        let id2 = id.clone();
        let meta = blocking(move || {
            svc.store
                .derive_and_save(&id2, &blob, format, limits.send_max_edge)
        })
        .await?;
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        Ok(meta.to_info(&id, &item.name))
    }

    /// Copy the source into `incoming/` with progress and cancellation, then
    /// move it into `blobs/`. Runs on a blocking thread.
    fn copy_and_place(
        &self,
        block_id: &str,
        batch_id: &str,
        item: &AttachmentPending,
        limits: Limits,
        cancel: &AtomicBool,
    ) -> Result<(String, PathBuf, image::ImageFormat), process::ProcessError> {
        let mut last = Instant::now() - PROGRESS_INTERVAL;
        let copied = self.store.copy_in(Path::new(&item.path), |done| {
            if cancel.load(Ordering::SeqCst) {
                return false;
            }
            if last.elapsed() >= PROGRESS_INTERVAL || done >= item.bytes {
                last = Instant::now();
                self.publish(
                    block_id,
                    mps::EVENT_ATTACHMENT_PROGRESS,
                    &AttachmentProgressEvent {
                        batch_id: batch_id.to_string(),
                        index: item.index,
                        stage: "copying".into(),
                        done_bytes: done,
                        total_bytes: item.bytes,
                    },
                );
            }
            // A file that grows while we copy it must not blow the limit.
            done <= limits.max_total_bytes
        });
        let (tmp, id, _) = copied.map_err(|e| {
            if cancel.load(Ordering::SeqCst) {
                process::ProcessError {
                    code: "cancelled",
                    message: "Cancelled.".into(),
                }
            } else {
                process::ProcessError {
                    code: "io",
                    message: format!("Couldn't read the file: {e}"),
                }
            }
        })?;
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&tmp);
            return Err(process::ProcessError {
                code: "cancelled",
                message: "Cancelled.".into(),
            });
        }
        let (blob, format) = self.store.place(&tmp, &id)?;
        Ok((id, blob, format))
    }

    fn publish<T: Serialize>(&self, block_id: &str, event: &str, data: &T) {
        self.broker.publish(MuxEvent {
            event: event.to_string(),
            scopes: vec![format!("block:{block_id}")],
            sender: String::new(),
            persist: 0,
            data: serde_json::to_value(data).ok(),
        });
    }
}

/// Run blocking work off the async workers, turning a panic into an error.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, process::ProcessError> + Send + 'static,
) -> Result<T, process::ProcessError> {
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| {
        Err(process::ProcessError {
            code: "decode",
            message: format!("Processing crashed: {e}"),
        })
    })
}

/// The outcome of classifying an ingest request against the limits.
#[derive(Debug, Default)]
pub struct Plan {
    pub accepted: Vec<AttachmentPending>,
    pub rejected: Vec<AttachmentRejected>,
    pub non_images: Vec<String>,
}

fn display_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn is_image_path(p: &Path) -> bool {
    let by_ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match by_ext {
        Some(e) if IMAGE_EXTS.contains(&e.as_str()) => true,
        Some(e) if e == "svg" => false,
        _ => !matches!(
            process::sniff_file(p),
            Ok(process::Sniffed::NotImage) | Ok(process::Sniffed::Svg) | Err(_)
        ),
    }
}

/// Images under `dir`, depth-first, sorted by name, skipping dot-folders.
fn images_in_dir(dir: &Path, budget: &mut usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            if !e.file_name().to_string_lossy().starts_with('.') {
                images_in_dir(&path, budget, out);
            }
        } else if ft.is_file() && is_image_path(&path) {
            out.push(path);
        }
    }
}

/// Classify paths into images (accepted or refused by the limits) and
/// non-images. Pure apart from reading file metadata and a few header bytes.
pub fn plan(paths: &[String], limits: &Limits, existing_count: u32, existing_bytes: u64) -> Plan {
    let mut out = Plan::default();
    let mut count = existing_count;
    let mut bytes = existing_bytes;
    let mut budget = MAX_WALK_ENTRIES;

    let mut candidates: Vec<PathBuf> = Vec::new();
    for raw in paths {
        let p = PathBuf::from(raw);
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() => images_in_dir(&p, &mut budget, &mut candidates),
            Ok(m) if m.is_file() => {
                if is_image_path(&p) {
                    candidates.push(p);
                } else {
                    out.non_images.push(raw.clone());
                }
            }
            _ => out.rejected.push(AttachmentRejected {
                path: raw.clone(),
                name: display_name(&p),
                code: "not_found".into(),
                reason: "The file doesn't exist or can't be read.".into(),
            }),
        }
    }

    for p in candidates {
        let name = display_name(&p);
        let path = p.to_string_lossy().into_owned();
        let size = match std::fs::metadata(&p) {
            Ok(m) => m.len(),
            Err(_) => {
                out.rejected.push(AttachmentRejected {
                    path,
                    name,
                    code: "unreadable".into(),
                    reason: "The file can't be read.".into(),
                });
                continue;
            }
        };
        if count >= limits.max_files {
            out.rejected.push(AttachmentRejected {
                path,
                name,
                code: "too_many".into(),
                reason: format!("A prompt can carry at most {} images.", limits.max_files),
            });
            continue;
        }
        if bytes.saturating_add(size) > limits.max_total_bytes {
            out.rejected.push(AttachmentRejected {
                path,
                name,
                code: "too_large".into(),
                reason: format!(
                    "Adding it would pass the {} limit for one prompt.",
                    format_limit(limits.max_total_bytes)
                ),
            });
            continue;
        }
        out.accepted.push(AttachmentPending {
            index: out.accepted.len() as u32,
            path,
            name,
            bytes: size,
        });
        count += 1;
        bytes += size;
    }
    out
}

fn format_limit(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    if bytes >= GB && bytes % GB == 0 {
        format!("{} GB", bytes / GB)
    } else {
        format!("{} MB", bytes / MB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_file(dir: &Path, name: &str, bytes: usize) -> String {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        // A PNG signature is enough for classification by content.
        let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        data.resize(bytes.max(8), 0);
        std::fs::write(&p, data).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn plan_splits_images_from_other_files() {
        let d = tempfile::tempdir().unwrap();
        let img = touch_file(d.path(), "a.png", 100);
        let noext = touch_file(d.path(), "screenshot", 100);
        let txt = d.path().join("notes.md");
        std::fs::write(&txt, "# hi").unwrap();
        let svg = d.path().join("icon.svg");
        std::fs::write(&svg, "<svg/>").unwrap();
        let p = plan(
            &[
                img.clone(),
                noext.clone(),
                txt.to_string_lossy().into(),
                svg.to_string_lossy().into(),
            ],
            &Limits::default(),
            0,
            0,
        );
        assert_eq!(
            p.accepted
                .iter()
                .map(|a| a.path.clone())
                .collect::<Vec<_>>(),
            vec![img, noext]
        );
        assert_eq!(p.non_images.len(), 2);
        assert!(p.rejected.is_empty());
    }

    #[test]
    fn plan_applies_count_limit_including_existing() {
        let d = tempfile::tempdir().unwrap();
        let paths: Vec<String> = (0..5)
            .map(|i| touch_file(d.path(), &format!("{i}.png"), 10))
            .collect();
        let limits = Limits {
            max_files: 4,
            ..Limits::default()
        };
        let p = plan(&paths, &limits, 2, 0);
        assert_eq!(p.accepted.len(), 2);
        assert_eq!(p.rejected.len(), 3);
        assert!(p.rejected.iter().all(|r| r.code == "too_many"));
        assert_eq!(
            p.accepted.iter().map(|a| a.index).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn plan_applies_byte_limit_and_skips_only_what_does_not_fit() {
        let d = tempfile::tempdir().unwrap();
        let big = touch_file(d.path(), "big.png", 800);
        let small = touch_file(d.path(), "small.png", 100);
        let limits = Limits {
            max_total_bytes: 1000,
            ..Limits::default()
        };
        let p = plan(&[big.clone(), big.clone(), small.clone()], &limits, 0, 0);
        assert_eq!(
            p.accepted
                .iter()
                .map(|a| a.path.clone())
                .collect::<Vec<_>>(),
            vec![big, small]
        );
        assert_eq!(p.rejected.len(), 1);
        assert_eq!(p.rejected[0].code, "too_large");
    }

    #[test]
    fn plan_expands_folders_sorted_and_skips_dot_folders() {
        let d = tempfile::tempdir().unwrap();
        touch_file(d.path(), "shots/b.png", 10);
        touch_file(d.path(), "shots/a.jpg", 10);
        touch_file(d.path(), "shots/.cache/x.png", 10);
        std::fs::write(d.path().join("shots/readme.txt"), "x").unwrap();
        let p = plan(
            &[d.path().join("shots").to_string_lossy().into()],
            &Limits::default(),
            0,
            0,
        );
        let names: Vec<_> = p.accepted.iter().map(|a| a.name.clone()).collect();
        assert_eq!(names, vec!["a.jpg", "b.png"]);
        // Non-images inside a dropped folder are ignored, not copied.
        assert!(p.non_images.is_empty());
    }

    #[test]
    fn plan_reports_missing_paths() {
        let p = plan(
            &["Z:/definitely/not/here.png".into()],
            &Limits::default(),
            0,
            0,
        );
        assert_eq!(p.rejected.len(), 1);
        assert_eq!(p.rejected[0].code, "not_found");
    }

    #[test]
    fn limits_read_from_settings_with_defaults() {
        let mut extra = HashMap::new();
        assert_eq!(Limits::from_settings(&extra), Limits::default());
        extra.insert("attachments:maxfiles".into(), serde_json::json!(10));
        extra.insert("attachments:maxtotalmb".into(), serde_json::json!(50));
        extra.insert("attachments:sendmaxedge".into(), serde_json::json!(99999));
        extra.insert("attachments:retentiondays".into(), serde_json::json!(-3));
        let l = Limits::from_settings(&extra);
        assert_eq!(l.max_files, 10);
        assert_eq!(l.max_total_bytes, 50 * 1024 * 1024);
        assert_eq!(l.send_max_edge, 8000);
        assert_eq!(l.retention, Limits::default().retention);
    }
}
