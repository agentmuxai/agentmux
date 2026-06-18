// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared helpers used by all block controllers (persistent, subprocess, acp).
//!
//! Extracted to avoid copy-paste across the four controller files. Each
//! controller keeps only its transport-specific logic (PTY / stream-json / ACP)
//! and delegates the common mechanics here.

use std::collections::HashMap;
use std::sync::Arc;

use super::health::HealthMonitor;
use crate::backend::eventbus::EventBus;
use crate::backend::storage::store::Store;

/// Block metadata key for the persisted agent session ID.
/// Used by the "My Agents" reattach path on the frontend: after a tab reload
/// the picker reads `block.meta["agent:sessionid"]` and passes it as the
/// `--resume` value so the CLI picks up the prior conversation.
pub(crate) const META_SESSION_ID: &str = "agent:sessionid";

/// Expand the working directory, create it if missing, set it on `cmd`, and
/// apply all env-var overrides from `env_vars`.
///
/// Call this from each controller's spawn routine BEFORE any transport-specific
/// `Command` flags (e.g. `CREATE_NO_WINDOW` on Windows, which must come after
/// the env setup in some implementations).
pub(crate) fn apply_working_dir(
    cmd: &mut tokio::process::Command,
    block_id: &str,
    working_dir: &str,
    env_vars: &HashMap<String, String>,
) {
    if !working_dir.is_empty() {
        let expanded_dir = expand_home_dir(working_dir);
        let dir_path = std::path::Path::new(&expanded_dir);
        if !dir_path.exists() {
            if let Err(e) = std::fs::create_dir_all(dir_path) {
                tracing::warn!(
                    block_id = %block_id,
                    dir = %expanded_dir,
                    error = %e,
                    "failed to create working directory",
                );
            }
        }
        if dir_path.exists() {
            cmd.current_dir(&expanded_dir);
            // Warn loudly if the agent workspace contains a nested .git (e.g.
            // an unintended `git clone` inside ~/.agentmux/agents/). A stale
            // nested clone can confuse agents into reading old code and waste
            // gigabytes of disk. Single fs::metadata call — no directory walk.
            let looks_like_agent_workspace = expanded_dir.contains("/.agentmux/agents/")
                || expanded_dir.contains("\\.agentmux\\agents\\");
            if looks_like_agent_workspace {
                let git_dir = dir_path.join(".git");
                if git_dir.exists() {
                    tracing::warn!(
                        block_id = %block_id,
                        cwd = %expanded_dir,
                        ".git detected inside agent workspace — this is usually \
                         an unintended nested clone and can waste gigabytes of \
                         disk. Clean up with: rm -rf {}/.git",
                        expanded_dir,
                    );
                }
            }
        }
    }
    for (k, v) in env_vars {
        let expanded = crate::backend::base::expand_home_dir_safe(v);
        cmd.env(k, expanded.to_string_lossy().as_ref());
    }
}

/// Expand a leading `~/` or bare `~` to the user's home directory.
pub(crate) fn expand_home_dir(dir: &str) -> String {
    if dir.starts_with("~/") || dir == "~" {
        if let Some(home) = dirs::home_dir() {
            return home
                .join(dir.trim_start_matches("~/"))
                .to_string_lossy()
                .to_string();
        }
    }
    dir.to_string()
}

/// Spawn the health-watchdog background task for a turn.
///
/// The watchdog polls `health_monitor.check()` every 5 s while a turn is
/// active and exits as soon as `is_active_turn()` returns false (i.e. after
/// the turn ends or the process exits). Duplicated verbatim in
/// persistent.rs and subprocess.rs (twice) before this extraction.
pub(crate) fn spawn_health_watchdog(health_monitor: &Arc<HealthMonitor>) {
    let health = Arc::clone(health_monitor);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            if !health.is_active_turn() {
                break;
            }
            health.check();
        }
    });
}

/// Persist a newly captured session ID to block metadata and broadcast the
/// `waveobj:update` event so the frontend reflects the change immediately.
///
/// This is the "careful" path from persistent.rs and subprocess.rs. ACP was
/// missing this step (it only set `inner.session_id` in memory) — A5 fixes it
/// by routing ACP through this same function.
///
/// No-ops silently when `wstore` is `None` (e.g. in unit tests that don't wire
/// up a store).
pub(crate) fn persist_session_id(
    block_id: &str,
    sid: &str,
    wstore: &Option<Arc<Store>>,
    event_bus: &Option<Arc<EventBus>>,
) {
    let Some(ref store) = wstore else {
        return;
    };
    let oref_str = format!("block:{}", block_id);
    let mut meta_update = crate::backend::obj::MetaMapType::new();
    meta_update.insert(
        META_SESSION_ID.to_string(),
        serde_json::Value::String(sid.to_string()),
    );
    match crate::server::service::update_object_meta(store, &oref_str, &meta_update) {
        Err(e) => {
            tracing::warn!(
                block_id = %block_id,
                error = %e,
                "failed to persist agent:sessionid",
            );
        }
        Ok(_) => {
            let Some(ref event_bus) = event_bus else {
                return;
            };
            if let Ok(updated_block) =
                store.must_get::<crate::backend::obj::Block>(block_id)
            {
                let update_data = serde_json::to_value(
                    &crate::backend::obj::WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: "block".into(),
                        oid: block_id.to_string(),
                        obj: Some(crate::backend::obj::wave_obj_to_value(&updated_block)),
                    },
                )
                .ok();
                event_bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                    eventtype: "waveobj:update".to_string(),
                    oref: oref_str,
                    data: update_data,
                });
            }
        }
    }
}
