// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The host folders that are a container agent's workspace.
//!
//! Git facts of a container agent's folder are not read on the host:
//! [`super::git`] starts no git for such a folder or for anything inside it,
//! so the work facts, the overlap and PR helpers and the Files pane's markers
//! have none there. (A temporary limit: those facts are to be read inside the
//! agent's container.)
//!
//! The folders are read from the panes in the store, the same value a
//! container's workspace is bound from (`cmd:cwd` of a pane whose agent runs
//! in a container), and reused for a moment so a burst of git runs reads the
//! store once.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::backend::blockcontroller::META_KEY_CMD_CWD;
use crate::backend::obj::{self, Block};
use crate::backend::runtime_adapter::RuntimeKind;
use crate::backend::storage::store::Store;

/// How long one read of the store's panes is reused.
const REUSE: Duration = Duration::from_secs(2);

#[derive(Default)]
struct State {
    store: Option<Arc<Store>>,
    /// The folders' keys ([`key`]), from the last read that worked.
    folders: Option<Vec<String>>,
    read_at: Option<Instant>,
    #[cfg(test)]
    for_tests: Vec<String>,
}

fn state() -> &'static parking_lot::Mutex<State> {
    static STATE: std::sync::LazyLock<parking_lot::Mutex<State>> = std::sync::LazyLock::new(Default::default);
    &STATE
}

/// Name the store the folders are read from. Called by everything that leads
/// to a git run before it runs one (the work-facts loop, the Files pane's
/// handler); the first call wins.
pub fn install(store: &Arc<Store>) {
    let mut s = state().lock();
    if s.store.is_none() {
        s.store = Some(Arc::clone(store));
    }
}

/// One spelling of a folder for comparing: the real path where it exists,
/// `/` separators, no long-path prefix, and lower case on Windows.
fn key(dir: &Path) -> String {
    let real = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let text = crate::backend::work_facts::paths::normalize_path(&real.to_string_lossy());
    let text = text.strip_prefix("//?/").map(str::to_string).unwrap_or(text);
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Whether `dir` is `folder` or inside it (both as [`key`]s).
fn inside(dir: &str, folder: &str) -> bool {
    !folder.is_empty()
        && dir.strip_prefix(folder).is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || folder.ends_with('/'))
}

/// The workspace folder of a pane whose agent runs in a container; `None`
/// for any other pane.
pub fn workspace_of(meta: &obj::MetaMapType) -> Option<std::path::PathBuf> {
    if !RuntimeKind::of_block_meta(meta).is_container() {
        return None;
    }
    let dir = obj::meta_get_string(meta, META_KEY_CMD_CWD, "");
    let dir = dir.trim();
    (!dir.is_empty()).then(|| crate::backend::base::expand_home_dir_safe(dir))
}

/// The keys of every container agent's workspace folder in `store`.
fn folders_of(store: &Store) -> Result<Vec<String>, crate::backend::storage::StoreError> {
    Ok(store.get_all::<Block>()?.iter().filter_map(|b| workspace_of(&b.meta)).map(|dir| key(&dir)).collect())
}

/// Whether `dir` is a container agent's workspace folder or inside one.
///
/// Reads the store (blocking): call it off the async workers. When the
/// panes can't be read and never were, the answer is yes: no git runs
/// anywhere until they can.
pub fn holds(dir: &str) -> bool {
    let dir = key(Path::new(dir));
    let mut s = state().lock();
    #[cfg(test)]
    if s.for_tests.iter().any(|f| inside(&dir, f)) {
        return true;
    }
    let Some(store) = s.store.clone() else {
        return false;
    };
    if s.read_at.is_none_or(|at| at.elapsed() >= REUSE) {
        match folders_of(&store) {
            Ok(folders) => s.folders = Some(folders),
            Err(e) => tracing::warn!(error = %e, "container folders: can't read the panes; keeping the last list"),
        }
        s.read_at = Some(Instant::now());
    }
    match &s.folders {
        Some(folders) => folders.iter().any(|f| inside(&dir, f)),
        None => true,
    }
}

/// Count `folder` as a container agent's workspace for the tests of this
/// crate (each test names its own temp folder).
#[cfg(test)]
pub(crate) fn add_for_test(folder: &Path) {
    state().lock().for_tests.push(key(folder));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_holds_itself_and_what_is_inside_it_only() {
        assert!(inside("/w/aria", "/w/aria"));
        assert!(inside("/w/aria/repo/src", "/w/aria"));
        assert!(!inside("/w/aria2", "/w/aria"), "a sibling with a longer name is outside");
        assert!(!inside("/w", "/w/aria"), "the folder above is outside");
        assert!(inside("c:/x", "c:/"));
        assert!(!inside("/w/aria", ""));
    }

    fn pane(mode: &str, cwd: &str) -> Block {
        let mut meta = obj::MetaMapType::new();
        meta.insert("agentMode".to_string(), serde_json::json!(mode));
        meta.insert(META_KEY_CMD_CWD.to_string(), serde_json::json!(cwd));
        Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() }
    }

    #[test]
    fn only_a_container_agents_pane_names_a_workspace() {
        assert_eq!(workspace_of(&pane("container", "/w/aria").meta), Some(std::path::PathBuf::from("/w/aria")));
        assert_eq!(workspace_of(&pane("host", "/w/host").meta), None);
        assert_eq!(workspace_of(&pane("container", " ").meta), None, "no folder is bound");
    }

    #[test]
    fn the_folders_are_the_container_agents_workspaces_in_the_store() {
        let store = Store::open_in_memory().unwrap();
        let container = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        store.insert(&mut pane("container", &container.path().to_string_lossy())).unwrap();
        store.insert(&mut pane("host", &host.path().to_string_lossy())).unwrap();
        let folders = folders_of(&store).unwrap();
        assert_eq!(folders, vec![key(container.path())]);
        let deep = container.path().join("repo").join("src");
        std::fs::create_dir_all(&deep).unwrap();
        assert!(inside(&key(&deep), &folders[0]));
        assert!(!inside(&key(host.path()), &folders[0]));
    }
}
