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
    COMMAND_LIST_AGENT_IDENTITIES,
    COMMAND_LIST_NAMED_AGENTS, COMMAND_HIDE_NAMED_AGENT,
    CommandListNamedAgentsData, CommandHideNamedAgentData,
    NamedAgentRow,
    CommandListIdentityAccountsData, CommandGetIdentityAccountData,
    CommandDeleteIdentityAccountData,
    CommandLinkAgentIdentityData, CommandUnlinkAgentIdentityData,
    CommandListAgentIdentitiesData,
};
use crate::backend::storage::store::{
    AgentInstance, IdentityAccount, SecretRef,
};

use super::super::AppState;

/// Request for `account.key.verify` (Trust Center key flow). The `api_key`
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
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                // Accept the full IdentityAccount payload. Missing `id` → mint
                // a fresh UUID; `created_at` and `updated_at` are server-set
                // so callers don't have to know the current time.
                let mut account: IdentityAccount = serde_json::from_value(data)
                    .map_err(|e| format!("upsertidentityaccount: {e}"))?;
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
                wstore
                    .identity_upsert(&account)
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

    // Trust Center: validate (optional) + securely store an API key.
    // The plaintext goes to the OS keychain; the DB row keeps only the
    // SecretRef::Keychain pointer + masked tail + non-secret metadata.
    // See specs/SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_ACCOUNT_KEY_VERIFY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
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
                if let Err(e) = wstore.identity_upsert(&account) {
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

    // ── Trust Center service OAuth (scaffold) ──
    // start: resolve config + client (gates on "not configured"), spawn the
    // flow, return session id + initial status. poll/cancel drive the rest.
    let oauth_wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_ACCOUNT_OAUTH_START,
        Box::new(move |data, _ctx| {
            let wstore = oauth_wstore.clone();
            Box::pin(async move {
                let req: OAuthStartReq = serde_json::from_value(data)
                    .map_err(|e| format!("account.oauth.start: {e}"))?;
                let byo = req.client_id.map(|cid| {
                    crate::identity::oauth_client::ByoCredentials {
                        client_id: cid,
                        client_secret: req.client_secret,
                    }
                });
                match crate::identity::oauth_client::start(&req.provider, req.name, byo, wstore) {
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
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteIdentityAccountData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteidentityaccount: {e}"))?;
                // If this account stored its secret in the OS keychain, drop
                // it too so no orphaned credential survives the DB row.
                // `keyring` is blocking, so run it via spawn_blocking.
                if let Ok(Some(acct)) = wstore.identity_get(&cmd.id) {
                    if matches!(acct.secret_ref, SecretRef::Keychain { .. }) {
                        let aid = cmd.id.clone();
                        let res = tokio::task::spawn_blocking(move || {
                            crate::identity::secret_store::delete(&aid)
                        })
                        .await
                        .unwrap_or_else(|je| Err(format!("join: {je}")));
                        if let Err(e) = res {
                            tracing::warn!(target: "identity", "keychain delete for {} failed: {e}", cmd.id);
                        }
                    }
                }
                let deleted = wstore
                    .identity_delete(&cmd.id)
                    .map_err(|e| format!("deleteidentityaccount: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "identityaccounts:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(json!({ "deleted": deleted })))
            })
        }),
    );

    // ---- Agent ↔ Identity junction ----

    let wstore = state.id_store.clone();
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

    let wstore = state.id_store.clone();
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
                if removed {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: format!("agentidentities:changed:{}", cmd.agent_id),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(json!({ "unlinked": removed })))
            })
        }),
    );

    let wstore = state.id_store.clone();
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

    // ---- v8: named agent continuation ----

    // listnamedagents — powers the launch modal's "Continue agent"
    // dropdown. Joins instance rows with the definition / identity /
    // memory bundle names so the frontend renders without follow-ups.
    let wstore = state.wstore.clone();
    let id_store_lna = state.id_store.clone();
    engine.register_handler(
        COMMAND_LIST_NAMED_AGENTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let id_store = id_store_lna.clone();
            Box::pin(async move {
                let cmd: CommandListNamedAgentsData =
                    serde_json::from_value(data).unwrap_or_default();
                let limit = if cmd.limit == 0 {
                    200
                } else {
                    cmd.limit.min(1000)
                };
                // Resolve bundle names once per response. With ≤200
                // rows and typical bundle counts in the low dozens,
                // a linear lookup on cached lists beats per-row
                // round-trips through the store.
                let defs = wstore
                    .agent_def_list()
                    .map_err(|e| format!("listnamedagents: agent_def_list: {e}"))?;
                let identities = id_store
                    .bundle_identity_list()
                    .map_err(|e| format!("listnamedagents: bundle_identity_list: {e}"))?;
                let memories = id_store
                    .bundle_memory_list()
                    .map_err(|e| format!("listnamedagents: bundle_memory_list: {e}"))?;

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
                                let identity_name = if identity_id_str.is_empty() {
                                    "(ambient creds)".to_string()
                                } else {
                                    identities
                                        .iter()
                                        .find(|i| i.id == identity_id_str)
                                        .map(|i| i.name.clone())
                                        .unwrap_or_else(|| "(missing identity)".to_string())
                                };
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
                                let identity_name = if inst.identity_id.is_empty() {
                                    "(ambient creds)".to_string()
                                } else {
                                    identities
                                        .iter()
                                        .find(|i| i.id == inst.identity_id)
                                        .map(|i| i.name.clone())
                                        .unwrap_or_else(|| "(missing identity)".to_string())
                                };
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
