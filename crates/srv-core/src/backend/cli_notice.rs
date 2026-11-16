// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane notices about an agent's CLI
//! (SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md §6.3,
//! phase LC4):
//!
//! - **Install:** while `ResolveCli` npm-installs the pinned CLI for a pane,
//!   an `agentmux_cli_install` frame says so — `installing`, then
//!   `installed` or `failed`, under one `install_id` so the row updates in
//!   place. A pane restored at startup used to install silently.
//! - **Version change:** each agent's last-run CLI version is kept in
//!   [`RECORD_FILE`], in the channel directory ([`record_dir`]). When the agent's CLI next starts on a different
//!   version, an `agentmux_cli_version_changed` frame tells the user
//!   (`from`, `to`, and the version AgentMux pins).
//!
//! Both are `system` frames appended to the pane's output, so they replay
//! with the transcript, and the digest never reads them (it only reads
//! assistant/user/result lines). They are for the user; the agent's context
//! is unchanged.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const INSTALL_SUBTYPE: &str = "agentmux_cli_install";
/// Subtype of the frames that say how a container agent's container is coming up.
pub const CONTAINER_START_SUBTYPE: &str = "agentmux_container_start";
pub const VERSION_CHANGED_SUBTYPE: &str = "agentmux_cli_version_changed";
pub const CONTAINER_IMAGE_SUBTYPE: &str = "agentmux_container_image";
pub const CONTAINER_LIMIT_SUBTYPE: &str = "agentmux_container_limit";
pub const CONTAINER_CACHES_SUBTYPE: &str = "agentmux_container_caches";
/// Subtype of the frame that says a container agent is ready and waiting for its first message.
pub const AGENT_READY_SUBTYPE: &str = "agentmux_agent_ready";

/// Per-agent last-run CLI versions, in [`record_dir`] (no schema change: a
/// new column would lock older builds out of a shared store.db).
pub const RECORD_FILE: &str = "agent-cli-versions.json";

/// Where [`RECORD_FILE`] lives: the channel directory
/// (`~/.agentmux/channels/<channel>/`, or a dev build's own root), next to
/// the shared store that holds the agents it is keyed by. Not `data_dir`:
/// an installed build's data dir is per AgentMux version
/// (`channels/<ch>/versions/<v>/data/`), so after an upgrade every agent
/// would read as a first run and the upgrade this exists to report would
/// go unnoticed (ReAgent P1 on #4128).
pub fn record_dir(paths: &agentmux_common::DataPaths) -> &Path {
    &paths.instance_dir
}

/// Where an install stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallState {
    Installing,
    Installed,
    Failed,
}

impl InstallState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Failed => "failed",
        }
    }
}

/// The version number in a CLI's `--version` output, e.g. `"2.1.285"` from
/// `"2.1.285 (Claude Code)"` or `"codex-cli 0.154.0"`. `None` if there is
/// none (`"unknown"`, an empty string).
pub fn version_token(raw: &str) -> Option<String> {
    raw.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.contains('.') && t.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

/// An `agentmux_cli_install` frame. `install_id` is the same for every frame
/// of one install, so the pane updates one row.
pub fn install_frame(
    install_id: &str,
    provider: &str,
    version: &str,
    state: InstallState,
    seconds: Option<f64>,
    error: Option<&str>,
) -> Value {
    let mut frame = json!({
        "type": "system",
        "subtype": INSTALL_SUBTYPE,
        "install_id": install_id,
        "provider": provider,
        "version": version,
        "state": state.as_str(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(s) = seconds {
        frame["seconds"] = json!((s * 10.0).round() / 10.0);
    }
    if let Some(e) = error {
        frame["error"] = json!(e);
    }
    frame
}

/// Where a container agent's start-up is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerStartState {
    /// The image is being downloaded.
    Pulling,
    /// The image is here; the container is being created.
    Starting,
    /// The container is up.
    Ready,
    Failed,
}

impl ContainerStartState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pulling => "pulling",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

/// An `agentmux_container_start` frame. `start_id` is the same for every frame
/// of one start-up, so the pane updates one row. `percent` is how much of the
/// image has arrived, once the daemon has said.
pub fn container_start_frame(
    start_id: &str,
    provider: &str,
    image: &str,
    state: ContainerStartState,
    percent: Option<u8>,
    seconds: Option<f64>,
    error: Option<&str>,
) -> Value {
    let mut frame = json!({
        "type": "system",
        "subtype": CONTAINER_START_SUBTYPE,
        "start_id": start_id,
        "provider": provider,
        "image": image,
        "state": state.as_str(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(p) = percent {
        frame["percent"] = json!(p.min(100));
    }
    if let Some(s) = seconds {
        frame["seconds"] = json!((s * 10.0).round() / 10.0);
    }
    if let Some(e) = error {
        frame["error"] = json!(e);
    }
    frame
}

/// An `agentmux_agent_ready` frame: a container agent's container and CLI are up and nothing is on
/// its way to it, so the pane waits for the user's first message. `seconds` is the whole start-up.
pub fn agent_ready_frame(ready_id: &str, provider: &str, seconds: f64) -> Value {
    json!({
        "type": "system",
        "subtype": AGENT_READY_SUBTYPE,
        "ready_id": ready_id,
        "provider": provider,
        "seconds": (seconds * 10.0).round() / 10.0,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
}

/// An `agentmux_container_image` frame: the container runs an older agent image than this
/// AgentMux (`state` `outdated`), has just been removed on request so the next message starts it from
/// the current one (`updated`), or was moved to the current one by AgentMux itself while the agent was idle
/// (`auto_updated`).
pub fn container_image_frame(provider: &str, block_id: &str, state: &str, running: &str, current: &str) -> Value {
    json!({
        "type": "system",
        "subtype": CONTAINER_IMAGE_SUBTYPE,
        "provider": provider,
        "block_id": block_id,
        "state": state,
        "running": running,
        "current": current,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
}

/// An `agentmux_container_limit` frame: a container's CPU or memory limit changed (`state` `changed`, by `by`),
/// something in the container was stopped for going over its memory limit (`oom`), or the daemon refused a limit
/// (`failed`, with `error`). `agent` is the agent's id and name, for the row's button. `memory_mb` and `cpus` are
/// the limits in force; each frame is its own row.
#[allow(clippy::too_many_arguments)]
pub fn container_limit_frame(
    provider: &str,
    block_id: &str,
    agent: (&str, &str),
    state: &str,
    memory_mb: Option<u64>,
    cpus: Option<f64>,
    by: Option<&str>,
    error: Option<&str>,
) -> Value {
    let mut frame = json!({
        "type": "system",
        "subtype": CONTAINER_LIMIT_SUBTYPE,
        "provider": provider,
        "block_id": block_id,
        "agent_id": agent.0,
        "agent_name": agent.1,
        "event_id": uuid::Uuid::new_v4().to_string(),
        "state": state,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(mb) = memory_mb {
        frame["memory_mb"] = json!(mb);
    }
    if let Some(cpus) = cpus {
        frame["cpus"] = json!((cpus * 100.0).round() / 100.0);
    }
    if let Some(by) = by {
        frame["by"] = json!(by);
    }
    if let Some(error) = error {
        frame["error"] = json!(error);
    }
    frame
}

/// An `agentmux_container_caches` frame: the cache volumes of a container agent (a developer agent's cargo registry and
/// build output) hold `bytes`, more than the warning level. `agent` is the agent's id and name, for the row's button.
pub fn container_caches_frame(provider: &str, block_id: &str, agent: (&str, &str), bytes: u64) -> Value {
    json!({
        "type": "system",
        "subtype": CONTAINER_CACHES_SUBTYPE,
        "provider": provider,
        "block_id": block_id,
        "agent_id": agent.0,
        "agent_name": agent.1,
        "event_id": uuid::Uuid::new_v4().to_string(),
        "bytes": bytes,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
}

/// An `agentmux_cli_version_changed` frame. `pinned` is the version this
/// AgentMux build installs, so the pane can say when `to` isn't it (a
/// self-update, or a stale install).
pub fn version_changed_frame(provider: &str, from: &str, to: &str, pinned: Option<&str>) -> Value {
    json!({
        "type": "system",
        "subtype": VERSION_CHANGED_SUBTYPE,
        "provider": provider,
        "from": from,
        "to": to,
        "pinned": pinned,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    })
}

/// Append `frame` to `block_id`'s output (and the agent's global transcript
/// zone), as the memory-delivery notice does.
pub fn append_to_pane(
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
    block_id: &str,
    frame: &Value,
) {
    append_line_to_pane(broker, filestore, mstore, block_id, &frame.to_string());
}

/// [`append_to_pane`] for a frame already serialized to its one line (no
/// trailing newline).
pub fn append_line_to_pane(
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
    block_id: &str,
    line: &str,
) {
    let line = format!("{line}\n");
    let zone = crate::backend::transcript::append::resolve_global_output_zone(&Some(mstore.clone()), block_id);
    crate::backend::blockcontroller::shell::handle_append_block_file(
        broker,
        block_id,
        crate::backend::blockcontroller::persistent::PERSISTENT_OUTPUT_SUBJECT,
        line.as_bytes(),
        Some(filestore),
        zone.as_deref(),
    );
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LastCli {
    provider: String,
    version: String,
    at: i64,
}

/// Serializes read-modify-write of [`RECORD_FILE`] within this srv.
static RECORD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Record that `agent_key`'s CLI just started on `version`, and return the
/// version it last ran if that was a different one of the same provider.
/// A first run, or a switch to another provider, records and returns `None`.
/// A record that can't be read or written is logged and treated as empty:
/// at worst a notice is missed, never a launch.
pub fn observe_version(dir: &Path, agent_key: &str, provider: &str, version: &str) -> Option<String> {
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = dir.join(RECORD_FILE);
    let mut records: BTreeMap<String, LastCli> = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let previous = records.get(agent_key).cloned();
    if previous.as_ref().is_some_and(|p| p.provider == provider && p.version == version) {
        return None;
    }
    records.insert(
        agent_key.to_string(),
        LastCli { provider: provider.to_string(), version: version.to_string(), at: agentmux_common::time::now_ms() },
    );
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(&records)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "cli notice: could not save the last-run CLI version");
    }
    previous.filter(|p| p.provider == provider).map(|p| p.version)
}

/// The key an agent's record is kept under: the block's `agentId` (the
/// agent's UID, or a provider key for a quick-launch pane). `None` for a
/// block with no agent.
pub fn agent_key_of_block(mstore: &crate::backend::storage::store::Store, block_id: &str) -> Option<String> {
    let block = mstore.get::<crate::backend::obj::Block>(block_id).ok().flatten()?;
    let id = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
    (!id.trim().is_empty()).then_some(id)
}

/// Observe `version` for an agent and, if it changed, append the notice to
/// `block_id`'s pane. The agent is `agent_id` when the caller knows it (a
/// new pane resolves its CLI before its `agentId` meta is written), else
/// the pane's `agentId`. The one entry point both sources use: Claude's own
/// `system/init` frame, and `ResolveCli` for the other providers.
pub fn observe_and_notify(
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
    block_id: &str,
    agent_id: Option<&str>,
    provider: &str,
    version: &str,
) {
    let Some(agent_key) = agent_id.map(str::to_string).or_else(|| agent_key_of_block(mstore, block_id)) else {
        return;
    };
    let Some(paths) = agentmux_common::DataPaths::from_env() else { return };
    if let Some(from) = observe_version(record_dir(&paths), &agent_key, provider, version) {
        let pinned = crate::backend::providers::get_provider(provider).map(|p| p.pinned_version);
        tracing::info!(block_id, provider, from = %from, to = version, "cli notice: agent's CLI version changed");
        append_to_pane(broker, filestore, mstore, block_id, &version_changed_frame(provider, &from, version, pinned));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_container_image_frame_names_its_pane_and_both_versions() {
        let f = container_image_frame("claude", "blk-1", "outdated", "0.59.41", "0.60.2");
        assert_eq!(f["subtype"], CONTAINER_IMAGE_SUBTYPE);
        assert_eq!((f["block_id"].as_str(), f["state"].as_str()), (Some("blk-1"), Some("outdated")));
        assert_eq!((f["running"].as_str(), f["current"].as_str()), (Some("0.59.41"), Some("0.60.2")));
    }

    #[test]
    fn the_container_limit_frame_carries_the_limits_in_force_and_who_changed_them() {
        let f = container_limit_frame("claude", "blk-1", ("a-1", "Aria"), "changed", Some(8192), Some(6.004), Some("Agent1"), None);
        assert_eq!(f["subtype"], CONTAINER_LIMIT_SUBTYPE);
        assert_eq!((f["state"].as_str(), f["block_id"].as_str(), f["by"].as_str()), (Some("changed"), Some("blk-1"), Some("Agent1")));
        assert_eq!((f["memory_mb"].as_u64(), f["cpus"].as_f64()), (Some(8192), Some(6.0)));
        assert!(f["event_id"].as_str().is_some_and(|id| !id.is_empty()), "each event is its own row");
        assert_eq!((f["agent_id"].as_str(), f["agent_name"].as_str()), (Some("a-1"), Some("Aria")));
        let oom = container_limit_frame("claude", "blk-1", ("a-1", "Aria"), "oom", Some(256), None, None, None);
        assert!(oom.get("by").is_none() && oom.get("cpus").is_none() && oom.get("error").is_none());
    }

    #[test]
    fn the_container_caches_frame_carries_the_size_and_whose_container_it_is() {
        let f = container_caches_frame("claude", "blk-1", ("a-1", "Aria"), 25_769_803_776);
        assert_eq!(f["subtype"], CONTAINER_CACHES_SUBTYPE);
        assert_eq!((f["bytes"].as_u64(), f["block_id"].as_str()), (Some(25_769_803_776), Some("blk-1")));
        assert_eq!((f["agent_id"].as_str(), f["agent_name"].as_str()), (Some("a-1"), Some("Aria")));
        assert!(f["event_id"].as_str().is_some_and(|id| !id.is_empty()));
    }

    /// The pane (`frontend/app/view/agent/cli-notice.ts`) reads these field names.
    #[test]
    fn a_container_start_frame_carries_what_the_pane_reads() {
        let f = container_start_frame("s1", "claude", "img:1", ContainerStartState::Pulling, Some(150), None, None);
        assert_eq!(f["type"], "system");
        assert_eq!(f["subtype"], "agentmux_container_start");
        assert_eq!(f["start_id"], "s1");
        assert_eq!(f["provider"], "claude");
        assert_eq!(f["image"], "img:1");
        assert_eq!(f["state"], "pulling");
        assert_eq!(f["percent"], 100, "a percentage never reads above 100");
        assert!(f.get("seconds").is_none() && f.get("error").is_none());
        let failed = container_start_frame("s1", "claude", "img:1", ContainerStartState::Failed, None, None, Some("why"));
        assert_eq!((failed["state"].as_str(), failed["error"].as_str()), (Some("failed"), Some("why")));
    }

    #[test]
    fn version_token_finds_the_number() {
        assert_eq!(version_token("2.1.285 (Claude Code)").as_deref(), Some("2.1.285"));
        assert_eq!(version_token("codex-cli 0.154.0").as_deref(), Some("0.154.0"));
        assert_eq!(version_token("v0.60.0").as_deref(), Some("0.60.0"));
        assert_eq!(version_token("unknown"), None);
        assert_eq!(version_token(""), None);
    }

    #[test]
    fn first_run_records_without_a_notice_then_a_change_returns_the_old_version() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(observe_version(dir.path(), "uid-1", "claude", "2.1.218"), None);
        assert_eq!(observe_version(dir.path(), "uid-1", "claude", "2.1.218"), None, "same version");
        assert_eq!(observe_version(dir.path(), "uid-1", "claude", "2.1.285").as_deref(), Some("2.1.218"));
        assert_eq!(observe_version(dir.path(), "uid-1", "claude", "2.1.285"), None, "recorded the new one");
    }

    #[test]
    fn agents_are_independent_and_a_provider_switch_is_not_an_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        observe_version(dir.path(), "uid-1", "claude", "2.1.218");
        assert_eq!(observe_version(dir.path(), "uid-2", "claude", "2.1.285"), None, "other agent's first run");
        assert_eq!(observe_version(dir.path(), "uid-1", "codex", "0.154.0"), None, "switched provider");
        assert_eq!(observe_version(dir.path(), "uid-1", "codex", "0.155.0").as_deref(), Some("0.154.0"));
    }

    #[test]
    fn an_unreadable_record_is_treated_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(RECORD_FILE), b"not json").unwrap();
        assert_eq!(observe_version(dir.path(), "uid-1", "claude", "2.1.285"), None);
        assert_eq!(observe_version(dir.path(), "uid-1", "claude", "2.1.286").as_deref(), Some("2.1.285"));
    }

    #[test]
    fn frames_carry_what_the_pane_shows() {
        let f = install_frame("claude-2.1.285-1", "claude", "2.1.285", InstallState::Installed, Some(14.04), None);
        assert_eq!(f["subtype"], INSTALL_SUBTYPE);
        assert_eq!(f["state"], "installed");
        assert_eq!(f["seconds"], 14.0);
        assert!(f.get("error").is_none());
        let failed = install_frame("i", "claude", "2.1.285", InstallState::Failed, None, Some("exit 1"));
        assert_eq!(failed["error"], "exit 1");

        let ready = agent_ready_frame("r1", "claude", 18.04);
        assert_eq!((ready["subtype"].as_str(), ready["ready_id"].as_str(), ready["seconds"].as_f64()), (Some(AGENT_READY_SUBTYPE), Some("r1"), Some(18.0)));

        let v = version_changed_frame("claude", "2.1.218", "2.1.285", Some("2.1.285"));
        assert_eq!(v["subtype"], VERSION_CHANGED_SUBTYPE);
        assert_eq!((v["from"].as_str(), v["to"].as_str(), v["pinned"].as_str()), (Some("2.1.218"), Some("2.1.285"), Some("2.1.285")));
    }
}
