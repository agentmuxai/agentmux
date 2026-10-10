// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Every call into the `trash` crate runs on one dedicated thread.
//!
//! The crate documents undefined behaviour when it is called from more than
//! one thread at a time on Linux/FreeBSD, and on Windows it initializes COM
//! per calling thread. A single long-lived worker satisfies both: calls are
//! serialized, and COM is set up once (spec §7.3).
//!
//! Spec: docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §7.3, §9.1.5.

use std::path::Path;
use std::sync::mpsc;
use std::sync::OnceLock;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// The worker's queue. `None` if the thread could not be started, in which
/// case every trash call fails cleanly instead of panicking a handler.
static WORKER: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();

fn worker() -> Option<&'static mpsc::Sender<Job>> {
    WORKER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            let spawned = std::thread::Builder::new().name("fs-trash".to_string()).spawn(move || {
                for job in rx {
                    // One panicking job must not end the thread, or every
                    // later trash call would fail.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            });
            match spawned {
                Ok(_) => Some(tx),
                Err(e) => {
                    tracing::error!(error = %e, "fs: could not start the trash worker thread");
                    None
                }
            }
        })
        .as_ref()
}

/// Run `f` on the trash thread and wait for its answer without blocking the
/// async runtime.
pub async fn run<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let sender = worker().ok_or_else(|| "The Trash isn't available right now.".to_string())?;
    sender
        .send(Box::new(move || {
            let _ = tx.send(f());
        }))
        .map_err(|_| "The Trash isn't available right now.".to_string())?;
    rx.await.map_err(|_| "Moving to the Trash failed unexpectedly.".to_string())
}

/// A `TrashContext` that, on macOS, uses `NSFileManager` rather than the
/// crate's default of scripting Finder: the Finder route raises an
/// Automation prompt ("AgentMux wants to control Finder"), and once refused,
/// later deletes silently do nothing (spec §9.1.5).
fn context() -> trash::TrashContext {
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx
}

/// Move `path` to the Trash. Must be called on the trash thread (via [`run`]).
/// The crate trashes a link itself, not its target.
pub fn trash_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if let Some(result) = macos::trash(path) {
        return result;
    }
    context().delete(path).map_err(|e| {
        tracing::warn!(path = %path.display(), error = ?e, "fs.trash: trash crate error");
        format!("Couldn't move it to the Trash: {}", describe(&e))
    })
}

/// A readable clause for a `trash::Error`, whose own `Display` is its
/// `Debug` dump.
fn describe(err: &trash::Error) -> String {
    match err {
        trash::Error::Os { description, .. } | trash::Error::Unknown { description } => description.clone(),
        trash::Error::TargetedRoot => "a drive or filesystem root can't be trashed".to_string(),
        trash::Error::CouldNotAccess { .. } => "it doesn't exist or can't be accessed".to_string(),
        trash::Error::CanonicalizePath { .. } => "its folder couldn't be found".to_string(),
        #[cfg(all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))]
        trash::Error::FileSystem { source, .. } => source.to_string(),
        other => format!("{other:?}"),
    }
}

/// Put each of `targets` back from the Trash: for each, the most recently
/// trashed item whose original location is that path. Must be called on the
/// trash thread (via [`run`]). One result per target, in order.
#[cfg(any(
    target_os = "windows",
    all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))
))]
pub fn restore_paths(targets: &[std::path::PathBuf]) -> Vec<Result<(), String>> {
    use std::collections::HashSet;

    let items = match trash::os_limited::list() {
        Ok(items) => items,
        Err(e) => {
            let msg = format!("Couldn't read the Trash: {e}");
            return targets.iter().map(|_| Err(msg.clone())).collect();
        }
    };
    // Items already put back by an earlier target in this same request, so a
    // path named twice doesn't try to restore one item twice.
    let mut used: HashSet<std::ffi::OsString> = HashSet::new();
    targets
        .iter()
        .map(|target| {
            let want = path_key(target);
            let item = items
                .iter()
                .filter(|it| !used.contains(&it.id) && path_key(&it.original_path()) == want)
                .max_by_key(|it| it.time_deleted)
                .cloned()
                .ok_or_else(|| "It isn't in the Trash anymore.".to_string())?;
            used.insert(item.id.clone());
            trash::os_limited::restore_all([item]).map_err(|e| match e {
                trash::Error::RestoreCollision { .. } => {
                    "Something with the same name is already there, so it can't be put back.".to_string()
                }
                other => {
                    tracing::warn!(path = %target.display(), error = ?other, "fs.restore: trash crate error");
                    format!("Couldn't put it back: {}", describe(&other))
                }
            })
        })
        .collect()
}

/// macOS: the `trash` crate can't list or restore the Trash there, so
/// [`macos::trash`] remembers where `NSFileManager` put each item and this
/// moves it back. Only items trashed since srv started can be put back.
#[cfg(target_os = "macos")]
pub fn restore_paths(targets: &[std::path::PathBuf]) -> Vec<Result<(), String>> {
    targets.iter().map(|target| macos::restore(target)).collect()
}

/// Other platforms the `trash` crate can't restore on.
#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(unix, not(target_os = "ios"), not(target_os = "android"))
)))]
pub fn restore_paths(targets: &[std::path::PathBuf]) -> Vec<Result<(), String>> {
    targets
        .iter()
        .map(|_| Err("Restoring from the Trash isn't supported on this system.".to_string()))
        .collect()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    use objc2_foundation::{NSFileManager, NSString, NSURL};

    use super::super::platform;
    use super::path_key;

    /// Where each item trashed since srv started went, by its original
    /// path's [`path_key`], oldest first.
    fn trashed() -> &'static Mutex<HashMap<String, Vec<PathBuf>>> {
        static TRASHED: OnceLock<Mutex<HashMap<String, Vec<PathBuf>>>> = OnceLock::new();
        TRASHED.get_or_init(Default::default)
    }

    /// Trash `path` through `NSFileManager` and remember where it went.
    /// `None` for a path that isn't UTF-8, which the crate handles instead.
    pub(super) fn trash(path: &Path) -> Option<Result<(), String>> {
        let utf8 = path.to_str()?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(utf8));
        let mut resulting = None;
        let file_mgr = NSFileManager::defaultManager();
        if let Err(e) = file_mgr.trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting)) {
            let description = e.localizedDescription().to_string();
            tracing::warn!(path = %path.display(), error = %description, "fs.trash: trashItemAtURL failed");
            return Some(Err(format!("Couldn't move it to the Trash: {description}")));
        }
        match resulting.and_then(|u| u.path()) {
            Some(dest) => {
                let mut map = trashed().lock().unwrap_or_else(|e| e.into_inner());
                map.entry(path_key(path)).or_default().push(PathBuf::from(dest.to_string()));
            }
            None => tracing::warn!(path = %path.display(), "fs.trash: trashed, but macOS didn't say where to"),
        }
        Some(Ok(()))
    }

    /// Put back the most recently trashed item that came from `target` and is
    /// still in the Trash, refusing if something is in its place now.
    pub(super) fn restore(target: &Path) -> Result<(), String> {
        let key = path_key(target);
        let mut map = trashed().lock().unwrap_or_else(|e| e.into_inner());
        let Some(stack) = map.get_mut(&key) else {
            return Err(
                "It was moved to the Trash before AgentMux started, so it can't be put back from here. \
                 Put Back in the Finder can."
                    .to_string(),
            );
        };
        // Drop what's no longer in the Trash: emptied, or moved out of it.
        while stack.last().is_some_and(|p| std::fs::symlink_metadata(p).is_err()) {
            stack.pop();
        }
        let Some(from) = stack.last().cloned() else {
            map.remove(&key);
            return Err("It isn't in the Trash anymore.".to_string());
        };
        if !target.parent().is_some_and(Path::is_dir) {
            return Err("The folder it was in isn't there anymore, so it can't be put back.".to_string());
        }
        platform::rename_no_replace(&from, target).map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                "Something with the same name is already there, so it can't be put back.".to_string()
            }
            _ => {
                tracing::warn!(path = %target.display(), error = %e, "fs.restore: move back failed");
                format!("Couldn't put it back: {e}")
            }
        })?;
        stack.pop();
        if stack.is_empty() {
            map.remove(&key);
        }
        Ok(())
    }
}

/// A path's identity for matching a trashed item's original location: the
/// display form, case-folded on Windows, whose filesystems are
/// case-insensitive and whose Recycle Bin reports plain (non-verbatim) paths.
#[cfg_attr(
    not(any(target_os = "windows", target_os = "macos", all(unix, not(target_os = "ios"), not(target_os = "android")))),
    allow(dead_code)
)]
fn path_key(path: &Path) -> String {
    let display = super::platform::display_path(path);
    if cfg!(windows) {
        display.to_lowercase()
    } else {
        display
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_job_runs_on_the_same_thread() {
        let a = run(|| std::thread::current().id()).await.unwrap();
        let b = run(|| std::thread::current().id()).await.unwrap();
        assert_eq!(a, b);
        assert_ne!(a, std::thread::current().id());
    }

    #[tokio::test]
    async fn a_panicking_job_does_not_end_the_worker() {
        let r = run(|| -> u8 { panic!("boom") }).await;
        assert!(r.is_err());
        assert_eq!(run(|| 7).await.unwrap(), 7);
    }

    /// Touches the real Recycle Bin / Trash, so it is opt-in:
    /// `cargo test -p agentmux-srv trash_and_restore_round_trip -- --ignored`.
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    #[ignore]
    async fn trash_and_restore_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().canonicalize().unwrap().join(format!("agentmux-trash-test-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&file, "x").unwrap();

        let f = file.clone();
        run(move || trash_path(&f)).await.unwrap().unwrap();
        assert!(!file.exists(), "trashed file must be gone from its folder");

        let f = file.clone();
        let results = run(move || restore_paths(&[f])).await.unwrap();
        assert_eq!(results.len(), 1);
        results[0].clone().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "x", "restored in place");
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_restore_refuses_what_it_did_not_trash() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().canonicalize().unwrap().join("never-trashed.txt");
        let results = run(move || restore_paths(&[file])).await.unwrap();
        assert!(results[0].as_ref().unwrap_err().contains("before AgentMux started"));
    }

    /// Touches the real Trash, so it is opt-in, like the round trip.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore]
    async fn macos_restore_leaves_a_new_item_in_its_place_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().canonicalize().unwrap().join(format!("agentmux-trash-test-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&file, "old").unwrap();
        let f = file.clone();
        run(move || trash_path(&f)).await.unwrap().unwrap();
        std::fs::write(&file, "new").unwrap();

        let f = file.clone();
        let results = run(move || restore_paths(&[f])).await.unwrap();
        assert!(results[0].as_ref().unwrap_err().contains("already there"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new", "the new file is untouched");

        // Once it's out of the way, the trashed one comes back.
        std::fs::remove_file(&file).unwrap();
        let f = file.clone();
        let results = run(move || restore_paths(&[f])).await.unwrap();
        results[0].clone().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");
    }
}
