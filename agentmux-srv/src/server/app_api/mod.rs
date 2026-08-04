// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App API — high-level commands for programmatic control of AgentMux.
//!
//! These commands orchestrate multiple low-level operations (CreateBlock, SetMeta,
//! ControllerResync) behind stable, intent-based interfaces. Callers express what
//! they want ("open an agent pane with AgentX"), not how to do it.

use std::sync::Arc;

use base64::Engine;
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::obj::{self, Block, Tab, Workspace, MetaMapType};
use crate::backend::providers;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;
use crate::backend::session_archive;
use crate::backend::storage::store::{Store, AgentContent, AgentDefinition, AgentInstance};
use crate::backend::storage::identities::IdentityAccount;
use crate::backend::storage::memory_bundles::Memory;

use super::AppState;
use crate::server::cli_handlers::resolve_cli_on_path;

mod agent_open;
mod agent_io;
mod agent_define;
mod pane;
mod blockfile;
pub(crate) mod session;
mod identity;
mod bundle;
mod memory;
mod skill;
mod mcp;

/// Register all App API handlers on the RPC engine.
pub fn register_app_api_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    agent_open::register(engine, state);
    agent_io::register(engine, state);
    agent_define::register(engine, state);
    pane::register(engine, state);
    blockfile::register(engine, state);
    session::register(engine, state);
    identity::register(engine, state);
    bundle::register(engine, state);
    memory::register(engine, state);
    skill::register(engine, state);
    mcp::register(engine, state);
}

/// Core `pane.open` logic, shared by the WebSocket RPC handler
/// (`register_pane_open`) and the HTTP route `POST /api/v1/pane/open`
/// (`agentmux-mcp`'s `OpenEditor` tool). Creates a block for the requested
/// view, enqueues a layout action (split or insert), and broadcasts the
/// block/tab/layout updates so the frontend renders the new pane.
pub async fn open_pane(state: &AppState, cmd: CommandPaneOpenData) -> Result<PaneOpenResult, String> {
    let wstore = state.wstore.clone();
    let event_bus = state.event_bus.clone();

    tracing::info!(view = %cmd.view, "pane.open");

    // Use caller-supplied meta when present (widget bar path: full blockdef.meta
    // already known); otherwise derive from view + args via build_pane_meta.
    let meta = match cmd.meta {
        Some(m) => m,
        None => pane::build_pane_meta(&cmd)?,
    };

    // Editor-pane reuse (SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md
    // Part 2): if the calling agent already has an Editor pane open in its own
    // tab, add the requested file as a new tab in that pane instead of always
    // spawning another Editor pane. Gated on the explicit `reuse_editor_pane`
    // opt-in only — NOT inferred from `meta`/`split_reference_block_id` being
    // present, since `EditorViewModel.openToTheSide`/`openInTerminal` also set
    // `split_reference_block_id` to their OWN block id for split placement and
    // must not trigger reuse (reagent P1 on PR #2404 caught an earlier version
    // of this check incorrectly reusing the calling pane itself for
    // `openToTheSide`). Also excludes `floating` requests — those always get
    // their own new window (codex P1 on PR #2404: this branch previously ran
    // before the floating check below and silently swallowed floating
    // requests into a reused docked pane).
    if cmd.view == "editor" && cmd.reuse_editor_pane == Some(true) && cmd.floating != Some(true) {
        if let (Some(caller_block_id), Some(file)) =
            (cmd.split_reference_block_id.as_deref(), cmd.file.as_deref())
        {
            if let Some(result) =
                pane::maybe_reuse_editor_pane(state, caller_block_id, file, cmd.focus).await?
            {
                return Ok(result);
            }
        }
    }

    // Resolve tab: explicit tab_id wins; otherwise, if the caller told us
    // which block to place this pane relative to (split_reference_block_id),
    // resolve THAT block's own tab rather than falling back to "whichever
    // tab happens to be globally active" — a caller specifying a relative
    // block virtually always means "my own tab" (codex P2 on PR #2404: the
    // editor-reuse check above already resolves this correctly-scoped tab
    // for its own lookup, but previously discarded it whenever reuse didn't
    // apply — e.g. a floating request, or no existing Editor pane yet —
    // silently falling through to the flakier "first workspace's active
    // tab" heuristic for the actual block creation/placement below, which
    // can place a pane in the wrong tab entirely in a multi-tab setup).
    let tab_id = if let Some(explicit) = cmd.tab_id.as_deref() {
        explicit.to_string()
    } else if let Some(derived) = cmd
        .split_reference_block_id
        .as_deref()
        .and_then(|id| resolve_tab_id_for_block(&wstore, id).ok())
    {
        derived
    } else {
        resolve_tab_id(&wstore, None)?
    };

    // Floating path (SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16):
    // create the block in a fresh floating workspace (via reducer CreateBlock +
    // the existing tear_off_block saga) and signal the source window's frontend
    // to materialize the chromeless OS window — srv can't open windows itself.
    if cmd.floating == Some(true) {
        return pane::open_pane_floating(state, &wstore, &event_bus, cmd.view, tab_id, meta).await;
    }

    // Skip-placement path (in-pane tabs — SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md
    // §4.2): create the block through the reducer, same as the docked path
    // below, but return immediately — no layout action, no tear_off_block
    // saga. The caller attaches it to an existing pane's block-stack instead
    // (`pushBlockOntoStack`), so it must never render docked or floating
    // first.
    if cmd.skip_placement == Some(true) {
        let meta_val = serde_json::to_value(&meta)
            .map_err(|e| format!("pane.open: skip_placement: meta serialize: {e}"))?;
        let create_events = crate::server::service::dispatch_to_reducer(
            state,
            agentmux_common::ipc::Command::CreateBlock {
                tab_id: tab_id.clone(),
                meta: meta_val,
            },
        )
        .await;
        if let Some(msg) = create_events.iter().find_map(|e| match e {
            agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
            _ => None,
        }) {
            return Err(format!("pane.open: skip_placement: CreateBlock: {msg}"));
        }
        let block_id = create_events
            .iter()
            .find_map(|e| match e {
                agentmux_common::ipc::Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                _ => None,
            })
            .ok_or_else(|| "pane.open: skip_placement: CreateBlock emitted no BlockCreated".to_string())?;
        for ev in &create_events {
            if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, &wstore) {
                tracing::warn!("pane.open: skip_placement: CreateBlock wstore apply failed: {e}");
            }
        }
        crate::server::service::publish_events(state, &create_events);
        tracing::info!(block_id = %block_id, view = %cmd.view, "pane.open: block created, placement skipped");
        return Ok(PaneOpenResult {
            block_id,
            tab_id,
            view: cmd.view,
            created: true,
        });
    }

    // Create block (docked path) THROUGH THE REDUCER (#1681), not wcore-direct.
    // A store-only `create_block` lands the block in SQLite but never in the
    // reducer-canonical `state.blocks` map — and this RPC runs after bootstrap,
    // which is the only time `srv_state` is hydrated from SQLite. The pane then
    // renders fine (frontend reads SQLite) but a later TearOffBlock /
    // RedockFloatingPane is rejected "block not found" because the saga
    // pre-conditions check the reducer. The BlockCreated event carries meta,
    // which apply_block_created writes to the wstore Block. Mirrors the
    // already-correct open_pane_floating path.
    let meta_val = serde_json::to_value(&meta)
        .map_err(|e| format!("pane.open: meta serialize: {e}"))?;
    let create_events = crate::server::service::dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CreateBlock {
            tab_id: tab_id.clone(),
            meta: meta_val,
        },
    )
    .await;
    if let Some(msg) = create_events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(format!("pane.open: CreateBlock: {msg}"));
    }
    let block_id = create_events
        .iter()
        .find_map(|e| match e {
            agentmux_common::ipc::Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .ok_or_else(|| "pane.open: CreateBlock emitted no BlockCreated".to_string())?;
    for ev in &create_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, &wstore) {
            tracing::warn!("pane.open: CreateBlock wstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &create_events);

    // Enqueue layout action — split if requested, else append
    let (actiontype, targetblockid, position) = pane::resolve_placement(
        cmd.split_direction.as_deref(),
        cmd.split_reference_block_id.as_deref(),
    );
    let focused = cmd.focus.unwrap_or(true);

    // SPEC_864 Phase 4 — append through the reducer (single writer of
    // db_layout). Best-effort like the store-direct write it replaces:
    // a failure leaves the block created but not laid out.
    {
        let action = obj::LayoutActionData {
            actiontype,
            actionid: uuid::Uuid::new_v4().to_string(),
            blockid: block_id.clone(),
            nodesize: None,
            nodesizefraction: None,
            indexarr: None,
            focused,
            magnified: false,
            ephemeral: false,
            targetblockid,
            position,
        };
        if let Err(e) = crate::server::service::queue_layout_actions_via_reducer(
            state,
            &tab_id,
            vec![action],
        )
        .await
        {
            tracing::warn!("pane.open: layout action enqueue failed: {e}");
        }
    }

    tracing::info!(
        block_id = %block_id,
        view = %cmd.view,
        "pane.open: block created + layout updated"
    );

    // Broadcast block + tab + layout updates
    {
        let mut updates = Vec::new();
        if let Ok(updated_block) = wstore.must_get::<Block>(&block_id) {
            updates.push(obj::WaveObjUpdate {
                updatetype: "update".into(),
                otype: "block".into(),
                oid: block_id.clone(),
                obj: Some(obj::wave_obj_to_value(&updated_block)),
            });
        }
        if let Ok(updated_tab) = wstore.must_get::<Tab>(&tab_id) {
            updates.push(obj::WaveObjUpdate {
                updatetype: "update".into(),
                otype: "tab".into(),
                oid: tab_id.clone(),
                obj: Some(obj::wave_obj_to_value(&updated_tab)),
            });
            if let Ok(updated_layout) = wstore.must_get::<obj::LayoutState>(&updated_tab.layoutstate) {
                updates.push(obj::WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: "layout".into(),
                    oid: updated_tab.layoutstate.clone(),
                    obj: Some(obj::wave_obj_to_value(&updated_layout)),
                });
            }
        }
        for update in &updates {
            let oref = format!("{}:{}", update.otype, update.oid);
            if let Ok(data) = serde_json::to_value(update) {
                event_bus.broadcast_event(
                    &crate::backend::eventbus::WSEventType {
                        eventtype: "waveobj:update".to_string(),
                        oref,
                        data: Some(data),
                    },
                );
            }
        }
    }

    Ok(PaneOpenResult {
        block_id,
        tab_id,
        view: cmd.view,
        created: true,
    })
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
    wstore: Arc<Store>,
    broker: Arc<crate::backend::wps::Broker>,
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

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

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
    };

    // Atomic check-then-insert.
    // Returns Some(existing) if a row matched by name/slug already exists;
    // None if the row was freshly inserted (def.slug now holds resolved slug).
    let existing_opt = wstore.agent_def_find_or_insert(&mut def)
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
                    match agent_define::make_stub_idempotent(&wstore, &existing.id, &existing.name, now) {
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
                    broker.publish(crate::backend::wps::WaveEvent {
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
                if !cmd.provider.is_empty() || !cmd.model.is_empty() { updated.provider = provider.clone(); }
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
                if !cmd.environment.is_empty() { updated.environment = cmd.environment.clone(); }
                // name update intentionally omitted — the slug is immutable;
                // renaming would create a slug mismatch. Use updateagent for renames.
                let did_update = wstore.agent_def_update(&mut updated)
                    .map_err(|e| format!("agent.define: update: {e}"))?;
                if !did_update {
                    return Err("agent.define: update: row was deleted between find and update".to_string());
                }
                agent_define::persist_define_content(&wstore, &updated.id, &cmd, now);
                let stub_id = if create_stub {
                    match agent_define::make_stub_idempotent(&wstore, &updated.id, &updated.name, now) {
                        Ok((id, _new)) => Some(id),
                        Err(e) => {
                            tracing::warn!(id = %updated.id, err = %e, "agent.define: update stub failed (non-fatal)");
                            None
                        }
                    }
                } else {
                    None
                };
                broker.publish(crate::backend::wps::WaveEvent {
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
    // Create the stub first so that listeners handling agents:changed can
    // immediately find the new agent via ListRecentSessionsCommand. The
    // definition is already committed; a stub failure is non-fatal (log +
    // continue) and we still broadcast so callers see the new definition.
    let stub_id = if create_stub {
        match agent_define::make_stub_idempotent(&wstore, &def.id, &def.name, now) {
            Ok((id, _new)) => Some(id),
            Err(e) => {
                tracing::warn!(id = %def.id, err = %e, "agent.define: stub failed (definition committed, non-fatal)");
                None
            }
        }
    } else {
        None
    };
    broker.publish(crate::backend::wps::WaveEvent {
        event: "agents:changed".to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: None,
    });
    agent_define::persist_define_content(&wstore, &def.id, &cmd, now);

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

pub(crate) async fn identity_self_accounts_impl(
    state: &AppState,
    agent_id: &str,
) -> Result<serde_json::Value, String> {
    // Link rows are keyed by definition id, not the S1 slug callers
    // authenticate with — see resolve_agent_definition_id.
    let def_id = resolve_agent_definition_id(state, agent_id)
        .map_err(|e| format!("identity.self.accounts: {e}"))?;
    let links = state.id_store.agent_identity_list_for_agent(&def_id)
        .map_err(|e| format!("identity.self.accounts: {e}"))?;
    let mut accounts = Vec::new();
    for link in &links {
        if let Some(acct) = state.id_store.identity_get(&link.account_id)
            .map_err(|e| format!("identity.self.accounts: {e}"))? {
            let masked_tail = acct.context.get("masked_tail")
                .and_then(|v| v.as_str()).unwrap_or("").to_string();
            accounts.push(json!({
                "account_id": acct.id, "provider": acct.provider, "name": acct.name,
                "kind": acct.kind, "status": acct.status, "masked_tail": masked_tail,
                "updated_at": acct.updated_at,
            }));
        }
    }
    Ok(json!({ "accounts": accounts }))
}

/// Validate one of the agent's own linked accounts by probing the provider with
/// the stored keychain secret. Ownership is verified (the account must be linked
/// to `agent_id`) before the secret is read.
pub(crate) async fn identity_account_validate_stored_impl(
    state: &AppState,
    agent_id: &str,
    account_id: &str,
) -> Result<serde_json::Value, String> {
    // Link rows are keyed by definition id, not the S1 slug — without the
    // resolution this ownership check always saw zero links and rejected.
    let def_id = resolve_agent_definition_id(state, agent_id)
        .map_err(|e| format!("identity.account.validate: {e}"))?;
    let links = state.id_store.agent_identity_list_for_agent(&def_id)
        .map_err(|e| format!("identity.account.validate: {e}"))?;
    if !links.iter().any(|l| l.account_id == account_id) {
        return Err("FORBIDDEN: account not linked to this agent".to_string());
    }
    let acct = state.id_store.identity_get(account_id)
        .map_err(|e| format!("identity.account.validate: {e}"))?
        .ok_or_else(|| format!("identity.account.validate: account {account_id} not found"))?;
    let aid = account_id.to_string();
    let plaintext = tokio::task::spawn_blocking(move || crate::identity::secret_store::get(&aid))
        .await.map_err(|e| format!("identity.account.validate: keychain task: {e}"))?
        .map_err(|e| format!("identity.account.validate: keychain: {e}"))?;
    let tail = crate::identity::key_validator::masked_tail(&plaintext);
    let outcome = crate::identity::key_validator::validate(&acct.provider, &plaintext).await;
    Ok(json!({
        "valid": outcome.valid,
        "status": if outcome.valid { "valid" } else { "invalid" },
        "masked_tail": tail,
        "error": outcome.error,
    }))
}

pub(crate) async fn bundle_list_impl(state: &AppState) -> Result<serde_json::Value, String> {
    let memories = state.id_store.bundle_memory_list().map_err(|e| format!("bundle.list: {e}"))?;
    let bundles: Vec<_> = memories.iter().map(|m| json!({
        "id": m.id, "name": m.name, "description": m.description,
        "provider": m.provider, "model": m.model, "is_blank": m.is_blank, "updated_at": m.updated_at,
    })).collect();
    // Emit both keys during the Preset→Bundle alias window: new callers of
    // `bundle.list` read `bundles`; the retained `preset.list` alias's existing
    // agent/REST consumers still read `presets`. Drop the `presets` key in
    // Phase 4 when the alias is removed (SPEC_PRESET_TO_BUNDLE_REFACTOR §2.1/§4.4).
    Ok(json!({ "bundles": bundles, "presets": bundles }))
}

pub(crate) async fn bundle_get_impl(
    state: &AppState,
    id: &str,
    name: &str,
) -> Result<serde_json::Value, String> {
    let memory = if !id.is_empty() {
        state.id_store.bundle_memory_get(id).map_err(|e| format!("bundle.get: {e}"))?
            .ok_or_else(|| format!("bundle.get: not found id={id}"))?
    } else if !name.is_empty() {
        let all = state.id_store.bundle_memory_list().map_err(|e| format!("bundle.get: {e}"))?;
        all.into_iter().filter(|m| m.name == name).max_by_key(|m| m.updated_at)
            .ok_or_else(|| format!("bundle.get: not found name={name}"))?
    } else {
        return Err("bundle.get: provide id or name".to_string());
    };
    serde_json::to_value(&memory).map_err(|e| e.to_string())
}

pub(crate) async fn bundle_self_get_impl(
    state: &AppState,
    agent_id: &str,
) -> Result<serde_json::Value, String> {
    let instance = state.wstore.instance_get_by_name(agent_id)
        .map_err(|e| format!("bundle.self.get: {e}"))?;
    let memory_id = instance.as_ref()
        .and_then(|i| if i.memory_id.is_empty() { None } else { Some(i.memory_id.clone()) });
    let memory = if let Some(mid) = memory_id {
        state.id_store.bundle_memory_get(&mid).map_err(|e| format!("bundle.self.get: {e}"))?
            .ok_or_else(|| format!("bundle.self.get: memory_id {mid} not found"))?
    } else {
        // No bundle bound: return the blank singleton (two-step — list to find
        // the blank id, then fetch the full object).
        let all = state.id_store.bundle_memory_list().map_err(|e| format!("bundle.self.get: {e}"))?;
        let blank_id = all.into_iter().find(|m| m.is_blank).map(|m| m.id)
            .ok_or_else(|| "bundle.self.get: blank singleton not found".to_string())?;
        state.id_store.bundle_memory_get(&blank_id).map_err(|e| format!("bundle.self.get: {e}"))?
            .ok_or_else(|| "bundle.self.get: blank singleton row missing".to_string())?
    };
    serde_json::to_value(&memory).map_err(|e| e.to_string())
}

pub(crate) fn memory_list_impl(
    state: &AppState,
    agent_id: &str,
) -> Result<serde_json::Value, String> {
    let memory_dir = crate::server::native_memory_handlers::memory_dir_for_agent(
        &state.wstore, agent_id,
    ).map_err(|e| format!("memory.list: {e}"))?;

    let mut files: Vec<NativeMemoryFileMeta> = Vec::new();
    let entries = match std::fs::read_dir(&memory_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({ "files": [] }));
        }
        Err(e) => return Err(format!("memory.list: read_dir: {e}")),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("memory.list: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".md") { continue; }
        let file_type = entry.file_type()
            .map_err(|e| format!("memory.list: file_type {name}: {e}"))?;
        if !file_type.is_file() { continue; }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("memory.list: metadata {name}: {e}")),
        };
        let preview_content = {
            use std::io::Read;
            std::fs::File::open(entry.path())
                .map(|f| {
                    let mut buf = Vec::with_capacity(512);
                    f.take(512).read_to_end(&mut buf).ok();
                    String::from_utf8_lossy(&buf).into_owned()
                })
                .unwrap_or_default()
        };
        files.push(NativeMemoryFileMeta {
            is_index: name == "MEMORY.md",
            metadata_type: crate::server::native_memory_handlers::parse_memory_frontmatter_type(&preview_content),
            size_bytes: meta.len(),
            modified_at: meta.modified().ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            filename: name,
        });
    }
    files.sort_by(|a, b| b.is_index.cmp(&a.is_index).then(a.filename.cmp(&b.filename)));
    serde_json::to_value(NativeMemoryListResult { files }).map_err(|e| e.to_string())
}

pub(crate) fn memory_read_impl(
    state: &AppState,
    agent_id: &str,
    filename: &str,
) -> Result<serde_json::Value, String> {
    crate::server::native_memory_handlers::validate_memory_filename(filename)
        .map_err(|e| format!("memory.read: {e}"))?;
    let path = crate::server::native_memory_handlers::memory_dir_for_agent(
        &state.wstore, agent_id,
    ).map_err(|e| format!("memory.read: {e}"))?.join(filename);

    let file_type = std::fs::symlink_metadata(&path)
        .map_err(|e| format!("memory.read: {filename}: {e}"))?.file_type();
    if !file_type.is_file() {
        return Err(format!("memory.read: {filename} is not a regular file"));
    }
    const MAX: u64 = 10 * 1024 * 1024;
    let mut buf = Vec::new();
    std::fs::File::open(&path)
        .and_then(|f| { use std::io::Read; f.take(MAX).read_to_end(&mut buf) })
        .map_err(|e| format!("memory.read: {filename}: {e}"))?;
    let content = String::from_utf8_lossy(&buf).into_owned();
    serde_json::to_value(NativeMemoryReadFileResult { content }).map_err(|e| e.to_string())
}

pub(crate) fn memory_write_impl(
    state: &AppState,
    agent_id: &str,
    filename: &str,
    content: &str,
) -> Result<(), String> {
    crate::server::native_memory_handlers::validate_memory_filename(filename)
        .map_err(|e| format!("memory.write: {e}"))?;
    const MAX: usize = 10 * 1024 * 1024;
    if content.len() > MAX {
        return Err(format!("memory.write: content too large ({} bytes, max {MAX})", content.len()));
    }
    let dir = crate::server::native_memory_handlers::memory_dir_for_agent(
        &state.wstore, agent_id,
    ).map_err(|e| format!("memory.write: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("memory.write: mkdir: {e}"))?;

    let dest = dir.join(filename);
    let tmp = dir.join(format!(".{}.{}.tmp", filename, uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&tmp, content) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("memory.write: write tmp: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("memory.write: rename: {e}"));
    }
    state.broker.publish(crate::backend::wps::WaveEvent {
        event: format!("agent:memory:changed:{agent_id}"),
        scopes: vec![], sender: String::new(), persist: 0, data: None,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers used by submodules (via `use super::*`)
// ---------------------------------------------------------------------------

/// If this channel has no local `output` for `block_id` but the agent's GLOBAL
/// transcript zone (`agent:<defId>:current`) does, return `(global_store,
/// agent_zone)`. Returns `None` when the local output is present and non-empty,
/// the block isn't agent-anchored, there's no global store, or the global zone
/// is empty — callers then read the per-channel store keyed by `block_id`.
///
/// Only the agent `output` stream is globalized; every other file stays local.
pub(super) fn global_output_source(
    _per_channel: &Arc<crate::backend::storage::filestore::FileStore>,
    global: &Option<Arc<crate::backend::storage::filestore::FileStore>>,
    wstore: &Arc<crate::backend::storage::store::Store>,
    block_id: &str,
    filename: &str,
) -> Option<(Arc<crate::backend::storage::filestore::FileStore>, String)> {
    if filename != crate::backend::agent_session::OUTPUT_FILE {
        return None;
    }
    let gfs = global.as_ref()?;
    let block = wstore.get::<Block>(block_id).ok().flatten()?;
    let archived = block
        .meta
        .get(crate::backend::session_archive::META_SESSION_ARCHIVED_AT)
        .and_then(|v| v.as_i64())
        .map(|v| v > 0)
        .unwrap_or(false);
    if archived {
        return None;
    }
    let zone = crate::backend::agent_session::agent_zone_for_block_meta(&block.meta)?;
    match gfs.stat(&zone, filename) {
        Ok(Some(ref wf)) if wf.size > 0 => Some((gfs.clone(), zone)),
        _ => None,
    }
}

/// Exact non-blank line count of the `output` file in `zone`, computed via the
/// same streaming index builder `read_range` uses (so the two endpoints always
/// agree). Returns `Some(0)` for an empty file, `None` on read failure.
pub(super) fn global_zone_line_count(
    gfs: &Arc<crate::backend::storage::filestore::FileStore>,
    zone: &str,
) -> Option<u64> {
    let stat = gfs
        .stat(zone, crate::backend::agent_session::OUTPUT_FILE)
        .ok()??;
    if stat.size == 0 {
        return Some(0);
    }
    crate::backend::blockcontroller::shell::rebuild_output_idx(gfs, zone, stat.size as u64)
}

/// Resolve a tab ID: use the provided one, or fall back to the first workspace's active tab.
pub(super) fn resolve_tab_id(wstore: &Store, explicit: Option<&str>) -> Result<String, String> {
    if let Some(tid) = explicit {
        return Ok(tid.to_string());
    }

    // Fall back to first workspace's active tab
    let workspaces: Vec<Workspace> = wstore.get_all::<Workspace>()
        .map_err(|e| format!("agent.open: list workspaces: {e}"))?;

    for ws in &workspaces {
        if !ws.activetabid.is_empty() {
            return Ok(ws.activetabid.clone());
        }
        if let Some(first_tab) = ws.tabids.first() {
            return Ok(first_tab.clone());
        }
    }

    Err("no tabs found in any workspace".to_string())
}

/// Find an existing agent block in a tab by agent ID.
pub(super) fn find_agent_block(wstore: &Store, tab_id: &str, agent_id: &str) -> Result<Option<Block>, String> {
    let tab: Tab = wstore.must_get(tab_id)
        .map_err(|e| format!("TAB_NOT_FOUND: {e}"))?;

    for block_id in &tab.blockids {
        if let Ok(Some(block)) = wstore.get::<Block>(block_id) {
            let block_agent_id = obj::meta_get_string(&block.meta, "agentId", "");
            if block_agent_id == agent_id {
                return Ok(Some(block));
            }
        }
    }
    Ok(None)
}

/// Find which tab a given block currently belongs to, by scanning every
/// tab's `blockids`. Needed because `resolve_tab_id` only resolves an
/// explicit tab_id or "the active tab" — neither answers "which tab is MY
/// OWN block in," which a caller passing its own block id (e.g. an MCP
/// tool's `split_reference_block_id`) actually needs.
pub(super) fn resolve_tab_id_for_block(wstore: &Store, block_id: &str) -> Result<String, String> {
    let tabs: Vec<Tab> = wstore.get_all::<Tab>()
        .map_err(|e| format!("resolve_tab_id_for_block: list tabs: {e}"))?;
    for tab in tabs {
        if tab.blockids.iter().any(|b| b == block_id) {
            return Ok(tab.oid);
        }
    }
    Err(format!("resolve_tab_id_for_block: block {block_id} not found in any tab"))
}

/// Find an existing Editor-view block in a tab, if any. Direct sibling of
/// `find_agent_block` above, checking `meta.view == "editor"` instead of
/// `meta.agentId`.
pub(super) fn find_editor_block(wstore: &Store, tab_id: &str) -> Result<Option<Block>, String> {
    let tab: Tab = wstore.must_get(tab_id)
        .map_err(|e| format!("TAB_NOT_FOUND: {e}"))?;

    for block_id in &tab.blockids {
        if let Ok(Some(block)) = wstore.get::<Block>(block_id) {
            if obj::meta_get_string(&block.meta, "view", "") == "editor" {
                return Ok(Some(block));
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// S1 enforcement helper
// ---------------------------------------------------------------------------

pub(super) fn check_s1(ctx: &RpcContext, req_agent_id: &str) -> Result<(), String> {
    if ctx.agent_id.is_empty() {
        return Err("FORBIDDEN: unauthenticated agent connection".to_string());
    }
    if ctx.agent_id != req_agent_id {
        return Err("FORBIDDEN: agent_id mismatch".to_string());
    }
    Ok(())
}

/// Resolve an S1-authenticated agent id (the slug — AGENTMUX_AGENT_ID /
/// bus:register id, e.g. "Agent3") to the agent's DEFINITION id, which is
/// what `db_agent_identity_links.agent_id` stores (== `AgentDefinition.id`;
/// see m0013 and `identity/resolver.rs::resolve_bindings_for_instance`).
///
/// Every link-table operation reached from the App API must go through
/// this: App API callers authenticate with the slug, but writing the slug
/// into the link table either trips the per-channel schema's
/// `FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id)` (loud
/// "FOREIGN KEY constraint failed" — the id_store fallback path when the
/// 0011 shared-store backfill hasn't applied), or — on the shared store,
/// whose links table carries no agent_id FK — silently writes a row keyed
/// by slug that the resolver (which reads by definition id) can never
/// match. Reads have the mirror-image bug: listing links by slug always
/// returns empty, so ownership checks reject and unlinks no-op.
///
/// A caller that already holds a definition id passes through unchanged
/// (verified against `agent_def_get`), so internal non-S1 callers of the
/// shared `*_impl` helpers stay valid.
pub(super) fn resolve_agent_definition_id(
    state: &AppState,
    agent_id: &str,
) -> Result<String, String> {
    if let Ok(Some(instance)) = state.wstore.instance_get_by_name(agent_id) {
        if !instance.definition_id.is_empty() {
            return Ok(instance.definition_id);
        }
    }
    if let Ok(Some(_)) = state.wstore.agent_def_get(agent_id) {
        return Ok(agent_id.to_string());
    }
    Err(format!(
        "unknown agent '{agent_id}': no instance with that name and no definition with that id"
    ))
}

/// Current unix time in milliseconds (0 if the clock is before the epoch).
pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Parse + validate a saved per-agent `ui:zoom` content blob for seeding a new
/// agent block's `term:zoom`. Returns `Some(z)` only for a parseable,
/// non-default (≠ 1.0), in-[0.5, 2.0] zoom (the range the frontend enforces in
/// term.tsx); anything else (default, out of range, garbage) returns `None` so
/// the new block opens at the default 1.0. See SPEC_AGENT_ZOOM_PERSISTENCE §4.2.
pub(super) fn parse_seed_zoom(raw: &str) -> Option<f64> {
    let z = raw.trim().parse::<f64>().ok()?;
    if (z - 1.0).abs() > f64::EPSILON && (0.5..=2.0).contains(&z) {
        Some(z)
    } else {
        None
    }
}

#[cfg(test)]
mod cross_channel_tests {
    use super::*;
    use crate::backend::agent_session::OUTPUT_FILE;
    use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

    fn mem_store() -> Arc<FileStore> {
        Arc::new(FileStore::open_in_memory().unwrap())
    }

    fn seed_output(fs: &Arc<FileStore>, zone: &str, body: &[u8]) {
        fs.make_file(zone, OUTPUT_FILE, FileMeta::default(), FileOpts::default())
            .unwrap();
        fs.append_data(zone, OUTPUT_FILE, body).unwrap();
    }

    fn insert_agent_block(wstore: &Arc<Store>, def_id: &str) -> String {
        let oid = uuid::Uuid::new_v4().to_string();
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        meta.insert("agentId".to_string(), serde_json::json!(def_id));
        let mut block = Block {
            oid: oid.clone(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta,
            subblockids: None,
        };
        wstore.insert(&mut block).expect("insert block");
        oid
    }

    #[test]
    fn global_output_source_falls_back_when_local_empty() {
        let per_channel = mem_store();
        let global = mem_store();
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let block_id = insert_agent_block(&wstore, "def-cc-1");

        // No local output for the block, but the global zone has content.
        seed_output(&global, "agent:def-cc-1:current", b"{\"type\":\"user\"}\n");

        let resolved = global_output_source(
            &per_channel,
            &Some(global.clone()),
            &wstore,
            &block_id,
            "output",
        );
        let (_store, zone) = resolved.expect("should fall back to global");
        assert_eq!(zone, "agent:def-cc-1:current");
    }

    #[test]
    fn global_output_source_prefers_global_even_when_local_present() {
        // After the Bug-1 fix: even when the local output is non-empty (current
        // session started writing), the global zone is still returned so that
        // cross-channel history load sees the FULL record, not just the current
        // session's lines.
        let per_channel = mem_store();
        let global = mem_store();
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let block_id = insert_agent_block(&wstore, "def-cc-2");

        seed_output(&per_channel, &block_id, b"{\"type\":\"local\"}\n");
        seed_output(&global, "agent:def-cc-2:current", b"{\"type\":\"global\"}\n");

        let resolved = global_output_source(
            &per_channel,
            &Some(global.clone()),
            &wstore,
            &block_id,
            "output",
        );
        let (_, zone) = resolved.expect("global always preferred when available");
        assert_eq!(zone, "agent:def-cc-2:current");
    }

    #[test]
    fn global_output_source_only_for_output_and_with_global_store() {
        let per_channel = mem_store();
        let global = mem_store();
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let block_id = insert_agent_block(&wstore, "def-cc-3");
        seed_output(&global, "agent:def-cc-3:current", b"{\"x\":1}\n");

        // Non-"output" filename is never globalized.
        assert!(global_output_source(&per_channel, &Some(global.clone()), &wstore, &block_id, "term").is_none());
        // No global store configured → None.
        assert!(global_output_source(&per_channel, &None, &wstore, &block_id, "output").is_none());
        // Non-agent block id → None.
        assert!(global_output_source(&per_channel, &Some(global), &wstore, "not-a-block", "output").is_none());
    }

    #[test]
    fn global_output_source_suppressed_for_archived_block() {
        // A block archived via the UI/sweep (`session:archived_at` set, local
        // output deleted) must NOT resurrect from the global mirror — it should
        // reopen archived/empty as pre-PR. (reagent P1 #1399.)
        let per_channel = mem_store();
        let global = mem_store();
        let wstore = Arc::new(Store::open_in_memory().unwrap());

        let oid = uuid::Uuid::new_v4().to_string();
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        meta.insert("agentId".to_string(), serde_json::json!("def-cc-arch"));
        meta.insert(
            crate::backend::session_archive::META_SESSION_ARCHIVED_AT.to_string(),
            serde_json::json!(1_700_000_000_000i64),
        );
        let mut block = Block {
            oid: oid.clone(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta,
            subblockids: None,
        };
        wstore.insert(&mut block).expect("insert block");

        // Global zone has content, but the block is archived → no fallback.
        seed_output(&global, "agent:def-cc-arch:current", b"{\"type\":\"user\"}\n");
        assert!(
            global_output_source(&per_channel, &Some(global), &wstore, &oid, "output").is_none(),
            "archived block must not fall back to the global mirror",
        );
    }

    #[test]
    fn global_zone_line_count_counts_non_blank_lines() {
        let global = mem_store();
        let zone = "agent:def-cc-4:current";
        seed_output(&global, zone, b"{\"a\":1}\n{\"b\":2}\n\n{\"c\":3}\n");
        // 3 non-blank NDJSON lines (the blank line is ignored, matching read_range).
        assert_eq!(global_zone_line_count(&global, zone), Some(3));

        // Empty / absent zone → Some(0) / None respectively.
        let empty_zone = "agent:def-empty:current";
        assert_eq!(global_zone_line_count(&global, empty_zone), None);
    }
}


#[cfg(test)]
mod pane_open_reducer_tests {
    use super::*;
    use crate::backend::rpc_types::CommandPaneOpenData;
    use crate::server::tests::test_state;
    use agentmux_common::ipc::{Command, Event};

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &evs {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        evs
    }

    /// Regression for #1681: the docked `pane.open` path created its block
    /// store-only (`wcore::create_block`), so the block was absent from the
    /// reducer's `state.blocks` and a later TearOffBlock / RedockFloatingPane
    /// was rejected "block not found". Assert the block now lands in `srv_state`
    /// and a tear-off of the freshly-opened pane succeeds end-to-end.
    #[tokio::test]
    async fn docked_pane_open_block_is_in_reducer_and_tears_off() {
        let state = test_state();

        // Workspace + tab through the reducer (→ srv_state AND, via apply, wstore).
        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();

        // Open a docked sysinfo pane (no required args).
        let cmd = CommandPaneOpenData {
            view: "sysinfo".into(),
            file: None,
            url: None,
            cwd: None,
            title: None,
            tab_id: Some(tab_id.clone()),
            split_direction: None,
            split_reference_block_id: None,
            focus: None,
            tree_expanded: None,
            floating: None,
            meta: None,
            skip_placement: None,
            reuse_editor_pane: None,
        };
        let res = open_pane(&state, cmd).await.expect("open_pane docked");

        // The block is now visible to the reducer (was the bug: store-only).
        {
            let s = state.srv_state.lock().await;
            assert!(
                s.blocks.contains_key(&res.block_id),
                "docked pane.open block must be tracked in srv_state"
            );
        }

        // And tearing it off no longer hits "block not found".
        let r = crate::sagas::tear_off_block::run(
            &state,
            res.block_id.clone(),
            tab_id.clone(),
            ws_id.clone(),
        )
        .await;
        assert!(r.is_ok(), "tear-off of an opened pane must succeed, got: {:?}", r.err());
    }

    /// In-pane tabs (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.2):
    /// `skip_placement` must create a reducer-tracked block (same as the
    /// docked path) but leave the tab's layout tree completely untouched —
    /// the caller is about to attach it to an existing leaf's block-stack,
    /// not give it its own tile.
    #[tokio::test]
    async fn skip_placement_creates_block_without_touching_the_layout_tree() {
        let state = test_state();

        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();

        // A fresh tab has no layout tree yet — confirm that baseline before
        // asserting skip_placement doesn't add one.
        {
            let s = state.srv_state.lock().await;
            assert!(s.tabs.get(&tab_id).unwrap().rootnode.is_none());
        }

        let cmd = CommandPaneOpenData {
            view: "agent".into(),
            file: None,
            url: None,
            cwd: None,
            title: None,
            tab_id: Some(tab_id.clone()),
            split_direction: None,
            split_reference_block_id: None,
            focus: None,
            tree_expanded: None,
            floating: None,
            meta: Some({
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m
            }),
            skip_placement: Some(true),
            reuse_editor_pane: None,
        };
        let res = open_pane(&state, cmd).await.expect("open_pane skip_placement");
        assert!(res.created);

        let s = state.srv_state.lock().await;
        assert!(
            s.blocks.contains_key(&res.block_id),
            "skip_placement block must still be tracked in srv_state"
        );
        assert!(
            s.tabs.get(&tab_id).unwrap().rootnode.is_none(),
            "skip_placement must not place the block into the tab's layout tree"
        );
    }

    fn editor_open_cmd(
        tab_id: Option<String>,
        file: &str,
        split_reference_block_id: Option<String>,
        reuse_editor_pane: Option<bool>,
    ) -> CommandPaneOpenData {
        CommandPaneOpenData {
            view: "editor".into(),
            file: Some(file.to_string()),
            url: None,
            cwd: None,
            title: None,
            tab_id,
            split_direction: None,
            split_reference_block_id,
            focus: None,
            tree_expanded: None,
            floating: None,
            meta: None,
            skip_placement: None,
            reuse_editor_pane,
        }
    }

    /// No existing Editor pane in the caller's tab → today's unchanged
    /// behavior: a new block is created (SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md
    /// Part 2's fall-through path).
    #[tokio::test]
    async fn open_editor_creates_new_pane_when_none_exists_in_tab() {
        let state = test_state();

        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();

        // A "caller" pane in the tab (stands in for the calling agent's own
        // block) — but no Editor pane exists yet, so reuse must not trigger.
        let caller = open_pane(&state, {
            let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
            cmd.view = "sysinfo".into();
            cmd.file = None;
            cmd
        })
        .await
        .expect("open_pane caller");

        // tab_id passed explicitly (matching this file's other tests) —
        // resolve_tab_id's separate "first workspace" fallback is exercised
        // by test_state()'s own seeded default workspace/tab and isn't part
        // of what this test is checking.
        let cmd = editor_open_cmd(Some(tab_id), "/tmp/a.md", Some(caller.block_id.clone()), Some(true));
        let res = open_pane(&state, cmd).await.expect("open_pane editor");
        assert!(res.created, "must create a new Editor pane when none exists in the tab");
        assert_ne!(res.block_id, caller.block_id);
    }

    /// An Editor pane already open in the caller's own tab → reused (new tab
    /// pushed into it via EVENT_EDITOR_OPEN_FILE_REQUEST) instead of spawning
    /// a second Editor pane.
    #[tokio::test]
    async fn open_editor_reuses_existing_pane_in_callers_tab() {
        let state = test_state();

        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();

        // Caller's own pane (e.g. the agent pane that will call OpenEditor).
        let caller = open_pane(&state, {
            let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
            cmd.view = "sysinfo".into();
            cmd.file = None;
            cmd
        })
        .await
        .expect("open_pane caller");

        // An Editor pane already open in the same tab.
        let first_editor = open_pane(
            &state,
            editor_open_cmd(Some(tab_id.clone()), "/tmp/first.md", None, None),
        )
        .await
        .expect("open_pane first editor");
        assert!(first_editor.created);

        // Simulate the frontend having already reported the materialized
        // tree back (LayoutQueueBackendActions only queues an action for the
        // frontend to apply — agentmux-srv/src/reducer/layout.rs's
        // handle_layout_queue_backend_actions never touches rootnode itself,
        // confirmed by reading it directly; the frontend round-trips a real
        // tree via LayoutSetTree). Realistic for a reuse target: by the time
        // a SECOND OpenEditor call reuses an existing pane, that pane's own
        // creation round-trip has long since completed in real usage — this
        // is not the same race maybe_reuse_editor_pane's meta-based file
        // delivery bridges (block just created THIS call), it's simulating
        // an already-settled pane from an EARLIER call.
        {
            let tab: Tab = state.wstore.must_get(&tab_id).unwrap();
            let mut layout: obj::LayoutState = state.wstore.must_get(&tab.layoutstate).unwrap();
            layout.rootnode = Some(obj::LayoutNode {
                id: "leaf-1".to_string(),
                data: Some(obj::LayoutNodeData {
                    block_id: first_editor.block_id.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            });
            state.wstore.update(&mut layout).unwrap();
        }

        // A second OpenEditor call from the caller, same tab, different file
        // — must reuse the existing Editor pane, not create a second one.
        let reused = open_pane(
            &state,
            editor_open_cmd(None, "/tmp/second.md", Some(caller.block_id.clone()), Some(true)),
        )
        .await
        .expect("open_pane reused editor");
        assert!(!reused.created, "must reuse the existing Editor pane, not create a second one");
        assert_eq!(reused.block_id, first_editor.block_id);
        assert_eq!(reused.tab_id, tab_id);

        // Regression for reagent P1 on PR #2404: focus (the OpenEditor
        // default, cmd.focus == None -> unwrap_or(true)) must still be
        // applied on the reuse path, not silently dropped. focused_node_id
        // is the layout LEAF id ("leaf-1", seeded above), NOT the block id
        // (codex P1: an earlier version of the fix passed the block id
        // directly, which is the wrong id type entirely).
        let s = state.srv_state.lock().await;
        assert_eq!(
            s.tabs.get(&tab_id).unwrap().focused_node_id,
            "leaf-1",
            "reusing an existing editor pane must still focus its tab's layout leaf, same as the create path"
        );
    }

    /// Regression for reagent P1 on PR #2404: `EditorViewModel.openToTheSide`/
    /// `openInTerminal` (`frontend/app/view/editor/editor-model.ts:958-984`)
    /// call the same generic `pane.open` RPC with `split_reference_block_id`
    /// set to their OWN block id, for split placement only — never setting
    /// `reuse_editor_pane`. Reuse must not trigger for them, or "Open to the
    /// Side" would silently redirect into the calling pane itself instead of
    /// creating the requested second pane.
    #[tokio::test]
    async fn open_editor_does_not_reuse_without_explicit_opt_in() {
        let state = test_state();

        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();

        // The "current" Editor pane, standing in for the one openToTheSide
        // is called from.
        let current_editor = open_pane(
            &state,
            editor_open_cmd(Some(tab_id.clone()), "/tmp/current.md", None, None),
        )
        .await
        .expect("open_pane current editor");

        // openToTheSide's exact shape: split_reference_block_id = its own
        // block id, no reuse_editor_pane set.
        let side = open_pane(
            &state,
            editor_open_cmd(
                Some(tab_id.clone()),
                "/tmp/side.md",
                Some(current_editor.block_id.clone()),
                None,
            ),
        )
        .await
        .expect("open_pane openToTheSide");
        assert!(
            side.created,
            "openToTheSide must always create its own new pane, never reuse the calling pane"
        );
        assert_ne!(side.block_id, current_editor.block_id);
    }

    /// Regression for codex P1 on PR #2404: a `floating: true` OpenEditor
    /// call must always get its own new floating window, even when an
    /// Editor pane already exists (with reuse opted in) in the caller's tab
    /// — reuse must not silently swallow it into the existing docked pane.
    #[tokio::test]
    async fn open_editor_floating_request_bypasses_reuse() {
        let state = test_state();

        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();

        let caller = open_pane(&state, {
            let mut cmd = editor_open_cmd(Some(tab_id.clone()), "/tmp/unused.txt", None, None);
            cmd.view = "sysinfo".into();
            cmd.file = None;
            cmd
        })
        .await
        .expect("open_pane caller");

        let existing_editor = open_pane(
            &state,
            editor_open_cmd(Some(tab_id.clone()), "/tmp/existing.md", None, None),
        )
        .await
        .expect("open_pane existing editor");

        let mut floating_cmd = editor_open_cmd(
            None,
            "/tmp/floating.md",
            Some(caller.block_id.clone()),
            Some(true),
        );
        floating_cmd.floating = Some(true);
        let floating = open_pane(&state, floating_cmd)
            .await
            .expect("open_pane floating editor");
        assert_ne!(
            floating.block_id, existing_editor.block_id,
            "a floating OpenEditor request must never be swallowed into an existing docked pane"
        );
    }
}

#[cfg(test)]
mod agent_zoom_seed_tests {
    use super::parse_seed_zoom;

    #[test]
    fn seeds_valid_non_default_zoom() {
        assert_eq!(parse_seed_zoom("1.3"), Some(1.3));
        assert_eq!(parse_seed_zoom("0.5"), Some(0.5));
        assert_eq!(parse_seed_zoom("2"), Some(2.0));
        assert_eq!(parse_seed_zoom("  1.4  "), Some(1.4)); // trims
    }

    #[test]
    fn rejects_default_out_of_range_and_garbage() {
        assert_eq!(parse_seed_zoom("1.0"), None, "default seeds nothing");
        assert_eq!(parse_seed_zoom("1"), None, "default seeds nothing");
        assert_eq!(parse_seed_zoom("2.5"), None, "above range");
        assert_eq!(parse_seed_zoom("0.4"), None, "below range");
        assert_eq!(parse_seed_zoom("abc"), None, "unparseable");
        assert_eq!(parse_seed_zoom(""), None, "empty");
    }
}

#[cfg(test)]
mod bundle_upsert_input_tests {
    use super::bundle::normalize_bundle_upsert_input;
    use serde_json::json;

    #[test]
    fn fills_missing_or_null_id_with_empty_string() {
        let no_id = normalize_bundle_upsert_input(json!({ "name": "p" }));
        assert_eq!(no_id["id"], json!(""));

        let null_id = normalize_bundle_upsert_input(json!({ "id": null, "name": "p" }));
        assert_eq!(null_id["id"], json!(""));

        // A real id is preserved untouched.
        let kept = normalize_bundle_upsert_input(json!({ "id": "abc", "name": "p" }));
        assert_eq!(kept["id"], json!("abc"));
    }

    #[test]
    fn encodes_array_fields_to_json_strings() {
        let out = normalize_bundle_upsert_input(json!({
            "name": "p",
            "context_files": [{ "path": "a.md", "content": "x" }],
            "mcp_servers": [],
            "skills": ["s1", "s2"],
        }));
        // serde_json::Value maps serialize keys in sorted order.
        assert_eq!(out["context_files"], json!("[{\"content\":\"x\",\"path\":\"a.md\"}]"));
        assert_eq!(out["mcp_servers"], json!("[]"));
        assert_eq!(out["skills"], json!("[\"s1\",\"s2\"]"));
    }

    #[test]
    fn leaves_string_fields_untouched() {
        let out = normalize_bundle_upsert_input(json!({
            "name": "p",
            "context_files": "[]",
            "skills": "[\"already\"]",
        }));
        assert_eq!(out["context_files"], json!("[]"));
        assert_eq!(out["skills"], json!("[\"already\"]"));
    }
}
