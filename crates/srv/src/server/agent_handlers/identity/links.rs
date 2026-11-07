// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent ↔ identity junction RPCs (`linkagentidentity`, `unlinkagentidentity`), moved out of
//! `identity::register` unchanged as named functions.

use std::sync::Arc;

use crate::backend::eventbus::EventBus;
use crate::backend::mps::Broker;
use crate::backend::rpc_types::{CommandLinkAgentIdentityData, CommandUnlinkAgentIdentityData, UnlinkAgentIdentityResult};
use crate::backend::storage::store::Store;

/// The stores and buses a link change touches: the global identity store holds the links, the pane store the
/// panes whose failure a rebind clears.
pub(super) struct LinkDeps {
    pub identity_store: Arc<Store>,
    pub broker: Arc<Broker>,
    pub panes: Arc<Store>,
    pub event_bus: Arc<EventBus>,
}

pub(super) fn link_agent_identity(deps: &LinkDeps, cmd: &CommandLinkAgentIdentityData) -> Result<(), String> {
    let LinkDeps { identity_store, broker, panes, event_bus } = deps;
    // Read before the link changes: an old pane counts against the agent's current links
    // (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.3).
    let failed = crate::server::account_episodes::failed_panes(panes, identity_store, &cmd.agent_id, &cmd.provider);
    identity_store
        .agent_identity_link(&cmd.agent_id, &cmd.account_id, &cmd.provider)
        .map_err(|e| format!("linkagentidentity: {e}"))?;
    crate::server::account_episodes::after_rebind(panes, event_bus, failed, &cmd.account_id);
    broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(&cmd.agent_id, &(), vec![], 0);
    // See app_api/identity.rs's identical call: watch_agent's one-shot config-dir resolution can race ahead of
    // this exact binding.
    if let Some(watcher) = crate::backend::subagent_watcher::global() {
        watcher.recheck_all_watched_agents();
    }
    Ok(())
}

pub(super) fn unlink_agent_identity(deps: &LinkDeps, cmd: &CommandUnlinkAgentIdentityData) -> Result<UnlinkAgentIdentityResult, String> {
    let LinkDeps { identity_store, broker, panes, event_bus } = deps;
    let failed = crate::server::account_episodes::failed_panes(panes, identity_store, &cmd.agent_id, &cmd.provider);
    let removed = identity_store
        .agent_identity_unlink(&cmd.agent_id, &cmd.provider)
        .map_err(|e| format!("unlinkagentidentity: {e}"))?;
    // info!, not debug!: the production filter is "agentmuxsrv=info,info". "identity.unlink:" is `muxlog auth`
    // vocabulary.
    tracing::info!(
        agent_id = %cmd.agent_id,
        provider = %cmd.provider,
        unlinked = removed,
        "identity.unlink: agent-identity link removed (unlinkagentidentity)"
    );
    if removed {
        // A silent unlink is the first half of a rebind (alias rows), the link that follows clears; a real one
        // leaves the agent without an account, so its old limit goes.
        if !cmd.silent {
            crate::server::account_episodes::after_rebind(panes, event_bus, failed, "");
        }
        broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(&cmd.agent_id, &(), vec![], 0);
        // Spec §3 names unlink alongside delete: a live process for this agent still holds the unlinked
        // account's tokens until restarted, so the same disclosure chip. Skipped for `silent` unlinks (alias
        // migration, not a real unbind: see CommandUnlinkAgentIdentityData).
        if !cmd.silent {
            let revoked = crate::backend::mps_events::CredentialsRevokedEvent {
                credentials_revoked: true,
                provider: cmd.provider.clone(),
                account_id: None,
            };
            broker.publish_event_for::<crate::backend::mps_events::AgentCredentialsRevoked>(&cmd.agent_id, &revoked, vec![], 0);
        }
    }
    Ok(UnlinkAgentIdentityResult { unlinked: removed })
}
