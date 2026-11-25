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

/// The signing values no start file holds, for any runtime: the file holds references, not values.
/// A container sees its workspace through a mount and srv signs its jekts itself, by its token; a host
/// agent's MCP server asks srv for its keys with its own token.
pub(crate) const SIGNING_SECRETS: [&str; 3] = ["AGENTMUX_JEKT_KEY", "AGENTMUX_LAN_KEY", "AGENTMUX_WAN_KEY"];

/// `content` without the signing keys of its agentmux server. `None` when it carries none, or isn't JSON.
pub(crate) fn strip_signing_keys(content: &str) -> Option<String> {
    let mut json: Value = serde_json::from_str(content).ok()?;
    let env = json.pointer_mut("/mcpServers/agentmux/env")?.as_object_mut()?;
    let mut removed = false;
    for key in SIGNING_SECRETS {
        removed |= env.remove(key).is_some();
    }
    removed.then(|| serde_json::to_string_pretty(&json).ok()).flatten()
}

/// A container agent's workspace `.mcp.json`, with the signing keys an earlier launch wrote taken out.
/// Run at each container start, under the lock the launch writes the file with. Only those keys are
/// removed, and only when the file has them, so a file with none is left byte-for-byte as it was.
/// This also covers a user's project: keys an older launch wrote there must not ride the workspace
/// mount into the container (a launch no longer writes any, #3680). Best-effort: a failure is
/// logged and the start goes on.
pub fn strip_workspace_signing_keys(work_dir: &str) {
    if work_dir.trim().is_empty() {
        return;
    }
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(work_dir);
    let Ok(wd) = crate::backend::workdir_fs::Workdir::open(std::path::Path::new(&expanded)) else { return };
    let Ok(path) = wd.resolve(".mcp.json") else { return };
    let Ok(content) = std::fs::read_to_string(&path) else { return };
    if strip_signing_keys(&content).is_none() {
        return;
    }
    let _lock = match wd
        .open_lock_file(crate::backend::agent_config::MCP_JSON_WRITE_LOCK)
        .and_then(|f| f.lock().map(|()| f))
    {
        Ok(lock) => lock,
        Err(e) => {
            tracing::warn!(work_dir, error = %e, "container start: .mcp.json lock unavailable; signing keys not removed this start");
            return;
        }
    };
    // Read again under the lock: a launch may have rewritten it meanwhile.
    let Ok(content) = std::fs::read_to_string(&path) else { return };
    let Some(body) = strip_signing_keys(&content) else { return };
    match wd.write(".mcp.json", body.as_bytes(), true) {
        Ok(()) => tracing::info!(work_dir, "container start: removed signing keys from the workspace .mcp.json"),
        Err(e) => tracing::warn!(work_dir, error = %e, "container start: could not remove signing keys from the workspace .mcp.json"),
    }
}

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
/// id than the definition's `slug`, or names it with a signing value an older version wrote, rewrite
/// it with `slug`, holding no signing value, under the same directory lock the
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
    // A user's project keeps its own `.mcp.json` (#3680): the agent's file is AgentMux's
    // relocated one, and that is the one a spawn may realign. So is the file of a host agent in a
    // folder a container agent can write.
    if let Some(path) = crate::backend::mcp_config_file::host_relocated_path(&expanded) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        let (patched, previous) = match set_agent_id(&content, slug) {
            Some((patched, previous)) => (patched, Some(previous)),
            None if carries_a_signing_value(&content, slug) => (content.clone(), None),
            None => return,
        };
        let Some(body) = crate::backend::agent_config::inject_jekt_signing_keys_into_mcp_json(&patched, mstore, slug) else {
            tracing::warn!(work_dir, agent_id = slug, "spawn: the agent's keys could not be ensured; its MCP config keeps its id and loses any signing value");
            if let Some(stripped) = strip_signing_keys(&content) {
                let body = crate::backend::mcp_config_file::host_content(&expanded, stripped);
                let _ = crate::backend::mcp_config_file::write_relocated(&path, &body);
            }
            return;
        };
        let body = crate::backend::mcp_config_file::host_content(&expanded, body);
        if serde_json::from_str::<Value>(&body).ok() == serde_json::from_str::<Value>(&content).ok() {
            return;
        }
        match crate::backend::mcp_config_file::write_relocated(&path, &body) {
            Ok(()) => tracing::info!(
                work_dir,
                previous = ?previous,
                agent_id = slug,
                "spawn: rewrote the agent's MCP config from the definition"
            ),
            Err(e) => tracing::warn!(work_dir, error = %e, "spawn: could not rewrite the agent's MCP config"),
        }
        return;
    }
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
        None if carries_a_signing_value(&content, slug) => (content.clone(), None),
        None => return,
    };
    // Never written without the new name's keys: a later spawn would find the
    // id already matching and leave the agent unsigned. Left as it was, the
    // next spawn tries again.
    let Some(body) = crate::backend::agent_config::inject_jekt_signing_keys_into_mcp_json(&patched, mstore, slug) else {
        tracing::warn!(work_dir, agent_id = slug, "spawn: the agent's keys could not be ensured; workspace .mcp.json keeps its id and loses any signing value");
        if let Some(stripped) = strip_signing_keys(&content) {
            let _ = wd.write(".mcp.json", stripped.as_bytes(), true);
        }
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
            "spawn: rewrote the workspace .mcp.json from the definition"
        ),
        Err(e) => tracing::warn!(work_dir, error = %e, "spawn: could not rewrite the workspace .mcp.json's agent id"),
    }
}

/// `content`'s agentmux entry names `slug` but still holds a signing value an older version wrote, or
/// lacks the channel, so the file is rewritten at the next spawn.
fn carries_a_signing_value(content: &str, slug: &str) -> bool {
    let Ok(json) = serde_json::from_str::<Value>(content) else { return false };
    let Some(env) = json.pointer("/mcpServers/agentmux/env") else { return false };
    env.get("AGENTMUX_AGENT_ID").and_then(Value::as_str) == Some(slug)
        && (SIGNING_SECRETS.iter().any(|k| env.get(*k).is_some()) || env.get("AGENTMUX_CHANNEL").is_none())
}

/// `agent_open`'s generated `.mcp.json`: no signing value for any agent (the file holds references, not
/// values), the keys ensured in the store for a host agent, and any value an earlier launch wrote taken out.
pub fn keys_for_agent_open(
    content: &mut String,
    mstore: &crate::backend::storage::store::Store,
    agent_slug: &str,
    agent: &crate::backend::storage::agents::AgentDefinition,
) {
    if !agent.runtime().adapter().carries_signing_keys() {
        if let Some(stripped) = strip_signing_keys(content) {
            *content = stripped;
        }
        return;
    }
    crate::backend::agent_resolve::record_config_id_drift("m4d.config_id_drift.agent_open", agent_slug, Some(&agent.slug));
    match crate::backend::agent_config::inject_jekt_signing_keys_into_mcp_json(content, mstore, agent_slug) {
        Some(rewritten) => *content = rewritten,
        None => {
            tracing::warn!(agent_id = %agent.id, "agent_open: the agent's signing keys could not be ensured");
            if let Some(stripped) = strip_signing_keys(content) {
                *content = stripped; // a legacy blob's own agentmux entry may carry values
            }
        }
    }
}

/// A launch's generated `.mcp.json` (`writeagentconfig`): named for the definition's agent id, holding
/// no signing value, and, when `hands_out_keys`, with that agent's keys ensured in the store.
///
/// Keys are ensured only for the AgentMux window, proven by its host (`RpcContext::operator_only`): a
/// caller that could name any agent would otherwise have keys made for it. The agent's MCP fetches its
/// own keys from srv with its token (`/agentmux/agents/self/keys`).
pub fn finish_launch(
    files: &mut [crate::backend::rpc_types::AgentConfigFile],
    mstore: &crate::backend::storage::store::Store,
    agent_id: Option<&str>,
    hands_out_keys: bool,
) {
    let Some(pos) = files.iter().position(|f| f.path == ".mcp.json") else { return };
    // The definition's slug, over one the launch computed (P0.3).
    let Some(agent_slug) = launch_agent_id(&mut files[pos].content, mstore, agent_id) else { return };
    crate::backend::agent_resolve::record_config_id_drift(
        "m4d.config_id_drift.write_agent_config",
        &agent_slug,
        crate::backend::agent_resolve::single_row_slug_folded(mstore, &agent_slug).as_deref(),
    );
    // A container agent gets none: the file is mounted into its container.
    let in_container = agent_id
        .and_then(|id| mstore.agent_def_get(id).ok().flatten())
        .is_some_and(|def| !def.runtime().adapter().carries_signing_keys());
    if in_container {
        if let Some(stripped) = strip_signing_keys(&files[pos].content) {
            files[pos].content = stripped;
        }
        return;
    }
    if !hands_out_keys {
        // No value is written either way: an older value the launch's content carries is taken out.
        if let Some(stripped) = strip_signing_keys(&files[pos].content) {
            files[pos].content = stripped;
        }
        tracing::info!(agent_slug = %agent_slug, "WriteAgentConfig: keys not ensured: the caller isn't the AgentMux window");
        return;
    }
    match crate::backend::agent_config::inject_jekt_signing_keys_into_mcp_json(&files[pos].content, mstore, &agent_slug) {
        Some(rewritten) => files[pos].content = rewritten,
        None => {
            tracing::warn!(agent_slug = %agent_slug, "WriteAgentConfig: the agent's signing keys could not be ensured");
            if let Some(stripped) = strip_signing_keys(&files[pos].content) {
                files[pos].content = stripped; // a legacy blob's own agentmux entry may carry values
            }
        }
    }
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
        let data = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path().to_string_lossy().into_owned();
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            // A user project's config is AgentMux's relocated file (#3680).
            let file = crate::backend::mcp_config_file::config_path(&work_dir);
            crate::backend::mcp_config_file::write_relocated(&file, &mcp("loap-loap-2")).unwrap();

            realign_workspace_mcp_json(&store, &work_dir, "container", "loap-2");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), mcp("loap-loap-2"));

            realign_workspace_mcp_json(&store, &work_dir, "host", "loap-2");
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
            assert_eq!(v.pointer(AGENT_ID).and_then(Value::as_str), Some("loap-2"));
            assert!(
                holds_no_key(&std::fs::read_to_string(&file).unwrap()),
                "the old name's key is gone and the new name's is not written: the file holds references, not values"
            );
            assert!(!dir.path().join(".mcp.json").exists(), "the project's own file is never written");
        });
    }

    fn file_id_and_channel(file: &std::path::Path) -> (String, Option<String>) {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        let env = v.pointer("/mcpServers/agentmux/env").unwrap();
        (env["AGENTMUX_AGENT_ID"].as_str().unwrap().to_string(), env.get("AGENTMUX_CHANNEL").and_then(Value::as_str).map(str::to_string))
    }

    /// When the new name's keys can't be ensured, the file is left as it was (old
    /// id, old values), and the next spawn finishes the job. A file already
    /// naming the slug, with no channel or with a value an older version wrote, is rewritten.
    #[test]
    fn a_rewrite_without_keys_is_not_persisted_and_is_retried() {
        let data = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path().to_string_lossy().into_owned();
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            let file = crate::backend::mcp_config_file::config_path(&work_dir);
            crate::backend::mcp_config_file::write_relocated(&file, &mcp("loap-loap-2")).unwrap();

            let broken = crate::backend::storage::store::Store::open_in_memory().unwrap();
            broken.test_break_name_key_tables().unwrap();
            realign_workspace_mcp_json(&broken, &work_dir, "host", "loap-2");
            let text = std::fs::read_to_string(&file).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), serde_json::from_str::<Value>(&strip_signing_keys(&mcp("loap-loap-2")).unwrap()).unwrap(), "not half-realigned, but no value is left in it");
            assert!(holds_no_key(&text));
            crate::backend::mcp_config_file::write_relocated(&file, &mcp("loap-loap-2")).unwrap();

            let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
            realign_workspace_mcp_json(&store, &work_dir, "host", "loap-2");
            let (id, channel) = file_id_and_channel(&file);
            assert_eq!(id, "loap-2");
            assert!(channel.is_some() && holds_no_key(&std::fs::read_to_string(&file).unwrap()));
            assert!(store.agent_jekt_key_load("loap-2").unwrap().is_some(), "the keys are ensured in the store for the agent's fetch");

            let (stripped, _) = set_agent_id(&mcp("old"), "loap-3").unwrap();
            crate::backend::mcp_config_file::write_relocated(&file, &stripped).unwrap();
            realign_workspace_mcp_json(&store, &work_dir, "host", "loap-3");
            assert!(file_id_and_channel(&file).1.is_some(), "a matching id with no channel gets it");

            let mut one_value: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
            one_value["mcpServers"]["agentmux"]["env"]
                .as_object_mut()
                .unwrap()
                .insert("AGENTMUX_LAN_KEY".into(), Value::String("old-lan".into()));
            crate::backend::mcp_config_file::write_relocated(&file, &one_value.to_string()).unwrap();
            realign_workspace_mcp_json(&store, &work_dir, "host", "loap-3");
            assert!(holds_no_key(&std::fs::read_to_string(&file).unwrap()), "a matching id with a value an older version wrote loses it");
        });
    }

    /// A start file an older version wrote (the id already right, both values in it) is rewritten at the
    /// agent's next spawn, and after it no file under the agent's workspace or the data home holds either
    /// value, though the keys are in the store.
    #[test]
    fn an_old_start_file_is_rewritten_and_no_file_holds_either_value() {
        use base64::Engine as _;
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let data = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let work_dir = dir.path().to_string_lossy().into_owned();
        let jekt = store.agent_jekt_key_ensure("loap-2").unwrap();
        let jekt = base64::engine::general_purpose::STANDARD.encode(jekt);
        let lan = store.agent_lan_key_ensure("loap-2").unwrap().private_key;
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            let file = crate::backend::mcp_config_file::config_path(&work_dir);
            let old = serde_json::json!({
                "mcpServers": { "agentmux": { "command": "agentmux-mcp", "env": {
                    "AGENTMUX_AGENT_ID": "loap-2", "AGENTMUX_JEKT_KEY": jekt, "AGENTMUX_LAN_KEY": lan, "AGENTMUX_CHANNEL": "stable"
                }}}
            })
            .to_string();
            crate::backend::mcp_config_file::write_relocated(&file, &old).unwrap();

            realign_workspace_mcp_json(&store, &work_dir, "host", "loap-2");

            assert!(holds_no_key(&std::fs::read_to_string(&file).unwrap()));
            for root in [data.path(), dir.path()] {
                for found in files_under(root) {
                    let text = std::fs::read_to_string(&found).unwrap_or_default();
                    assert!(!text.contains(&jekt) && !text.contains(&lan), "{} holds a signing value", found.display());
                }
            }
        });
    }

    fn files_under(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(files_under(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    /// No value, and none of the names that carry one (the file holds references, not values).
    fn holds_no_key(content: &str) -> bool {
        SIGNING_SECRETS.iter().all(|k| !content.contains(k)) && !content.contains("old-key") && !content.contains("old-lan")
    }

    #[test]
    fn stripping_takes_out_the_signing_keys_and_only_them() {
        let out = strip_signing_keys(&mcp("loap-2")).unwrap();
        assert!(holds_no_key(&out));
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v.pointer(AGENT_ID).and_then(Value::as_str), Some("loap-2"));
        assert_eq!(v.pointer("/mcpServers/theirs/command").and_then(Value::as_str), Some("x"));
        assert!(strip_signing_keys(&out).is_none(), "a file with none is left alone");
        assert!(strip_signing_keys("not json").is_none());
    }

    #[test]
    fn a_container_start_removes_keys_an_earlier_launch_wrote_to_the_mounted_workspace() {
        // A user's project too: keys an older launch wrote there must not reach the container.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".mcp.json");
        std::fs::write(&file, mcp("loap-2")).unwrap();
        strip_workspace_signing_keys(&dir.path().to_string_lossy());
        assert!(holds_no_key(&std::fs::read_to_string(&file).unwrap()));

        // A file with none of our keys is left byte-for-byte, and nothing there makes none.
        let clean_body = r#"{ "mcpServers": { "mine": { "command": "mine-mcp" } } }"#;
        let clean_dir = tempfile::tempdir().unwrap();
        std::fs::write(clean_dir.path().join(".mcp.json"), clean_body).unwrap();
        strip_workspace_signing_keys(&clean_dir.path().to_string_lossy());
        assert_eq!(std::fs::read_to_string(clean_dir.path().join(".mcp.json")).unwrap(), clean_body);
        let empty = tempfile::tempdir().unwrap();
        strip_workspace_signing_keys(&empty.path().to_string_lossy());
        assert!(!empty.path().join(".mcp.json").exists());
    }

    fn agent_of_type(store: &crate::backend::storage::store::Store, agent_type: &str) -> (String, String) {
        let mut def: crate::backend::storage::agents::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "name": "Loap", "icon": "", "provider": "claude", "description": "",
            "created_at": 0, "agent_type": agent_type,
        }))
        .unwrap();
        store.agent_def_insert(&mut def).unwrap();
        (def.id, def.slug)
    }

    fn launch(store: &crate::backend::storage::store::Store, id: &str, slug: &str, hands_out_keys: bool) -> String {
        let mut files = vec![crate::backend::rpc_types::AgentConfigFile {
            path: ".mcp.json".into(),
            content: mcp(slug),
        }];
        finish_launch(&mut files, store, Some(id), hands_out_keys);
        files.remove(0).content
    }

    /// The file a launch writes is mounted into a container agent's container, so no key reaches it
    /// there, even for the AgentMux window that is otherwise handed them, and a host agent still gets its own.
    #[test]
    fn a_launch_writes_no_signing_key_for_a_container_agent() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let (boxed_id, boxed_slug) = agent_of_type(&store, "container");
        let written = launch(&store, &boxed_id, &boxed_slug, true);
        assert!(holds_no_key(&written), "{written}");
        assert!(store.agent_jekt_key_load(&boxed_slug).unwrap().is_none(), "and none minted for it");

        let (host_id, host_slug) = agent_of_type(&store, "host");
        let written = launch(&store, &host_id, &host_slug, true);
        assert!(holds_no_key(&written), "a host agent's file holds references, not values: {written}");
        let v: Value = serde_json::from_str(&written).unwrap();
        assert!(v.pointer("/mcpServers/agentmux/env/AGENTMUX_CHANNEL").is_some());
        assert!(store.agent_jekt_key_load(&host_slug).unwrap().is_some() && store.agent_lan_key_load(&host_slug).unwrap().is_some(), "its keys are ensured for its fetch");

        // The window is the one that has keys made; a launch from any other caller makes none, and writes no value either.
        let (other_id, other_slug) = agent_of_type(&store, "host");
        let written = launch(&store, &other_id, &other_slug, false);
        assert!(holds_no_key(&written), "{written}");
        assert!(store.agent_jekt_key_load(&other_slug).unwrap().is_none());
    }

    /// `agent_open` writes the same file: no value for either kind of agent; keys ensured only for a host agent.
    #[test]
    fn an_agent_open_writes_no_signing_value_for_any_agent() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        for (agent_type, keyed) in [("container", false), ("host", true)] {
            let (id, slug) = agent_of_type(&store, agent_type);
            let agent = store.agent_def_get(&id).unwrap().unwrap();
            let mut content = mcp(&slug);
            keys_for_agent_open(&mut content, &store, &slug, &agent);
            assert!(holds_no_key(&content), "{agent_type}: {content}");
            assert_eq!(store.agent_jekt_key_load(&slug).unwrap().is_some(), keyed, "{agent_type}");
        }
    }

    /// A legacy blob's own agentmux entry may carry values; they are taken out even when the keys cannot be ensured.
    #[test]
    fn a_launch_removes_values_a_legacy_entry_carries_even_when_the_keys_cannot_be_ensured() {
        let broken = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let (id, slug) = agent_of_type(&broken, "host");
        broken.test_break_name_key_tables().unwrap();
        assert!(holds_no_key(&launch(&broken, &id, &slug, true)));
        let agent = broken.agent_def_get(&id).unwrap().unwrap();
        let mut content = mcp(&slug);
        keys_for_agent_open(&mut content, &broken, &slug, &agent);
        assert!(holds_no_key(&content), "{content}");
    }

    /// No lock, no rewrite: a `.claude` that is a file makes the lock
    /// unopenable, and the stale file stays for the next spawn.
    #[test]
    fn no_rewrite_without_the_lock() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let data = tempfile::tempdir().unwrap();
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            let dir = data.path().join("agents").join("loap");
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join(".mcp.json");
            std::fs::write(&file, mcp("loap-loap-2")).unwrap();
            std::fs::write(dir.join(".claude"), "not a folder").unwrap();
            realign_workspace_mcp_json(&store, &dir.to_string_lossy(), "host", "loap-2");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), mcp("loap-loap-2"));
        });
    }
}
