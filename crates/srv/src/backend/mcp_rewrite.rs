// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Rungs 3 and 4 of the MCP self-heal ladder (`mcp_health`,
//! `docs/specs/SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md` §5): put the `agentmux` entry of the agent's
//! workspace `.mcp.json` right, then reconnect. Rung 3 rewrites it the way a launch writes it (the
//! bare `agentmux-mcp` command, this agent's id and signing keys), which mends a missing or stale
//! entry or one the user edited. Rung 4 makes the command the absolute path of the `agentmux-mcp`
//! that ships with this install, which mends a `PATH` that does not find it.
//!
//! Only `mcpServers.agentmux` is touched. A launch's merge (`write_mcp_json_respecting_user_servers`)
//! would also drop the servers the agent had bound and unbound since, which a repair in the middle of
//! a session must not do. The write is under the same directory lock the launch writes with.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::backend::mcp_health::Shape;

const COMMAND: &str = "agentmux-mcp";

/// Block -> the directory its CLI was last spawned in, where its `.mcp.json` is.
fn workdirs() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static DIRS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();
    DIRS.get_or_init(Default::default)
}

/// A pane's CLI is spawned in `work_dir` (called at each spawn).
pub(crate) fn remember_workdir(block_id: &str, work_dir: &str) {
    workdirs().lock().unwrap_or_else(|e| e.into_inner()).insert(block_id.to_string(), work_dir.to_string());
}

/// The directory `block_id`'s CLI was last spawned in.
pub(crate) fn workdir_of(block_id: &str) -> Option<String> {
    workdirs().lock().unwrap_or_else(|e| e.into_inner()).get(block_id).cloned()
}

/// The `agentmux-mcp` that ships with this install: the bundled copy, else the user tools dir's
/// (where `task dev` puts it), the order `tool_store::bashwrap_path` uses.
fn shipped_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "agentmux-mcp.exe" } else { COMMAND };
    [crate::backend::tool_store::bundled_tools_dir(), crate::backend::tool_store::user_tools_dir()]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// `content` (a `.mcp.json`; empty when there is none) with its `agentmux` entry made `shape` for
/// `slug`: type `stdio`, the command, no arguments, and the entry's `env` with `AGENTMUX_AGENT_ID` set.
/// The rest of `env` (bus id, channel, keys) and every other server stay as they were. `Err` when the
/// file is not a JSON object, which is not ours to replace, or when the shape needs an `exe` and there
/// is none.
fn repaired(content: &str, slug: &str, shape: Shape, exe: Option<&Path>) -> Result<String, String> {
    // An entry that names another id is a rename's leftover: its keys go with the old name.
    let content = crate::backend::mcp_json_identity::set_agent_id(content, slug).map_or_else(|| content.to_string(), |(c, _)| c);
    let mut doc: Value = if content.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&content).map_err(|e| format!(".mcp.json is not JSON: {e}"))?
    };
    let root = doc.as_object_mut().ok_or("the .mcp.json is not a JSON object")?;
    let command = match shape {
        Shape::Canonical => COMMAND.to_string(),
        Shape::AbsolutePath => exe.ok_or("no agentmux-mcp ships with this install")?.to_string_lossy().into_owned(),
    };
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or("mcpServers is not an object")?;
    let mut env = servers
        .get("agentmux")
        .and_then(|e| e.get("env"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    env.insert("AGENTMUX_AGENT_ID".to_string(), json!(slug));
    servers.insert("agentmux".to_string(), json!({ "type": "stdio", "command": command, "args": [], "env": env }));
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}

/// Rewrite the `agentmux` entry of the `.mcp.json` in `work_dir` for `slug`, with its signing keys,
/// under the launch's directory lock. `Ok(true)` when the file changed, `Ok(false)` when it already was
/// so (a reconnect cannot come out differently, so the ladder moves on).
fn rewrite_file(
    mstore: &crate::backend::storage::store::Store,
    work_dir: &str,
    slug: &str,
    shape: Shape,
    exe: Option<&Path>,
) -> Result<bool, String> {
    if work_dir.trim().is_empty() {
        return Err("the pane has no working directory".to_string());
    }
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(work_dir);
    let wd = crate::backend::workdir_fs::Workdir::open(Path::new(&expanded)).map_err(|e| format!("cannot open {expanded}: {e}"))?;
    let path = wd.resolve(".mcp.json").map_err(|e| format!("cannot use .mcp.json: {e}"))?;
    let _lock = wd
        .open_lock_file(crate::backend::agent_config::MCP_JSON_WRITE_LOCK)
        .and_then(|f| f.lock().map(|()| f))
        .map_err(|e| format!(".mcp.json lock unavailable: {e}"))?;
    // Read under the lock: a launch may have written it meanwhile.
    let content = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read .mcp.json: {e}")),
    };
    let body = repaired(&content, slug, shape, exe)?;
    // Keys kept as the file has them if they cannot be ensured now.
    let body = crate::backend::agent_config::inject_jekt_signing_keys_into_mcp_json(&body, mstore, slug).unwrap_or(body);
    if serde_json::from_str::<Value>(&body).ok() == serde_json::from_str::<Value>(&content).ok() {
        return Ok(false);
    }
    wd.write(".mcp.json", body.as_bytes(), true).map_err(|e| format!("cannot write .mcp.json: {e}"))?;
    Ok(true)
}

/// Rung 3 or 4 for the agent on `block_id`, whose pane runs in `work_dir`. A container agent starts its
/// server inside the image, from a copy of the file the host does not own.
pub fn run(mstore: &crate::backend::storage::store::Store, block_id: &str, work_dir: &str, shape: Shape) -> Result<bool, String> {
    let instance = mstore
        .instance_get_active_for_block(block_id)
        .map_err(|e| e.to_string())?
        .ok_or("no agent runs in this pane")?;
    let def = mstore
        .agent_def_get(&instance.definition_id)
        .map_err(|e| e.to_string())?
        .ok_or("the agent's definition is gone")?;
    if def.agent_type == "container" {
        return Err("a container agent starts its server inside the image".to_string());
    }
    if def.slug.trim().is_empty() {
        return Err("the agent has no id".to_string());
    }
    rewrite_file(mstore, work_dir, &def.slug, shape, shipped_exe().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(content: &str) -> Value {
        serde_json::from_str::<Value>(content).unwrap()["mcpServers"]["agentmux"].clone()
    }

    #[test]
    fn a_missing_file_or_entry_gets_the_canonical_entry_and_the_users_servers_stay() {
        let fresh = entry(&repaired("", "manoz", Shape::Canonical, None).unwrap());
        assert_eq!(fresh, json!({ "type": "stdio", "command": "agentmux-mcp", "args": [], "env": { "AGENTMUX_AGENT_ID": "manoz" } }));

        let theirs = r#"{ "mcpServers": { "github": { "command": "gh-mcp" } }, "other": 1 }"#;
        let out: Value = serde_json::from_str(&repaired(theirs, "manoz", Shape::Canonical, None).unwrap()).unwrap();
        assert_eq!(out["mcpServers"]["github"], json!({ "command": "gh-mcp" }));
        assert_eq!(out["other"], 1);
        assert_eq!(out["mcpServers"]["agentmux"]["command"], "agentmux-mcp");
    }

    #[test]
    fn an_edited_entry_is_restored_and_its_env_is_kept() {
        let edited = json!({ "mcpServers": { "agentmux": {
            "type": "sse", "command": "/old/agentmux-mcp", "args": ["--debug"], "extra": true,
            "env": { "AGENTMUX_AGENT_ID": "manoz", "AGENTMUX_AGENT_BUS_ID": "bus-1", "AGENTMUX_JEKT_KEY": "k" }
        }}})
        .to_string();
        let out = entry(&repaired(&edited, "manoz", Shape::Canonical, None).unwrap());
        assert_eq!(out["type"], "stdio");
        assert_eq!(out["command"], "agentmux-mcp");
        assert_eq!(out["args"], json!([]));
        assert!(out.get("extra").is_none());
        assert_eq!(out["env"]["AGENTMUX_AGENT_BUS_ID"], "bus-1");
        assert_eq!(out["env"]["AGENTMUX_JEKT_KEY"], "k", "the id is unchanged, so its key still signs");
    }

    #[test]
    fn a_renamed_agents_old_keys_do_not_sign_for_the_new_name() {
        let old = json!({ "mcpServers": { "agentmux": { "command": "agentmux-mcp", "env": {
            "AGENTMUX_AGENT_ID": "old-name", "AGENTMUX_JEKT_KEY": "k", "AGENTMUX_LAN_KEY": "l"
        }}}})
        .to_string();
        let out = entry(&repaired(&old, "new-name", Shape::Canonical, None).unwrap());
        assert_eq!(out["env"]["AGENTMUX_AGENT_ID"], "new-name");
        assert!(out["env"].get("AGENTMUX_JEKT_KEY").is_none() && out["env"].get("AGENTMUX_LAN_KEY").is_none());
    }

    #[test]
    fn the_absolute_shape_names_the_shipped_binary_and_needs_one() {
        let exe = Path::new("/app/tools/bin/agentmux-mcp");
        let out = entry(&repaired("", "manoz", Shape::AbsolutePath, Some(exe)).unwrap());
        assert_eq!(out["command"], exe.to_string_lossy().as_ref());
        assert!(repaired("", "manoz", Shape::AbsolutePath, None).unwrap_err().contains("no agentmux-mcp ships"));
    }

    #[test]
    fn a_file_that_is_not_ours_to_replace_is_an_error() {
        assert!(repaired("not json", "manoz", Shape::Canonical, None).unwrap_err().contains("not JSON"));
        assert!(repaired("[1]", "manoz", Shape::Canonical, None).unwrap_err().contains("not a JSON object"));
        assert!(repaired(r#"{"mcpServers": 3}"#, "manoz", Shape::Canonical, None).unwrap_err().contains("not an object"));
    }

    #[test]
    fn a_file_already_canonical_is_unchanged_and_one_that_is_not_is_rewritten_once() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".mcp.json");
        let work_dir = dir.path().to_string_lossy().into_owned();
        let broken = json!({ "mcpServers": {
            "agentmux": { "command": "/gone/agentmux-mcp", "args": [], "env": { "AGENTMUX_AGENT_ID": "manoz" } },
            "github": { "command": "gh-mcp" }
        }})
        .to_string();
        std::fs::write(&file, &broken).unwrap();

        assert_eq!(rewrite_file(&store, &work_dir, "manoz", Shape::Canonical, None), Ok(true));
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["agentmux"]["command"], "agentmux-mcp");
        assert_eq!(after["mcpServers"]["github"]["command"], "gh-mcp");
        assert!(after["mcpServers"]["agentmux"]["env"]["AGENTMUX_JEKT_KEY"].is_string(), "its keys are put back");

        assert_eq!(rewrite_file(&store, &work_dir, "manoz", Shape::Canonical, None), Ok(false), "nothing left to change");
        let exe = dir.path().join("agentmux-mcp");
        assert_eq!(rewrite_file(&store, &work_dir, "manoz", Shape::AbsolutePath, Some(&exe)), Ok(true));
        assert_eq!(rewrite_file(&store, &work_dir, "manoz", Shape::AbsolutePath, Some(&exe)), Ok(false));
    }

    #[test]
    fn a_pane_without_a_directory_or_agent_is_an_error_not_a_panic() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        assert!(rewrite_file(&store, "", "manoz", Shape::Canonical, None).unwrap_err().contains("no working directory"));
        assert!(rewrite_file(&store, "/nonexistent/dir/for/rewrite", "manoz", Shape::Canonical, None).is_err());
        assert!(run(&store, "no-such-block", "/tmp", Shape::Canonical).unwrap_err().contains("no agent runs"));
    }
}
