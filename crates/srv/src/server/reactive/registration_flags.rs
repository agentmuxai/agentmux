//! `reactive.registration_flags`: which registered agents have a registration problem, for the
//! Swarm's row badge. Shares its two lookups with `reactive.registrations`.

use super::{is_self_registration, AppState, MismatchAuditSummary, RemoteRegistrationEntry};
use crate::backend::reactive::registry as agent_registry;

/// Every OTHER live instance/channel on this host claiming `agent_id`, freshest first.
///
/// Every agent (this instance's own included) writes itself into the same host-global shared
/// registry, so this instance's own entry is filtered out or the "registered elsewhere too" badge
/// would fire on every healthy agent. A crashed sibling's entry lingers until the next
/// startup-only `cleanup_stale_shared` sweep, so the pid is checked live here (same-host by
/// construction, per `pid_alive`).
pub(super) fn remote_claims(state: &AppState, agent_id: &str) -> Vec<RemoteRegistrationEntry> {
    crate::registry::resolve_shared_reactive_dir()
        .map(|shared_dir| {
            agent_registry::lookup_all_shared(&shared_dir, agent_id)
                .into_iter()
                .filter(|e| !is_self_registration(&e.local_url, &state.local_web_url))
                .filter(|e| agent_registry::pid_alive(e.pid))
                .map(|e| RemoteRegistrationEntry {
                    channel: e.channel,
                    pid: e.pid,
                    updated_at: e.updated_at,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// The newest refusal by the recipient identity guard in `audit` (newest first) for `agent_id` or
/// for `block_id`. A message addressed by uid or alias is logged under that address, not the
/// display name, but always with the block it resolved to.
pub(super) fn recent_mismatch_for(
    audit: &[crate::backend::reactive::AuditLogEntry],
    agent_id: &str,
    block_id: Option<&str>,
) -> Option<MismatchAuditSummary> {
    let target_lower = agent_id.to_lowercase();
    audit
        .iter()
        .find(|e| {
            e.outcome.as_deref() == Some("identity-mismatch")
                && (e.target_agent.to_lowercase() == target_lower
                    || block_id.is_some_and(|b| e.block_id == b))
        })
        .map(|e| MismatchAuditSummary {
            timestamp: e.timestamp,
            block_id: e.block_id.clone(),
            error_message: e.error_message.clone(),
        })
}

/// One registered agent that has a registration problem: another instance claims its id, or a
/// message to it was recently refused by the identity guard.
#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub(super) struct RegistrationFlag {
    agent_id: String,
    block_id: String,
    remote: Vec<RemoteRegistrationEntry>,
    recent_mismatch: Option<MismatchAuditSummary>,
}

/// `reactive.registration_flags`: the agents with a registration problem, for the Swarm's row badge.
/// A healthy agent is not listed, so one call every few seconds covers every row.
#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub(super) struct ReactiveRegistrationFlagsResult {
    flags: Vec<RegistrationFlag>,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub(super) struct ReactiveRegistrationFlagsParams {}

fn flags(state: &AppState) -> ReactiveRegistrationFlagsResult {
    let audit = state.reactive_handler.get_audit_log(100);
    let flags = state
        .reactive_handler
        .list_agents()
        .into_iter()
        .filter_map(|a| {
            let remote = remote_claims(state, &a.agent_id);
            let recent_mismatch = recent_mismatch_for(&audit, &a.agent_id, Some(&a.block_id));
            (!remote.is_empty() || recent_mismatch.is_some()).then(|| RegistrationFlag {
                agent_id: a.agent_id,
                block_id: a.block_id,
                remote,
                recent_mismatch,
            })
        })
        .collect();
    ReactiveRegistrationFlagsResult { flags }
}

pub(super) fn register(
    engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>,
    state: AppState,
) {
    engine.register_typed(
        "reactive.registration_flags",
        move |_: ReactiveRegistrationFlagsParams, _ctx| {
            let state = state.clone();
            async move { Ok(flags(&state)) }
        },
    );
}
