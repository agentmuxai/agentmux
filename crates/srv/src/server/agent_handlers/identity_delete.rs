// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Deleting an identity account, whoever asks: the window's `deleteidentityaccount` and an
//! agent's `Google disconnect` run this one function, so the grant is revoked at the provider,
//! the secrets, links, Google grants and log are removed, and the windows and the agents that
//! held it are told, the same way either time.

use std::sync::Arc;

use serde_json::json;

use super::identity::{revoke_keeps_account, DeleteIdentityAccountResult};
use crate::backend::mps::Broker;
use crate::backend::storage::store::{IdentityAccount, Store};

/// Revokes the account's grant at its provider, removes its credential material and rows, and
/// announces it. With `require_revoke` a revoke that failed keeps the account (`deleted: false`),
/// so the delete can be retried while the grant can still be ended.
pub(crate) async fn delete_identity_account(
    mstore: &Arc<Store>,
    identity_store: &Arc<Store>,
    broker: &Arc<Broker>,
    id: &str,
    require_revoke: bool,
) -> Result<DeleteIdentityAccountResult, String> {
    // The credential material goes BEFORE the row (layer 1 of
    // ANALYSIS_ACCOUNT_DELETE_AUTH_LIFECYCLE_GAP_2026_07_14.md §4), best-effort: cleanup trouble
    // never blocks the delete. The provider is read first so the log lines can carry it.
    let acct = mstore.identity_get(id).ok().flatten();
    let provider = acct.as_ref().map(|a| a.provider.clone()).unwrap_or_default();
    let revoke_report = revoke_at_provider(mstore, acct.as_ref()).await;
    if revoke_keeps_account(&revoke_report, require_revoke) {
        return Ok(DeleteIdentityAccountResult {
            deleted: false,
            affected_agents: Vec::new(),
            cleanup: serde_json::Value::Null,
            revoke: revoke_report,
        });
    }
    let cleanup_report = remove_secrets(acct).await;
    let outcome = mstore
        .identity_delete(id)
        .map_err(|e| format!("deleteidentityaccount: {e}"))?;
    let deleted = outcome.deleted;
    // Links live in identity_store, not the store holding the account row, so the cascade above
    // finds none (SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md): remove the real rows here and merge the
    // two outcomes, or every delete would orphan its links.
    let (links_cascaded_from_identity_store, mut affected_agents) = identity_store
        .agent_identity_unlink_by_account(id)
        .map_err(|e| format!("deleteidentityaccount: identity_store cleanup: {e}"))?;
    let links_cascaded = outcome.links_cascaded + links_cascaded_from_identity_store;
    affected_agents.extend(outcome.affected_agents);
    affected_agents.sort();
    affected_agents.dedup();
    // Also remove the account's MIRROR row (identity_upsert_with_mirror's
    // counterpart on delete) — without this, a stale copy of a
    // deleted account survives in identity_store, and
    // resolve_account's fallback would incorrectly resolve it
    // again on the next spawn once id_store's own row is gone.
    // Best-effort: a mirror cleanup failure must not block the
    // real delete, which already succeeded above.
    if let Err(e) = identity_store.identity_delete(id) {
        tracing::warn!(
            account_id = %id,
            error = %e,
            "deleteidentityaccount: identity_store mirror row cleanup failed (non-fatal)"
        );
    }
    // A Google account's grants and call log go with it (connectors spec D4, D7).
    if let Err(e) = identity_store.google_account_forget(id) {
        tracing::warn!(account_id = %id, error = %e, "deleteidentityaccount: Google grants cleanup failed (non-fatal)");
    }
    if links_cascaded > 0 {
        // info!, not debug!: the production filter is
        // "agentmuxsrv=info,info" (reagent P1, PR #2143).
        // "identity.delete:" is `muxlog auth` vocabulary.
        tracing::info!(
            account_id = %id,
            provider = %provider,
            links = links_cascaded,
            "identity.delete: links cascaded"
        );
    }
    tracing::info!(
        account_id = %id,
        provider = %provider,
        deleted,
        "identity.delete: account removed (deleteidentityaccount)"
    );
    if deleted {
        tell_windows_and_agents(broker, id, &provider, &affected_agents);
    }
    Ok(DeleteIdentityAccountResult {
        deleted,
        // Layer 4: delete-time disclosure (spec §4).
        affected_agents,
        // Null when there was no account row to clean up after.
        cleanup: cleanup_report,
        revoke: revoke_report,
    })
}

/// An OAuth grant is revoked at the provider first, while the token is still in the keychain;
/// otherwise it would live on there (spec §7 "Disconnect"). Null when there's none.
async fn revoke_at_provider(mstore: &Arc<Store>, acct: Option<&IdentityAccount>) -> serde_json::Value {
    let Some(acct) = acct else { return serde_json::Value::Null };
    use crate::identity::oauth_tokens::{revoke_before_delete, KeychainSecrets, LiveHttp};
    // A GitHub grant made through the broker is ended there.
    let report = match crate::identity::github_connect::revoke_before_delete(mstore, &KeychainSecrets, acct).await {
        Some(report) => Some(report),
        None => revoke_before_delete(&LiveHttp, &KeychainSecrets, acct).await,
    };
    report.unwrap_or(serde_json::Value::Null)
}

/// What the cleanup did, returned to Connectors so a login that couldn't be removed is reported,
/// not hidden (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md H3). Null when there
/// was no account row to clean up after.
async fn remove_secrets(acct: Option<IdentityAccount>) -> serde_json::Value {
    let Some(acct) = acct else { return serde_json::Value::Null };
    // Containment root for OAuth dirs: only paths inside ~/.agentmux/shared/identities/ are ever
    // removed (the legacy ~/.claude migration dir is the user's global CLI login: never ours to delete).
    let identities_root = agentmux_common::DataPaths::from_env().map(|p| p.identities_dir());
    match tokio::task::spawn_blocking(move || crate::identity::cleanup::cleanup_account_secrets(&acct, identities_root.as_deref())).await {
        Ok(outcome) => outcome.report(),
        Err(e) => json!({ "outcome": "failed", "detail": e.to_string() }),
    }
}

/// The windows read the accounts again, and each agent that held the account hears its credentials
/// were revoked.
fn tell_windows_and_agents(broker: &Arc<Broker>, id: &str, provider: &str, affected_agents: &[String]) {
    broker.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
    // Layer 2 — running-agent reconciliation (spec SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md
    // §3): the cascaded links name agents that may have a LIVE process still holding this account's
    // tokens. We surface, not hard-kill — a per-agent revocation event drives the pane chip;
    // enforcement lands at the next spawn (layer 3). Also poke the per-agent link-changed event so
    // link tables refresh.
    if affected_agents.is_empty() {
        return;
    }
    // info!, not debug!: the production filter is "agentmuxsrv=info,info". "identity.delete:" is
    // `muxlog auth` vocabulary.
    tracing::info!(
        account_id = %id,
        provider = %provider,
        count = affected_agents.len(),
        agent_ids = %affected_agents.join(","),
        "identity.delete: running agent(s) affected"
    );
    for agent_id in affected_agents {
        broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(agent_id, &(), vec![], 0);
        let revoked = crate::backend::mps_events::CredentialsRevokedEvent {
            credentials_revoked: true,
            provider: provider.to_string(),
            account_id: Some(id.to_string()),
        };
        broker.publish_event_for::<crate::backend::mps_events::AgentCredentialsRevoked>(agent_id, &revoked, vec![], 0);
    }
}
