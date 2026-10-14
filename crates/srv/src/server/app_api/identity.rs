use super::*;

/// `identity.self.accounts` and `identity.self.unlink`: the agent asking.
#[derive(Debug, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentitySelfAccountsReq {
    pub agent_id: String,
}

/// `identity.self.accounts`: the provider accounts linked to the agent, with no secrets.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentitySelfAccountsResult {
    pub accounts: Vec<IdentitySelfAccount>,
}

/// One linked account in `IdentitySelfAccountsResult`.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentitySelfAccount {
    pub account_id: String,
    pub provider: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub masked_tail: String,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// `identity.account.upsert`: an account for the agent's own use, created or updated, and linked.
#[derive(Debug, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct IdentityAccountUpsertReq {
    pub agent_id: String,
    pub provider: String,
    pub name: String,
    #[serde(default)]
    #[ts(as = "Option<String>", optional)]
    pub kind: String,
    pub secret: String,
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub validate: bool,
    #[serde(default)]
    #[ts(as = "Option<String>", optional)]
    pub account_id: String,
}

/// `identity.account.upsert`: the account as stored, and the validation result when one was asked for.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentityAccountUpsertResult {
    pub account_id: String,
    pub provider: String,
    pub name: String,
    #[ts(type = r#""valid" | "invalid" | "unknown""#)]
    pub status: String,
    pub masked_tail: String,
    pub valid: bool,
    pub error: Option<String>,
}

/// `identity.account.validate`: a stored account to probe, or a provider and secret to probe ad hoc.
#[derive(Debug, Default, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub struct IdentityAccountValidateReq {
    #[serde(default)]
    #[ts(as = "Option<String>", optional)]
    pub agent_id: String,
    #[serde(default)]
    #[ts(as = "Option<String>", optional)]
    pub account_id: String,
    #[serde(default)]
    #[ts(as = "Option<String>", optional)]
    pub provider: String,
    #[serde(default)]
    #[ts(as = "Option<String>", optional)]
    pub secret: String,
}

/// `identity.account.validate`: whether the provider accepted the key.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentityAccountValidateResult {
    pub valid: bool,
    #[ts(type = r#""valid" | "invalid""#)]
    pub status: String,
    pub masked_tail: String,
    pub error: Option<String>,
}

impl IdentityAccountValidateResult {
    fn of(outcome: crate::identity::key_validator::ValidationOutcome, masked_tail: String) -> Self {
        IdentityAccountValidateResult {
            valid: outcome.valid,
            status: if outcome.valid { "valid" } else { "invalid" }.to_string(),
            masked_tail,
            error: outcome.error,
        }
    }
}

/// `identity.self.unlink`: the provider whose account to unlink from the agent.
#[derive(Debug, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentitySelfUnlinkReq {
    pub agent_id: String,
    pub provider: String,
}

/// `identity.self.unlink`: whether a link was removed.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentitySelfUnlinkResult {
    pub unlinked: bool,
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_identity_self_accounts(engine, state);
    register_identity_account_upsert(engine, state);
    register_identity_account_validate(engine, state);
    register_identity_self_unlink(engine, state);
}

fn register_identity_self_accounts(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_IDENTITY_SELF_ACCOUNTS, move |req: IdentitySelfAccountsReq, ctx| {
        let state = state.clone();
        async move {
            check_s1(&state.mstore, &ctx, &req.agent_id)?;
            identity_self_accounts_impl(&state, &req.agent_id).await
        }
    });
}

fn register_identity_account_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_IDENTITY_ACCOUNT_UPSERT, move |req: IdentityAccountUpsertReq, ctx| {
            let state = state.clone();
            async move {
                let id_store = &state.id_store;
                let identity_store = &state.identity_store;
                let broker = &state.broker;
                check_s1(&state.mstore, &ctx, &req.agent_id)?;
                if req.secret.is_empty() {
                    return Err("identity.account.upsert: secret must not be empty".to_string());
                }

                // The link table is keyed by definition id; req.agent_id is
                // the S1 slug. Resolve once, use for every link-table call
                // below. See resolve_agent_definition_id (mod.rs) for the
                // two failure modes writing the slug causes.
                let def_id = resolve_agent_definition_id(&state, &req.agent_id)
                    .map_err(|e| format!("identity.account.upsert: {e}"))?;

                let is_new = req.account_id.is_empty();
                let account_id = if is_new {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    // Ownership check: the supplied account_id must already be
                    // linked to the calling agent, so callers can't overwrite
                    // another agent's credentials by guessing a UUID.
                    let links = identity_store
                        .agent_identity_list_for_agent(&def_id)
                        .map_err(|e| format!("identity.account.upsert: {e}"))?;
                    let owned = links.iter().any(|l| l.account_id == req.account_id);
                    if !owned {
                        return Err("FORBIDDEN: account not linked to this agent".to_string());
                    }
                    req.account_id.clone()
                };

                // Step 1: store in keychain unconditionally.
                {
                    let aid = account_id.clone();
                    let key = req.secret.clone();
                    tokio::task::spawn_blocking(move || {
                        crate::identity::secret_store::put(&aid, &key)
                    })
                    .await
                    .map_err(|e| format!("identity.account.upsert: keychain task: {e}"))?
                    .map_err(|e| format!("identity.account.upsert: keychain: {e}"))?;
                }

                // Step 1b: optional provider probe (validate controls this only).
                let masked_tail = crate::identity::key_validator::masked_tail(&req.secret);
                let (status, valid, error_msg) = if req.validate {
                    let outcome = crate::identity::key_validator::validate(&req.provider, &req.secret).await;
                    if outcome.valid {
                        ("valid".to_string(), true, None)
                    } else {
                        ("invalid".to_string(), false, outcome.error)
                    }
                } else {
                    ("unknown".to_string(), false, None)
                };

                let now = agentmux_common::time::now_ms();
                let existing = id_store.identity_get(&account_id).ok().flatten();
                let created_at = existing.as_ref()
                    .map(|a| a.created_at)
                    .filter(|&c| c != 0)
                    .unwrap_or(now);

                let mut context = json!({ "masked_tail": masked_tail });
                if let serde_json::Value::Object(ref mut m) = context {
                    if let Some(existing_ctx) = existing.as_ref().map(|a| &a.context) {
                        if let Some(obj) = existing_ctx.as_object() {
                            for (k, v) in obj {
                                m.entry(k).or_insert_with(|| v.clone());
                            }
                        }
                    }
                }

                let account = IdentityAccount {
                    id: account_id.clone(),
                    name: req.name.clone(),
                    provider: req.provider.clone(),
                    kind: if req.kind.is_empty() { "api_key".to_string() } else { req.kind.clone() },
                    display_name: String::new(),
                    secret_ref: crate::backend::storage::identities::SecretRef::Keychain {
                        service: crate::identity::secret_store::SERVICE.to_string(),
                        account: crate::identity::secret_store::account_key(&account_id),
                    },
                    context,
                    status: status.clone(),
                    created_at,
                    updated_at: now,
                };

                // Step 3 (upsert DB). Compensate on failure for new accounts.
                // identity_upsert_with_mirror, not plain identity_upsert —
                // reagentx P0 review on PR #2632: without the mirror write,
                // an account created/updated after the fix shipped still had
                // no fallback entry and reproduced the reported bug on its
                // own next channel switch.
                if let Err(e) = id_store.identity_upsert_with_mirror(&identity_store, &account) {
                    if is_new {
                        let aid = account_id.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            crate::identity::secret_store::delete(&aid)
                        }).await;
                    }
                    return Err(format!("identity.account.upsert: db: {e}"));
                }

                // Step 2/4: (re)point the agent's provider link at this account.
                // agent_identity_link's own `ON CONFLICT(agent_id, provider) DO
                // UPDATE` already overwrites whatever account_id was linked
                // before, so no separate unlink is needed for the success path.
                // A preceding unlink-then-link was here previously (reagent P1 on
                // PR #2056): now that def_id resolves correctly, an unlink that
                // succeeds followed by a link that fails would permanently drop
                // the agent's existing provider link with no compensation (the
                // failure branch below only cleans up for is_new accounts) —
                // removed rather than adding yet another compensating delete.
                if let Err(e) = identity_store.agent_identity_link(&def_id, &account_id, &req.provider) {
                    // Only clean up for new accounts — on the update path the
                    // account still exists in the DB and may be linked to other providers,
                    // so deleting the keychain secret would destroy a valid credential.
                    // Delete the just-upserted db_accounts row too, not only the
                    // keychain secret: without it a failed link left an orphaned,
                    // unlinked account row behind (agent3's report on #1624 PR-C).
                    if is_new {
                        let _ = id_store.identity_delete(&account_id);
                        let aid = account_id.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            crate::identity::secret_store::delete(&aid)
                        }).await;
                    }
                    return Err(format!("identity.account.upsert: link: {e}"));
                }

                broker.publish_event::<crate::backend::mps_events::IdentityAccountsChanged>(&(), vec![], 0);
                broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(&req.agent_id, &(), vec![], 0);

                // The subagent watcher's config-dir resolution for this
                // agent's pane(s) can have raced ahead of this exact
                // binding (fired at reactive-register time, before the
                // launch flow's own account-bind write necessarily lands)
                // and be permanently stuck watching a stale/ambient
                // directory with no other correction mechanism — see
                // `recheck_config_dir`'s own doc comment. Cheap, safe
                // no-op when nothing changed or nothing needs re-pointing.
                if let Some(watcher) = crate::backend::subagent_watcher::global() {
                    watcher.recheck_all_watched_agents();
                }

                Ok(IdentityAccountUpsertResult {
                    account_id,
                    provider: req.provider,
                    name: req.name,
                    status,
                    masked_tail,
                    valid,
                    error: error_msg,
                })
            }
        });
}

fn register_identity_account_validate(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_IDENTITY_ACCOUNT_VALIDATE, move |req: IdentityAccountValidateReq, ctx| {
            let state = state.clone();
            async move {

                if !req.account_id.is_empty() {
                    // Stored-account path: S1 + ownership verification, then probe
                    // using the stored keychain secret (shared with the REST path).
                    check_s1(&state.mstore, &ctx, &req.agent_id)?;
                    return identity_account_validate_stored_impl(&state, &req.agent_id, &req.account_id).await;
                }
                if !req.provider.is_empty() && !req.secret.is_empty() {
                    // Ad-hoc probe — caller supplies their own secret, nothing
                    // stored. WS-only; not exposed over REST/MCP (no inline secret).
                    let masked_tail = crate::identity::key_validator::masked_tail(&req.secret);
                    let outcome = crate::identity::key_validator::validate(&req.provider, &req.secret).await;
                    return Ok(IdentityAccountValidateResult::of(outcome, masked_tail));
                }
                Err("identity.account.validate: provide account_id or (provider + secret)".to_string())
            }
        });
}

fn register_identity_self_unlink(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_IDENTITY_SELF_UNLINK, move |req: IdentitySelfUnlinkReq, ctx| {
            let state = state.clone();
            async move {
                let identity_store = &state.identity_store;
                let broker = &state.broker;
                check_s1(&state.mstore, &ctx, &req.agent_id)?;

                // Link rows are keyed by definition id, not the S1 slug —
                // unlinking by slug always matched zero rows (silent no-op).
                let def_id = resolve_agent_definition_id(&state, &req.agent_id)
                    .map_err(|e| format!("identity.self.unlink: {e}"))?;
                let unlinked = identity_store
                    .agent_identity_unlink(&def_id, &req.provider)
                    .map_err(|e| format!("identity.self.unlink: {e}"))?;
                // info!, not debug!: the production filter is
                // "agentmuxsrv=info,info" — debug lines never reach the log
                // (reagent P1 on PR #2143). Message prefix "identity.unlink:"
                // is part of the `muxlog auth` vocabulary — keep it stable.
                // `unlinked == false` (no link row matched) is logged too:
                // a silent no-op unlink is exactly what an auth stress run
                // needs to see. Both ids logged: link rows are keyed on
                // def_id, not the S1 slug (the historical silent-no-op bug
                // noted above) — a bad slug→def_id resolution is only
                // visible if the log carries both.
                tracing::info!(
                    agent_id = %req.agent_id,
                    def_id = %def_id,
                    provider = %req.provider,
                    unlinked,
                    "identity.unlink: self-service provider unlink (identity.self.unlink)"
                );
                if unlinked {
                    broker.publish_event_for::<crate::backend::mps_events::AgentIdentitiesChanged>(&req.agent_id, &(), vec![], 0);
                }
                Ok(IdentitySelfUnlinkResult { unlinked })
            }
        });
}

pub(crate) async fn identity_self_accounts_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
) -> Result<IdentitySelfAccountsResult, String> {
    // Link rows are keyed by definition id, not the S1 slug callers
    // authenticate with — see resolve_agent_definition_id. Identity M4c-2c:
    // an attributed caller's is its own row's (`SelfOwner`).
    let def_id = owner.into().owner_id(&state.mstore)
        .map_err(|e| format!("identity.self.accounts: {e}"))?;
    let links = state.identity_store.agent_identity_list_for_agent(&def_id)
        .map_err(|e| format!("identity.self.accounts: {e}"))?;
    let mut accounts = Vec::new();
    for link in &links {
        // A malformed `secret_ref` on ONE linked account must not hide this
        // agent's other, perfectly readable accounts — the same "one bad row
        // hides everything" bug class `identity_list` was fixed for (#2419),
        // just reachable through this separate per-agent lookup too. Skip and
        // log rather than `?`-propagate.
        // resolve_account (WITH the global-mirror fallback), not
        // resolve_account_for_spawn: without the fallback a migrated/continuing
        // account showed as "missing" here (#2632).
        //
        // CAVEAT (#2878): this is NO LONGER
        // consistent with the spawn path. `inject.rs` now uses
        // `resolve_account_for_spawn`, which deliberately has no fallback, so an
        // oauth-class account resolvable ONLY via the mirror is listed here yet
        // refused at spawn with `MissingCredentials` (an api-key-class one is
        // silently skipped instead). That divergence is deliberate — a listing
        // should describe what exists rather than silently hide it, and hiding
        // it would reproduce exactly the "my account vanished" confusion #2632
        // fixed — but it is a real UX gap: nothing in this payload tells the
        // caller the account can't satisfy a spawn in THIS channel. Logged
        // below so it is at least observable; surfacing it in the payload (and
        // in Connectors) is tracked as follow-up, not silently assumed fine. See
        // docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md §6.
        match crate::identity::resolver::resolve_account(&state.id_store, &state.identity_store, &link.account_id) {
            Ok(Some((acct, account_store))) => {
                // Observability for the divergence documented above: this
                // account exists only in the always-global mirror, so a spawn in
                // this channel will not accept it.
                if !std::sync::Arc::ptr_eq(&account_store, &state.id_store) {
                    tracing::warn!(
                        target: "identity",
                        account_id = %acct.id,
                        agent_id = %def_id,
                        provider = %acct.provider,
                        "identity.self.accounts: account resolved only via the global mirror; it will NOT satisfy a spawn in this channel (per-channel auth enforcement)"
                    );
                }
                let masked_tail = acct.context.get("masked_tail")
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                accounts.push(IdentitySelfAccount {
                    account_id: acct.id, provider: acct.provider, name: acct.name,
                    kind: acct.kind, status: acct.status, masked_tail,
                    updated_at: acct.updated_at,
                });
            }
            Ok(None) => {
                // Link points at a since-deleted account — skip silently.
            }
            Err(e) => {
                tracing::warn!(
                    target: "identity",
                    account_id = %link.account_id,
                    agent_id = %def_id,
                    error = %e,
                    "identity.self.accounts: skipping unreadable linked account",
                );
            }
        }
    }
    Ok(IdentitySelfAccountsResult { accounts })
}

/// Validate one of the agent's own linked accounts by probing the provider with
/// the stored keychain secret. Ownership is verified (the account must be linked
/// to `agent_id`) before the secret is read.
pub(crate) async fn identity_account_validate_stored_impl<'o>(
    state: &AppState,
    owner: impl Into<SelfOwner<'o>>,
    account_id: &str,
) -> Result<IdentityAccountValidateResult, String> {
    // Link rows are keyed by definition id, not the S1 slug — without the
    // resolution this ownership check always saw zero links and rejected.
    // Identity M4c-2c: an attributed caller's is its own row's, so it can
    // no longer live-probe a same-named agent's stored secret.
    let def_id = owner.into().owner_id(&state.mstore)
        .map_err(|e| format!("identity.account.validate: {e}"))?;
    let links = state.identity_store.agent_identity_list_for_agent(&def_id)
        .map_err(|e| format!("identity.account.validate: {e}"))?;
    if !links.iter().any(|l| l.account_id == account_id) {
        return Err("FORBIDDEN: account not linked to this agent".to_string());
    }
    // resolve_account (WITH the global-mirror fallback) — same reagentx P1
    // review as identity_self_accounts_impl above.
    //
    // CAVEAT since 2026-08-31 (reagent P2 on PR #2878): "consistently with the
    // spawn path" is no longer true — the spawn path dropped this fallback. What
    // this RPC answers is narrower than it looks: it validates the CREDENTIAL
    // (is the key live at the provider), not whether a spawn in this channel
    // would accept the account. A cross-channel-only account can therefore come
    // back `valid: true` and still be refused at spawn. Kept as-is because the
    // credential-validity answer is genuinely correct and callers use it to
    // check a key, not to predict a spawn — but do not read `valid` as
    // "this agent can launch". See the note in identity_self_accounts_impl.
    let (acct, _account_store) = crate::identity::resolver::resolve_account(&state.id_store, &state.identity_store, account_id)
        .map_err(|e| format!("identity.account.validate: {e}"))?
        .ok_or_else(|| format!("identity.account.validate: account {account_id} not found"))?;
    let aid = account_id.to_string();
    let plaintext = tokio::task::spawn_blocking(move || crate::identity::secret_store::get(&aid))
        .await.map_err(|e| format!("identity.account.validate: keychain task: {e}"))?
        .map_err(|e| format!("identity.account.validate: keychain: {e}"))?;
    let tail = crate::identity::key_validator::masked_tail(&plaintext);
    let outcome = crate::identity::key_validator::validate(&acct.provider, &plaintext).await;
    Ok(IdentityAccountValidateResult::of(outcome, tail))
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    /// Each response serializes as the `json!` it replaced did: same keys, `error` null when there is none.
    #[test]
    fn identity_responses_serialize_as_before() {
        let account = IdentitySelfAccount {
            account_id: "a1".into(),
            provider: "claude".into(),
            name: "Work".into(),
            kind: "api_key".into(),
            status: "ok".into(),
            masked_tail: "…wxyz".into(),
            updated_at: 7,
        };
        assert_eq!(
            serde_json::to_value(IdentitySelfAccountsResult { accounts: vec![account] }).unwrap(),
            json!({ "accounts": [{ "account_id": "a1", "provider": "claude", "name": "Work", "kind": "api_key", "status": "ok", "masked_tail": "…wxyz", "updated_at": 7 }] })
        );
        let upsert = IdentityAccountUpsertResult {
            account_id: "a1".into(),
            provider: "claude".into(),
            name: "Work".into(),
            status: "unknown".into(),
            masked_tail: "…wxyz".into(),
            valid: false,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(upsert).unwrap(),
            json!({ "account_id": "a1", "provider": "claude", "name": "Work", "status": "unknown", "masked_tail": "…wxyz", "valid": false, "error": null })
        );
        let outcome = crate::identity::key_validator::ValidationOutcome {
            valid: false,
            metadata: json!({}),
            masked_tail: String::new(),
            error: Some("401".into()),
        };
        assert_eq!(
            serde_json::to_value(IdentityAccountValidateResult::of(outcome, "…wxyz".into())).unwrap(),
            json!({ "valid": false, "status": "invalid", "masked_tail": "…wxyz", "error": "401" })
        );
        assert_eq!(serde_json::to_value(IdentitySelfUnlinkResult { unlinked: true }).unwrap(), json!({ "unlinked": true }));
    }
}
