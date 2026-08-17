// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `IdentityAccount` persistence on a successful OAuth handshake.
//!
//! Split out of `identity_handlers.rs` (module-organization pass, see
//! `docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md`) —
//! called from `identity_auth_spawn`'s drain/post-exit success paths
//! (`spawn_auth_cli` + `spawn_auth_cli_pty`, two call sites each).

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::storage::store::{IdentityAccount, SecretRef, Store};
use crate::backend::wps::Broker;

/// Upserts the `IdentityAccount` (`SecretRef::OAuthConfigDir`, status
/// "valid") on a successful OAuth handshake (CLI exited 0 +
/// authCheckCommand confirmed). The actual `agent_identity_link` write
/// happens later, once the agent exists (the launch-flow write-through
/// reconcile) — this function only makes sure the account itself exists
/// and is ready to be linked.
///
/// Publishes `identityaccounts:changed` (the same broad event
/// `account.key.verify`/`upsertidentityaccount` already use) rather
/// than a bundle-scoped event, since there's no bundle id to scope to.
///
/// Returns `None` on any persistence failure (dir never resolved, or
/// the account upsert itself failed) — same "log + skip, session still
/// succeeds" contract as the bundle path, just without a synthetic
/// placeholder to fall back to (direct-account mode has no "ambient"
/// concept to fall back to; the caller surfaces `account_id: None` on
/// the wire and the frontend treats that as "nothing to select").
fn persist_oauth_direct_account(
    wstore: &Arc<Store>,
    identity_store: &Arc<Store>,
    broker: &Arc<Broker>,
    account_id: &str,
    provider_id: &str,
    dir: Option<&str>,
    _session_id: &str,
) -> Option<String> {
    let dir = match dir.filter(|s| !s.is_empty()) {
        Some(d) => d,
        None => {
            tracing::warn!(
                target: "identity",
                account_id,
                provider_id,
                "auth success (direct-account): dir unresolved — skipping account persist"
            );
            return None;
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let account = IdentityAccount {
        id: account_id.to_string(),
        name: format!("{provider_id}-oauth"),
        provider: provider_id.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: dir.to_string() },
        context: serde_json::json!({}),
        // Same rationale as the bundle path: a binding the user JUST
        // OAuth'd into is `valid` by definition.
        status: crate::identity::resolver::oauth_status::VALID.to_string(),
        created_at: now,
        updated_at: now,
    };
    // identity_upsert_with_mirror — reagentx P0 review on PR #2632: this
    // is THE primary OAuth account-creation path (auth.start), so without
    // the mirror write every newly-OAuth'd account had no fallback entry
    // and reproduced the reported bug on its own next channel switch.
    if let Err(e) = wstore.identity_upsert_with_mirror(identity_store, &account) {
        tracing::warn!(
            target: "identity",
            account_id,
            provider_id,
            error = %e,
            "auth success (direct-account): identity_upsert failed"
        );
        return None;
    }
    broker.publish(crate::backend::wps::WaveEvent {
        event: "identityaccounts:changed".to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: None,
    });
    tracing::info!(
        target: "identity",
        account_id,
        provider_id,
        dir,
        "auth success (direct-account): OAuth account persisted"
    );
    Some(account_id.to_string())
}

/// Shared by all 4 OAuth-success call sites (pipes drain/post-exit, PTY
/// drain/post-exit) — persists the account and builds the
/// `(bundle_id, account_id)` pair `AuthSessionManager::finish_success`
/// expects. `bundle_id` is always empty now: bundle mode (`db_identity_bundles`
/// binding) was retired in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md
/// — confirmed unreachable from the frontend (`AuthFlowController` hardcodes
/// `directAccount: true`). `_direct_account`/`_into_bundle_id` stay as
/// parameters so the wire request shape and the 4 call sites don't need
/// touching. `dir` is the account's own isolation dir, resolved once at
/// spawn time by `compute_and_ensure_*_dir` in the `auth.start` handler.
///
/// Guards on `account_id` being non-empty before persisting: `auth.start`
/// only populates a real account_id when `direct_account` is true (via
/// `compute_and_ensure_account_dir`, which always mints/reuses a real id);
/// when `direct_account` is false (the wire default, still reachable by
/// any caller other than the one production frontend path), `account_id`
/// is `""`. Without this guard an empty id would flow into
/// `persist_oauth_direct_account`'s `identity_upsert`, whose
/// `ON CONFLICT(id) DO UPDATE` would silently overwrite/corrupt any prior
/// row that happened to have `id=""`. Reagent P1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn persist_oauth_success(
    wstore: &Arc<Store>,
    identity_store: &Arc<Store>,
    broker: &Arc<Broker>,
    _direct_account: bool,
    account_id: &str,
    _into_bundle_id: Option<&str>,
    provider_id: &str,
    dir: Option<&str>,
    session_id: &str,
) -> (String, Option<String>) {
    if account_id.is_empty() {
        return (String::new(), None);
    }
    let persisted = persist_oauth_direct_account(wstore, identity_store, broker, account_id, provider_id, dir, session_id);
    (String::new(), persisted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_oauth_direct_account_round_trip() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let r = persist_oauth_direct_account(
            &wstore,
            &identity_store,
            &broker,
            "acc-1",
            "claude",
            Some("/some/account/dir"),
            "sess-z",
        );
        assert_eq!(r, Some("acc-1".to_string()));

        let acct = wstore.identity_get("acc-1").unwrap().expect("account row exists");
        assert_eq!(acct.provider, "claude");
        assert_eq!(acct.kind, "oauth");
        assert_eq!(acct.status, "valid");
        match acct.secret_ref {
            SecretRef::OAuthConfigDir { dir } => assert_eq!(dir, "/some/account/dir"),
            other => panic!("expected OAuthConfigDir, got {other:?}"),
        }
    }

    #[test]
    fn persist_oauth_direct_account_returns_none_when_dir_unresolved() {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let r = persist_oauth_direct_account(&wstore, &identity_store, &broker, "acc-1", "claude", None, "sess-z");
        assert!(r.is_none());
        assert!(wstore.identity_get("acc-1").unwrap().is_none(), "nothing persisted when dir is unresolved");
    }

    #[test]
    fn persist_oauth_success_always_routes_direct_account_mode() {
        // Bundle mode was retired in Phase 4c of
        // SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md — persist_oauth_success
        // always persists a direct account now, regardless of the
        // (now-vestigial) direct_account/into_bundle_id parameters.
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let (bundle_id, account_id) = persist_oauth_success(
            &wstore,
            &identity_store,
            &broker,
            true,
            "acc-1",
            None,
            "claude",
            Some("/some/dir"),
            "sess-route",
        );
        assert_eq!(bundle_id, "", "bundle id is always empty now");
        assert_eq!(account_id, Some("acc-1".to_string()));
        assert!(wstore.identity_get("acc-1").unwrap().is_some());
    }

    #[test]
    fn persist_oauth_success_skips_persistence_when_account_id_is_empty() {
        // Reagent P1: `auth.start` sets account_id = "" whenever
        // `direct_account` is false (the wire default) — a caller other
        // than the one production frontend path (which always sends
        // `directAccount: true`) can still reach this. Without the
        // empty-id guard, persist_oauth_direct_account's identity_upsert
        // would silently write/overwrite a db_accounts row with id="".
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let (bundle_id, account_id) = persist_oauth_success(
            &wstore,
            &identity_store,
            &broker,
            false,
            "",
            None,
            "claude",
            Some("/some/dir"),
            "sess-empty",
        );
        assert_eq!(bundle_id, "");
        assert_eq!(account_id, None, "empty account_id must not be persisted");
        assert!(
            wstore.identity_get("").unwrap().is_none(),
            "no row with id=\"\" should ever be written"
        );
    }
}
