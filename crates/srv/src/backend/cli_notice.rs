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
//!   [`RECORD_FILE`]. When the agent's CLI next starts on a different
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
pub const VERSION_CHANGED_SUBTYPE: &str = "agentmux_cli_version_changed";

/// Per-agent last-run CLI versions, in the srv data dir (no schema change:
/// a new column would lock older builds out of a shared store.db).
pub const RECORD_FILE: &str = "agent-cli-versions.json";

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
    let line = format!("{frame}\n");
    let zone = crate::backend::blockcontroller::shell::resolve_global_output_zone(&Some(mstore.clone()), block_id);
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
pub fn observe_version(data_dir: &Path, agent_key: &str, provider: &str, version: &str) -> Option<String> {
    let _guard = RECORD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = data_dir.join(RECORD_FILE);
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

/// Observe `version` for the agent on `block_id` and, if it changed, append
/// the notice to that pane. The one entry point both sources use: Claude's
/// own `system/init` frame, and `ResolveCli` for the other providers.
pub fn observe_and_notify(
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
    block_id: &str,
    provider: &str,
    version: &str,
) {
    let Some(agent_key) = agent_key_of_block(mstore, block_id) else { return };
    let Some(data_dir) = agentmux_common::DataPaths::from_env().map(|p| p.data_dir) else { return };
    if let Some(from) = observe_version(&data_dir, &agent_key, provider, version) {
        let pinned = crate::backend::providers::get_provider(provider).map(|p| p.pinned_version);
        tracing::info!(block_id, provider, from = %from, to = version, "cli notice: agent's CLI version changed");
        append_to_pane(broker, filestore, mstore, block_id, &version_changed_frame(provider, &from, version, pinned));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let v = version_changed_frame("claude", "2.1.218", "2.1.285", Some("2.1.285"));
        assert_eq!(v["subtype"], VERSION_CHANGED_SUBTYPE);
        assert_eq!((v["from"].as_str(), v["to"].as_str(), v["pinned"].as_str()), (Some("2.1.218"), Some("2.1.285"), Some("2.1.285")));
    }
}
