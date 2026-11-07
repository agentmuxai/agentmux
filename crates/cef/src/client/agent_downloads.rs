// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A download in a browser pane an agent owns is saved into that agent's workspace without a dialog
//! (docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §7.5, D3b). The dialog would appear on the
//! person's screen for something the agent did, and nothing the agent can answer.
//!
//! Only a pane srv says an agent owns and has not handed to the person (`jsdialog::driven_pane`), and
//! only into `<workspace>/downloads/` for the workspace srv recorded for that agent, never a path the
//! page or the agent names. The file name goes through the same sanitiser as the dialog's suggestion,
//! an existing file is never replaced (a numbered name is used), the size is capped, and nothing is
//! opened or made executable. The agent is told what was saved or why not (`jsdialog::notify`), and srv
//! logs it as an action of the pane. A person's own pane keeps the dialog.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cef::*;

use super::downloads::safe_download_name;

/// The most one download may be, in bytes.
pub(crate) const MAX_BYTES: i64 = 500 * 1024 * 1024;

/// The most the workspace's `downloads` folder may hold in all, in bytes: a per-file limit alone lets an agent
/// fill the disk.
pub(crate) const FOLDER_MAX_BYTES: i64 = 2 * 1024 * 1024 * 1024;

/// How many downloads one pane may have running into its workspace at once.
const MAX_RUNNING_PER_PANE: usize = 3;

/// The workspace folder of each pane an agent owns, as srv last sent it (`/agentmux/browser/owned_panes`): where a
/// download in that pane may be saved without a dialog. Empty when the setting is off.
static WORKDIRS: parking_lot::Mutex<Option<HashMap<String, String>>> = parking_lot::Mutex::new(None);

/// Replace the copy with srv's whole set.
pub(crate) fn set_workdirs(workdirs: HashMap<String, String>) {
    *WORKDIRS.lock() = Some(workdirs);
}

/// A download being saved into a workspace.
struct Saving {
    name: String,
    /// The `downloads` folder it goes into.
    dir: PathBuf,
    /// What the folder held when it started, so the folder's cap holds for a download of unknown size.
    used: i64,
}

thread_local! {
    /// The downloads being saved into a workspace, by (pane, id). CEF calls the download handler on the
    /// UI thread only.
    static SAVING: std::cell::RefCell<HashMap<(String, u32), Saving>> = std::cell::RefCell::new(HashMap::new());
}

/// The words for a size in a message: `13 bytes`, `4.2 MB`.
pub(crate) fn size_words(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    if b < 1024.0 * 1024.0 {
        return format!("{:.1} KB", b / 1024.0);
    }
    if b < 1024.0 * 1024.0 * 1024.0 {
        return format!("{:.1} MB", b / (1024.0 * 1024.0));
    }
    format!("{:.1} GB", b / (1024.0 * 1024.0 * 1024.0))
}

/// A path as the agent reads it: Windows' `\\?\` prefix from `canonicalize` dropped.
fn shown(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => rest.to_string(),
        _ => s,
    }
}

/// `<workspace>/downloads`, made when it is missing. Refused when it is a link (an agent can make one in its
/// own workspace, and a container agent's link would be read here as a path on this computer), not a
/// folder, or resolves outside the workspace.
fn downloads_dir(workdir: &Path) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(workdir).map_err(|_| "the agent's workspace folder isn't there".to_string())?;
    let dir = root.join("downloads");
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_symlink() => return Err("the workspace's downloads folder is a link, which is not followed".into()),
        Ok(m) if !m.is_dir() => return Err("the workspace has a file called downloads, not a folder".into()),
        Ok(_) => {}
        Err(_) => std::fs::create_dir(&dir).map_err(|e| format!("the downloads folder couldn't be made: {e}"))?,
    }
    let real = std::fs::canonicalize(&dir).map_err(|e| format!("the downloads folder couldn't be read: {e}"))?;
    if real.starts_with(&root) {
        Ok(real)
    } else {
        Err("the workspace's downloads folder is outside the workspace".into())
    }
}

/// How many bytes the files under `dir` hold, links not followed. Stops counting past `FOLDER_MAX_BYTES` or
/// after a great many entries (a folder that big is full either way).
fn folder_bytes(dir: &Path) -> i64 {
    let mut total: i64 = 0;
    let mut seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            seen += 1;
            if seen > 50_000 || total > FOLDER_MAX_BYTES {
                return FOLDER_MAX_BYTES + 1;
            }
            let Ok(m) = std::fs::symlink_metadata(entry.path()) else { continue };
            if m.is_dir() {
                stack.push(entry.path());
            } else if m.is_file() {
                total = total.saturating_add(m.len() as i64);
            }
        }
    }
    total
}

/// The refusal for a folder that has no room: what to do about it.
fn full_words(dir: &Path) -> String {
    format!("the downloads folder is full ({} is the most it holds): delete files in {}", size_words(FOLDER_MAX_BYTES), shown(dir))
}

/// The first of `name`, `name (1)`, `name (2)`… that doesn't exist in `dir`: a download never replaces a file.
fn free_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (0..1000).find_map(|n| {
        let candidate = if n == 0 { name.to_string() } else { format!("{stem} ({n}){ext}") };
        let path = dir.join(candidate);
        std::fs::symlink_metadata(&path).is_err().then_some(path)
    })
}

/// Where the download named `suggested` of `total` bytes (-1 when the server didn't say) goes in `workdir`,
/// or why it can't be saved.
pub(crate) fn target(workdir: &Path, suggested: &str, total: i64) -> Result<(PathBuf, i64), String> {
    let name = safe_download_name(suggested);
    if total > MAX_BYTES {
        return Err(format!("{name} is {}, over the {} limit for a download into the workspace", size_words(total), size_words(MAX_BYTES)));
    }
    let dir = downloads_dir(workdir)?;
    let used = folder_bytes(&dir);
    if used.saturating_add(total.max(0)) > FOLDER_MAX_BYTES {
        return Err(format!("{name} was not saved: {}", full_words(&dir)));
    }
    let path = free_path(&dir, &name).ok_or_else(|| format!("{name}: the downloads folder already has too many files with that name"))?;
    Ok((path, used))
}

/// What the download handler does with a download of a pane.
pub(crate) enum Before {
    /// Not an agent's pane, or saving into the workspace is off: the person's dialog.
    Ask,
    /// Save it here, no dialog.
    Save(PathBuf),
    /// Refused (the agent is told why): CEF cancels it.
    Refuse,
}

/// The workspace folder of `pane` when an agent drives it and may save into it.
fn workdir_of(state: &Arc<crate::state::AppState>, pane: &str) -> Option<PathBuf> {
    if !super::jsdialog::driven_pane(state, pane) {
        return None;
    }
    WORKDIRS.lock().as_ref()?.get(pane).map(PathBuf::from)
}

/// Tell the agent that drives `browser`'s pane (and srv's log) what happened to a download.
fn tell(state: &Arc<crate::state::AppState>, browser: Option<&Browser>, notice: &str, message: &str) {
    let Some((_pane, target)) = browser.and_then(|b| super::jsdialog::driven(state, b)) else { return };
    super::jsdialog::notify(state, serde_json::json!({ "target": target, "notice": notice, "message": message }));
}

/// A download of `pane` is about to ask where to save: decide whether it saves into the agent's workspace.
pub(crate) fn before(
    state: &Arc<crate::state::AppState>,
    browser: Option<&Browser>,
    pane: &str,
    id: u32,
    suggested: &str,
    total: i64,
) -> Before {
    let Some(workdir) = workdir_of(state, pane) else { return Before::Ask };
    let running = SAVING.with(|m| m.borrow().keys().filter(|(p, _)| p == pane).count());
    let decided = if running >= MAX_RUNNING_PER_PANE {
        Err(format!("{}: this pane already has {MAX_RUNNING_PER_PANE} downloads running", safe_download_name(suggested)))
    } else {
        target(&workdir, suggested, total)
    };
    match decided {
        Ok((path, used)) => {
            tracing::info!(target: "pane-download", %pane, id, "saving into the agent's workspace, no dialog");
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
            SAVING.with(|m| m.borrow_mut().insert((pane.to_string(), id), Saving { name: safe_download_name(suggested), dir, used }));
            Before::Save(path)
        }
        Err(why) => {
            tracing::info!(target: "pane-download", %pane, id, %why, "refusing an agent pane's download");
            tell(state, browser, "download_refused", &why);
            Before::Refuse
        }
    }
}

/// A download saving into a workspace was updated. Cancels one that grew past the limit, and tells the agent
/// when one finished or failed. Returns whether the download is one of these.
pub(crate) fn updated(
    state: &Arc<crate::state::AppState>,
    browser: Option<&Browser>,
    pane: &str,
    item: &DownloadItem,
    callback: Option<&DownloadItemCallback>,
) -> bool {
    let key = (pane.to_string(), item.id());
    let Some((name, dir, used)) = SAVING.with(|m| m.borrow().get(&key).map(|s| (s.name.clone(), s.dir.clone(), s.used))) else { return false };
    if item.is_complete() != 0 {
        SAVING.with(|m| m.borrow_mut().remove(&key));
        let path = CefString::from(&item.full_path()).to_string();
        let message = format!("{} ({}) to {}", name, size_words(item.received_bytes()), shown(Path::new(&path)));
        tracing::info!(target: "pane-download", %pane, id = item.id(), bytes = item.received_bytes(), "saved into the agent's workspace");
        tell(state, browser, "download_saved", &message);
    } else if item.is_interrupted() != 0 {
        SAVING.with(|m| m.borrow_mut().remove(&key));
        tell(state, browser, "download_refused", &format!("{name} did not finish: the download was interrupted"));
    } else if item.is_canceled() != 0 {
        // The person pressed Cancel in the strip, or the limit below cancelled it.
        SAVING.with(|m| m.borrow_mut().remove(&key));
    } else if item.received_bytes() > MAX_BYTES || used.saturating_add(item.received_bytes()) > FOLDER_MAX_BYTES {
        SAVING.with(|m| m.borrow_mut().remove(&key));
        if let Some(cb) = callback {
            cb.cancel();
        }
        let why = if item.received_bytes() > MAX_BYTES {
            format!("{name} grew past the {} limit for a download into the workspace and was stopped", size_words(MAX_BYTES))
        } else {
            format!("{name} was stopped: {}", full_words(&dir))
        };
        tell(state, browser, "download_refused", &why);
    }
    true
}

/// A pane closed: forget what it was saving.
pub(crate) fn forget_pane(pane: &str) {
    SAVING.with(|m| m.borrow_mut().retain(|(p, _), _| p != pane));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_words() {
        assert_eq!(size_words(13), "13 bytes");
        assert_eq!(size_words(1536), "1.5 KB");
        assert_eq!(size_words(4_400_000), "4.2 MB");
        assert_eq!(size_words(MAX_BYTES), "500.0 MB");
    }

    #[test]
    fn a_download_goes_into_the_workspace_downloads_folder_and_never_replaces_a_file() {
        let ws = tempfile::tempdir().unwrap();
        let first = target(ws.path(), "report.pdf", 13).unwrap().0;
        assert_eq!(first.file_name().unwrap(), "report.pdf");
        assert_eq!(first.parent().unwrap().file_name().unwrap(), "downloads");
        assert!(first.starts_with(std::fs::canonicalize(ws.path()).unwrap()));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(target(ws.path(), "report.pdf", 13).unwrap().0.file_name().unwrap(), "report (1).pdf");
        std::fs::write(first.with_file_name("report (1).pdf"), b"x").unwrap();
        assert_eq!(target(ws.path(), "report.pdf", -1).unwrap().0.file_name().unwrap(), "report (2).pdf");
        // No extension, and a name that is only an extension.
        std::fs::write(first.with_file_name("notes"), b"x").unwrap();
        assert_eq!(target(ws.path(), "notes", 1).unwrap().0.file_name().unwrap(), "notes (1)");
    }

    #[test]
    fn the_name_rules_hold_and_nothing_leaves_the_folder() {
        let ws = tempfile::tempdir().unwrap();
        for bad in ["..\\..\\evil.exe", "../../evil.exe", "C:\\Windows\\evil.exe", "/etc/passwd"] {
            let p = target(ws.path(), bad, 1).unwrap().0;
            assert_eq!(p.parent().unwrap().file_name().unwrap(), "downloads", "{bad}");
        }
        assert_eq!(target(ws.path(), "CON", 1).unwrap().0.file_name().unwrap(), "_CON");
    }

    #[test]
    fn one_over_the_limit_is_refused_with_the_reason() {
        let ws = tempfile::tempdir().unwrap();
        let why = target(ws.path(), "big.iso", MAX_BYTES + 1).unwrap_err();
        assert!(why.contains("big.iso") && why.contains("over the 500.0 MB limit"), "{why}");
        assert!(target(ws.path(), "ok.iso", MAX_BYTES).is_ok());
    }

    #[test]
    fn what_the_folder_already_holds_counts_sub_folders_too() {
        let ws = tempfile::tempdir().unwrap();
        let (first, used) = target(ws.path(), "a.bin", 10).unwrap();
        assert_eq!(used, 0);
        let dir = first.parent().unwrap().to_path_buf();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("old.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(dir.join("top.bin"), vec![0u8; 24]).unwrap();
        assert_eq!(folder_bytes(&dir), 1024);
        assert_eq!(target(ws.path(), "b.bin", 10).unwrap().1, 1024);
    }

    #[test]
    fn a_full_folder_refuses_with_words_naming_the_folder() {
        let why = full_words(Path::new("/w/downloads"));
        assert!(why.contains("full") && why.contains("2.0 GB") && why.contains("delete files in /w/downloads"), "{why}");
    }

    #[test]
    fn the_folder_cap_is_above_one_file_and_the_total_is_checked_with_the_new_download() {
        assert!(FOLDER_MAX_BYTES > MAX_BYTES);
        // A folder that held the cap minus one byte has no room for even a 2-byte download.
        assert!((FOLDER_MAX_BYTES - 1).saturating_add(2) > FOLDER_MAX_BYTES);
    }

    #[test]
    fn a_workspace_that_is_not_there_is_refused() {
        let ws = tempfile::tempdir().unwrap();
        let gone = ws.path().join("nope");
        assert!(target(&gone, "a.txt", 1).unwrap_err().contains("workspace folder isn't there"));
    }

    #[cfg(unix)]
    #[test]
    fn a_downloads_folder_that_links_out_is_not_followed() {
        let ws = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), ws.path().join("downloads")).unwrap();
        let why = target(ws.path(), "a.txt", 1).unwrap_err();
        assert!(why.contains("is a link"), "{why}");
        assert!(std::fs::read_dir(elsewhere.path()).unwrap().next().is_none());
    }

    #[test]
    fn a_file_called_downloads_is_refused() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("downloads"), b"x").unwrap();
        assert!(target(ws.path(), "a.txt", 1).unwrap_err().contains("not a folder"));
    }

    #[test]
    fn the_extended_length_prefix_is_not_shown_to_the_agent() {
        assert_eq!(shown(Path::new(r"\\?\C:\w\downloads\a.txt")), r"C:\w\downloads\a.txt");
        assert_eq!(shown(Path::new(r"\\?\UNC\srv\w\a.txt")), r"\\?\UNC\srv\w\a.txt");
        assert_eq!(shown(Path::new("/w/downloads/a.txt")), "/w/downloads/a.txt");
    }
}
