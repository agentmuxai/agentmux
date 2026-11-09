// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A page's file chooser, kept for the agent
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §6).
//!
//! `filedialog.rs` cancels every chooser a page opens in a pane an agent drives.
//! When srv says the agent holds the uploads power on that site, the host keeps
//! CEF's callback instead, for [`HOLD`], and `set_files` with `chooser: true`
//! answers it (`browser_api/chooser_files.rs`). The files are the bytes srv read
//! from the agent's workspace; they are written into a private folder of their
//! own and CEF is handed those paths, so the page sees exactly what a person's
//! pick would give it (a `change` event the browser raised itself) and never a
//! path in the workspace. Only the newest chooser per pane is kept.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use cef::*;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::browser_api::act::UploadFile;
use crate::browser_api::file_chooser::files_refusal;

/// How long a chooser is kept for the agent.
pub(crate) const HOLD: Duration = Duration::from_secs(30);
/// Folders of chosen files kept per pane: a page may read a file long after it was chosen
/// (an upload that starts on a later click), so they stay until the pane closes or this many newer ones exist.
const KEPT_PER_PANE: usize = 8;
/// Leftovers of an earlier run are removed once they are this old.
const STALE: Duration = Duration::from_secs(24 * 60 * 60);

struct Held {
    id: String,
    callback: FileDialogCallback,
    multiple: bool,
    /// The chooser's `accept` filters, comma-joined as the attribute reads.
    accept: String,
    /// The site that opened it.
    origin: String,
    opened: Instant,
}

fn held() -> &'static Mutex<HashMap<String, Held>> {
    static H: OnceLock<Mutex<HashMap<String, Held>>> = OnceLock::new();
    H.get_or_init(Default::default)
}

/// The folders written for each pane, oldest first.
fn written() -> &'static Mutex<HashMap<String, Vec<PathBuf>>> {
    static W: OnceLock<Mutex<HashMap<String, Vec<PathBuf>>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

fn site_of(url: &str) -> Option<String> {
    agentmux_common::allowed_origins::target_origin(url)
}

/// Keep `callback` for `target` while srv decides, and ask it (`body` says what the page asked
/// for). A chooser kept earlier for the pane is cancelled: only the newest counts. Anything but
/// srv's "hold" cancels it, and so does [`HOLD`] running out.
pub(super) fn park(
    state: &Arc<crate::state::AppState>,
    target: &str,
    callback: &FileDialogCallback,
    multiple: bool,
    accept: Vec<String>,
    origin: &str,
    mut body: Value,
) {
    let id = uuid::Uuid::new_v4().to_string();
    let entry = Held { id: id.clone(), callback: callback.clone(), multiple, accept: accept.join(","), origin: origin.to_string(), opened: Instant::now() };
    if let Some(old) = held().lock().insert(target.to_string(), entry) {
        old.callback.cancel();
    }
    tracing::info!(target = %target, id = %id, "browser pane: keeping a page's file chooser while srv decides");
    if let Some(rt) = crate::browser_pane::auth::runtime() {
        let (target, id) = (target.to_string(), id.clone());
        rt.spawn(async move {
            tokio::time::sleep(HOLD).await;
            drop_if(&target, &id);
        });
    }
    body["id"] = json!(id);
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let ipc_token = state.ipc_token.clone();
    let target = target.to_string();
    let asked = std::thread::Builder::new().name("browser-chooser".into()).spawn({
        let (target, id) = (target.clone(), id.clone());
        move || match super::backend_browser_dialog(&web_endpoint, &auth_key, &ipc_token, &body) {
            Ok(d) if d == "hold" => {}
            Ok(_) => drop_if(&target, &id),
            Err(e) => {
                tracing::warn!(error = %e, "browser pane: couldn't ask srv about a page's file chooser; cancelling it");
                drop_if(&target, &id);
            }
        }
    });
    if asked.is_err() {
        drop_if(&target, &id);
    }
}

/// Cancel the chooser kept for `target` if it is still the one `id` names.
fn drop_if(target: &str, id: &str) {
    let mut h = held().lock();
    if h.get(target).is_some_and(|e| e.id == id) {
        if let Some(e) = h.remove(target) {
            e.callback.cancel();
            tracing::info!(target = %target, "browser pane: a page's file chooser was cancelled (not kept, or nobody filled it in time)");
        }
    }
}

/// The pane is closing: cancel its chooser and remove the files chosen in it.
pub(crate) fn cancel_for_block(target: &str) {
    if let Some(e) = held().lock().remove(target) {
        e.callback.cancel();
    }
    for dir in written().lock().remove(target).unwrap_or_default() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Why the kept chooser can't take `files` now, or `None`; `true` says the chooser is gone for good
/// (it ran out, or the page moved on), `false` that only these files are refused. `now_site` is the
/// site the pane is on.
fn refusal(opened: Instant, now: Instant, opened_site: &str, now_site: &str, multiple: bool, accept: &str, files: &[UploadFile]) -> Option<(bool, String)> {
    if now.duration_since(opened) >= HOLD {
        return Some((true, no_chooser()));
    }
    if opened_site != now_site {
        return Some((true, "the page moved to another site after it opened the file chooser, so nothing was chosen: open the chooser again".to_string()));
    }
    files_refusal(files, multiple, accept).map(|why| (false, why))
}

fn no_chooser() -> String {
    format!(
        "no file chooser is waiting: the page hasn't opened one in the last {} seconds, or it was already filled. Click the control that opens it, then call set_files with chooser: true again",
        HOLD.as_secs()
    )
}

/// Put `files` in the chooser kept for `target`, which the pane's page at `page` must still be on the
/// site of. Answers what was chosen. A refusal the agent can fix (a file the page doesn't accept)
/// leaves the chooser kept.
pub(crate) fn fill(target: &str, page: &str, files: &[UploadFile]) -> Result<Vec<Value>, String> {
    let now_site = site_of(page).ok_or("couldn't tell which site the page is on")?;
    let kept = {
        let mut h = held().lock();
        let Some(e) = h.get(target) else { return Err(no_chooser()) };
        if let Some((gone, why)) = refusal(e.opened, Instant::now(), &site_of(&e.origin).unwrap_or_default(), &now_site, e.multiple, &e.accept, files) {
            if gone {
                if let Some(e) = h.remove(target) {
                    e.callback.cancel();
                }
            }
            return Err(why);
        }
        h.remove(target).ok_or_else(no_chooser)?
    };
    let dir = match write_files(&base_dir(), files) {
        Ok(d) => d,
        Err(e) => {
            kept.callback.cancel();
            return Err(format!("{e}; the chooser was closed, open it again"));
        }
    };
    let mut paths = CefStringList::new();
    for f in &dir.files {
        paths.append(&f.to_string_lossy());
    }
    kept.callback.cont(Some(&mut paths));
    tracing::info!(target = %target, files = files.len(), "browser pane: a page's file chooser was filled for the agent");
    keep(target, dir.root);
    Ok(files.iter().zip(&dir.sizes).map(|(f, n)| json!({ "name": safe_name(&f.name).unwrap_or_default(), "size": n })).collect())
}

/// Remember `dir` for `target`; the oldest beyond [`KEPT_PER_PANE`] go.
fn keep(target: &str, dir: PathBuf) {
    let mut w = written().lock();
    let list = w.entry(target.to_string()).or_default();
    list.push(dir);
    while list.len() > KEPT_PER_PANE {
        let _ = std::fs::remove_dir_all(list.remove(0));
    }
}

/// Where chosen files are written: one folder for AgentMux in the system's temp folder.
fn base_dir() -> PathBuf {
    std::env::temp_dir().join("agentmux-chooser")
}

/// The name a file is given on disk: its last path component only, without anything a file
/// system treats specially. `None` when nothing usable is left.
fn safe_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let clean: String = last
        .chars()
        .map(|c| if c.is_control() || (cfg!(windows) && "<>:\"|?*".contains(c)) { '_' } else { c })
        .collect();
    (!clean.is_empty() && clean != "." && clean != "..").then_some(clean)
}

struct Written {
    root: PathBuf,
    files: Vec<PathBuf>,
    sizes: Vec<usize>,
}

#[cfg(unix)]
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
}

#[cfg(not(unix))]
fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Remove folders of an earlier run (or pane) left in `base` for longer than [`STALE`].
fn sweep(base: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= STALE);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Write each file under its own numbered folder (two files may share a name) inside a new
/// private folder of `base`.
fn write_files(base: &Path, files: &[UploadFile]) -> Result<Written, String> {
    use base64::Engine as _;
    static SWEPT: OnceLock<()> = OnceLock::new();
    private_dir(base).map_err(|e| format!("couldn't make a folder for the chosen files: {e}"))?;
    SWEPT.get_or_init(|| sweep(base, std::time::SystemTime::now()));
    let root = base.join(uuid::Uuid::new_v4().to_string());
    private_dir(&root).map_err(|e| format!("couldn't make a folder for the chosen files: {e}"))?;
    let mut out = Written { root: root.clone(), files: vec![], sizes: vec![] };
    for (i, f) in files.iter().enumerate() {
        let written = (|| {
            let name = safe_name(&f.name).ok_or_else(|| format!("{:?} is not a usable file name", f.name))?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(&f.data).map_err(|e| format!("{} isn't readable: {e}", f.name))?;
            let dir = root.join(i.to_string());
            private_dir(&dir).map_err(|e| e.to_string())?;
            let path = dir.join(name);
            std::fs::write(&path, &bytes).map_err(|e| format!("couldn't write {}: {e}", f.name))?;
            Ok::<_, String>((path, bytes.len()))
        })();
        match written {
            Ok((path, n)) => {
                out.files.push(path);
                out.sizes.push(n);
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(e);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn file(name: &str, bytes: &[u8]) -> UploadFile {
        UploadFile { name: name.to_string(), mime: String::new(), data: base64::engine::general_purpose::STANDARD.encode(bytes) }
    }

    #[test]
    fn a_name_keeps_its_last_component_and_nothing_else() {
        assert_eq!(safe_name("notes.txt").as_deref(), Some("notes.txt"));
        assert_eq!(safe_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_name("C:\\Users\\me\\a.pdf").as_deref(), Some("a.pdf"));
        assert_eq!(safe_name("a\nb.txt").as_deref(), Some("a_b.txt"));
        assert_eq!(safe_name(".."), None);
        assert_eq!(safe_name("dir/"), None);
        assert_eq!(safe_name("  "), None);
    }

    #[test]
    fn files_are_written_under_a_folder_each_and_keep_their_bytes() {
        let base = std::env::temp_dir().join(format!("agentmux-chooser-test-{}", uuid::Uuid::new_v4()));
        let w = write_files(&base, &[file("a.txt", b"one"), file("a.txt", b"three")]).unwrap();
        assert_eq!(w.sizes, vec![3, 5]);
        assert_ne!(w.files[0], w.files[1], "two files of one name don't overwrite each other");
        assert_eq!(std::fs::read(&w.files[0]).unwrap(), b"one");
        assert_eq!(std::fs::read(&w.files[1]).unwrap(), b"three");
        assert!(w.files.iter().all(|f| f.starts_with(&base) && f.file_name().unwrap() == "a.txt"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_bad_file_leaves_nothing_behind() {
        let base = std::env::temp_dir().join(format!("agentmux-chooser-test-{}", uuid::Uuid::new_v4()));
        let bad = UploadFile { name: "b.txt".into(), mime: String::new(), data: "not base64 !!".into() };
        assert!(write_files(&base, &[file("a.txt", b"x"), bad]).is_err());
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0, "the folder written so far is removed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn old_folders_are_swept_and_new_ones_stay() {
        let base = std::env::temp_dir().join(format!("agentmux-chooser-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("fresh")).unwrap();
        sweep(&base, std::time::SystemTime::now());
        assert!(base.join("fresh").exists());
        sweep(&base, std::time::SystemTime::now() + STALE + Duration::from_secs(60));
        assert!(!base.join("fresh").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_chooser_is_refused_files_when_it_ran_out_the_page_moved_or_the_files_dont_fit() {
        let t0 = Instant::now();
        let ok = [file("a.txt", b"x")];
        let site = "https://a.example";
        assert_eq!(refusal(t0, t0 + Duration::from_secs(5), site, site, false, ".txt", &ok), None);
        let (gone, late) = refusal(t0, t0 + HOLD, site, site, false, "", &ok).unwrap();
        assert!(gone && late.starts_with("no file chooser") && late.contains("30 seconds"), "{late}");
        let (gone, moved) = refusal(t0, t0, site, "https://b.example", false, "", &ok).unwrap();
        assert!(gone && moved.starts_with("the page moved"), "{moved}");
        let (gone, wrong) = refusal(t0, t0, site, site, false, ".pdf", &ok).unwrap();
        assert!(!gone, "a file the page doesn't accept leaves the chooser kept");
        assert!(wrong.contains(".pdf") && wrong.contains("a.txt"), "{wrong}");
        let two = [file("a.txt", b"x"), file("b.txt", b"y")];
        assert!(refusal(t0, t0, site, site, false, "", &two).unwrap().1.contains("one file"));
        assert_eq!(refusal(t0, t0, site, site, true, "", &two), None);
    }
}
