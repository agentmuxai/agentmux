// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Copy and move as background jobs: `fs.op.start`, `fs.op.resolve`,
//! `fs.op.cancel`, with progress published as `files:op` events.
//!
//! Each op runs on its own std thread, never on the async runtime: copying
//! blocks for as long as the disk takes, and a conflict blocks until the
//! user answers. At most `JobConfig::max_concurrent` ops work at once; the
//! rest wait their turn, reported as `running` with no progress.
//!
//! The rules every step follows, because each guards against losing data:
//! - A file is written under a temporary name in the destination folder and
//!   only then renamed into place, so a partial file never carries the real
//!   name, and canceling or failing removes it.
//! - Replace never truncates: a file over a file is a rename over it; any
//!   other replace first moves the old entry aside and puts it back if the
//!   new one can't be put in its place.
//! - A move deletes a source only after its copy fully succeeded; a folder
//!   is removed only once everything inside it was moved.
//! - No step follows a link: links are copied as links, never walked.
//! - Nothing that exists is overwritten without an answer: every create is
//!   a create-new or a no-replace rename, so a name taken in the meantime is
//!   a fresh conflict, not a silent overwrite.
//!
//! Spec: docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §7.1, §7.2, §9.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::backend::mps::{Broker, MuxEvent};
use crate::backend::rpc_types::{
    FsOpChoice, FsOpConflict, FsOpEvent, FsOpEventState, FsOpKind, FsOpResult, FsOpStartReq, FsOpStartResult,
};

use super::platform::{self, display_path};
use super::{path_key, resolve_entry_path, resolve_request_path, ProtectedPaths};

/// MPS event carrying an op's progress. Scoped to `block:<id>`; payload
/// `FsOpEvent`.
pub const EVENT_FILES_OP: &str = "files:op";

/// `running` events are sent at most this often (spec §10: one patch per
/// ~100 ms). Every other state goes out at once.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// How many `name (n)` candidates keep-both tries before giving up.
const MAX_KEEP_BOTH: u32 = 10_000;

/// How many times one item is retried when its destination keeps appearing
/// between the check and the write.
const MAX_RACE_RETRIES: usize = 8;

/// Delivers an op's events. In production, a `files:op` publish scoped to
/// the block that started it.
pub type Emit = Arc<dyn Fn(&FsOpEvent) + Send + Sync>;

/// Called after each chunk of a file is written, with the bytes written so
/// far. Tests use it to cancel mid-file.
type ChunkHook = Arc<dyn Fn(&OpControl, u64) + Send + Sync>;

/// The `Emit` that publishes `files:op` to `block:<block_id>`.
pub fn broker_emitter(broker: Arc<Broker>, block_id: String) -> Emit {
    let scope = format!("block:{block_id}");
    Arc::new(move |event: &FsOpEvent| {
        let data = match serde_json::to_value(event) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!(error = %e, "fs.op: couldn't encode a progress event");
                return;
            }
        };
        broker.publish(MuxEvent {
            event: EVENT_FILES_OP.to_string(),
            scopes: vec![scope.clone()],
            sender: String::new(),
            persist: 0,
            data: Some(data),
        });
    })
}

// ── Registry ─────────────────────────────────────────────────────────────

/// Tunables, with test seams that production leaves unset.
pub struct JobConfig {
    /// Bytes per read/write: bounds memory, and is how often a large file
    /// checks for cancel.
    pub chunk_size: usize,
    /// Ops working at once; later ones queue.
    pub max_concurrent: usize,
    /// How long a conflict waits for an answer before the op stops by
    /// itself. A pane closed mid-question never answers; without a limit
    /// its op would hold a slot, and its thread, until srv exits.
    pub conflict_wait_limit: Duration,
    protect: Option<ProtectedPaths>,
    /// Treat every move as crossing volumes, so tests reach copy-then-delete
    /// without a second volume.
    assume_cross_volume: bool,
    after_chunk: Option<ChunkHook>,
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            chunk_size: 1 << 20,
            max_concurrent: 4,
            conflict_wait_limit: Duration::from_secs(60 * 60),
            protect: None,
            assume_cross_volume: false,
            after_chunk: None,
        }
    }
}

/// One op's controls, shared by its thread and the RPC handlers.
#[derive(Default)]
pub struct OpControl {
    canceled: AtomicBool,
    question: Mutex<Question>,
    answered: Condvar,
}

#[derive(Default)]
struct Question {
    /// The op is blocked in the `conflict` state.
    waiting: bool,
    answer: Option<(FsOpChoice, bool)>,
}

impl OpControl {
    fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }

    /// Set the flag, then wake a conflict wait. The flag is set before the
    /// lock is taken and the waiter checks it under the lock, so the wake
    /// can't be missed.
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::SeqCst);
        let _question = lock(&self.question);
        self.answered.notify_all();
    }
}

/// A lock recovered if a panic elsewhere poisoned it: every holder leaves
/// the guarded state consistent, and a poisoned lock must not wedge every
/// later op.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The ops in flight, and the slots they take turns on.
pub struct Jobs {
    config: JobConfig,
    ops: Mutex<HashMap<String, Arc<OpControl>>>,
    active: Mutex<usize>,
    slot_freed: Condvar,
}

/// The process-wide job registry behind the `fs.op.*` handlers. Process-wide
/// rather than per connection, so a window reload doesn't orphan an op.
pub static JOBS: LazyLock<Arc<Jobs>> = LazyLock::new(|| Arc::new(Jobs::new(JobConfig::default())));

impl Jobs {
    pub fn new(config: JobConfig) -> Self {
        Self { config, ops: Mutex::new(HashMap::new()), active: Mutex::new(0), slot_freed: Condvar::new() }
    }

    fn protected(&self) -> ProtectedPaths {
        self.config.protect.clone().unwrap_or_else(ProtectedPaths::current)
    }

    /// `fs.op.start`: validate, then run in the background. Blocking (it
    /// stats and canonicalizes), so call it from `spawn_blocking`.
    pub fn start(self: &Arc<Self>, req: &FsOpStartReq, emit: Emit) -> Result<FsOpStartResult, String> {
        let plan = plan(req, &self.protected())?;
        let op_id = uuid::Uuid::new_v4().to_string();
        let control = Arc::new(OpControl::default());
        lock(&self.ops).insert(op_id.clone(), control.clone());

        let jobs = self.clone();
        let id = op_id.clone();
        let spawned = std::thread::Builder::new()
            .name("fs-op".to_string())
            .spawn(move || jobs.run(id, plan, control, emit));
        if let Err(e) = spawned {
            lock(&self.ops).remove(&op_id);
            return Err(format!("Couldn't start the operation: {e}"));
        }
        Ok(FsOpStartResult { op_id })
    }

    /// `fs.op.resolve`. Only an op blocked on a conflict takes an answer: one
    /// stored ahead of time would silently decide whatever conflict came
    /// next.
    pub fn resolve(&self, op_id: &str, choice: FsOpChoice, apply_to_all: bool) -> Result<(), String> {
        let control = lock(&self.ops)
            .get(op_id)
            .cloned()
            .ok_or_else(|| "That operation has already finished.".to_string())?;
        let mut question = lock(&control.question);
        if !question.waiting || question.answer.is_some() {
            return Err("That operation isn't waiting for an answer.".to_string());
        }
        question.answer = Some((choice, apply_to_all));
        control.answered.notify_all();
        Ok(())
    }

    /// `fs.op.cancel`. Wakes the op whether it is copying, waiting on a
    /// conflict or queued for a slot. Unknown or finished ops are ignored.
    pub fn cancel(&self, op_id: &str) {
        let Some(control) = lock(&self.ops).get(op_id).cloned() else {
            return;
        };
        control.cancel();
        // A queued op waits on `slot_freed`, not on its own condvar.
        let _active = lock(&self.active);
        self.slot_freed.notify_all();
    }

    #[cfg(test)]
    fn op_count(&self) -> usize {
        lock(&self.ops).len()
    }

    /// Block until a slot is free, or return `None` if canceled first.
    fn take_slot(&self, control: &OpControl) -> Option<Slot<'_>> {
        let mut active = lock(&self.active);
        loop {
            if control.is_canceled() {
                return None;
            }
            if *active < self.config.max_concurrent {
                *active += 1;
                return Some(Slot { jobs: self });
            }
            active = self.slot_freed.wait(active).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// The op's thread. Its registry entry and slot are released by guards,
    /// so they go however the op ends, a panic included.
    fn run(self: Arc<Self>, op_id: String, plan: Plan, control: Arc<OpControl>, emit: Emit) {
        let _registered = Registered { jobs: &self, op_id: &op_id };
        let mut job = Job::new(&self.config, op_id.clone(), plan.kind, control.clone(), emit);
        // The first event, at once, so the op shows up even while queued.
        job.emit_now(FsOpEventState::Running);
        let ending = match self.take_slot(&control) {
            None => Ending::Canceled,
            Some(_slot) => {
                job.protect = self.protected();
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.execute(&plan))).unwrap_or_else(|_| {
                    tracing::error!(op_id, "fs.op: the operation panicked");
                    Ending::Failed("The operation stopped unexpectedly.".to_string())
                })
            }
        };
        job.finish(ending, &plan);
    }
}

/// Removes an op from the registry when its thread ends.
struct Registered<'a> {
    jobs: &'a Jobs,
    op_id: &'a str,
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        lock(&self.jobs.ops).remove(self.op_id);
    }
}

/// One of the `max_concurrent` working slots.
struct Slot<'a> {
    jobs: &'a Jobs,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *lock(&self.jobs.active) -= 1;
        // Every waiter, not one: a woken waiter that was canceled leaves
        // without taking the slot, and a single wake would be lost with it.
        self.jobs.slot_freed.notify_all();
    }
}

// ── Plan (validated before `fs.op.start` answers) ────────────────────────

struct Plan {
    kind: FsOpKind,
    dest_dir: PathBuf,
    items: Vec<PlanItem>,
    /// Sources as requested, for the audit line (no-op moves included).
    requested: usize,
}

struct PlanItem {
    src: PathBuf,
    dest: PathBuf,
    /// Copying an entry into the folder it is already in: there's nothing
    /// to replace or skip, so it is always kept as `name (2)`, like a
    /// duplicate in a file manager.
    duplicate: bool,
}

fn verb(kind: FsOpKind) -> &'static str {
    match kind {
        FsOpKind::Copy => "copy",
        FsOpKind::Move => "move",
    }
}

fn plan(req: &FsOpStartReq, protect: &ProtectedPaths) -> Result<Plan, String> {
    let verb = verb(req.kind);
    if req.sources.is_empty() {
        return Err(format!("Nothing was given to {verb}."));
    }
    let dest_dir = resolve_request_path(&req.dest_dir)?.canonicalize().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => "The destination folder doesn't exist.".to_string(),
        io::ErrorKind::PermissionDenied => "You don't have permission to open the destination folder.".to_string(),
        _ => format!("Couldn't open the destination folder: {e}"),
    })?;
    if !std::fs::metadata(&dest_dir).is_ok_and(|m| m.is_dir()) {
        return Err("The destination isn't a folder.".to_string());
    }
    let dest_key = path_key(&dest_dir);

    let mut items = Vec::with_capacity(req.sources.len());
    for raw in &req.sources {
        let src = resolve_entry_path(raw)?;
        let name = src.file_name().ok_or_else(|| format!("“{raw}” isn't a valid path."))?.to_os_string();
        let shown = name.to_string_lossy().into_owned();
        std::fs::symlink_metadata(&src).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => format!("“{shown}” doesn't exist anymore."),
            io::ErrorKind::PermissionDenied => format!("You don't have permission to {verb} “{shown}”."),
            _ => format!("Couldn't read “{shown}”: {e}"),
        })?;
        // Path keys: canonical parents, case-folded on Windows. The source
        // keeps its own last component, so a link to an ancestor is not
        // mistaken for the ancestor (it is copied as a link).
        let src_key = path_key(&src);
        if dest_key.starts_with(&src_key) {
            return Err(format!("Can't {verb} a folder into itself."));
        }
        let in_dest_already = src.parent().map(path_key).as_ref() == Some(&dest_key);
        if in_dest_already && req.kind == FsOpKind::Move {
            // Already there: nothing to do.
            continue;
        }
        let dest = dest_dir.join(&name);
        protect.check(&dest)?;
        if req.kind == FsOpKind::Move {
            protect.check(&src)?;
        }
        items.push(PlanItem { src, dest, duplicate: in_dest_already });
    }

    // A destination entry must not hold another source: merging into it,
    // or replacing something in it, would act on that source before (or
    // while) it is copied, and a replace could delete it. E.g. copying
    // `/a/d/d` into `/a` merges into `/a/d`, which holds the source itself.
    let landing: HashSet<OsString> = items
        .iter()
        .filter(|i| !i.duplicate)
        .filter_map(|i| path_key(&i.dest).file_name().map(OsStr::to_os_string))
        .collect();
    for item in &items {
        let src_key = path_key(&item.src);
        let Ok(rel) = src_key.strip_prefix(&dest_key) else { continue };
        let mut parts = rel.components();
        if let (Some(first), Some(_)) = (parts.next(), parts.next()) {
            if landing.contains(first.as_os_str()) {
                return Err(format!(
                    "Can't {verb} there: “{}” would land on the folder that holds “{}”.",
                    first.as_os_str().to_string_lossy(),
                    file_name_lossy(&item.src)
                ));
            }
        }
    }

    Ok(Plan { kind: req.kind, dest_dir, items, requested: req.sources.len() })
}

fn file_name_lossy(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| display_path(path))
}

// ── The job ──────────────────────────────────────────────────────────────

/// What became of one entry and everything under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Fully copied or moved.
    Done,
    /// Skipped, or failed (the failure is recorded). A move keeps the
    /// source, and the folder holding it.
    NotDone,
    /// Canceled, or stopped waiting for an answer: end the op now.
    Stopped,
}

/// The destination's name was taken between the check and the write: look
/// again (it is now a conflict, or a folder to merge into).
struct Raced;

/// How to put an entry at its destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    /// Nothing is there; fail with `Raced` if something appears.
    New,
    /// Under the first free `name (n)`.
    KeepBoth,
    /// Over what is there, which had this type when the user was asked.
    Replace(std::fs::FileType),
}

enum Ending {
    Done,
    Canceled,
    Failed(String),
}

/// Entries and file bytes under a path, counted the same way as progress.
#[derive(Debug, Clone, Copy, Default)]
struct Measure {
    items: u64,
    bytes: u64,
}

impl Measure {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self { items: 1, bytes: if meta.is_file() { meta.len() } else { 0 } }
    }
}

enum CopyError {
    Stopped,
    Failed(String),
}

struct Job<'a> {
    config: &'a JobConfig,
    op_id: String,
    kind: FsOpKind,
    control: Arc<OpControl>,
    emit: Emit,
    protect: ProtectedPaths,
    done_items: u64,
    total_items: u64,
    done_bytes: u64,
    total_bytes: u64,
    current: Option<String>,
    failures: Vec<FsOpResult>,
    /// An answer given with "apply to all".
    sticky: Option<FsOpChoice>,
    /// Why the op stopped by itself, for the final `canceled` event.
    stop_reason: Option<String>,
    last_running: Option<Instant>,
    buf: Vec<u8>,
}

impl<'a> Job<'a> {
    fn new(config: &'a JobConfig, op_id: String, kind: FsOpKind, control: Arc<OpControl>, emit: Emit) -> Self {
        Self {
            config,
            op_id,
            kind,
            control,
            emit,
            protect: ProtectedPaths::default(),
            done_items: 0,
            total_items: 0,
            done_bytes: 0,
            total_bytes: 0,
            current: None,
            failures: Vec::new(),
            sticky: None,
            stop_reason: None,
            last_running: None,
            buf: Vec::new(),
        }
    }

    fn verb(&self) -> &'static str {
        verb(self.kind)
    }

    // ── Events ──

    fn event(&self, state: FsOpEventState) -> FsOpEvent {
        FsOpEvent {
            op_id: self.op_id.clone(),
            kind: self.kind,
            state,
            done_items: self.done_items,
            total_items: self.total_items,
            done_bytes: self.done_bytes,
            total_bytes: self.total_bytes,
            current: self.current.clone(),
            conflict: None,
            error: None,
            failures: None,
        }
    }

    fn emit_now(&mut self, state: FsOpEventState) {
        if state == FsOpEventState::Running {
            self.last_running = Some(Instant::now());
        }
        (self.emit)(&self.event(state));
    }

    /// A `running` event, unless one went out within `PROGRESS_INTERVAL`.
    /// Dropping one loses nothing: the next carries the counts, and the
    /// final event always goes out.
    fn tick(&mut self) {
        if self.last_running.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
            self.emit_now(FsOpEventState::Running);
        }
    }

    fn set_current(&mut self, src: &Path) {
        self.current = Some(display_path(src));
    }

    fn advance(&mut self, m: Measure) {
        self.done_items += m.items;
        self.done_bytes += m.bytes;
    }

    fn fail(&mut self, src: &Path, error: String) {
        tracing::debug!(op_id = %self.op_id, path = %display_path(src), error = %error, "fs.op: an item failed");
        self.failures.push(FsOpResult { path: display_path(src), ok: false, error: Some(error) });
    }

    /// Count `src`'s subtree as handled without copying it (skipped, or
    /// failed before anything was written), so progress still reaches the
    /// total.
    fn pass_over(&mut self, src: &Path, meta: &std::fs::Metadata, hint: Option<Measure>) {
        let m = match hint {
            Some(m) => m,
            None if meta.is_dir() => self.measure(src).unwrap_or_default(),
            None => Measure::of(meta),
        };
        self.advance(m);
    }

    fn finish(&mut self, ending: Ending, plan: &Plan) {
        let (state, error, outcome) = match ending {
            Ending::Done => (FsOpEventState::Done, None, "done"),
            Ending::Canceled => (FsOpEventState::Canceled, self.stop_reason.clone(), "canceled"),
            Ending::Failed(e) => (FsOpEventState::Failed, Some(e), "failed"),
        };
        tracing::info!(
            op = "fs.op",
            op_id = %self.op_id,
            kind = self.verb(),
            sources = plan.requested,
            dest = %display_path(&plan.dest_dir),
            outcome,
            done_items = self.done_items,
            total_items = self.total_items,
            failures = self.failures.len(),
            error = error.as_deref().unwrap_or(""),
            "fs mutation"
        );
        let mut event = self.event(state);
        event.current = None;
        event.error = error;
        event.failures = (!self.failures.is_empty()).then(|| self.failures.clone());
        (self.emit)(&event);
    }

    // ── The run ──

    fn execute(&mut self, plan: &Plan) -> Ending {
        // The folder may have gone while the op was queued.
        if !std::fs::symlink_metadata(&plan.dest_dir).is_ok_and(|m| m.is_dir()) {
            return Ending::Failed("The destination folder doesn't exist anymore.".to_string());
        }
        self.buf = vec![0; self.config.chunk_size.max(1)];

        // Totals first, so the bar means something from the start.
        let mut measures = Vec::with_capacity(plan.items.len());
        for item in &plan.items {
            let Some(m) = self.measure(&item.src) else { return Ending::Canceled };
            self.total_items += m.items;
            self.total_bytes += m.bytes;
            measures.push(m);
        }
        self.emit_now(FsOpEventState::Running);

        for (item, m) in plan.items.iter().zip(measures) {
            let outcome = self.transfer(&item.src, &item.dest, item.duplicate, Some(m), false);
            if outcome == Outcome::Stopped {
                return Ending::Canceled;
            }
        }
        Ending::Done
    }

    /// Count entries and file bytes under `path` without following links.
    /// What can't be read counts as one entry; the copy reports it.
    /// `None` if canceled meanwhile.
    fn measure(&self, path: &Path) -> Option<Measure> {
        let mut m = Measure::default();
        let Ok(root) = std::fs::symlink_metadata(path) else { return Some(Measure { items: 1, bytes: 0 }) };
        let mut stack = vec![(path.to_path_buf(), root)];
        while let Some((p, meta)) = stack.pop() {
            if self.control.is_canceled() {
                return None;
            }
            let own = Measure::of(&meta);
            m.items += own.items;
            m.bytes += own.bytes;
            if meta.is_dir() {
                let Ok(entries) = std::fs::read_dir(&p) else { continue };
                // `DirEntry::metadata` never follows a link.
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        stack.push((entry.path(), meta));
                    }
                }
            }
        }
        Some(m)
    }

    /// Copy or move `src` to `dest`, asking about a conflict, merging a
    /// folder into a folder. `hint` is `src`'s measure when already known.
    /// `cross` is set inside a move that already found it crosses volumes,
    /// so each entry doesn't retry the rename.
    fn transfer(&mut self, src: &Path, dest: &Path, duplicate: bool, hint: Option<Measure>, cross: bool) -> Outcome {
        if self.control.is_canceled() {
            return Outcome::Stopped;
        }
        self.set_current(src);
        self.tick();

        let meta = match std::fs::symlink_metadata(src) {
            Ok(meta) => meta,
            Err(e) => {
                let message = match e.kind() {
                    io::ErrorKind::NotFound => format!("“{}” doesn't exist anymore.", file_name_lossy(src)),
                    _ => describe(&e, "read", src),
                };
                self.fail(src, message);
                self.advance(hint.unwrap_or(Measure { items: 1, bytes: 0 }));
                return Outcome::NotDone;
            }
        };
        // Re-checked at the moment of writing (spec §9): `fs.op.start`
        // checked the top level, but the policy is computed afresh here and
        // applies to every entry.
        let checked = self.protect.check(dest).and_then(|()| match self.kind {
            FsOpKind::Move => self.protect.check(src),
            FsOpKind::Copy => Ok(()),
        });
        if let Err(message) = checked {
            self.fail(src, message);
            self.pass_over(src, &meta, hint);
            return Outcome::NotDone;
        }

        for _ in 0..MAX_RACE_RETRIES {
            let how = match std::fs::symlink_metadata(dest) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => How::New,
                Err(e) => {
                    self.fail(src, format!("Couldn't check the destination of “{}”: {e}", file_name_lossy(src)));
                    self.pass_over(src, &meta, hint);
                    return Outcome::NotDone;
                }
                Ok(_) if duplicate => How::KeepBoth,
                // Both real folders (neither a link): merge, asking per
                // entry inside (spec §7.2).
                Ok(dest_meta) if meta.is_dir() && dest_meta.is_dir() => return self.merge(src, dest, cross),
                Ok(dest_meta) => match self.ask(src, &meta, dest, &dest_meta) {
                    None => return Outcome::Stopped,
                    Some(FsOpChoice::Skip) => {
                        self.pass_over(src, &meta, hint);
                        return Outcome::NotDone;
                    }
                    Some(FsOpChoice::KeepBoth) => How::KeepBoth,
                    Some(FsOpChoice::Replace) => How::Replace(dest_meta.file_type()),
                },
            };
            match self.place(src, &meta, dest, how, hint, cross) {
                Ok(outcome) => return outcome,
                Err(Raced) => continue,
            }
        }
        self.fail(src, format!("“{}” kept changing at the destination, so it was left alone.", file_name_lossy(src)));
        self.pass_over(src, &meta, hint);
        Outcome::NotDone
    }

    /// Block in `conflict` until answered. `None` if canceled, or if the
    /// wait limit passed (then `stop_reason` says so).
    fn ask(&mut self, src: &Path, meta: &std::fs::Metadata, dest: &Path, dest_meta: &std::fs::Metadata) -> Option<FsOpChoice> {
        if let Some(choice) = self.sticky {
            return Some(choice);
        }
        // `waiting` is set before the event goes out, so an answer sent the
        // moment the prompt shows is never refused as "not waiting".
        {
            let mut question = lock(&self.control.question);
            question.waiting = true;
            question.answer = None;
        }
        let mut event = self.event(FsOpEventState::Conflict);
        event.conflict = Some(FsOpConflict {
            source: display_path(src),
            dest: display_path(dest),
            source_is_dir: meta.is_dir(),
            dest_is_dir: dest_meta.is_dir(),
            source_size: meta.is_file().then_some(meta.len()),
            dest_size: dest_meta.is_file().then_some(dest_meta.len()),
            source_mtime: mtime_ms(meta),
            dest_mtime: mtime_ms(dest_meta),
        });
        (self.emit)(&event);

        let deadline = Instant::now() + self.config.conflict_wait_limit;
        let answer = {
            let mut question = lock(&self.control.question);
            let answer = loop {
                if self.control.is_canceled() {
                    break None;
                }
                if let Some(answer) = question.answer.take() {
                    break Some(answer);
                }
                let now = Instant::now();
                if now >= deadline {
                    self.stop_reason = Some(format!(
                        "No one answered about “{}”, so the operation stopped.",
                        file_name_lossy(src)
                    ));
                    break None;
                }
                question = self.control.answered.wait_timeout(question, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
            };
            question.waiting = false;
            answer
        };
        let (choice, apply_to_all) = answer?;
        if apply_to_all {
            self.sticky = Some(choice);
        }
        self.emit_now(FsOpEventState::Running);
        Some(choice)
    }

    /// Put `src` at `dest` the way `how` says.
    fn place(
        &mut self,
        src: &Path,
        meta: &std::fs::Metadata,
        dest: &Path,
        mut how: How,
        hint: Option<Measure>,
        cross: bool,
    ) -> Result<Outcome, Raced> {
        if let How::Replace(asked) = how {
            // The answer may come long after the question. If what is there
            // now isn't what the user agreed to replace (a file became a
            // folder, say), ask again rather than delete something unseen.
            match std::fs::symlink_metadata(dest) {
                Ok(now) if now.file_type() == asked => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => how = How::New,
                _ => return Err(Raced),
            }
        }
        let mut cross = cross || self.config.assume_cross_volume;
        if self.kind == FsOpKind::Move && !cross {
            match self.place_by_rename(src, meta, dest, how, hint) {
                Renamed::Finished(outcome) => return Ok(outcome),
                Renamed::Raced => return Err(Raced),
                // Another volume: copy, then delete the source.
                Renamed::CrossVolume => cross = true,
            }
        }
        if meta.is_file() {
            self.place_file(src, meta, dest, how)
        } else if meta.is_symlink() {
            self.place_link(src, meta, dest, how)
        } else if meta.is_dir() {
            self.place_dir(src, meta, dest, how, cross)
        } else {
            // A FIFO, socket or device: opening one to read could block
            // forever or read without end.
            self.fail(src, format!("“{}” isn't a file, folder or link, so it can't be copied.", file_name_lossy(src)));
            self.advance(Measure::of(meta));
            Ok(Outcome::NotDone)
        }
    }

    // ── Move by rename (same volume) ──

    fn place_by_rename(&mut self, src: &Path, meta: &std::fs::Metadata, dest: &Path, how: How, hint: Option<Measure>) -> Renamed {
        // Measured first: once renamed, there is nothing at `src` to count.
        let m = match hint {
            Some(m) => m,
            None if meta.is_dir() => self.measure(src).unwrap_or_default(),
            None => Measure::of(meta),
        };
        let result = match how {
            How::New => platform::rename_no_replace(src, dest),
            How::KeepBoth => keep_both(dest, meta.is_dir(), &self.protect, |c| platform::rename_no_replace(src, c)).map(drop),
            How::Replace(_) => match std::fs::symlink_metadata(dest) {
                // A file over a file: one atomic rename, nothing to put back.
                Ok(d) if meta.is_file() && d.is_file() => std::fs::rename(src, dest),
                Ok(_) => self.replace_aside(src, dest, |d| platform::rename_no_replace(src, d)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => platform::rename_no_replace(src, dest),
                Err(e) => Err(e),
            },
        };
        match result {
            Ok(()) => {
                self.advance(m);
                Renamed::Finished(Outcome::Done)
            }
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => Renamed::CrossVolume,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && how == How::New => Renamed::Raced,
            Err(e) => {
                let message = describe(&e, "move", src);
                self.fail(src, message);
                self.advance(m);
                Renamed::Finished(Outcome::NotDone)
            }
        }
    }

    // ── Copy (and move across volumes) ──

    fn place_file(&mut self, src: &Path, meta: &std::fs::Metadata, dest: &Path, how: How) -> Result<Outcome, Raced> {
        let start_bytes = self.done_bytes;
        let tmp = match self.copy_to_temp(src, meta, dest) {
            Ok(tmp) => tmp,
            Err(CopyError::Stopped) => return Ok(Outcome::Stopped),
            Err(CopyError::Failed(message)) => {
                self.fail(src, message);
                self.settle_file(start_bytes, meta);
                return Ok(Outcome::NotDone);
            }
        };
        let placed = match how {
            How::New => match platform::rename_no_replace(&tmp, dest) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    discard(&tmp);
                    self.done_bytes = start_bytes;
                    return Err(Raced);
                }
                other => other,
            },
            How::KeepBoth => keep_both(dest, false, &self.protect, |c| platform::rename_no_replace(&tmp, c)).map(drop),
            How::Replace(_) => match std::fs::symlink_metadata(dest) {
                // A file over a file: renamed over it in one step, so the
                // original is intact until the copy is complete.
                Ok(d) if d.is_file() => std::fs::rename(&tmp, dest),
                Ok(_) => self.replace_aside(src, dest, |d| platform::rename_no_replace(&tmp, d)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => platform::rename_no_replace(&tmp, dest),
                Err(e) => Err(e),
            },
        };
        self.settle_file(start_bytes, meta);
        if let Err(e) = placed {
            discard(&tmp);
            let message = describe(&e, self.verb(), src);
            self.fail(src, message);
            return Ok(Outcome::NotDone);
        }
        if self.kind == FsOpKind::Move {
            // Only now, with the copy complete and in place.
            if let Err(e) = platform::remove_file_force(src) {
                self.fail(src, format!("Copied “{}”, but couldn't remove the original: {e}", file_name_lossy(src)));
                return Ok(Outcome::NotDone);
            }
        }
        Ok(Outcome::Done)
    }

    /// The file's progress once it is handled, whatever happened: one item,
    /// and its size (or what was written, if it grew).
    fn settle_file(&mut self, start_bytes: u64, meta: &std::fs::Metadata) {
        self.done_bytes = self.done_bytes.max(start_bytes + meta.len());
        self.done_items += 1;
    }

    /// Stream `src` into a new temporary file beside `dest`. On any error
    /// or cancel the temporary file is removed.
    fn copy_to_temp(&mut self, src: &Path, meta: &std::fs::Metadata, dest: &Path) -> Result<PathBuf, CopyError> {
        let name = file_name_lossy(src);
        let mut from = platform::open_no_follow(src).map_err(|e| CopyError::Failed(describe(&e, "read", src)))?;
        // The opened file must still be a regular file: not a link or
        // anything else swapped in since it was listed.
        match from.metadata() {
            Ok(m) if m.is_file() => {}
            Ok(_) => return Err(CopyError::Failed(format!("“{name}” changed while it was being copied."))),
            Err(e) => return Err(CopyError::Failed(describe(&e, "read", src))),
        }
        let tmp = sibling_temp(dest, "part");
        let mut to = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| CopyError::Failed(describe(&e, self.verb(), src)))?;

        let result = self.stream(&mut from, &mut to, src).and_then(|written| {
            // Best effort, as a file manager does: a filesystem that can't
            // keep a time or a mode still gets the contents.
            if let Ok(mtime) = meta.modified() {
                let _ = to.set_modified(mtime);
            }
            if self.kind == FsOpKind::Move {
                // The original is about to be deleted: the copy must be on
                // disk and whole first.
                to.sync_all().map_err(|e| CopyError::Failed(describe(&e, "move", src)))?;
                let now_len = from.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
                let copied_len = to.metadata().map(|m| m.len()).unwrap_or(u64::MAX - 1);
                if now_len != written || copied_len != written {
                    return Err(CopyError::Failed(format!(
                        "“{name}” changed while it was being moved, so the original was kept."
                    )));
                }
            }
            // Last: a read-only mode would refuse the steps above.
            let _ = to.set_permissions(meta.permissions());
            Ok(())
        });
        drop(to);
        match result {
            Ok(()) => Ok(tmp),
            Err(e) => {
                discard(&tmp);
                Err(e)
            }
        }
    }

    /// Copy all of `from` into `to` in `chunk_size` pieces, checking for
    /// cancel before each. Returns the bytes written.
    fn stream(&mut self, from: &mut std::fs::File, to: &mut std::fs::File, src: &Path) -> Result<u64, CopyError> {
        // The job's one buffer, lent out so `self` stays free for progress.
        let mut buf = std::mem::take(&mut self.buf);
        let result = self.stream_with(&mut buf, from, to, src);
        self.buf = buf;
        result
    }

    fn stream_with(&mut self, buf: &mut [u8], from: &mut std::fs::File, to: &mut std::fs::File, src: &Path) -> Result<u64, CopyError> {
        let mut written = 0u64;
        loop {
            if self.control.is_canceled() {
                return Err(CopyError::Stopped);
            }
            let n = match from.read(buf) {
                Ok(0) => return Ok(written),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(CopyError::Failed(describe(&e, "read", src))),
            };
            to.write_all(&buf[..n]).map_err(|e| CopyError::Failed(describe(&e, self.verb(), src)))?;
            written += n as u64;
            self.done_bytes += n as u64;
            self.tick();
            if let Some(hook) = &self.config.after_chunk {
                hook(&self.control, written);
            }
        }
    }

    fn place_link(&mut self, src: &Path, meta: &std::fs::Metadata, dest: &Path, how: How) -> Result<Outcome, Raced> {
        let target = match std::fs::read_link(src) {
            Ok(target) => target,
            Err(e) => {
                let message = describe(&e, "read", src);
                self.fail(src, message);
                self.advance(Measure::of(meta));
                return Ok(Outcome::NotDone);
            }
        };
        let kind = meta.file_type();
        let make = |at: &Path| platform::create_symlink(&target, at, &kind);
        let made = match how {
            How::New => match make(dest) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(Raced),
                other => other,
            },
            How::KeepBoth => keep_both(dest, false, &self.protect, make).map(drop),
            How::Replace(_) => self.replace_aside(src, dest, make),
        };
        self.advance(Measure::of(meta));
        if let Err(e) = made {
            let message = if platform::is_symlink_privilege_error(&e) {
                format!(
                    "Windows didn't allow creating the link “{}”. Turn on Developer Mode, or run AgentMux as an administrator.",
                    file_name_lossy(src)
                )
            } else {
                describe(&e, self.verb(), src)
            };
            self.fail(src, message);
            return Ok(Outcome::NotDone);
        }
        if self.kind == FsOpKind::Move {
            if let Err(e) = platform::remove_entry_no_follow(src) {
                self.fail(src, format!("Copied the link “{}”, but couldn't remove the original: {e}", file_name_lossy(src)));
                return Ok(Outcome::NotDone);
            }
        }
        Ok(Outcome::Done)
    }

    fn place_dir(&mut self, src: &Path, meta: &std::fs::Metadata, dest: &Path, how: How, cross: bool) -> Result<Outcome, Raced> {
        let mut aside = None;
        let created = match how {
            How::New => match std::fs::create_dir(dest) {
                Ok(()) => Ok(dest.to_path_buf()),
                // Now a folder (merge) or a conflict: look again.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(Raced),
                Err(e) => Err(e),
            },
            How::KeepBoth => keep_both(dest, true, &self.protect, |c| std::fs::create_dir(c)),
            // `dest` is a file or a link (a folder would have been merged):
            // move it aside until the new folder is complete.
            How::Replace(_) => {
                let spot = sibling_temp(dest, "replaced");
                match platform::rename_no_replace(dest, &spot) {
                    Ok(()) => {
                        aside = Some(spot);
                        std::fs::create_dir(dest).map(|()| dest.to_path_buf())
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::create_dir(dest).map(|()| dest.to_path_buf()),
                    Err(e) => Err(e),
                }
            }
        };
        let created = match created {
            Ok(created) => created,
            Err(e) => {
                if let Some(spot) = aside {
                    self.restore_aside(src, &spot, dest);
                }
                let message = describe(&e, self.verb(), src);
                self.fail(src, message);
                self.pass_over(src, meta, None);
                return Ok(Outcome::NotDone);
            }
        };
        self.advance(Measure { items: 1, bytes: 0 });
        let outcome = self.copy_children(src, &created, cross);
        // Unix folder modes, applied once the contents are in (a read-only
        // mode would refuse them). Windows' read-only flag on a folder
        // means "customized", not read-only, so it isn't copied there.
        if cfg!(unix) && outcome == Outcome::Done {
            let _ = std::fs::set_permissions(&created, meta.permissions());
        }

        if let Some(spot) = aside {
            if outcome == Outcome::Done {
                self.drop_aside(src, &spot);
            } else {
                // Put the original back. A copy's partial folder holds only
                // copies and can go; a move's holds moved files and stays
                // (the original then comes back under a free name).
                match self.kind {
                    FsOpKind::Copy => {
                        let _ = platform::remove_entry_no_follow(&created);
                    }
                    FsOpKind::Move => {
                        let _ = std::fs::remove_dir(&created);
                    }
                }
                self.restore_aside(src, &spot, dest);
            }
        }
        if outcome == Outcome::Done && self.kind == FsOpKind::Move {
            return Ok(self.remove_moved_dir(src));
        }
        Ok(outcome)
    }

    /// Merge the folder `src` into the existing folder `dest` (spec §7.2).
    fn merge(&mut self, src: &Path, dest: &Path, cross: bool) -> Outcome {
        self.advance(Measure { items: 1, bytes: 0 });
        let outcome = self.copy_children(src, dest, cross);
        if outcome == Outcome::Done && self.kind == FsOpKind::Move {
            return self.remove_moved_dir(src);
        }
        outcome
    }

    /// A moved folder's source, once everything inside it went. Only ever
    /// an empty folder: `remove_dir` refuses anything else.
    fn remove_moved_dir(&mut self, src: &Path) -> Outcome {
        match std::fs::remove_dir(src) {
            Ok(()) => Outcome::Done,
            Err(e) => {
                self.fail(src, format!("Moved what was in “{}”, but couldn't remove the folder itself: {e}", file_name_lossy(src)));
                Outcome::NotDone
            }
        }
    }

    /// Each entry of `src` into `dest`. The names are read up front: a move
    /// takes entries out of `src` while going, and a directory changing
    /// under an open listing may skip entries on some filesystems.
    fn copy_children(&mut self, src: &Path, dest: &Path, cross: bool) -> Outcome {
        let mut names = Vec::new();
        let mut complete = true;
        match std::fs::read_dir(src) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => names.push(entry.file_name()),
                        Err(_) => complete = false,
                    }
                }
            }
            Err(e) => {
                let message = describe(&e, "read", src);
                self.fail(src, message);
                return Outcome::NotDone;
            }
        }
        if !complete {
            self.fail(src, format!("Some of what's in “{}” couldn't be read, so it was left out.", file_name_lossy(src)));
        }
        for name in names {
            match self.transfer(&src.join(&name), &dest.join(&name), false, None, cross) {
                Outcome::Stopped => return Outcome::Stopped,
                Outcome::NotDone => complete = false,
                Outcome::Done => {}
            }
        }
        if complete {
            Outcome::Done
        } else {
            Outcome::NotDone
        }
    }

    // ── Replace helpers ──

    /// Replace `dest` with what `put` creates there: move `dest` aside under
    /// a temporary name in the same folder first, remove it once `put`
    /// succeeded, or put it back if `put` failed. Used whenever a rename
    /// can't atomically replace (a folder, a link, a type change).
    fn replace_aside(&mut self, src: &Path, dest: &Path, put: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
        let spot = sibling_temp(dest, "replaced");
        match platform::rename_no_replace(dest, &spot) {
            Ok(()) => {}
            // Gone meanwhile: nothing left to replace.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return put(dest),
            Err(e) => return Err(e),
        }
        match put(dest) {
            Ok(()) => {
                self.drop_aside(src, &spot);
                Ok(())
            }
            Err(e) => {
                self.restore_aside(src, &spot, dest);
                Err(e)
            }
        }
    }

    /// Delete the replaced original. It was the user's choice to replace it;
    /// failing to delete it is reported but loses nothing.
    fn drop_aside(&mut self, src: &Path, spot: &Path) {
        if let Err(e) = platform::remove_entry_no_follow(spot) {
            self.fail(
                src,
                format!(
                    "Replaced “{}”, but couldn't remove the old one; it's still there as “{}”: {e}",
                    file_name_lossy(src),
                    file_name_lossy(spot)
                ),
            );
        }
    }

    /// Put a set-aside original back at `dest`, or, if something took that
    /// name meanwhile, under the first free `name (n)` beside it rather
    /// than leave it under a hidden temporary name.
    fn restore_aside(&mut self, src: &Path, spot: &Path, dest: &Path) {
        if platform::rename_no_replace(spot, dest).is_ok() {
            return;
        }
        let is_dir = std::fs::symlink_metadata(spot).is_ok_and(|m| m.is_dir());
        match keep_both(dest, is_dir, &self.protect, |c| platform::rename_no_replace(spot, c)) {
            Ok(put) => self.fail(
                src,
                format!("The original “{}” couldn't go back under its name; it's now “{}”.", file_name_lossy(dest), file_name_lossy(&put)),
            ),
            Err(e) => self.fail(
                src,
                format!("The original “{}” couldn't be put back; it's still there as “{}”: {e}", file_name_lossy(dest), file_name_lossy(spot)),
            ),
        }
    }
}

enum Renamed {
    Finished(Outcome),
    Raced,
    CrossVolume,
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// A hidden, unique name beside `dest`, in the same folder so a rename
/// from it is never a cross-volume move.
fn sibling_temp(dest: &Path, tag: &str) -> PathBuf {
    dest.with_file_name(format!(".agentmux-{tag}-{}", uuid::Uuid::new_v4().simple()))
}

/// Remove a temporary file this job wrote. Best effort: the failure that
/// led here is the one worth reporting.
fn discard(tmp: &Path) {
    if let Err(e) = platform::remove_file_force(tmp) {
        tracing::warn!(path = %display_path(tmp), error = %e, "fs.op: couldn't remove a partial copy");
    }
}

/// `name (n).ext` for a file, `name (n)` for a folder (whose dots aren't an
/// extension) or a name with no extension. Works on the raw name, so a
/// name that isn't valid Unicode keeps its bytes.
fn numbered_name(name: &OsStr, n: u32, is_dir: bool) -> OsString {
    let path = Path::new(name);
    let mut out = OsString::new();
    match (is_dir, path.file_stem(), path.extension()) {
        (false, Some(stem), Some(ext)) => {
            out.push(stem);
            out.push(format!(" ({n})."));
            out.push(ext);
        }
        _ => {
            out.push(name);
            out.push(format!(" ({n})"));
        }
    }
    out
}

/// Keep both: `put` at the first free `name (2)`, `name (3)`… beside `dest`.
/// Each attempt is itself the existence check (`put` must fail with
/// `AlreadyExists` on a taken name), so a name taken by another writer
/// between attempts moves on to the next number instead of overwriting.
fn keep_both(dest: &Path, is_dir: bool, protect: &ProtectedPaths, mut put: impl FnMut(&Path) -> io::Result<()>) -> io::Result<PathBuf> {
    let name = dest.file_name().ok_or_else(|| io::Error::other("That path has no name."))?;
    for n in 2..=MAX_KEEP_BOTH {
        let candidate = dest.with_file_name(numbered_name(name, n, is_dir));
        protect.check(&candidate).map_err(io::Error::other)?;
        match put(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(format!(
        "There's no free name left for a copy of “{}”.",
        name.to_string_lossy()
    )))
}

/// An I/O error as a sentence naming the item.
fn describe(err: &io::Error, verb: &str, src: &Path) -> String {
    let name = file_name_lossy(src);
    // Our own errors (`io::Error::other`) already are sentences.
    if err.raw_os_error().is_none() && err.get_ref().is_some() {
        return err.to_string();
    }
    match err.kind() {
        io::ErrorKind::NotFound => format!("“{name}” or its destination doesn't exist anymore."),
        io::ErrorKind::PermissionDenied => format!("You don't have permission to {verb} “{name}” there."),
        io::ErrorKind::StorageFull => format!("There isn't enough space to {verb} “{name}”."),
        _ => format!("Couldn't {verb} “{name}”: {err}"),
    }
}

/// Unix millis of a modification time, for the conflict prompt.
fn mtime_ms(meta: &std::fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
