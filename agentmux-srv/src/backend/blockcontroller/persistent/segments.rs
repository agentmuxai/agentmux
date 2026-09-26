// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! This controller's side of the segment index
//! (`backend::continuity_segments`): each spawn is one segment of its
//! agent's conversation. Best-effort throughout: a failed write is logged and
//! never affects the process.

use super::*;
use crate::backend::continuity_segments as segs;

/// The segment a spawned process belongs to: `(agent_uid, segment_id)`.
pub(super) type SegmentRef = Option<(String, String)>;

/// The config dir the spawn gives its provider: the provider's own
/// `auth_config_dir_env_var` (Gemini's is `GEMINI_CLI_HOME`, not the
/// `GEMINI_CONFIG_DIR` a hand-kept list here once named), else the first
/// known provider's variable the env carries.
fn spawn_config_dir(provider: &str, env: &std::collections::HashMap<String, String>) -> Option<String> {
    if let Some(p) = crate::backend::providers::get_provider(provider) {
        if let Some(v) = env.get(p.auth_config_dir_env_var) {
            return Some(v.clone());
        }
    }
    crate::backend::providers::all_providers().find_map(|p| env.get(p.auth_config_dir_env_var).cloned())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Size of `output` in the global zone, read from the database rather than
/// this process's cache, so another channel's appends count.
fn zone_size(zone: &str) -> Option<i64> {
    let gfs = crate::backend::agent_session::global_transcript_store()?;
    gfs.line_state(zone, crate::backend::agent_session::OUTPUT_FILE).ok()?.map(|s| s.size)
}

impl PersistentSubprocessController {
    /// The one resume gate
    /// (SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md §4.2):
    /// before this spawn passes `--resume`, only the head of the agent's
    /// segment chain may be the session it resumes. A stale id — the
    /// conversation moved on in another channel, build or pane — is
    /// redirected to the head when this config dir reaches it, and cleared
    /// otherwise, so the spawn goes fresh and carries AgentMux's record.
    /// Runs after every path that sets the id (hydrate from config, first
    /// spawn onto rendered history), before the held-elsewhere check and the
    /// lease, which then apply to whatever id survives. Never fails the spawn:
    /// anything unknown leaves the id as it was.
    pub(super) fn apply_resume_gate(&self, config: &PersistentSpawnConfig) {
        let gfs = crate::backend::agent_session::global_transcript_store();
        self.apply_resume_gate_with(config, gfs.map(|g| &**g));
    }

    pub(super) fn apply_resume_gate_with(&self, config: &PersistentSpawnConfig, gfs: Option<&FileStore>) {
        if config.resume_flag.is_empty() {
            return;
        }
        let (candidate, poisoned) = {
            let inner = self.inner.lock().unwrap();
            (inner.session_id.clone(), inner.resume_poisoned.clone())
        };
        let Some(candidate) = candidate else { return };
        // Reachability is a file check under the provider's config dir; only
        // Claude's is known. Without one the head can't be checked, so the
        // gate stays out of the way.
        let Some(config_dir) = config.env_vars.get("CLAUDE_CONFIG_DIR") else { return };
        // A non-Claude spawn can still inherit that variable; its sessions
        // aren't under it, so checking there would refuse every resume.
        let provider = self
            .mstore
            .as_deref()
            .and_then(|s| s.get::<crate::backend::obj::Block>(&self.block_id).ok().flatten())
            .map(|b| crate::backend::obj::meta_get_string(&b.meta, "agentProvider", ""))
            .unwrap_or_default();
        if !matches!(provider.as_str(), "" | "claude" | "claude-code") {
            return;
        }
        let (Some(gfs), Some(uid)) = (gfs, config.env_vars.get("AGENTMUX_AGENT_UID").filter(|u| !u.trim().is_empty()))
        else {
            return;
        };
        let head = segs::chain_head(gfs, uid.trim());
        let decision = segs::resume_gate(&candidate, head.as_deref(), poisoned.as_deref(), |h| {
            crate::backend::session_backfill::session_is_reachable(config_dir, &config.working_dir, h)
        });
        let next = match decision {
            segs::ResumeGate::Allow => return,
            segs::ResumeGate::Redirect { head } => {
                tracing::info!(
                    target: "continuity",
                    block_id = %self.block_id,
                    candidate = %candidate,
                    head = %head,
                    "resume gate: the conversation moved on; resuming the chain head instead"
                );
                Some(head)
            }
            segs::ResumeGate::Refuse { head } => {
                tracing::info!(
                    target: "continuity",
                    block_id = %self.block_id,
                    candidate = %candidate,
                    head = %head,
                    "resume gate: the conversation moved on to a session this config dir can't reach; starting fresh with the record"
                );
                None
            }
        };
        {
            let mut inner = self.inner.lock().unwrap();
            // Only replace what the gate judged: another path may have moved
            // the id meanwhile.
            if inner.session_id.as_deref() != Some(candidate.as_str()) {
                return;
            }
            inner.session_id = next.clone();
        }
        core::persist_session_id(&self.block_id, next.as_deref().unwrap_or(""), &self.mstore, &self.event_bus);
    }

    /// Reconcile the agent's memory record with the folder this spawn is
    /// about to use, before the provider starts, within
    /// [`memory_reconcile::RECONCILE_BUDGET`](crate::backend::memory_reconcile::RECONCILE_BUDGET)
    /// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.3). Never fails the
    /// spawn. Off for an agent with `agent:memoryrecord` = `off`.
    pub(super) fn reconcile_memory_before_spawn(&self, config: &PersistentSpawnConfig) {
        let (Some(gfs), Some(mstore)) = (crate::backend::agent_session::global_transcript_store(), self.mstore.as_deref()) else {
            return;
        };
        let Some(uid) = config.env_vars.get("AGENTMUX_AGENT_UID").filter(|u| !u.is_empty()) else {
            return;
        };
        let meta = mstore
            .get::<crate::backend::obj::Block>(&self.block_id)
            .ok()
            .flatten()
            .map(|b| b.meta)
            .unwrap_or_default();
        if crate::backend::obj::meta_get_string(&meta, "agent:memoryrecord", "") == "off" {
            return;
        }
        // The same agent live in another pane (a second pane starting fresh
        // beside it) writes the same folder while the pass would run: leave
        // it to that agent's next spawn.
        let live_elsewhere = super::super::get_all_controllers().into_iter().any(|(block_id, ctrl)| {
            block_id != self.block_id
                && ctrl.as_any().downcast_ref::<PersistentSubprocessController>().is_some_and(|other| {
                    other.stable_agent_uid.lock().unwrap().as_deref() == Some(uid.as_str())
                        && other.inner.lock().unwrap().current_pid.is_some()
                })
        });
        if live_elsewhere {
            tracing::info!(block_id = %self.block_id, uid, "memory reconcile skipped: the agent is live in another pane");
            return;
        }
        let provider = crate::backend::obj::meta_get_string(&meta, "agentProvider", "");
        let reconcile = || {
            crate::backend::memory_reconcile::reconcile_before_spawn(
                gfs,
                mstore,
                &crate::backend::memory_reconcile::SpawnMemory {
                    uid,
                    provider: &provider,
                    config_dir: config.env_vars.get("CLAUDE_CONFIG_DIR").map(String::as_str),
                    cwd: &config.working_dir,
                    overridden: crate::backend::memory_reconcile::spawn_overrides_memory_dir(&config.env_vars, &config.cli_args),
                    history_stores: None,
                },
                crate::backend::memory_reconcile::RECONCILE_BUDGET,
            )
        };
        // Up to a second of blocking file and SQLite I/O: on the async
        // callers' runtime (run_agent_turn, agent.send) hand the worker's
        // other tasks off first. block_in_place panics on a current-thread
        // runtime, where there is no other worker to hand them to.
        let on_multi_thread = tokio::runtime::Handle::try_current()
            .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
        let report = if on_multi_thread { tokio::task::block_in_place(reconcile) } else { reconcile() };
        if report.deferred {
            tracing::info!(block_id = %self.block_id, uid, "memory reconcile deferred: over budget");
        } else if report != Default::default() {
            tracing::info!(block_id = %self.block_id, uid, ?report, "memory reconciled before spawn");
        }
    }

    /// Records the start of the segment this spawn begins. `None` when the
    /// spawn carries no agent UID (quick-launch panes, a continuation that
    /// resumes before its row exists) or the write fails.
    pub(super) fn record_segment_start(
        &self,
        agent_uid: &str,
        config: &PersistentSpawnConfig,
        attempted_resume_sid: Option<&str>,
        carries_packet: bool,
        lease_epoch: Option<u64>,
    ) -> SegmentRef {
        let gfs = crate::backend::agent_session::global_transcript_store()?;
        let meta = self
            .mstore
            .as_deref()
            .and_then(|s| s.get::<crate::backend::obj::Block>(&self.block_id).ok().flatten())
            .map(|b| b.meta)
            .unwrap_or_default();
        let definition_id = Some(crate::backend::obj::meta_get_string(&meta, "agentId", "")).filter(|d| !d.is_empty());
        let provider = crate::backend::obj::meta_get_string(&meta, "agentProvider", "");
        let zone = crate::backend::agent_session::agent_zone_for_block_meta(&meta);
        let start = segs::Start {
            segment_id: String::new(),
            agent_uid: agent_uid.to_string(),
            definition_id,
            config_dir: spawn_config_dir(&provider, &config.env_vars),
            provider,
            provider_session_id: attempted_resume_sid.map(str::to_string),
            cwd: config.working_dir.clone(),
            channel: crate::backend::reactive::registry::local_channel_id(),
            agentmux_version: crate::backend::base::get_version().to_string(),
            block_id: self.block_id.clone(),
            byte_start: zone.as_deref().and_then(zone_size),
            zone,
            started_at_ms: now_ms(),
            continuity_rung: segs::rung_for_spawn(attempted_resume_sid.is_some(), carries_packet),
            predecessor_segment_id: None,
            lease_epoch,
        };
        let rung = start.continuity_rung;
        match segs::record_start(gfs, start) {
            Ok(id) => {
                tracing::info!(block_id = %self.block_id, segment_id = %id, rung = ?rung, "continuity: segment started");
                Some((agent_uid.to_string(), id))
            }
            Err(e) => {
                tracing::warn!(block_id = %self.block_id, error = %e, "continuity: segment start not recorded");
                None
            }
        }
    }
}

/// A fresh process reported its provider session id.
pub(super) fn record_segment_session(segment: &SegmentRef, provider_session_id: &str) {
    let (Some((uid, id)), Some(gfs)) = (segment, crate::backend::agent_session::global_transcript_store()) else {
        return;
    };
    if let Err(e) = segs::record_session(gfs, uid, id, provider_session_id, now_ms()) {
        tracing::warn!(segment_id = %id, error = %e, "continuity: segment session not recorded");
    }
}

/// The segment's process went away. `zone` is its global transcript zone.
pub(super) fn record_segment_end(segment: &SegmentRef, reason: segs::EndReason, zone: Option<&str>) {
    let (Some((uid, id)), Some(gfs)) = (segment, crate::backend::agent_session::global_transcript_store()) else {
        return;
    };
    if let Err(e) = segs::record_end(gfs, uid, id, reason, zone.and_then(zone_size), now_ms()) {
        tracing::warn!(segment_id = %id, error = %e, "continuity: segment end not recorded");
    } else {
        tracing::info!(segment_id = %id, reason = ?reason, "continuity: segment ended");
    }
}

#[cfg(test)]
mod config_dir_tests {
    use super::spawn_config_dir;

    fn env(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn each_provider_records_its_own_config_dir() {
        assert_eq!(spawn_config_dir("claude", &env(&[("CLAUDE_CONFIG_DIR", "/c")])).as_deref(), Some("/c"));
        assert_eq!(spawn_config_dir("gemini", &env(&[("GEMINI_CLI_HOME", "/g")])).as_deref(), Some("/g"));
        assert_eq!(spawn_config_dir("qwen", &env(&[("QWEN_HOME", "/q")])).as_deref(), Some("/q"));
        assert_eq!(spawn_config_dir("kimi", &env(&[("KIMI_SHARE_DIR", "/k")])).as_deref(), Some("/k"));
    }

    #[test]
    fn the_providers_own_variable_wins_over_another_in_the_env() {
        let e = env(&[("CLAUDE_CONFIG_DIR", "/c"), ("CODEX_HOME", "/x")]);
        assert_eq!(spawn_config_dir("codex", &e).as_deref(), Some("/x"));
        assert_eq!(spawn_config_dir("", &env(&[])), None);
    }
}
