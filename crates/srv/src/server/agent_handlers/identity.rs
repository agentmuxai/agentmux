// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_IDENTITY_ACCOUNTS, COMMAND_GET_IDENTITY_ACCOUNT,
    COMMAND_UPSERT_IDENTITY_ACCOUNT, COMMAND_DELETE_IDENTITY_ACCOUNT,
    COMMAND_ACCOUNT_KEY_VERIFY,
    COMMAND_ACCOUNT_OAUTH_START, COMMAND_ACCOUNT_OAUTH_POLL, COMMAND_ACCOUNT_OAUTH_CANCEL,
    COMMAND_ACCOUNT_OAUTH_CLIENT_GET, COMMAND_ACCOUNT_OAUTH_CLIENT_CLEAR, COMMAND_GITHUB_CONNECTION, COMMAND_GITHUB_INSTALLATIONS,
    COMMAND_LINK_AGENT_IDENTITY, COMMAND_UNLINK_AGENT_IDENTITY,
    COMMAND_LIST_AGENT_IDENTITIES, COMMAND_LIST_ALL_AGENT_IDENTITIES,
    COMMAND_LIST_NAMED_AGENTS, COMMAND_HIDE_NAMED_AGENT,
    CommandHideNamedAgentData,
    CommandListIdentityAccountsData, CommandGetIdentityAccountData,
    CommandDeleteIdentityAccountData,
    CommandLinkAgentIdentityData, CommandUnlinkAgentIdentityData,
    CommandListAgentIdentitiesData, CommandListAllAgentIdentitiesData,
};
use crate::backend::storage::store::{
    IdentityAccount, SecretRef,
};

use super::super::AppState;

mod accounts;
mod links;
mod named_agents;
mod oauth;

/// Refuses to persist an OAuth account bound directly to its provider's
/// ambient home dir (e.g. `~/.claude`) — nothing downstream validates
/// `secret_ref.dir`, and a real, currently-live account configured exactly
/// this way was found in this repo's own data
/// (`docs/status/STATUS_IDENTITY_ISOLATION_GATE_NOT_ENFORCING_2026_08_20.md`
/// §8). Split out of the `upsertidentityaccount` handler closure so it's
/// directly unit-testable without spinning up the RPC engine. See
/// `docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md`.
fn reject_ambient_home_dir_binding(account: &IdentityAccount) -> Result<(), String> {
    let SecretRef::OAuthConfigDir { dir } = &account.secret_ref else {
        return Ok(());
    };
    let Some(provider_cfg) = crate::backend::providers::get_provider(
        &crate::backend::providers::resolve_provider_alias(&account.provider),
    ) else {
        return Ok(());
    };
    if crate::backend::providers::is_provider_ambient_home_dir(provider_cfg, dir) {
        return Err(format!(
            "upsertidentityaccount: refusing to bind {} identity to its \
             ambient home directory ({dir}) — this would let a spawned \
             agent silently share your personal CLI login/session state \
             instead of using an isolated AgentMux account. Use an \
             isolated config dir for this account instead.",
            account.provider,
        ));
    }
    Ok(())
}

/// Request for `account.key.verify` (account key flow). The `api_key`
/// field is a secret — never log this struct.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyKeyReq {
    /// Service id: "github" | "openai" | "anthropic" | "slack" | … .
    provider: String,
    /// Account display name (user-chosen label).
    name: String,
    #[serde(default)]
    display_name: String,
    /// Account kind; defaults to "api_key" when empty.
    #[serde(default)]
    kind: String,
    /// The pasted secret. Used once to (optionally) validate + store in the
    /// OS keychain, then dropped. Never persisted in the DB, never logged.
    api_key: String,
    /// When true, run a live validation probe before storing (user clicked
    /// "Validate"). When false, store with status "unknown" (the "Save
    /// without validating" air-gapped path).
    #[serde(default)]
    validate: bool,
    /// Set to replace the key on an existing account; empty mints a new one.
    #[serde(default)]
    account_id: String,
    /// User-entered, non-secret context (github_username, scopes, notes, …).
    /// Merged over any existing context so editing a key never wipes fields
    /// the user set previously.
    #[serde(default)]
    context: serde_json::Value,
}

/// Request for `account.oauth.start`. `clientId`/`clientSecret` are BYO OAuth
/// app credentials (optional; required for secret-mandatory providers). Never
/// log this struct: `clientSecret` is a secret.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthStartReq {
    provider: String,
    /// The new account's label; ignored when `accountId` is set.
    #[serde(default)]
    name: String,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
    /// `"byo"` (default) or `"workspace"`: what kind of client `clientId` is.
    #[serde(default)]
    client_mode: Option<String>,
    /// Save `clientId`/`clientSecret` in the keychain for later connects to
    /// this provider once the flow succeeds (default true).
    #[serde(default)]
    save_client: Option<bool>,
    /// Scopes beyond the provider's base ones (a service being turned on).
    #[serde(default)]
    scopes: Vec<String>,
    /// Authorize this existing account again (turn a service on, or
    /// reconnect) instead of creating a new one.
    #[serde(default)]
    account_id: Option<String>,
    /// GitHub: the person agreed to replace the GitHub user this AgentMux account is
    /// connected as with the one who just authorized.
    #[serde(default)]
    replace_github_user: bool,
}

#[derive(serde::Deserialize, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct OAuthSessionReq {
    pub session_id: String,
}

/// Request for `github.connection`: nothing.
#[derive(Default, serde::Deserialize, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct GithubConnectionReq {}

/// Request for `account.oauth.client.get` / `account.oauth.client.clear`.
#[derive(serde::Deserialize, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct OAuthClientReq {
    pub provider: String,
}

/// What `account.oauth.client.get` answers: whether the user's own client is
/// saved for the provider, never its secret.
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AccountOAuthClientInfo {
    saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    client_id: Option<String>,
    has_secret: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "\"byo\" | \"workspace\"")]
    mode: Option<String>,
    /// Whether AgentMux ships its own client for the provider (the one-click
    /// path); false means a connect needs the user's own client.
    builtin_available: bool,
    /// For Google: the services AgentMux's own client may ask for in this build (`google_offered`);
    /// empty for any other provider, and moot for a client the person brought.
    offered_services: Vec<String>,
}

/// What `account.oauth.client.clear` answers: whether a client was saved.
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AccountOAuthClientClearResult {
    cleared: bool,
}

/// Whether AgentMux ships its own client for `provider`, and (Google) which services it may ask for in this build.
fn builtin_client_of(provider: &str) -> Result<(bool, Vec<String>), String> {
    let cfg = crate::identity::oauth_client::config_for(provider)
        .ok_or_else(|| format!("account.oauth.client.get: unknown OAuth provider: {provider}"))?;
    let offered = if provider == "google" { crate::identity::google_offered::offered() } else { Vec::new() };
    Ok((crate::identity::oauth_client::builtin_client_available(&cfg), offered))
}

/// The `OAuthStartReq` fields that become `oauth_client::StartOptions`,
/// minus the reads (saved client, existing account) the handler does.
fn byo_from_req(req: &OAuthStartReq) -> Result<Option<crate::identity::oauth_client::ByoCredentials>, String> {
    let Some(client_id) = req.client_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let mode = crate::identity::oauth_tokens::ClientMode::parse_user(req.client_mode.as_deref())?;
    Ok(Some(crate::identity::oauth_client::ByoCredentials {
        client_id: client_id.to_string(),
        client_secret: req.client_secret.clone().filter(|s| !s.is_empty()),
        mode,
    }))
}

/// Serialize an OAuthStatus to the frontend wire shape.
/// What the Accounts panel's Connect starts a flow with: the consent page opens in the system
/// browser, and Google's account picker has no hint.
fn panel_start_options(
    req: &OAuthStartReq,
    target: crate::identity::oauth_client::FlowTarget,
    byo: Option<crate::identity::oauth_client::ByoCredentials>,
    saved: Option<crate::identity::oauth_tokens::SavedClient>,
    cloud_signed_in: bool,
) -> crate::identity::oauth_client::StartOptions {
    crate::identity::oauth_client::StartOptions {
        target,
        byo,
        saved,
        extra_scopes: req.scopes.clone(),
        save_client: req.save_client.unwrap_or(true),
        cloud_signed_in,
        replace_user: req.replace_github_user,
        open_system_browser: true,
        login_hint: None,
    }
}

fn oauth_status_wire(s: &crate::identity::oauth_client::OAuthStatus) -> serde_json::Value {
    use crate::identity::oauth_client::OAuthStatus;
    match s {
        OAuthStatus::Pending => serde_json::json!({ "status": "pending" }),
        OAuthStatus::UrlAvailable { auth_url } => {
            serde_json::json!({ "status": "url-available", "authUrl": auth_url })
        }
        OAuthStatus::CodeEmitted { user_code, verification_uri } => serde_json::json!({
            "status": "code-emitted",
            "userCode": user_code,
            "verificationUri": verification_uri,
        }),
        OAuthStatus::Success { account_id, granted_scopes, missing_scopes, email } => serde_json::json!({
            "status": "success",
            "accountId": account_id,
            "grantedScopes": granted_scopes,
            "missingScopes": missing_scopes,
            "email": email,
        }),
        OAuthStatus::Failed { error } => serde_json::json!({ "status": "failed", "error": error }),
        OAuthStatus::Picked { account_id, file_ids } => {
            serde_json::json!({ "status": "picked", "accountId": account_id, "fileIds": file_ids })
        }
        OAuthStatus::ConfirmUser { current_login, current_id, new_login, new_id } => serde_json::json!({
            "status": "confirm-user",
            "currentLogin": current_login,
            "currentId": current_id,
            "newLogin": new_login,
            "newId": new_id,
        }),
    }
}

/// Whether a delete stops, keeping the account, because revoking its grant
/// failed and the caller asked for the revoke to succeed first.
pub(super) fn revoke_keeps_account(revoke_report: &serde_json::Value, require_revoke: bool) -> bool {
    require_revoke && revoke_report.get("outcome").and_then(|v| v.as_str()) == Some("failed")
}

/// Shallow-merge the keys of `overlay` (if both are JSON objects) into `base`.
fn merge_json_object(base: &mut serde_json::Value, overlay: &serde_json::Value) {
    if let (Some(b), Some(o)) = (base.as_object_mut(), overlay.as_object()) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
}

/// What `deleteidentityaccount` answers.
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct DeleteIdentityAccountResult {
    pub(crate) deleted: bool,
    /// Agents whose link to the account was removed (delete-time disclosure).
    pub(crate) affected_agents: Vec<String>,
    /// What removing the saved login did; null when there was no account row.
    #[ts(type = "{ outcome: \"removed\" | \"absent\" | \"skipped\" | \"failed\" | \"none\"; path?: string; detail?: string; historyKept?: boolean; historyPath?: string } | null")]
    pub(crate) cleanup: serde_json::Value,
    /// Revoking an OAuth account's grant at the provider (Google) before the
    /// delete; null when there was nothing to revoke. `failed` means the grant
    /// may live on at the provider. With `require_revoke` the account was then
    /// kept (`deleted: false`) so the delete can be retried; without it the
    /// account is gone and the grant is left to the provider's account page.
    #[ts(type = "{ outcome: \"revoked\" | \"failed\"; detail?: string } | null")]
    pub(crate) revoke: serde_json::Value,
}

/// What `account.key.verify` answers: `{ valid: false, error }` when validation
/// failed, otherwise every field but `error` (the same keys the handler sent).
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AccountKeyVerifyResult {
    valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "string")]
    masked_tail: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "Record<string, unknown>")]
    metadata: Option<serde_json::Value>,
}

/// What `account.oauth.start` answers: `{ sessionId, status }`, or `{ error }`.
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AccountOAuthStartResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    session_id: Option<String>,
    /// `oauth_status_wire`'s shape, which the frontend models as OAuthFlowStatus.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "import(\"@/app/store/rpc-api/types\").OAuthFlowStatus")]
    status: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    error: Option<String>,
}

/// `github.installations`: the installations of the GitHub App this account is linked to.
fn register_github_installations(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_GITHUB_INSTALLATIONS,
        move |req: crate::identity::github_installs::GithubInstallationsReq, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move { Ok(crate::identity::github_installs::run(&mstore, &identity_store, req).await) }
        },
    );
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_accounts(engine, state);
    register_oauth(engine, state);
    register_agent_links(engine, state);
    register_named_agents(engine, state);
}

/// Identity account CRUD. `id_store` routes to shared/store.db when available so accounts survive version
/// upgrades, and falls back to `mstore` transparently.
fn register_accounts(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let (mstore, identity_store, broker) = (state.id_store.clone(), state.identity_store.clone(), state.broker.clone());
    {
        let (mstore, identity_store) = (mstore.clone(), identity_store.clone());
        engine.register_typed(COMMAND_LIST_IDENTITY_ACCOUNTS, move |cmd: CommandListIdentityAccountsData, _ctx| {
            accounts::list_identity_accounts(mstore.clone(), identity_store.clone(), cmd.provider)
        });
    }
    {
        let mstore = mstore.clone();
        engine.register_typed(COMMAND_GET_IDENTITY_ACCOUNT, move |cmd: CommandGetIdentityAccountData, _ctx| {
            let mstore = mstore.clone();
            async move { accounts::get_identity_account(&mstore, &cmd.id) }
        });
    }
    // `Value` request: callers send a partial account (generated-manifest.json keeps `Partial<IdentityAccount>`),
    // which is parsed into an `IdentityAccount`.
    {
        let (mstore, identity_store, broker) = (mstore.clone(), identity_store.clone(), broker.clone());
        engine.register_typed(COMMAND_UPSERT_IDENTITY_ACCOUNT, move |data: serde_json::Value, _ctx| {
            let (mstore, identity_store, broker) = (mstore.clone(), identity_store.clone(), broker.clone());
            async move { accounts::upsert_identity_account(&mstore, &identity_store, &broker, data) }
        });
    }
    // `Value` request, parsed into `VerifyKeyReq`; the stub keeps its type.
    {
        let (mstore, identity_store, broker) = (mstore.clone(), identity_store.clone(), broker.clone());
        engine.register_typed(COMMAND_ACCOUNT_KEY_VERIFY, move |data: serde_json::Value, _ctx| {
            accounts::verify_account_key(mstore.clone(), identity_store.clone(), broker.clone(), data)
        });
    }
    engine.register_typed(COMMAND_DELETE_IDENTITY_ACCOUNT, move |cmd: CommandDeleteIdentityAccountData, _ctx| {
        let (mstore, identity_store, broker) = (mstore.clone(), identity_store.clone(), broker.clone());
        async move { super::identity_delete::delete_identity_account(&mstore, &identity_store, &broker, &cmd.id, cmd.require_revoke).await }
    });
}

/// Service OAuth, and the GitHub connection that rides on it.
fn register_oauth(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let (mstore, identity_store) = (state.id_store.clone(), state.identity_store.clone());
    {
        let (mstore, identity_store) = (mstore.clone(), identity_store.clone());
        // `Value` request, parsed into `OAuthStartReq`; the stub keeps its type.
        engine.register_typed(COMMAND_ACCOUNT_OAUTH_START, move |data: serde_json::Value, _ctx| {
            oauth::start_oauth(mstore.clone(), identity_store.clone(), data)
        });
    }
    engine.register_typed(COMMAND_ACCOUNT_OAUTH_CLIENT_GET, move |req: OAuthClientReq, _ctx| oauth::oauth_client_info(req));
    engine.register_typed(COMMAND_ACCOUNT_OAUTH_CLIENT_CLEAR, move |req: OAuthClientReq, _ctx| oauth::clear_oauth_client(req));
    engine.register_typed(COMMAND_ACCOUNT_OAUTH_POLL, move |req: OAuthSessionReq, _ctx| async move { oauth::poll_oauth(&req) });
    engine.register_typed(COMMAND_ACCOUNT_OAUTH_CANCEL, move |req: OAuthSessionReq, _ctx| async move { Ok(oauth::cancel_oauth(&req)) });
    // What the broker says about the GitHub connection; a grant it says is gone marks the brokered account as
    // needing to be connected again (spec §5.7).
    engine.register_typed(COMMAND_GITHUB_CONNECTION, move |_req: GithubConnectionReq, _ctx| {
        let (mstore, identity_store) = (mstore.clone(), identity_store.clone());
        async move { Ok(crate::identity::github_connect::sync_connection(&mstore, &identity_store).await) }
    });
    register_github_installations(engine, state);
}

/// The agent ↔ identity junction.
fn register_agent_links(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let deps = Arc::new(links::LinkDeps {
        identity_store: state.identity_store.clone(),
        broker: state.broker.clone(),
        panes: state.mstore.clone(),
        event_bus: state.event_bus.clone(),
    });
    {
        let deps = deps.clone();
        engine.register_typed(COMMAND_LINK_AGENT_IDENTITY, move |cmd: CommandLinkAgentIdentityData, _ctx| {
            let deps = deps.clone();
            async move { links::link_agent_identity(&deps, &cmd) }
        });
    }
    {
        let deps = deps.clone();
        engine.register_typed(COMMAND_UNLINK_AGENT_IDENTITY, move |cmd: CommandUnlinkAgentIdentityData, _ctx| {
            let deps = deps.clone();
            async move { links::unlink_agent_identity(&deps, &cmd) }
        });
    }
    {
        let deps = deps.clone();
        engine.register_typed(COMMAND_LIST_AGENT_IDENTITIES, move |cmd: CommandListAgentIdentitiesData, _ctx| {
            let deps = deps.clone();
            async move {
                deps.identity_store.agent_identity_list_for_agent(&cmd.agent_id).map_err(|e| format!("listagentidentities: {e}"))
            }
        });
    }
    // Every direct link across every agent: see the constant's doc comment.
    engine.register_typed(COMMAND_LIST_ALL_AGENT_IDENTITIES, move |_req: CommandListAllAgentIdentitiesData, _ctx| {
        let deps = deps.clone();
        async move { deps.identity_store.agent_identity_list_all().map_err(|e| format!("listallagentidentities: {e}")) }
    });
}

/// Named agent continuation (v8).
fn register_named_agents(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let (mstore, id_store, identity_store) = (state.mstore.clone(), state.id_store.clone(), state.identity_store.clone());
    let broker = state.broker.clone();
    {
        let mstore = mstore.clone();
        engine.register_typed(COMMAND_LIST_NAMED_AGENTS, move |data: serde_json::Value, _ctx| {
            let (mstore, id_store, identity_store) = (mstore.clone(), id_store.clone(), identity_store.clone());
            async move { named_agents::list_named_agents(&mstore, &id_store, &identity_store, data) }
        });
    }
    engine.register_typed(COMMAND_HIDE_NAMED_AGENT, move |cmd: CommandHideNamedAgentData, _ctx| {
        let (mstore, broker) = (mstore.clone(), broker.clone());
        async move { named_agents::hide_named_agent(&mstore, &broker, &cmd.id) }
    });
}

#[cfg(test)]
mod oauth_rpc_tests {
    use super::*;
    use crate::identity::oauth_client::OAuthStatus;
    use crate::identity::oauth_tokens::ClientMode;

    #[test]
    fn a_start_request_without_the_new_fields_still_parses() {
        let old: OAuthStartReq =
            serde_json::from_value(json!({ "provider": "google", "name": "Work", "clientId": "cid" })).unwrap();
        assert!(old.scopes.is_empty() && old.account_id.is_none() && old.save_client.is_none());
        let byo = byo_from_req(&old).unwrap().unwrap();
        assert_eq!((byo.client_id.as_str(), byo.mode, byo.client_secret), ("cid", ClientMode::Byo, None));
    }

    #[test]
    fn a_start_request_can_turn_a_service_on_for_an_existing_account() {
        let req: OAuthStartReq = serde_json::from_value(json!({
            "provider": "google",
            "accountId": "acc-1",
            "scopes": ["https://www.googleapis.com/auth/calendar.events.readonly"],
            "clientId": " cid ", "clientSecret": "", "clientMode": "workspace", "saveClient": false,
        }))
        .unwrap();
        assert_eq!(req.account_id.as_deref(), Some("acc-1"));
        assert_eq!(req.scopes.len(), 1);
        assert_eq!(req.save_client, Some(false));
        let byo = byo_from_req(&req).unwrap().unwrap();
        assert_eq!((byo.client_id.as_str(), byo.mode, byo.client_secret), ("cid", ClientMode::Workspace, None));
        // A blank client id means "none passed"; an unknown mode is refused.
        let blank: OAuthStartReq = serde_json::from_value(json!({ "provider": "google", "clientId": "  " })).unwrap();
        assert!(byo_from_req(&blank).unwrap().is_none());
        let bad: OAuthStartReq =
            serde_json::from_value(json!({ "provider": "google", "clientId": "c", "clientMode": "builtin" })).unwrap();
        assert!(byo_from_req(&bad).is_err());
    }

    #[test]
    fn success_reports_the_granted_and_missing_scopes_and_email() {
        let wire = oauth_status_wire(&OAuthStatus::Success {
            account_id: "acc-1".into(),
            granted_scopes: vec!["openid".into()],
            missing_scopes: vec!["https://www.googleapis.com/auth/calendar.events".into()],
            email: Some("me@example.com".into()),
        });
        assert_eq!(
            wire,
            json!({
                "status": "success", "accountId": "acc-1", "grantedScopes": ["openid"],
                "missingScopes": ["https://www.googleapis.com/auth/calendar.events"], "email": "me@example.com",
            })
        );
    }

    #[test]
    fn only_a_failed_revoke_with_require_revoke_keeps_the_account() {
        let failed = json!({ "outcome": "failed", "detail": "offline" });
        assert!(revoke_keeps_account(&failed, true));
        assert!(!revoke_keeps_account(&failed, false), "by default the account is deleted anyway");
        assert!(!revoke_keeps_account(&json!({ "outcome": "revoked" }), true));
        assert!(!revoke_keeps_account(&serde_json::Value::Null, true));
        // The old request shape still parses, and doesn't require the revoke.
        let old: CommandDeleteIdentityAccountData = serde_json::from_value(json!({ "id": "acc-1" })).unwrap();
        assert!(!old.require_revoke);
    }

    #[test]
    fn the_delete_result_carries_the_revoke_outcome() {
        let r = DeleteIdentityAccountResult {
            deleted: true,
            affected_agents: vec![],
            cleanup: json!({ "outcome": "removed" }),
            revoke: json!({ "outcome": "revoked" }),
        };
        let v = serde_json::to_value(r).unwrap();
        assert_eq!(v["revoke"]["outcome"], "revoked");
        assert_eq!(v["cleanup"]["outcome"], "removed");
    }

    #[tokio::test]
    async fn the_saved_client_commands_are_registered_typed() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        register(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let row = |c: &str| rows.iter().find(|r| r["command"] == c).unwrap_or_else(|| panic!("{c} missing")).clone();
        assert_eq!(row(COMMAND_ACCOUNT_OAUTH_CLIENT_GET)["responseName"], "AccountOAuthClientInfo");
        assert_eq!(row(COMMAND_ACCOUNT_OAUTH_CLIENT_CLEAR)["responseName"], "AccountOAuthClientClearResult");
        assert_eq!(row(COMMAND_ACCOUNT_OAUTH_CLIENT_CLEAR)["requestName"], "OAuthClientReq");
    }
}

#[cfg(test)]
mod ambient_home_dir_binding_tests {
    use super::*;
    use crate::server::tests::test_state;

    fn make_account(provider: &str, secret_ref: SecretRef) -> IdentityAccount {
        IdentityAccount {
            id: "acct-1".to_string(),
            name: "test".to_string(),
            provider: provider.to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref,
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    // docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md.
    // Uses the REAL get_home_dir() (not a mock) — same reasoning as the
    // inject.rs spawn-time test, deterministic regardless of whether
    // ~/.claude exists on the test machine.
    #[test]
    fn refuses_oauth_config_dir_pointed_at_the_ambient_home() {
        let ambient = crate::backend::base::get_home_dir()
            .join(".claude")
            .to_string_lossy()
            .into_owned();
        let account = make_account("claude", SecretRef::OAuthConfigDir { dir: ambient.clone() });

        let err = reject_ambient_home_dir_binding(&account)
            .expect_err("must refuse an account bound to the ambient home dir");
        assert!(err.contains(&ambient), "error should name the offending dir: {err}");
        assert!(err.contains("claude"), "error should name the provider: {err}");
    }

    #[test]
    fn allows_oauth_config_dir_pointed_at_an_isolated_dir() {
        let isolated = crate::backend::base::get_home_dir()
            .join(".agentmux")
            .join("shared")
            .join("identities")
            .join("some-id")
            .join("claude")
            .to_string_lossy()
            .into_owned();
        let account = make_account("claude", SecretRef::OAuthConfigDir { dir: isolated });

        assert!(reject_ambient_home_dir_binding(&account).is_ok());
    }

    #[test]
    fn allows_non_oauth_secret_refs_unconditionally() {
        let account = make_account("github", SecretRef::Env { env_var: "GITHUB_TOKEN".to_string() });
        assert!(reject_ambient_home_dir_binding(&account).is_ok());
    }

    #[test]
    fn allows_an_unknown_provider_id_unconditionally() {
        // get_provider() returns None for an unrecognized id — the guard
        // has nothing to check against, so it must not block (matches the
        // existing "unknown provider" skip behavior elsewhere in this
        // codebase, not a new failure mode).
        let account = make_account(
            "not-a-real-provider",
            SecretRef::OAuthConfigDir { dir: "/tmp/whatever".to_string() },
        );
        assert!(reject_ambient_home_dir_binding(&account).is_ok());
    }

    /// `hidenamedagent` is recorded in the engine's schema with its exact
    /// type names. `HideNamedAgentResult` didn't exist before this
    /// migration; the handler used to answer with an anonymous
    /// `json!({"hidden": ..})`.
    #[tokio::test]
    async fn register_typed_records_hidenamedagent() {
        let state = test_state();
        let (engine, _rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        register(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let row = rows
            .iter()
            .find(|r| r["command"] == COMMAND_HIDE_NAMED_AGENT)
            .expect("hidenamedagent missing from the schema");
        assert_eq!(row["requestName"], "CommandHideNamedAgentData");
        assert_eq!(row["responseName"], "HideNamedAgentResult");
        // listnamedagents keeps a loose `Value` request (a malformed body means the
        // defaults), but its response is recorded, so the generated stub returns NamedAgentRow.
        let list = rows
            .iter()
            .find(|r| r["command"] == COMMAND_LIST_NAMED_AGENTS)
            .expect("listnamedagents missing from the schema");
        assert_eq!(list["requestName"], "Value");
        assert_eq!(list["responseTs"]["name"], "Array<NamedAgentRow>");
    }
}
