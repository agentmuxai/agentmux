// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Making a user-owned agent from a seeded template: the core of the `agentdefcreatefromtemplate` RPC
//! (Phase 1 of the two-tier picker, SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md), shared with the `Agents create`
//! API (`app_api/agent_spawn/create.rs`).
//!
//! The template stays pristine; the new row carries `is_seeded = 0`. Validation:
//!  - `template_id` must resolve to a row with `is_seeded = 1`. Cloning a user-owned row would be confusing
//!    semantics; use `forkagentdefinition` for that.
//!  - `name` is non-empty, at most 200 characters, and not already taken by any `is_seeded = 0` row.

use crate::backend::mps::Broker;
use crate::backend::rpc_types::{AgentDefCreateFromTemplateResult, CommandAgentDefCreateFromTemplateData};
use crate::backend::storage::store::Store;
use crate::backend::storage::AgentDefinition;

use super::template::{former_name_of_another, name_taken};

/// Clone the template `cmd.template_id` into a new agent named `cmd.name`.
pub(crate) fn create_from_template(
    mstore: &Store,
    id_store: &Store,
    broker: &Broker,
    cmd: CommandAgentDefCreateFromTemplateData,
) -> Result<AgentDefCreateFromTemplateResult, String> {
    let name = cmd.name.trim().to_string();
    if name.is_empty() {
        return Err("agentdefcreatefromtemplate: name must be non-empty".into());
    }
    if name.chars().count() > 200 {
        return Err(
            "agentdefcreatefromtemplate: name must be ≤200 characters".into(),
        );
    }

    let all = mstore
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
    if name_taken(&all, &name) {
        return Err(format!(
            "agentdefcreatefromtemplate: an agent named {:?} already exists",
            name
        ));
    }
    if former_name_of_another(&mstore, &name, "") {
        return Err(format!("agentdefcreatefromtemplate: {name:?} still addresses another agent"));
    }

    let now = agentmux_common::time::now_ms();
    // Runtime is the user's instantiation-time choice, not a
    // template property. When supplied, the clone records it
    // (and the matching `environment`); empty falls back to
    // the template's value for back-compat with older callers.
    let chosen_agent_type = match cmd.agent_type.as_deref().unwrap_or("").trim() {
        "host" | "container" => cmd.agent_type.as_deref().unwrap_or("").trim().to_string(),
        _ => template.agent_type.clone(),
    };
    let chosen_environment = if chosen_agent_type == "container" {
        "docker".to_string()
    } else {
        "local".to_string()
    };
    // Template's EFFECTIVE provider, from the shared resolver
    // (#2594; SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.1). Used for both the vendor-base-url
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
    let chosen_container_image = super::template_image::chosen_container_image(cmd.container_image.as_deref(), &chosen_agent_type, &template.container_image)
        .map_err(|e| format!("agentdefcreatefromtemplate: {e}"))?;
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
        // The template's image unless the picker chose one (above), so container-type
        // templates still propagate theirs to user-cloned agents.
        container_image: chosen_container_image,
        container_volumes: template.container_volumes.clone(),
        container_name: String::new(),
        use_ambient_login: 0,
        model_vendor_base_url: chosen_model_vendor_base_url,
        memory_id: String::new(),
        // Fail-closed default, not inherited from the template —
        // same convention as auto_start/use_ambient_login above
        // (opt-in settings reset on a fresh clone, not carried
        // over).
        seat: None,
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
    };
    mstore
        .agent_def_insert(&mut new_def)
        .map_err(|e| format!("agentdefcreatefromtemplate: insert: {e}"))?;
    // Own dedicated ABF bundle, not the template's — every
    // agent has its own (ARCHITECTURE_MANDATORY_ABF_RETHINK_
    // 2026_08_14.md §3.2, "strong reading").
    mstore.agent_def_provision_and_bind_bundle(&id_store, &mut new_def, now);
    // After the own bundle exists, so a pick of it is dropped.
    if let Some(bundle_ids) = cmd.bundle_ids.as_deref().filter(|ids| !ids.is_empty()) {
        mstore
            .agent_bundles_set(&new_def.id, bundle_ids)
            .map_err(|e| format!("agentdefcreatefromtemplate: bundles: {e}"))?;
    }

    broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);

    let resp = AgentDefCreateFromTemplateResult {
        definition_id: new_def.id.clone(),
        identity_id: cmd.identity_id.unwrap_or_default(),
        memory_id: cmd.memory_id.unwrap_or_default(),
        slug: new_def.slug.clone(),
    };
    tracing::info!(
        template_id = %cmd.template_id,
        new_definition_id = %new_def.id,
        new_name = %new_def.name,
        "agentdefcreatefromtemplate: cloned template into user agent"
    );
    Ok(resp)
}
