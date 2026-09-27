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
    /// Claims this agent's single-live-instance lease, then runs the resume
    /// gate ([`apply_resume_gate_with`](Self::apply_resume_gate_with)) under it.
    ///
    /// The lease is one live instance per agent across every AgentMux instance
    /// on this host (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md Phase 1),
    /// held for the spawned process's lifetime. Claiming it FIRST means the
    /// gate's sweep, relocation and settle (spec §4.3) run for one spawn of an
    /// agent at a time: two controllers of one agent racing those steps kept
    /// opening narrower windows (#3924). The session is only a hint to the
    /// lease — recorded, not a key — so the gate settling the final id
    /// afterwards changes nothing about the claim.
    ///
    /// When leasing is unavailable (no store, or no agent UID) the gate runs
    /// unguarded, as it did before Phase 1.
    pub(super) fn lease_then_gate(
        &self,
        config: &PersistentSpawnConfig,
        gfs: Option<&FileStore>,
        history_session: Option<String>,
    ) -> Result<Option<Arc<crate::backend::agent_admission::HeldAgentLease>>, String> {
        let candidate = if config.resume_flag.is_empty() {
            None
        } else {
            self.inner.lock().unwrap().session_id.clone().or_else(|| history_session.clone())
        };
        let lease = match self.acquire_agent_lease(config, candidate.as_deref()) {
            Ok(lease) => lease,
            // The agent is open in another pane of this process (it holds the
            // lease, whatever session either pane holds), or this conversation
            // is live or still closing there. Report that as the held-elsewhere
            // refusal it is: callers treat only that one as "don't fall back to
            // a fresh session" (`is_held_elsewhere_error`), and the wording is
            // "another pane", not "another instance".
            Err(e) => {
                let uid = config.env_vars.get("AGENTMUX_AGENT_UID").map(|u| u.trim()).filter(|u| !u.is_empty());
                if let Some(other) = uid.and_then(|uid| self.agent_held_by_other_pane(uid)) {
                    return Err(super::held_elsewhere_error(&other, false));
                }
                if let Some((other, closing)) = candidate.as_deref().and_then(|sid| self.session_held_elsewhere(sid)) {
                    return Err(super::held_elsewhere_error(&other, closing));
                }
                return Err(e);
            }
        };
        self.apply_resume_gate_with(config, gfs, history_session);
        Ok(lease)
    }

    /// The one resume gate
    /// (SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md §4.2):
    /// before this spawn passes `--resume`, only the head of the agent's
    /// segment chain may be the session it resumes. A stale id — the
    /// conversation moved on in another channel, build or pane — is
    /// redirected to the head when this config dir reaches it, and cleared
    /// otherwise, so the spawn goes fresh and carries AgentMux's record.
    /// Runs after every path that sets the id (hydrate from config, first
    /// spawn onto rendered history) and under the agent's lease
    /// ([`lease_then_gate`](Self::lease_then_gate)), before the held-elsewhere
    /// check, which then applies to whatever id survives. Never fails the
    /// spawn: anything unknown leaves the id as it was.
    ///
    /// `history_session`: on a first spawn holding no id, the session of the
    /// history the pane renders when `--resume` can't reach it here. It is a
    /// candidate for relocation only (a rebuild onto a new login of the same
    /// identity); any other decision leaves the spawn holding no id, as
    /// before.
    pub(super) fn apply_resume_gate_with(
        &self,
        config: &PersistentSpawnConfig,
        gfs: Option<&FileStore>,
        history_session: Option<String>,
    ) {
        if config.resume_flag.is_empty() {
            return;
        }
        let (held, poisoned) = {
            let inner = self.inner.lock().unwrap();
            (inner.session_id.clone(), inner.resume_poisoned.clone())
        };
        let relocate_only = held.is_none();
        let Some(candidate) = held.or(history_session).filter(|c| poisoned.as_deref() != Some(c.as_str())) else {
            return;
        };
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
        let uid = uid.trim();
        let cwd = config.working_dir.as_str();
        // A copy this agent relocated earlier and whose fork never reported an
        // id (the process died) goes first: left in place, it would look
        // reachable and be resumed in place, a lasting second file with the
        // head's id (spec §3 I4).
        let swept = crate::backend::continuity_relocate::sweep(config_dir, cwd, uid);
        if swept > 0 {
            tracing::info!(target: "continuity", block_id = %self.block_id, swept, "resume gate: removed relocated copies a dead spawn left");
        }
        let head = segs::chain_head(gfs, uid).map(segs::with_legacy_identity);
        let identity = crate::identity::account_email::identity_key_from_oauth_dir("claude", config_dir);
        let input = segs::GateInput {
            candidate: &candidate,
            head: head.as_ref(),
            identity: identity.as_deref(),
            poisoned: poisoned.as_deref(),
            config_dir,
            cwd,
        };
        let decision = segs::resume_gate(
            &input,
            |sid| provider_file_size(config_dir, cwd, sid),
            |dir, sid| {
                crate::backend::continuity_relocate::source_file(dir, cwd, sid)
                    .is_some_and(|p| crate::backend::continuity_relocate::is_resumable_file(&p))
            },
        );
        let refuse = |head: &str, why: &str| {
            tracing::info!(
                target: "continuity",
                block_id = %self.block_id,
                candidate = %candidate,
                head = %head,
                why,
                "resume gate: the conversation is in a session this spawn can't resume; starting fresh with the record"
            );
            (None, None, false)
        };
        if relocate_only && !matches!(decision, segs::ResumeGate::Relocate { .. }) {
            return;
        }
        let (next, fork_copy, fork) = match decision {
            segs::ResumeGate::Allow => return,
            segs::ResumeGate::Fork { head } => {
                tracing::info!(
                    target: "continuity",
                    block_id = %self.block_id,
                    candidate = %candidate,
                    head = %head,
                    "resume gate: the session grew after AgentMux last saw it (continued outside AgentMux); forking it"
                );
                (Some(head), None, true)
            }
            segs::ResumeGate::Redirect { head } => {
                tracing::info!(
                    target: "continuity",
                    block_id = %self.block_id,
                    candidate = %candidate,
                    head = %head,
                    "resume gate: the conversation moved on; resuming the chain head instead"
                );
                (Some(head), None, false)
            }
            segs::ResumeGate::Refuse { head } => refuse(&head, "out of reach, or another identity's"),
            segs::ResumeGate::Relocate { head, from_config_dir } => {
                let placed = crate::backend::continuity_relocate::source_file(&from_config_dir, cwd, &head)
                    .ok_or_else(|| "source gone".to_string())
                    .and_then(|src| crate::backend::continuity_relocate::relocate(&src, config_dir, cwd, &head, uid));
                match placed {
                    Ok(copy) => {
                        tracing::info!(
                            target: "continuity",
                            block_id = %self.block_id,
                            candidate = %candidate,
                            head = %head,
                            from = %from_config_dir,
                            "resume gate: the conversation is under another login of the same identity; relocated it to fork from"
                        );
                        (Some(head), Some(copy), true)
                    }
                    Err(e) => refuse(&head, &format!("relocation failed: {e}")),
                }
            }
        };
        {
            let mut inner = self.inner.lock().unwrap();
            // Only replace what the gate judged: another path may have moved
            // the id meanwhile.
            let judged = if relocate_only { None } else { Some(candidate.as_str()) };
            if inner.session_id.as_deref() != judged {
                drop(inner);
                if let Some(copy) = fork_copy {
                    crate::backend::continuity_relocate::remove(&copy);
                }
                return;
            }
            inner.session_id = next.clone();
            inner.fork_copy = fork_copy;
            inner.fork_next = fork;
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_segment_start(
        &self,
        agent_uid: &str,
        config: &PersistentSpawnConfig,
        attempted_resume_sid: Option<&str>,
        forked_from: Option<&str>,
        relocated: bool,
        carries_packet: bool,
        lease_epoch: Option<u64>,
    ) -> SegmentRef {
        let gfs = crate::backend::agent_session::global_transcript_store()?;
        let start = self.segment_start(agent_uid, config, attempted_resume_sid, forked_from, relocated, carries_packet, lease_epoch);
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

    /// The `Start` event [`record_segment_start`](Self::record_segment_start)
    /// writes. A relocated fork records no session until it reports its own:
    /// the attempted id names the copy, which is swept if the process dies
    /// first, so as the head it would send the next spawn to `--resume` a
    /// file that is gone instead of relocating again (spec §4.3).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn segment_start(
        &self,
        agent_uid: &str,
        config: &PersistentSpawnConfig,
        attempted_resume_sid: Option<&str>,
        forked_from: Option<&str>,
        relocated: bool,
        carries_packet: bool,
        lease_epoch: Option<u64>,
    ) -> segs::Start {
        let meta = self
            .mstore
            .as_deref()
            .and_then(|s| s.get::<crate::backend::obj::Block>(&self.block_id).ok().flatten())
            .map(|b| b.meta)
            .unwrap_or_default();
        let definition_id = Some(crate::backend::obj::meta_get_string(&meta, "agentId", "")).filter(|d| !d.is_empty());
        let provider = crate::backend::obj::meta_get_string(&meta, "agentProvider", "");
        let zone = crate::backend::agent_session::agent_zone_for_block_meta(&meta);
        let config_dir = spawn_config_dir(&provider, &config.env_vars);
        let identity_key = config_dir
            .as_deref()
            .and_then(|dir| crate::identity::account_email::identity_key_from_oauth_dir(&provider, dir));
        segs::Start {
            segment_id: String::new(),
            agent_uid: agent_uid.to_string(),
            definition_id,
            config_dir,
            identity_key,
            forked_from: forked_from.map(str::to_string),
            provider,
            provider_session_id: attempted_resume_sid.filter(|_| !relocated).map(str::to_string),
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
/// Size of the Claude session file `sid` for `cwd` under `config_dir`;
/// `None` when it isn't there.
fn provider_file_size(config_dir: &str, cwd: &str, sid: &str) -> Option<u64> {
    let path = crate::backend::session_backfill::session_file_path(config_dir, cwd, sid)?;
    std::fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| m.len())
}

/// The provider session file's size for segment `id`, from what its own
/// record says about where it ran (spec §4.4). Claude only.
fn provider_bytes_for_segment(gfs: &FileStore, uid: &str, id: &str) -> Option<i64> {
    let segment = segs::segments(gfs, uid).into_iter().find(|s| s.start.segment_id == id)?;
    let start = segment.start;
    if !matches!(start.provider.as_str(), "claude" | "claude-code") {
        return None;
    }
    let size = provider_file_size(start.config_dir.as_deref()?, &start.cwd, start.provider_session_id.as_deref()?)?;
    i64::try_from(size).ok()
}

pub(super) fn record_segment_end(segment: &SegmentRef, reason: segs::EndReason, zone: Option<&str>) {
    let (Some((uid, id)), Some(gfs)) = (segment, crate::backend::agent_session::global_transcript_store()) else {
        return;
    };
    let provider_bytes = provider_bytes_for_segment(gfs, uid, id);
    if let Err(e) = segs::record_end_with_provider_bytes(gfs, uid, id, reason, zone.and_then(zone_size), provider_bytes, now_ms()) {
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

/// The outcome of a relocated resume (spec §4.3): the fork reports its own
/// id, which `persistent_resume` reads as the CLI starting fresh. Carrying
/// the whole conversation into a new id is the resume succeeding.
pub(super) fn forked_outcome(
    outcome: persistent_resume::SessionOutcome,
    forked_from: Option<&str>,
    attempted_sid: &str,
    actual_sid: Option<&str>,
) -> persistent_resume::SessionOutcome {
    let forked = forked_from == Some(attempted_sid) && actual_sid.is_some_and(|a| a != attempted_sid);
    if forked {
        persistent_resume::SessionOutcome::Resumed
    } else {
        outcome
    }
}

/// A relocated resume the CLI kept under the attempted id, resumed in place
/// rather than forked. Capture skips an id the controller already holds, so
/// this is settled on its own, once a result frame proves the resume worked
/// (an earlier frame echoes the attempted id even when the resume goes on to
/// fail). Left unsettled, the segment would hold no session (a relocated
/// Start records none) and the next spawn would sweep the live copy.
/// `holds_current`: the reader's generation is still current and holds the
/// id; a superseded one's copy path may already be the replacement's.
pub(super) fn kept_relocated_id(
    adopted: bool,
    confirmed: bool,
    holds_current: bool,
    has_copy: bool,
    forked_from: Option<&str>,
    captured: &str,
) -> bool {
    !adopted && confirmed && holds_current && has_copy && forked_from == Some(captured)
}

/// Once a relocated resume reports its session: a fork (a new id) never
/// wrote the copy, so it goes. The same id back means the CLI resumed the
/// copy in place instead of forking; it is live now, so only its marker
/// goes, and the duplicate is logged (spec §3 I4). Returns whether the copy
/// was still this spawn's: a replacement's sweep may have taken it and
/// placed its own at the same path, which is then left alone.
pub(super) fn settle_relocated_copy(
    block_id: &str,
    copy: &crate::backend::continuity_relocate::Relocated,
    forked_from: Option<&str>,
    captured: &str,
) -> bool {
    if forked_from == Some(captured) {
        let owned = crate::backend::continuity_relocate::unmark(copy);
        if owned {
            tracing::warn!(
                target: "continuity",
                block_id,
                session_id = captured,
                "relocated resume did not fork; keeping the copy as the live session"
            );
        }
        owned
    } else {
        crate::backend::continuity_relocate::remove(copy)
    }
}
