// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_IDENTITY_ACCOUNTS, COMMAND_GET_IDENTITY_ACCOUNT,
    COMMAND_UPSERT_IDENTITY_ACCOUNT, COMMAND_DELETE_IDENTITY_ACCOUNT,
    COMMAND_ACCOUNT_KEY_VERIFY,
    COMMAND_ACCOUNT_OAUTH_START, COMMAND_ACCOUNT_OAUTH_POLL, COMMAND_ACCOUNT_OAUTH_CANCEL,
    COMMAND_LINK_AGENT_IDENTITY, COMMAND_UNLINK_AGENT_IDENTITY,
    COMMAND_LIST_AGENT_IDENTITIES, COMMAND_LIST_ALL_AGENT_IDENTITIES,
    COMMAND_LIST_NAMED_AGENTS, COMMAND_HIDE_NAMED_AGENT,
    CommandListNamedAgentsData, CommandHideNamedAgentData,
    NamedAgentRow,
    CommandListIdentityAccountsData, CommandGetIdentityAccountData,
    CommandDeleteIdentityAccountData,
    CommandLinkAgentIdentityData, CommandUnlinkAgentIdentityData,
    CommandListAgentIdentitiesData,
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

/// Request for `account.key.verify` (Armory key flow). The `api_key`
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
/// app credentials (optional; required for secret-mandatory providers).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthStartReq {
    provider: String,
    name: String,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthSessionReq {
    session_id: String,
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
        OAuthStatus::Success { account_id } => {
            serde_json::json!({ "status": "success", "accountId": account_id })
        }
        OAuthStatus::Failed { error } => serde_json::json!({ "status": "failed", "error": error }),
    }
}

/// Shallow-merge the keys of `overlay` (if both are JSON objects) into `base`.
fn merge_json_object(base: &mut serde_json::Value, overlay: &serde_json::Value) {
    if let (Some(b), Some(o)) = (base.as_object_mut(), overlay.as_object()) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Identity account CRUD ----
    // id_store: routes to shared/store.db when available so accounts survive
    // version upgrades. Falls back to wstore transparently.

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_LIST_IDENTITY_ACCOUNTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandListIdentityAccountsData =
                    serde_json::from_value(data).unwrap_or_default();
                let accounts = wstore
                    .identity_list(cmd.provider.as_deref())
                    .map_err(|e| format!("listidentityaccounts: {e}"))?;
                Ok(Some(serde_json::to_value(&accounts).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_GET_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetIdentityAccountData =
                    serde_json::from_value(data).map_err(|e| format!("getidentityaccount: {e}"))?;
                match wstore
                    .identity_get(&cmd.id)
                    .map_err(|e| format!("getidentityaccount: {e}"))?
                {
                    Some(a) => Ok(Some(serde_json::to_value(&a).unwrap_or_default())),
                    None => Err(format!("getidentityaccount: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                // Accept the full IdentityAccount payload. Missing `id` → mint
                // a fresh UUID; `created_at` and `updated_at` are server-set
                // so callers don't have to know the current time.
                let mut account: IdentityAccount = serde_json::from_value(data)
                    .map_err(|e| format!("upsertidentityaccount: {e}"))?;
                reject_ambient_home_dir_binding(&account)?;
                if account.id.is_empty() {
                    account.id = uuid::Uuid::new_v4().to_string();
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if account.created_at == 0 {
                    account.created_at = now;
                }
                account.updated_at = now;
                // identity_upsert_with_mirror — reagentx P0 review on PR
                // #2632.
                wstore
                    .identity_upsert_with_mirror(&identity_store, &account)
                    .map_err(|e| format!("upsertidentityaccount: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "identityaccounts:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&account).unwrap_or_default()))
            })
        }),
    );

    // Armory: validate (optional) + securely store an API key.
    // The plaintext goes to the OS keychain; the DB row keeps only the
    // SecretRef::Keychain pointer + masked tail + non-secret metadata.
    // See specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
    let wstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_ACCOUNT_KEY_VERIFY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
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
                        return Ok(Some(serde_json::json!({
                            "valid": false,
                            "error": outcome.error.unwrap_or_else(|| "validation failed".to_string()),
                        })));
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
                let existing = wstore.identity_get(&account_id).ok().flatten();

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

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
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
                if let Err(e) = wstore.identity_upsert_with_mirror(&identity_store, &account) {
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
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "identityaccounts:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::json!({
                    "valid": valid,
                    "accountId": account_id,
                    "maskedTail": account.context.get("masked_tail").cloned().unwrap_or_default(),
                    "status": account.status,
                    "metadata": account.context,
                })))
            })
        }),
    );

    // ── Armory service OAuth (scaffold) ──
    // start: resolve config + client (gates on "not configured"), spawn the
    // flow, return session id + initial status. poll/cancel drive the rest.
    let oauth_wstore = state.id_store.clone();
    let oauth_identity_store = state.identity_store.clone();
    engine.register_handler(
        COMMAND_ACCOUNT_OAUTH_START,
        Box::new(move |data, _ctx| {
            let wstore = oauth_wstore.clone();
            let identity_store = oauth_identity_store.clone();
            Box::pin(async move {
                let req: OAuthStartReq = serde_json::from_value(data)
                    .map_err(|e| format!("account.oauth.start: {e}"))?;
                let byo = req.client_id.map(|cid| {
                    crate::identity::oauth_client::ByoCredentials {
                        client_id: cid,
                        client_secret: req.client_secret,
                    }
                });
                match crate::identity::oauth_client::start(&req.provider, req.name, byo, wstore, identity_store) {
                    Ok((session_id, status)) => Ok(Some(serde_json::json!({
                        "sessionId": session_id,
                        "status": oauth_status_wire(&status),
                    }))),
                    // "not configured" / unknown provider surface as a clean
                    // error field, not an RPC failure, so the UI can show it.
                    Err(e) => Ok(Some(serde_json::json!({ "error": e }))),
                }
            })
        }),
    );

    engine.register_handler(
        COMMAND_ACCOUNT_OAUTH_POLL,
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let req: OAuthSessionReq = serde_json::from_value(data)
                    .map_err(|e| format!("account.oauth.poll: {e}"))?;
                match crate::identity::oauth_client::manager().poll(&req.session_id) {
                    Some(s) => Ok(Some(oauth_status_wire(&s))),
                    None => Err(format!("account.oauth.poll: unknown session {}", req.session_id)),
                }
            })
        }),
    );

    engine.register_handler(
        COMMAND_ACCOUNT_OAUTH_CANCEL,
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let req: OAuthSessionReq = serde_json::from_value(data)
                    .map_err(|e| format!("account.oauth.cancel: {e}"))?;
                let cancelled = crate::identity::oauth_client::manager().cancel(&req.session_id);
                Ok(Some(serde_json::json!({ "cancelled": cancelled })))
            })
        }),
    );

    let wstore = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let identity_store = identity_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteIdentityAccountData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteidentityaccount: {e}"))?;
                // Drop the credential material behind the row BEFORE the row
                // itself: keychain entry for `SecretRef::Keychain`, on-disk
                // OAuth token dir for `SecretRef::OAuthConfigDir` (layer 1 of
                // ANALYSIS_ACCOUNT_DELETE_AUTH_LIFECYCLE_GAP_2026_07_14.md §4).
                // Best-effort — cleanup trouble never blocks the delete; the
                // cleanup fn logs every outcome under "identity.delete:".
                // keyring + fs are blocking, so run via spawn_blocking.
                // Capture provider before the row is deleted so the logout-
                // side log lines can carry it.
                let acct = wstore.identity_get(&cmd.id).ok().flatten();
                let provider = acct.as_ref().map(|a| a.provider.clone()).unwrap_or_default();
                if let Some(acct) = acct {
                    // Containment root for OAuth dirs: only paths inside
                    // ~/.agentmux/shared/identities/ are ever removed (the
                    // legacy ~/.claude migration dir is the user's global
                    // CLI login — never ours to delete).
                    let identities_root = agentmux_common::DataPaths::from_env()
                        .map(|p| p.identities_dir());
                    let _ = tokio::task::spawn_blocking(move || {
                        crate::identity::cleanup::cleanup_account_secrets(
                            &acct,
                            identities_root.as_deref(),
                        )
                    })
                    .await;
                }
                let outcome = wstore
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
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "identityaccounts:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
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
                            broker.publish(crate::backend::wps::WaveEvent {
                                event: format!("agentidentities:changed:{agent_id}"),
                                scopes: vec![],
                                sender: String::new(),
                                persist: 0,
                                data: None,
                            });
                            broker.publish(crate::backend::wps::WaveEvent {
                                event: format!("agentcredentials:revoked:{agent_id}"),
                                scopes: vec![],
                                sender: String::new(),
                                persist: 0,
                                data: Some(json!({
                                    "credentialsRevoked": true,
                                    "provider": provider,
                                    "accountId": cmd.id,
                                })),
                            });
                        }
                    }
                }
                Ok(Some(json!({
                    "deleted": deleted,
                    // Layer 4 — Armory delete-time disclosure (spec §4).
                    "affectedAgents": affected_agents,
                })))
            })
        }),
    );

    // ---- Agent ↔ Identity junction ----

    let wstore = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_LINK_AGENT_IDENTITY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandLinkAgentIdentityData = serde_json::from_value(data)
                    .map_err(|e| format!("linkagentidentity: {e}"))?;
                wstore
                    .agent_identity_link(&cmd.agent_id, &cmd.account_id, &cmd.provider)
                    .map_err(|e| format!("linkagentidentity: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agentidentities:changed:{}", cmd.agent_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    let wstore = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UNLINK_AGENT_IDENTITY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandUnlinkAgentIdentityData = serde_json::from_value(data)
                    .map_err(|e| format!("unlinkagentidentity: {e}"))?;
                let removed = wstore
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
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: format!("agentidentities:changed:{}", cmd.agent_id),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                    // Spec §3 names unlink alongside delete: a live process
                    // for this agent still holds the unlinked account's
                    // tokens until restarted — same disclosure chip. Skipped
                    // for `silent` unlinks (alias migration, not a real
                    // unbind — see CommandUnlinkAgentIdentityData's doc
                    // comment; reagent P2 on PR #2414).
                    if !cmd.silent {
                        broker.publish(crate::backend::wps::WaveEvent {
                            event: format!("agentcredentials:revoked:{}", cmd.agent_id),
                            scopes: vec![],
                            sender: String::new(),
                            persist: 0,
                            data: Some(json!({
                                "credentialsRevoked": true,
                                "provider": cmd.provider,
                            })),
                        });
                    }
                }
                Ok(Some(json!({ "unlinked": removed })))
            })
        }),
    );

    let wstore = state.identity_store.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_IDENTITIES,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandListAgentIdentitiesData = serde_json::from_value(data)
                    .map_err(|e| format!("listagentidentities: {e}"))?;
                let rows = wstore
                    .agent_identity_list_for_agent(&cmd.agent_id)
                    .map_err(|e| format!("listagentidentities: {e}"))?;
                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );

    // Every direct link across every agent — see the constant's doc comment.
    let wstore = state.identity_store.clone();
    engine.register_handler(
        COMMAND_LIST_ALL_AGENT_IDENTITIES,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let rows = wstore
                    .agent_identity_list_all()
                    .map_err(|e| format!("listallagentidentities: {e}"))?;
                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );

    // ---- v8: named agent continuation ----

    // listnamedagents — powers the launch modal's "Continue agent"
    // dropdown. Joins instance rows with the definition / identity /
    // memory bundle names so the frontend renders without follow-ups.
    let wstore = state.wstore.clone();
    let id_store_lna = state.id_store.clone();
    let identity_store_lna = state.identity_store.clone();
    engine.register_handler(
        COMMAND_LIST_NAMED_AGENTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let id_store = id_store_lna.clone();
            let identity_store = identity_store_lna.clone();
            Box::pin(async move {
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
                let defs = wstore
                    .agent_def_list()
                    .map_err(|e| format!("listnamedagents: agent_def_list: {e}"))?;
                let memories = id_store
                    .bundle_memory_list()
                    .map_err(|e| format!("listnamedagents: bundle_memory_list: {e}"))?;

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
                let accounts_by_id: std::collections::HashMap<&str, &IdentityAccount> =
                    accounts.iter().map(|a| (a.id.as_str(), a)).collect();
                let mut links_by_agent: std::collections::HashMap<&str, Vec<&AgentIdentityLink>> =
                    std::collections::HashMap::new();
                for link in &agent_identity_links {
                    links_by_agent
                        .entry(link.agent_id.as_str())
                        .or_default()
                        .push(link);
                }
                let resolve_identity_name = |definition_id: &str| -> String {
                    match links_by_agent.get(definition_id) {
                        Some(links) if !links.is_empty() => {
                            let mut names: Vec<String> = links
                                .iter()
                                .map(|link| {
                                    accounts_by_id
                                        .get(link.account_id.as_str())
                                        .map(|a| a.name.clone())
                                        .unwrap_or_else(|| "(missing account)".to_string())
                                })
                                .collect();
                            names.sort();
                            names.dedup();
                            names.join(", ")
                        }
                        _ => "(ambient creds)".to_string(),
                    }
                };

                // PR B — read from the cross-version registry when
                // it's available. Falls back to SQLite when the
                // registry couldn't be resolved at startup (CI / odd
                // environments). SQLite remains authoritative for
                // PR B (parallel-write is still active); the choice
                // here just affects which surface gets surfaced.
                let rows: Vec<NamedAgentRow> = match wstore.shared_agent_registry() {
                    Some(reg) => {
                        // Re-join relative working_dir against the CURRENT
                        // channel's agents dir (symmetric with the write
                        // mirror), not the registry's own parent — P0.3
                        // re-roots the registry out of channels/<ch>/agents/.
                        let agents_root = wstore.registry_agents_base();
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
                        let sqlite_rows: Vec<AgentInstance> = wstore
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
                        let instances = wstore
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

                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );

    // hidenamedagent — soft-delete (sets display_hidden = 1) so the
    // row disappears from the dropdown. Working dir stays on disk.
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_HIDE_NAMED_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandHideNamedAgentData = serde_json::from_value(data)
                    .map_err(|e| format!("hidenamedagent: {e}"))?;
                let hidden = wstore
                    .instance_set_hidden(&cmd.id, true)
                    .map_err(|e| format!("hidenamedagent: {e}"))?;
                if hidden {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "namedagents:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(json!({ "hidden": hidden })))
            })
        }),
    );
}

#[cfg(test)]
mod ambient_home_dir_binding_tests {
    use super::*;

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
}
