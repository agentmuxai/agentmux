// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use chrono::Utc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_AGENTS, COMMAND_CREATE_AGENT, COMMAND_UPDATE_AGENT,
    COMMAND_DELETE_AGENT, COMMAND_GET_AGENT_CONTENT, COMMAND_SET_AGENT_CONTENT,
    COMMAND_GET_ALL_AGENT_CONTENT,
    COMMAND_LIST_AGENT_SKILLS, COMMAND_CREATE_AGENT_SKILL, COMMAND_UPDATE_AGENT_SKILL,
    COMMAND_DELETE_AGENT_SKILL,
    COMMAND_APPEND_AGENT_HISTORY, COMMAND_LIST_AGENT_HISTORY, COMMAND_SEARCH_AGENT_HISTORY,
    COMMAND_IMPORT_AGENT_FROM_CLAW, COMMAND_IMPORT_AGENTS, COMMAND_EXPORT_AGENTS,
    COMMAND_RESEED_AGENTS,
    // Two-tier picker — Phase 1 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md)
    COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
    COMMAND_CONTAINER_RUNTIME_AVAILABLE,
    CommandAgentDefCreateFromTemplateData, AgentDefCreateFromTemplateResult,
    CommandListAgentDefinitionsData,
    // Two-tier picker — Phase 2 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md
    // Q2 Decision Y: hide templates).
    COMMAND_AGENT_DEF_HIDE, COMMAND_AGENT_DEF_UNHIDE,
    COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES,
    CommandAgentDefHideData, AgentDefHideResult,
    CommandCreateAgentDefinitionData, CommandUpdateAgentDefinitionData, CommandDeleteAgentDefinitionData,
    CommandGetAgentContentData, CommandSetAgentContentData, CommandGetAllAgentContentData,
    CommandListAgentSkillsData, CommandCreateAgentSkillData, CommandUpdateAgentSkillData,
    CommandDeleteAgentSkillData,
    CommandAppendAgentHistoryData, CommandListAgentHistoryData, CommandSearchAgentHistoryData,
    CommandImportAgentFromClawData,
    CommandImportAgentDefinitionsData, ImportAgentDefinitionsResult,
    ExportAgentDefinitionsResult, AgentDefinitionExport, AgentSkillExport,
    // v6 identity / instance / fork
    COMMAND_LIST_IDENTITY_ACCOUNTS, COMMAND_GET_IDENTITY_ACCOUNT,
    COMMAND_UPSERT_IDENTITY_ACCOUNT, COMMAND_DELETE_IDENTITY_ACCOUNT,
    COMMAND_ACCOUNT_KEY_VERIFY,
    COMMAND_ACCOUNT_OAUTH_START, COMMAND_ACCOUNT_OAUTH_POLL, COMMAND_ACCOUNT_OAUTH_CANCEL,
    COMMAND_LINK_AGENT_IDENTITY, COMMAND_UNLINK_AGENT_IDENTITY,
    COMMAND_LIST_AGENT_IDENTITIES,
    COMMAND_LIST_AGENT_INSTANCES, COMMAND_GET_AGENT_INSTANCE,
    COMMAND_CREATE_AGENT_INSTANCE, COMMAND_UPDATE_AGENT_INSTANCE,
    COMMAND_DELETE_AGENT_INSTANCE,
    COMMAND_LIST_NAMED_AGENTS, COMMAND_HIDE_NAMED_AGENT,
    CommandListNamedAgentsData, CommandHideNamedAgentData,
    NamedAgentRow,
    COMMAND_LIST_RECENT_SESSIONS, CommandListRecentSessionsData,
    RecentSessionRow,
    // Option E (PR 1 of 2) — agent-anchored session zones.
    COMMAND_AGENT_SESSION_READ, COMMAND_AGENT_SESSION_WRITE_STATE,
    COMMAND_AGENT_SESSION_APPEND_OUTPUT, COMMAND_AGENT_SESSION_ARCHIVE,
    COMMAND_AGENT_SESSION_LIST_ARCHIVES,
    CommandAgentSessionReadData, AgentSessionReadResult,
    CommandAgentSessionWriteStateData, AgentSessionWriteStateResult,
    CommandAgentSessionAppendOutputData, AgentSessionAppendOutputResult,
    CommandAgentSessionArchiveData, AgentSessionArchiveResult,
    CommandAgentSessionListArchivesData, AgentArchiveRow,
    COMMAND_FORK_AGENT_DEFINITION,
    COMMAND_FORK_AGENT_DEFINITION_SUGGEST,
    CommandForkAgentDefinitionSuggestData, ForkAgentDefinitionSuggestResult,
    CommandListIdentityAccountsData, CommandGetIdentityAccountData,
    CommandDeleteIdentityAccountData,
    CommandLinkAgentIdentityData, CommandUnlinkAgentIdentityData,
    CommandListAgentIdentitiesData,
    CommandListAgentInstancesData, CommandGetAgentInstanceData,
    CommandCreateAgentInstanceData, CommandUpdateAgentInstanceData,
    CommandDeleteAgentInstanceData,
    CommandForkAgentDefinitionData,
    // v7 Identity bundles + Memory
    COMMAND_LIST_IDENTITY_BUNDLES, COMMAND_GET_IDENTITY_BUNDLE,
    COMMAND_UPSERT_IDENTITY_BUNDLE, COMMAND_DELETE_IDENTITY_BUNDLE,
    COMMAND_BIND_IDENTITY_ACCOUNT, COMMAND_UNBIND_IDENTITY_ACCOUNT,
    COMMAND_LIST_IDENTITY_BINDINGS,
    COMMAND_LIST_MEMORIES, COMMAND_GET_MEMORY,
    COMMAND_UPSERT_MEMORY, COMMAND_DELETE_MEMORY, COMMAND_REORDER_GLOBAL_BRAIN,
    CommandGetIdentityBundleData, CommandDeleteIdentityBundleData,
    CommandBindIdentityAccountData, CommandUnbindIdentityAccountData,
    CommandListIdentityBindingsData,
    CommandGetMemoryData, CommandDeleteMemoryData, CommandReorderGlobalBrainData,
};
use crate::backend::storage::{AgentDefinition, AgentContent, AgentSkill};
use crate::backend::storage::store::{
    AgentInstance, Identity, IdentityAccount, InstanceStatus, Memory, SecretRef,
};
use crate::backend::rpc_types::{
    COMMAND_SUBPROCESS_SPAWN, COMMAND_AGENT_INPUT, COMMAND_AGENT_STOP,
    CommandSubprocessSpawnData, CommandAgentInputData, CommandAgentStopData,
};
use crate::backend::obj::Block;
use crate::backend::blockcontroller;

use super::AppState;

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

pub fn register_agent_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // listagents → return all agent definitions, optionally filtered by
    // `is_seeded`. Filter input is backward-compatible: callers that
    // pass `null` / `{}` (every existing caller) get the full list.
    // The two-tier picker (Phase 1) passes `{ is_seeded: 0 }` for the
    // "My Agents" section and `{ is_seeded: 1 }` for the "Templates"
    // section — see SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md.
    //
    // Phase 2 (Q2 Decision Y — hide templates): templates with
    // `user_hidden = 1` are filtered out by default. Callers that want
    // them back (the settings panel's "Hidden templates" surface) pass
    // `include_hidden: true`. Hide filter applies ONLY to templates —
    // user-owned definitions are unaffected (their `user_hidden` is
    // always 0 by backend invariant; `agent_def_set_hidden` rejects
    // non-template ids).
    let wstore_lfa = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfa.clone();
            Box::pin(async move {
                // unwrap_or_default — both `null` and `{}` deserialize
                // to the default (no filter). Anything malformed falls
                // back to no-filter rather than erroring; older clients
                // never sent a body for this RPC and we can't know
                // which JSON shape they're on.
                let cmd: CommandListAgentDefinitionsData =
                    serde_json::from_value(data).unwrap_or_default();
                let agents = wstore.agent_def_list().map_err(|e| format!("listagents: {e}"))?;
                let is_seeded_filter = cmd.is_seeded;
                let include_hidden = cmd.include_hidden;
                let filtered: Vec<_> = agents
                    .into_iter()
                    .filter(|a| match is_seeded_filter {
                        Some(flag) => a.is_seeded == flag,
                        None => true,
                    })
                    // Default behaviour: drop hidden templates. The
                    // settings panel opts back in with include_hidden.
                    // User-owned rows (is_seeded == 0) are never
                    // hideable; the conditional below is a no-op for
                    // them.
                    .filter(|a| {
                        include_hidden || a.is_seeded != 1 || a.user_hidden == 0
                    })
                    .collect();
                Ok(Some(serde_json::to_value(&filtered).unwrap_or_default()))
            })
        }),
    );

    // createagent → insert new agent, broadcast agents:changed
    let wstore_cfa = state.wstore.clone();
    let broker_cfa = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_cfa.clone();
            let broker = broker_cfa.clone();
            Box::pin(async move {
                let cmd: CommandCreateAgentDefinitionData = serde_json::from_value(data)
                    .map_err(|e| format!("createagent: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                // slug is empty here — agent_def_insert auto-derives it
                // from name AND collision-resolves AND mutates the
                // struct so we serialize the resolved value back to
                // the frontend (not "").
                let mut agent = AgentDefinition {
                    id: uuid::Uuid::new_v4().to_string(),
                    slug: String::new(),
                    name: cmd.name,
                    icon: cmd.icon,
                    provider: cmd.provider,
                    description: cmd.description,
                    working_directory: cmd.working_directory,
                    shell: cmd.shell,
                    provider_flags: cmd.provider_flags,
                    auto_start: cmd.auto_start,
                    restart_on_crash: cmd.restart_on_crash,
                    idle_timeout_minutes: cmd.idle_timeout_minutes,
                    created_at: now,
                    agent_type: cmd.agent_type,
                    environment: cmd.environment,
                    agent_bus_id: cmd.agent_bus_id,
                    is_seeded: 0,
                    accounts: String::new(),
                    parent_id: String::new(),
                    branch_label: String::new(),
                    updated_at: now,
                    user_hidden: 0,
                    container_image: String::new(),
                    container_volumes: "[]".to_string(),
                    container_name: String::new(),
                };
                wstore.agent_def_insert(&mut agent).map_err(|e| format!("createagent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // updateagent → update existing agent, broadcast agents:changed
    let wstore_ufa = state.wstore.clone();
    let broker_ufa = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ufa.clone();
            let broker = broker_ufa.clone();
            Box::pin(async move {
                let cmd: CommandUpdateAgentDefinitionData = serde_json::from_value(data)
                    .map_err(|e| format!("updateagent: {e}"))?;
                // Fetch existing to preserve created_at
                let existing = wstore.agent_def_list().map_err(|e| format!("updateagent: {e}"))?;
                let old = existing.iter().find(|a| a.id == cmd.id)
                    .ok_or_else(|| format!("updateagent: agent {} not found", cmd.id))?;
                // slug is preserved from the existing row — it's
                // immutable after creation. The update path never
                // accepts a new slug from the client.
                let mut agent = AgentDefinition {
                    id: cmd.id,
                    slug: old.slug.clone(),
                    name: cmd.name,
                    icon: cmd.icon,
                    provider: cmd.provider,
                    description: cmd.description,
                    working_directory: cmd.working_directory,
                    shell: cmd.shell,
                    provider_flags: cmd.provider_flags,
                    auto_start: cmd.auto_start,
                    restart_on_crash: cmd.restart_on_crash,
                    idle_timeout_minutes: cmd.idle_timeout_minutes,
                    created_at: old.created_at,
                    agent_type: cmd.agent_type,
                    environment: cmd.environment,
                    agent_bus_id: cmd.agent_bus_id,
                    is_seeded: old.is_seeded,
                    // Preserve existing accounts when the caller omits the field
                    // (cmd.accounts defaults to "" via #[serde(default)]). Callers
                    // that only update name/icon/etc. (AgentDefForm, AgentPicker rename)
                    // don't carry accounts, so falling back to old.accounts prevents
                    // silently wiping saved assignments.
                    accounts: if cmd.accounts.is_empty() { old.accounts.clone() } else { cmd.accounts },
                    // parent_id + branch_label describe provenance and
                    // are immutable post-insert (forks are separate rows,
                    // not in-place edits).
                    parent_id: old.parent_id.clone(),
                    branch_label: old.branch_label.clone(),
                    // Placeholder — agent_def_update self-stamps the real
                    // timestamp and writes it back into `agent` below, so
                    // the response body carries the fresh value.
                    updated_at: old.updated_at,
                    // Preserve user_hidden — updateagent edits the
                    // definition payload, not the per-user view-state
                    // flag. Hide/unhide go through their dedicated RPCs
                    // (`agentdefhide` / `agentdefunhide`). Phase 2 of
                    // SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md.
                    user_hidden: old.user_hidden,
                    // Preserve existing container config when the caller omits the field
                    // (cmd.container_image defaults to "" via #[serde(default)]). Callers
                    // that only update name/icon/etc. (AgentDefForm) don't carry container
                    // fields, so falling back to old values prevents silently wiping a
                    // container agent's image and volumes — same guard as `accounts` above.
                    container_image: if cmd.container_image.is_empty() { old.container_image.clone() } else { cmd.container_image },
                    container_volumes: if cmd.container_volumes == "[]" { old.container_volumes.clone() } else { cmd.container_volumes },
                    // container_name is server-managed; preserve the existing value.
                    container_name: old.container_name.clone(),
                };
                let found = wstore.agent_def_update(&mut agent).map_err(|e| format!("updateagent: {e}"))?;
                if !found {
                    return Err(format!("updateagent: agent {} not found", agent.id));
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // deleteagent → delete agent by id, broadcast agents:changed
    let wstore_dfa = state.wstore.clone();
    let broker_dfa = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dfa.clone();
            let broker = broker_dfa.clone();
            Box::pin(async move {
                let cmd: CommandDeleteAgentDefinitionData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteagent: {e}"))?;
                wstore.agent_def_delete(&cmd.id).map_err(|e| format!("deleteagent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    // agentdefcreatefromtemplate → clone a seeded template into a new
    // user-owned definition (Phase 1 two-tier picker —
    // SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md). The template stays
    // pristine; the new row carries `is_seeded = 0`. Returns the new
    // definition_id so the frontend can immediately launch.
    //
    // Validation rules:
    //  - `template_id` MUST resolve to a row with `is_seeded = 1`.
    //    Cloning a user-owned row would be confusing semantics — use
    //    the existing `forkagentdefinition` RPC for that case.
    //  - `name` non-empty, ≤200 chars, and not already taken by any
    //    `is_seeded = 0` row. Avoids collisions in the picker's
    //    "My Agents" list.
    let wstore_act = state.wstore.clone();
    let broker_act = state.broker.clone();
    engine.register_handler(
        COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_act.clone();
            let broker = broker_act.clone();
            Box::pin(async move {
                let cmd: CommandAgentDefCreateFromTemplateData = serde_json::from_value(data)
                    .map_err(|e| format!("agentdefcreatefromtemplate: {e}"))?;
                let name = cmd.name.trim().to_string();
                if name.is_empty() {
                    return Err("agentdefcreatefromtemplate: name must be non-empty".into());
                }
                if name.chars().count() > 200 {
                    return Err(
                        "agentdefcreatefromtemplate: name must be ≤200 characters".into(),
                    );
                }

                let all = wstore
                    .agent_def_list()
                    .map_err(|e| format!("agentdefcreatefromtemplate: list: {e}"))?;
                let template = all
                    .iter()
                    .find(|a| a.id == cmd.template_id)
                    .ok_or_else(|| {
                        format!(
                            "agentdefcreatefromtemplate: template {} not found",
                            cmd.template_id
                        )
                    })?;
                if template.is_seeded != 1 {
                    return Err(format!(
                        "agentdefcreatefromtemplate: {} is not a seeded template (is_seeded={})",
                        cmd.template_id, template.is_seeded
                    ));
                }
                if all
                    .iter()
                    .any(|a| a.is_seeded == 0 && a.name.eq_ignore_ascii_case(&name))
                {
                    return Err(format!(
                        "agentdefcreatefromtemplate: an agent named {:?} already exists",
                        name
                    ));
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                // Runtime is the user's instantiation-time choice, not a
                // template property. When supplied, the clone records it
                // (and the matching `environment`); empty falls back to
                // the template's value for back-compat with older callers.
                let chosen_agent_type = match cmd.agent_type.trim() {
                    "host" | "container" => cmd.agent_type.trim().to_string(),
                    _ => template.agent_type.clone(),
                };
                let chosen_environment = if chosen_agent_type == "container" {
                    "docker".to_string()
                } else {
                    "local".to_string()
                };
                let mut new_def = AgentDefinition {
                    id: uuid::Uuid::new_v4().to_string(),
                    // agent_def_insert derives a unique slug from the
                    // name when this is empty + collision-resolves.
                    slug: String::new(),
                    name: name.clone(),
                    icon: template.icon.clone(),
                    provider: template.provider.clone(),
                    description: template.description.clone(),
                    // Force re-allocation of the per-agent working
                    // directory at first launch via the new slug —
                    // matches forkagentdefinition's behaviour.
                    working_directory: String::new(),
                    shell: template.shell.clone(),
                    provider_flags: template.provider_flags.clone(),
                    // Users opt in to auto-start explicitly; cloning
                    // shouldn't carry it over (mirrors fork).
                    auto_start: 0,
                    restart_on_crash: template.restart_on_crash,
                    idle_timeout_minutes: template.idle_timeout_minutes,
                    created_at: now,
                    agent_type: chosen_agent_type,
                    environment: chosen_environment,
                    agent_bus_id: String::new(),
                    is_seeded: 0,
                    accounts: String::new(),
                    parent_id: template.id.clone(),
                    branch_label: String::new(),
                    updated_at: now,
                    // New user-owned agent starts visible. Phase 2
                    // (Q2 Decision Y) — hide applies only to seeded
                    // templates, never to user-owned agents.
                    user_hidden: 0,
                    // Inherit container config from template so container-type
                    // templates propagate their image to user-cloned agents.
                    container_image: template.container_image.clone(),
                    container_volumes: template.container_volumes.clone(),
                    container_name: String::new(),
                };
                wstore
                    .agent_def_insert(&mut new_def)
                    .map_err(|e| format!("agentdefcreatefromtemplate: insert: {e}"))?;

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });

                let resp = AgentDefCreateFromTemplateResult {
                    definition_id: new_def.id.clone(),
                    identity_id: cmd.identity_id,
                    memory_id: cmd.memory_id,
                };
                tracing::info!(
                    template_id = %cmd.template_id,
                    new_definition_id = %new_def.id,
                    new_name = %new_def.name,
                    "agentdefcreatefromtemplate: cloned template into user agent"
                );
                Ok(Some(serde_json::to_value(&resp).unwrap_or_default()))
            })
        }),
    );

    // containerruntimeavailable → does the Docker DAEMON answer a ping
    // right now? Returns `{ available: bool }`. The create-from-template
    // modal uses this to gate/default the container runtime. Distinct
    // from `resolvecli docker`, which only confirms the CLI binary is on
    // PATH — that false-positives when Docker is installed but the daemon
    // is stopped, steering users into a container agent that can't start
    // (codex P1 on #1576). `None` manager (Docker absent at startup) →
    // false; `Some` → live `check_available()` ping so a daemon that
    // came up or went down since launch is reflected.
    let container_manager_cra = state.container_manager.clone();
    engine.register_handler(
        COMMAND_CONTAINER_RUNTIME_AVAILABLE,
        Box::new(move |_data, _ctx| {
            let cm = container_manager_cra.clone();
            Box::pin(async move {
                let available = match cm {
                    Some(mgr) => mgr.check_available().await.is_ok(),
                    None => false,
                };
                Ok(Some(serde_json::json!({ "available": available })))
            })
        }),
    );

    // agentdefhide → set user_hidden = 1 on a seeded template, so it
    // disappears from the picker's "+ New from template" tier. Phase 2
    // (Q2 Decision Y) of SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md.
    //
    // Validation:
    //  - `definition_id` MUST exist. Missing → returns `{ ok: false }`.
    //  - The row MUST be a seeded template (`is_seeded = 1`). User-owned
    //    rows reject with a hard error — they have their own delete path
    //    and a hide flag on them would be misleading.
    //
    // Broadcasts `agents:changed` so the picker refetches and the card
    // disappears (existing list query already excludes hidden by default).
    let wstore_hide = state.wstore.clone();
    let broker_hide = state.broker.clone();
    engine.register_handler(
        COMMAND_AGENT_DEF_HIDE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_hide.clone();
            let broker = broker_hide.clone();
            Box::pin(async move {
                let cmd: CommandAgentDefHideData = serde_json::from_value(data)
                    .map_err(|e| format!("agentdefhide: {e}"))?;
                let ok = wstore
                    .agent_def_set_hidden(&cmd.definition_id, true)
                    .map_err(|e| format!("agentdefhide: {e}"))?;
                if ok {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "agents:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                    tracing::info!(
                        definition_id = %cmd.definition_id,
                        "agentdefhide: hid template"
                    );
                }
                let resp = AgentDefHideResult { ok };
                Ok(Some(serde_json::to_value(&resp).unwrap_or_default()))
            })
        }),
    );

    // agentdefunhide → set user_hidden = 0 on a seeded template,
    // bringing it back into the picker. Same validation + broadcast as
    // agentdefhide. Phase 2 of the two-tier picker spec.
    let wstore_unhide = state.wstore.clone();
    let broker_unhide = state.broker.clone();
    engine.register_handler(
        COMMAND_AGENT_DEF_UNHIDE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_unhide.clone();
            let broker = broker_unhide.clone();
            Box::pin(async move {
                let cmd: CommandAgentDefHideData = serde_json::from_value(data)
                    .map_err(|e| format!("agentdefunhide: {e}"))?;
                let ok = wstore
                    .agent_def_set_hidden(&cmd.definition_id, false)
                    .map_err(|e| format!("agentdefunhide: {e}"))?;
                if ok {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "agents:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                    tracing::info!(
                        definition_id = %cmd.definition_id,
                        "agentdefunhide: unhid template"
                    );
                }
                let resp = AgentDefHideResult { ok };
                Ok(Some(serde_json::to_value(&resp).unwrap_or_default()))
            })
        }),
    );

    // agentdeflisthiddentemplates → templates the user has hidden
    // (is_seeded = 1 AND user_hidden = 1). Used by the settings panel
    // to render the unhide list. The picker proper never calls this —
    // it uses `listagents` with the default-filter-out behaviour.
    let wstore_lh = state.wstore.clone();
    engine.register_handler(
        COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_lh.clone();
            Box::pin(async move {
                let agents = wstore
                    .agent_def_list()
                    .map_err(|e| format!("agentdeflisthiddentemplates: {e}"))?;
                let hidden: Vec<_> = agents
                    .into_iter()
                    .filter(|a| a.is_seeded == 1 && a.user_hidden == 1)
                    .collect();
                Ok(Some(serde_json::to_value(&hidden).unwrap_or_default()))
            })
        }),
    );

    // getagentcontent → return a single content blob for an agent
    let wstore_gfc = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_AGENT_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gfc.clone();
            Box::pin(async move {
                let cmd: CommandGetAgentContentData = serde_json::from_value(data)
                    .map_err(|e| format!("getagentcontent: {e}"))?;
                let content = wstore.agent_content_get(&cmd.agent_id, &cmd.content_type)
                    .map_err(|e| format!("getagentcontent: {e}"))?;
                Ok(content.map(|c| serde_json::to_value(&c).unwrap_or_default()))
            })
        }),
    );

    // setagentcontent → upsert a content blob, broadcast agentcontent:changed
    let wstore_sfc = state.wstore.clone();
    let broker_sfc = state.broker.clone();
    engine.register_handler(
        COMMAND_SET_AGENT_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfc.clone();
            let broker = broker_sfc.clone();
            Box::pin(async move {
                let cmd: CommandSetAgentContentData = serde_json::from_value(data)
                    .map_err(|e| format!("setagentcontent: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let content = AgentContent {
                    agent_id: cmd.agent_id,
                    content_type: cmd.content_type,
                    content: cmd.content,
                    updated_at: now,
                };
                wstore.agent_content_set(&content).map_err(|e| format!("setagentcontent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agentcontent:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&content).unwrap_or_default()))
            })
        }),
    );

    // getallagentcontent → return all content blobs for an agent
    let wstore_gafc = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_ALL_AGENT_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gafc.clone();
            Box::pin(async move {
                let cmd: CommandGetAllAgentContentData = serde_json::from_value(data)
                    .map_err(|e| format!("getallagentcontent: {e}"))?;
                let contents = wstore.agent_content_get_all(&cmd.agent_id)
                    .map_err(|e| format!("getallagentcontent: {e}"))?;
                Ok(Some(serde_json::to_value(&contents).unwrap_or_default()))
            })
        }),
    );

    // ── Agent Skills handlers ──────────────────────────────────────────────

    // listagentskills → return all skills for an agent
    let wstore_lfs = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_SKILLS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfs.clone();
            Box::pin(async move {
                let cmd: CommandListAgentSkillsData = serde_json::from_value(data)
                    .map_err(|e| format!("listagentskills: {e}"))?;
                let skills = wstore.agent_skill_list(&cmd.agent_id)
                    .map_err(|e| format!("listagentskills: {e}"))?;
                Ok(Some(serde_json::to_value(&skills).unwrap_or_default()))
            })
        }),
    );

    // createagentskill → insert new skill, broadcast agentskills:changed
    let wstore_cfs = state.wstore.clone();
    let broker_cfs = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_AGENT_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_cfs.clone();
            let broker = broker_cfs.clone();
            Box::pin(async move {
                let cmd: CommandCreateAgentSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("createagentskill: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let skill = AgentSkill {
                    id: uuid::Uuid::new_v4().to_string(),
                    agent_id: cmd.agent_id,
                    name: cmd.name,
                    trigger: cmd.trigger,
                    skill_type: cmd.skill_type,
                    description: cmd.description,
                    content: cmd.content,
                    created_at: now,
                };
                wstore.agent_skill_insert(&skill).map_err(|e| format!("createagentskill: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&skill).unwrap_or_default()))
            })
        }),
    );

    // updateagentskill → update existing skill, broadcast agentskills:changed
    let wstore_ufs = state.wstore.clone();
    let broker_ufs = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_AGENT_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ufs.clone();
            let broker = broker_ufs.clone();
            Box::pin(async move {
                let cmd: CommandUpdateAgentSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("updateagentskill: {e}"))?;
                let existing = wstore.agent_skill_get(&cmd.id)
                    .map_err(|e| format!("updateagentskill: {e}"))?
                    .ok_or_else(|| format!("updateagentskill: skill {} not found", cmd.id))?;
                let skill = AgentSkill {
                    id: cmd.id,
                    agent_id: existing.agent_id,
                    name: cmd.name,
                    trigger: cmd.trigger,
                    skill_type: cmd.skill_type,
                    description: cmd.description,
                    content: cmd.content,
                    created_at: existing.created_at,
                };
                let found = wstore.agent_skill_update(&skill).map_err(|e| format!("updateagentskill: {e}"))?;
                if !found {
                    return Err(format!("updateagentskill: skill {} not found", skill.id));
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&skill).unwrap_or_default()))
            })
        }),
    );

    // deleteagentskill → delete skill by id, broadcast agentskills:changed
    let wstore_dfs = state.wstore.clone();
    let broker_dfs = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_AGENT_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dfs.clone();
            let broker = broker_dfs.clone();
            Box::pin(async move {
                let cmd: CommandDeleteAgentSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteagentskill: {e}"))?;
                wstore.agent_skill_delete(&cmd.id).map_err(|e| format!("deleteagentskill: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    // ── Agent History handlers ─────────────────────────────────────────────

    // appendagenthistory → append a history entry, broadcast agenthistory:changed
    let wstore_afh = state.wstore.clone();
    let broker_afh = state.broker.clone();
    engine.register_handler(
        COMMAND_APPEND_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_afh.clone();
            let broker = broker_afh.clone();
            Box::pin(async move {
                let cmd: CommandAppendAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("appendagenthistory: {e}"))?;
                let entry = wstore.agent_history_append(&cmd.agent_id, &cmd.entry)
                    .map_err(|e| format!("appendagenthistory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agenthistory:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&entry).unwrap_or_default()))
            })
        }),
    );

    // listagenthistory → return history entries with pagination
    let wstore_lfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfh.clone();
            Box::pin(async move {
                let cmd: CommandListAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("listagenthistory: {e}"))?;
                let entries = wstore.agent_history_list(
                    &cmd.agent_id,
                    cmd.session_date.as_deref(),
                    cmd.limit,
                    cmd.offset,
                ).map_err(|e| format!("listagenthistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

    // searchagenthistory → search history entries by query
    let wstore_sfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_SEARCH_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfh.clone();
            Box::pin(async move {
                let cmd: CommandSearchAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("searchagenthistory: {e}"))?;
                let entries = wstore.agent_history_search(&cmd.agent_id, &cmd.query, cmd.limit)
                    .map_err(|e| format!("searchagenthistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

    // ── Agent Import handler ───────────────────────────────────────────────

    // importagentfromclaw → read claw workspace, create agent + content
    let wstore_ifc = state.wstore.clone();
    let broker_ifc = state.broker.clone();
    engine.register_handler(
        COMMAND_IMPORT_AGENT_FROM_CLAW,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ifc.clone();
            let broker = broker_ifc.clone();
            Box::pin(async move {
                let cmd: CommandImportAgentFromClawData = serde_json::from_value(data)
                    .map_err(|e| format!("importagentfromclaw: {e}"))?;

                let workspace_path = std::path::Path::new(&cmd.workspace_path);
                if !workspace_path.exists() {
                    return Err(format!("importagentfromclaw: path does not exist: {}", cmd.workspace_path));
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                // Detect provider from .claude/settings.json if present
                let mut provider = "claude".to_string();
                let settings_path = workspace_path.join(".claude").join("settings.json");
                if settings_path.exists() {
                    if let Ok(settings_str) = std::fs::read_to_string(&settings_path) {
                        if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&settings_str) {
                            if let Some(p) = settings.get("provider").and_then(|v| v.as_str()) {
                                provider = p.to_string();
                            }
                        }
                    }
                }

                // Create the agent — slug is empty, agent_def_insert will
                // auto-derive from agent_name and mutate the struct
                // so the resolved slug is returned to the frontend.
                let mut agent = AgentDefinition {
                    id: uuid::Uuid::new_v4().to_string(),
                    slug: String::new(),
                    name: cmd.agent_name.clone(),
                    icon: "\u{2726}".to_string(),
                    provider,
                    description: format!("Imported from {}", cmd.workspace_path),
                    working_directory: cmd.workspace_path.clone(),
                    shell: String::new(),
                    provider_flags: String::new(),
                    auto_start: 0,
                    restart_on_crash: 0,
                    idle_timeout_minutes: 0,
                    created_at: now,
                    agent_type: "standalone".to_string(),
                    environment: String::new(),
                    agent_bus_id: String::new(),
                    is_seeded: 0,
                    accounts: String::new(),
                    parent_id: String::new(),
                    branch_label: String::new(),
                    updated_at: now,
                    user_hidden: 0,
                    container_image: String::new(),
                    container_volumes: "[]".to_string(),
                    container_name: String::new(),
                };
                wstore.agent_def_insert(&mut agent).map_err(|e| format!("importagentfromclaw: {e}"))?;

                // Read CLAUDE.md → agentmd content
                let claude_md_path = workspace_path.join("CLAUDE.md");
                if claude_md_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&claude_md_path) {
                        let fc = AgentContent {
                            agent_id: agent.id.clone(),
                            content_type: "agentmd".to_string(),
                            content,
                            updated_at: now,
                        };
                        let _ = wstore.agent_content_set(&fc);
                    }
                }

                // Read .mcp.json → mcp content
                let mcp_path = workspace_path.join(".mcp.json");
                if mcp_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&mcp_path) {
                        let fc = AgentContent {
                            agent_id: agent.id.clone(),
                            content_type: "mcp".to_string(),
                            content,
                            updated_at: now,
                        };
                        let _ = wstore.agent_content_set(&fc);
                    }
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // reseedagents → delete all seeded agents and re-run seed from manifest
    let wstore_rsfa = state.wstore.clone();
    let broker_rsfa = state.broker.clone();
    engine.register_handler(
        COMMAND_RESEED_AGENTS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_rsfa.clone();
            let broker = broker_rsfa.clone();
            Box::pin(async move {
                // Delete all previously seeded agents (cascade deletes content, skills, history)
                let deleted = wstore.agent_def_delete_seeded()
                    .map_err(|e| format!("reseedagents: delete seeded: {e}"))?;

                // Re-run seed
                let report = crate::backend::agent_seed::seed_agents(&wstore)
                    .map_err(|e| format!("reseedagents: seed: {e}"))?;

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(json!({
                    "deleted": deleted,
                    "created": report.created,
                    "skipped": report.skipped,
                })))
            })
        }),
    );

    // importagents — bulk import from JSON export format
    let wstore_ifa = state.wstore.clone();
    let broker_ifa = state.broker.clone();
    engine.register_handler(
        COMMAND_IMPORT_AGENTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ifa.clone();
            let broker = broker_ifa.clone();
            Box::pin(async move {
                let cmd: CommandImportAgentDefinitionsData = serde_json::from_value(data)
                    .map_err(|e| format!("importagents: {e}"))?;

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                let mut imported: Vec<String> = Vec::new();
                let mut skipped: Vec<String> = Vec::new();
                let mut failed: Vec<String> = Vec::new();

                for agent_import in cmd.agents {
                    // Check for existing agent by slug (id field from export)
                    let existing = wstore.agent_def_list()
                        .unwrap_or_default()
                        .into_iter()
                        .any(|a| a.slug == agent_import.id);

                    if existing {
                        skipped.push(agent_import.name.clone());
                        continue;
                    }

                    let mut agent = AgentDefinition {
                        id: uuid::Uuid::new_v4().to_string(),
                        slug: agent_import.id.clone(),
                        name: agent_import.name.clone(),
                        icon: agent_import.icon.clone(),
                        provider: agent_import.provider.clone(),
                        description: agent_import.description.clone(),
                        working_directory: agent_import.working_directory.clone(),
                        shell: agent_import.shell.clone(),
                        provider_flags: String::new(),
                        auto_start: 0,
                        restart_on_crash: if agent_import.restart_on_crash { 1 } else { 0 },
                        idle_timeout_minutes: 0,
                        created_at: now,
                        agent_type: agent_import.agent_type.clone(),
                        environment: agent_import.environment.clone(),
                        agent_bus_id: agent_import.agent_bus_id.clone(),
                        is_seeded: 0,
                        accounts: String::new(),
                        parent_id: String::new(),
                        branch_label: String::new(),
                        updated_at: now,
                        user_hidden: 0,
                        container_image: String::new(),
                        container_volumes: "[]".to_string(),
                        container_name: String::new(),
                    };

                    if let Err(e) = wstore.agent_def_insert(&mut agent) {
                        failed.push(format!("{}: {e}", agent_import.name));
                        continue;
                    }

                    // Insert content types
                    let mut content_ok = true;
                    for (content_type, content) in &agent_import.content {
                        let fc = AgentContent {
                            agent_id: agent.id.clone(),
                            content_type: content_type.clone(),
                            content: content.clone(),
                            updated_at: now,
                        };
                        if let Err(e) = wstore.agent_content_set(&fc) {
                            tracing::warn!("import: failed to set content for agent {}: {e}", agent.id);
                            content_ok = false;
                        }
                    }

                    // Insert skills
                    let mut skills_ok = true;
                    for skill_import in &agent_import.skills {
                        let skill = AgentSkill {
                            id: uuid::Uuid::new_v4().to_string(),
                            agent_id: agent.id.clone(),
                            name: skill_import.name.clone(),
                            trigger: skill_import.trigger.clone(),
                            skill_type: skill_import.skill_type.clone(),
                            description: skill_import.description.clone(),
                            content: skill_import.content.clone(),
                            created_at: now,
                        };
                        if let Err(e) = wstore.agent_skill_insert(&skill) {
                            tracing::warn!("import: failed to insert skill '{}' for agent {}: {e}", skill.name, agent.id);
                            skills_ok = false;
                        }
                    }

                    if content_ok && skills_ok {
                        imported.push(agent_import.name.clone());
                    } else {
                        failed.push(agent_import.name.clone());
                    }
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });

                let result = ImportAgentDefinitionsResult { imported, skipped, failed };
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );

    // exportagents — export all agent definitions with content and skills
    let wstore_efa = state.wstore.clone();
    engine.register_handler(
        COMMAND_EXPORT_AGENTS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_efa.clone();
            Box::pin(async move {
                let agents = wstore.agent_def_list()
                    .map_err(|e| format!("exportagents: list: {e}"))?;

                let mut agent_exports: Vec<AgentDefinitionExport> = Vec::new();

                for agent in agents {
                    let content_map = wstore.agent_content_get_all(&agent.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|fc| (fc.content_type, fc.content))
                        .collect::<std::collections::HashMap<String, String>>();

                    let skills = wstore.agent_skill_list(&agent.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|s| AgentSkillExport {
                            name: s.name,
                            trigger: s.trigger,
                            skill_type: s.skill_type,
                            description: s.description,
                            content: s.content,
                        })
                        .collect::<Vec<_>>();

                    agent_exports.push(AgentDefinitionExport {
                        id: agent.slug.clone(),
                        name: agent.name,
                        icon: agent.icon,
                        description: agent.description,
                        provider: agent.provider,
                        shell: agent.shell,
                        working_directory: agent.working_directory,
                        agent_bus_id: agent.agent_bus_id,
                        agent_type: agent.agent_type,
                        environment: agent.environment,
                        restart_on_crash: agent.restart_on_crash != 0,
                        content: content_map,
                        skills,
                    });
                }

                let exported_at = Utc::now().to_rfc3339();

                let result = ExportAgentDefinitionsResult {
                    version: 4,
                    exported_at,
                    source: "agentmux-export".to_string(),
                    agents: agent_exports,
                };
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );

    register_v6_handlers(engine, state);
}

/// v6 handlers — identity accounts, agent instances, definition branching.
/// See specs/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md §Phase 3.
fn register_v6_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Identity account CRUD ----

    let wstore = state.wstore.clone();
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

    let wstore = state.wstore.clone();
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

    let wstore = state.wstore.clone();
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
    let wstore = state.wstore.clone();
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
    let oauth_wstore = state.wstore.clone();
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

    let wstore = state.wstore.clone();
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

    let wstore = state.wstore.clone();
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

    let wstore = state.wstore.clone();
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

    let wstore = state.wstore.clone();
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

    // ---- Agent instance CRUD ----

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_INSTANCES,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandListAgentInstancesData =
                    serde_json::from_value(data).unwrap_or_default();
                let rows = wstore
                    .instance_list(cmd.definition_id.as_deref(), cmd.status.as_deref())
                    .map_err(|e| format!("listagentinstances: {e}"))?;
                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_AGENT_INSTANCE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetAgentInstanceData = serde_json::from_value(data)
                    .map_err(|e| format!("getagentinstance: {e}"))?;
                match wstore
                    .instance_get(&cmd.id)
                    .map_err(|e| format!("getagentinstance: {e}"))?
                {
                    Some(i) => Ok(Some(serde_json::to_value(&i).unwrap_or_default())),
                    None => Err(format!("getagentinstance: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_AGENT_INSTANCE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandCreateAgentInstanceData = serde_json::from_value(data)
                    .map_err(|e| format!("createagentinstance: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let inst = AgentInstance {
                    id: uuid::Uuid::new_v4().to_string(),
                    definition_id: cmd.definition_id,
                    parent_instance_id: cmd.parent_instance_id,
                    block_id: cmd.block_id,
                    session_id: String::new(),
                    status: InstanceStatus::Running.as_str().to_string(),
                    github_context: String::new(),
                    started_at: now,
                    ended_at: 0,
                    created_at: now,
                    // PR-F.3: launch modal passes through Identity +
                    // Memory bundle picks. Empty string = blank
                    // singleton (no override; the resolver returns
                    // immediately on either "" or "blank").
                    identity_id: cmd.identity_id,
                    memory_id: cmd.memory_id,
                    // v8: named-agent continuation. instance_name +
                    // working_directory come from the launch-modal
                    // overrides via CommandCreateAgentInstanceData
                    // (added in the same spec). Empty string for
                    // legacy/ambient launches.
                    instance_name: cmd.instance_name.clone(),
                    working_directory: cmd.working_directory.clone(),
                    display_hidden: false,
                };
                wstore
                    .instance_create(&inst)
                    .map_err(|e| format!("createagentinstance: {e}"))?;

                // Option E (PR 1 of 2) — stamp the agent-anchored
                // session zone reference onto the block meta. Every
                // block of this agent definition reads/writes through
                // `agent:<defId>:current`. Continuation is now
                // structural (same zone, different block) rather than
                // parametric (per-block snapshot copy + --continue).
                // See docs/specs/SPEC_CONTINUATION_SESSION_PERSISTENCE_2026_05_23.md.
                if !inst.block_id.is_empty()
                    && crate::backend::agent_session::is_valid_definition_id(&inst.definition_id)
                {
                    let zone = crate::backend::agent_session::agent_current_zone(
                        &inst.definition_id,
                    );
                    let mut meta_update = crate::backend::obj::MetaMapType::new();
                    meta_update.insert(
                        "agent:sessionZone".to_string(),
                        serde_json::json!(zone),
                    );
                    let oref_str = format!("block:{}", inst.block_id);
                    if let Err(e) = crate::server::service::update_object_meta(
                        &wstore, &oref_str, &meta_update,
                    ) {
                        // Non-fatal — the instance row is the source
                        // of truth, the meta stamp is a frontend
                        // convenience. Log + continue so the launch
                        // doesn't abort mid-flow.
                        tracing::warn!(
                            block_id = %inst.block_id,
                            definition_id = %inst.definition_id,
                            error = %e,
                            "createagentinstance: failed to stamp agent:sessionZone"
                        );
                    }
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agentinstances:changed:{}", inst.definition_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&inst).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_AGENT_INSTANCE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandUpdateAgentInstanceData = serde_json::from_value(data)
                    .map_err(|e| format!("updateagentinstance: {e}"))?;
                // Partial write — only the fields the command provided.
                // No fetch-and-merge: this used to `instance_get` the full
                // row to fill the unspecified fields, which was the sole
                // production caller needing `instance_get`'s transient
                // per-launch columns. The store builds a dynamic UPDATE
                // and returns the post-write row (for the event scope +
                // response) from the reload it already runs.
                // SPEC_UPDATEAGENTINSTANCE_PARTIAL_UPDATE_2026_05_29.md.
                let upd = crate::backend::storage::InstanceUpdate {
                    block_id: cmd.block_id,
                    session_id: cmd.session_id,
                    status: cmd.status,
                    github_context: cmd.github_context,
                    ended_at: cmd.ended_at,
                };
                let fresh = wstore
                    .instance_update_partial(&cmd.id, &upd)
                    .map_err(|e| format!("updateagentinstance: {e}"))?
                    .ok_or_else(|| format!("updateagentinstance: not found id={}", cmd.id))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agentinstances:changed:{}", fresh.definition_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&fresh).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_AGENT_INSTANCE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteAgentInstanceData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteagentinstance: {e}"))?;
                // Read the row first so we can emit a scoped event after.
                let definition_id = wstore
                    .instance_get(&cmd.id)
                    .map_err(|e| format!("deleteagentinstance: {e}"))?
                    .map(|i| i.definition_id);
                let deleted = wstore
                    .instance_delete(&cmd.id)
                    .map_err(|e| format!("deleteagentinstance: {e}"))?;
                if let Some(def_id) = definition_id.filter(|_| deleted) {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: format!("agentinstances:changed:{}", def_id),
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

    // ---- v8: named agent continuation ----

    // listnamedagents — powers the launch modal's "Continue agent"
    // dropdown. Joins instance rows with the definition / identity /
    // memory bundle names so the frontend renders without follow-ups.
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_NAMED_AGENTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
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
                let identities = wstore
                    .bundle_identity_list()
                    .map_err(|e| format!("listnamedagents: bundle_identity_list: {e}"))?;
                let memories = wstore
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

    // ---- Recent sessions (cascade follow-up 2026-05-23) ----
    //
    // listrecentsessions — joins `db_agent_instances` with the
    // filestore `output.state.json` snapshot for each instance's
    // block_id_hint, producing a preview + node count so the
    // AgentPicker can show actual conversation context instead of just
    // metadata. Sort key is the snapshot modts (last activity)
    // descending; rows without a snapshot fall back to the instance
    // started_at and are de-prioritized. Cap at 20 rows.
    //
    // The reattach mechanism is the existing continuation flow:
    // continueOfInstanceId + workDirOverride (see PR #977). This RPC
    // is a more discoverable surface for finding sessions to continue
    // — particularly orphaned ones whose pane crashed.
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_LIST_RECENT_SESSIONS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandListRecentSessionsData =
                    serde_json::from_value(data).unwrap_or_default();
                let limit = if cmd.limit == 0 {
                    20
                } else {
                    cmd.limit.min(100)
                };
                // Pull up to ~10x the requested cap so we can post-
                // filter by snapshot presence + identity_id without
                // running out of candidates. 10x is a safety margin
                // and stays well inside the 200 default of
                // instance_list_named.
                let raw_limit = (limit * 10).max(50).min(500);

                // Identity filter is pushed INTO `instance_list_named`
                // (codex P2 #3 on PR #1096): when a chain has
                // continuations with different identity bundles, the
                // ranking must run on identity-matching rows so the
                // newest match wins. Post-query filtering would drop
                // the chain entirely if the newest row used a
                // different identity, even when older rows match.
                let identity_filter = cmd
                    .identity_id
                    .as_deref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());

                // "My Agents" sources from the cross-version REGISTRY when it's
                // available, so agents created in ANY build / channel / version
                // appear here — not just this instance's local SQLite sessions
                // (the live mirror, registry_mirror.rs, keeps the registry current
                // for global workspaces). Falls back to local SQLite when the
                // registry couldn't be resolved (CI / odd envs). Cross-channel rows
                // arrive as synthetic instances (no live block); the per-instance
                // snapshot enrichment below lights up the ones that ALSO ran here.
                let instances: Vec<AgentInstance> = match wstore.shared_agent_registry() {
                    Some(reg) => {
                        let agents_root = wstore.registry_agents_base();
                        let mut records = reg
                            .list_active()
                            .map_err(|e| format!("listrecentsessions: registry: {e}"))?;
                        if let Some(idf) = identity_filter {
                            records.retain(|r| r.data.identity_id.as_deref() == Some(idf));
                        }
                        // Dedup by (definition_id, instance_name) keeping the newest
                        // launch — the registry read path lacks SQLite's chain-root
                        // collapse, so two fresh heads of one logical agent would
                        // otherwise double up. Sort newest-first within each group,
                        // collapse, then re-sort by recency.
                        records.sort_by(|a, b| {
                            a.data
                                .definition_id
                                .cmp(&b.data.definition_id)
                                .then_with(|| a.data.instance_name.cmp(&b.data.instance_name))
                                .then_with(|| {
                                    b.data.last_launched_at_ms.cmp(&a.data.last_launched_at_ms)
                                })
                        });
                        records.dedup_by(|a, b| {
                            a.data.definition_id == b.data.definition_id
                                && a.data.instance_name == b.data.instance_name
                        });
                        records.sort_by(|a, b| {
                            b.data.last_launched_at_ms.cmp(&a.data.last_launched_at_ms)
                        });
                        records.truncate(raw_limit);
                        // Local agents in PICKER mode (include_continuations=true):
                        // collapses each chain to one row AND surfaces orphan
                        // continuations (head hard-deleted) as their own root, so
                        // they don't vanish from "My Agents" (reagent P2). Indexed by
                        // (definition_id, instance_name) — the SAME key the registry
                        // dedup uses — keeping the newest, so overlay AND the
                        // local-only append agree on identity (reagent P1).
                        let local = wstore
                            .instance_list_named(raw_limit, None, identity_filter, true)
                            .unwrap_or_default();
                        let mut local_by_key: std::collections::HashMap<
                            (String, String),
                            AgentInstance,
                        > = std::collections::HashMap::new();
                        for li in local {
                            let key = (li.definition_id.clone(), li.instance_name.clone());
                            match local_by_key.get(&key) {
                                Some(e) if e.started_at >= li.started_at => {}
                                _ => {
                                    local_by_key.insert(key, li);
                                }
                            }
                        }
                        let mut out: Vec<AgentInstance> = records
                            .into_iter()
                            .map(|rec| {
                                let d = rec.data;
                                let key = (d.definition_id.clone(), d.instance_name.clone());
                                if let Some(li) = local_by_key.get(&key) {
                                    return li.clone();
                                }
                                // Reconstruct the absolute workdir from the record's
                                // source base (v3) or the current channel (legacy).
                                let working_directory = match d.source_agents_base.as_deref() {
                                    Some(src) => std::path::Path::new(src)
                                        .join(&d.working_dir)
                                        .to_string_lossy()
                                        .to_string(),
                                    None => match agents_root.as_ref() {
                                        Some(root) => root
                                            .join(&d.working_dir)
                                            .to_string_lossy()
                                            .to_string(),
                                        None => d.working_dir.clone(),
                                    },
                                };
                                AgentInstance {
                                    id: d.instance_id,
                                    definition_id: d.definition_id,
                                    parent_instance_id: String::new(),
                                    block_id: String::new(),
                                    session_id: d.session_id.unwrap_or_default(),
                                    status: "available".to_string(),
                                    github_context: String::new(),
                                    started_at: d.last_launched_at_ms,
                                    ended_at: 0,
                                    created_at: d.created_at_ms,
                                    identity_id: d.identity_id.unwrap_or_default(),
                                    memory_id: d.memory_id.unwrap_or_default(),
                                    instance_name: d.instance_name,
                                    working_directory,
                                    display_hidden: false,
                                }
                            })
                            .collect();
                        // APPEND local-only agents — those whose (definition_id,
                        // instance_name) no registry record represents (created
                        // before the live mirror could register them, or orphan
                        // continuations). Keyed identically to the dedup, so a deduped
                        // agent's local head never re-appears as a duplicate row.
                        let have_keys: std::collections::HashSet<(String, String)> = out
                            .iter()
                            .map(|i| (i.definition_id.clone(), i.instance_name.clone()))
                            .collect();
                        for (key, li) in local_by_key {
                            if !have_keys.contains(&key) {
                                out.push(li);
                            }
                        }
                        out
                    }
                    None => wstore
                        .instance_list_named(raw_limit, None, identity_filter, true)
                        .map_err(|e| format!("listrecentsessions: {e}"))?,
                };

                let defs = wstore
                    .agent_def_list()
                    .map_err(|e| format!("listrecentsessions: defs: {e}"))?;
                let identities = wstore
                    .bundle_identity_list()
                    .map_err(|e| format!("listrecentsessions: identities: {e}"))?;
                let memories = wstore
                    .bundle_memory_list()
                    .map_err(|e| format!("listrecentsessions: memories: {e}"))?;

                // Build rows. Hits filestore once per instance; with
                // raw_limit ≤ 500 and stat() being a single indexed
                // SQLite query, the per-call cost is dominated by
                // the eventual snapshot read for the top-20.
                let mut rows: Vec<RecentSessionRow> = Vec::with_capacity(instances.len());
                for inst in instances {
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

                    // Stat first (cheap) — gives us the modts for
                    // sorting. Only fetch the full content if the
                    // snapshot exists.
                    let (has_snapshot, last_active_at, preview, node_count) =
                        if inst.block_id.is_empty() {
                            (false, inst.started_at, String::new(), 0usize)
                        } else {
                            match filestore.stat(&inst.block_id, "output.state.json") {
                                Ok(Some(file)) => {
                                    let modts = if file.modts > 0 {
                                        file.modts
                                    } else {
                                        inst.started_at
                                    };
                                    let (preview, node_count) = read_session_preview(
                                        &filestore,
                                        &inst.block_id,
                                    );
                                    (true, modts, preview, node_count)
                                }
                                _ => (false, inst.started_at, String::new(), 0usize),
                            }
                        };

                    rows.push(RecentSessionRow {
                        instance_id: inst.id,
                        instance_name: inst.instance_name,
                        definition_id: inst.definition_id.clone(),
                        definition_name: def
                            .map(|d| d.name.clone())
                            .unwrap_or_else(|| "(missing definition)".to_string()),
                        provider: def.map(|d| d.provider.clone()).unwrap_or_default(),
                        working_directory: inst.working_directory,
                        identity_id: inst.identity_id,
                        identity_name,
                        memory_id: inst.memory_id,
                        memory_name,
                        block_id_hint: inst.block_id,
                        // Surface the CLI-captured session id so the
                        // picker reattach can `--resume <sid>` on the
                        // FIRST turn of the new block. Without this
                        // the new subprocess starts a fresh session
                        // and the CLI re-injects the startup context.
                        session_id: inst.session_id,
                        preview,
                        node_count,
                        last_active_at,
                        has_snapshot,
                        agent_created_at: def.map(|d| d.created_at).unwrap_or(0),
                        started_at: inst.started_at,
                        agent_type: def.map(|d| d.agent_type.clone()).unwrap_or_default(),
                    });
                }

                // Sort: rows with a snapshot first (descending by
                // modts), then no-snapshot rows by started_at desc.
                // This keeps live conversations at the top while
                // still surfacing legacy rows.
                rows.sort_by(|a, b| match (a.has_snapshot, b.has_snapshot) {
                    (true, true) | (false, false) => {
                        b.last_active_at.cmp(&a.last_active_at)
                    }
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                });
                rows.truncate(limit);

                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );

    // ---- Definition fork ----

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_FORK_AGENT_DEFINITION,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandForkAgentDefinitionData = serde_json::from_value(data)
                    .map_err(|e| format!("forkagentdefinition: {e}"))?;

                // Find the source definition by id.
                let all_defs = wstore
                    .agent_def_list()
                    .map_err(|e| format!("forkagentdefinition: {e}"))?;
                let source = all_defs
                    .iter()
                    .find(|a| a.id == cmd.source_id)
                    .cloned()
                    .ok_or_else(|| format!("forkagentdefinition: source not found: {}", cmd.source_id))?;

                // Build a new definition that shares the source's content but
                // has a fresh id/slug and records the lineage. Seed-bit is
                // cleared — forks are always user-owned, not built-in.
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);

                // branch_label is the fork's full display name when provided.
                // When empty, auto-generate "Name #N" based on existing fork count.
                let fork_name = if cmd.branch_label.is_empty() {
                    let existing_fork_count = all_defs
                        .iter()
                        .filter(|a| a.parent_id == cmd.source_id && a.is_seeded == 0)
                        .count();
                    format!("{} #{}", source.name, existing_fork_count + 2)
                } else {
                    cmd.branch_label.clone()
                };
                let branch_label = if cmd.branch_label.is_empty() {
                    fork_name.clone()
                } else {
                    cmd.branch_label.clone()
                };
                let branch_slug_part = crate::backend::storage::store::derive_slug(&branch_label);
                let mut fork = AgentDefinition {
                    id: uuid::Uuid::new_v4().to_string(),
                    // Empty slug → agent_def_insert derives + resolves collisions.
                    slug: format!("{}-{}", source.slug, branch_slug_part),
                    name: fork_name,
                    icon: source.icon.clone(),
                    provider: source.provider.clone(),
                    description: source.description.clone(),
                    working_directory: String::new(), // force re-resolve via agentmuxHome()
                    shell: source.shell.clone(),
                    provider_flags: source.provider_flags.clone(),
                    auto_start: 0, // forks don't auto-start; explicit launch only
                    restart_on_crash: source.restart_on_crash,
                    idle_timeout_minutes: source.idle_timeout_minutes,
                    created_at: now,
                    agent_type: source.agent_type.clone(),
                    environment: source.environment.clone(),
                    agent_bus_id: String::new(), // fresh bus id so broadcasts don't cross
                    is_seeded: 0,
                    accounts: String::new(),
                    parent_id: source.id.clone(),
                    branch_label: branch_label.clone(),
                    updated_at: now,
                    user_hidden: 0,
                    // Forks inherit container config from source so forked container agents
                    // retain their image and volumes.
                    container_image: source.container_image.clone(),
                    container_volumes: source.container_volumes.clone(),
                    container_name: String::new(),
                };
                wstore
                    .agent_def_insert(&mut fork)
                    .map_err(|e| format!("forkagentdefinition: {e}"))?;

                // Deep-copy content blobs + skills from source. Cascade foreign
                // keys on the source are unaffected — we're copying out, not
                // moving.
                let source_contents = wstore
                    .agent_content_get_all(&source.id)
                    .map_err(|e| format!("forkagentdefinition content: {e}"))?;
                for c in source_contents {
                    let new_content = AgentContent {
                        agent_id: fork.id.clone(),
                        content_type: c.content_type,
                        content: c.content,
                        updated_at: now,
                    };
                    wstore
                        .agent_content_set(&new_content)
                        .map_err(|e| format!("forkagentdefinition content: {e}"))?;
                }
                let source_skills = wstore
                    .agent_skill_list(&source.id)
                    .map_err(|e| format!("forkagentdefinition skills: {e}"))?;
                for s in source_skills {
                    let new_skill = AgentSkill {
                        id: uuid::Uuid::new_v4().to_string(),
                        agent_id: fork.id.clone(),
                        name: s.name,
                        trigger: s.trigger,
                        skill_type: s.skill_type,
                        description: s.description,
                        content: s.content,
                        created_at: now,
                    };
                    wstore
                        .agent_skill_insert(&new_skill)
                        .map_err(|e| format!("forkagentdefinition skill: {e}"))?;
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });

                Ok(Some(serde_json::to_value(&fork).unwrap_or_default()))
            })
        }),
    );

    // ---- Definition fork suggest (read-only — no mutation) ----

    let wstore_sug = state.wstore.clone();
    engine.register_handler(
        COMMAND_FORK_AGENT_DEFINITION_SUGGEST,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sug.clone();
            Box::pin(async move {
                let cmd: CommandForkAgentDefinitionSuggestData = serde_json::from_value(data)
                    .map_err(|e| format!("forkagentdefinitionsuggest: {e}"))?;

                let all = wstore
                    .agent_def_list()
                    .map_err(|e| format!("forkagentdefinitionsuggest: {e}"))?;
                let source = all
                    .iter()
                    .find(|a| a.id == cmd.source_id)
                    .ok_or_else(|| format!("forkagentdefinitionsuggest: source not found: {}", cmd.source_id))?;

                let existing_fork_count = all
                    .iter()
                    .filter(|a| a.parent_id == cmd.source_id && a.is_seeded == 0)
                    .count();
                let suggested_label = format!("{} #{}", source.name, existing_fork_count + 2);

                let result = ForkAgentDefinitionSuggestResult { suggested_label };
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );

    register_agent_session_handlers(engine, state);
    register_v7_handlers(engine, state);
}

/// Option E (PR 1 of 2) — agent-anchored session zone RPCs.
///
/// These commands read/write the per-agent FileStore zone
/// `agent:<definition_id>:current` and the per-archive zones
/// `agent:<definition_id>:archive:<ts_ms>`. Session is bound to the
/// agent definition, NOT the identity bundle — see the spec.
fn register_agent_session_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- agent:session:read ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_READ,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionReadData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:session:read: {e}"))?;
                let (content, modts) =
                    crate::backend::agent_session::read_session_state(&filestore, &cmd.definition_id)
                        .map_err(|e| format!("agent:session:read: {e}"))?;
                Ok(Some(
                    serde_json::to_value(&AgentSessionReadResult { content, modts })
                        .unwrap_or_default(),
                ))
            })
        }),
    );

    // ---- agent:session:write_state ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_WRITE_STATE,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionWriteStateData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:session:write_state: {e}"))?;
                let bytes = cmd.content.as_bytes();
                let bytes_written = bytes.len() as u64;
                crate::backend::agent_session::write_session_state(
                    &filestore,
                    &cmd.definition_id,
                    bytes,
                )
                .map_err(|e| format!("agent:session:write_state: {e}"))?;
                Ok(Some(
                    serde_json::to_value(&AgentSessionWriteStateResult { bytes_written })
                        .unwrap_or_default(),
                ))
            })
        }),
    );

    // ---- agent:session:append_output ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_APPEND_OUTPUT,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionAppendOutputData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:session:append_output: {e}"))?;
                let bytes_written = crate::backend::agent_session::append_session_output(
                    &filestore,
                    &cmd.definition_id,
                    &cmd.line,
                )
                .map_err(|e| format!("agent:session:append_output: {e}"))?;
                Ok(Some(
                    serde_json::to_value(&AgentSessionAppendOutputResult { bytes_written })
                        .unwrap_or_default(),
                ))
            })
        }),
    );

    // ---- agent:session:archive ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_ARCHIVE,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionArchiveData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:session:archive: {e}"))?;
                let result =
                    crate::backend::agent_session::archive_session(&filestore, &cmd.definition_id)
                        .map_err(|e| format!("agent:session:archive: {e}"))?;
                let (archive_zoneid, archived_at_ms) = match result {
                    Some((z, ts)) => (z, ts),
                    None => (String::new(), 0),
                };
                Ok(Some(
                    serde_json::to_value(&AgentSessionArchiveResult {
                        archive_zoneid,
                        archived_at_ms,
                    })
                    .unwrap_or_default(),
                ))
            })
        }),
    );

    // ---- agent:session:list_archives ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_LIST_ARCHIVES,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionListArchivesData =
                    serde_json::from_value(data).unwrap_or_default();
                let summaries = crate::backend::agent_session::list_archives(
                    &filestore,
                    &cmd.definition_id,
                    cmd.limit,
                )
                .map_err(|e| format!("agent:session:list_archives: {e}"))?;
                let rows: Vec<AgentArchiveRow> = summaries
                    .into_iter()
                    .map(|s| AgentArchiveRow {
                        archive_zoneid: s.archive_zoneid,
                        archived_at_ms: s.archived_at_ms,
                        preview: s.preview,
                        node_count: s.node_count,
                    })
                    .collect();
                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );
}

/// v7 handlers — Identity bundles (named credential bundles) + Memory bundles.
/// See `docs/specs/identity-forge-integration-and-vault-2026-05-08.md`.
///
/// Identity bundles aggregate accounts (one per provider) under a named
/// label, replacing the per-agent `db_agent_identity_links` semantics.
/// Memory bundles hold the agent's personality + capability stack.
fn register_v7_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Identity bundle CRUD ----

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_IDENTITY_BUNDLES,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let bundles = wstore
                    .bundle_identity_list()
                    .map_err(|e| format!("listidentitybundles: {e}"))?;
                Ok(Some(serde_json::to_value(&bundles).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_IDENTITY_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetIdentityBundleData = serde_json::from_value(data)
                    .map_err(|e| format!("getidentitybundle: {e}"))?;
                match wstore
                    .bundle_identity_get(&cmd.id)
                    .map_err(|e| format!("getidentitybundle: {e}"))?
                {
                    Some(b) => Ok(Some(serde_json::to_value(&b).unwrap_or_default())),
                    None => Err(format!("getidentitybundle: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_IDENTITY_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut bundle: Identity = serde_json::from_value(data)
                    .map_err(|e| format!("upsertidentitybundle: {e}"))?;
                // Guard on BOTH client-supplied is_blank AND id == "blank".
                // Without the id check a caller could send
                // {id:"blank", is_blank:false, name:"evil"} and the
                // ON CONFLICT(id) DO UPDATE path would rename/re-describe
                // the seeded singleton. (reagent P1, 2026-05-08).
                if bundle.is_blank || bundle.id == "blank" {
                    return Err(
                        "upsertidentitybundle: cannot mutate the blank singleton".to_string(),
                    );
                }
                if bundle.id.is_empty() {
                    bundle.id = uuid::Uuid::new_v4().to_string();
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if bundle.created_at == 0 {
                    bundle.created_at = now;
                }
                bundle.updated_at = now;
                wstore
                    .bundle_identity_upsert(&bundle)
                    .map_err(|e| format!("upsertidentitybundle: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "identitybundles:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&bundle).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_IDENTITY_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteIdentityBundleData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteidentitybundle: {e}"))?;
                let deleted = wstore
                    .bundle_identity_delete(&cmd.id)
                    .map_err(|e| format!("deleteidentitybundle: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "identitybundles:changed".to_string(),
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

    // ---- Identity bundle bindings (junction with accounts) ----

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_BIND_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandBindIdentityAccountData = serde_json::from_value(data)
                    .map_err(|e| format!("bindidentityaccount: {e}"))?;
                wstore
                    .bundle_identity_bind(&cmd.identity_id, &cmd.provider, &cmd.account_id)
                    .map_err(|e| format!("bindidentityaccount: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("identitybundlebindings:changed:{}", cmd.identity_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UNBIND_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandUnbindIdentityAccountData = serde_json::from_value(data)
                    .map_err(|e| format!("unbindidentityaccount: {e}"))?;
                let removed = wstore
                    .bundle_identity_unbind(&cmd.identity_id, &cmd.provider)
                    .map_err(|e| format!("unbindidentityaccount: {e}"))?;
                if removed {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: format!("identitybundlebindings:changed:{}", cmd.identity_id),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(json!({ "unbound": removed })))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_IDENTITY_BINDINGS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandListIdentityBindingsData = serde_json::from_value(data)
                    .map_err(|e| format!("listidentitybindings: {e}"))?;
                let bindings = wstore
                    .bundle_identity_bindings(&cmd.identity_id)
                    .map_err(|e| format!("listidentitybindings: {e}"))?;
                Ok(Some(serde_json::to_value(&bindings).unwrap_or_default()))
            })
        }),
    );

    // ---- Memory bundle CRUD ----

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_MEMORIES,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let memories = wstore
                    .bundle_memory_list()
                    .map_err(|e| format!("listmemories: {e}"))?;
                Ok(Some(serde_json::to_value(&memories).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetMemoryData = serde_json::from_value(data)
                    .map_err(|e| format!("getmemory: {e}"))?;
                match wstore
                    .bundle_memory_get(&cmd.id)
                    .map_err(|e| format!("getmemory: {e}"))?
                {
                    Some(m) => Ok(Some(serde_json::to_value(&m).unwrap_or_default())),
                    None => Err(format!("getmemory: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut memory: Memory = serde_json::from_value(data)
                    .map_err(|e| format!("upsertmemory: {e}"))?;
                // Guard on BOTH client-supplied is_blank AND id == "blank".
                // Same bypass as upsertidentitybundle — see that comment.
                // (reagent P1, 2026-05-08).
                if memory.is_blank || memory.id == "blank" {
                    return Err("upsertmemory: cannot mutate the blank singleton".to_string());
                }
                if memory.id.is_empty() {
                    memory.id = uuid::Uuid::new_v4().to_string();
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if memory.created_at == 0 {
                    memory.created_at = now;
                }
                memory.updated_at = now;
                wstore
                    .bundle_memory_upsert(&memory)
                    .map_err(|e| format!("upsertmemory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&memory).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteMemoryData = serde_json::from_value(data)
                    .map_err(|e| format!("deletememory: {e}"))?;
                let deleted = wstore
                    .bundle_memory_delete(&cmd.id)
                    .map_err(|e| format!("deletememory: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "memories:changed".to_string(),
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

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_REORDER_GLOBAL_BRAIN,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandReorderGlobalBrainData = serde_json::from_value(data)
                    .map_err(|e| format!("reorderglobalbrain: {e}"))?;
                let updated = wstore
                    .bundle_memory_reorder(&cmd.ids)
                    .map_err(|e| format!("reorderglobalbrain: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(json!({ "updated": updated })))
            })
        }),
    );
}

pub fn register_agent_input_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // subprocessspawn → spawn agent CLI as subprocess for a single turn
    let wstore_spawn = state.wstore.clone();
    let broker_spawn = state.broker.clone();
    let event_bus_spawn = state.event_bus.clone();
    let filestore_spawn = state.filestore.clone();
    engine.register_handler(
        COMMAND_SUBPROCESS_SPAWN,
        Box::new(move |data, _ctx| {
            let wstore = wstore_spawn.clone();
            let broker = broker_spawn.clone();
            let event_bus = event_bus_spawn.clone();
            let filestore = filestore_spawn.clone();
            Box::pin(async move {
                let cmd: CommandSubprocessSpawnData = serde_json::from_value(data)
                    .map_err(|e| format!("subprocessspawn: {e}"))?;
                tracing::info!(
                    block_id = %cmd.blockid,
                    cli = %cmd.cli_command,
                    "SubprocessSpawn"
                );

                // Get or create a SubprocessController for this block
                let ctrl = match blockcontroller::get_controller(&cmd.blockid) {
                    Some(c) if c.controller_type() == blockcontroller::BLOCK_CONTROLLER_SUBPROCESS => c,
                    _ => {
                        // Create and register a new SubprocessController
                        let ctrl = blockcontroller::subprocess::SubprocessController::new(
                            cmd.tabid.clone(),
                            cmd.blockid.clone(),
                            Some(broker),
                            Some(event_bus),
                            Some(wstore),
                            Some(filestore),
                        );
                        let ctrl = std::sync::Arc::new(ctrl);
                        ctrl.set_self_ref();
                        blockcontroller::register_controller(&cmd.blockid, ctrl.clone());
                        ctrl as std::sync::Arc<dyn blockcontroller::Controller>
                    }
                };

                // Downcast to SubprocessController to call spawn_turn
                let subprocess_ctrl = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                    .ok_or_else(|| "controller is not a SubprocessController".to_string())?;

                let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                    cli_command: cmd.cli_command,
                    cli_args: cmd.cli_args,
                    working_dir: cmd.working_dir,
                    env_vars: cmd.env_vars,
                    message: cmd.message,
                    resume_flag: "--resume".to_string(),
                    session_id_field: "session_id".to_string(),
                    message_id: None,
                    // Direct-spawn legacy command — caller doesn't
                    // carry a reattach context. Greenfield session id
                    // is None; spawn_turn captures it from CLI stdout
                    // on the first turn as before.
                    session_id: None,
                };
                subprocess_ctrl.spawn_turn(config)?;
                Ok(None)
            })
        }),
    );

    // agentinput → send message to agent (persistent or per-turn subprocess)
    let wstore_ai = state.wstore.clone();
    // Streaming-bash wrapper auth — clone the per-launch auth_key into the
    // handler's closure so each spawn can inject it into Claude's env.
    // See SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7.
    let auth_key_ai = state.auth_key.clone();
    // Broker — passed into the identity-injection path so the OAuth
    // expiry probe (PR D, spec §4.4) can publish
    // `identitybundlebindings:changed:<bundle_id>` on status change.
    let broker_ai = state.broker.clone();
    // Container manager — None on hosts without Docker; container agents
    // return an error to the caller rather than crashing the server.
    let container_manager_ai = state.container_manager.clone();
    engine.register_handler(
        COMMAND_AGENT_INPUT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ai.clone();
            let auth_key = auth_key_ai.clone();
            let broker = broker_ai.clone();
            let container_manager = container_manager_ai.clone();
            Box::pin(async move {
                let cmd: CommandAgentInputData = serde_json::from_value(data)
                    .map_err(|e| format!("agentinput: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, "AgentInput");

                let ctrl = blockcontroller::get_controller(&cmd.blockid)
                    .ok_or_else(|| format!("no controller for block {}", cmd.blockid))?;

                // Re-read the spawn config from block metadata
                let block: Block = wstore
                    .get(&cmd.blockid)
                    .map_err(|e| format!("agentinput: load block: {e}"))?
                    .ok_or_else(|| format!("block {} not found", cmd.blockid))?;

                let cli_command = crate::backend::obj::meta_get_string(
                    &block.meta, "cmd", "claude",
                );
                let cli_args: Vec<String> = match block.meta.get("cmd:args") {
                    Some(serde_json::Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                    _ => vec![
                        "-p".to_string(),
                        "--input-format".to_string(),
                        "stream-json".to_string(),
                        "--output-format".to_string(),
                        "stream-json".to_string(),
                    ],
                };
                let working_dir = crate::backend::obj::meta_get_string(
                    &block.meta, "cmd:cwd", "",
                );
                let mut env_vars: std::collections::HashMap<String, String> = match block.meta.get("cmd:env") {
                    Some(serde_json::Value::Object(obj)) => obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect(),
                    _ => std::collections::HashMap::new(),
                };
                // Identity injection: look up the active AgentInstance for
                // this block, resolve its identity_id's bindings, and merge
                // each per-provider env var into the spawn map. Failures
                // are logged and skipped — the agent CLI launches with
                // whatever resolved cleanly plus the static cmd:env block.
                // See agentmux-srv/src/identity/resolver.rs. Broker
                // hand-in lets the OAuth expiry probe (PR D, spec §4.4)
                // publish `identitybundlebindings:changed:<bundle_id>`
                // when it flips a token's status valid→expired etc.
                env_vars = crate::identity::resolver::inject_identity_env_async(
                    wstore.clone(),
                    Some(broker.clone()),
                    cmd.blockid.clone(),
                    env_vars,
                )
                .await;
                // MuxBus cloud token — injects MUXBUS_TOKEN + MUXBUS_COGNITO_DOMAIN
                // if the user has authenticated via muxbus.login. No-op if no
                // credentials are stored. Auto-refreshes if token is nearly expired.
                crate::server::muxbus_handlers::inject_muxbus_env(&wstore, &mut env_vars);
                // Streaming-bash wrapper auth + discovery
                // (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7).
                //
                // 1. AGENTMUX_AUTH_KEY — config.rs:42 removed it from
                //    the process env at startup (security PR #801).
                //    Re-inject for this spawn so the wrapper (running
                //    inside Claude's bash subprocess tree) can
                //    authenticate against the auth_middleware-gated
                //    /agentmux/wps/publish endpoint via X-AuthKey.
                // 2. PATH — prepend the bundled tools/bin dir so
                //    `agentmux-bashwrap.exe` resolves when the
                //    PreToolUse hook (auto-injected by agent_config.rs)
                //    rewrites the command to invoke it. AGENTMUX_LOCAL_URL
                //    is already in the inherited process env (main.rs:498).
                env_vars.insert("AGENTMUX_AUTH_KEY".to_string(), auth_key.clone());
                // Block id so the wrapper can scope its WPS publishes
                // to `block:<id>`. Without this, chunks publish without
                // a scope and the frontend's per-block subscription
                // doesn't receive them.
                env_vars.insert("AGENTMUX_BLOCKID".to_string(), cmd.blockid.clone());
                // Agent display name for MuxBus self-identification.
                // muxbus-client reads AGENTMUX_AGENT_ID (preferred) or AGENT_NAME.
                // Only set if not already present in cmd:env — user-provided values take precedence.
                if !env_vars.contains_key("AGENTMUX_AGENT_ID") {
                    let agent_display_name = crate::backend::obj::meta_get_string(
                        &block.meta, "agentName", "",
                    );
                    if !agent_display_name.is_empty() {
                        env_vars.insert("AGENTMUX_AGENT_ID".to_string(), agent_display_name);
                    }
                }
                // PATH includes BOTH bundled tools dir (portable
                // builds, runtime/tools/bin/) AND user tools dir
                // (~/.agentmux/tools/bin/). bundled is None in dev
                // mode (target/debug exclusion in tool_store), so
                // without user_tools_dir the wrapper wouldn't be on
                // the agent's PATH during `task dev`.
                {
                    let existing = env_vars
                        .get("PATH")
                        .cloned()
                        .or_else(|| std::env::var("PATH").ok())
                        .unwrap_or_default();
                    let sep = if cfg!(windows) { ";" } else { ":" };
                    let mut extras: Vec<String> = Vec::new();
                    if let Some(d) = crate::backend::tool_store::bundled_tools_dir() {
                        if d.exists() {
                            extras.push(d.to_string_lossy().into_owned());
                        }
                    }
                    if let Some(d) = crate::backend::tool_store::user_tools_dir() {
                        if d.exists() {
                            extras.push(d.to_string_lossy().into_owned());
                        }
                    }
                    if !extras.is_empty() {
                        let new_path = format!("{}{}{}", extras.join(sep), sep, existing);
                        env_vars.insert("PATH".to_string(), new_path);
                    }
                }

                let session_id_field = crate::backend::obj::meta_get_string(
                    &block.meta, "agent:session_id_field", "session_id",
                );

                // Try persistent controller first, fall back to subprocess
                if let Some(persistent_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
                {
                    // Container agents use per-turn docker exec — incompatible with a
                    // long-lived persistent subprocess. Fail loudly instead of silently
                    // spawning the CLI on the host.
                    let agent_mode = crate::backend::obj::meta_get_string(&block.meta, "agentMode", "host");
                    if agent_mode == "container" {
                        return Err("container agents require a subprocess controller; this provider uses a persistent controller".to_string());
                    }
                    // Resume support: a /model (or effort/permission) change
                    // respawns the persistent CLI with new flags; pass the
                    // resume flag + captured session id so the respawn continues
                    // the same conversation. Same meta keys the subprocess path
                    // reads below. Without this, switching model on a persistent
                    // agent would either no-op (old behavior) or lose context.
                    let resume_flag = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:resume_flag", "--resume",
                    );
                    let persisted_session_id = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:sessionid", "",
                    );
                    let config = blockcontroller::persistent::PersistentSpawnConfig {
                        cli_command,
                        cli_args,
                        working_dir,
                        env_vars,
                        session_id_field,
                        resume_flag,
                        session_id: persisted_session_id,
                        message_id: cmd.message_id.clone(),
                    };
                    persistent_ctrl.send_message(cmd.message, config)?;
                } else if let Some(subprocess_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                {
                    let resume_flag = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:resume_flag", "--resume",
                    );
                    // Picker reattach: the frontend writes the prior
                    // block's session id here when launching with
                    // `continueOfInstanceId`. spawn_turn hydrates its
                    // inner.session_id from this on the first turn so
                    // --resume <sid> lands on the very first launch.
                    let persisted_session_id = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:sessionid", "",
                    );

                    // Container agent branch: use Docker socket API exec (P1a: no
                    // secrets in argv). Host agent branch: regular CLI subprocess.
                    let agent_mode = crate::backend::obj::meta_get_string(
                        &block.meta, "agentMode", "host",
                    );
                    if agent_mode == "container" {
                        let cm = container_manager.as_deref()
                            .ok_or_else(|| "Docker not available on this host; cannot start container agent".to_string())?;
                        let container_image = crate::backend::obj::meta_get_string(
                            &block.meta, "agent:container_image", "ghcr.io/agentmuxai/agent-claude:latest",
                        );
                        // Use agentId (UUID) — always valid as a Docker name; display names can have spaces.
                        let agent_id = crate::backend::obj::meta_get_string(
                            &block.meta, "agentId", "",
                        );
                        let container_name = crate::backend::container::container_name_for_slug(&agent_id);
                        let volumes_json = crate::backend::obj::meta_get_string(
                            &block.meta, "agent:container_volumes", "[]",
                        );
                        let volumes: Vec<String> = serde_json::from_str(&volumes_json).unwrap_or_default();

                        // Ensure container is alive (pull image if needed — P1b).
                        if let Err(e) = cm.ensure_running(&container_name, &container_image, &volumes, &[]).await {
                            // Surface the error in the agent pane before returning, so the user
                            // sees why the container failed (image not found, Docker down, etc.).
                            let error_frame = serde_json::json!({
                                "type": "result",
                                "is_error": true,
                                "subtype": "error_during_execution",
                                "error": {"message": format!("[AgentMux] container ensure_running failed: {e}")}
                            }).to_string();
                            crate::backend::blockcontroller::shell::handle_append_block_file(
                                &broker,
                                &cmd.blockid,
                                crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
                                format!("{error_frame}\n").as_bytes(),
                                None,
                                None,
                            );
                            return Err(format!("container ensure_running failed: {e}"));
                        }

                        tracing::info!(
                            block_id = %cmd.blockid,
                            container = %container_name,
                            image = %container_image,
                            "container agent turn: bollard exec (env via Docker socket, not argv)",
                        );

                        // Env is passed via CreateExecOptions.env (Docker socket API),
                        // NOT as -e KEY=VALUE argv args — this prevents CWE-214 exposure.
                        // spawn_container_turn filters config.env_vars (denylist) per
                        // turn, so cmd:cwd (host path) and host-path vars never reach
                        // the container, and each queued turn uses its own env.

                        // Base cmd: [container_command, ...cli_args]. The command
                        // is the provider CLI resolved INSIDE the image (on PATH,
                        // e.g. `claude`) — NOT `cli_command`/`cmd`, which is the
                        // host-resolved absolute npm path and does not exist in the
                        // container (docker exec would fail "no such file or
                        // directory"). cli_args are format flags (-p, --input-format
                        // …) + provider flags — no host paths, safe as-is.
                        // spawn_container_turn appends --resume <sid> internally.
                        let container_command = crate::backend::obj::meta_get_string(
                            &block.meta, "agent:container_command", "claude",
                        );
                        let mut base_cmd = vec![container_command];
                        base_cmd.extend(cli_args);

                        let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                            cli_command: String::new(), // unused by spawn_container_turn
                            cli_args: vec![],           // unused by spawn_container_turn
                            working_dir: String::new(), // unused — container has own cwd
                            env_vars,
                            message: cmd.message,
                            resume_flag,
                            session_id_field,
                            message_id: cmd.message_id,
                            session_id: if persisted_session_id.is_empty() {
                                None
                            } else {
                                Some(persisted_session_id)
                            },
                        };
                        subprocess_ctrl.spawn_container_turn(cm.clone(), container_name, base_cmd, config)?;
                    } else {
                        // Host agent: regular CLI subprocess (env set on child process, not in argv).
                        let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                            cli_command,
                            cli_args,
                            working_dir,
                            env_vars,
                            message: cmd.message,
                            resume_flag,
                            session_id_field,
                            message_id: cmd.message_id,
                            session_id: if persisted_session_id.is_empty() {
                                None
                            } else {
                                Some(persisted_session_id)
                            },
                        };
                        subprocess_ctrl.spawn_turn(config)?;
                    }
                } else {
                    return Err("controller is not a SubprocessController or PersistentSubprocessController".to_string());
                }

                // Register with cloud subscriber + reactive handler so cloud-injected
                // messages (e.g. GitHub PR review notifications) reach this agent.
                // Uses agentName (the logical display name, e.g. "smike") as the key —
                // matching the namespace used by reactive.rs:233 (`req.agent_id`) and the
                // delivery path (`agent_to_block` keyed by lowercased logical agent_id).
                // PR bodies embed $AGENTMUX_AGENT_ID (same value) so the cloud injection
                // key and the poll key are always consistent.
                // Both calls are idempotent: add_agent skips the WS send if already
                // subscribed; register_agent replaces any stale mapping from a prior session.
                let agent_name = crate::backend::obj::meta_get_string(
                    &block.meta, "agentName", "",
                );
                if !agent_name.is_empty() {
                    let registered = crate::backend::reactive::handler::get_global_handler()
                        .register_agent(&agent_name, &cmd.blockid, None);
                    if registered.is_ok() {
                        if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                            sub.add_agent(&agent_name);
                        }
                    }
                }

                Ok(None)
            })
        }),
    );

    // agentstop → stop the running agent subprocess
    engine.register_handler(
        COMMAND_AGENT_STOP,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandAgentStopData = serde_json::from_value(data)
                    .map_err(|e| format!("agentstop: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, force = cmd.force, "AgentStop");
                match blockcontroller::get_controller(&cmd.blockid) {
                    Some(ctrl) => {
                        ctrl.stop(!cmd.force, blockcontroller::STATUS_DONE)?;
                        // Deregister: unregister_block cleans up both agent_to_block and
                        // block_to_agent maps; remove_agent then removes the cloud poll entry
                        // using the logical agent_id recovered from block_to_agent.
                        let handler = crate::backend::reactive::handler::get_global_handler();
                        let agent_name = handler.agent_id_for_block(&cmd.blockid);
                        handler.unregister_block(&cmd.blockid);
                        if let (Some(sub), Some(name)) = (
                            crate::muxbus::cloud_subscriber::get_global_subscriber(),
                            agent_name,
                        ) {
                            sub.remove_agent(&name);
                        }
                        Ok(None)
                    }
                    None => Ok(None),
                }
            })
        }),
    );
}

/// Read the per-block `output.state.json` snapshot from filestore and
/// extract a `(preview, node_count)` pair for the AgentPicker's
/// "Recent sessions" list.
///
/// The snapshot shape is owned by the frontend (see
/// `frontend/app/view/agent/agent-view.tsx::writeSnapshotNow`):
/// `{ schemaVersion, savedAt, highWaterMark, historyOffset, nodes: [DocumentNode...] }`.
/// We only touch two fields:
/// - `nodes.length` → `node_count`.
/// - The first node with `type === "user_message"`, `message` field →
///   `preview` (trimmed, newlines collapsed, max 240 chars).
///
/// On any error (snapshot missing, malformed JSON, no user message),
/// returns `("", 0)`. Callers treat that the same as "no preview".
fn read_session_preview(
    filestore: &crate::backend::storage::filestore::FileStore,
    block_id: &str,
) -> (String, usize) {
    let bytes = match filestore.read_file(block_id, "output.state.json") {
        Ok(Some(b)) => b,
        _ => return (String::new(), 0),
    };
    // Cap the parse budget — a misbehaving / corrupted snapshot
    // shouldn't be able to stall this handler. 4MiB is well above the
    // typical conversation snapshot (Maks's was ~750KiB for 169 nodes)
    // but bounded enough to fail fast on garbage.
    if bytes.len() > 4 * 1024 * 1024 {
        tracing::warn!(
            block_id = %block_id,
            size = bytes.len(),
            "listrecentsessions: snapshot too large; skipping preview"
        );
        return (String::new(), 0);
    }
    let json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return (String::new(), 0),
    };
    let nodes = match json.get("nodes").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return (String::new(), 0),
    };
    let node_count = nodes.len();
    // First user_message wins. Skip the bootstrap "Session Context"
    // prompt when present — it's always the first node and is system
    // boilerplate the user didn't type; if a subsequent user_message
    // exists, that's the more useful preview. Heuristic: if the first
    // user message starts with "# Session Context", scan for the next.
    let mut preview = String::new();
    for node in nodes {
        let ty = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "user_message" {
            continue;
        }
        let msg = node
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if msg.is_empty() {
            continue;
        }
        if preview.is_empty() && msg.starts_with("# Session Context") {
            // Stash as fallback in case there's no later user_message.
            preview = collapse_preview(msg);
            continue;
        }
        preview = collapse_preview(msg);
        break;
    }
    (preview, node_count)
}

/// Collapse newlines + extra whitespace, cap at 240 chars. Output is
/// safe to render inline in a single-line preview row.
fn collapse_preview(s: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut buf = String::with_capacity(s.len().min(MAX_CHARS + 4));
    let mut prev_space = false;
    for ch in s.chars() {
        if buf.chars().count() >= MAX_CHARS {
            buf.push('\u{2026}'); // "…"
            return buf;
        }
        if ch.is_whitespace() {
            if !prev_space && !buf.is_empty() {
                buf.push(' ');
                prev_space = true;
            }
        } else {
            buf.push(ch);
            prev_space = false;
        }
    }
    buf
}

#[cfg(test)]
mod recent_sessions_tests {
    use super::*;
    use crate::backend::storage::filestore::FileStore;

    fn fresh_filestore() -> std::sync::Arc<FileStore> {
        std::sync::Arc::new(FileStore::open_in_memory().unwrap())
    }

    fn write_snapshot(fs: &FileStore, block_id: &str, body: &str) {
        // make_file then write_file mirrors the production
        // BlockfileWriteState handler path.
        let meta: crate::backend::storage::filestore::FileMeta =
            std::collections::HashMap::new();
        let opts = crate::backend::storage::filestore::FileOpts::default();
        fs.make_file(block_id, "output.state.json", meta, opts)
            .expect("make_file");
        fs.write_file(block_id, "output.state.json", body.as_bytes())
            .expect("write_file");
    }

    #[test]
    fn collapse_preview_strips_newlines_and_caps_length() {
        let s = "hello\n\nworld\n  next   line";
        assert_eq!(collapse_preview(s), "hello world next line");
        let long: String = "a".repeat(500);
        let out = collapse_preview(&long);
        // 240 chars + ellipsis.
        assert!(out.ends_with('\u{2026}'));
        assert!(out.chars().count() <= 241);
    }

    #[test]
    fn read_session_preview_missing_returns_zero() {
        let fs = fresh_filestore();
        let (preview, count) = read_session_preview(&fs, "no-such-block");
        assert_eq!(preview, "");
        assert_eq!(count, 0);
    }

    #[test]
    fn read_session_preview_extracts_first_user_message_skipping_context() {
        let fs = fresh_filestore();
        // Two user messages: first is the boilerplate Session Context;
        // second is the user's real prompt. Preview should be the real one.
        let snapshot = serde_json::json!({
            "schemaVersion": 1,
            "savedAt": "2026-05-23T08:00:00Z",
            "highWaterMark": 169,
            "historyOffset": 0,
            "nodes": [
                {
                    "type": "user_message",
                    "id": "u0",
                    "timestamp": 0,
                    "collapsed": false,
                    "summary": "👤 User Message",
                    "message": "# Session Context\nIdentity: Claude\n## Description\nStartup boilerplate"
                },
                { "type": "markdown", "id": "m0", "content": "ack" },
                {
                    "type": "user_message",
                    "id": "u1",
                    "timestamp": 100,
                    "collapsed": false,
                    "summary": "👤 User Message",
                    "message": "check the agentmuxai/agentmux history, get the latest code"
                }
            ]
        });
        write_snapshot(&fs, "blk-1", &snapshot.to_string());
        let (preview, count) = read_session_preview(&fs, "blk-1");
        assert_eq!(count, 3);
        assert!(preview.starts_with("check the agentmuxai/agentmux"));
    }

    #[test]
    fn read_session_preview_falls_back_to_session_context_when_only_one() {
        let fs = fresh_filestore();
        let snapshot = serde_json::json!({
            "schemaVersion": 1,
            "nodes": [
                {
                    "type": "user_message",
                    "id": "u0",
                    "message": "# Session Context\nIdentity: Claude\nStartup boilerplate"
                }
            ]
        });
        write_snapshot(&fs, "blk-2", &snapshot.to_string());
        let (preview, count) = read_session_preview(&fs, "blk-2");
        assert_eq!(count, 1);
        // Newlines collapsed; starts with the boilerplate marker.
        assert!(preview.starts_with("# Session Context"));
    }

    #[test]
    fn read_session_preview_handles_malformed_json() {
        let fs = fresh_filestore();
        write_snapshot(&fs, "blk-3", "not valid json {");
        let (preview, count) = read_session_preview(&fs, "blk-3");
        assert_eq!(preview, "");
        assert_eq!(count, 0);
    }

    #[test]
    fn read_session_preview_handles_no_user_messages() {
        let fs = fresh_filestore();
        let snapshot = serde_json::json!({
            "schemaVersion": 1,
            "nodes": [
                { "type": "markdown", "id": "m0", "content": "system note" }
            ]
        });
        write_snapshot(&fs, "blk-4", &snapshot.to_string());
        let (preview, count) = read_session_preview(&fs, "blk-4");
        assert_eq!(preview, "");
        assert_eq!(count, 1);
    }

    // ── Integration test: full listrecentsessions handler ────────────
    //
    // Spins up the same engine + state shape as the production
    // websocket path so the handler runs end-to-end against an
    // in-memory wstore + filestore. Asserts the row shape, the
    // identity filter, the snapshot-first sort, the preview extraction,
    // and the cross-version "no snapshot" fallback. This is the
    // backend correctness gate for the AgentPicker's Recent Sessions
    // surface (cascade follow-up 2026-05-23).
    use crate::backend::storage::store::{
        AgentDefinition, AgentInstance, Identity, InstanceStatus, Memory, Store,
    };
    use crate::backend::rpc::engine::WshRpcEngine;
    use crate::server::AppState;
    use std::sync::Arc;

    /// Drive a single RPC round-trip against the in-memory engine,
    /// asserting success + deserializing the JSON payload into `T`.
    async fn call_rpc<T: serde::de::DeserializeOwned>(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>,
        command: &str,
        data: serde_json::Value,
    ) -> T {
        let req_id = format!("test-{}", uuid::Uuid::new_v4());
        let msg = crate::backend::rpc_types::RpcMessage {
            command: command.to_string(),
            reqid: req_id.clone(),
            data: Some(data),
            ..Default::default()
        };
        engine.handle_message(msg);
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("handler timed out")
            .expect("output channel closed");
        assert_eq!(resp.resid, req_id, "unexpected response id");
        assert!(resp.error.is_empty(), "handler returned error: {}", resp.error);
        let payload = resp.data.unwrap_or(serde_json::Value::Null);
        serde_json::from_value(payload).expect("response deserialize")
    }

    fn build_state_with_seed() -> (
        AppState,
        Arc<WshRpcEngine>,
        tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>,
    ) {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let event_bus = Arc::new(crate::backend::eventbus::EventBus::new());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let reactive_handler = crate::backend::reactive::get_global_handler();
        let poller = Arc::new(crate::backend::reactive::Poller::new(
            crate::backend::reactive::PollerConfig {
                agentmux_url: None,
                agentmux_token: None,
                poll_interval_secs: 30,
            },
            reactive_handler,
        ));
        crate::backend::wcore::ensure_initial_data(&wstore).unwrap();
        let config_watcher = Arc::new(crate::backend::wconfig::ConfigWatcher::new());
        let process_tracker = Arc::new(
            crate::backend::process_tracker::registry::AgentProcessRegistry::new(Some(broker.clone())),
        );
        let state = AppState {
            auth_key: "test".to_string(),
            version: "test".to_string(),
            app_path: String::new(),
            wstore: wstore.clone(),
            filestore: filestore.clone(),
            global_transcript_store: None,
            event_bus: event_bus.clone(),
            broker,
            reactive_handler,
            poller,
            config_watcher,
            messagebus: Arc::new(crate::backend::messagebus::MessageBus::new()),
            http_client: reqwest::Client::new(),
            local_web_url: String::new(),
            subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus.clone())),
            history_service: Arc::new(crate::backend::history::HistoryService::new()),
            lan_discovery: Arc::new(crate::backend::lan_discovery::LanDiscoveryController::new(
                "test-instance".to_string(),
                "test-host".to_string(),
                "0.28.20".to_string(),
                0,
                event_bus.clone(),
                String::new(),
            )),
            lsp_supervisor: Arc::new(crate::backend::lsp::LspSupervisor::new(event_bus.clone())),
            process_tracker,
            srv_state: Arc::new(tokio::sync::Mutex::new(crate::state::State::default())),
            srv_events_tx: tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(64).0,
            saga_id_alloc: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            saga_log: Arc::new(crate::sagas::log::SagaLog::open_in_memory().unwrap()),
            auth_session_manager: Arc::new(crate::identity::auth_session::AuthSessionManager::new()),
            install_sessions: crate::server::install_handlers::InstallSessionRegistry::new(),
            container_manager: None,
            shell_sessions: crate::backend::shell_node::ShellSessionRegistry::new(),
        };

        // Seed: 1 SEEDED definition (template), 1 identity bundle, 1
        // memory bundle. Phase 3b note: seeded as a template so that
        // each instance projection in `db_agents` lands on its own row
        // (`is_template = 0`, `id = inst.id`, `parent_template_id =
        // def.id`) rather than folding into the def-projection and
        // clobbering its name. The handler resolves `definition_name`
        // via `defs.iter().find(|d| d.id == inst.definition_id)`, which
        // hits the template row and returns "Claude Code". Under the
        // pre-Phase 3b reader, def name was always preserved because
        // `agent_def_list` queried `db_agent_definitions` directly;
        // db_agents fold semantics require the seed shape to avoid
        // the collision.
        let def = AgentDefinition {
            id: "def-claude".to_string(),
            slug: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
        };
        let mut def_mut = def.clone();
        wstore.agent_def_insert(&mut def_mut).unwrap();
        let identity = Identity {
            id: "id-work".to_string(),
            name: "Work".to_string(),
            description: String::new(),
            is_blank: false,
            created_at: 0,
            updated_at: 0,
        };
        wstore.bundle_identity_upsert(&identity).unwrap();
        let memory = Memory {
            id: "mem-notes".to_string(),
            name: "Notes".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        };
        wstore.bundle_memory_upsert(&memory).unwrap();

        // 3 instances:
        //   - blk-recent: has snapshot, more recent activity
        //   - blk-older:  has snapshot, older activity
        //   - blk-none:   no snapshot at all (legacy / pre-persistence row)
        // All three use the same identity bundle so the filter test
        // can also exercise it without re-seeding.
        for (id, block, started) in [
            ("inst-recent", "blk-recent", 1_700_000_100_000_i64),
            ("inst-older", "blk-older", 1_700_000_000_000_i64),
            ("inst-none", "blk-none", 1_700_000_050_000_i64),
        ] {
            let inst = AgentInstance {
                id: id.to_string(),
                definition_id: "def-claude".to_string(),
                parent_instance_id: String::new(),
                block_id: block.to_string(),
                session_id: String::new(),
                status: InstanceStatus::Running.as_str().to_string(),
                github_context: String::new(),
                started_at: started,
                ended_at: 0,
                created_at: started,
                identity_id: "id-work".to_string(),
                memory_id: "mem-notes".to_string(),
                instance_name: format!("name-{id}"),
                working_directory: format!("/tmp/{id}"),
                display_hidden: false,
            };
            wstore.instance_create(&inst).unwrap();
        }

        // Snapshots for the two with snapshots. Write the OLDER one
        // first so its filestore-stamped modts is strictly less than the
        // recent one — the handler sorts snapshot-bearing rows by modts
        // desc, so writing blk-older second would invert the assertions.
        // (Pre-Phase 3b this ordering was fragile because the dual-write
        // chain ran fewer SQL statements between successive inserts, so
        // adjacent writes landed in the same millisecond and the stable
        // sort preserved instance_list_named's started_at order; now the
        // additional db_agents UPDATE per instance widens the gap and
        // distinct modts dominate the stable sort.)
        let snap_older = serde_json::json!({
            "schemaVersion": 1,
            "nodes": [
                {"type": "user_message", "id": "u0",
                 "message": "earlier conversation"}
            ]
        });
        write_snapshot(&filestore, "blk-older", &snap_older.to_string());
        let snap_recent = serde_json::json!({
            "schemaVersion": 1,
            "nodes": [
                {"type": "user_message", "id": "u0",
                 "message": "# Session Context\nboilerplate"},
                {"type": "markdown", "id": "m0", "content": "ack"},
                {"type": "user_message", "id": "u1",
                 "message": "fix the live-feed hover delay"}
            ]
        });
        write_snapshot(&filestore, "blk-recent", &snap_recent.to_string());

        let (engine, rx) = WshRpcEngine::new();
        super::register_agent_handlers(&engine, &state);
        (state, engine, rx)
    }

    #[tokio::test]
    async fn handler_returns_sessions_with_previews_sorted_by_snapshot_first() {
        let (_state, engine, mut rx) = build_state_with_seed();
        let rows: Vec<RecentSessionRow> = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(rows.len(), 3, "all three sessions surfaced");

        // Sort: snapshot-bearing rows first (recent then older), then
        // the no-snapshot row at the tail.
        assert_eq!(rows[0].instance_id, "inst-recent");
        assert!(rows[0].has_snapshot);
        assert_eq!(rows[0].node_count, 3);
        assert!(
            rows[0].preview.starts_with("fix the live-feed"),
            "preview should be the post-context user message, got {:?}",
            rows[0].preview
        );

        assert_eq!(rows[1].instance_id, "inst-older");
        assert!(rows[1].has_snapshot);
        assert_eq!(rows[1].node_count, 1);
        assert_eq!(rows[1].preview, "earlier conversation");

        assert_eq!(rows[2].instance_id, "inst-none");
        assert!(!rows[2].has_snapshot);
        assert_eq!(rows[2].node_count, 0);
        assert_eq!(rows[2].preview, "");

        // Joins: definition + identity + memory names resolved.
        assert_eq!(rows[0].definition_name, "Claude Code");
        assert_eq!(rows[0].identity_name, "Work");
        assert_eq!(rows[0].memory_name, "Notes");
        assert_eq!(rows[0].block_id_hint, "blk-recent");
    }

    #[tokio::test]
    async fn handler_identity_filter_restricts_rows() {
        let (_state, engine, mut rx) = build_state_with_seed();
        // Filter to a non-existent identity → empty list.
        let rows: Vec<RecentSessionRow> = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({ "identity_id": "no-such-bundle" }),
        )
        .await;
        assert_eq!(rows.len(), 0);

        // Filter to the seeded one → all three.
        let rows: Vec<RecentSessionRow> = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({ "identity_id": "id-work" }),
        )
        .await;
        assert_eq!(rows.len(), 3);

        // Empty-string identity_id is treated as "no filter" so the
        // frontend can pass `""` without special-casing.
        let rows: Vec<RecentSessionRow> = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({ "identity_id": "" }),
        )
        .await;
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn handler_respects_limit() {
        let (_state, engine, mut rx) = build_state_with_seed();
        let rows: Vec<RecentSessionRow> = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({ "limit": 1 }),
        )
        .await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].instance_id, "inst-recent");
    }

    // ---- Two-tier picker Phase 1: create-from-template + listagents filter ----
    //
    // SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md.

    /// Same shape as build_state_with_seed but with a seeded template
    /// and no instances, so the create-from-template path is exercised
    /// against a known-good template row.
    fn build_state_with_template_seed() -> (
        AppState,
        Arc<WshRpcEngine>,
        tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>,
    ) {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let event_bus = Arc::new(crate::backend::eventbus::EventBus::new());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let reactive_handler = crate::backend::reactive::get_global_handler();
        let poller = Arc::new(crate::backend::reactive::Poller::new(
            crate::backend::reactive::PollerConfig {
                agentmux_url: None,
                agentmux_token: None,
                poll_interval_secs: 30,
            },
            reactive_handler,
        ));
        crate::backend::wcore::ensure_initial_data(&wstore).unwrap();
        let config_watcher = Arc::new(crate::backend::wconfig::ConfigWatcher::new());
        let process_tracker = Arc::new(
            crate::backend::process_tracker::registry::AgentProcessRegistry::new(Some(broker.clone())),
        );
        let state = AppState {
            auth_key: "test".to_string(),
            version: "test".to_string(),
            app_path: String::new(),
            wstore: wstore.clone(),
            filestore: filestore.clone(),
            global_transcript_store: None,
            event_bus: event_bus.clone(),
            broker,
            reactive_handler,
            poller,
            config_watcher,
            messagebus: Arc::new(crate::backend::messagebus::MessageBus::new()),
            http_client: reqwest::Client::new(),
            local_web_url: String::new(),
            subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus.clone())),
            history_service: Arc::new(crate::backend::history::HistoryService::new()),
            lan_discovery: Arc::new(crate::backend::lan_discovery::LanDiscoveryController::new(
                "test-instance".to_string(),
                "test-host".to_string(),
                "0.28.20".to_string(),
                0,
                event_bus.clone(),
                String::new(),
            )),
            lsp_supervisor: Arc::new(crate::backend::lsp::LspSupervisor::new(event_bus.clone())),
            process_tracker,
            srv_state: Arc::new(tokio::sync::Mutex::new(crate::state::State::default())),
            srv_events_tx: tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(64).0,
            saga_id_alloc: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            saga_log: Arc::new(crate::sagas::log::SagaLog::open_in_memory().unwrap()),
            auth_session_manager: Arc::new(crate::identity::auth_session::AuthSessionManager::new()),
            install_sessions: crate::server::install_handlers::InstallSessionRegistry::new(),
            container_manager: None,
            shell_sessions: crate::backend::shell_node::ShellSessionRegistry::new(),
        };

        // One seeded template + one already-user-owned definition.
        let mut tpl = AgentDefinition {
            id: "tpl-claude".to_string(),
            slug: String::new(),
            name: "Claude Code".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: "Anthropic's coding agent".to_string(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: "--model haiku".to_string(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
        };
        wstore.agent_def_insert(&mut tpl).unwrap();

        let mut user_a = AgentDefinition {
            id: "user-a".to_string(),
            slug: String::new(),
            name: "Maks".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_001_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_001_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
        };
        wstore.agent_def_insert(&mut user_a).unwrap();

        let (engine, rx) = WshRpcEngine::new();
        super::register_agent_handlers(&engine, &state);
        (state, engine, rx)
    }

    #[tokio::test]
    async fn listagents_no_filter_returns_all() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        assert!(agents.iter().any(|a| a.id == "tpl-claude"));
        assert!(agents.iter().any(|a| a.id == "user-a"));
    }

    #[tokio::test]
    async fn listagents_filter_templates_only() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "is_seeded": 1 }),
        )
        .await;
        assert!(agents.iter().all(|a| a.is_seeded == 1));
        assert!(agents.iter().any(|a| a.id == "tpl-claude"));
        assert!(!agents.iter().any(|a| a.id == "user-a"));
    }

    #[tokio::test]
    async fn listagents_filter_user_owned_only() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "is_seeded": 0 }),
        )
        .await;
        assert!(agents.iter().all(|a| a.is_seeded == 0));
        assert!(agents.iter().any(|a| a.id == "user-a"));
        assert!(!agents.iter().any(|a| a.id == "tpl-claude"));
    }

    #[tokio::test]
    async fn create_from_template_happy_path_clones_and_returns_id() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let resp: crate::backend::rpc_types::AgentDefCreateFromTemplateResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "tpl-claude",
                "name": "Asaf",
                "identity_id": "id-work",
                "memory_id": "mem-notes",
            }),
        )
        .await;
        assert!(!resp.definition_id.is_empty());
        assert_eq!(resp.identity_id, "id-work");
        assert_eq!(resp.memory_id, "mem-notes");

        // The new row is user-owned, carries provider + flags from template.
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        let new_def = agents
            .iter()
            .find(|a| a.id == resp.definition_id)
            .expect("new definition should appear in listagents");
        assert_eq!(new_def.is_seeded, 0);
        assert_eq!(new_def.name, "Asaf");
        assert_eq!(new_def.provider, "claude");
        assert_eq!(new_def.provider_flags, "--model haiku");
        assert_eq!(new_def.parent_id, "tpl-claude");
    }

    async fn call_rpc_expect_error(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>,
        command: &str,
        data: serde_json::Value,
    ) -> String {
        let req_id = format!("test-{}", uuid::Uuid::new_v4());
        let msg = crate::backend::rpc_types::RpcMessage {
            command: command.to_string(),
            reqid: req_id.clone(),
            data: Some(data),
            ..Default::default()
        };
        engine.handle_message(msg);
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("handler timed out")
            .expect("output channel closed");
        assert_eq!(resp.resid, req_id);
        assert!(
            !resp.error.is_empty(),
            "expected error, got success payload: {:?}",
            resp.data
        );
        resp.error
    }

    #[tokio::test]
    async fn create_from_template_rejects_non_template_id() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        // "user-a" is is_seeded=0 — not a template.
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "user-a",
                "name": "another",
            }),
        )
        .await;
        assert!(
            err.contains("not a seeded template"),
            "wrong error: {err}"
        );
    }

    #[tokio::test]
    async fn create_from_template_rejects_unknown_template_id() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "no-such-id",
                "name": "x",
            }),
        )
        .await;
        assert!(err.contains("not found"), "wrong error: {err}");
    }

    #[tokio::test]
    async fn create_from_template_rejects_duplicate_user_name() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        // "Maks" already exists as a user-owned agent.
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "tpl-claude",
                "name": "Maks",
            }),
        )
        .await;
        assert!(
            err.contains("already exists"),
            "wrong error: {err}"
        );
    }

    #[tokio::test]
    async fn create_from_template_rejects_empty_name() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "tpl-claude",
                "name": "   ",
            }),
        )
        .await;
        assert!(err.contains("non-empty"), "wrong error: {err}");
    }

    // ---- Two-tier picker Phase 2: hide / unhide templates ----
    //
    // SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md Q2 Decision Y.

    #[tokio::test]
    async fn hide_template_then_listagents_excludes_it_by_default() {
        let (_state, engine, mut rx) = build_state_with_template_seed();

        // Before hide: template is in the default listagents result.
        let before: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        assert!(before.iter().any(|a| a.id == "tpl-claude"));

        // Hide the template.
        let resp: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        assert!(resp.ok);

        // After hide: default listagents no longer surfaces it.
        let after: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        assert!(
            !after.iter().any(|a| a.id == "tpl-claude"),
            "hidden template should NOT appear by default",
        );

        // But user-owned rows (is_seeded=0) still appear — hide only
        // affects templates.
        assert!(after.iter().any(|a| a.id == "user-a"));

        // include_hidden = true brings it back (settings panel surface).
        let included: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "include_hidden": true }),
        )
        .await;
        let tpl = included
            .iter()
            .find(|a| a.id == "tpl-claude")
            .expect("hidden template should appear with include_hidden=true");
        assert_eq!(tpl.user_hidden, 1);
    }

    #[tokio::test]
    async fn hide_then_unhide_round_trip() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let _: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        let resp: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_UNHIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        assert!(resp.ok);
        // Listagents now shows it again, default-filter included.
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        let tpl = agents
            .iter()
            .find(|a| a.id == "tpl-claude")
            .expect("unhidden template should appear");
        assert_eq!(tpl.user_hidden, 0);
    }

    #[tokio::test]
    async fn hide_rejects_user_owned_definition() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        // "user-a" is is_seeded=0 — hide must reject.
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "user-a" }),
        )
        .await;
        assert!(
            err.contains("not a seeded template"),
            "wrong error: {err}"
        );
    }

    #[tokio::test]
    async fn hide_unknown_id_returns_ok_false() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let resp: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "no-such-id" }),
        )
        .await;
        assert!(!resp.ok);
    }

    #[tokio::test]
    async fn list_hidden_templates_returns_only_hidden_templates() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        // Empty initially.
        let empty: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES,
            serde_json::json!({}),
        )
        .await;
        assert!(empty.is_empty());

        // Hide one; expect it to surface.
        let _: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        let hidden: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].id, "tpl-claude");
        assert_eq!(hidden[0].is_seeded, 1);
        assert_eq!(hidden[0].user_hidden, 1);
    }

    #[tokio::test]
    async fn listagents_is_seeded_filter_with_include_hidden_combines() {
        // Templates-only filter + include_hidden = the settings panel's
        // canonical query if it ever wanted the full template universe.
        // Without include_hidden + is_seeded=1 the hidden ones drop out.
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let _: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        let templates_visible: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "is_seeded": 1 }),
        )
        .await;
        assert!(
            !templates_visible.iter().any(|a| a.id == "tpl-claude"),
            "hidden template should be excluded from is_seeded=1 default query",
        );
        let templates_all: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "is_seeded": 1, "include_hidden": true }),
        )
        .await;
        assert!(templates_all.iter().any(|a| a.id == "tpl-claude"));
    }
}
