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

use std::collections::{HashMap, HashSet};
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

    /// The dialog for `id` was answered (or the download ended). A repeat for
    /// the same id, or an id never added, changes nothing.
    pub(crate) fn release(&mut self, id: u32) {
        self.by_pane.retain(|_, ids| {
            ids.remove(&id);
            !ids.is_empty()
        });
    }

    /// The pane closed: nothing of its can still be waiting.
    pub(crate) fn clear_pane(&mut self, pane: &str) {
        self.by_pane.remove(pane);
    }

    /// The pane a waiting download belongs to.
    fn pane_of(&self, id: u32) -> Option<&str> {
        self.by_pane
            .iter()
            .find(|(_, ids)| ids.contains(&id))
            .map(|(p, _)| p.as_str())
    }
}

static WAITING: Mutex<Option<WaitingDownloads>> = Mutex::new(None);

fn with_waiting<R>(f: impl FnOnce(&mut WaitingDownloads) -> R) -> R {
    f(WAITING.lock().get_or_insert_with(WaitingDownloads::default))
}

/// A pane closed: drop what it had waiting. Safe from any thread, and for a
/// pane that never had a download.
pub(crate) fn forget_pane(block_id: &str) {
    with_waiting(|w| w.clear_pane(block_id));
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
            if !with_waiting(|w| w.has_room(&block_id)) {
                tracing::warn!(
                    target: "pane-download",
                    %block_id, origin = %origin_of(&url),
                    "refusing download — this pane already has {MAX_WAITING_PER_PANE} waiting for a Save As answer"
                );
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
                return 0;
            }

            let path = default_save_path(&suggested);
            tracing::info!(
                target: "pane-download",
                %block_id, id, ext = %extension_of(&suggested),
                "asking where to save"
            );
            // show_dialog = true: CEF opens the platform's Save As dialog,
            // starting at `path`, and cancels the download if the user cancels.
            cb.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 1);
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            _callback: Option<&mut DownloadItemCallback>,
        ) {
            let Some(item) = download_item else { return };
            let id = item.id();
            let path = CefString::from(&item.full_path()).to_string();
            let ended = item.is_complete() != 0 || item.is_canceled() != 0 || item.is_interrupted() != 0;
            // An answered dialog shows as a chosen path or as a final state.
            // Anything else (an update before the user has answered) leaves
            // the download counted as waiting.
            if path.is_empty() && !ended {
                return;
            }
            let pane = with_waiting(|w| {
                let pane = w.pane_of(id).map(str::to_string);
                w.release(id);
                pane
            });
            if !path.is_empty() {
                remember_download_dir(Path::new(&path));
            }
            if ended && pane.is_some() {
                tracing::info!(
                    target: "pane-download",
                    id,
                    complete = item.is_complete() != 0,
                    canceled = item.is_canceled() != 0,
                    interrupted = item.is_interrupted() != 0,
                    bytes = item.received_bytes(),
                    ext = %extension_of(&path),
                    "download ended"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
        w.release(2);
        assert!(w.has_room("p1"));
        // A repeat update for the same download frees nothing more.
        w.release(2);
        w.release(2);
        assert!(w.add("p1", 4));
        assert!(!w.has_room("p1"));
        // An id that was never waiting changes nothing.
        w.release(99);
        assert!(!w.has_room("p1"));
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
        assert_eq!(w.pane_of(7), Some("p2"));
        assert_eq!(w.pane_of(1), None);
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
}
