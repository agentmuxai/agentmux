use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_agent_define(engine, state);
}

/// Rejects a non-empty `model_vendor_base_url` unless `provider_id`'s
/// `ProviderConfig` declares support for redirection via
/// `base_url_env_var`. Pure — no I/O — so it's directly unit-testable
/// without the async `Store`/`Broker` harness `agent_define_core` needs.
/// An empty `base_url` is always fine — that's "use the harness's default
/// vendor endpoint," never rejected regardless of provider.
///
/// `pub(crate)` (not `pub(super)`) so `server::agent_handlers::template`'s
/// `agentdefcreatefromtemplate` and `server::agent_handlers::core`'s
/// `updateagent` handlers can reuse it too, instead of duplicating this
/// check for the human-facing creation/edit paths.
pub(crate) fn validate_vendor_base_url(provider_id: &str, base_url: &str) -> Result<(), String> {
    if base_url.is_empty() {
        return Ok(());
    }
    match providers::get_provider(provider_id) {
        Some(p) if p.base_url_env_var.is_some() => Ok(()),
        _ => Err(format!(
            "agent.define: provider '{provider_id}' does not support a custom model vendor base URL"
        )),
    }
}

/// An agent's provider is read-only once set: its sessions, linked accounts
/// and native memory all belong to that harness, so switching it would
/// strand them. Run the same setup on another harness by forking the agent.
/// The lock moved here from the agent's bundle
/// (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.1). An empty stored
/// provider can still be set once.
pub(crate) fn check_provider_unchanged(agent_id: &str, existing: &str, incoming: &str) -> Result<(), String> {
    if existing.is_empty() || existing == incoming {
        return Ok(());
    }
    Err(format!(
        "FORBIDDEN: agent {agent_id} provider is readonly once set (has '{existing}', got '{incoming}'); fork the agent to run it on another provider"
    ))
}

/// Infer a provider slug from a model name prefix.
/// Only maps prefixes that correspond to a registered provider slug.
/// Callers must still validate the result via `providers::get_provider`.
pub(super) fn infer_provider_from_model(model: &str) -> String {
    let m = model.to_lowercase();
    if m.starts_with("claude") {
        "claude".to_string()
    } else if m.starts_with("gemini") {
        "gemini".to_string()
    } else if m.starts_with("codex") {
        "codex".to_string()
    } else if m.starts_with("qwen") {
        "qwen".to_string()
    } else if m.starts_with("kimi") {
        "kimi".to_string()
    } else {
        // Unknown prefix — return as-is; get_provider will reject it with
        // a "cannot infer provider" error so callers know to set provider explicitly.
        model.to_string()
    }
}

/// Returns `(agent_row_id, newly_inserted)`. In the consolidated model the
/// definition row IS the agent, so the "stub instance" that makes a defined
/// agent visible in My Agents is that same row carrying an `instance_name`;
/// `instance_create` folds into it. `newly_inserted = false` when the row
/// already carries a name (the stub already existed); callers use this to
/// avoid broadcasting `agents:changed` on no-op calls. The returned id is
/// the row's id, which is what `instance_create` reports back.
pub(super) fn make_stub_idempotent(
    mstore: &crate::backend::storage::store::Store,
    def_id: &str,
    name: &str,
    now: i64,
) -> Result<(String, bool), String> {
    if let Some(existing) = mstore
        .instance_get(def_id)
        .map_err(|e| format!("agent.define: read agent row: {e}"))?
    {
        if !existing.instance_name.is_empty() {
            return Ok((existing.id, false));
        }
    }
    let stub_id = format!("si-{}", def_id.replace('-', ""));
    let inst = AgentInstance {
        id: stub_id.clone(),
        definition_id: def_id.to_string(),
        parent_instance_id: String::new(),
        block_id: String::new(),
        session_id: String::new(),
        status: "stopped".to_string(),
        github_context: String::new(),
        started_at: now,
        ended_at: 0,
        created_at: now,
        identity_id: String::new(),
        memory_id: String::new(),
        instance_name: name.to_string(),
        working_directory: String::new(),
        display_hidden: false,
    };
    match mstore.instance_create(&inst) {
        Ok(canonical) => Ok((canonical.id, true)),
        Err(e) => Err(format!("agent.define: create stub instance: {e}")),
    }
}

/// Core logic for the `agent.define` command, shared by the WebSocket RPC
/// handler and the HTTP service dispatch (`("agent", "define")` in service.rs).
/// Persist `system_prompt` and `env` content blobs for a freshly created or
/// updated agent definition.  Errors are logged but not propagated — the
/// definition row is already committed and the caller has already published
/// `agents:changed`, so a content-write failure must not abort the response.
pub(super) fn persist_define_content(
    mstore: &Store,
    agent_id: &str,
    cmd: &CommandAgentDefineData,
    now: i64,
) {
    if let Some(prompt) = &cmd.system_prompt {
        if !prompt.is_empty() {
            if let Err(e) = mstore.agent_content_set(&AgentContent {
                agent_id: agent_id.to_string(),
                content_type: "agentmd".to_string(),
                content: prompt.clone(),
                updated_at: now,
            }) {
                tracing::warn!(agent_id, err = %e, "agent.define: failed to persist system_prompt (non-fatal)");
            }
        }
    }
    if let Some(env_map) = &cmd.env {
        if !env_map.is_empty() {
            let content = env_map.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("\n");
            if let Err(e) = mstore.agent_content_set(&AgentContent {
                agent_id: agent_id.to_string(),
                content_type: "env".to_string(),
                content,
                updated_at: now,
            }) {
                tracing::warn!(agent_id, err = %e, "agent.define: failed to persist env (non-fatal)");
            }
        }
    }
}

fn register_agent_define(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let id_store = state.id_store.clone();
    let broker = state.broker.clone();

    engine.register_handler(
        COMMAND_AGENT_DEFINE,
        Box::new(move |data, _ctx| {
            let mstore = mstore.clone();
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandAgentDefineData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.define: {e}"))?;
                agent_define_core(mstore, id_store, broker, cmd).await
                    .map(|r| Some(serde_json::to_value(&r).unwrap()))
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_base_url_is_always_valid() {
        // Even for a provider with no base_url_env_var — "unset" never needs
        // the capability to exist.
        assert!(validate_vendor_base_url("codex", "").is_ok());
        assert!(validate_vendor_base_url("claude", "").is_ok());
        assert!(validate_vendor_base_url("nonexistent-provider", "").is_ok());
    }

    #[test]
    fn non_empty_base_url_accepted_for_a_supporting_provider() {
        assert!(validate_vendor_base_url("claude", "https://my-proxy.example.com").is_ok());
    }

    #[test]
    fn non_empty_base_url_rejected_for_a_non_supporting_provider() {
        let err = validate_vendor_base_url("codex", "https://my-proxy.example.com").unwrap_err();
        assert!(err.contains("codex"));
        assert!(err.contains("does not support"));
    }

    #[test]
    fn non_empty_base_url_rejected_for_an_unknown_provider() {
        assert!(validate_vendor_base_url("not-a-real-provider", "https://x").is_err());
    }
}

// reagent P1 on PR #2505: agent.define's update path had no way to distinguish
// "don't touch model_vendor_base_url" from "clear it" (a plain empty String
// always meant "don't touch"). Combined with the authoritative validation
// re-running on every update against whatever is currently stored, a provider
// change away from a vendor-capable provider while an old override sat there
// permanently blocked every future agent.define call for that agent — there
// was no way to ever un-set the stale value. Fixed via Option<String>: `None`
// = don't touch, `Some("")` = explicit clear.
#[cfg(test)]
mod agent_define_core_vendor_tests {
    use super::*;
    use crate::server::tests::test_state;

    fn cmd(value: serde_json::Value) -> CommandAgentDefineData {
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn omitting_the_field_on_update_does_not_clear_a_previously_set_override() {
        let state = test_state();

        let created = agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "vendor-test-agent",
            "provider": "claude",
            "model_vendor_base_url": "https://my-proxy.example.com",
        }))).await.unwrap();
        let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
        assert_eq!(def.model_vendor_base_url, "https://my-proxy.example.com");

        // Update that touches an unrelated field and omits model_vendor_base_url.
        agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "vendor-test-agent",
            "if_exists": "update",
            "description": "just touching something else",
        }))).await.unwrap();
        let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
        assert_eq!(
            def.model_vendor_base_url, "https://my-proxy.example.com",
            "an omitted field must not clear the stored override"
        );
    }

    #[tokio::test]
    async fn explicit_empty_string_clears_a_previously_set_override() {
        let state = test_state();

        let created = agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "vendor-clear-test",
            "provider": "claude",
            "model_vendor_base_url": "https://my-proxy.example.com",
        }))).await.unwrap();

        agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "vendor-clear-test",
            "if_exists": "update",
            "model_vendor_base_url": "",
        }))).await.unwrap();
        let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
        assert_eq!(def.model_vendor_base_url, "", "explicit empty string must clear the override");
    }

    // The provider is read-only once set (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md
    // §3.1), so the old "change provider and clear a stale override in the
    // same call" path is now a refusal; the override itself still updates.
    #[tokio::test]
    async fn an_update_cannot_change_the_provider_but_can_change_the_rest() {
        let state = test_state();

        let created = agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "provider-lock-test",
            "provider": "claude",
            "model_vendor_base_url": "https://my-proxy.example.com",
        }))).await.unwrap();

        for change in [json!({ "provider": "codex" }), json!({ "model": "gemini-2.5-pro" })] {
            let mut req = json!({ "name": "provider-lock-test", "if_exists": "update" });
            req.as_object_mut().unwrap().extend(change.as_object().unwrap().clone());
            let err = agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(req))
                .await
                .unwrap_err();
            assert!(err.contains("readonly once set"), "{err}");
        }

        agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "provider-lock-test",
            "if_exists": "update",
            "provider": "claude",
            "model_vendor_base_url": "",
            "description": "updated",
        }))).await.unwrap();
        let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
        assert_eq!(def.provider, "claude");
        assert_eq!(def.model_vendor_base_url, "");
        assert_eq!(def.description, "updated");
    }

    #[test]
    fn check_provider_unchanged_allows_setting_an_empty_provider_once() {
        assert!(check_provider_unchanged("a", "", "codex").is_ok());
        assert!(check_provider_unchanged("a", "codex", "codex").is_ok());
        assert!(check_provider_unchanged("a", "codex", "claude").is_err());
    }
}

// Mandatory ABF (ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.2):
// a fresh `agent.define` call provisions its own dedicated bundle.
#[cfg(test)]
mod agent_define_core_bundle_provisioning_tests {
    use super::*;
    use crate::server::tests::test_state;

    fn cmd(value: serde_json::Value) -> CommandAgentDefineData {
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn fresh_insert_provisions_its_own_bundle_and_the_agent_keeps_the_provider() {
        let state = test_state();

        let created = agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "define-bundle-test",
            "provider": "gemini",
        }))).await.unwrap();

        let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
        assert!(!def.memory_id.is_empty(), "fresh agent.define insert must bind a bundle");
        let bundle = state.mstore.bundle_get(&def.memory_id).unwrap().unwrap();
        assert!(!bundle.is_blank);
        assert_eq!(def.provider, "gemini");
        assert_eq!((bundle.provider.as_str(), bundle.model.as_str()), ("", ""), "the bundle carries no harness");
    }

    #[tokio::test]
    async fn skip_of_an_existing_definition_does_not_leak_a_stray_bundle() {
        let state = test_state();

        let first = agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "skip-no-leak-test",
            "provider": "claude",
            "if_exists": "skip",
        }))).await.unwrap();
        assert_eq!(first.action, "created");
        let bundles_after_first = state.mstore.bundle_list().unwrap().len();

        // Same name again, if_exists=skip — must resolve to the SAME
        // definition without creating (and leaking) a second bundle. The
        // bundle-provision step only runs on agent_define_core's
        // genuinely-fresh-insert path, not the atomic find-or-insert's
        // own lookup, precisely so idempotent callers don't leak one
        // unbound bundle per repeated call.
        let second = agent_define_core(state.mstore.clone(), state.id_store.clone(), state.broker.clone(), cmd(json!({
            "name": "skip-no-leak-test",
            "provider": "claude",
            "if_exists": "skip",
        }))).await.unwrap();
        assert_eq!(second.action, "skipped");
        assert_eq!(second.definition_id, first.definition_id);
        let bundles_after_second = state.mstore.bundle_list().unwrap().len();
        assert_eq!(bundles_after_first, bundles_after_second, "skip path must not leak an extra bundle");
    }
}

/// Atomically allocate an agent working directory.
///
/// Tries to atomically create `desired` via `std::fs::create_dir`. If
/// that fails because the directory already exists, tries `<desired>-1`,
/// `<desired>-2`, …, up to `-99`. The atomic `create_dir` (NOT
/// `create_dir_all` for the leaf) is the reservation mechanism: two
/// concurrent callers competing for the same path race on the OS
/// `mkdir` syscall and one wins; the loser sees `AlreadyExists` and
/// moves on.
///
/// Caller is responsible for distinguishing auto-generated paths from
/// user-specified ones — this function rewrites the path on collision,
/// which would clobber a user's intent if they pointed an agent at
/// `~/projects/myrepo` and that already had a `CLAUDE.md`.
pub fn allocate_agent_workdir(desired: &str) -> Result<String, String> {
    let p = std::path::Path::new(desired);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("allocate_agent_workdir: parent {}: {e}", parent.display()))?;
        }
    }
    match std::fs::create_dir(p) {
        Ok(()) => return Ok(desired.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("allocate_agent_workdir: create_dir({}): {e}", desired)),
    }
    for n in 1..=99u32 {
        let candidate = format!("{desired}-{n}");
        match std::fs::create_dir(std::path::Path::new(&candidate)) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("allocate_agent_workdir: create_dir({candidate}): {e}")),
        }
    }
    Err(format!(
        "allocate_agent_workdir: too many collisions (>99) under {desired}-N — clean up old runs"
    ))
}

pub(crate) async fn agent_define_core(
    mstore: Arc<Store>,
    id_store: Arc<Store>,
    broker: Arc<crate::backend::mps::Broker>,
    cmd: CommandAgentDefineData,
) -> Result<AgentDefineResult, String> {
    if cmd.name.trim().is_empty() {
        return Err("agent.define: name is required".to_string());
    }

    // Validate if_exists early so a typo is caught even for new definitions,
    // not only when a matching definition already exists.
    let if_exists = cmd.if_exists.as_deref().unwrap_or("skip");
    if !matches!(if_exists, "skip" | "update" | "error") {
        return Err(format!(
            "agent.define: unknown if_exists value '{if_exists}'; valid: skip, update, error"
        ));
    }

    // Resolve provider: explicit `provider` wins; fall back to inference from
    // `model` prefix; default to "claude" when neither is supplied.
    let provider = if !cmd.provider.is_empty() {
        if providers::get_provider(&cmd.provider).is_none() {
            return Err(format!(
                "agent.define: unknown provider '{}'; valid: claude, codex, gemini, qwen, kimi, openclaw, pi, copilot",
                cmd.provider
            ));
        }
        cmd.provider.clone()
    } else if !cmd.model.is_empty() {
        let inferred = agent_define::infer_provider_from_model(&cmd.model);
        if providers::get_provider(&inferred).is_none() {
            return Err(format!(
                "agent.define: cannot infer provider from model '{}'; set provider explicitly",
                cmd.model
            ));
        }
        inferred
    } else {
        "claude".to_string()
    };

    let create_stub = cmd.create_instance_stub.unwrap_or(true);

    let now = agentmux_common::time::now_ms();

    // Gates the fresh-insert path below (the "no existing match" case, where
    // `def` — built from `provider` — is what actually gets written). The
    // "update" branch further down re-validates against the EXISTING
    // agent's actual provider (not this possibly-defaulted `provider`,
    // which may not reflect an unspecified `cmd.provider` on an update
    // call) right before its own write — this check doesn't gate that path.
    let cmd_model_vendor_base_url = cmd.model_vendor_base_url.clone().unwrap_or_default();
    agent_define::validate_vendor_base_url(&provider, &cmd_model_vendor_base_url)?;

    // Build the new definition struct up-front so agent_def_find_or_insert
    // can use it as both the lookup key and the insert payload.
    // agent_def_find_or_insert holds a single mutex guard for the check +
    // conditional insert — closing the TOCTOU window between list and insert.
    let mut def = AgentDefinition {
        id: uuid::Uuid::new_v4().to_string(),
        slug: String::new(), // resolved by agent_def_find_or_insert
        name: cmd.name.clone(),
        icon: cmd.icon.clone(),
        provider: provider.clone(),
        description: cmd.description.clone(),
        working_directory: cmd.working_directory.clone(),
        shell: cmd.shell.clone(),
        environment: cmd.environment.clone(),
        // Persist the requested model as a CLI flag so the agent launches
        // with the specified model rather than the provider default.
        provider_flags: if cmd.model.is_empty() {
            String::new()
        } else {
            format!("--model {}", cmd.model)
        },
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: now,
        agent_type: cmd.agent_type.clone(),
        agent_bus_id: String::new(),
        is_seeded: 0,
        accounts: String::new(),
        parent_id: String::new(),
        branch_label: String::new(),
        updated_at: now,
        user_hidden: 0,
        container_image: cmd.container_image.clone(),
        container_volumes: cmd.container_volumes.clone(),
        container_name: String::new(), // assigned by ContainerManager on first spawn
        use_ambient_login: 0,
        model_vendor_base_url: cmd_model_vendor_base_url.clone(),
        auto_continue_enabled: 0,
        memory_id: String::new(),
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
    };

    // Atomic check-then-insert.
    // Returns Some(existing) if a row matched by name/slug already exists;
    // None if the row was freshly inserted (def.slug now holds resolved slug).
    let existing_opt = mstore.agent_def_find_or_insert(&mut def)
        .map_err(|e| format!("agent.define: find_or_insert: {e}"))?;

    if let Some(existing) = existing_opt {
        // A definition with this name/slug already exists — apply if_exists policy.
        match if_exists {
            "skip" => {
                // Honor create_instance_stub even on skip: a definition that was
                // created with create_instance_stub=false (or imported via another
                // path) might not have a stub yet; a subsequent idempotent call
                // with create_instance_stub=true should make it visible in My Agents.
                // Only fire agents:changed when the stub was actually newly inserted.
                let (stub_id, stub_new) = if create_stub {
                    match agent_define::make_stub_idempotent(&mstore, &existing.id, &existing.name, now) {
                        Ok((id, new)) => (Some(id), new),
                        Err(e) => {
                            tracing::warn!(id = %existing.id, err = %e, "agent.define: skip stub failed (non-fatal)");
                            (None, false)
                        }
                    }
                } else {
                    (None, false)
                };
                if stub_new {
                    broker.publish(crate::backend::mps::MuxEvent {
                        event: "agents:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                tracing::info!(id = %existing.id, slug = %existing.slug, stub = stub_id.is_some(), "agent.define: skipped (exists)");
                return Ok(AgentDefineResult {
                    definition_id: existing.id.clone(),
                    slug: existing.slug.clone(),
                    action: "skipped".to_string(),
                    instance_stub_id: stub_id,
                });
            }
            "error" => {
                return Err(format!(
                    "agent.define: definition '{}' already exists (if_exists=error)",
                    cmd.name.trim()
                ));
            }
            "update" => {
                let mut updated = existing.clone();
                // provider was already validated/defaulted above; only
                // overwrite if the caller explicitly supplied a provider or model.
                if !cmd.provider.is_empty() || !cmd.model.is_empty() {
                    agent_define::check_provider_unchanged(&existing.id, &existing.provider, &provider)?;
                    updated.provider = provider.clone();
                }
                // Persist the model as a CLI flag so the agent launches with
                // the requested model rather than the provider default.
                // If the provider changes but no model is supplied, clear stale
                // flags from the old provider so the new provider's default is used.
                if !cmd.model.is_empty() {
                    updated.provider_flags = format!("--model {}", cmd.model);
                } else if !cmd.provider.is_empty() {
                    updated.provider_flags = String::new();
                }
                if !cmd.icon.is_empty()     { updated.icon = cmd.icon.clone(); }
                if !cmd.description.is_empty() { updated.description = cmd.description.clone(); }
                if !cmd.working_directory.is_empty() { updated.working_directory = cmd.working_directory.clone(); }
                if !cmd.shell.is_empty()    { updated.shell = cmd.shell.clone(); }
                // `None` = don't touch; `Some(_)` (including `Some("")`) sets
                // it explicitly — the caller MUST be able to pass `Some("")`
                // to clear a stale override, or a provider change away from
                // a vendor-capable provider (see validation below) would
                // permanently block every future agent.define call for this
                // agent, since there'd be no way to ever un-set the old value.
                if let Some(url) = &cmd.model_vendor_base_url { updated.model_vendor_base_url = url.clone(); }
                // Authoritative check for this write: validates the FINAL
                // effective (provider, override) pair — catches both a
                // freshly-supplied override against the real provider, and a
                // provider change that leaves a stale override from before
                // now invalid (the caller must clear it explicitly rather
                // than silently carrying an inconsistent combination).
                agent_define::validate_vendor_base_url(&updated.provider, &updated.model_vendor_base_url)?;
                if !cmd.environment.is_empty() { updated.environment = cmd.environment.clone(); }
                // name update intentionally omitted — the slug is immutable;
                // renaming would create a slug mismatch. Use updateagent for renames.
                let did_update = mstore.agent_def_update(&mut updated)
                    .map_err(|e| format!("agent.define: update: {e}"))?;
                if !did_update {
                    return Err("agent.define: update: row was deleted between find and update".to_string());
                }
                agent_define::persist_define_content(&mstore, &updated.id, &cmd, now);
                let stub_id = if create_stub {
                    match agent_define::make_stub_idempotent(&mstore, &updated.id, &updated.name, now) {
                        Ok((id, _new)) => Some(id),
                        Err(e) => {
                            tracing::warn!(id = %updated.id, err = %e, "agent.define: update stub failed (non-fatal)");
                            None
                        }
                    }
                } else {
                    None
                };
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                tracing::info!(id = %updated.id, slug = %updated.slug, stub = stub_id.is_some(), "agent.define: updated");
                return Ok(AgentDefineResult {
                    definition_id: updated.id.clone(),
                    slug: updated.slug.clone(),
                    action: "updated".to_string(),
                    instance_stub_id: stub_id,
                });
            }
            other => {
                return Err(format!("agent.define: unknown if_exists value '{other}'"));
            }
        }
    }

    // Fresh insert — def.slug is now set by agent_def_find_or_insert.
    // Every agent gets its own dedicated ABF bundle
    // (ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.2). Done here,
    // after the atomic find-or-insert has confirmed this is a genuinely
    // NEW definition — not before, or every idempotent `if_exists=skip`/
    // `update` call against an existing name would leak an unbound bundle
    // (see `agent_def_provision_and_bind_bundle`'s own doc comment).
    mstore.agent_def_provision_and_bind_bundle(&id_store, &mut def, now);
    // Create the stub first so that listeners handling agents:changed can
    // immediately find the new agent via ListRecentSessionsCommand. The
    // definition is already committed; a stub failure is non-fatal (log +
    // continue) and we still broadcast so callers see the new definition.
    let stub_id = if create_stub {
        match agent_define::make_stub_idempotent(&mstore, &def.id, &def.name, now) {
            Ok((id, _new)) => Some(id),
            Err(e) => {
                tracing::warn!(id = %def.id, err = %e, "agent.define: stub failed (definition committed, non-fatal)");
                None
            }
        }
    } else {
        None
    };
    broker.publish(crate::backend::mps::MuxEvent {
        event: "agents:changed".to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: None,
    });
    agent_define::persist_define_content(&mstore, &def.id, &cmd, now);

    tracing::info!(
        id = %def.id,
        slug = %def.slug,
        stub = stub_id.is_some(),
        "agent.define: created"
    );

    Ok(AgentDefineResult {
        definition_id: def.id.clone(),
        slug: def.slug.clone(),
        action: "created".to_string(),
        instance_stub_id: stub_id,
    })
}
