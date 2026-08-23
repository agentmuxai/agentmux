// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};


use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
    CommandAgentDefCreateFromTemplateData, AgentDefCreateFromTemplateResult,
    // Two-tier picker — Phase 2 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md
    // Q2 Decision Y: hide templates).
    COMMAND_AGENT_DEF_HIDE, COMMAND_AGENT_DEF_UNHIDE,
    COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES,
    CommandAgentDefHideData, AgentDefHideResult,
    COMMAND_FORK_AGENT_DEFINITION,
    COMMAND_FORK_AGENT_DEFINITION_SUGGEST,
    CommandForkAgentDefinitionSuggestData, ForkAgentDefinitionSuggestResult,
    CommandForkAgentDefinitionData,
    COMMAND_RENAME_AGENT_DEFINITION_TITLE,
    CommandRenameAgentDefinitionTitleData,
};
use crate::backend::storage::{AgentDefinition, AgentContent, AgentSkill};

use super::super::AppState;

/// Resolve a definition's **fork**-lineage root by walking `parent_id`.
///
/// `parent_id` is overloaded in this schema: `agentdefcreatefromtemplate`
/// sets it to the *template's* id for every freshly-instantiated user
/// agent (see that handler below), not just `forkagentdefinition`'s actual
/// forks — so two unrelated agents both created fresh from the same
/// template share a `parent_id`, but are not forks of each other. The one
/// field that reliably distinguishes an actual fork is `branch_label`:
/// `forkagentdefinition` always sets it (non-empty); `agentdefcreatefromtemplate`
/// always leaves it empty. Walk upward only while the current node is
/// itself a fork (non-empty `branch_label`); stop at the first non-fork
/// ancestor (a template, or a first-generation user agent) — that's the
/// lineage's root for fork-counting purposes. Confirmed bug this fixes
/// (Codex's review of PR #2721, docs/specs/SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md
/// §2): the naming rule requires a flat, lineage-wide counter ("AgentX #2"
/// → "AgentX #3"), but the previous `parent_id == source_id` filter only
/// counted immediate children, so forking a fork produced "AgentX #2 #2".
fn fork_lineage_root_id(defs: &[AgentDefinition], start_id: &str) -> String {
    let mut current = start_id.to_string();
    loop {
        match defs.iter().find(|a| a.id == current) {
            Some(def) if !def.branch_label.is_empty() && !def.parent_id.is_empty() => {
                current = def.parent_id.clone();
            }
            _ => return current,
        }
    }
}

/// Build the flat, lineage-wide "#N" auto-suggestion for forking `source`.
///
/// Two bugs, both from the same root cause (no lineage-root resolution),
/// fixed together: (1) the existing-fork count must include every fork
/// anywhere in the lineage, not just `source`'s immediate children —
/// otherwise forking a fork undercounts and can repeat a number; (2) the
/// suggested name must be built from the lineage **root's** name, not
/// `source`'s own (possibly already-suffixed) name — otherwise forking
/// "AgentX #2" produces "AgentX #2 #3" instead of the flat "AgentX #3"
/// `SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md` §4.5
/// requires. If the root can't be found (should not happen — `source`
/// itself is always in `defs`), falls back to `source`'s own name rather
/// than panicking.
fn suggest_fork_name(defs: &[AgentDefinition], source: &AgentDefinition) -> String {
    let root_id = fork_lineage_root_id(defs, &source.id);
    let root_name = defs
        .iter()
        .find(|a| a.id == root_id)
        .map(|a| a.name.as_str())
        .unwrap_or(source.name.as_str());
    let existing_fork_count = defs
        .iter()
        .filter(|a| {
            a.is_seeded == 0 && !a.branch_label.is_empty() && fork_lineage_root_id(defs, &a.id) == root_id
        })
        .count();
    format!("{root_name} #{}", existing_fork_count + 2)
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
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
    let id_store_act = state.id_store.clone();
    let broker_act = state.broker.clone();
    engine.register_handler(
        COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_act.clone();
            let id_store = id_store_act.clone();
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
                // Template's EFFECTIVE provider — resolved through its own
                // bound bundle when it has one, not the possibly-drifted
                // `db_agent_definitions.provider` column directly (#2594,
                // same "gate vs. actual launch can disagree" risk class
                // #2592 fixed). Used for both the vendor-base-url
                // validation below and the clone's own `provider` field so
                // the two can't disagree with each other.
                let effective_provider = id_store.resolve_effective_provider_id(template);
                // Model vendor base URL: `None` (omitted) inherits the
                // template's own value (the only behavior before this
                // field existed on this RPC); `Some(url)` overrides it,
                // including `Some("")` to explicitly clear a
                // template-inherited override. Same validation `agent.define`
                // already applies — rejected unless the template's provider
                // declares `base_url_env_var`.
                let chosen_model_vendor_base_url = match &cmd.model_vendor_base_url {
                    Some(url) => {
                        crate::server::app_api::validate_vendor_base_url(&effective_provider, url)
                            .map_err(|e| format!("agentdefcreatefromtemplate: {e}"))?;
                        url.clone()
                    }
                    None => template.model_vendor_base_url.clone(),
                };
                let mut new_def = AgentDefinition {
                    id: uuid::Uuid::new_v4().to_string(),
                    // agent_def_insert derives a unique slug from the
                    // name when this is empty + collision-resolves.
                    slug: String::new(),
                    name: name.clone(),
                    icon: template.icon.clone(),
                    provider: effective_provider.clone(),
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
                    use_ambient_login: 0,
                    model_vendor_base_url: chosen_model_vendor_base_url,
                    auto_continue_enabled: 0,
                    memory_id: String::new(),
                    // Fail-closed default, not inherited from the template —
                    // same convention as auto_start/use_ambient_login/
                    // auto_continue_enabled above (opt-in settings reset on
                    // a fresh clone, not carried over).
                    conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
                };
                wstore
                    .agent_def_insert(&mut new_def)
                    .map_err(|e| format!("agentdefcreatefromtemplate: insert: {e}"))?;
                // Own dedicated ABF bundle, not the template's — every
                // agent has its own (ARCHITECTURE_MANDATORY_ABF_RETHINK_
                // 2026_08_14.md §3.2, "strong reading").
                wstore.agent_def_provision_and_bind_bundle(&id_store, &mut new_def, now);

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

    // ---- Definition fork ----

    let wstore = state.wstore.clone();
    let id_store = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_FORK_AGENT_DEFINITION,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let id_store = id_store.clone();
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
                // When empty, auto-generate a flat, lineage-wide "Name #N"
                // (see suggest_fork_name's doc comment).
                let fork_name = if cmd.branch_label.is_empty() {
                    suggest_fork_name(&all_defs, &source)
                } else {
                    cmd.branch_label.clone()
                };
                let branch_label = if cmd.branch_label.is_empty() {
                    fork_name.clone()
                } else {
                    cmd.branch_label.clone()
                };
                let branch_slug_part = crate::backend::storage::store::derive_slug(&branch_label);
                // Source's EFFECTIVE provider — resolved through its own
                // bound bundle when it has one, not the possibly-drifted
                // `db_agent_definitions.provider` column directly (#2594,
                // same pattern as the create-from-template clone site).
                let effective_provider = id_store.resolve_effective_provider_id(&source);
                let mut fork = AgentDefinition {
                    id: uuid::Uuid::new_v4().to_string(),
                    // Empty slug → agent_def_insert derives + resolves collisions.
                    slug: format!("{}-{}", source.slug, branch_slug_part),
                    name: fork_name,
                    icon: source.icon.clone(),
                    provider: effective_provider,
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
                    use_ambient_login: 0,
                    model_vendor_base_url: source.model_vendor_base_url.clone(),
                    auto_continue_enabled: 0,
                    memory_id: String::new(),
                    // Fail-closed default, not inherited from source — same
                    // convention as use_ambient_login/auto_continue_enabled
                    // above (opt-in settings reset on a fork, not carried
                    // over, even though other config like container image
                    // and model vendor IS inherited).
                    conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
                };
                wstore
                    .agent_def_insert(&mut fork)
                    .map_err(|e| format!("forkagentdefinition: {e}"))?;
                // Own dedicated ABF bundle, not the source's — same "every
                // agent has its own" rule as the template-clone path above.
                wstore.agent_def_provision_and_bind_bundle(&id_store, &mut fork, now);

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

                let suggested_label = suggest_fork_name(&all, source);

                let result = ForkAgentDefinitionSuggestResult { suggested_label };
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );

    // ---- Rename a fork/agent tab's displayed title ----
    //
    // Deliberately separate from `updateagent`, which preserves
    // `branch_label` unconditionally (core.rs: "parent_id + branch_label
    // describe provenance and are immutable post-insert"). That contract
    // stays true for every OTHER caller of `updateagent` — this handler is
    // the one narrow, explicit path that's allowed to change it, and only
    // the field `fork-set.ts`'s `titleOf()` actually displays: `branch_label`
    // when the row already has one (a fork), else `name` (a lineage root).
    // See SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md §4.
    let wstore_rn = state.wstore.clone();
    let broker_rn = state.broker.clone();
    engine.register_handler(
        COMMAND_RENAME_AGENT_DEFINITION_TITLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore_rn.clone();
            let broker = broker_rn.clone();
            Box::pin(async move {
                let cmd: CommandRenameAgentDefinitionTitleData = serde_json::from_value(data)
                    .map_err(|e| format!("renameagentdefinitiontitle: {e}"))?;
                let title = cmd.title.trim();
                if title.is_empty() {
                    return Err("renameagentdefinitiontitle: title must not be empty".to_string());
                }

                let all = wstore
                    .agent_def_list()
                    .map_err(|e| format!("renameagentdefinitiontitle: {e}"))?;
                let old = all
                    .iter()
                    .find(|a| a.id == cmd.id)
                    .ok_or_else(|| format!("renameagentdefinitiontitle: agent {} not found", cmd.id))?;

                let mut updated = old.clone();
                if !updated.branch_label.is_empty() {
                    updated.branch_label = title.to_string();
                } else {
                    updated.name = title.to_string();
                }

                let found = wstore
                    .agent_def_update(&mut updated)
                    .map_err(|e| format!("renameagentdefinitiontitle: {e}"))?;
                if !found {
                    return Err(format!("renameagentdefinitiontitle: agent {} not found", cmd.id));
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });

                Ok(Some(serde_json::to_value(&updated).unwrap_or_default()))
            })
        }),
    );

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::RpcMessage;
    use crate::backend::storage::Memory;
    use crate::server::tests::test_state;

    fn seed_bundle(state: &AppState, id: &str, provider: &str) {
        let bundle = Memory {
            id: id.to_string(),
            name: "Bundle".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: provider.to_string(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        };
        state.id_store.bundle_memory_upsert(&bundle).unwrap();
    }

    // Template's own `.provider` column says "codex" (drifted/stale —
    // simulates the same drift class #2592 fixed: some definition-time
    // write path changed this column after the bundle was already
    // provisioned/immutable), but its bound bundle's REAL provider is
    // "claude". A correct clone/fork must carry "claude", not "codex".
    fn seed_drifted_template(state: &AppState, def_id: &str, bundle_id: &str) {
        let mut def = AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: def_id.to_string(),
            slug: String::new(),
            name: "Drifted Template".to_string(),
            icon: String::new(),
            provider: "codex".to_string(),
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
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: bundle_id.to_string(),
        };
        state.wstore.agent_def_insert(&mut def).unwrap();
    }

    #[tokio::test]
    async fn agentdefcreatefromtemplate_resolves_provider_through_the_templates_bundle() {
        let state = test_state();
        seed_bundle(&state, "bundle-claude", "claude");
        seed_drifted_template(&state, "tpl-1", "bundle-claude");

        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);

        engine.handle_message(RpcMessage {
            command: COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE.to_string(),
            reqid: "req-1".to_string(),
            data: Some(serde_json::json!({
                "template_id": "tpl-1",
                "name": "My Clone",
            })),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: AgentDefCreateFromTemplateResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();

        let cloned = state
            .wstore
            .agent_def_get(&result.definition_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            cloned.provider, "claude",
            "clone must carry the template's REAL (bundle-resolved) provider, not the drifted `codex` column"
        );
    }

    #[tokio::test]
    async fn forkagentdefinition_resolves_provider_through_the_sources_bundle() {
        let state = test_state();
        seed_bundle(&state, "bundle-claude", "claude");
        seed_drifted_template(&state, "src-1", "bundle-claude");

        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);

        engine.handle_message(RpcMessage {
            command: COMMAND_FORK_AGENT_DEFINITION.to_string(),
            reqid: "req-1".to_string(),
            data: Some(serde_json::json!({
                "source_id": "src-1",
                "branch_label": "",
            })),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let fork: AgentDefinition =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();

        assert_eq!(
            fork.provider, "claude",
            "fork must carry the source's REAL (bundle-resolved) provider, not the drifted `codex` column"
        );
    }

    // #2721 Phase 1 (Codex's review) — the auto-suggested name must be a
    // flat, lineage-wide counter off the ROOT's name, not the immediate
    // parent's (possibly already-suffixed) name. Forking a fork used to
    // produce "Drifted Template #2 #2" (immediate-parent-only count) or
    // even "Drifted Template #2 #3" (root-based count, but still built off
    // the parent's own name) — the correct result is the flat
    // "Drifted Template #3".
    #[tokio::test]
    async fn forking_a_fork_produces_a_flat_lineage_wide_name() {
        let state = test_state();
        seed_bundle(&state, "bundle-claude", "claude");
        seed_drifted_template(&state, "root-1", "bundle-claude");

        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);

        let fork_once = |engine: &Arc<WshRpcEngine>, source_id: &str| {
            engine.handle_message(RpcMessage {
                command: COMMAND_FORK_AGENT_DEFINITION.to_string(),
                reqid: "req".to_string(),
                data: Some(serde_json::json!({ "source_id": source_id, "branch_label": "" })),
                ..Default::default()
            });
        };

        fork_once(&engine, "root-1");
        let resp1 = tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(resp1.error.is_empty(), "unexpected error: {}", resp1.error);
        let fork1: AgentDefinition = serde_json::from_value(resp1.data.expect("expected result data")).unwrap();
        assert_eq!(fork1.name, "Drifted Template #2");

        fork_once(&engine, &fork1.id);
        let resp2 = tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(resp2.error.is_empty(), "unexpected error: {}", resp2.error);
        let fork2: AgentDefinition = serde_json::from_value(resp2.data.expect("expected result data")).unwrap();
        assert_eq!(
            fork2.name, "Drifted Template #3",
            "forking a fork must produce a flat, lineage-wide name — not \"#2 #2\" (immediate-parent-only \
             count) or \"#2 #3\" (root-based count off the parent's own already-suffixed name)"
        );

        // The suggest RPC (used to preview the name before the user
        // confirms) must agree with what an actual fork would produce.
        engine.handle_message(RpcMessage {
            command: COMMAND_FORK_AGENT_DEFINITION_SUGGEST.to_string(),
            reqid: "req-suggest".to_string(),
            data: Some(serde_json::json!({ "source_id": fork2.id })),
            ..Default::default()
        });
        let resp3 = tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(resp3.error.is_empty(), "unexpected error: {}", resp3.error);
        let suggestion: ForkAgentDefinitionSuggestResult =
            serde_json::from_value(resp3.data.expect("expected result data")).unwrap();
        assert_eq!(suggestion.suggested_label, "Drifted Template #4");
    }
}
