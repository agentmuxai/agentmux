// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Downloads in a browser pane: the OS "Save as" dialog.
//!
//! Spec: `docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md`.
//!
//! A pane is an Alloy-style browser, and without a `DownloadHandler` CEF
//! cancels every download, so a click on a download did nothing. This handler
//! answers `on_before_download` with `cont(default_path, show_dialog = true)`:
//! CEF then shows the platform's own Save As dialog (owned by the pane's
//! window) and writes the file wherever the user chose. We write no file code
//! and no dialog code, and the page never picks a path (§7.1).
//!
//! Everything that can be decided without a running CEF lives in plain
//! functions and structs here (the name sanitiser, the default folder, the
//! per-pane limit) so `cargo test` covers it; `cargo test` has no CEF process
//! (the constraint recorded in `media_prompt.rs`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

/// How many downloads one pane may have waiting for a Save As answer (§7.2).
/// A page can start downloads without a click; the dialog is the consent, so
/// the limit is on how many may wait for one, not on how many a user may take.
const MAX_WAITING_PER_PANE: usize = 3;

/// Longest file name we suggest, keeping the extension (§7.1).
const MAX_NAME_CHARS: usize = 200;

/// The name used when nothing usable is left of the suggested one.
const FALLBACK_NAME: &str = "download";

// ---------------------------------------------------------------------------
// The suggested file name (spec §7.1)
// ---------------------------------------------------------------------------

/// Names Windows refuses as a file stem, case-insensitively, with or without
/// an extension (`CON.txt` is as reserved as `CON`).
const RESERVED_STEMS: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A file name safe to suggest, from the server's `Content-Disposition` or the
/// URL, both of which are page-controlled text.
///
/// Only the file-name part survives (no directory or drive, with either
/// separator), control characters and `<>:"|?*` go, trailing dots and spaces
/// go, a Windows device name gets a `_` prefix, and the length is capped
/// without losing the extension. The dialog shows the result and the user may
/// change it; the host never writes to any path but the one the dialog returns.
pub(crate) fn safe_download_name(raw: &str) -> String {
    // Last component after either separator: `..\..\x.exe` -> `x.exe`,
    // `C:\x` -> `x`, `a/b.txt` -> `b.txt`.
    let leaf = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = leaf
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .collect();
    let cleaned = cleaned
        .trim_end_matches(['.', ' '])
        .trim_start()
        .to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return FALLBACK_NAME.to_string();
    }

    let cleaned = cap_length(&cleaned);

    // Windows device names are reserved by stem, so `con.tar.gz` counts too.
    let stem = cleaned.split('.').next().unwrap_or("");
    if RESERVED_STEMS.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return format!("_{cleaned}");
    }
    cleaned
}

/// Shorten `name` to `MAX_NAME_CHARS` characters, keeping its last extension
/// (`.gz` of `x.tar.gz`) so the file still opens with the right program.
fn cap_length(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_CHARS {
        return name.to_string();
    }
    // An "extension" longer than 16 characters is part of the name, not one.
    let ext = name
        .rfind('.')
        .map(|i| &name[i..])
        .filter(|e| e.chars().count() <= 16 && e.len() > 1)
        .unwrap_or("");
    let keep = MAX_NAME_CHARS - ext.chars().count();
    let head: String = name.chars().take(keep).collect();
    let head = head.trim_end_matches(['.', ' ']);
    format!("{head}{ext}")
}

// ---------------------------------------------------------------------------
// The default folder (spec §4.4)
// ---------------------------------------------------------------------------

/// The first of `candidates` that is an existing directory.
fn first_existing_dir(candidates: &[Option<PathBuf>]) -> Option<PathBuf> {
    candidates.iter().flatten().find(|d| d.is_dir()).cloned()
}

/// Where the Save As dialog opens: the last folder a pane download went to,
/// else the OS Downloads folder, else the home folder. A remembered folder
/// that has since been deleted is skipped; none is ever created.
pub(crate) fn default_save_dir(
    last: Option<PathBuf>,
    downloads: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    first_existing_dir(&[last, downloads, home])
}

/// The last folder a pane download was saved to this session. In memory only
/// in this version (spec §4.4).
static LAST_DOWNLOAD_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

fn remember_download_dir(path: &Path) {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        *LAST_DOWNLOAD_DIR.lock() = Some(dir.to_path_buf());
    }
}

/// The full path offered to the dialog for a download named `suggested`.
fn default_save_path(suggested: &str) -> PathBuf {
    let name = safe_download_name(suggested);
    let dir = default_save_dir(
        LAST_DOWNLOAD_DIR.lock().clone(),
        dirs::download_dir(),
        dirs::home_dir(),
    );
    match dir {
        Some(d) => d.join(name),
        // No folder at all: a bare name, which the dialog resolves itself.
        None => PathBuf::from(name),
    }
}

// ---------------------------------------------------------------------------
// The per-pane limit (spec §7.2)
// ---------------------------------------------------------------------------

/// Downloads waiting for a Save As answer, per pane. Plain data, no CEF, so it
/// is tested directly.
///
/// A download is *waiting* from `on_before_download` until its first update
/// that shows the dialog was answered: a path chosen (`running`) or a final
/// state (cancelled, failed). The pane closing clears its entries.
#[derive(Default)]
pub(crate) struct WaitingDownloads {
    by_pane: HashMap<String, HashSet<u32>>,
}

impl WaitingDownloads {
    /// Can this pane have another download wait for its dialog?
    pub(crate) fn has_room(&self, pane: &str) -> bool {
        self.by_pane.get(pane).map_or(0, HashSet::len) < MAX_WAITING_PER_PANE
    }

    /// A download is now waiting. Returns false (and records nothing) when the
    /// pane is full, so the caller cancels it.
    pub(crate) fn add(&mut self, pane: &str, id: u32) -> bool {
        if !self.has_room(pane) {
            return false;
        }
        self.by_pane.entry(pane.to_string()).or_default().insert(id);
        true
    }

    /// The dialog for `id` in `pane` was answered (or the download ended).
    /// Returns whether it was waiting; a repeat, or an id never added, changes
    /// nothing. Ids are only unique within one request context (incognito jars
    /// and named profiles each have their own download manager), so the pane
    /// is part of the key and one pane's update can never free another's slot.
    pub(crate) fn release(&mut self, pane: &str, id: u32) -> bool {
        let Some(ids) = self.by_pane.get_mut(pane) else {
            return false;
        };
        let was_waiting = ids.remove(&id);
        if ids.is_empty() {
            self.by_pane.remove(pane);
        }
        was_waiting
    }

    /// The pane closed: nothing of its can still be waiting.
    pub(crate) fn clear_pane(&mut self, pane: &str) {
        self.by_pane.remove(pane);
    }
}

/// One Save As dialog at a time per pane. CEF routes a download's dialog through the pane's single file
/// chooser, so a second `cont(.., show_dialog = true)` while one is open cancels the new download at
/// once and dismisses the open dialog without its download ever ending (macOS, 2026-11-06: ids 5 and 6;
/// the first kept its waiting slot and its hidden temp file). So only the first download of a pane asks;
/// the rest are parked, still counted as waiting, and ask in turn as each dialog is answered. Plain
/// data over the parked value `T`, so it is tested without CEF.
pub(crate) struct SaveDialogs<T> {
    open: HashMap<String, u32>,
    parked: HashMap<String, VecDeque<(u32, T)>>,
}

impl<T> Default for SaveDialogs<T> {
    fn default() -> Self {
        Self { open: HashMap::new(), parked: HashMap::new() }
    }
}

impl<T> SaveDialogs<T> {
    /// A download of `pane` is ready to ask. Returns it back when its dialog may open now; `None`
    /// means it is parked behind the pane's open dialog.
    pub(crate) fn offer(&mut self, pane: &str, id: u32, item: T) -> Option<T> {
        if self.open.contains_key(pane) {
            self.parked.entry(pane.to_string()).or_default().push_back((id, item));
            return None;
        }
        self.open.insert(pane.to_string(), id);
        Some(item)
    }

    /// The dialog for `id` was answered, or the download ended. When it was the open one, the pane's
    /// next parked download is returned and is now the open one. A parked download that ended before
    /// its turn is dropped; an id never offered changes nothing.
    pub(crate) fn finished(&mut self, pane: &str, id: u32) -> Option<(u32, T)> {
        if self.open.get(pane) != Some(&id) {
            if let Some(q) = self.parked.get_mut(pane) {
                q.retain(|(p, _)| *p != id);
                if q.is_empty() {
                    self.parked.remove(pane);
                }
            }
            return None;
        }
        self.open.remove(pane);
        let q = self.parked.get_mut(pane)?;
        let next = q.pop_front();
        if q.is_empty() {
            self.parked.remove(pane);
        }
        if let Some((nid, _)) = &next {
            self.open.insert(pane.to_string(), *nid);
        }
        next
    }

    /// The pane closed: nothing of its is open or parked.
    pub(crate) fn clear_pane(&mut self, pane: &str) {
        self.open.remove(pane);
        self.parked.remove(pane);
    }
}

thread_local! {
    /// The panes' Save As queues. The parked value holds a `BeforeDownloadCallback`, which is not
    /// `Send`, so it lives on the CEF UI thread like `RUNNING`.
    static SAVE_DIALOGS: std::cell::RefCell<SaveDialogs<(String, BeforeDownloadCallback)>> =
        std::cell::RefCell::new(SaveDialogs::default());
}

/// Whether the Save As dialog is ours (`rfd`) rather than CEF's. On Linux CEF's chooser answered
/// "cancel" within milliseconds with no dialog on the packaged build (2026-11-06, D0), so Linux asks
/// with the same dialog library the Vault export and layout dialogs use (spec §4.3).
const OWN_SAVE_DIALOG: bool = cfg!(target_os = "linux");

thread_local! {
    /// The callbacks of downloads whose `rfd` dialog is open, by (pane, id). Not `Send`, so they stay on
    /// the UI thread; the dialog's thread posts the answer back there.
    static ASKING: std::cell::RefCell<HashMap<(String, u32), BeforeDownloadCallback>> =
        std::cell::RefCell::new(HashMap::new());
}

wrap_task! {
    struct SaveAnswerTask {
        block_id: String,
        id: u32,
        path: Option<PathBuf>,
    }

    impl Task {
        fn execute(&self) {
            let cb = ASKING.with(|m| m.borrow_mut().remove(&(self.block_id.clone(), self.id)));
            // Gone: the pane closed while the dialog was open.
            let Some(cb) = cb else { return };
            match &self.path {
                // `show_dialog = false`: the person already chose, CEF just writes there.
                Some(p) => cb.cont(Some(&CefString::from(p.to_string_lossy().as_ref())), 0),
                // Cancelled: dropping the last reference cancels the download (CEF does that for a
                // callback never answered). An empty path would instead save to the temp folder.
                None => drop(cb),
            }
        }
    }
}

/// Ask where to save with `rfd` on its own thread (the dialog blocks), then answer on the UI thread. A
/// dialog can't be closed from outside, so one still open when its pane closes stays until the person
/// answers; the answer is then dropped (`SaveAnswerTask` finds no callback).
fn ask_with_rfd(block_id: &str, id: u32, path: PathBuf, cb: &BeforeDownloadCallback) {
    ASKING.with(|m| m.borrow_mut().insert((block_id.to_string(), id), cb.clone()));
    let block_id = block_id.to_string();
    std::thread::spawn(move || {
        let mut dialog = rfd::FileDialog::new().set_title("Save as");
        if let Some(name) = path.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
        if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let chosen = dialog.save_file();
        let mut task = SaveAnswerTask::new(block_id.clone(), id, chosen);
        if post_task(ThreadId::UI, Some(&mut task)) == 0 {
            tracing::warn!(target: "pane-download", %block_id, id, "could not post the Save As answer to the UI thread");
        }
    });
}

/// Open the Save As dialog for a download, starting at the folder it would go to.
fn show_save_dialog(block_id: &str, id: u32, suggested: &str, cb: &BeforeDownloadCallback) {
    let path = default_save_path(suggested);
    if OWN_SAVE_DIALOG {
        ask_with_rfd(block_id, id, path, cb);
        return;
    }
    // show_dialog = true: CEF opens the platform's Save As dialog, starting at `path`, and cancels
    // the download if the user cancels.
    cb.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 1);
}

static WAITING: Mutex<Option<WaitingDownloads>> = Mutex::new(None);

fn with_waiting<R>(f: impl FnOnce(&mut WaitingDownloads) -> R) -> R {
    f(WAITING.lock().get_or_insert_with(WaitingDownloads::default))
}

// ---------------------------------------------------------------------------
// A download for the Vault (`Browser` `capture` with `download`, spec §15.1)
// ---------------------------------------------------------------------------

/// Panes waiting for a download to go into the Vault, with how many downloads
/// reached this handler meanwhile. The capture takes the file off the network
/// before it becomes a download; one that reaches CEF anyway (a file the page
/// built itself, or one from a frame the capture can't intercept) would be
/// written to disk, so it is refused, and counted so the capture can say why.
static FOR_VAULT: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

/// While held, the pane's downloads are refused (`refused` counts them).
pub(crate) struct VaultHold {
    block_id: String,
}

/// Holds `block_id`'s downloads for the Vault; `None` when a capture already does.
pub(crate) fn hold_for_vault(block_id: &str) -> Option<VaultHold> {
    let mut held = FOR_VAULT.lock();
    let held = held.get_or_insert_with(HashMap::new);
    if held.contains_key(block_id) {
        return None;
    }
    held.insert(block_id.to_string(), 0);
    Some(VaultHold { block_id: block_id.to_string() })
}

impl VaultHold {
    /// How many downloads were refused while held.
    pub(crate) fn refused(&self) -> usize {
        FOR_VAULT.lock().as_ref().and_then(|h| h.get(&self.block_id).copied()).unwrap_or(0)
    }
}

impl Drop for VaultHold {
    fn drop(&mut self) {
        if let Some(h) = FOR_VAULT.lock().as_mut() {
            h.remove(&self.block_id);
        }
    }
}

/// Whether `block_id`'s downloads are held for the Vault, counting this one.
fn refuse_for_vault(block_id: &str) -> bool {
    match FOR_VAULT.lock().as_mut().and_then(|h| h.get_mut(block_id)) {
        Some(n) => {
            *n += 1;
            true
        }
        None => false,
    }
}

/// A pane closed: drop what it had waiting. Safe from any thread, and for a
/// pane that never had a download.
pub(crate) fn forget_pane(block_id: &str) {
    with_waiting(|w| w.clear_pane(block_id));
    with_throttle(|t| t.clear_pane(block_id));
    // The parked callbacks belong to the UI thread.
    let mut task = ForgetPaneTask::new(block_id.to_string());
    let _ = post_task(ThreadId::UI, Some(&mut task));
}

// ---------------------------------------------------------------------------
// Progress events (spec §5)
// ---------------------------------------------------------------------------

/// The event the pane's window gets for a download.
pub(crate) const UPDATE_EVENT: &str = "pane-download-update";

/// At most this many `running` events per download per second (spec §5).
const MIN_RUNNING_GAP_MS: u64 = 250;

/// Where a download stands, as the pane's strip shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DownloadState {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl DownloadState {
    fn as_str(self) -> &'static str {
        match self {
            DownloadState::Running => "running",
            DownloadState::Done => "done",
            DownloadState::Failed => "failed",
            DownloadState::Cancelled => "cancelled",
        }
    }

    fn is_final(self) -> bool {
        self != DownloadState::Running
    }
}

/// What CEF's item says, reduced to what the event carries. Plain data so the
/// payload shape is tested without a CEF process.
pub(crate) struct DownloadSnapshot {
    pub state: DownloadState,
    /// The path the dialog returned (the user sees it, so it is sent whole;
    /// logs never carry it, §7.4).
    pub path: String,
    pub received: i64,
    /// -1 when the server didn't say.
    pub total: i64,
    /// Why it failed, in words (failed only).
    pub reason: Option<String>,
}

/// The file name part of a saved path, for the strip. Both separators, like
/// the sanitiser: a Windows path can reach a Linux build in a test.
fn leaf_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or("")
}

/// The `pane-download-update` payload (snake_case, like every other pane
/// event). `path` is set for a finished download only, `reason` for a failed
/// one only: the strip's Show in folder trusts the host's path alone.
pub(crate) fn update_payload(block_id: &str, id: u32, snap: &DownloadSnapshot) -> serde_json::Value {
    let mut v = serde_json::json!({
        "block_id": block_id,
        "download_id": id,
        "name": leaf_name(&snap.path),
        "state": snap.state.as_str(),
        "received": snap.received.max(0),
        "total": if snap.total > 0 { snap.total } else { -1 },
    });
    if snap.state == DownloadState::Done {
        v["path"] = serde_json::json!(snap.path);
    }
    if snap.state == DownloadState::Failed {
        v["reason"] = serde_json::json!(snap.reason.clone().unwrap_or_else(|| "The download failed.".into()));
    }
    v
}

/// The strip's line for a page that started more downloads than may wait
/// for a dialog (§7.2). Not tied to one download, so `download_id` is null.
pub(crate) fn ignored_payload(block_id: &str) -> serde_json::Value {
    serde_json::json!({
        "block_id": block_id,
        "download_id": serde_json::Value::Null,
        "state": "ignored",
        "reason": "This page tried to start more downloads. Ignored.",
    })
}

/// Keeps `running` events to `MIN_RUNNING_GAP_MS` apart per download; a
/// final state always passes and forgets the download. Plain data and a
/// caller-supplied clock, so it is tested directly.
#[derive(Default)]
pub(crate) struct ProgressThrottle {
    last_sent_ms: HashMap<(String, u32), u64>,
}

impl ProgressThrottle {
    pub(crate) fn allow(&mut self, pane: &str, id: u32, state: DownloadState, now_ms: u64) -> bool {
        let key = (pane.to_string(), id);
        if state.is_final() {
            self.last_sent_ms.remove(&key);
            return true;
        }
        match self.last_sent_ms.get(&key) {
            Some(&last) if now_ms.saturating_sub(last) < MIN_RUNNING_GAP_MS => false,
            _ => {
                self.last_sent_ms.insert(key, now_ms);
                true
            }
        }
    }

    /// The pane closed: its downloads' timers are not needed any more.
    pub(crate) fn clear_pane(&mut self, pane: &str) {
        self.last_sent_ms.retain(|(p, _), _| p != pane);
    }
}

static THROTTLE: Mutex<Option<ProgressThrottle>> = Mutex::new(None);

fn with_throttle<R>(f: impl FnOnce(&mut ProgressThrottle) -> R) -> R {
    f(THROTTLE.lock().get_or_insert_with(ProgressThrottle::default))
}

/// What CEF reports for an interrupted download, in words for the strip. The
/// reason is CEF's own and never page text, so it is safe to show.
fn failure_reason(reason: &str) -> String {
    let words = reason
        .trim_start_matches("CEF_DOWNLOAD_INTERRUPT_REASON_")
        .trim_start_matches("DOWNLOAD_INTERRUPT_REASON_");
    if words.is_empty() || words.eq_ignore_ascii_case("none") {
        return "The download failed.".to_string();
    }
    let spaced = words.replace('_', " ").to_ascii_lowercase();
    format!("The download failed: {spaced}.")
}

// ---------------------------------------------------------------------------
// Cancel (spec §5)
// ---------------------------------------------------------------------------

thread_local! {
    /// The `DownloadItemCallback` of every running download, by (pane, id).
    /// The callback is not `Send`, so it lives on the CEF UI thread and every
    /// access goes through a task posted there (the `media_prompt.rs` shape).
    static RUNNING: std::cell::RefCell<HashMap<(String, u32), DownloadItemCallback>> =
        std::cell::RefCell::new(HashMap::new());
}

wrap_task! {
    struct CancelTask {
        block_id: String,
        download_id: u32,
    }

    impl Task {
        fn execute(&self) {
            // Remove before cancelling: a repeat press finds nothing, and the
            // final update CEF sends for the cancel has nothing left to drop.
            let cb = RUNNING.with(|m| m.borrow_mut().remove(&(self.block_id.clone(), self.download_id)));
            if let Some(cb) = cb {
                tracing::info!(target: "pane-download", block_id = %self.block_id, id = self.download_id, "cancelled from the strip");
                cb.cancel();
            }
        }
    }
}

wrap_task! {
    struct ForgetPaneTask {
        block_id: String,
    }

    impl Task {
        fn execute(&self) {
            RUNNING.with(|m| m.borrow_mut().retain(|(p, _), _| p != &self.block_id));
            SAVE_DIALOGS.with(|d| d.borrow_mut().clear_pane(&self.block_id));
            ASKING.with(|m| m.borrow_mut().retain(|(p, _), _| p != &self.block_id));
        }
    }
}

/// Cancel a running download of a pane. A download this host no longer holds
/// (finished, or its pane closed) is a no-op, so a late press is harmless.
pub(crate) fn cancel_download(block_id: &str, download_id: u32) {
    let mut task = CancelTask::new(block_id.to_string(), download_id);
    if post_task(ThreadId::UI, Some(&mut task)) == 0 {
        tracing::warn!(target: "pane-download", %block_id, id = download_id, "could not post the cancel to the UI thread");
    }
}

// ---------------------------------------------------------------------------
// The CEF handler
// ---------------------------------------------------------------------------

/// What a log line may say about a URL: its origin. The full URL can carry
/// tokens (spec §7.4).
fn origin_of(url: &str) -> String {
    match url.find("://") {
        Some(i) => {
            let rest = &url[i + 3..];
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            format!("{}{}", &url[..i + 3], &rest[..end])
        }
        // `blob:` and `data:` carry their payload or id after the scheme.
        None => match url.split_once(':') {
            Some((scheme, _)) => format!("{scheme}:"),
            None => String::new(),
        },
    }
}

/// The extension of a file name, lower-cased, for logs (the name itself and
/// the full path contain the user's words and user name).
fn extension_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

wrap_download_handler! {
    pub(crate) struct AgentMuxDownloadHandler {
        state: Arc<AppState>,
    }

    impl DownloadHandler {
        fn can_download(
            &self,
            browser: Option<&mut Browser>,
            url: Option<&CefString>,
            request_method: Option<&CefString>,
        ) -> ::std::os::raw::c_int {
            let url = url.map(|s| s.to_string()).unwrap_or_default();
            let method = request_method.map(|s| s.to_string()).unwrap_or_default();
            // A download that can't be attributed to a live pane has nowhere
            // to show its dialog; refuse it rather than save it unseen.
            let Some(block_id) = browser.and_then(|b| self.state.block_id_for_browser(b)) else {
                tracing::info!(
                    target: "pane-download",
                    origin = %origin_of(&url),
                    "refusing download — not attributable to a live pane"
                );
                return 0;
            };
            if refuse_for_vault(&block_id) {
                tracing::info!(
                    target: "pane-download",
                    %block_id, origin = %origin_of(&url),
                    "refusing download — the pane's download goes to the Vault, never to disk"
                );
                return 0;
            }
            if !with_waiting(|w| w.has_room(&block_id)) {
                tracing::warn!(
                    target: "pane-download",
                    %block_id, origin = %origin_of(&url),
                    "refusing download — this pane already has {MAX_WAITING_PER_PANE} waiting for a Save As answer"
                );
                crate::events::emit_browser_pane_event(&self.state, &block_id, UPDATE_EVENT, &ignored_payload(&block_id));
                return 0;
            }
            tracing::info!(
                target: "pane-download",
                %block_id, %method, origin = %origin_of(&url),
                "download requested"
            );
            1
        }

        fn on_before_download(
            &self,
            browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(item), Some(cb)) = (download_item, callback) else {
                // Nothing to answer with: leave CEF's default (cancel).
                return 0;
            };
            let id = item.id();
            let suggested = suggested_name.map(|s| s.to_string()).unwrap_or_default();

            let Some(block_id) = browser.and_then(|b| self.state.block_id_for_browser(b)) else {
                tracing::info!(target: "pane-download", id, "dropping download — pane is gone");
                return 0;
            };
            // `can_download` already checked, but another download may have
            // slipped in between; the add is the check that counts.
            if !with_waiting(|w| w.add(&block_id, id)) {
                tracing::warn!(target: "pane-download", %block_id, id, "dropping download — too many waiting");
                crate::events::emit_browser_pane_event(&self.state, &block_id, UPDATE_EVENT, &ignored_payload(&block_id));
                return 0;
            }

            // One dialog at a time per pane: a second would cancel itself and the open one.
            let parked = (suggested.clone(), cb.clone());
            match SAVE_DIALOGS.with(|d| d.borrow_mut().offer(&block_id, id, parked)) {
                Some((suggested, cb)) => {
                    tracing::info!(
                        target: "pane-download",
                        %block_id, id, ext = %extension_of(&suggested),
                        "asking where to save"
                    );
                    show_save_dialog(&block_id, id, &suggested, &cb);
                }
                None => tracing::info!(
                    target: "pane-download",
                    %block_id, id,
                    "waiting for the pane's open Save As dialog"
                ),
            }
            1
        }

        fn on_download_updated(
            &self,
            browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            let Some(item) = download_item else { return };
            let id = item.id();
            let path = CefString::from(&item.full_path()).to_string();
            let complete = item.is_complete() != 0;
            let canceled = item.is_canceled() != 0;
            let interrupted = item.is_interrupted() != 0;
            let ended = complete || canceled || interrupted;
            // An answered dialog shows as a chosen path or as a final state.
            // Anything else (an update before the user has answered) leaves
            // the download counted as waiting.
            if path.is_empty() && !ended {
                return;
            }
            // Ids repeat across panes that use different request contexts, so
            // the pane comes from the browser, not from the id. A pane that is
            // already gone had its entries cleared when it closed.
            let block_id = browser.and_then(|b| self.state.block_id_for_browser(b));
            if let Some(block_id) = &block_id {
                with_waiting(|w| w.release(block_id, id));
                // This dialog is over: the pane's next parked download asks now.
                if let Some((nid, (suggested, cb))) = SAVE_DIALOGS.with(|d| d.borrow_mut().finished(block_id, id)) {
                    tracing::info!(target: "pane-download", %block_id, id = nid, ext = %extension_of(&suggested), "asking where to save");
                    show_save_dialog(block_id, nid, &suggested, &cb);
                }
            }
            if !path.is_empty() {
                remember_download_dir(Path::new(&path));
            }
            if ended {
                tracing::info!(
                    target: "pane-download",
                    id, complete, canceled, interrupted,
                    bytes = item.received_bytes(),
                    ext = %extension_of(&path),
                    "download ended"
                );
            }
            let Some(block_id) = block_id else { return };

            // Keep the callback while it runs so Cancel can reach it; drop it
            // at the end. (This handler runs on the UI thread, where `RUNNING` lives.)
            let key = (block_id.clone(), id);
            if ended {
                RUNNING.with(|m| m.borrow_mut().remove(&key));
            } else if let Some(cb) = callback {
                RUNNING.with(|m| m.borrow_mut().insert(key, cb.clone()));
            }

            let state = if complete {
                DownloadState::Done
            } else if canceled {
                DownloadState::Cancelled
            } else if interrupted {
                DownloadState::Failed
            } else {
                DownloadState::Running
            };
            if !with_throttle(|t| t.allow(&block_id, id, state, agentmux_common::time::now_ms_u64())) {
                return;
            }
            let snap = DownloadSnapshot {
                state,
                path,
                received: item.received_bytes(),
                total: item.total_bytes(),
                reason: (state == DownloadState::Failed)
                    .then(|| failure_reason(&format!("{:?}", item.interrupt_reason()))),
            };
            crate::events::emit_browser_pane_event(
                &self.state,
                &block_id,
                UPDATE_EVENT,
                &update_payload(&block_id, id, &snap),
            );
        }
    }
}

#[cfg(test)]
mod save_dialog_tests {
    use super::*;

    #[test]
    fn only_the_first_download_of_a_pane_asks_and_the_rest_wait_their_turn() {
        let mut q = SaveDialogs::<&str>::default();
        assert_eq!(q.offer("p", 5, "a"), Some("a"));
        assert_eq!(q.offer("p", 6, "b"), None);
        assert_eq!(q.offer("p", 7, "c"), None);
        // 5's dialog is answered: 6 asks next, then 7.
        assert_eq!(q.finished("p", 5), Some((6, "b")));
        assert_eq!(q.finished("p", 6), Some((7, "c")));
        assert_eq!(q.finished("p", 7), None);
        // Nothing is open any more: the next one asks at once.
        assert_eq!(q.offer("p", 8, "d"), Some("d"));
    }

    #[test]
    fn a_repeat_update_for_an_answered_dialog_does_not_open_another() {
        let mut q = SaveDialogs::<&str>::default();
        assert_eq!(q.offer("p", 1, "a"), Some("a"));
        assert_eq!(q.offer("p", 2, "b"), None);
        assert_eq!(q.finished("p", 1), Some((2, "b")));
        // 1 finishes later (its file is done): 2 is the open one, so nothing changes.
        assert_eq!(q.finished("p", 1), None);
        assert_eq!(q.offer("p", 3, "c"), None);
    }

    #[test]
    fn a_parked_download_that_ends_before_its_turn_is_dropped() {
        let mut q = SaveDialogs::<&str>::default();
        q.offer("p", 1, "a");
        q.offer("p", 2, "b");
        q.offer("p", 3, "c");
        assert_eq!(q.finished("p", 2), None);
        assert_eq!(q.finished("p", 1), Some((3, "c")));
    }

    #[test]
    fn panes_do_not_wait_on_each_other_and_a_closed_pane_clears() {
        let mut q = SaveDialogs::<&str>::default();
        assert_eq!(q.offer("p", 1, "a"), Some("a"));
        // The same id in another pane (another request context) is its own dialog.
        assert_eq!(q.offer("q", 1, "x"), Some("x"));
        assert_eq!(q.offer("p", 2, "b"), None);
        q.clear_pane("p");
        assert_eq!(q.finished("p", 1), None);
        assert_eq!(q.offer("p", 3, "c"), Some("c"));
        assert_eq!(q.finished("q", 1), None);
    }
}

#[cfg(test)]
mod vault_hold_tests {
    use super::*;

    #[test]
    fn a_held_pane_refuses_its_downloads_and_counts_them_until_released() {
        let hold = hold_for_vault("pane-vault-1").unwrap();
        assert!(hold_for_vault("pane-vault-1").is_none(), "one capture at a time per pane");
        assert!(!refuse_for_vault("pane-vault-2"), "another pane downloads as usual");
        assert!(refuse_for_vault("pane-vault-1") && refuse_for_vault("pane-vault-1"));
        assert_eq!(hold.refused(), 2);
        drop(hold);
        assert!(!refuse_for_vault("pane-vault-1"));
        assert!(hold_for_vault("pane-vault-1").is_some());
    }
}

/// The pane's download commands from the frontend. Cancel is the strip's: the host holds the callback, so the
/// page and the strip name only a pane and a download id, and an id the host no longer holds does nothing
/// (spec §5). Start is the right-click menu's "Save link as…" / "Save image as…" (`start_download` checks the
/// address, which is the page's text).
pub fn run_command(state: &std::sync::Arc<crate::state::AppState>, cmd: &str, args: &serde_json::Value) {
    let block_id = args.get("block_id").and_then(|v| v.as_str()).unwrap_or("");
    match cmd {
        "browser_download_cancel" => {
            let id = args.get("download_id").and_then(|v| v.as_u64()).and_then(|v| u32::try_from(v).ok());
            if let Some(id) = id {
                cancel_download(block_id, id);
            }
        }
        "browser_download_start" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            state.browser_panes.start_download(block_id, url, state);
        }
        _ => {}
    }
}

/// The most a link's or image's address can be for "Save link as…" / "Save image as…" (a `data:` image can be long).
const MAX_SAVE_URL_BYTES: usize = 2 * 1024 * 1024;

/// Whether a page's link or image address may be saved from the right-click menu. The menu hands the host
/// text the page chose, so only the schemes a page can already show a person go further: not `file:`,
/// `javascript:`, `chrome:` or anything that opens another program.
pub fn download_url_allowed(url: &str) -> bool {
    if url.len() > MAX_SAVE_URL_BYTES || url.chars().any(char::is_control) {
        return false;
    }
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    ["http://", "https://", "blob:", "data:"].iter().any(|scheme| lower.starts_with(scheme))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_schemes_a_page_can_show_a_person_can_be_saved_from_the_menu() {
        for ok in ["http://example.com/a.pdf", "https://example.com/p.png?x=1", "HTTPS://EXAMPLE.COM/", "blob:https://example.com/0f1e", "data:image/png;base64,iVBOR"] {
            assert!(download_url_allowed(ok), "{ok}");
        }
        for bad in [
            "",
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "chrome://settings",
            "view-source:https://example.com/",
            "ftp://example.com/a",
            "ms-msdt:/id",
            "//example.com/a",
            "https//example.com",
            "https://example.com/a\nb",
            "https://example.com/\u{0}",
        ] {
            assert!(!download_url_allowed(bad), "{bad:?}");
        }
    }

    #[test]
    fn an_address_past_the_cap_is_refused() {
        let long = format!("data:image/png;base64,{}", "A".repeat(MAX_SAVE_URL_BYTES));
        assert!(!download_url_allowed(&long));
        assert!(download_url_allowed(&format!("https://example.com/{}", "a".repeat(2000))));
    }

    use super::*;

    #[test]
    fn a_plain_name_is_kept() {
        assert_eq!(safe_download_name("report.pdf"), "report.pdf");
        assert_eq!(
            safe_download_name("My report (final).pdf"),
            "My report (final).pdf"
        );
        assert_eq!(safe_download_name("x.tar.gz"), "x.tar.gz");
        assert_eq!(safe_download_name(".bashrc"), ".bashrc");
        assert_eq!(safe_download_name("ünïcode名前.txt"), "ünïcode名前.txt");
    }

    #[test]
    fn only_the_file_name_part_survives() {
        assert_eq!(safe_download_name("..\\..\\x.exe"), "x.exe");
        assert_eq!(safe_download_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_download_name("a/b.txt"), "b.txt");
        assert_eq!(safe_download_name("C:\\Windows\\x.dll"), "x.dll");
        assert_eq!(safe_download_name("C:x.dll"), "Cx.dll");
        assert_eq!(safe_download_name("dir/"), FALLBACK_NAME);
    }

    #[test]
    fn illegal_characters_and_trailing_dots_go() {
        assert_eq!(safe_download_name("a<b>c:d\"e|f?g*h.txt"), "abcdefgh.txt");
        assert_eq!(safe_download_name("tab\there\nnew.txt"), "tabherenew.txt");
        assert_eq!(safe_download_name("name. . "), "name");
        assert_eq!(safe_download_name("  lead.txt"), "lead.txt");
    }

    #[test]
    fn nothing_usable_falls_back() {
        for raw in ["", " ", "...", ". .", "<>:\"|?*", "\u{7}", "/", "\\"] {
            assert_eq!(safe_download_name(raw), FALLBACK_NAME, "{raw:?}");
        }
    }

    #[test]
    fn windows_device_names_are_prefixed() {
        assert_eq!(safe_download_name("CON"), "_CON");
        assert_eq!(safe_download_name("nul.txt"), "_nul.txt");
        assert_eq!(safe_download_name("Com1.tar.gz"), "_Com1.tar.gz");
        assert_eq!(safe_download_name("LPT9"), "_LPT9");
        // Only the stem counts: a longer name that starts the same is fine.
        assert_eq!(safe_download_name("console.log"), "console.log");
        assert_eq!(safe_download_name("COM10.txt"), "COM10.txt");
    }

    #[test]
    fn long_names_are_capped_and_keep_their_extension() {
        let long = format!("{}.pdf", "a".repeat(300));
        let out = safe_download_name(&long);
        assert_eq!(out.chars().count(), MAX_NAME_CHARS);
        assert!(out.ends_with(".pdf"));

        let no_ext = "b".repeat(300);
        assert_eq!(safe_download_name(&no_ext).chars().count(), MAX_NAME_CHARS);

        // A "extension" that is really most of the name is not kept whole.
        let odd = format!("x.{}", "c".repeat(300));
        assert_eq!(safe_download_name(&odd).chars().count(), MAX_NAME_CHARS);

        // Multi-byte characters are counted as characters, never cut mid-way.
        let wide = format!("{}.txt", "名".repeat(300));
        let out = safe_download_name(&wide);
        assert_eq!(out.chars().count(), MAX_NAME_CHARS);
        assert!(out.ends_with(".txt"));
    }

    #[test]
    fn the_dialog_opens_in_the_first_folder_that_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("deleted");

        // The remembered folder wins when it exists.
        assert_eq!(
            default_save_dir(Some(tmp.path().into()), Some(other.path().into()), None),
            Some(tmp.path().to_path_buf())
        );
        // A deleted remembered folder falls back to Downloads, and nothing is created.
        assert_eq!(
            default_save_dir(Some(gone.clone()), Some(other.path().into()), None),
            Some(other.path().to_path_buf())
        );
        assert!(!gone.exists());
        // Then to the home folder.
        assert_eq!(
            default_save_dir(None, Some(gone), Some(tmp.path().into())),
            Some(tmp.path().to_path_buf())
        );
        assert_eq!(default_save_dir(None, None, None), None);
    }

    #[test]
    fn a_saved_path_is_remembered_by_its_folder() {
        let tmp = tempfile::tempdir().unwrap();
        remember_download_dir(&tmp.path().join("report.pdf"));
        assert_eq!(
            LAST_DOWNLOAD_DIR.lock().clone(),
            Some(tmp.path().to_path_buf())
        );
        // A bare name has no folder to remember and leaves the last one alone.
        remember_download_dir(Path::new("report.pdf"));
        assert_eq!(
            LAST_DOWNLOAD_DIR.lock().clone(),
            Some(tmp.path().to_path_buf())
        );
    }

    #[test]
    fn a_pane_may_have_three_waiting_and_not_a_fourth() {
        let mut w = WaitingDownloads::default();
        assert!(w.add("p1", 1));
        assert!(w.add("p1", 2));
        assert!(w.add("p1", 3));
        assert!(!w.has_room("p1"));
        assert!(!w.add("p1", 4), "the fourth is refused");
        // Another pane is not affected.
        assert!(w.add("p2", 5));
    }

    #[test]
    fn answering_a_dialog_frees_a_slot_once() {
        let mut w = WaitingDownloads::default();
        for id in 1..=3 {
            assert!(w.add("p1", id));
        }
        assert!(w.release("p1", 2));
        assert!(w.has_room("p1"));
        // A repeat update for the same download frees nothing more.
        assert!(!w.release("p1", 2));
        assert!(!w.release("p1", 2));
        assert!(w.add("p1", 4));
        assert!(!w.has_room("p1"));
        // An id that was never waiting changes nothing.
        assert!(!w.release("p1", 99));
        assert!(!w.has_room("p1"));
    }

    #[test]
    fn the_same_id_in_two_panes_is_two_downloads() {
        // Panes in different request contexts count ids separately, so both
        // can have a download "1" waiting; one finishing frees only its own.
        let mut w = WaitingDownloads::default();
        for id in 1..=3 {
            assert!(w.add("p1", id));
        }
        assert!(w.add("p2", 1));
        assert!(w.release("p2", 1));
        assert!(!w.has_room("p1"), "p2's update must not free a p1 slot");
        assert!(!w.release("p2", 2), "p1's id 2 is not p2's");
        assert!(w.release("p1", 1));
        assert!(w.has_room("p1"));
    }

    #[test]
    fn closing_a_pane_clears_only_that_pane() {
        let mut w = WaitingDownloads::default();
        for id in 1..=3 {
            assert!(w.add("p1", id));
        }
        assert!(w.add("p2", 7));
        w.clear_pane("p1");
        assert!(w.has_room("p1"));
        assert!(
            w.release("p2", 7),
            "the other pane's download is still waiting"
        );
        assert!(!w.release("p1", 1), "the closed pane's are gone");
        w.clear_pane("never-had-one");
    }

    #[test]
    fn logs_carry_the_origin_and_never_the_rest_of_the_url() {
        assert_eq!(
            origin_of("https://example.com/a/b?token=secret#x"),
            "https://example.com"
        );
        assert_eq!(
            origin_of("https://example.com?token=secret"),
            "https://example.com"
        );
        assert_eq!(
            origin_of("http://127.0.0.1:8080/file.bin"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            origin_of("blob:https://example.com/0a1b2c"),
            "blob:https://example.com"
        );
        assert_eq!(origin_of("data:text/plain;base64,AAAA"), "data:");
        assert_eq!(origin_of(""), "");
    }

    #[test]
    fn extensions_are_lowercased_and_optional() {
        assert_eq!(extension_of("Report.PDF"), "pdf");
        assert_eq!(extension_of("x.tar.gz"), "gz");
        assert_eq!(extension_of("noext"), "");
        assert_eq!(extension_of(""), "");
    }

    fn snap(state: DownloadState) -> DownloadSnapshot {
        DownloadSnapshot {
            state,
            path: r"C:\Users\me\Downloads\report.pdf".into(),
            received: 4_200_000,
            total: 18_000_000,
            reason: None,
        }
    }

    #[test]
    fn a_running_update_has_the_golden_shape() {
        assert_eq!(
            update_payload("blk", 7, &snap(DownloadState::Running)),
            serde_json::json!({
                "block_id": "blk",
                "download_id": 7,
                "name": "report.pdf",
                "state": "running",
                "received": 4_200_000,
                "total": 18_000_000,
            })
        );
    }

    #[test]
    fn only_a_finished_download_carries_a_path_and_only_a_failed_one_a_reason() {
        let done = update_payload("blk", 7, &snap(DownloadState::Done));
        assert_eq!(done["path"], r"C:\Users\me\Downloads\report.pdf");
        assert!(done.get("reason").is_none());

        let mut failed = snap(DownloadState::Failed);
        failed.reason = Some("The download failed: disk full.".into());
        let failed = update_payload("blk", 7, &failed);
        assert_eq!(failed["reason"], "The download failed: disk full.");
        assert!(failed.get("path").is_none());

        let cancelled = update_payload("blk", 7, &snap(DownloadState::Cancelled));
        assert!(cancelled.get("path").is_none() && cancelled.get("reason").is_none());
    }

    #[test]
    fn an_unknown_total_is_minus_one_and_received_never_negative() {
        let mut s = snap(DownloadState::Running);
        s.total = 0;
        s.received = -1;
        let v = update_payload("blk", 1, &s);
        assert_eq!(v["total"], -1);
        assert_eq!(v["received"], 0);
        s.total = -1;
        assert_eq!(update_payload("blk", 1, &s)["total"], -1);
    }

    #[test]
    fn the_flood_line_is_not_tied_to_a_download() {
        let v = ignored_payload("blk");
        assert_eq!(v["state"], "ignored");
        assert!(v["download_id"].is_null());
        assert_eq!(v["reason"], "This page tried to start more downloads. Ignored.");
    }

    #[test]
    fn running_updates_are_at_most_four_a_second_and_final_ones_always_pass() {
        let mut t = ProgressThrottle::default();
        let r = DownloadState::Running;
        assert!(t.allow("p", 1, r, 1_000));
        assert!(!t.allow("p", 1, r, 1_100));
        assert!(!t.allow("p", 1, r, 1_249));
        assert!(t.allow("p", 1, r, 1_250));
        // Another download, or another pane's same id, has its own clock.
        assert!(t.allow("p", 2, r, 1_260));
        assert!(t.allow("q", 1, r, 1_260));
        // A final state passes however soon it follows, and forgets the timer.
        assert!(t.allow("p", 1, DownloadState::Done, 1_251));
        assert!(t.allow("p", 1, DownloadState::Cancelled, 1_252));
        assert!(t.allow("p", 1, r, 1_253), "a finished id starts fresh");
    }

    #[test]
    fn closing_a_pane_forgets_its_timers() {
        let mut t = ProgressThrottle::default();
        assert!(t.allow("p", 1, DownloadState::Running, 1_000));
        assert!(t.allow("q", 1, DownloadState::Running, 1_000));
        t.clear_pane("p");
        assert!(t.allow("p", 1, DownloadState::Running, 1_001));
        assert!(!t.allow("q", 1, DownloadState::Running, 1_001));
    }

    #[test]
    fn a_failure_is_said_in_words() {
        assert_eq!(failure_reason("None"), "The download failed.");
        assert_eq!(failure_reason(""), "The download failed.");
        assert_eq!(
            failure_reason("CEF_DOWNLOAD_INTERRUPT_REASON_FILE_NO_SPACE"),
            "The download failed: file no space."
        );
        assert_eq!(
            failure_reason("DOWNLOAD_INTERRUPT_REASON_NETWORK_TIMEOUT"),
            "The download failed: network timeout."
        );
    }

    #[test]
    fn the_strip_shows_the_file_name_only() {
        assert_eq!(leaf_name(r"C:\Users\me\x.zip"), "x.zip");
        assert_eq!(leaf_name("/home/me/x.zip"), "x.zip");
        assert_eq!(leaf_name(""), "");
    }
}
