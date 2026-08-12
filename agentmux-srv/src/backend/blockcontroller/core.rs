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

/// Block metadata key for the last classified agent failure.
/// Written on every non-zero / in-band-error exit; cleared on clean success.
/// The frontend reads this on pane mount so the recovery banner survives
/// tab switches and page reloads without requiring the WPS event to be
/// received in real time.
pub(crate) const META_LAST_FAILURE: &str = "agent:last_failure";

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
/// Also writes through to the matching `db_agent_instances` row's
/// `session_id` column (SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md
/// §4.1). Without this, that column was set to `""` at instance creation and
/// never updated again in production — `ListRecentSessionsCommand` prefers
/// the local row over the shared registry whenever both exist, so the "My
/// Agents" picker's reattach flow (`AgentPicker.tsx`'s `handleReattach`) was
/// handed an empty session id even mid-session, and — per
/// `agent-model.ts`'s own documented invariant that an empty
/// `continueSessionId` clears `agent:sessionid` on the new block — silently
/// started a genuinely fresh conversation instead of resuming. Best-effort:
/// logs and continues on failure, since the block-meta write above (the
/// live-turn source of truth) already succeeded.
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
            sync_instance_session_id(store, block_id, sid);

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

/// Write `sid` into the `db_agent_instances` row for `block_id`, if one
/// exists. Best-effort — no-ops (with a debug log, not a warn — most turns
/// on a block with no matching instance row are expected, e.g. terminal
/// panes never have one) when there's nothing to update.
fn sync_instance_session_id(store: &Arc<Store>, block_id: &str, sid: &str) {
    let instance = match store.instance_get_by_block_id(block_id) {
        Ok(Some(i)) => i,
        Ok(None) => return,
        Err(e) => {
            tracing::debug!(
                block_id = %block_id,
                error = %e,
                "sync_instance_session_id: instance lookup failed"
            );
            return;
        }
    };
    let upd = crate::backend::storage::InstanceUpdate {
        session_id: Some(sid.to_string()),
        ..Default::default()
    };
    if let Err(e) = store.instance_update_partial(&instance.id, &upd) {
        tracing::warn!(
            block_id = %block_id,
            instance_id = %instance.id,
            error = %e,
            "sync_instance_session_id: failed to write session_id"
        );
    }
}

/// Persist or clear the last agent failure in block metadata.
///
/// Pass `Some(failure)` on a failed exit to write `agent:last_failure` into the
/// block's meta so the pane can recover the recovery banner on any future load
/// without needing the ephemeral WPS event. Pass `None` on a clean exit to
/// remove the key (setting it to JSON null triggers `merge_meta`'s delete path).
/// Broadcasts a `waveobj:update` so active frontend subscribers see the change
/// immediately via the block atom, not just on next full load.
pub(crate) fn persist_last_failure(
    block_id: &str,
    failure: Option<&crate::agents::failure::AgentFailure>,
    wstore: &Option<Arc<Store>>,
    event_bus: &Option<Arc<EventBus>>,
) {
    let Some(ref store) = wstore else {
        return;
    };
    // On a clean exit (failure=None), only write null (which merge_meta uses to
    // delete the key) if the key actually exists — otherwise every successful
    // turn would trigger a redundant DB write + waveobj:update broadcast.
    if failure.is_none() {
        let key_exists = store
            .must_get::<crate::backend::obj::Block>(block_id)
            .ok()
            .and_then(|b| b.meta.get(META_LAST_FAILURE).cloned())
            .map(|v| !v.is_null())
            .unwrap_or(false);
        if !key_exists {
            return;
        }
    }
    let oref_str = format!("block:{}", block_id);
    let mut meta_update = crate::backend::obj::MetaMapType::new();
    let val = match failure {
        Some(f) => match serde_json::to_value(f) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id,
                    error = %e,
                    "failed to serialize AgentFailure for agent:last_failure — skipping meta write",
                );
                return;
            }
        },
        None => serde_json::Value::Null, // null → merge_meta removes the key
    };
    meta_update.insert(META_LAST_FAILURE.to_string(), val);
    match crate::server::service::update_object_meta(store, &oref_str, &meta_update) {
        Err(e) => {
            tracing::warn!(
                block_id = %block_id,
                error = %e,
                "failed to persist agent:last_failure",
            );
        }
        Ok(_) => {
            let Some(ref bus) = event_bus else {
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
                bus.broadcast_event(&crate::backend::eventbus::WSEventType {
                    eventtype: "waveobj:update".to_string(),
                    oref: oref_str,
                    data: update_data,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{Block, MetaMapType};
    use crate::backend::storage::store::{AgentDefinition, AgentInstance, InstanceStatus};

    /// Minimal definition satisfying `db_agent_instances`'s FK on
    /// `definition_id` — field values otherwise irrelevant to these tests.
    fn sample_agent(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            slug: id.to_string(),
            name: id.to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        }
    }

    /// SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md §4.1: this
    /// is the end-to-end assertion that `persist_session_id` (the live-turn
    /// call site every controller uses) keeps `db_agent_instances.session_id`
    /// current, not just the block's own `agent:sessionid` meta — closing the
    /// gap where "My Agents"'s reattach flow saw a permanently-empty local
    /// session id.
    #[test]
    fn persist_session_id_writes_through_to_the_instance_row() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let block_id = "55555555-5555-5555-5555-555555555555";
        let mut block = Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        let mut def = sample_agent("def-live");
        store.agent_def_insert(&mut def).unwrap();

        let inst = AgentInstance {
            id: "inst-live".to_string(),
            definition_id: "def-live".to_string(),
            parent_instance_id: String::new(),
            block_id: block_id.to_string(),
            session_id: String::new(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        };
        store.instance_create(&inst).unwrap();

        persist_session_id(block_id, "sess-abc", &Some(store.clone()), &None);

        // Block meta: the pre-existing behavior.
        let updated_block: Block = store.get(block_id).unwrap().unwrap();
        assert_eq!(
            updated_block.meta.get(META_SESSION_ID).and_then(|v| v.as_str()),
            Some("sess-abc"),
        );

        // Instance row: the new write-through.
        let updated_instance = store.instance_get_by_block_id(block_id).unwrap().unwrap();
        assert_eq!(updated_instance.session_id, "sess-abc");
    }

    /// A block with no matching instance row (e.g. a terminal pane) must not
    /// cause persist_session_id to error or panic — sync_instance_session_id
    /// is best-effort and silently no-ops.
    #[test]
    fn persist_session_id_is_a_noop_for_a_block_with_no_instance_row() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let block_id = "66666666-6666-6666-6666-666666666666";
        let mut block = Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: MetaMapType::new(),
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        // Must not panic.
        persist_session_id(block_id, "sess-xyz", &Some(store.clone()), &None);

        let updated_block: Block = store.get(block_id).unwrap().unwrap();
        assert_eq!(
            updated_block.meta.get(META_SESSION_ID).and_then(|v| v.as_str()),
            Some("sess-xyz"),
            "block meta write still happens regardless of instance-row presence",
        );
    }
}
