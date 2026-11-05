// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `start` does once the shell's PTY child exists: records it, registers the pane with
//! jekt and seeds its working directory.

use super::controller::ShellController;
use crate::backend::obj::{self, MetaMapType};

impl ShellController {
    /// Registers the child's PID with the per-pane metrics and records its spawn metadata.
    pub(super) fn record_spawn(
        &self,
        pid: Option<u32>,
        is_agent: bool,
        prog_name: &str,
        agent_id: Option<String>,
        spawn_ts_ms: i64,
    ) {
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(pid) = pid {
                super::super::pidregistry::register(&self.block_id, pid);
                if is_agent {
                    crate::backend::process_tracker::registry::track_spawned_agent(
                        &self.block_id,
                        pid,
                    );
                } else {
                    crate::backend::process_tracker::registry::track_spawned(&self.block_id, pid);
                }
                inner.child_pid = Some(pid);
            }
            inner.spawn_ts_ms = Some(spawn_ts_ms);
            inner.shell_name = prog_name.to_string();
            inner.is_agent_pane = is_agent;
            inner.agent_id = agent_id;
        }
    }

    /// Maps `agent_id` to this pane in the jekt handler (and the cross-instance registry), so
    /// jekt can deliver to this PTY without a separate register call.
    pub(super) fn register_agent_with_jekt(&self, agent_id: &str) {
        // Identity M2: a PTY pane has no `db_agents` row, so no UID —
        // registered by name only, counted (spec §4.4.1 row 4).
        match crate::backend::reactive::get_global_handler().register_agent_full(
            agent_id,
            &self.block_id,
            Some(&self.tab_id),
            0,
            None,
            None,
            "registration.no_uid.shell_pane",
        ) {
            Ok(()) => {
                tracing::info!(
                    block_id = %self.block_id,
                    agent_id = %agent_id,
                    "jekt: auto-registered"
                );
                // Also write to cross-instance file registry, and its
                // host-global sibling (Tier 2b, issue #1916) — this
                // auto-register path bypasses the HTTP register handler
                // entirely, so it needs its own mirror call too.
                if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
                    let data_dir = crate::backend::base::get_mux_data_dir();
                    crate::backend::reactive::registry::write(
                        &data_dir,
                        agent_id,
                        &local_url,
                        &self.block_id,
                    );
                    crate::backend::reactive::registry::write_shared_from_env(
                        agent_id,
                        &local_url,
                        &self.block_id,
                    );
                }
            }
            Err(e) => tracing::warn!(
                block_id = %self.block_id,
                agent_id = %agent_id,
                error = %e,
                "jekt: auto-register failed"
            ),
        }
    }

    /// Seeds `cmd:cwd` in the block meta right after spawn so drag-and-drop works before the shell
    /// emits its first OSC 7 (or for shells without integration).
    pub(super) fn seed_cmd_cwd(&self, cwd: &str, remote: bool) {
        tracing::info!(
            block_id = %self.block_id,
            mstore_present = self.mstore.is_some(),
            event_bus_present = self.event_bus.is_some(),
            "[dnd-debug] pre-seed state after spawn"
        );

        // Seed cmd:cwd in block meta immediately after spawn so drag-and-drop works
        // before the shell emits its first OSC 7 (or for shells without integration).
        // Not for an SSH, WSL or container pane: its cwd is a path on the other side, and
        // this machine's directory would be sent there on the next launch.
        if let Some(store) = self.mstore.as_ref().filter(|_| !remote) {
            let effective_cwd = cwd.to_string(); // spawn_cwd: never empty for a local shell
            tracing::debug!(block_id = %self.block_id, cwd = %effective_cwd, "seeding cmd:cwd");
            if !effective_cwd.is_empty() {
                let oref_str = format!("block:{}", self.block_id);
                let mut meta_update = MetaMapType::new();
                meta_update.insert(
                    super::super::META_KEY_CMD_CWD.to_string(),
                    serde_json::Value::String(effective_cwd),
                );
                // Only set if not already populated — don't clobber a restored session CWD
                match store.must_get::<crate::backend::obj::Block>(&self.block_id) {
                    Ok(block)
                        if obj::meta_get_string(
                            &block.meta,
                            super::super::META_KEY_CMD_CWD,
                            "",
                        )
                        .is_empty() =>
                    {
                        match crate::backend::wcore::object_meta::update_object_meta(
                            store,
                            &oref_str,
                            &meta_update,
                        ) {
                            Ok(()) => {
                                // Re-read updated block and broadcast obj:update so the
                                // frontend Jotai atom refreshes (update_object_meta only writes
                                // to SQLite — it does NOT send a WebSocket event on its own).
                                if let Ok(updated_block) =
                                    store.must_get::<crate::backend::obj::Block>(&self.block_id)
                                {
                                    if let Some(ref event_bus) = self.event_bus {
                                        let update = obj::MuxObjUpdate {
                                            updatetype: "update".into(),
                                            otype: "block".into(),
                                            oid: self.block_id.clone(),
                                            obj: Some(obj::mux_obj_to_value(&updated_block)),
                                        };
                                        event_bus.broadcast_event(
                                            &crate::backend::eventbus::EventBus::typed_ws_event::<
                                                crate::backend::mps_events::MuxObjUpdated,
                                            >(
                                                &update, oref_str.clone()
                                            ),
                                        );
                                        tracing::info!(block_id = %self.block_id, "cmd:cwd seeded and broadcast to frontend");
                                    } else {
                                        tracing::warn!(block_id = %self.block_id, "cmd:cwd written to store but no event_bus to broadcast — frontend won't update");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(block_id = %self.block_id, error = %e, "failed to seed cmd:cwd in store");
                            }
                        }
                    }
                    Ok(_) => {
                        tracing::debug!(block_id = %self.block_id, "cmd:cwd already set, skipping seed");
                    }
                    Err(e) => {
                        tracing::warn!(block_id = %self.block_id, error = %e, "failed to read block for cmd:cwd seed");
                    }
                }
            }
        }
    }
}
