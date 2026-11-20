// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a person's pane download goes without the Save As dialog, and the folder the dialog starts in.
//!
//! Spec: `docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md` §4.4 and D3.
//!
//! * The setting `browser:askdownloadlocation` (default on, as asked for in the spec): while it is on, every
//!   download opens the dialog. Turned off, a download is saved straight into the Downloads folder under a
//!   name that doesn't replace a file. The page still never chooses the folder or the name's directory part
//!   (§7.1), a pane may run only a few at a time, and the strip shows each one as before.
//! * The last folder a Save As answered with is kept in a one-line file in the instance's data home, so the
//!   dialog opens there after a restart too. It is only the dialog's starting folder: the person still
//!   confirms where a file goes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cef::*;

use super::agent_downloads::free_path;
use super::downloads::{default_save_dir, ignored_payload, safe_download_name, UPDATE_EVENT};
use crate::state::AppState;

/// The setting that turns the Save As dialog on or off.
const ASK_KEY: &str = "browser:askdownloadlocation";

/// How many downloads one pane may have running without a dialog. With no dialog there is no consent step, so
/// a page that starts downloads on its own is held to the same few as the dialog's waiting limit.
const MAX_QUIET_PER_PANE: usize = 3;

/// The file in the data home that holds the last folder.
const LAST_DIR_FILE: &str = "last-download-dir.txt";

/// Whether a download asks where to save. On unless the setting says `false`.
fn ask_where_to_save() -> bool {
    crate::app::window_settings::read_window_bool_setting(ASK_KEY, true)
}

/// What to do with one download while the dialog is off.
#[derive(Debug, PartialEq)]
enum Quiet {
    /// Save it here.
    Save(PathBuf),
    /// The pane already has its share running: refuse this one.
    TooMany,
    /// No usable folder, or no free name: ask as usual rather than lose the file.
    Ask,
}

/// The decision, over plain data so it is tested without CEF. `running_here` counts the pane's quiet downloads
/// in flight, `reserved` is every file one of them is writing (CEF has not made it yet, so two downloads of one
/// name still take two names), `dir` is the folder a download goes into.
fn plan(running_here: usize, reserved: &[PathBuf], dir: Option<PathBuf>, suggested: &str) -> Quiet {
    if running_here >= MAX_QUIET_PER_PANE {
        return Quiet::TooMany;
    }
    let Some(dir) = dir else { return Quiet::Ask };
    match free_path(&dir, &safe_download_name(suggested), reserved) {
        Some(path) => Quiet::Save(path),
        None => Quiet::Ask,
    }
}

thread_local! {
    /// The files the quiet downloads in flight are writing, by (pane, id). CEF calls the download handler on the
    /// UI thread only. Ids repeat across panes that use different request contexts, so the pane is part of the key.
    static QUIET: RefCell<HashMap<(String, u32), PathBuf>> = RefCell::new(HashMap::new());
}

fn decide(pane: &str, id: u32, suggested: &str) -> Quiet {
    let dir = default_save_dir(None, dirs::download_dir(), dirs::home_dir());
    QUIET.with(|m| {
        let mut m = m.borrow_mut();
        let running = m.keys().filter(|(p, _)| p == pane).count();
        let reserved: Vec<PathBuf> = m.values().cloned().collect();
        let decided = plan(running, &reserved, dir, suggested);
        if let Quiet::Save(path) = &decided {
            m.insert((pane.to_string(), id), path.clone());
        }
        decided
    })
}

/// Whether this download is being saved without a dialog.
pub(crate) fn is_quiet(pane: &str, id: u32) -> bool {
    QUIET.with(|m| m.borrow().contains_key(&(pane.to_string(), id)))
}

/// The download ended (done, failed or cancelled): its file name and its slot are free.
pub(crate) fn ended(pane: &str, id: u32) {
    QUIET.with(|m| m.borrow_mut().remove(&(pane.to_string(), id)));
}

/// The pane closed: none of its downloads is still running here.
pub(crate) fn forget_pane(pane: &str) {
    QUIET.with(|m| m.borrow_mut().retain(|(p, _), _| p != pane));
}

/// Save the download without the dialog when the setting is off. Returns whether it was answered (saved, or
/// refused as one too many); `false` leaves it to the dialog.
pub(crate) fn answer_quietly(
    state: &Arc<AppState>,
    block_id: &str,
    id: u32,
    suggested: &str,
    cb: &BeforeDownloadCallback,
) -> bool {
    if ask_where_to_save() {
        return false;
    }
    match decide(block_id, id, suggested) {
        Quiet::Save(path) => {
            tracing::info!(target: "pane-download", %block_id, id, "saving without asking: the setting is off");
            cb.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 0);
            true
        }
        Quiet::TooMany => {
            tracing::warn!(target: "pane-download", %block_id, id, "dropping download: this pane already has {MAX_QUIET_PER_PANE} running");
            super::downloads::note_skipped(block_id, id);
            crate::events::emit_browser_pane_event(state, block_id, UPDATE_EVENT, &ignored_payload(block_id));
            // `cont` is never called, so CEF cancels the download.
            true
        }
        Quiet::Ask => false,
    }
}

// ---------------------------------------------------------------------------
// The last folder, kept across restarts
// ---------------------------------------------------------------------------

/// The folder in `data_dir`'s file, when it is an absolute path.
fn read_last_dir(data_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(data_dir.join(LAST_DIR_FILE)).ok()?;
    let line = text.lines().next()?.trim();
    let dir = PathBuf::from(line);
    (!line.is_empty() && dir.is_absolute()).then_some(dir)
}

/// Write the folder down. A failure is only logged: the folder is a convenience.
pub(crate) fn save_last_dir(data_dir: &Path, dir: &Path) {
    if let Err(e) = std::fs::write(data_dir.join(LAST_DIR_FILE), dir.to_string_lossy().as_bytes()) {
        tracing::warn!(target: "pane-download", "couldn't keep the last download folder: {e}");
    }
}

/// Load the kept folder the first time a download asks for one, if nothing newer is known.
pub(crate) fn load_last_dir_once(data_dir: Option<&str>) {
    static LOADED: std::sync::Once = std::sync::Once::new();
    LOADED.call_once(|| {
        if let Some(dir) = data_dir.filter(|d| !d.is_empty()).and_then(|d| read_last_dir(Path::new(d))) {
            super::downloads::seed_last_download_dir(dir);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fourth_quiet_download_of_a_pane_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Some(tmp.path().to_path_buf());
        assert!(matches!(plan(2, &[], dir.clone(), "a.zip"), Quiet::Save(_)));
        assert_eq!(plan(3, &[], dir, "a.zip"), Quiet::TooMany);
    }

    #[test]
    fn no_folder_asks_as_usual() {
        assert_eq!(plan(0, &[], None, "a.zip"), Quiet::Ask);
    }

    #[test]
    fn a_file_is_never_replaced_and_a_name_in_flight_is_taken() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::write(dir.join("report.pdf"), b"x").unwrap();
        assert_eq!(plan(0, &[], Some(dir.clone()), "report.pdf"), Quiet::Save(dir.join("report (1).pdf")));
        // The first of these is still being written, so it isn't on disk yet but is not free either.
        let reserved = vec![dir.join("report (1).pdf")];
        assert_eq!(plan(1, &reserved, Some(dir.clone()), "report.pdf"), Quiet::Save(dir.join("report (2).pdf")));
    }

    #[test]
    fn the_page_cannot_choose_a_folder_through_the_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let Quiet::Save(path) = plan(0, &[], Some(dir.clone()), r"..\..\evil/x.exe") else {
            panic!("saved");
        };
        assert_eq!(path.parent(), Some(dir.as_path()));
    }

    #[test]
    fn a_quiet_download_holds_its_slot_until_it_ends() {
        forget_pane("quiet-p");
        let first = decide("quiet-p", 1, "a.bin");
        let second = decide("quiet-p", 2, "a.bin");
        if let (Quiet::Save(a), Quiet::Save(b)) = (&first, &second) {
            assert_ne!(a, b, "two downloads of one name get two names");
        }
        ended("quiet-p", 1);
        ended("quiet-p", 2);
        forget_pane("quiet-p");
        QUIET.with(|m| assert!(m.borrow().keys().all(|(p, _)| p != "quiet-p")));
    }

    #[test]
    fn the_last_folder_round_trips_through_its_file() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(read_last_dir(data.path()), None);
        save_last_dir(data.path(), folder.path());
        assert_eq!(read_last_dir(data.path()), Some(folder.path().to_path_buf()));
    }

    #[test]
    fn a_relative_or_empty_kept_folder_is_ignored() {
        let data = tempfile::tempdir().unwrap();
        std::fs::write(data.path().join(LAST_DIR_FILE), "relative/dir").unwrap();
        assert_eq!(read_last_dir(data.path()), None);
        std::fs::write(data.path().join(LAST_DIR_FILE), "\n").unwrap();
        assert_eq!(read_last_dir(data.path()), None);
    }
}
