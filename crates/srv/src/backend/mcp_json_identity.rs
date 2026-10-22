// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent id in a workspace `.mcp.json`
//! (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` P0.3).
//!
//! `mcpServers.agentmux.env.AGENTMUX_AGENT_ID` is the name the agent's
//! `agentmux-mcp` process routes and signs as, beside the signing keys filed
//! under that name. The file is written at launch and then reused by every
//! respawn, so after a rename it names the old id until something rewrites
//! it. Both writers of that entry correct it from the definition: the
//! launch's `writeagentconfig` before it injects the keys, and each host
//! spawn ([`realign_workspace_mcp_json`]).

use serde_json::Value;

const AGENT_ID: &str = "/mcpServers/agentmux/env/AGENTMUX_AGENT_ID";

/// The keys filed under the name `.mcp.json` carried. They sign as that name
/// only, so they go with it; `inject_jekt_signing_keys_into_mcp_json` adds
/// the new name's.
const NAME_KEYED: [&str; 4] = ["AGENTMUX_JEKT_KEY", "AGENTMUX_LAN_KEY", "AGENTMUX_WAN_KEY", "AGENTMUX_HOST_LABEL"];

/// `content` with its agentmux server's `AGENTMUX_AGENT_ID` set to `slug`,
/// and the id it replaced. `None` when it already is `slug`, when there is no
/// such entry (a `.mcp.json` of the user's own, or none of AgentMux's), or
/// when `content` isn't JSON.
pub(crate) fn set_agent_id(content: &str, slug: &str) -> Option<(String, String)> {
    let mut json: Value = serde_json::from_str(content).ok()?;
    let current = json.pointer(AGENT_ID)?.as_str()?.to_string();
    if current == slug {
        return None;
    }
    let env = json.pointer_mut("/mcpServers/agentmux/env")?.as_object_mut()?;
    env.insert("AGENTMUX_AGENT_ID".to_string(), Value::String(slug.to_string()));
    for key in NAME_KEYED {
        env.remove(key);
    }
    Some((serde_json::to_string_pretty(&json).ok()?, current))
}

/// The id a launch's generated `.mcp.json` signs as (`writeagentconfig`): the
/// slug of definition `agent_id` when it has one, `content` rewritten to it
/// if the launch computed another; else the id `content` carries. `None`
/// when `content` has no agentmux entry.
pub(crate) fn launch_agent_id(
    content: &mut String,
    mstore: &crate::backend::storage::store::Store,
    agent_id: Option<&str>,
) -> Option<String> {
    let slug = agent_id
        .filter(|id| !id.is_empty())
        .and_then(|id| mstore.agent_def_get(id).ok().flatten())
        .map(|def| def.slug)
        .filter(|s| !s.trim().is_empty());
    if let Some((patched, previous)) = slug.as_deref().and_then(|s| set_agent_id(content, s)) {
        tracing::info!(previous = %previous, agent_id = ?slug, "writeagentconfig: the launch's .mcp.json named another agent id; using the definition's");
        *content = patched;
    }
    let json: Value = serde_json::from_str(content).ok()?;
    json.pointer(AGENT_ID)?.as_str().map(str::to_string)
}

/// For a host agent's spawn: when its workspace `.mcp.json` names another
/// id than the definition's `slug`, or names it with no signing keys, rewrite
/// it with `slug` and that slug's keys, under the same directory lock the
/// launch writes it with.
/// Best-effort: a failure is logged, never a spawn failure, and a container
/// agent (whose workspace path means nothing on the host) or an empty
/// `cmd:cwd` is left alone.
pub(crate) fn realign_workspace_mcp_json(
    mstore: &crate::backend::storage::store::Store,
    work_dir: &str,
    agent_mode: &str,
    slug: &str,
) {
    if crate::backend::operator_config_seed::agent_kind(agent_mode) != "host" || work_dir.trim().is_empty() {
        return;
    }
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(work_dir);
    let Ok(wd) = crate::backend::workdir_fs::Workdir::open(std::path::Path::new(&expanded)) else {
        return;
    };
    let Ok(path) = wd.resolve(".mcp.json") else {
        return;
    };
    // Held from the read to the write, so a launch writing the file at the
    // same time is not overwritten with this spawn's older copy. Without it,
    // no rewrite: the next spawn tries again.
    let _lock = match wd
        .open_lock_file(crate::backend::agent_config::MCP_JSON_WRITE_LOCK)
        .and_then(|f| f.lock().map(|()| f))
    {
        Ok(lock) => lock,
        Err(e) => {
            tracing::warn!(work_dir, error = %e, "spawn: .mcp.json lock unavailable; its agent id is not realigned this spawn");
            return;
        }
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    let (patched, previous) = match set_agent_id(&content, slug) {
        Some((patched, previous)) => (patched, Some(previous)),
        None if names_missing_a_key(&content, slug) => (content.clone(), None),
        None => return,
    };
    // Never written without the new name's keys: a later spawn would find the
    // id already matching and leave the agent unsigned. Left as it was, the
    // next spawn tries again.
    let Some(body) = crate::backend::agent_config::inject_jekt_signing_keys_into_mcp_json(&patched, mstore, slug) else {
        tracing::warn!(work_dir, agent_id = slug, "spawn: signing keys unavailable; workspace .mcp.json left as it was");
        return;
    };
    if serde_json::from_str::<Value>(&body).ok() == serde_json::from_str::<Value>(&content).ok() {
        return;
    }
    match wd.write(".mcp.json", body.as_bytes(), true) {
        Ok(()) => tracing::info!(
            work_dir,
            previous = ?previous,
            agent_id = slug,
            "spawn: rewrote the workspace .mcp.json's agent id and signing keys from the definition"
        ),
        Err(e) => tracing::warn!(work_dir, error = %e, "spawn: could not rewrite the workspace .mcp.json's agent id"),
    }
}

/// `content`'s agentmux entry names `slug` but lacks a signing key: either
/// one, so a realignment that got only one of them is retried.
fn names_missing_a_key(content: &str, slug: &str) -> bool {
    let Ok(json) = serde_json::from_str::<Value>(content) else { return false };
    let Some(env) = json.pointer("/mcpServers/agentmux/env") else { return false };
    env.get("AGENTMUX_AGENT_ID").and_then(Value::as_str) == Some(slug)
        && (env.get("AGENTMUX_JEKT_KEY").is_none() || env.get("AGENTMUX_LAN_KEY").is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp(agent_id: &str) -> String {
        serde_json::json!({
            "mcpServers": {
                "agentmux": { "command": "agentmux-mcp", "env": {
                    "AGENTMUX_AGENT_ID": agent_id, "AGENTMUX_AGENT_BUS_ID": "bus-1",
                    "AGENTMUX_JEKT_KEY": "old-key", "AGENTMUX_LAN_KEY": "old-lan"
                }},
                "theirs": { "command": "x" }
            }
        })
        .to_string()
    }

    #[test]
    fn the_id_is_replaced_with_its_keys_and_nothing_else() {
        let (out, previous) = set_agent_id(&mcp("loap-loap-2"), "loap-2").unwrap();
        assert_eq!(previous, "loap-loap-2");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v.pointer(AGENT_ID).and_then(Value::as_str), Some("loap-2"));
        let env = v.pointer("/mcpServers/agentmux/env").unwrap();
        assert!(env.get("AGENTMUX_JEKT_KEY").is_none() && env.get("AGENTMUX_LAN_KEY").is_none());
        assert_eq!(env["AGENTMUX_AGENT_BUS_ID"], "bus-1");
        assert_eq!(v.pointer("/mcpServers/theirs/command").and_then(Value::as_str), Some("x"));
    }

    #[test]
    fn a_matching_or_foreign_file_is_left_alone() {
        assert!(set_agent_id(&mcp("loap-2"), "loap-2").is_none());
        assert!(set_agent_id(r#"{"mcpServers":{"theirs":{}}}"#, "loap-2").is_none());
        assert!(set_agent_id("not json", "loap-2").is_none());
    }

    /// P0.3 on spawn: a stale workspace file is rewritten for a host agent,
    /// and a container agent's is not touched.
    #[test]
    fn a_host_spawn_rewrites_a_stale_workspace_file() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".mcp.json");
        std::fs::write(&file, mcp("loap-loap-2")).unwrap();
        let work_dir = dir.path().to_string_lossy().into_owned();

        realign_workspace_mcp_json(&store, &work_dir, "container", "loap-2");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), mcp("loap-loap-2"));

        realign_workspace_mcp_json(&store, &work_dir, "host", "loap-2");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v.pointer(AGENT_ID).and_then(Value::as_str), Some("loap-2"));
        assert_ne!(
            v.pointer("/mcpServers/agentmux/env/AGENTMUX_JEKT_KEY").and_then(Value::as_str),
            Some("old-key"),
            "the old name's key does not sign for the new one"
        );
    }

    fn file_id_and_key(file: &std::path::Path) -> (String, Option<String>) {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        let env = v.pointer("/mcpServers/agentmux/env").unwrap();
        (env["AGENTMUX_AGENT_ID"].as_str().unwrap().to_string(), env.get("AGENTMUX_JEKT_KEY").and_then(Value::as_str).map(str::to_string))
    }

    /// When the new name's keys can't be had, the file is left as it was (old
    /// id, old keys), and the next spawn finishes the job. A file already
    /// naming the slug but carrying no keys gets them.
    #[test]
    fn a_rewrite_without_keys_is_not_persisted_and_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".mcp.json");
        std::fs::write(&file, mcp("loap-loap-2")).unwrap();
        let work_dir = dir.path().to_string_lossy().into_owned();

        let broken = crate::backend::storage::store::Store::open_in_memory().unwrap();
        broken.test_break_name_key_tables().unwrap();
        realign_workspace_mcp_json(&broken, &work_dir, "host", "loap-2");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), mcp("loap-loap-2"), "not half-realigned");

        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        realign_workspace_mcp_json(&store, &work_dir, "host", "loap-2");
        let (id, key) = file_id_and_key(&file);
        assert_eq!(id, "loap-2");
        assert!(key.is_some_and(|k| k != "old-key"));

        let (stripped, _) = set_agent_id(&mcp("old"), "loap-3").unwrap();
        std::fs::write(&file, stripped).unwrap();
        realign_workspace_mcp_json(&store, &work_dir, "host", "loap-3");
        assert!(file_id_and_key(&file).1.is_some(), "a matching id with no keys gets them");

        let mut one_key: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        one_key["mcpServers"]["agentmux"]["env"].as_object_mut().unwrap().remove("AGENTMUX_LAN_KEY");
        std::fs::write(&file, one_key.to_string()).unwrap();
        realign_workspace_mcp_json(&store, &work_dir, "host", "loap-3");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(v.pointer("/mcpServers/agentmux/env/AGENTMUX_LAN_KEY").is_some(), "a matching id with one key gets the other");
    }

    /// No lock, no rewrite: a `.claude` that is a file makes the lock
    /// unopenable, and the stale file stays for the next spawn.
    #[test]
    fn no_rewrite_without_the_lock() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".mcp.json");
        std::fs::write(&file, mcp("loap-loap-2")).unwrap();
        std::fs::write(dir.path().join(".claude"), "not a folder").unwrap();
        realign_workspace_mcp_json(&store, &dir.path().to_string_lossy(), "host", "loap-2");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), mcp("loap-loap-2"));
    }
}
