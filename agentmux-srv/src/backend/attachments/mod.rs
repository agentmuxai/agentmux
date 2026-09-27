// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Attachments for the agent composer: ingest from paths, process on
//! a bounded blocking pool, report progress on the event bus, and keep a
//! content-addressed store with retention by last use.
//! docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6,
//! SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md.

pub mod extract;
pub mod kind;
pub mod process;
pub mod prompt;
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

pub use kind::FileKind;
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

/// Folders skipped when a dropped folder is expanded: dependency, build and
/// VCS trees that would fill the 128-file limit with noise.
/// SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §8.
const SKIPPED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "__pycache__",
    ".venv",
    "venv",
    "bower_components",
];

/// Limits and knobs, read from settings on every ingest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_files: u32,
    pub max_total_bytes: u64,
    pub send_max_edge: u32,
    pub retention: Duration,
    /// Images Claude gets inline (the rest by path only). 0 = paths only.
    pub claude_inline_max: usize,
    /// Base64 bytes Claude may get inline across one session.
    pub claude_session_inline_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_FILES,
            max_total_bytes: DEFAULT_MAX_TOTAL_MB * 1024 * 1024,
            send_max_edge: DEFAULT_SEND_MAX_EDGE,
            retention: Duration::from_secs(DEFAULT_RETENTION_DAYS * 24 * 3600),
            claude_inline_max: prompt::DEFAULT_INLINE_MAX_COUNT,
            claude_session_inline_bytes: prompt::DEFAULT_SESSION_INLINE_MB * 1024 * 1024,
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
            // 0 is meaningful here (paths only), so not `num`.
            claude_inline_max: extra
                .get("attachments:claudeinlinemax")
                .and_then(|v| v.as_f64())
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(|n| (n as usize).min(100))
                .unwrap_or(d.claude_inline_max),
            claude_session_inline_bytes: extra
                .get("attachments:claudesessioninlinemb")
                .and_then(|v| v.as_f64())
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(|n| (n * 1024.0 * 1024.0) as u64)
                .unwrap_or(d.claude_session_inline_bytes),
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
    /// Jobs currently processing each id. Identical images processed at
    /// once share one stored original, so a failing job may only discard it
    /// when no other job is still using it. Placing an original and the
    /// discard check both happen under this lock.
    inflight: Arc<Mutex<HashMap<String, usize>>>,
    /// Serializes the per-session inline-budget read, spend and write, so two
    /// overlapping turns of one Claude session can't both spend the same
    /// remaining budget or overwrite each other's count.
    session_budget: Mutex<()>,
}

/// One job's claim on an id in [`Service::inflight`], released on drop.
struct InflightGuard {
    map: Arc<Mutex<HashMap<String, usize>>>,
    id: String,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        let mut map = self.map.lock().unwrap();
        if let Some(n) = map.get_mut(&self.id) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.id);
            }
        }
    }
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

/// The service when it has been initialised (by the RPC or HTTP layer).
/// Prompt delivery uses this: it has no `AppState` to initialise from.
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
            inflight: Arc::new(Mutex::new(HashMap::new())),
            session_budget: Mutex::new(()),
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
    pub async fn info(self: &Arc<Self>, ids: &[String]) -> Vec<AttachmentInfo> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(meta) = self.ensure_derived(id).await {
                out.push(meta.to_info(id, ""));
            }
        }
        out
    }

    /// Metadata under the current fingerprint. When the send edge or the
    /// pipeline changed since the attachment was processed, re-derive it
    /// from the stored original under the same CPU and memory limits as
    /// ingest, so a transcript full of old thumbnails can't start an
    /// unbounded number of decodes at once.
    pub async fn ensure_derived(self: &Arc<Self>, id: &str) -> Option<store::StoredMeta> {
        let edge = self.limits().send_max_edge;
        let fp = store::fingerprint(edge);
        if let Some(meta) = self.store.meta(id, &fp) {
            return Some(meta);
        }
        let (blob, kind) = self.store.find_blob(id)?;
        let _cpu = self.permits.clone().acquire_owned().await.ok()?;
        // Another request may have derived it while this one waited.
        if let Some(meta) = self.store.meta(id, &fp) {
            return Some(meta);
        }
        let name = blob.file_name()?.to_string_lossy().into_owned();
        self.derive_kind(id, blob, kind, &name, edge).await.ok()
    }

    /// Derive a placed original. Images reserve decode memory from their
    /// header first; other files are bounded by the extraction limits
    /// (`extract.rs`) and PDFs run out of process. The caller holds a CPU slot.
    async fn derive_kind(
        self: &Arc<Self>,
        id: &str,
        blob: PathBuf,
        kind: FileKind,
        name: &str,
        edge: u32,
    ) -> Result<store::StoredMeta, process::ProcessError> {
        let _mem = if matches!(kind, FileKind::Image(_)) {
            let blob_for_dims = blob.clone();
            let (w, h) = blocking(move || process::header_dimensions(&blob_for_dims)).await?;
            self.reserve_memory(w, h).await
        } else {
            None
        };
        let svc = Arc::clone(self);
        let (id, name) = (id.to_string(), name.to_string());
        blocking(move || svc.store.derive_and_save(&id, &blob, kind, &name, edge)).await
    }

    /// Run `f` with the inline budget left in Claude session `key` (out of
    /// `max`), and record what it reports spending, as one step under a
    /// lock. `carry_from` is a per-block key counted before the session had
    /// an id; its count moves into `key` first. Blocking.
    pub fn with_session_inline_budget<T>(
        &self,
        key: &str,
        carry_from: Option<&str>,
        max: u64,
        f: impl FnOnce(u64) -> (T, u64),
    ) -> T {
        let _guard = self.session_budget.lock().unwrap();
        if let Some(from) = carry_from.filter(|from| *from != key) {
            self.store.adopt_session_inline_bytes(from, key);
        }
        let used = self.store.session_inline_bytes(key);
        let (out, spent) = f(max.saturating_sub(used));
        self.store.add_session_inline_bytes(key, spent);
        out
    }

    /// What the agent gets for attachment `id` sent under the user's `name`,
    /// marking it used: an image's send-copy, or for any other file a
    /// freshly made copy named `name`, with a note for the agent's list.
    /// Doesn't decode: call [`Self::ensure_derived`] first so a stale
    /// fingerprint is re-derived under the limits.
    pub fn send_target(&self, id: &str, name: &str) -> Option<SendTarget> {
        let fp = store::fingerprint(self.limits().send_max_edge);
        let meta = self.store.meta(id, &fp)?;
        let (path, mime) = if meta.is_image() {
            self.store.file(id, &fp, Kind::Send)?
        } else {
            (self.store.named_link(id, name)?, meta.mime.clone())
        };
        let text_path = self.store.file(id, &fp, Kind::Text).map(|(p, _)| p);
        self.store.mark_sent(id);
        self.store.touch(id, &fp);
        Some(SendTarget {
            path,
            mime,
            page_count: meta.page_count,
            note: file_note(&meta, text_path.as_deref()),
        })
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
        // Same CPU limit as batch jobs, so a burst of pastes can't run more
        // decodes at once than a drop can.
        let _cpu =
            self.permits
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| process::ProcessError {
                    code: "cancelled",
                    message: "Cancelled.".into(),
                })?;
        let edge = self.limits().send_max_edge;
        let svc = Arc::clone(self);
        let place_id = id.clone();
        let place_name = name.clone();
        let (blob, kind, guard) =
            blocking(move || svc.place_tracked(&tmp, &place_id, &place_name)).await?;
        let fp = store::fingerprint(edge);
        if let Some(meta) = self.store.meta(&id, &fp) {
            self.store.touch(&id, &fp);
            return Ok(meta.to_info(&id, &name));
        }
        let result = self.derive_kind(&id, blob, kind, &name, edge).await;
        match result {
            Ok(meta) => Ok(meta.to_info(&id, &name)),
            Err(e) => {
                self.discard_unprocessed(guard).await;
                Err(e)
            }
        }
    }

    /// [`Store::place`], registering this job as a user of the id first,
    /// under the same lock the discard check takes. Blocking.
    fn place_tracked(
        &self,
        tmp: &Path,
        id: &str,
        name: &str,
    ) -> Result<(PathBuf, FileKind, InflightGuard), process::ProcessError> {
        let mut map = self.inflight.lock().unwrap();
        *map.entry(id.to_string()).or_insert(0) += 1;
        let guard = InflightGuard {
            map: Arc::clone(&self.inflight),
            id: id.to_string(),
        };
        match self.store.place(tmp, id, name) {
            Ok((blob, kind)) => {
                drop(map);
                Ok((blob, kind, guard))
            }
            Err(e) => {
                drop(map);
                drop(guard);
                Err(e)
            }
        }
    }

    /// After a failed job: discard the original unless another job for the
    /// same bytes is still processing it ([`Store::discard_if_unprocessed`]
    /// also keeps it when any fingerprint has metadata). Consumes the guard.
    async fn discard_unprocessed(&self, guard: InflightGuard) {
        let store = self.store.clone();
        let inflight = Arc::clone(&self.inflight);
        let _ = tokio::task::spawn_blocking(move || {
            let map = inflight.lock().unwrap();
            if map.get(&guard.id).copied() == Some(1) {
                store.discard_if_unprocessed(&guard.id);
            }
            drop(map);
            drop(guard);
        })
        .await;
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
        let (id, blob, kind, guard) = placed;

        let fp = store::fingerprint(limits.send_max_edge);
        if let Some(meta) = self.store.meta(&id, &fp) {
            self.store.touch(&id, &fp);
            return Ok(meta.to_info(&id, &item.name));
        }

        let result = self
            .derive_placed(block_id, batch_id, &item, &id, blob, kind, limits, cancel)
            .await;
        if result.is_err() {
            // A decode failure or a cancel after the original was stored
            // must not leave it behind with no metadata.
            self.discard_unprocessed(guard).await;
        }
        result
    }

    /// The part of [`Self::run_item`] after the original is in `blobs/`:
    /// reserve decode memory from the header and derive.
    #[allow(clippy::too_many_arguments)]
    async fn derive_placed(
        self: &Arc<Self>,
        block_id: &str,
        batch_id: &str,
        item: &AttachmentPending,
        id: &str,
        blob: PathBuf,
        kind: FileKind,
        limits: Limits,
        cancel: &Arc<AtomicBool>,
    ) -> Result<AttachmentInfo, process::ProcessError> {
        let cancelled = || process::ProcessError {
            code: "cancelled",
            message: "Cancelled.".into(),
        };
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
        let meta = self
            .derive_kind(id, blob, kind, &item.name, limits.send_max_edge)
            .await?;
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        Ok(meta.to_info(id, &item.name))
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
    ) -> Result<(String, PathBuf, FileKind, InflightGuard), process::ProcessError> {
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
        let (blob, kind, guard) = self.place_tracked(&tmp, &id, &item.name)?;
        Ok((id, blob, kind, guard))
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

/// One attachment as delivered to an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendTarget {
    /// The file the agent opens: an image's send-copy, or a named copy.
    pub path: PathBuf,
    pub mime: String,
    pub page_count: Option<u32>,
    /// Shown in square brackets after the name in the agent's list.
    pub note: Option<String>,
}

/// "PDF, 12 pages; text version: /…/text.txt", "Word document; no text
/// version, save it as .docx", "spreadsheet; contains macros". `None` for
/// images and for kinds with nothing to add.
fn file_note(meta: &store::StoredMeta, text_path: Option<&Path>) -> Option<String> {
    if meta.is_image() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    let label = match meta.kind.as_str() {
        "pdf" => "PDF",
        "word" => "Word document",
        "excel" => "spreadsheet",
        "powerpoint" => "presentation",
        "svg" => "SVG image",
        "archive" => "archive",
        "audio" => "audio",
        "video" => "video",
        "image_file" => "image (not decoded)",
        _ => "",
    };
    match (label, meta.page_count) {
        ("", _) => {}
        (l, Some(n)) => parts.push(format!("{l}, {n} page{}", if n == 1 { "" } else { "s" })),
        (l, None) => parts.push(l.to_string()),
    }
    if let Some(p) = text_path {
        parts.push(format!("text version: {}", p.display()));
    } else if let Some(n) = &meta.text_note {
        parts.push(n.clone());
    }
    if meta.macros {
        parts.push("contains macros".into());
    }
    (!parts.is_empty()).then(|| parts.join("; "))
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

/// Files under `dir`, depth-first, sorted by name. Hidden files and folders
/// (a leading dot: `.git`, `.env`, …) and [`SKIPPED_DIRS`] are left out —
/// only a file dropped on its own is attached whatever its name.
fn files_in_dir(dir: &Path, budget: &mut usize, out: &mut Vec<PathBuf>) {
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
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                files_in_dir(&path, budget, out);
            }
        } else if ft.is_file() {
            out.push(path);
        }
    }
}

/// Every dropped file (and every file in a dropped folder) against the
/// limits. `non_images` is always empty now that any file can be attached;
/// the field stays for older frontends. Pure apart from reading metadata.
pub fn plan(paths: &[String], limits: &Limits, existing_count: u32, existing_bytes: u64) -> Plan {
    let mut out = Plan::default();
    let mut count = existing_count;
    let mut bytes = existing_bytes;
    let mut budget = MAX_WALK_ENTRIES;

    let mut candidates: Vec<PathBuf> = Vec::new();
    for raw in paths {
        let p = PathBuf::from(raw);
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() => files_in_dir(&p, &mut budget, &mut candidates),
            Ok(m) if m.is_file() => candidates.push(p),
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
                reason: format!(
                    "A message can carry at most {} attachments.",
                    limits.max_files
                ),
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

    #[tokio::test]
    async fn a_failing_job_keeps_an_original_another_job_is_still_using() {
        let d = tempfile::tempdir().unwrap();
        let svc = Arc::new(Service::new(
            Store::new(d.path().join("attachments")),
            Arc::new(Broker::new()),
            Arc::new(ConfigState::new()),
        ));
        // Two identical, undecodable-but-sniffable images in flight at once.
        let src = touch_file(d.path(), "same.png", 64);
        let (tmp_a, id, _) = svc.store.copy_in(Path::new(&src), |_| true).unwrap();
        let (tmp_b, id_b, _) = svc.store.copy_in(Path::new(&src), |_| true).unwrap();
        assert_eq!(id, id_b);
        let (blob, _, guard_a) = svc.place_tracked(&tmp_a, &id, "same.png").unwrap();
        let (_, _, guard_b) = svc.place_tracked(&tmp_b, &id, "same.png").unwrap();

        svc.discard_unprocessed(guard_a).await;
        assert!(blob.is_file(), "job B still needs the shared original");
        svc.discard_unprocessed(guard_b).await;
        assert!(!blob.is_file(), "the last failing job cleans it up");
        assert!(svc.inflight.lock().unwrap().is_empty());
    }

    #[test]
    fn concurrent_turns_spend_one_session_budget_once() {
        let d = tempfile::tempdir().unwrap();
        let svc = Arc::new(Service::new(
            Store::new(d.path().join("attachments")),
            Arc::new(Broker::new()),
            Arc::new(ConfigState::new()),
        ));
        // Eight turns race for a 100-byte budget, each wanting up to 30.
        let turns: Vec<_> = (0..8)
            .map(|_| {
                let svc = Arc::clone(&svc);
                std::thread::spawn(move || {
                    svc.with_session_inline_budget("session:s", None, 100, |budget| {
                        std::thread::sleep(Duration::from_millis(2));
                        ((), budget.min(30))
                    })
                })
            })
            .collect();
        for t in turns {
            t.join().unwrap();
        }
        assert_eq!(svc.store().session_inline_bytes("session:s"), 100);
    }

    #[test]
    fn plan_accepts_every_file() {
        let d = tempfile::tempdir().unwrap();
        let img = touch_file(d.path(), "a.png", 100);
        let noext = touch_file(d.path(), "screenshot", 100);
        let txt = d.path().join("notes.md");
        std::fs::write(&txt, "# hi").unwrap();
        let svg = d.path().join("icon.svg");
        std::fs::write(&svg, "<svg/>").unwrap();
        let (txt, svg): (String, String) =
            (txt.to_string_lossy().into(), svg.to_string_lossy().into());
        let p = plan(
            &[img.clone(), noext.clone(), txt.clone(), svg.clone()],
            &Limits::default(),
            0,
            0,
        );
        assert_eq!(
            p.accepted
                .iter()
                .map(|a| a.path.clone())
                .collect::<Vec<_>>(),
            vec![img, noext, txt, svg]
        );
        assert!(p.non_images.is_empty());
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
    fn plan_expands_folders_sorted_skipping_hidden_and_build_trees() {
        let d = tempfile::tempdir().unwrap();
        touch_file(d.path(), "proj/b.png", 10);
        touch_file(d.path(), "proj/a.jpg", 10);
        touch_file(d.path(), "proj/.cache/x.png", 10);
        touch_file(d.path(), "proj/node_modules/m/index.png", 10);
        touch_file(d.path(), "proj/target/debug/out.png", 10);
        std::fs::write(d.path().join("proj/readme.txt"), "x").unwrap();
        std::fs::write(d.path().join("proj/.env"), "SECRET=1").unwrap();
        let p = plan(
            &[d.path().join("proj").to_string_lossy().into()],
            &Limits::default(),
            0,
            0,
        );
        let names: Vec<_> = p.accepted.iter().map(|a| a.name.clone()).collect();
        assert_eq!(names, vec!["a.jpg", "b.png", "readme.txt"]);
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
        assert_eq!(l.claude_inline_max, prompt::DEFAULT_INLINE_MAX_COUNT);
        extra.insert("attachments:claudeinlinemax".into(), serde_json::json!(0));
        assert_eq!(
            Limits::from_settings(&extra).claude_inline_max,
            0,
            "0 means paths only"
        );
    }
}
