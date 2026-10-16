// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The note each agent gets when it resumes after AgentMux was upgraded
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §5.2,
//! phase 4b): `[AgentMux] AgentMux was upgraded from 0.59.26 to 0.59.27 …`,
//! once per agent, as its next input. The agent that asked for the upgrade
//! (`App` `upgrade`) gets it too, which is how it learns the upgrade is done.
//!
//! srv records the version it runs in the channel folder ([`RECORD_FILE`],
//! like `cli_notice`'s CLI versions): an installed build's data folder is
//! per version, so the new version's own would read as a first start. A
//! start on a newer version than the one recorded is an upgrade; a first
//! start, the same version or an older one (a downgrade, or the older build
//! started again) is not.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The last AgentMux version this channel ran, in its folder.
pub const RECORD_FILE: &str = "agentmux-last-version.json";
/// How long a resumed agent is waited for to register before it is told.
const REGISTER_WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LastRun {
    version: String,
    at: i64,
}

/// This start was an upgrade from `from` to `to`, at `at_ms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgraded {
    pub from: String,
    pub to: String,
    pub at_ms: i64,
}

/// Record that `version` started now, and return the upgrade it was, if the
/// recorded version is an older release. A record that can't be read or
/// written is logged and treated as none: at worst a note is missed.
pub fn observe_start(dir: &Path, version: &str, now_ms: i64) -> Option<Upgraded> {
    let path = dir.join(RECORD_FILE);
    let previous: Option<LastRun> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let record = LastRun { version: version.to_string(), at: now_ms };
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec_pretty(&record)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "upgrade notice: could not record this version");
    }
    let from = previous?.version;
    let newer = crate::backend::update_check::version_core(version) > crate::backend::update_check::version_core(&from)
        && crate::backend::update_check::version_core(&from).is_some();
    newer.then(|| Upgraded { from, to: version.to_string(), at_ms: now_ms })
}

/// The note, with the restart's local time.
pub fn note_text(u: &Upgraded) -> String {
    let at = chrono::DateTime::from_timestamp_millis(u.at_ms)
        .map(|t| t.with_timezone(&chrono::Local).format(" at %H:%M").to_string())
        .unwrap_or_default();
    format!(
        "[AgentMux] AgentMux was upgraded from {} to {}{at}. Your session resumed; nothing is needed from you.",
        u.from, u.to
    )
}

static UPGRADED: OnceLock<Option<Upgraded>> = OnceLock::new();
static NOTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// At srv start (`bootstrap::background`): record this version and remember
/// whether the start was an upgrade.
pub fn init() {
    let upgraded = agentmux_common::DataPaths::from_env().and_then(|paths| {
        observe_start(&paths.instance_dir, env!("CARGO_PKG_VERSION"), agentmux_common::time::now_ms())
    });
    if let Some(u) = &upgraded {
        tracing::info!(from = %u.from, to = %u.to, "upgrade notice: this start is an upgrade; resumed agents will be told");
    }
    let _ = UPGRADED.set(upgraded);
}

/// An agent's session was resumed in `block_id` (eager resume): if this
/// start was an upgrade, tell it, once, when it has registered.
pub fn on_resumed(block_id: &str) {
    let Some(Some(upgraded)) = UPGRADED.get() else { return };
    {
        let mut noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
        if !noted.get_or_insert_with(HashSet::new).insert(block_id.to_string()) {
            return;
        }
    }
    let (block_id, text) = (block_id.to_string(), note_text(upgraded));
    tokio::spawn(async move {
        let handler = crate::backend::reactive::get_global_handler();
        let deadline = tokio::time::Instant::now() + REGISTER_WAIT;
        loop {
            let target = handler
                .list_agents()
                .into_iter()
                .find(|r| r.block_id == block_id)
                .map(|r| r.uid.unwrap_or(r.agent_id));
            if let Some(target) = target {
                if let Err(e) = handler.deliver_system_note(&target, &text) {
                    tracing::info!(block_id = %block_id, error = %e, "upgrade notice: not delivered");
                }
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::info!(block_id = %block_id, "upgrade notice: the agent didn't register in time");
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_start_on_a_newer_release_is_an_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        // First start: recorded, nothing to say.
        assert_eq!(observe_start(dir.path(), "0.59.26", 1), None);
        // The same version again.
        assert_eq!(observe_start(dir.path(), "0.59.26", 2), None);
        // Upgraded.
        assert_eq!(
            observe_start(dir.path(), "0.59.27", 3),
            Some(Upgraded { from: "0.59.26".into(), to: "0.59.27".into(), at_ms: 3 })
        );
        // The older build started again isn't one, and the newer one after it
        // is, from the older.
        assert_eq!(observe_start(dir.path(), "0.59.26", 4), None);
        assert_eq!(observe_start(dir.path(), "0.59.100", 5).map(|u| u.from), Some("0.59.26".into()));
    }

    #[test]
    fn an_unreadable_record_is_a_first_start() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(RECORD_FILE), b"not json").unwrap();
        assert_eq!(observe_start(dir.path(), "0.59.27", 1), None);
        assert_eq!(observe_start(dir.path(), "0.59.28", 2).map(|u| u.from), Some("0.59.27".into()));
    }

    #[test]
    fn the_note_is_a_system_note() {
        let text = note_text(&Upgraded { from: "0.59.26".into(), to: "0.59.27".into(), at_ms: 1_792_000_000_000 });
        assert!(text.starts_with("[AgentMux] AgentMux was upgraded from 0.59.26 to 0.59.27 at "), "{text}");
        assert!(text.ends_with("Your session resumed; nothing is needed from you."));
    }
}
