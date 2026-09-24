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
use crate::backend::mps::Broker;

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
    mstore: &Arc<Store>,
    identity_store: &Arc<Store>,
    broker: &Arc<Broker>,
    account_id: &str,
    provider_id: &str,
    dir: Option<&str>,
    _session_id: &str,
    email: Option<&str>,
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
        // `display_name` is deliberately left for the USER to set — see
        // SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md §2. The login email is
        // provider metadata and belongs in `context`, so naming an account
        // "work" later cannot clobber it and vice versa.
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: dir.to_string() },
        // The key is OMITTED, not set empty, when the provider reported no
        // email — so "this provider does not surface one" stays
        // distinguishable from "logged in as a blank address" downstream.
        // Falls back to the email the CLI recorded in the account's own
        // config dir (Claude's login transcript prints none) — §4.
        context: match email
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .or_else(|| crate::identity::account_email::email_from_oauth_dir(provider_id, dir))
        {
            Some(e) => serde_json::json!({ "email": e }),
            None => serde_json::json!({}),
        },
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
    if let Err(e) = mstore.identity_upsert_with_mirror(identity_store, &account) {
        tracing::warn!(
            target: "identity",
            account_id,
            provider_id,
            error = %e,
            "auth success (direct-account): identity_upsert failed"
        );
        return None;
    }
    broker.publish(crate::backend::mps::MuxEvent {
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
    mstore: &Arc<Store>,
    identity_store: &Arc<Store>,
    broker: &Arc<Broker>,
    _direct_account: bool,
    account_id: &str,
    _into_bundle_id: Option<&str>,
    provider_id: &str,
    dir: Option<&str>,
    session_id: &str,
    email: Option<&str>,
) -> (String, Option<String>) {
    if account_id.is_empty() {
        return (String::new(), None);
    }
    let persisted = persist_oauth_direct_account(mstore, identity_store, broker, account_id, provider_id, dir, session_id, email);
    (String::new(), persisted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_oauth_direct_account_round_trip() {
        let mstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let r = persist_oauth_direct_account(
            &mstore,
            &identity_store,
            &broker,
            "acc-1",
            "claude",
            Some("/some/account/dir"),
            "sess-z",
            None,
        );
        assert_eq!(r, Some("acc-1".to_string()));

        let acct = mstore.identity_get("acc-1").unwrap().expect("account row exists");
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
        let mstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let r = persist_oauth_direct_account(&mstore, &identity_store, &broker, "acc-1", "claude", None, "sess-z", None);
        assert!(r.is_none());
        assert!(mstore.identity_get("acc-1").unwrap().is_none(), "nothing persisted when dir is unresolved");
    }

    // SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md. The login email is provider
    // metadata and goes in `context`, NOT `display_name` — that field belongs
    // to the user, so writing an email there would clobber any label they set
    // (and vice versa).
    #[test]
    fn persist_records_the_login_email_in_context_leaving_display_name_free() {
        let mstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::mps::Broker::new());
        persist_oauth_direct_account(
            &mstore,
            &identity_store,
            &broker,
            "acc-1",
            "claude",
            Some("/dir"),
            "sess-z",
            Some("user@example.com"),
        );
        let acct = mstore.identity_get("acc-1").unwrap().expect("account row");
        assert_eq!(acct.context["email"], "user@example.com");
        assert_eq!(acct.display_name, "", "display_name stays the user's to set");
    }

    // Absent, not empty. A provider whose CLI reports no email must stay
    // distinguishable from one that reported a blank address — otherwise the
    // Armory cannot tell "nothing to show" from "logged in as ''".
    #[test]
    fn persist_omits_the_email_key_entirely_when_none_was_captured() {
        let mstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::mps::Broker::new());
        for email in [None, Some(""), Some("   ")] {
            let id = format!("acc-{}", email.unwrap_or("none").len());
            persist_oauth_direct_account(
                &mstore, &identity_store, &broker, &id, "claude", Some("/dir"), "sess-z", email,
            );
            let acct = mstore.identity_get(&id).unwrap().expect("account row");
            assert!(
                acct.context.get("email").is_none(),
                "blank email {email:?} must not create the key"
            );
        }
    }

    #[test]
    fn persist_oauth_success_always_routes_direct_account_mode() {
        // Bundle mode was retired in Phase 4c of
        // SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md — persist_oauth_success
        // always persists a direct account now, regardless of the
        // (now-vestigial) direct_account/into_bundle_id parameters.
        let mstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let (bundle_id, account_id) = persist_oauth_success(
            &mstore,
            &identity_store,
            &broker,
            true,
            "acc-1",
            None,
            "claude",
            Some("/some/dir"),
            "sess-route",
            None,
        );
        assert_eq!(bundle_id, "", "bundle id is always empty now");
        assert_eq!(account_id, Some("acc-1".to_string()));
        assert!(mstore.identity_get("acc-1").unwrap().is_some());
    }

    #[test]
    fn persist_oauth_success_skips_persistence_when_account_id_is_empty() {
        // Reagent P1: `auth.start` sets account_id = "" whenever
        // `direct_account` is false (the wire default) — a caller other
        // than the one production frontend path (which always sends
        // `directAccount: true`) can still reach this. Without the
        // empty-id guard, persist_oauth_direct_account's identity_upsert
        // would silently write/overwrite a db_accounts row with id="".
        let mstore = Arc::new(Store::open_in_memory().unwrap());
        let identity_store = Arc::new(Store::open_in_memory().unwrap());
        let broker = Arc::new(crate::backend::mps::Broker::new());
        let (bundle_id, account_id) = persist_oauth_success(
            &mstore,
            &identity_store,
            &broker,
            false,
            "",
            None,
            "claude",
            Some("/some/dir"),
            "sess-empty",
            None,
        );
        assert_eq!(bundle_id, "");
        assert_eq!(account_id, None, "empty account_id must not be persisted");
        assert!(
            mstore.identity_get("").unwrap().is_none(),
            "no row with id=\"\" should ever be written"
        );
    }
}
