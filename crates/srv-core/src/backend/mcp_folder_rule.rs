// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP servers of a host agent whose folder a container agent can write.
//!
//! A host agent's CLI starts the MCP servers of its config on this computer. Where a container agent can write
//! the agent's working folder or a folder beneath it, or has run in that folder, the host agent gets only the
//! servers AgentMux writes:
//!
//! - its config is kept outside the folder (`mcp_config_file::host_config_path`), and the folder's own
//!   `.mcp.json` is neither merged into it nor handed to a harness (`mcp_config_file::read_for_turn`);
//! - a Claude Code turn is told to use that config alone (`agent_mcp_flag`), and the config names the
//!   `agentmux` command by its path ([`with_the_command_by_path`]);
//! - any other harness, and any agent whose CLI is started by a terminal, session or app-server controller,
//!   is not started there ([`refusal`], [`turn_refusal`]): what it reads from its folder is its own, and
//!   AgentMux has no switch for it;
//! - the pane says which servers of the folder's file were left out ([`left_out`], `cli_notice`).
//!
//! "Can write" is asked of [`container_folders`](crate::backend::fs_ops::container_folders), as the Editor's
//! language servers ask it; where that can't be told, the rule holds. "Has run in it" is a mark kept in the
//! shared folder ([`mark`]), so the rule still holds when the agent is switched to the host, or another host agent
//! is given the folder, a folder in it or a folder that holds it. A folder no container agent has to do with
//! is not touched by any of this.

use std::path::{Path, PathBuf};

use agentmux_common::mcp_config_path as shared;
use serde_json::Value;

use crate::backend::fs_ops::container_folders::{holds_a_container_agents_folder, place, Place};
use crate::backend::obj::{meta_get_string, MetaMapType};

/// A test's own folder for the marks, set with its data home (`mcp_config_file::with_data_home`). A test
/// build never reads or writes the real shared folder: with none set there are no marks.
#[cfg(any(test, feature = "test-support"))]
static MARKS_DIR_FOR_TEST: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Sets the folder a test's marks are kept in, and returns the one set before.
#[cfg(any(test, feature = "test-support"))]
pub fn set_marks_dir_for_test(dir: Option<PathBuf>) -> Option<PathBuf> {
    std::mem::replace(&mut *MARKS_DIR_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()), dir)
}

/// The folder the marks are kept in: in the shared folder (`resolve_shared_container_agent_folders_dir`), so
/// every version and channel of an install reads the same ones, an upgrade and the removal of an old version
/// leave them, and so does the deletion of an agent. It is bound into no container. One file per spelling of
/// a marked folder, named by that spelling's key; it holds the folder's spellings, one a line.
fn marks_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    return MARKS_DIR_FOR_TEST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    #[cfg(not(any(test, feature = "test-support")))]
    return crate::registry::paths::resolve_shared_container_agent_folders_dir();
}
/// The most servers a pane row names, and the most characters of one name: they are read from the folder's
/// own file.
const MOST_NAMES: usize = 20;
const LONGEST_NAME: usize = 64;

fn expanded(work_dir: &str) -> String {
    crate::backend::blockcontroller::core::expand_home_dir(work_dir)
}

/// The spellings `expanded` is compared under: as written and as the filesystem names it, so one folder
/// written another way (letter case and a trailing separator on Windows, a link or junction to it) is the
/// same folder here.
fn spellings(expanded: &str) -> Vec<String> {
    shared::spellings(Path::new(expanded))
}

fn mark_file(spelling: &str) -> Option<PathBuf> {
    Some(marks_dir()?.join(shared::spelling_key(spelling)))
}

/// Record that a container agent runs with `work_dir` as its folder. The mark stays: from now on a host agent
/// in that folder, in one beneath it or in one that holds it gets only the servers AgentMux writes.
/// Best-effort; a failure is logged.
pub fn mark(work_dir: &str) {
    if work_dir.trim().is_empty() {
        return;
    }
    let all = spellings(&expanded(work_dir));
    let text = all.join("\n");
    for path in all.iter().filter_map(|spelling| mark_file(spelling)) {
        if path.is_file() {
            continue;
        }
        // Whole or not at all: a reader in another AgentMux never sees half a mark.
        let half = path.with_extension(format!("{}.tmp", std::process::id()));
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&half, &text))
            .and_then(|()| std::fs::rename(&half, &path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&half);
            tracing::warn!(work_dir, error = %e, "the folder could not be marked as a container agent's");
        }
    }
}

/// [`mark`] the folder of an agent whose definition just changed from `old` to `new`, when that moved it from
/// a container to the host: the folder was the container's until now. An agent with no folder of its own named
/// is marked by its turns in the container.
pub fn mark_when_moved_to_the_host(old: &crate::backend::storage::AgentDefinition, new: &crate::backend::storage::AgentDefinition) {
    if old.runtime().is_container() && !new.runtime().is_container() {
        mark(&old.working_directory);
    }
}

/// Every spelling a mark records.
fn marked_folders() -> Vec<String> {
    let Some(entries) = marks_dir().and_then(|dir| std::fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().extension().is_none())
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .flat_map(|text| text.lines().map(str::to_string).collect::<Vec<_>>())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Whether `expanded` carries the mark, lies in a folder that does, or holds one that does.
fn marked(expanded: &str) -> bool {
    let asked = spellings(expanded);
    if asked.iter().any(|spelling| mark_file(spelling).is_some_and(|path| path.is_file())) {
        return true;
    }
    let recorded = marked_folders();
    asked.iter().any(|a| recorded.iter().any(|r| shared::spelled_within(a, r) || shared::spelled_within(r, a)))
}

/// Whether a host agent in `work_dir` gets only the servers AgentMux writes: a container agent can write the
/// folder or one beneath it, or has run in it, or that can't be told. Reads the disk and the store.
pub fn holds_in(work_dir: &str) -> bool {
    if work_dir.trim().is_empty() {
        return false;
    }
    let dir = expanded(work_dir);
    decided(&place(&dir), holds_a_container_agents_folder(&dir), marked(&dir))
}

/// The rule from its three answers: where the folder is, whether a container agent's folder lies beneath it,
/// and whether it carries the mark. Anything but a plain host folder, with nothing beneath and no mark, has it.
fn decided(place: &Place, holds_one: bool, marked: bool) -> bool {
    !matches!(place, Place::Host) || holds_one || marked
}

/// The working folder of the host agent on a pane with `meta`, expanded, when the rule holds for it.
pub fn folder_of(meta: &MetaMapType) -> Option<String> {
    if crate::backend::runtime_adapter::RuntimeKind::of_block_meta(meta).is_container() {
        return None;
    }
    let work_dir = meta_get_string(meta, "cmd:cwd", "");
    holds_in(&work_dir).then(|| expanded(&work_dir))
}

/// The pane's harness as its meta names it: empty for a pane that is no agent's. A name no harness answers
/// to is still an agent's, so it is refused like the rest.
fn provider_of(meta: &MetaMapType) -> String {
    meta_get_string(meta, "agentProvider", "").trim().to_string()
}

/// Why the agent on a pane with `meta` is not started on this computer by a controller that starts its CLI
/// itself (a terminal agent, a session agent, an app-server agent): the rule holds for its folder, and nothing
/// on that path keeps a CLI to AgentMux's config, whatever the harness. `None` for a container agent, for a
/// pane that is no agent's, and for every other folder.
pub fn refusal(meta: &MetaMapType) -> Option<String> {
    if provider_of(meta).is_empty() {
        return None;
    }
    folder_of(meta)?;
    Some(REFUSAL.to_string())
}

/// `Err` with [`refusal`]'s reason when the agent on a pane with `meta` is not started on this computer: the
/// first line of a controller's start whose CLI starts with it.
pub fn not_refused(meta: &MetaMapType) -> Result<(), String> {
    refusal(meta).map_or(Ok(()), Err)
}

/// Why a turn of the agent on a pane with `meta` is not started on this computer: as [`refusal`], except for
/// Claude Code, whose turn goes on because the flags it is started with keep it to AgentMux's config
/// (`agent_mcp_flag`). Asked only where those flags are added (`agent_runtime::with_runtime_flags`): the turn
/// of a turn-based agent, and the start of its process ahead of a turn.
pub fn turn_refusal(meta: &MetaMapType) -> Option<String> {
    if crate::backend::providers::resolve_provider_alias(&provider_of(meta)) == "claude" {
        return None;
    }
    refusal(meta).map(|_| TURN_REFUSAL.to_string())
}

/// What a pane is told when [`refusal`] stops its agent.
pub const REFUSAL: &str = "A container agent can write in this agent's folder, or has run in it, so this agent is not started on this computer there: an agent that runs as a terminal or as a session can't be kept to the settings and MCP servers set in AgentMux. Run this agent in a container, or give it a folder no container agent can write.";

/// What a pane is told when [`turn_refusal`] stops its agent's turn.
pub const TURN_REFUSAL: &str = "A container agent can write in this agent's folder, or has run in it, so this agent is not started on this computer there: only Claude Code can be kept to the settings and MCP servers set in AgentMux. Run this agent in a container, or give it a folder no container agent can write.";

/// Whether `exe` may be named as a command in the config of a host agent in the folder spelled `folder`: an
/// absolute path that is not in that folder and that no container agent can write.
fn outside(exe: &Path, folder: &[String]) -> bool {
    let Some(parent) = exe.parent() else { return false };
    exe.is_absolute()
        && !shared::spellings(exe).iter().any(|e| folder.iter().any(|f| shared::spelled_within(e, f)))
        && matches!(place(&parent.to_string_lossy()), Place::Host)
}

/// Whether `exe` may be named as a command to a host agent of `work_dir` ([`outside`]).
pub fn is_outside(exe: &Path, work_dir: &str) -> bool {
    outside(exe, &spellings(&expanded(work_dir)))
}

/// `config` (the text of an MCP config) with every command named by a path: the `agentmux` entry's is `exe`,
/// and any other server's bare name is what `found` answers for it. An entry whose command has no such path
/// is taken out, and its name is returned. A command already written as an absolute path is the person's own
/// choice and stays. A config that is not a JSON object with servers is returned as it is.
fn by_path(config: &str, exe: Option<&Path>, found: &dyn Fn(&str) -> Option<PathBuf>) -> (String, Vec<String>) {
    let Ok(was) = serde_json::from_str::<Value>(config) else { return (config.to_string(), Vec::new()) };
    let mut doc = was.clone();
    let Some(servers) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) else { return (config.to_string(), Vec::new()) };
    let mut taken_out = Vec::new();
    for (name, entry) in servers.iter_mut() {
        let Some(entry) = entry.as_object_mut() else { continue };
        let path = if name == "agentmux" {
            exe.map(Path::to_path_buf)
        } else {
            match entry.get("command").and_then(Value::as_str) {
                // A server reached over the network has no command to name.
                None => continue,
                Some(command) if Path::new(command).is_absolute() => continue,
                Some(command) => found(command),
            }
        };
        match path {
            Some(path) => {
                entry.insert("command".to_string(), Value::String(path.to_string_lossy().into_owned()));
            }
            None => taken_out.push(name.clone()),
        }
    }
    for name in &taken_out {
        servers.remove(name);
    }
    if doc == was {
        return (config.to_string(), taken_out);
    }
    (serde_json::to_string_pretty(&doc).unwrap_or_else(|_| config.to_string()), taken_out)
}

/// The program a bare command name stands for on this computer, looked up on srv's own `PATH` and never in a
/// working folder; `None` for a name with a folder in it (a relative path) or one that is not found.
fn on_path(command: &str) -> Option<PathBuf> {
    if command.contains(['/', '\\']) {
        return None;
    }
    which::which_in(command, std::env::var_os("PATH"), std::env::temp_dir()).ok().filter(|path| path.is_absolute())
}

/// `config` (the text of a host agent's MCP config) as it is written for `work_dir`, a folder the rule holds
/// for: the `agentmux` entry's command is the absolute path of the `agentmux-mcp` that ships with this
/// install, never a name. Where no such file ships outside the folder, the entry is taken out: no server
/// rather than one found by name.
pub fn with_the_command_by_path(config: &str, work_dir: &str) -> String {
    with_commands_by_path(config, work_dir).0
}

/// [`with_the_command_by_path`], and the same for every other server of the config: a server the person set
/// with a bare command (`npx`, `uvx`) is named by the path that name has on this computer's `PATH`, outside
/// the folder. A server whose command has no such path is taken out, and the names of those are returned
/// (the `agentmux` entry is not among them: it is AgentMux's own).
pub fn with_commands_by_path(config: &str, work_dir: &str) -> (String, Vec<String>) {
    let folder = spellings(&expanded(work_dir));
    let exe = crate::backend::mcp_rewrite::shipped_exe().filter(|exe| outside(exe, &folder));
    if exe.is_none() {
        tracing::warn!(work_dir, "no agentmux-mcp ships outside this folder: the agent's config is written without the agentmux server");
    }
    let (text, mut taken_out) = by_path(config, exe.as_deref(), &|command| on_path(command).filter(|path| outside(path, &folder)));
    taken_out.retain(|name| name != "agentmux");
    if !taken_out.is_empty() {
        tracing::warn!(work_dir, count = taken_out.len(), "MCP servers set with a command that has no path outside this folder are left out of the agent's config");
    }
    (text, taken_out)
}

/// Whether `config` (the text of an MCP config) names every command it has by an absolute path (a server
/// reached over the network has none): what a host turn in a folder the rule holds for is given, and nothing
/// else.
pub fn names_the_command_by_path(config: &str) -> bool {
    let Ok(doc) = serde_json::from_str::<Value>(config) else { return false };
    let Some(servers) = doc.get("mcpServers").and_then(Value::as_object) else { return true };
    servers.values().all(|entry| match entry.get("command") {
        None => true,
        Some(command) => command.as_str().is_some_and(|command| Path::new(command).is_absolute()),
    })
}

/// The text of `folder`'s own `.mcp.json` for a host agent that is defined from the folder (an import), which
/// keeps the folder's servers as its own. `None` where the rule holds: the agent takes no server from the file.
pub fn config_to_import(folder: &Path) -> Option<String> {
    if holds_in(&folder.to_string_lossy()) {
        return None;
    }
    std::fs::read_to_string(folder.join(crate::backend::mcp_config_file::MCP_JSON)).ok()
}

/// `name` as a pane row shows it: no control characters, and no longer than a name needs to be.
fn shown(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(LONGEST_NAME).collect()
}

/// The names of the `mcpServers` of `config` (a `.mcp.json`'s text).
fn server_names(config: &str) -> Vec<String> {
    serde_json::from_str::<Value>(config)
        .ok()
        .and_then(|doc| doc.get("mcpServers").and_then(Value::as_object).map(|servers| servers.keys().cloned().collect()))
        .unwrap_or_default()
}

/// The servers of `folder`'s own `.mcp.json` that the agent's config (`ours`, its text) does not have: the
/// ones a host agent there is not given. By name, sorted, at most [`MOST_NAMES`]. Only reads the file.
pub fn left_out(folder: &str, ours: Option<&str>) -> Vec<String> {
    let Ok(theirs) = std::fs::read_to_string(Path::new(folder).join(crate::backend::mcp_config_file::MCP_JSON)) else {
        return Vec::new();
    };
    let ours = ours.map(server_names).unwrap_or_default();
    let mut names: Vec<String> = server_names(&theirs).into_iter().filter(|name| !ours.contains(name)).map(|name| shown(&name)).filter(|name| !name.is_empty()).collect();
    names.sort();
    names.dedup();
    names.truncate(MOST_NAMES);
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fs_ops::container_folders::add_for_test;
    use crate::backend::mcp_config_file::with_data_home;
    use serde_json::json;

    fn pane(mode: &str, provider: &str, cwd: &Path) -> MetaMapType {
        [("agentMode", json!(mode)), ("agentProvider", json!(provider)), ("cmd:cwd", json!(cwd.to_string_lossy()))]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    #[test]
    fn the_rule_holds_where_a_container_agent_can_write_and_nowhere_else() {
        let (data, bound, own) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        add_for_test(bound.path(), "agentmux-rule-a", "/workspace");
        with_data_home(data.path(), || {
            assert!(holds_in(&bound.path().to_string_lossy()), "the folder the agent can write");
            assert!(holds_in(&bound.path().join("src").to_string_lossy()), "a folder inside it");
            assert!(!holds_in(&own.path().to_string_lossy()), "a folder no container agent has to do with");
            assert!(!holds_in(""), "no folder, no rule");
        });
    }

    #[test]
    fn the_rule_holds_while_the_folders_place_cant_be_told() {
        assert!(decided(&Place::Unknown, false, false), "unknown: the rule holds");
        assert!(!decided(&Place::Host, false, false));
        assert!(decided(&Place::Host, true, false) && decided(&Place::Host, false, true));
    }

    #[test]
    fn the_rule_holds_for_a_root_that_holds_a_container_agents_folder() {
        let (data, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let inner = root.path().join("agents").join("aria");
        std::fs::create_dir_all(&inner).unwrap();
        add_for_test(&inner, "agentmux-rule-b", "/workspace");
        with_data_home(data.path(), || {
            assert!(holds_in(&root.path().to_string_lossy()), "a container agent's folder lies beneath it");
            assert!(holds_in(&root.path().join("agents").to_string_lossy()));
        });
    }

    #[test]
    fn a_folder_a_container_agent_ran_in_keeps_the_rule() {
        let (data, was) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir = was.path().to_string_lossy().into_owned();
        with_data_home(data.path(), || {
            assert!(!holds_in(&dir));
            mark(&dir);
            mark(&dir);
            assert!(holds_in(&dir), "no container has it now, and the rule still holds");
            // Kept in the marks' own folder: not in the marked folder, and not in the folder of a relocated
            // config, which a container is given (read-only).
            let kept = mark_file(&spellings(&dir)[0]).unwrap();
            assert!(kept.is_file() && !kept.starts_with(was.path()), "{kept:?}");
            let given_to_a_container = crate::backend::mcp_config_file::relocated_dir(&dir).expect("a project's config folder");
            assert!(!kept.starts_with(&given_to_a_container), "{given_to_a_container:?}");
            assert!(!crate::backend::mcp_config_file::host_config_path(&dir).starts_with(&given_to_a_container), "nor is the host agent's config");
        });
    }

    /// The mark is not one version's: made while one version's data root is in use, it holds under another's,
    /// and neither data root has it.
    #[test]
    fn a_mark_made_by_one_version_holds_for_another() {
        let (shared_dir, one, other, was) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir = was.path().to_string_lossy().into_owned();
        fn files_under(root: &Path) -> usize {
            std::fs::read_dir(root).map_or(0, |entries| {
                entries.flatten().map(|e| if e.path().is_dir() { files_under(&e.path()) } else { 1 }).sum()
            })
        }
        with_data_home(one.path(), || {
            let before = set_marks_dir_for_test(Some(shared_dir.path().join("container-agent-folders")));
            mark(&dir);
            // Another version of the same install: its own data root, the same shared folder.
            std::env::set_var("AGENTMUX_DATA_HOME", other.path());
            let holds = holds_in(&dir);
            set_marks_dir_for_test(before);
            assert!(holds, "the mark is read from the shared folder");
            assert_eq!(files_under(one.path()) + files_under(other.path()), 0, "no version's data root holds a mark");
            assert_eq!(files_under(shared_dir.path()), spellings(&dir).len());
        });
    }

    #[test]
    fn only_a_host_agent_is_refused_and_only_there_and_a_turn_lets_claude_code_alone_through() {
        let (data, bound, own) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        add_for_test(bound.path(), "agentmux-rule-c", "/workspace");
        with_data_home(data.path(), || {
            for provider in ["codex", "gemini", "kimi", "muxcode", "opencode"] {
                assert_eq!(turn_refusal(&pane("host", provider, bound.path())).as_deref(), Some(TURN_REFUSAL), "{provider}");
                assert_eq!(turn_refusal(&pane("host", provider, own.path())), None, "{provider} in a folder of its own");
                assert_eq!(turn_refusal(&pane("container", provider, bound.path())), None, "{provider} in the container");
            }
            assert_eq!(turn_refusal(&pane("host", "claude", bound.path())), None, "a Claude Code turn is kept to AgentMux's config instead");
            // Where the CLI starts with its controller there are no such flags: every harness is refused.
            for provider in ["claude", "codex", "gemini"] {
                assert_eq!(refusal(&pane("host", provider, bound.path())).as_deref(), Some(REFUSAL), "{provider}");
                assert_eq!(not_refused(&pane("host", provider, own.path())), Ok(()), "{provider} in a folder of its own");
                assert_eq!(refusal(&pane("container", provider, bound.path())), None, "{provider} in the container");
            }
            assert_eq!(refusal(&pane("host", "", bound.path())), None, "a pane that is no agent's");
            assert_eq!(turn_refusal(&pane("host", "", bound.path())), None);
            assert_eq!(folder_of(&pane("host", "claude", bound.path())).map(PathBuf::from), Some(PathBuf::from(expanded(&bound.path().to_string_lossy()))));
            assert_eq!(folder_of(&pane("container", "claude", bound.path())), None);
        });
    }

    #[test]
    fn the_servers_left_out_are_the_folders_own_by_name() {
        let folder = tempfile::tempdir().unwrap();
        let dir = folder.path().to_string_lossy().into_owned();
        assert!(left_out(&dir, None).is_empty(), "no file, nothing left out");
        let long = "n".repeat(200);
        let theirs = json!({ "mcpServers": { "agentmux": { "command": "other" }, "zeta": {}, "alpha\u{7}": {}, long.as_str(): {} } });
        std::fs::write(folder.path().join(".mcp.json"), theirs.to_string()).unwrap();
        let ours = r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"},"git":{}}}"#;
        assert_eq!(left_out(&dir, Some(ours)), vec!["alpha".to_string(), "n".repeat(LONGEST_NAME), "zeta".to_string()]);
        assert_eq!(left_out(&dir, None).len(), 4, "with no config of its own, every server of the file is left out");
        std::fs::write(folder.path().join(".mcp.json"), "not json").unwrap();
        assert!(left_out(&dir, Some(ours)).is_empty());
        let many: serde_json::Map<String, Value> = (0..40).map(|i| (format!("s{i:02}"), json!({}))).collect();
        std::fs::write(folder.path().join(".mcp.json"), json!({ "mcpServers": many }).to_string()).unwrap();
        assert_eq!(left_out(&dir, Some(ours)).len(), MOST_NAMES);
    }

    #[test]
    fn an_agent_moved_from_a_container_to_the_host_leaves_its_folder_marked() {
        use crate::backend::storage::agents::test_agent_def;
        let (data, moved, stays, never) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let def = |agent_type: &str, folder: &Path| {
            let mut def = test_agent_def("a-1", "Aria", "claude", agent_type, 1, "");
            def.working_directory = folder.to_string_lossy().into_owned();
            def
        };
        with_data_home(data.path(), || {
            mark_when_moved_to_the_host(&def("container", moved.path()), &def("host", moved.path()));
            assert!(holds_in(&moved.path().to_string_lossy()), "the folder was the container's until now");
            mark_when_moved_to_the_host(&def("container", stays.path()), &def("container", stays.path()));
            mark_when_moved_to_the_host(&def("host", never.path()), &def("host", never.path()));
            mark_when_moved_to_the_host(&def("host", never.path()), &def("container", never.path()));
            assert!(!holds_in(&stays.path().to_string_lossy()), "an agent that stays in its container marks its folder by its turns, not here");
            assert!(!holds_in(&never.path().to_string_lossy()));
        });
    }

    #[test]
    fn an_import_takes_the_folders_servers_only_where_no_container_agent_has_to_do_with_it() {
        let (data, own, bound, ran) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let config = r#"{"mcpServers":{"theirs":{"command":"theirs-mcp"}}}"#;
        for folder in [&own, &bound, &ran] {
            std::fs::write(folder.path().join(".mcp.json"), config).unwrap();
        }
        add_for_test(bound.path(), "agentmux-rule-d", "/workspace");
        with_data_home(data.path(), || {
            mark(&ran.path().to_string_lossy());
            assert_eq!(config_to_import(own.path()).as_deref(), Some(config), "as before");
            assert_eq!(config_to_import(bound.path()), None, "a container agent can write it");
            assert_eq!(config_to_import(ran.path()), None, "a container agent ran in it");
        });
    }

    #[cfg(unix)]
    fn link_dir(link: &Path, target: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    /// One mark, and the folder is found however it is written, and so are the folders in it and around it.
    #[test]
    fn the_mark_is_found_however_the_folder_is_written_and_for_folders_in_it_and_around_it() {
        let (data, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let was = tmp.path().join("Work").join("Agent");
        let beside = tmp.path().join("Work").join("Other");
        std::fs::create_dir_all(was.join("src")).unwrap();
        std::fs::create_dir_all(&beside).unwrap();
        let held = |folder: &Path| holds_in(&folder.to_string_lossy());
        with_data_home(data.path(), || {
            assert!(!held(&was) && !held(&was.join("src")) && !held(&tmp.path().join("Work")));
            mark(&was.to_string_lossy());
            assert!(held(&was));
            assert!(held(&PathBuf::from(format!("{}{}", was.display(), std::path::MAIN_SEPARATOR))), "a trailing separator");
            assert!(held(&was.join("src").join("..")), "a way round");
            assert!(held(&was.join("src")), "a folder in it");
            assert!(held(&was.join("not").join("there").join("yet")), "one in it that does not exist yet");
            assert!(held(&tmp.path().join("Work")), "a folder that holds it");
            assert!(!held(&beside), "a folder beside it");
            #[cfg(windows)]
            assert!(held(Path::new(&was.to_string_lossy().to_uppercase())), "another letter case");
            #[cfg(unix)]
            {
                let link = tmp.path().join("link");
                link_dir(&link, &was);
                assert!(held(&link) && held(&link.join("src")), "a link to it");
            }
        });
    }

    /// The mark made through a link is the folder's own.
    #[cfg(unix)]
    #[test]
    fn a_folder_marked_through_a_link_is_marked_itself() {
        let (data, tmp) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let real = tmp.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link");
        link_dir(&link, &real);
        with_data_home(data.path(), || {
            mark(&link.to_string_lossy());
            assert!(holds_in(&real.to_string_lossy()) && holds_in(&link.to_string_lossy()));
        });
    }

    /// The `agentmux` command of a host agent's config there is a path outside the folder, never a name,
    /// with a file of that name in the folder and beneath it.
    #[test]
    fn the_agentmux_command_is_named_by_a_path_outside_the_folder_or_not_at_all() {
        let (folder, tools, bound) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let name = if cfg!(windows) { "agentmux-mcp.exe" } else { "agentmux-mcp" };
        let (planted, shipped, writable) = (folder.path().join(name), tools.path().join(name), bound.path().join(name));
        for file in [&planted, &shipped, &writable] {
            std::fs::write(file, b"x").unwrap();
        }
        add_for_test(bound.path(), "agentmux-rule-e", "/tools");
        let config = r#"{"mcpServers":{"agentmux":{"type":"stdio","command":"agentmux-mcp","args":[],"env":{"AGENTMUX_AGENT_ID":"aria"}},"git":{"command":"git-mcp"}}}"#;
        let the_folder = spellings(&folder.path().to_string_lossy());

        assert!(outside(&shipped, &the_folder), "the one that ships with the install");
        assert!(!outside(&planted, &the_folder), "a file of that name in the folder");
        assert!(!outside(&folder.path().join("bin").join(name), &the_folder), "or beneath it");
        assert!(!outside(&writable, &the_folder), "one a container agent can write");
        assert!(!outside(Path::new(name), &the_folder), "a bare name");

        // The person's own server, set with a bare name: `git-mcp` is on the PATH here, `gone-mcp` is not.
        let git = tools.path().join(if cfg!(windows) { "git-mcp.exe" } else { "git-mcp" });
        let found = |command: &str| (command == "git-mcp").then(|| git.clone());
        let config = &config.replace(r#""git":{"command":"git-mcp"}"#, r#""git":{"command":"git-mcp","args":["--x"]},"gone":{"command":"gone-mcp"},"web":{"type":"http","url":"https://example.invalid/mcp"}"#);

        let (text, taken_out) = by_path(config, Some(&shipped), &found);
        let written: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(written["mcpServers"]["agentmux"]["command"], json!(shipped.to_string_lossy()));
        assert_eq!(written["mcpServers"]["agentmux"]["env"]["AGENTMUX_AGENT_ID"], "aria", "the rest of the entry stays");
        assert_eq!(written["mcpServers"]["git"]["command"], json!(git.to_string_lossy()), "a bare name becomes the path it has on the PATH");
        assert_eq!(written["mcpServers"]["git"]["args"], json!(["--x"]));
        assert!(written["mcpServers"].get("gone").is_none(), "a name with no path is not kept as a name");
        assert_eq!(taken_out, vec!["gone".to_string()]);
        assert_eq!(written["mcpServers"]["web"]["url"], "https://example.invalid/mcp", "a server with no command stays");
        assert!(names_the_command_by_path(&text));

        let (text, taken_out) = by_path(config, None, &found);
        let without: Value = serde_json::from_str(&text).unwrap();
        assert!(without["mcpServers"].get("agentmux").is_none(), "with none that ships outside the folder: no entry, not a name");
        assert_eq!(taken_out, vec!["agentmux".to_string(), "gone".to_string()]);
        assert!(names_the_command_by_path(&text));

        // A command the person wrote as a path is theirs and stays; so does a config with nothing to change.
        let by_hand = r#"{"mcpServers":{"own":{"command":"/opt/own/server"}}}"#;
        let by_hand = &if cfg!(windows) { by_hand.replace("/opt/own/server", "C:\\\\own\\\\server.exe") } else { by_hand.to_string() };
        assert_eq!(by_path(by_hand, Some(&shipped), &found), (by_hand.clone(), Vec::new()));

        assert!(!names_the_command_by_path(config), "a bare name is not handed to a turn there");
        assert!(!names_the_command_by_path(r#"{"mcpServers":{"agentmux":{"command":"/abs/agentmux-mcp"},"git":{"command":"git-mcp"}}}"#), "nor is any other server's");
        assert!(!names_the_command_by_path("not json"));
        assert_eq!(by_path("not json", Some(&shipped), &found).0, "not json");
        assert_eq!(on_path("bin/tool"), None, "a relative path is not looked up");
        assert_eq!(on_path("no-such-program-agentmux-test"), None);

        // As a writer calls it: whatever ships on this computer, the result never names the command barely,
        // and never by a path in the folder.
        let data = tempfile::tempdir().unwrap();
        with_data_home(data.path(), || {
            let dir = folder.path().to_string_lossy().into_owned();
            mark(&dir);
            let text = with_the_command_by_path(config, &dir);
            assert!(names_the_command_by_path(&text), "{text}");
            let doc: Value = serde_json::from_str(&text).unwrap();
            if let Some(command) = doc.pointer("/mcpServers/agentmux/command").and_then(Value::as_str) {
                assert!(outside(Path::new(command), &the_folder), "{command}");
            }
        });
    }

    /// A container agent in its own folder, which is marked from its first turn: every place that finds a
    /// host agent's config outside the folder leaves the container agent's alone or never runs for it.
    #[test]
    fn a_container_agent_in_its_own_marked_folder_keeps_the_file_in_its_folder() {
        let (data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir = folder.path().to_string_lossy().into_owned();
        add_for_test(folder.path(), "agentmux-rule-f", "/workspace");
        let in_folder = folder.path().join(".mcp.json");
        let own = r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp","env":{"AGENTMUX_AGENT_ID":"loap-loap-2"}}}}"#;
        std::fs::write(&in_folder, own).unwrap();
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        with_data_home(data.path(), || {
            mark(&dir);
            let container = pane("container", "claude", folder.path());
            assert_eq!(folder_of(&container), None, "the rule is a host agent's");
            assert_eq!(refusal(&container), None);
            assert_eq!(turn_refusal(&container), None);

            // The spawn environment names no host config for it.
            let mut env = std::collections::HashMap::from([("AGENTMUX_MCP_CONFIG".to_string(), "stale".to_string())]);
            crate::backend::spawn_env::block_identity::carry_agent_workdir_env(&mut env, &container);
            assert!(!env.contains_key("AGENTMUX_MCP_CONFIG"), "{env:?}");
            // The spawn's realign leaves its file as it is and writes none outside.
            crate::backend::mcp_json_identity::realign_workspace_mcp_json(&store, &dir, "container", "loap-2");
            assert_eq!(std::fs::read_to_string(&in_folder).unwrap(), own);
            // Its turn gets no flag that points away from its folder.
            let flags = crate::backend::agent_runtime::with_runtime_flags(&container, vec!["-p".to_string()]);
            assert!(!flags.iter().any(|flag| flag.contains("mcp-config")), "{flags:?}");
            // Its own config path is the file in its folder, and nothing was written for a host agent.
            assert_eq!(crate::backend::mcp_config_file::relocated_path(&dir).is_none(), agentmux_common::mcp_config_path::is_agent_owned_workdir(folder.path()));
            assert!(!crate::backend::mcp_config_file::host_config_path(&dir).exists(), "no host config was made by any of this");
        });
    }
}
