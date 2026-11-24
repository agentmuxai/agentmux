// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A host agent whose folder a container agent can write.
//!
//! A container agent changes the files of its workspace and of every writable volume. A host agent that works in
//! one of those folders, or in a folder that holds one, runs what the container agent left there, as the person.
//! Where a host agent is opened or created, or a container agent's workspace or volumes are saved, so that the two
//! share a folder, the person is asked and the choice is kept for that pair of agents. Nothing but the person's
//! window records the choice ([`accept`]); a caller that is not the person is refused with [`refusal`].
//!
//! A choice is a file, and a file is not proof that the person made it: each go-ahead is also recorded as a row of
//! the audit log made by the window, written before the file, and a choice file with no such row does not count
//! ([`accepted`]): the pair asks again and says why ([`Overlap::untrusted`]).
//!
//! The folders are those of the two agents' definitions: a container agent's workspace and its writable host
//! volumes (`container::mounts::writable`), a host agent's working folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::backend::obj::MetaMapType;
use crate::backend::storage::agents::AgentDefinition;
use crate::backend::storage::store::Store;

/// A host agent and a container agent whose folders overlap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct Overlap {
    pub host_id: String,
    pub host_name: String,
    pub container_id: String,
    pub container_name: String,
    /// A choice for this pair is kept, but the audit log has no record that the person made it, so it does not
    /// count and the pair asks again.
    #[serde(default)]
    pub untrusted: bool,
}

/// The folder an agent definition works in as set, expanded; `None` when it names none.
fn working_folder(def: &AgentDefinition) -> Option<PathBuf> {
    let dir = def.working_directory.trim();
    (!dir.is_empty()).then(|| crate::backend::base::expand_home_dir_safe(dir))
}

/// The host folders a container agent's definition binds writable: its workspace and its volumes that are host
/// folders not bound read-only. Asked of the same rule a running pane's container is made by.
fn writable_folders(def: &AgentDefinition) -> Vec<PathBuf> {
    if !crate::backend::runtime_adapter::RuntimeKind::of_agent_mode(&def.agent_type).is_container() {
        return Vec::new();
    }
    let mut meta = MetaMapType::new();
    meta.insert("agentMode".to_string(), serde_json::json!("container"));
    meta.insert(crate::backend::blockcontroller::META_KEY_CMD_CWD.to_string(), serde_json::json!(def.working_directory));
    meta.insert("agent:container_volumes".to_string(), serde_json::json!(def.container_volumes));
    crate::backend::container::mounts::writable::writable_binds(&meta, None).into_iter().map(|bind| bind.host).collect()
}

fn is_container(def: &AgentDefinition) -> bool {
    crate::backend::runtime_adapter::RuntimeKind::of_agent_mode(&def.agent_type).is_container()
}

fn between(host: &AgentDefinition, folder: &Path, container: &AgentDefinition) -> Option<Overlap> {
    writable_folders(container)
        .iter()
        .any(|bound| crate::backend::fs_ops::container_folders::overlap(folder, bound))
        .then(|| Overlap {
            host_id: host.id.clone(),
            host_name: host.name.clone(),
            container_id: container.id.clone(),
            container_name: container.name.clone(),
            untrusted: untrusted(&host.id, &container.id),
        })
}

/// The container agents whose writable folders overlap `folder`, the folder the host agent `host` works in.
pub fn for_host_in(store: &Store, host: &AgentDefinition, folder: &Path) -> Vec<Overlap> {
    if is_container(host) {
        return Vec::new();
    }
    let all = store.agent_def_list().unwrap_or_default();
    all.iter().filter(|other| other.id != host.id && is_container(other)).filter_map(|c| between(host, folder, c)).collect()
}

/// The overlaps of `def` as it is saved or opened: for a host agent, with the container agents; for a container
/// agent, with the host agents. `host_folder` is the folder a host agent opens in when it is not the one its
/// definition names (a blank one opens in the agent's own folder).
pub fn of(store: &Store, def: &AgentDefinition, host_folder: Option<&Path>) -> Vec<Overlap> {
    if !is_container(def) {
        let folder = host_folder.map(Path::to_path_buf).or_else(|| working_folder(def));
        return folder.map_or_else(Vec::new, |folder| for_host_in(store, def, &folder));
    }
    let all = store.agent_def_list().unwrap_or_default();
    all.iter()
        .filter(|other| other.id != def.id && !is_container(other))
        .filter_map(|host| working_folder(host).and_then(|folder| between(host, &folder, def)))
        .collect()
}

/// The directory the choices are kept in; a test sets its own.
fn choices_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return CHOICES_DIR_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    #[cfg(not(any(test, feature = "test-support")))]
    return crate::registry::paths::resolve_shared_folder_overlap_choices_dir();
}

#[cfg(any(test, feature = "test-support"))]
static CHOICES_DIR_FOR_TEST: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Sets the folder a test's choices are kept in, and returns the one set before.
#[cfg(any(test, feature = "test-support"))]
pub fn set_choices_dir_for_test(dir: Option<PathBuf>) -> Option<PathBuf> {
    std::mem::replace(&mut *CHOICES_DIR_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()), dir)
}

fn choice_file(host_id: &str, container_id: &str) -> Option<PathBuf> {
    let key = agentmux_common::mcp_config_path::spelling_key(&format!("{host_id}\n{container_id}"));
    Some(choices_dir()?.join(key))
}

/// The audit action that records the person's go-ahead for a pair.
pub const GO_AHEAD_ACTION: &str = "folder-overlap.go-ahead";

/// The note of an audit row about a pair: the action and the two agents' ids, which find the row (a name changes), then
/// the names, for whoever reads the Audit view.
pub fn row_note(action: &str, host: (&str, &str), container: (&str, &str)) -> String {
    format!("{action} {} {}: host agent {} and container agent {}", host.0, container.0, host.1, container.1)
}

fn row_prefix(host_id: &str, container_id: &str) -> String {
    format!("{GO_AHEAD_ACTION} {host_id} {container_id}:")
}

/// Whether `entry` is the window's record of a go-ahead for the pair `prefix` names: an action row, made by the window
/// (no agent made it), that went through.
fn is_go_ahead_row(entry: &crate::backend::reactive::AuditLogEntry, prefix: &str) -> bool {
    entry.event_kind == "action"
        && entry.outcome.as_deref() == Some(GO_AHEAD_ACTION)
        && entry.source_agent.is_none()
        && entry.success
        && entry.reason.as_deref().is_some_and(|reason| reason.starts_with(prefix))
}

/// The audit files to read for a record: the one every srv on this machine shares, and this channel's own.
fn audit_files() -> Vec<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return AUDIT_FILES_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    #[cfg(not(any(test, feature = "test-support")))]
    return [super::reactive::audit_store::shared_path_for_reading(), super::reactive::audit_store::own_path()].into_iter().flatten().collect();
}

#[cfg(any(test, feature = "test-support"))]
static AUDIT_FILES_FOR_TEST: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Sets the audit files a test reads a record from (none by default), and returns the ones set before.
#[cfg(any(test, feature = "test-support"))]
pub fn set_audit_files_for_test(files: Vec<PathBuf>) -> Vec<PathBuf> {
    std::mem::replace(&mut *AUDIT_FILES_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()), files)
}

/// Whether any line of the audit file at `path` is the record `prefix` names. The file is never rotated, so a line
/// is parsed only when it holds the prefix.
fn file_has_record(path: &Path, prefix: &str) -> bool {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else { return false };
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| line.contains(prefix))
        .filter_map(|line| serde_json::from_str::<crate::backend::reactive::AuditLogEntry>(&line).ok())
        .any(|entry| is_go_ahead_row(&entry, prefix))
}

/// The pairs whose go-ahead was found on record in this process, as `host\ncontainer`: a record does not go away, so it
/// is looked for once.
fn recorded_pairs() -> &'static Mutex<HashSet<String>> {
    static RECORDED: std::sync::OnceLock<Mutex<HashSet<String>>> = std::sync::OnceLock::new();
    RECORDED.get_or_init(Mutex::default)
}

fn recorded_key(host_id: &str, container_id: &str) -> String {
    format!("{host_id}\n{container_id}")
}

/// Whether the audit log records the person's go-ahead for the pair: in the rows this srv still holds in memory (the
/// row is on its way to the file when the person has only just gone ahead), or in the audit file, which survives a
/// restart.
fn recorded(host_id: &str, container_id: &str) -> bool {
    let key = recorded_key(host_id, container_id);
    if recorded_pairs().lock().unwrap_or_else(|e| e.into_inner()).contains(&key) {
        return true;
    }
    let prefix = row_prefix(host_id, container_id);
    let in_memory = super::reactive::get_global_handler().get_audit_log(usize::MAX).iter().any(|entry| is_go_ahead_row(entry, &prefix));
    let found = in_memory || audit_files().iter().any(|file| file_has_record(file, &prefix));
    if found {
        recorded_pairs().lock().unwrap_or_else(|e| e.into_inner()).insert(key);
    }
    found
}

fn has_choice_file(host_id: &str, container_id: &str) -> bool {
    choice_file(host_id, container_id).is_some_and(|file| file.is_file())
}

/// Whether the person has gone ahead with this host agent and this container agent sharing a folder: a choice is kept
/// and the audit log records that the person made it.
pub fn accepted(host_id: &str, container_id: &str) -> bool {
    has_choice_file(host_id, container_id) && recorded(host_id, container_id)
}

/// Whether a choice is kept for the pair that the audit log has no record of: it does not count.
pub fn untrusted(host_id: &str, container_id: &str) -> bool {
    has_choice_file(host_id, container_id) && !recorded(host_id, container_id)
}

/// Keeps the person's choice for the pair, naming both agents by their ids. Only the person's window calls this
/// (`app_api::folder_overlap`).
pub fn accept(host_id: &str, container_id: &str) -> Result<(), String> {
    accept_as((host_id, host_id), (container_id, container_id))
}

/// [`accept`], with the agents' names (id, name) for the record. The audit row is written first and the file after, so a
/// file is never kept without the record of who made it.
pub fn accept_as(host: (&str, &str), container: (&str, &str)) -> Result<(), String> {
    let file = choice_file(host.0, container.0).ok_or("the choice can't be kept: AgentMux has no shared folder here")?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("the choice can't be kept: {e}"))?;
    }
    let note = row_note(GO_AHEAD_ACTION, host, container);
    super::reactive::get_global_handler().log_fleet_action_audit(None, host.1, "", GO_AHEAD_ACTION, true, None, &uuid::Uuid::new_v4().to_string(), Some(&note));
    recorded_pairs().lock().unwrap_or_else(|e| e.into_inner()).insert(recorded_key(host.0, container.0));
    std::fs::write(&file, format!("{}\n{}\n", host.0, container.0)).map_err(|e| format!("the choice can't be kept: {e}"))
}

/// A choice file with no audit row, as an agent that wrote the folder would leave it.
#[cfg(any(test, feature = "test-support"))]
pub fn write_choice_without_a_record_for_test(host_id: &str, container_id: &str) {
    let file = choice_file(host_id, container_id).expect("a choices folder is set");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, format!("{host_id}\n{container_id}\n")).unwrap();
}

/// Forgets which pairs were found on record, as a restart does.
#[cfg(any(test, feature = "test-support"))]
pub fn forget_what_was_recorded_for_test() {
    recorded_pairs().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Forgets every choice the person made for a pair that includes agent `id`: the agent is deleted.
pub fn forget_agent(id: &str) {
    recorded_pairs().lock().unwrap_or_else(|e| e.into_inner()).retain(|key| !key.lines().any(|line| line == id));
    let Some(dir) = choices_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if std::fs::read_to_string(&path).is_ok_and(|text| text.lines().any(|line| line == id)) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The overlaps the person has not gone ahead with.
pub fn unchosen(overlaps: Vec<Overlap>) -> Vec<Overlap> {
    overlaps.into_iter().filter(|o| !accepted(&o.host_id, &o.container_id)).collect()
}

/// What the person is told, and what a caller that is not the person is refused with: both agents, what the shared
/// folder means, and what to do instead.
pub fn refusal(overlaps: &[Overlap]) -> String {
    let pairs = overlaps
        .iter()
        .map(|o| format!("{} (host) and {} (container)", o.host_name, o.container_name))
        .collect::<Vec<_>>()
        .join("; ");
    let untrusted = overlaps.iter().filter(|o| o.untrusted).map(|o| format!("{} and {}", o.host_name, o.container_name)).collect::<Vec<_>>().join("; ");
    let why = if untrusted.is_empty() {
        String::new()
    } else {
        format!(" A choice is kept for {untrusted}, but AgentMux has no record that the person made it, so it does not count.")
    };
    format!(
        "{pairs} share a folder, so the container agent can change what the host agent runs.{why} Only the person can go ahead with that, from the AgentMux window. Give them separate workspaces, or bind the shared folder read-only into the container agent (a volume ending in :ro)"
    )
}

/// `Err` with [`refusal`] when `def`, as saved or opened, shares a folder with an agent of the other kind that
/// the person has not gone ahead with.
pub fn check(store: &Store, def: &AgentDefinition, host_folder: Option<&Path>) -> Result<(), String> {
    let pending = unchosen(of(store, def, host_folder));
    if pending.is_empty() { Ok(()) } else { Err(refusal(&pending)) }
}

/// What an `agent.open` refusal for a shared folder starts with, so the window knows to ask the person and try again.
pub const OPEN_REFUSAL_CODE: &str = "SHARED_FOLDER";

/// The folder `agent.open` runs a host agent in, expanded: the one it names, else its own folder.
pub fn open_folder(def: &AgentDefinition) -> PathBuf {
    let named = def.working_directory.trim();
    let dir = if named.is_empty() { crate::backend::storage::agents::default_agent_working_dir(&def.name) } else { named.to_string() };
    crate::backend::base::expand_home_dir_safe(&dir)
}

/// [`check`] for an agent being opened, in [`open_folder`], with the refusal marked [`OPEN_REFUSAL_CODE`]. A container
/// agent is not refused here: it is asked when its own folders are saved.
pub fn check_open(store: &Store, def: &AgentDefinition) -> Result<(), String> {
    if is_container(def) {
        return Ok(());
    }
    check(store, def, Some(&open_folder(def))).map_err(|refusal| format!("{OPEN_REFUSAL_CODE}: {refusal}"))
}

/// Whether a save of `new` over `old` changes a folder an agent works in or can write.
pub fn folders_changed(old: &AgentDefinition, new: &AgentDefinition) -> bool {
    old.working_directory != new.working_directory || old.agent_type != new.agent_type || old.container_volumes != new.container_volumes
}

/// Whether `host_id` names a host agent and `container_id` a container agent.
pub fn is_a_pair(store: &Store, host_id: &str, container_id: &str) -> bool {
    let kind = |id: &str| store.agent_def_get(id).ok().flatten().map(|def| is_container(&def));
    kind(host_id) == Some(false) && kind(container_id) == Some(true)
}

#[cfg(test)]
#[path = "folder_overlap_tests.rs"]
mod tests;
