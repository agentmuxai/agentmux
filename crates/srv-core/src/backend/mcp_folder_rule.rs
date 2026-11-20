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
//! - Claude Code is told to use that config alone (`agent_runtime`);
//! - any other harness is not started there ([`refusal`]): what it reads from its folder is its own, and
//!   AgentMux has no switch for it;
//! - the pane says which servers of the folder's file were left out ([`left_out`], `cli_notice`).
//!
//! "Can write" is asked of [`container_folders`](crate::backend::fs_ops::container_folders), as the Editor's
//! language servers ask it; where that can't be told, the rule holds. "Has run in it" is a mark kept under the
//! data root ([`mark`]), so the rule still holds when the agent is switched to the host, or another host agent
//! is given the folder. A folder no container agent has to do with is not touched by any of this.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::backend::fs_ops::container_folders::{holds_a_container_agents_folder, place, Place};
use crate::backend::obj::{meta_get_string, MetaMapType};

/// The mark's file, in the folder under the data root that is keyed by the working folder.
const MARK: &str = "container-agent-folder";
/// The most servers a pane row names, and the most characters of one name: they are read from the folder's
/// own file.
const MOST_NAMES: usize = 20;
const LONGEST_NAME: usize = 64;

fn expanded(work_dir: &str) -> String {
    crate::backend::blockcontroller::core::expand_home_dir(work_dir)
}

fn mark_path(expanded: &str) -> Option<PathBuf> {
    agentmux_common::mcp_config_path::outside_dir_for(Path::new(expanded)).map(|dir| dir.join(MARK))
}

/// Record that a container agent runs with `work_dir` as its folder. The mark stays: from now on a host agent
/// in that folder gets only the servers AgentMux writes. Best-effort; a failure is logged.
pub fn mark(work_dir: &str) {
    if work_dir.trim().is_empty() {
        return;
    }
    let Some(path) = mark_path(&expanded(work_dir)) else { return };
    if path.is_file() {
        return;
    }
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, b""));
    if let Err(e) = written {
        tracing::warn!(work_dir, error = %e, "the folder could not be marked as a container agent's");
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

fn marked(expanded: &str) -> bool {
    mark_path(expanded).is_some_and(|path| path.is_file())
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

/// Why the agent on a pane with `meta` is not started on this computer, when it is not: its harness is not
/// Claude Code and the rule holds for its folder. `None` for a container agent, for Claude Code, for a pane
/// that is no agent's, and for every other folder.
pub fn refusal(meta: &MetaMapType) -> Option<String> {
    let provider = crate::backend::providers::resolve_provider_alias(&meta_get_string(meta, "agentProvider", ""));
    if provider.is_empty() || provider == "claude" {
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

/// What a pane is told when [`refusal`] stops its agent.
pub const REFUSAL: &str = "A container agent can write in this agent's folder, or has run in it, so this agent is not started on this computer there: only Claude Code can be kept to the MCP servers set in AgentMux. Run this agent in a container, or give it a folder no container agent can write.";

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
            assert!(!was.path().join(MARK).exists(), "the mark is kept outside the folder");
        });
    }

    #[test]
    fn only_a_host_agent_that_is_not_claude_code_is_refused_and_only_there() {
        let (data, bound, own) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        add_for_test(bound.path(), "agentmux-rule-c", "/workspace");
        with_data_home(data.path(), || {
            for provider in ["codex", "gemini", "kimi", "muxcode", "opencode"] {
                assert_eq!(refusal(&pane("host", provider, bound.path())).as_deref(), Some(REFUSAL), "{provider}");
                assert_eq!(refusal(&pane("host", provider, own.path())), None, "{provider} in a folder of its own");
                assert_eq!(refusal(&pane("container", provider, bound.path())), None, "{provider} in the container");
            }
            assert_eq!(refusal(&pane("host", "claude", bound.path())), None, "Claude Code is kept to AgentMux's config instead");
            assert_eq!(refusal(&pane("host", "", bound.path())), None, "a pane that is no agent's");
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
}
