// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The host folders a container agent can write, and where each is inside the agent's container.
//!
//! Git facts of a container agent are read inside its container: for such a folder, or anything inside it,
//! [`super::git`] starts no git on the host and runs it in the agent's container instead, at the same folder
//! there ([`place`]). So the work facts, the overlap and PR helpers and the Files pane's markers say what the
//! agent's own git says.
//!
//! The folders are read from the panes in the store, the values a container's mounts are made from
//! (`container::mounts::writable`): the workspace (`cmd:cwd` of a pane whose agent runs in a container), the
//! agent's Personal Memory folder, and each host folder of its volumes that is not bound read-only. The list
//! is reused for a moment so a burst of git runs reads the store once.
//!
//! A folder is asked about as it is spelled and as it resolves (links followed): either one inside a bound
//! folder makes it the container's.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::backend::blockcontroller::META_KEY_CMD_CWD;
use crate::backend::obj::{self, Block};
use crate::backend::storage::store::Store;
use crate::backend::work_facts::paths::{normalize_path, relative_to};

/// How long one read of the store's panes is reused: a new container agent's folders are known this soon.
const REUSE: Duration = Duration::from_millis(250);

/// One bound folder: its host spellings (as written, and as it resolves when that differs), the container
/// it is bound into (empty when the pane names no agent) and its place there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Folder {
    hosts: Vec<String>,
    container: String,
    target: String,
}

/// Where git for a folder runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// On the host: no container agent can write the folder.
    Host,
    /// In a container agent's container.
    Inside(Inside),
    /// Not known (the panes can't be read and never were): nowhere.
    Unknown,
}

/// A host folder as its container sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inside {
    /// The container's name; empty when the pane names no agent, which no read answers for.
    pub container: String,
    /// The folder's absolute path inside the container.
    pub dir: String,
    /// The bound folder on the host, in the spelling the asked folder was found under.
    host: String,
    /// Where the bound folder is inside the container.
    target: String,
}

impl Inside {
    /// The host path of `inside`, an absolute path inside the container; `None` when it is not in the bound
    /// folder this was found under.
    pub fn host_path(&self, inside: &str) -> Option<String> {
        let inside = crate::backend::cli_messaging::normalized(inside);
        let target = self.target.trim_end_matches('/');
        let rest = match inside.strip_prefix(target) {
            Some("") => "",
            Some(rest) if rest.starts_with('/') => &rest[1..],
            _ => return None,
        };
        Some(if rest.is_empty() { self.host.clone() } else { format!("{}/{rest}", self.host.trim_end_matches('/')) })
    }
}

#[derive(Default)]
struct State {
    store: Option<Arc<Store>>,
    /// From the last read that worked.
    folders: Option<Vec<Folder>>,
    read_at: Option<Instant>,
    #[cfg(test)]
    for_tests: Vec<Folder>,
}

fn state() -> &'static parking_lot::Mutex<State> {
    static STATE: std::sync::LazyLock<parking_lot::Mutex<State>> = std::sync::LazyLock::new(Default::default);
    &STATE
}

/// Name the store the folders are read from. Called by everything that leads to a git run before it runs
/// one (the work-facts loop, the Files pane's handler); the first call wins.
pub fn install(store: &Arc<Store>) {
    let mut s = state().lock();
    if s.store.is_none() {
        s.store = Some(Arc::clone(store));
    }
}

/// `text` as one spelling for comparing: `/` separators, no long-path prefix, no trailing separator.
fn one_spelling(text: &str) -> String {
    let text = normalize_path(text);
    text.strip_prefix("//?/").map(str::to_string).unwrap_or(text)
}

/// `dir` as it is written: absolute, with `.` and `..` taken out, and nothing looked up on the disk.
fn spelled(dir: &Path) -> String {
    let absolute = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let text = one_spelling(&absolute.to_string_lossy());
    let resolved = crate::backend::cli_messaging::normalized(&text);
    // A drive's own folder keeps its separator.
    if resolved.ends_with(':') {
        format!("{resolved}/")
    } else {
        resolved
    }
}

/// `dir` as it resolves on the disk (links followed); `None` where it does not exist.
fn resolved(dir: &Path) -> Option<String> {
    std::fs::canonicalize(dir).ok().map(|real| one_spelling(&real.to_string_lossy()))
}

/// Both spellings of `dir`, the written one first.
fn spellings(dir: &Path) -> Vec<String> {
    let mut all = vec![spelled(dir)];
    all.extend(resolved(dir).filter(|real| *real != all[0]));
    all
}

/// The host folder of the Personal Memory of the agent of a container pane, as its container's mounts name it:
/// after the workspace's own path (`written` says which folders a container agent can write).
fn memory_of(store: &Store, meta: &obj::MetaMapType, written: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    if !crate::backend::runtime_adapter::RuntimeKind::of_block_meta(meta).is_container() {
        return None;
    }
    let agent = obj::meta_get_string(meta, "agentId", "");
    let cwd = obj::meta_get_string(meta, META_KEY_CMD_CWD, "");
    if agent.is_empty() || cwd.trim().is_empty() {
        return None;
    }
    let config = crate::backend::memory::native_files::agent_claude_config_dir(store, &agent);
    Some(crate::backend::memory::native_files::memory_dir_for_cwd_with(&config, cwd.trim(), written))
}

/// The folders the pane `meta` binds writable into its agent's container.
fn folders_of_pane(meta: &obj::MetaMapType, memory: Option<&Path>) -> Vec<Folder> {
    let container = crate::backend::container::container_of_meta(meta).unwrap_or_default();
    crate::backend::container::mounts::writable::writable_binds(meta, memory)
        .into_iter()
        .map(|bind| Folder { hosts: spellings(&bind.host), container: container.clone(), target: bind.target })
        .collect()
}

/// Every folder a container agent of `store` can write.
fn folders_of(store: &Store) -> Result<Vec<Folder>, crate::backend::storage::StoreError> {
    let panes = store.get_all::<Block>()?;
    // The workspaces and volumes first: they say which folders a memory folder may not be named from.
    let bound: Vec<Folder> = panes.iter().flat_map(|b| folders_of_pane(&b.meta, None)).collect();
    let written = |dir: &Path| inside_of(&spellings(dir), &bound).is_some();
    let mut folders: Vec<Folder> =
        panes.iter().flat_map(|b| folders_of_pane(&b.meta, memory_of(store, &b.meta, &written).as_deref())).collect();
    folders.sort_by(|a, b| a.container.cmp(&b.container));
    Ok(folders)
}

/// Where `asked` (the spellings of one folder) is inside `folders`: the deepest bound folder that holds it,
/// the written spelling before the resolved one.
fn inside_of(asked: &[String], folders: &[Folder]) -> Option<Inside> {
    let mut best: Option<Inside> = None;
    for spelling in asked {
        for folder in folders {
            for host in &folder.hosts {
                let Some(rest) = relative_to(spelling, host) else { continue };
                if best.as_ref().is_some_and(|b| b.host.len() >= host.len()) {
                    continue;
                }
                let target = folder.target.trim_end_matches('/');
                best = Some(Inside {
                    container: folder.container.clone(),
                    dir: if rest.is_empty() { folder.target.clone() } else { format!("{target}/{rest}") },
                    host: host.clone(),
                    target: folder.target.clone(),
                });
            }
        }
    }
    best
}

/// Where git for `dir` runs.
///
/// Reads the disk and the store (blocking): call it off the async workers. When the panes can't be read and
/// never were, the answer is [`Place::Unknown`]: no git runs anywhere until they can.
pub fn place(dir: &str) -> Place {
    thread_local! {
        /// Set while this thread reads the panes.
        static READING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    let asked = spellings(Path::new(dir));
    #[cfg(test)]
    if let Some(inside) = inside_of(&asked, &state().lock().for_tests) {
        return Place::Inside(inside);
    }
    let (store, stale) = {
        let s = state().lock();
        (s.store.clone(), s.read_at.is_none_or(|at| at.elapsed() >= REUSE))
    };
    let Some(store) = store else {
        return Place::Host;
    };
    if stale {
        // The panes are read with no lock held. Asked again from inside that reading: not known.
        if READING.replace(true) {
            return Place::Unknown;
        }
        let read = folders_of(&store);
        READING.set(false);
        let mut s = state().lock();
        match read {
            Ok(folders) => s.folders = Some(folders),
            Err(e) => tracing::warn!(error = %e, "container folders: can't read the panes; keeping the last list"),
        }
        s.read_at = Some(Instant::now());
    }
    let s = state().lock();
    match &s.folders {
        Some(folders) => inside_of(&asked, folders).map_or(Place::Host, Place::Inside),
        None => Place::Unknown,
    }
}

/// Whether a container agent can write `dir`: it is a folder bound writable into an agent's container, or
/// inside one. Yes while that can't be told. Reads the disk and the store (see [`place`]).
pub fn written_by_a_container_agent(dir: &Path) -> bool {
    !matches!(place(&dir.to_string_lossy()), Place::Host)
}

/// Count `folder` as bound writable into `container` at `target`, for the tests of this crate (each test
/// names its own temp folder and container).
#[cfg(test)]
pub(crate) fn add_for_test(folder: &Path, container: &str, target: &str) {
    state().lock().for_tests.push(Folder { hosts: spellings(folder), container: container.to_string(), target: target.to_string() });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(host: &str, container: &str, target: &str) -> Folder {
        Folder { hosts: vec![host.to_string()], container: container.to_string(), target: target.to_string() }
    }

    fn asked(dir: &str) -> Vec<String> {
        vec![dir.to_string()]
    }

    #[test]
    fn a_folder_inside_a_bound_one_is_the_same_folder_in_the_container() {
        let folders = [folder("/w/aria", "agentmux-a", "/workspace"), folder("/data/shared", "agentmux-a", "/data/")];
        let top = inside_of(&asked("/w/aria"), &folders).unwrap();
        assert_eq!((top.container.as_str(), top.dir.as_str()), ("agentmux-a", "/workspace"));
        assert_eq!(inside_of(&asked("/w/aria/repo/src"), &folders).unwrap().dir, "/workspace/repo/src");
        assert_eq!(inside_of(&asked("/data/shared/x"), &folders).unwrap().dir, "/data/x");
        assert_eq!(inside_of(&asked("/w/aria2"), &folders), None, "a sibling with a longer name is outside");
        assert_eq!(inside_of(&asked("/w"), &folders), None, "the folder above is outside");
    }

    #[test]
    fn the_deepest_bound_folder_holds_it_and_either_spelling_finds_it() {
        let folders = [folder("/w", "agentmux-outer", "/workspace"), folder("/w/aria", "agentmux-inner", "/workspace")];
        assert_eq!(inside_of(&asked("/w/aria/repo"), &folders).unwrap().container, "agentmux-inner");
        assert_eq!(inside_of(&asked("/w/other"), &folders).unwrap().container, "agentmux-outer");
        // Written outside every bound folder, resolving into one: the container's.
        let by_link = inside_of(&["/links/aria/repo".to_string(), "/w/aria/repo".to_string()], &folders).unwrap();
        assert_eq!((by_link.container.as_str(), by_link.dir.as_str()), ("agentmux-inner", "/workspace/repo"));
        // Written inside one and resolving outside every one: still the container's, at the place it is written.
        let out = inside_of(&["/w/aria/link".to_string(), "/elsewhere".to_string()], &folders).unwrap();
        assert_eq!(out.dir, "/workspace/link");
    }

    #[test]
    fn a_path_inside_the_container_names_its_host_folder_only_inside_the_bound_one() {
        let inside = inside_of(&asked("/w/aria/repo/src"), &[folder("/w/aria", "agentmux-a", "/workspace")]).unwrap();
        assert_eq!(inside.host_path("/workspace/repo").as_deref(), Some("/w/aria/repo"));
        assert_eq!(inside.host_path("/workspace").as_deref(), Some("/w/aria"));
        assert_eq!(inside.host_path("/workspace/repo/../other/").as_deref(), Some("/w/aria/other"));
        assert_eq!(inside.host_path("/workspace2/repo"), None);
        assert_eq!(inside.host_path("/home/agent/repo"), None);
        assert_eq!(inside.host_path("/workspace/../etc"), None, "a path that leaves the bound folder names nothing");
    }

    #[test]
    fn a_folder_is_spelled_without_looking_at_the_disk() {
        let base = std::env::temp_dir().join("agentmux-cf-spelled-nowhere");
        let odd = base.join("a").join("..").join("b").join(".");
        assert_eq!(spelled(&odd), spelled(&base.join("b")));
        assert_eq!(resolved(&odd), None, "it is not there");
    }

    fn pane(mode: &str, agent: &str, cwd: &str, volumes: &str) -> Block {
        let mut meta = obj::MetaMapType::new();
        meta.insert("agentMode".to_string(), serde_json::json!(mode));
        meta.insert("agentId".to_string(), serde_json::json!(agent));
        meta.insert(META_KEY_CMD_CWD.to_string(), serde_json::json!(cwd));
        meta.insert("agent:container_volumes".to_string(), serde_json::json!(volumes));
        Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() }
    }

    #[test]
    fn the_folders_are_what_the_container_agents_of_the_store_can_write() {
        let store = Store::open_in_memory().unwrap();
        let (work, shared, docs, host) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let text = |d: &tempfile::TempDir| d.path().to_string_lossy().replace('\\', "/");
        let volumes = serde_json::json!([format!("{}:/data", text(&shared)), format!("{}:/docs:ro", text(&docs))]).to_string();
        store.insert(&mut pane("container", "aria", &text(&work), &volumes)).unwrap();
        store.insert(&mut pane("host", "hugo", &text(&host), "[]")).unwrap();
        let folders = folders_of(&store).unwrap();

        let deep = work.path().join("repo").join("src");
        std::fs::create_dir_all(&deep).unwrap();
        let at = |dir: &Path| inside_of(&spellings(dir), &folders);
        let in_work = at(&deep).expect("the workspace is the agent's");
        assert_eq!((in_work.container.as_str(), in_work.dir.as_str()), ("agentmux-aria", "/workspace/repo/src"));
        assert_eq!(at(shared.path()).expect("a writable volume is the agent's").dir, "/data");
        assert_eq!(at(docs.path()), None, "a read-only volume is not");
        assert_eq!(at(host.path()), None, "a host agent's folder is not");
        let own = |dir: &Path| dir.starts_with(work.path());
        let memory = memory_of(&store, &pane("container", "aria", &text(&work), "[]").meta, &own).expect("the agent has a memory folder");
        assert_eq!(at(&memory).expect("its memory folder is the agent's").dir, crate::backend::container::mounts::container_memory_dir());
        assert_eq!(memory_of(&store, &pane("host", "hugo", &text(&host), "[]").meta, &own), None);
    }
}
