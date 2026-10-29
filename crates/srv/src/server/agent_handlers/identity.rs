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
    COMMAND_ACCOUNT_OAUTH_CLIENT_GET, COMMAND_ACCOUNT_OAUTH_CLIENT_CLEAR, COMMAND_GITHUB_CONNECTION,
    COMMAND_LINK_AGENT_IDENTITY, COMMAND_UNLINK_AGENT_IDENTITY,
    COMMAND_LIST_AGENT_IDENTITIES, COMMAND_LIST_ALL_AGENT_IDENTITIES,
    COMMAND_LIST_NAMED_AGENTS, COMMAND_HIDE_NAMED_AGENT,
    CommandListNamedAgentsData, CommandHideNamedAgentData, HideNamedAgentResult,
    NamedAgentRow,
    CommandListIdentityAccountsData, CommandGetIdentityAccountData,
    CommandDeleteIdentityAccountData,
    CommandLinkAgentIdentityData, CommandUnlinkAgentIdentityData,
    CommandListAgentIdentitiesData, CommandListAllAgentIdentitiesData,
    UnlinkAgentIdentityResult, AccountOAuthCancelResult,
};
use crate::backend::storage::store::{
    AgentIdentityLink, AgentInstance, IdentityAccount, SecretRef,
};

use super::super::AppState;

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
}

/// What `account.oauth.client.clear` answers: whether a client was saved.
#[derive(serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct AccountOAuthClientClearResult {
    cleared: bool,
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
fn revoke_keeps_account(revoke_report: &serde_json::Value, require_revoke: bool) -> bool {
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
    deleted: bool,
    /// Agents whose link to the account was removed (delete-time disclosure).
    affected_agents: Vec<String>,
    /// What removing the saved login did; null when there was no account row.
    #[ts(type = "{ outcome: \"removed\" | \"absent\" | \"skipped\" | \"failed\" | \"none\"; path?: string; detail?: string; historyKept?: boolean; historyPath?: string } | null")]
    cleanup: serde_json::Value,
    /// Revoking an OAuth account's grant at the provider (Google) before the
    /// delete; null when there was nothing to revoke. `failed` means the grant
    /// may live on at the provider. With `require_revoke` the account was then
    /// kept (`deleted: false`) so the delete can be retried; without it the
    /// account is gone and the grant is left to the provider's account page.
    #[ts(type = "{ outcome: \"revoked\" | \"failed\"; detail?: string } | null")]
    revoke: serde_json::Value,
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

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Identity account CRUD ----
    // id_store: routes to shared/store.db when available so accounts survive
    // version upgrades. Falls back to mstore transparently.

    let mstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_LIST_IDENTITY_ACCOUNTS,
        move |cmd: CommandListIdentityAccountsData, _ctx| {
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move {
                // On the blocking pool: the backfill below reads each OAuth
                // account's `.claude.json` and canonicalizes its dir (muxreview
                // P1 on #3617), like this file's other fs/keyring calls.
                tokio::task::spawn_blocking(move || {
                let mut accounts = mstore
                    .identity_list(cmd.provider.as_deref())
                    .map_err(|e| format!("listidentityaccounts: {e}"))?;
                // SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md §4: backfill each
                // OAuth account's login email from its own config dir, so
                // accounts signed in before #3541 — and every Claude login,
                // whose transcript prints no email — show it. Written only
                // when it changed, and only that one field, in this store and
                // the global mirror (a full upsert here could re-create an
                // account deleted since the list, undo a status written
                // meanwhile, or repoint the mirror's `secret_ref`). A failed
                // write still returns the email.
                for account in &mut accounts {
                    let before = account.context.get("email").and_then(|e| e.as_str()).unwrap_or("").to_string();
                    if let Some(email) = crate::identity::account_email::refresh_account_email(account) {
                        // Another address is another group (the store drops the number the same way).
                        if !crate::identity::account_numbering::same_email(&before, &email) {
                            if let Some(ctx) = account.context.as_object_mut() {
                                ctx.remove("seq");
                                ctx.remove("seqmax");
                            }
                        }
                        for store in [&mstore, &identity_store] {
                            if let Err(e) = store.identity_set_context_email(&account.id, &email) {
                                tracing::warn!(target: "identity", account_id = %account.id, error = %e, "listidentityaccounts: email backfill write failed");
                            }
                        }
                    }
                }
                // #4936 §3.3: give each account that shares a provider and email
                // with another a number that stays its own. Computed over the
                // unfiltered list, so a provider filter can't renumber a group.
                // One allocation at a time, from a fresh read: two overlapping lists must not
                // both plan the same number, or the one written last renumbers what the first returned.
                static NUMBERING: std::sync::Mutex<()> = std::sync::Mutex::new(());
                let _numbering = NUMBERING.lock().unwrap_or_else(|e| e.into_inner());
                let all = mstore.identity_list(None).unwrap_or_default();
                for u in crate::identity::account_numbering::numbering_updates(&all) {
                    for store in [&mstore, &identity_store] {
                        if let Err(e) = store.identity_set_context_numbering(&u.id, u.seq, u.high) {
                            tracing::warn!(target: "identity", account_id = %u.id, error = %e, "listidentityaccounts: number write failed");
                        }
                    }
                }
                // Answer with the numbers as stored now, including any a list that ran just
                // before this one allocated.
                let numbered = mstore.identity_list(None).unwrap_or_default();
                for a in &mut accounts {
                    let Some(stored) = numbered.iter().find(|n| n.id == a.id) else { continue };
                    for key in ["seq", "seqmax"] {
                        match stored.context.get(key) {
                            Some(v) => {
                                if !a.context.is_object() {
                                    a.context = serde_json::json!({});
                                }
                                a.context[key] = v.clone();
                            }
                            None => {
                                if let Some(ctx) = a.context.as_object_mut() {
                                    ctx.remove(key);
                                }
                            }
                        }
                    }
                }
                Ok(accounts)
                })
                .await
                .map_err(|e| format!("listidentityaccounts: task: {e}"))?
            }
        },
    );

    let mstore = state.id_store.clone();
    engine.register_typed(
        COMMAND_GET_IDENTITY_ACCOUNT,
        move |cmd: CommandGetIdentityAccountData, _ctx| {
            let mstore = mstore.clone();
            async move {
                match mstore
                    .identity_get(&cmd.id)
                    .map_err(|e| format!("getidentityaccount: {e}"))?
                {
                    Some(a) => Ok(a),
                    None => Err(format!("getidentityaccount: not found id={}", cmd.id)),
                }
            }
        },
    );

    let mstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    // `Value` request: callers send a partial account (generated-manifest.json
    // keeps `Partial<IdentityAccount>`), which this parses into IdentityAccount.
    engine.register_typed(COMMAND_UPSERT_IDENTITY_ACCOUNT, move |data: serde_json::Value, _ctx| {
        let mstore = mstore.clone();
        let identity_store = identity_store.clone();
        let broker = broker.clone();
        async move {
                // Accept the full IdentityAccount payload. Missing `id` → mint
                // a fresh UUID; `created_at` and `updated_at` are server-set
                // so callers don't have to know the current time.
                let mut account: IdentityAccount = serde_json::from_value(data)
                    .map_err(|e| format!("upsertidentityaccount: {e}"))?;
                reject_ambient_home_dir_binding(&account)?;
                if account.id.is_empty() {
                    account.id = uuid::Uuid::new_v4().to_string();
                }
                let now = agentmux_common::time::now_ms();
                if account.created_at == 0 {
                    account.created_at = now;
                }
                account.updated_at = now;
                // identity_upsert_with_mirror — reagentx P0 review on PR
                // #2632.
                mstore
                    .identity_upsert_with_mirror(&identity_store, &account)
                    .map_err(|e| format!("upsertidentityaccount: {e}"))?;
                broker.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
                Ok(account)
        }
    });

    // Connectors → Accounts: validate (optional) + securely store an API key.
    // The plaintext goes to the OS keychain; the DB row keeps only the
    // SecretRef::Keychain pointer + masked tail + non-secret metadata.
    // See docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
    let mstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    // `Value` request, parsed into VerifyKeyReq below; the stub keeps its type.
    engine.register_typed(COMMAND_ACCOUNT_KEY_VERIFY, move |data: serde_json::Value, _ctx| {
        let mstore = mstore.clone();
        let identity_store = identity_store.clone();
        let broker = broker.clone();
        async move {
                // NB: `req.api_key` is a secret — never log `req`.
                let req: VerifyKeyReq = serde_json::from_value(data)
                    .map_err(|e| format!("account.key.verify: {e}"))?;

                // New account (mint id) vs. key replacement on an existing
                // account. Rollback semantics differ: see the upsert below.
                let is_new = req.account_id.is_empty();
                let account_id = if is_new {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    req.account_id.clone()
                };

                // Optional live validation — the single outbound probe, fired
                // only when the user clicked "Validate" (validate=true).
                let (status, metadata, masked_tail, valid) = if req.validate {
                    let outcome =
                        crate::identity::key_validator::validate(&req.provider, &req.api_key).await;
                    if !outcome.valid {
                        // Nothing stored — surface a structured error so the UI
                        // stays in the entry state.
                        return Ok(AccountKeyVerifyResult {
                            valid: false,
                            error: Some(outcome.error.unwrap_or_else(|| "validation failed".to_string())),
                            account_id: None,
                            masked_tail: None,
                            status: None,
                            metadata: None,
                        });
                    }
                    ("valid".to_string(), outcome.metadata, outcome.masked_tail, true)
                } else {
                    (
                        "unknown".to_string(),
                        serde_json::json!({}),
                        crate::identity::key_validator::masked_tail(&req.api_key),
                        false,
                    )
                };

                // Store the plaintext in the OS keychain; the DB never sees it.
                // `keyring` is blocking (sync D-Bus on Linux), so run it off the
                // async runtime worker via spawn_blocking.
                {
                    let aid = account_id.clone();
                    let key = req.api_key.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::identity::secret_store::put(&aid, &key)
                    })
                    .await
                    .map_err(|e| format!("account.key.verify: keychain task: {e}"))?
                    .map_err(|e| format!("account.key.verify: {e}"))?;
                }

                // Fetch the existing row once (replacement path) — preserves
                // created_at and any previously-stored context.
                let existing = mstore.identity_get(&account_id).ok().flatten();

                // Non-secret context, merged so nothing the user set is lost:
                //   existing context  →  user-entered context  →  validation
                //   metadata  →  masked tail.
                let mut context = existing
                    .as_ref()
                    .map(|a| a.context.clone())
                    .unwrap_or_else(|| serde_json::json!({}));
                if !context.is_object() {
                    context = serde_json::json!({});
                }
                merge_json_object(&mut context, &req.context);
                merge_json_object(&mut context, &metadata);
                if let serde_json::Value::Object(ref mut m) = context {
                    m.insert("masked_tail".to_string(), serde_json::json!(masked_tail));
                }

                let now = agentmux_common::time::now_ms();
                let created_at = existing
                    .as_ref()
                    .map(|a| a.created_at)
                    .filter(|&c| c != 0)
                    .unwrap_or(now);

                let account = IdentityAccount {
                    id: account_id.clone(),
                    name: req.name.clone(),
                    provider: req.provider.clone(),
                    kind: if req.kind.is_empty() { "api_key".to_string() } else { req.kind.clone() },
                    display_name: req.display_name.clone(),
                    secret_ref: SecretRef::Keychain {
                        service: crate::identity::secret_store::SERVICE.to_string(),
                        account: crate::identity::secret_store::account_key(&account_id),
                    },
                    context,
                    status,
                    created_at,
                    updated_at: now,
                };
                // identity_upsert_with_mirror — reagentx P0 review on PR #2632.
                if let Err(e) = mstore.identity_upsert_with_mirror(&identity_store, &account) {
                    // DB write failed after the keychain write.
                    //  - New account: nothing references the secret yet, so
                    //    roll it back to avoid an orphan with no DB row.
                    //  - Replacement: the existing DB row still points at this
                    //    keychain entry. Deleting it would destroy the
                    //    previously-working credential; leave the (now
                    //    overwritten) secret in place — resolution still works
                    //    with the new key, and the user can retry to fix
                    //    metadata. So only roll back for new accounts.
                    if is_new {
                        let aid = account_id.clone();
                        if let Err(de) = tokio::task::spawn_blocking(move || {
                            crate::identity::secret_store::delete(&aid)
                        })
                        .await
                        .unwrap_or_else(|je| Err(format!("join: {je}")))
                        {
                            tracing::warn!(
                                target: "identity",
                                "rollback of orphaned keychain secret for {} failed: {de}",
                                account_id,
                            );
                        }
                    }
                    return Err(format!("account.key.verify: {e}"));
                }
                broker.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
                // A key the provider just accepted proves the account works (#2237).
                if valid {
                    crate::server::account_episodes::after_sign_in(&account_id);
                }
                Ok(AccountKeyVerifyResult {
                    valid,
                    error: None,
                    account_id: Some(account_id),
                    masked_tail: Some(account.context.get("masked_tail").cloned().unwrap_or_default()),
                    status: Some(account.status),
                    metadata: Some(account.context),
                })
        }
    });

    // ── Service OAuth ──
    // start: resolve config + client (gates on "not configured"), spawn the
    // flow, return session id + initial status. poll/cancel drive the rest.
    // With `accountId` it authorizes that account again (more scopes, or a
    // reconnect) instead of creating one. Spec G0 of
    // docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md.
    let oauth_mstore = state.id_store.clone();
    let oauth_identity_store = state.identity_store.clone();
    // `Value` request, parsed into OAuthStartReq below; the stub keeps its type.
    engine.register_typed(COMMAND_ACCOUNT_OAUTH_START, move |data: serde_json::Value, _ctx| {
        let mstore = oauth_mstore.clone();
        let identity_store = oauth_identity_store.clone();
        async move {
                use crate::identity::oauth_client::{FlowTarget, StartOptions};
                // NB: `req.client_secret` is a secret — never log `req`.
                let req: OAuthStartReq = serde_json::from_value(data)
                    .map_err(|e| format!("account.oauth.start: {e}"))?;
                let failed = |e: String| -> Result<AccountOAuthStartResult, String> {
                    Ok(AccountOAuthStartResult { session_id: None, status: None, error: Some(e) })
                };
                let byo = match byo_from_req(&req) {
                    Ok(b) => b,
                    Err(e) => return failed(e),
                };
                let target = match req.account_id.as_deref().filter(|s| !s.is_empty()) {
                    None => FlowTarget::New { name: req.name.clone() },
                    Some(id) => match mstore.identity_get(id) {
                        Ok(Some(acct)) => FlowTarget::Existing(acct),
                        Ok(None) => return failed(format!("account {id} not found")),
                        Err(e) => return failed(format!("account {id} could not be read: {e}")),
                    },
                };
                // The user's saved client, when none came with the request. A
                // keychain that can't be read means none: the panel asks again.
                let saved = if byo.is_some() {
                    None
                } else {
                    let provider = req.provider.clone();
                    let loaded = tokio::task::spawn_blocking(move || crate::identity::oauth_tokens::load_saved_client(&provider))
                        .await
                        .map_err(|e| format!("keychain task: {e}"))
                        .and_then(|r| r);
                    match loaded {
                        Ok(saved) => saved,
                        Err(e) => {
                            tracing::warn!(target: "identity", provider = %req.provider, error = %e, "account.oauth.start: the saved OAuth client could not be read");
                            None
                        }
                    }
                };
                // GitHub connects through the broker when signed in to AgentMux Cloud.
                let cloud_signed_in = if req.provider == crate::identity::github_connect::PROVIDER {
                    let store = mstore.clone();
                    tokio::task::spawn_blocking(move || store.muxbus_load().ok().flatten().is_some()).await.unwrap_or(false)
                } else {
                    false
                };
                let opts = StartOptions {
                    target,
                    byo,
                    saved,
                    extra_scopes: req.scopes.clone(),
                    save_client: req.save_client.unwrap_or(true),
                    cloud_signed_in,
                    replace_user: req.replace_github_user,
                };
                match crate::identity::oauth_client::start(&req.provider, opts, mstore, identity_store) {
                    Ok((session_id, status)) => Ok(AccountOAuthStartResult {
                        session_id: Some(session_id),
                        status: Some(oauth_status_wire(&status)),
                        error: None,
                    }),
                    // "not configured" / unknown provider surface as a clean
                    // error field, not an RPC failure, so the UI can show it.
                    Err(e) => failed(e),
                }
        }
    });

    // The user's own OAuth client saved for a provider (D8): whether there is
    // one, and forgetting it. Accounts made with it keep refreshing: each
    // keeps its own copy of what a refresh needs.
    engine.register_typed(COMMAND_ACCOUNT_OAUTH_CLIENT_GET, move |req: OAuthClientReq, _ctx| async move {
        let builtin_available = crate::identity::oauth_client::config_for(&req.provider)
            .map(|cfg| crate::identity::oauth_client::builtin_client_available(&cfg))
            .ok_or_else(|| format!("account.oauth.client.get: unknown OAuth provider: {}", req.provider))?;
        let provider = req.provider.clone();
        let saved = tokio::task::spawn_blocking(move || crate::identity::oauth_tokens::load_saved_client(&provider))
            .await
            .map_err(|e| format!("account.oauth.client.get: keychain task: {e}"))?
            .map_err(|e| format!("account.oauth.client.get: {e}"))?;
        Ok(AccountOAuthClientInfo {
            saved: saved.is_some(),
            client_id: saved.as_ref().map(|c| c.client_id.clone()),
            has_secret: saved.as_ref().is_some_and(|c| c.client_secret.is_some()),
            mode: saved.as_ref().map(|c| c.mode.as_str().to_string()),
            builtin_available,
        })
    });

    engine.register_typed(COMMAND_ACCOUNT_OAUTH_CLIENT_CLEAR, move |req: OAuthClientReq, _ctx| async move {
        if crate::identity::oauth_client::config_for(&req.provider).is_none() {
            return Err(format!("account.oauth.client.clear: unknown OAuth provider: {}", req.provider));
        }
        let provider = req.provider.clone();
        let cleared = tokio::task::spawn_blocking(move || crate::identity::oauth_tokens::clear_saved_client(&provider))
            .await
            .map_err(|e| format!("account.oauth.client.clear: keychain task: {e}"))?
            .map_err(|e| format!("account.oauth.client.clear: {e}"))?;
        tracing::info!(target: "identity", provider = %req.provider, cleared, "account.oauth.client.clear: saved OAuth client forgotten");
        Ok(AccountOAuthClientClearResult { cleared })
    });

    // What the broker says about the GitHub connection; a grant it says is gone marks the
    // brokered account as needing to be connected again (spec §5.7).
    let github_mstore = state.id_store.clone();
    let github_identity_store = state.identity_store.clone();
    engine.register_typed(COMMAND_GITHUB_CONNECTION, move |_req: GithubConnectionReq, _ctx| {
        let mstore = github_mstore.clone();
        let identity_store = github_identity_store.clone();
        async move { Ok(crate::identity::github_connect::sync_connection(&mstore, &identity_store).await) }
    });

    // The response stays `Value`: oauth_status_wire builds it, and the frontend
    // models it as OAuthFlowStatus (generated-manifest.json `returns`).
    engine.register_typed(COMMAND_ACCOUNT_OAUTH_POLL, move |req: OAuthSessionReq, _ctx| {
        async move {
                match crate::identity::oauth_client::manager().poll(&req.session_id) {
                    Some(s) => Ok(oauth_status_wire(&s)),
                    None => Err(format!("account.oauth.poll: unknown session {}", req.session_id)),
                }
        }
    });

    engine.register_typed(
        COMMAND_ACCOUNT_OAUTH_CANCEL,
        move |req: OAuthSessionReq, _ctx| {
            async move {
                let cancelled = crate::identity::oauth_client::manager().cancel(&req.session_id);
                Ok(AccountOAuthCancelResult { cancelled })
            }
        },
    );

    let mstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(COMMAND_DELETE_IDENTITY_ACCOUNT, move |cmd: CommandDeleteIdentityAccountData, _ctx| {
        let mstore = mstore.clone();
        let identity_store = identity_store.clone();
        let broker = broker.clone();
        async move {
                // Drop the credential material behind the row BEFORE the row
                // itself: keychain entry for `SecretRef::Keychain`, on-disk
                // OAuth token dir for `SecretRef::OAuthConfigDir` (layer 1 of
                // ANALYSIS_ACCOUNT_DELETE_AUTH_LIFECYCLE_GAP_2026_07_14.md §4).
                // Best-effort — cleanup trouble never blocks the delete; the
                // cleanup fn logs every outcome under "identity.delete:".
                // keyring + fs are blocking, so run via spawn_blocking.
                // Capture provider before the row is deleted so the logout-
                // side log lines can carry it.
                let acct = mstore.identity_get(&cmd.id).ok().flatten();
                let provider = acct.as_ref().map(|a| a.provider.clone()).unwrap_or_default();
                // An OAuth grant is revoked at the provider first, while the
                // token is still in the keychain; otherwise it would live on
                // there (spec §7 "Disconnect"). Null when there's none.
                let mut revoke_report = serde_json::Value::Null;
                if let Some(acct) = acct.as_ref() {
                    use crate::identity::oauth_tokens::{revoke_before_delete, KeychainSecrets, LiveHttp};
                    // A GitHub grant made through the broker is ended there.
                    let report = match crate::identity::github_connect::revoke_before_delete(&mstore, &KeychainSecrets, acct).await {
                        Some(report) => Some(report),
                        None => revoke_before_delete(&LiveHttp, &KeychainSecrets, acct).await,
                    };
                    if let Some(report) = report {
                        revoke_report = report;
                    }
                }
                if revoke_keeps_account(&revoke_report, cmd.require_revoke) {
                    return Ok(DeleteIdentityAccountResult {
                        deleted: false,
                        affected_agents: Vec::new(),
                        cleanup: serde_json::Value::Null,
                        revoke: revoke_report,
                    });
                }
                // What the cleanup did, returned to Connectors so a login that
                // couldn't be removed is reported, not hidden
                // (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md H3).
                let mut cleanup_report = serde_json::Value::Null;
                if let Some(acct) = acct {
                    // Containment root for OAuth dirs: only paths inside
                    // ~/.agentmux/shared/identities/ are ever removed (the
                    // legacy ~/.claude migration dir is the user's global
                    // CLI login — never ours to delete).
                    let identities_root = agentmux_common::DataPaths::from_env()
                        .map(|p| p.identities_dir());
                    cleanup_report = match tokio::task::spawn_blocking(move || {
                        crate::identity::cleanup::cleanup_account_secrets(
                            &acct,
                            identities_root.as_deref(),
                        )
                    })
                    .await
                    {
                        Ok(outcome) => outcome.report(),
                        Err(e) => json!({ "outcome": "failed", "detail": e.to_string() }),
                    };
                }
                let outcome = mstore
                    .identity_delete(&cmd.id)
                    .map_err(|e| format!("deleteidentityaccount: {e}"))?;
                let deleted = outcome.deleted;
                // Links now live in identity_store, a different physical
                // store than the account row (id_store) —
                // identity_delete's own cascade above only ever reaches
                // rows in the SAME store it's called on, so it no longer
                // finds anything to cascade (SPEC_IDENTITY_STORE_SPLIT_
                // 2026_08_17.md). Clean up the real link rows here
                // explicitly and merge the two outcomes for the existing
                // notification logic below, so this doesn't regress into
                // silently orphaning links on every account delete.
                let (links_cascaded_from_identity_store, mut affected_agents) = identity_store
                    .agent_identity_unlink_by_account(&cmd.id)
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
                if let Err(e) = identity_store.identity_delete(&cmd.id) {
                    tracing::warn!(
                        account_id = %cmd.id,
                        error = %e,
                        "deleteidentityaccount: identity_store mirror row cleanup failed (non-fatal)"
                    );
                }
                // A Google account's grants and call log go with it (connectors spec D4, D7).
                if let Err(e) = identity_store.google_account_forget(&cmd.id) {
                    tracing::warn!(account_id = %cmd.id, error = %e, "deleteidentityaccount: Google grants cleanup failed (non-fatal)");
                }
                if links_cascaded > 0 {
                    // info!, not debug!: the production filter is
                    // "agentmuxsrv=info,info" (reagent P1, PR #2143).
                    // "identity.delete:" is `muxlog auth` vocabulary.
                    tracing::info!(
                        account_id = %cmd.id,
                        provider = %provider,
                        links = links_cascaded,
                        "identity.delete: links cascaded"
                    );
                }
                tracing::info!(
                    account_id = %cmd.id,
                    provider = %provider,
                    deleted,
                    "identity.delete: account removed (deleteidentityaccount)"
                );
                if deleted {
                    broker.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
                    // Layer 2 — running-agent reconciliation (spec
                    // SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §3):
                    // the cascaded links name agents that may have a LIVE
                    // process still holding this account's tokens. We
                    // surface, not hard-kill — a per-agent revocation event
                    // drives the pane chip; enforcement lands at the next
                    // spawn (layer 3). Also poke the per-agent link-changed
                    // event so link tables refresh.
                    if !affected_agents.is_empty() {
                        // info!, not debug!: the production filter is
                        // "agentmuxsrv=info,info". "identity.delete:" is
                        // `muxlog auth` vocabulary.
                        tracing::info!(
                            account_id = %cmd.id,
                            provider = %provider,
                            count = affected_agents.len(),
                            agent_ids = %affected_agents.join(","),
                            "identity.delete: running agent(s) affected"
                        );
                        for agent_id in &affected_agents {
                            broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(&agent_id, &(), vec![], 0);
                            let revoked = crate::backend::mps_events::CredentialsRevokedEvent {
                                credentials_revoked: true,
                                provider: provider.to_string(),
                                account_id: Some(cmd.id.clone()),
                            };
                            broker.publish_event_for::<crate::backend::mps_events::AgentCredentialsRevoked>(&agent_id, &revoked, vec![], 0);
                        }
                    }
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
    });

    // ---- Agent ↔ Identity junction ----

    let mstore = state.identity_store.clone();
    let broker = state.broker.clone();
    let (panes, event_bus) = (state.mstore.clone(), state.event_bus.clone());
    engine.register_typed(
        COMMAND_LINK_AGENT_IDENTITY,
        move |cmd: CommandLinkAgentIdentityData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            let (panes, event_bus) = (panes.clone(), event_bus.clone());
            async move {
                // Read before the link changes: an old pane counts against
                // the agent's current links (SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.3).
                let failed = crate::server::account_episodes::failed_panes(&panes, &mstore, &cmd.agent_id, &cmd.provider);
                mstore
                    .agent_identity_link(&cmd.agent_id, &cmd.account_id, &cmd.provider)
                    .map_err(|e| format!("linkagentidentity: {e}"))?;
                crate::server::account_episodes::after_rebind(&panes, &event_bus, failed, &cmd.account_id);
                broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(&cmd.agent_id, &(), vec![], 0);
                // See app_api/identity.rs's identical call for why this is
                // needed: watch_agent's one-shot config-dir resolution can
                // race ahead of this exact binding.
                if let Some(watcher) = crate::backend::subagent_watcher::global() {
                    watcher.recheck_all_watched_agents();
                }
                Ok(())
            }
        },
    );

    let mstore = state.identity_store.clone();
    let broker = state.broker.clone();
    let (panes, event_bus) = (state.mstore.clone(), state.event_bus.clone());
    engine.register_typed(
        COMMAND_UNLINK_AGENT_IDENTITY,
        move |cmd: CommandUnlinkAgentIdentityData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            let (panes, event_bus) = (panes.clone(), event_bus.clone());
            async move {
                let failed = crate::server::account_episodes::failed_panes(&panes, &mstore, &cmd.agent_id, &cmd.provider);
                let removed = mstore
                    .agent_identity_unlink(&cmd.agent_id, &cmd.provider)
                    .map_err(|e| format!("unlinkagentidentity: {e}"))?;
                // info!, not debug!: the production filter is
                // "agentmuxsrv=info,info" (reagent P1, PR #2143).
                // "identity.unlink:" is `muxlog auth` vocabulary.
                tracing::info!(
                    agent_id = %cmd.agent_id,
                    provider = %cmd.provider,
                    unlinked = removed,
                    "identity.unlink: agent-identity link removed (unlinkagentidentity)"
                );
                if removed {
                    // A silent unlink is the first half of a rebind (alias rows), the link that follows
                    // clears; a real one leaves the agent without an account, so its old limit goes.
                    if !cmd.silent {
                        crate::server::account_episodes::after_rebind(&panes, &event_bus, failed, "");
                    }
                    broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(&cmd.agent_id, &(), vec![], 0);
                    // Spec §3 names unlink alongside delete: a live process
                    // for this agent still holds the unlinked account's
                    // tokens until restarted — same disclosure chip. Skipped
                    // for `silent` unlinks (alias migration, not a real
                    // unbind — see CommandUnlinkAgentIdentityData's doc
                    // comment; reagent P2 on PR #2414).
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
        },
    );

    let mstore = state.identity_store.clone();
    engine.register_typed(
        COMMAND_LIST_AGENT_IDENTITIES,
        move |cmd: CommandListAgentIdentitiesData, _ctx| {
            let mstore = mstore.clone();
            async move {
                let rows = mstore
                    .agent_identity_list_for_agent(&cmd.agent_id)
                    .map_err(|e| format!("listagentidentities: {e}"))?;
                Ok(rows)
            }
        },
    );

    // Every direct link across every agent — see the constant's doc comment.
    let mstore = state.identity_store.clone();
    engine.register_typed(
        COMMAND_LIST_ALL_AGENT_IDENTITIES,
        move |_req: CommandListAllAgentIdentitiesData, _ctx| {
            let mstore = mstore.clone();
            async move {
                let rows = mstore
                    .agent_identity_list_all()
                    .map_err(|e| format!("listallagentidentities: {e}"))?;
                Ok(rows)
            }
        },
    );

    // ---- v8: named agent continuation ----

    // listnamedagents — powers the picker's My Agents and the Files
    // pane. Joins instance rows with the definition, account and
    // bundle names so the frontend renders without follow-ups.
    let mstore = state.mstore.clone();
    let id_store_lna = state.id_store.clone();
    let identity_store_lna = state.identity_store.clone();
    // `Value` request: a malformed one means the defaults (unwrap_or_default).
    engine.register_typed(COMMAND_LIST_NAMED_AGENTS, move |data: serde_json::Value, _ctx| {
        let mstore = mstore.clone();
        let id_store = id_store_lna.clone();
        let identity_store = identity_store_lna.clone();
        async move {
                let cmd: CommandListNamedAgentsData =
                    serde_json::from_value(data).unwrap_or_default();
                let limit = if cmd.limit == 0 {
                    200
                } else {
                    cmd.limit.min(1000)
                };
                // Resolve names once per response. With ≤200 rows and
                // typical account/bundle counts in the low dozens, a
                // linear lookup on cached lists beats per-row
                // round-trips through the store.
                let defs = mstore
                    .agent_def_list()
                    .map_err(|e| format!("listnamedagents: agent_def_list: {e}"))?;
                let memories = id_store
                    .bundle_list()
                    .map_err(|e| format!("listnamedagents: bundle_list: {e}"))?;

                // Identity display names resolve off the direct
                // agent<->account links (db_agent_identity_links /
                // db_accounts) now, not the retired bundle tables — see
                // SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md
                // §4 item 2. Bulk-fetched once and grouped by
                // definition_id rather than queried per-row.
                let agent_identity_links = identity_store
                    .agent_identity_list_all()
                    .map_err(|e| format!("listnamedagents: agent_identity_links: {e}"))?;
                let accounts = id_store
                    .identity_list(None)
                    .map_err(|e| format!("listnamedagents: accounts: {e}"))?;
                let own_ids: std::collections::HashSet<&str> = accounts.iter().map(|a| a.id.as_str()).collect();
                let mut mirror_failed = false;
                let mirror_accounts: Vec<IdentityAccount> =
                    if agent_identity_links.iter().any(|l| !own_ids.contains(l.account_id.as_str())) {
                        identity_store.identity_list(None).unwrap_or_else(|e| {
                            tracing::warn!(error = %e, "listnamedagents: the global account mirror could not be read");
                            mirror_failed = true;
                            Vec::new()
                        })
                    } else {
                        Vec::new()
                    };
                let account_names = super::account_label::names_by_id(&accounts, &mirror_accounts);
                let mut links_by_agent: std::collections::HashMap<&str, Vec<&AgentIdentityLink>> =
                    std::collections::HashMap::new();
                for link in &agent_identity_links {
                    links_by_agent
                        .entry(link.agent_id.as_str())
                        .or_default()
                        .push(link);
                }
                let resolve_identity_name = |definition_id: &str| -> String {
                    super::account_label::account_line(
                        links_by_agent.get(definition_id).map(|v| v.as_slice()),
                        &account_names,
                        super::account_label::Degraded { links: false, accounts: mirror_failed },
                    )
                    .text
                };

                // PR B — read from the cross-version registry when
                // it's available. Falls back to SQLite when the
                // registry couldn't be resolved at startup (CI / odd
                // environments). SQLite remains authoritative for
                // PR B (parallel-write is still active); the choice
                // here just affects which surface gets surfaced.
                let rows: Vec<NamedAgentRow> = match mstore.shared_agent_registry() {
                    Some(reg) => {
                        // Re-join relative working_dir against the CURRENT
                        // channel's agents dir (symmetric with the write
                        // mirror), not the registry's own parent — P0.3
                        // re-roots the registry out of channels/<ch>/agents/.
                        let agents_root = mstore.registry_agents_base();
                        let mut records = reg
                            .list_active()
                            .map_err(|e| format!("listnamedagents: registry: {e}"))?;
                        if let Some(def_filter) = cmd.definition_id.as_deref() {
                            records.retain(|r| r.data.definition_id == def_filter);
                        }
                        records.sort_by(|a, b| {
                            b.data
                                .last_launched_at_ms
                                .cmp(&a.data.last_launched_at_ms)
                        });
                        records.truncate(limit);
                        // Pre-fetch all candidate same-version rows
                        // ONCE so enrichment doesn't issue N+1 queries.
                        // Indexed by instance_id; rows that aren't in
                        // current SQLite fall through to sentinels.
                        // Registry enrichment: keep head-of-chain
                        // only. The registry mirror itself excludes
                        // continuations (see
                        // `registry_upsert_if_named`), so the SQLite
                        // side must match — else under the `limit`
                        // truncation continuation rows displace
                        // registry-head rows and the merge-by-id
                        // enrichment misses, silently downgrading
                        // running-state badges and block_id_hints to
                        // "available" / empty.
                        let sqlite_rows: Vec<AgentInstance> = mstore
                            .instance_list_named(
                                records.len().max(1),
                                cmd.definition_id.as_deref(),
                                /* identity_id */ None,
                                /* include_continuations */ false,
                            )
                            .unwrap_or_default();
                        let sqlite_by_id: std::collections::HashMap<&str, &AgentInstance> =
                            sqlite_rows.iter().map(|i| (i.id.as_str(), i)).collect();
                        records
                            .into_iter()
                            .map(|rec| {
                                let d = rec.data;
                                let def = defs.iter().find(|x| x.id == d.definition_id);
                                let identity_id_str =
                                    d.identity_id.clone().unwrap_or_default();
                                let memory_id_str = d.memory_id.clone().unwrap_or_default();
                                let identity_name = resolve_identity_name(&d.definition_id);
                                let memory_name = if memory_id_str.is_empty() {
                                    "(vanilla CLI)".to_string()
                                } else {
                                    memories
                                        .iter()
                                        .find(|m| m.id == memory_id_str)
                                        .map(|m| m.name.clone())
                                        .unwrap_or_else(|| "(missing memory)".to_string())
                                };
                                // Reconstruct the absolute working_directory.
                                // v3 records carry their SOURCE channel agents
                                // dir, so a row from another channel resolves
                                // to its real workspace; legacy (v1/v2) records
                                // fall back to the current channel base — the
                                // pre-P0.4 behavior (correct for same-channel
                                // rows, which is all v1/v2 could represent).
                                let working_directory = if let Some(src) =
                                    d.source_agents_base.as_deref()
                                {
                                    std::path::Path::new(src)
                                        .join(&d.working_dir)
                                        .to_string_lossy()
                                        .to_string()
                                } else {
                                    match agents_root.as_ref() {
                                        Some(root) => root
                                            .join(&d.working_dir)
                                            .to_string_lossy()
                                            .to_string(),
                                        None => d.working_dir.clone(),
                                    }
                                };
                                // Same-version enrichment: if this id
                                // also exists in current SQLite, the
                                // row carries runtime state (block_id
                                // for focus-existing-pane, status,
                                // ended_at) that the registry
                                // intentionally doesn't track.
                                // Cross-version rows fall through with
                                // sentinel "available" status and
                                // empty block_id_hint.
                                let (block_id_hint, status, ended_at) =
                                    match sqlite_by_id.get(d.instance_id.as_str()) {
                                        Some(inst) => (
                                            inst.block_id.clone(),
                                            inst.status.clone(),
                                            inst.ended_at,
                                        ),
                                        None => (String::new(), "available".to_string(), 0),
                                    };
                                NamedAgentRow {
                                    instance_id: d.instance_id,
                                    instance_name: d.instance_name,
                                    definition_id: d.definition_id.clone(),
                                    definition_name: def
                                        .map(|x| x.name.clone())
                                        .unwrap_or_else(|| "(missing definition)".to_string()),
                                    provider: def
                                        .map(|x| x.provider.clone())
                                        .unwrap_or_default(),
                                    working_directory,
                                    identity_id: identity_id_str,
                                    identity_name,
                                    memory_id: memory_id_str,
                                    memory_name,
                                    started_at: d.last_launched_at_ms,
                                    ended_at,
                                    status,
                                    block_id_hint,
                                }
                            })
                            .collect()
                    }
                    None => {
                        // No-registry fallback: drives the launch
                        // modal's "Continue agent" dropdown directly.
                        // One entry per chain root, mirroring the
                        // registry path's semantics.
                        let instances = mstore
                            .instance_list_named(
                                limit,
                                cmd.definition_id.as_deref(),
                                /* identity_id */ None,
                                /* include_continuations */ false,
                            )
                            .map_err(|e| format!("listnamedagents: {e}"))?;
                        instances
                            .into_iter()
                            .map(|inst| {
                                let def = defs.iter().find(|d| d.id == inst.definition_id);
                                let identity_name = resolve_identity_name(&inst.definition_id);
                                let memory_name = if inst.memory_id.is_empty() {
                                    "(vanilla CLI)".to_string()
                                } else {
                                    memories
                                        .iter()
                                        .find(|m| m.id == inst.memory_id)
                                        .map(|m| m.name.clone())
                                        .unwrap_or_else(|| "(missing memory)".to_string())
                                };
                                NamedAgentRow {
                                    instance_id: inst.id,
                                    instance_name: inst.instance_name,
                                    definition_id: inst.definition_id.clone(),
                                    definition_name: def
                                        .map(|d| d.name.clone())
                                        .unwrap_or_else(|| "(missing definition)".to_string()),
                                    provider: def
                                        .map(|d| d.provider.clone())
                                        .unwrap_or_default(),
                                    working_directory: inst.working_directory,
                                    identity_id: inst.identity_id,
                                    identity_name,
                                    memory_id: inst.memory_id,
                                    memory_name,
                                    started_at: inst.started_at,
                                    ended_at: inst.ended_at,
                                    status: inst.status,
                                    block_id_hint: inst.block_id,
                                }
                            })
                            .collect()
                    }
                };

                Ok(rows)
        }
    });

    // hidenamedagent — soft-delete (sets display_hidden = 1) so the
    // row disappears from the dropdown. Working dir stays on disk.
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_HIDE_NAMED_AGENT,
        move |cmd: CommandHideNamedAgentData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                let hidden = mstore
                    .instance_set_hidden(&cmd.id, true)
                    .map_err(|e| format!("hidenamedagent: {e}"))?;
                if hidden {
                    broker.publish_event::<crate::backend::mps_events::NamedAgentsChanged>(&(), vec![], 0);
                }
                Ok(HideNamedAgentResult { hidden })
            }
        },
    );
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
