use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_identity_self_accounts(engine, state);
    register_identity_account_upsert(engine, state);
    register_identity_account_validate(engine, state);
    register_identity_self_unlink(engine, state);
}

fn register_identity_self_accounts(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_IDENTITY_SELF_ACCOUNTS,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("identity.self.accounts: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
                Ok(Some(identity_self_accounts_impl(&state, &req.agent_id).await?))
            })
        }),
    );
}

fn register_identity_account_upsert(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_IDENTITY_ACCOUNT_UPSERT,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                let id_store = &state.id_store;
                let identity_store = &state.identity_store;
                let broker = &state.broker;
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "snake_case")]
                struct Req {
                    agent_id: String,
                    provider: String,
                    name: String,
                    #[serde(default)]
                    kind: String,
                    secret: String,
                    #[serde(default)]
                    validate: bool,
                    #[serde(default)]
                    account_id: String,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("identity.account.upsert: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;
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

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
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

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "identityaccounts:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agentidentities:changed:{}", req.agent_id),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });

                Ok(Some(json!({
                    "account_id":  account_id,
                    "provider":    req.provider,
                    "name":        req.name,
                    "status":      status,
                    "masked_tail": masked_tail,
                    "valid":       valid,
                    "error":       error_msg,
                })))
            })
        }),
    );
}

fn register_identity_account_validate(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_IDENTITY_ACCOUNT_VALIDATE,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize, Default)]
                #[serde(rename_all = "snake_case")]
                struct Req {
                    #[serde(default)] agent_id: String,
                    #[serde(default)] account_id: String,
                    #[serde(default)] provider: String,
                    #[serde(default)] secret: String,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("identity.account.validate: {e}"))?;

                if !req.account_id.is_empty() {
                    // Stored-account path: S1 + ownership verification, then probe
                    // using the stored keychain secret (shared with the REST path).
                    check_s1(&ctx, &req.agent_id)?;
                    return Ok(Some(
                        identity_account_validate_stored_impl(&state, &req.agent_id, &req.account_id).await?,
                    ));
                }
                if !req.provider.is_empty() && !req.secret.is_empty() {
                    // Ad-hoc probe — caller supplies their own secret, nothing
                    // stored. WS-only; not exposed over REST/MCP (no inline secret).
                    let masked_tail = crate::identity::key_validator::masked_tail(&req.secret);
                    let outcome = crate::identity::key_validator::validate(&req.provider, &req.secret).await;
                    return Ok(Some(json!({
                        "valid": outcome.valid,
                        "status": if outcome.valid { "valid" } else { "invalid" },
                        "masked_tail": masked_tail,
                        "error": outcome.error,
                    })));
                }
                Err("identity.account.validate: provide account_id or (provider + secret)".to_string())
            })
        }),
    );
}

fn register_identity_self_unlink(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_handler(
        COMMAND_IDENTITY_SELF_UNLINK,
        Box::new(move |data, ctx| {
            let state = state.clone();
            Box::pin(async move {
                let id_store = &state.id_store;
                let identity_store = &state.identity_store;
                let broker = &state.broker;
                #[derive(serde::Deserialize)]
                struct Req { agent_id: String, provider: String }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("identity.self.unlink: {e}"))?;
                check_s1(&ctx, &req.agent_id)?;

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
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: format!("agentidentities:changed:{}", req.agent_id),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }
                Ok(Some(json!({ "unlinked": unlinked })))
            })
        }),
    );
}
