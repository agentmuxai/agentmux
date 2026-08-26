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
// pub(crate): `server/mod.rs`'s `/agentmux/agent/stop` handler (the
// cross-channel bulk-stop forward target,
// SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22.md) calls
// `stop_one_agent_block` directly — same function `fleet_bulk_stop_impl`
// already uses for a LOCAL target, reused rather than duplicated for a
// forwarded one.
pub(crate) mod agent_io;
mod agent_define;
/// Re-exported so the human-facing creation/edit RPC handlers
/// (`server::agent_handlers::template`/`core`) can reuse the same
/// vendor-base-url validation `agent.define` already uses, instead of
/// duplicating it.
pub(crate) use agent_define::validate_vendor_base_url;
mod pane;
mod blockfile;
pub(crate) mod session;
mod identity;
mod bundle;
mod memory;
mod skill;
mod mcp;
mod bookmarks;
mod voice;
pub(crate) mod fleet;

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
    bookmarks::register(engine, state);
    voice::register(engine, state);
    fleet::register(engine, state);
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
    // requests into a reused docked pane). Also excludes an explicit
    // `tree_expanded` request (`OpenEditor`'s `collapse_tree` option) —
    // reagent P2 on PR #2404: a reused pane keeps ITS OWN existing tree
    // state, with no live mechanism to apply a new one (same class of
    // construction-time-only limitation as focus, see
    // `maybe_reuse_editor_pane`'s doc comment) — bypassing reuse for this
    // specific request and falling through to the create path (which
    // already honors `tree_expanded` correctly) is far simpler than
    // building live meta-application, and was the reviewer's own suggested
    // alternative.
    if cmd.view == "editor"
        && cmd.reuse_editor_pane == Some(true)
        && cmd.floating != Some(true)
        && cmd.tree_expanded.is_none()
    {
        if let (Some(caller_block_id), Some(file)) =
            (cmd.split_reference_block_id.as_deref(), cmd.file.as_deref())
        {
            if let Some(result) = pane::maybe_reuse_editor_pane(state, caller_block_id, file).await? {
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
        // One batched frame so the renderer applies all of them in a single
        // reactive flush — see EventBus::broadcast_wave_obj_updates.
        event_bus.broadcast_wave_obj_updates(&updates);
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
    id_store: Arc<Store>,
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
    // Every agent gets its own dedicated ABF bundle
    // (ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.2). Done here,
    // after the atomic find-or-insert has confirmed this is a genuinely
    // NEW definition — not before, or every idempotent `if_exists=skip`/
    // `update` call against an existing name would leak an unbound bundle
    // (see `agent_def_provision_and_bind_bundle`'s own doc comment).
    wstore.agent_def_provision_and_bind_bundle(&id_store, &mut def, now);
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
    let links = state.identity_store.agent_identity_list_for_agent(&def_id)
        .map_err(|e| format!("identity.self.accounts: {e}"))?;
    let mut accounts = Vec::new();
    for link in &links {
        // A malformed `secret_ref` on ONE linked account must not hide this
        // agent's other, perfectly readable accounts — the same "one bad row
        // hides everything" bug class `identity_list` was fixed for
        // (ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04.md),
        // just reachable through this separate per-agent lookup too (reagent
        // P1 on PR #2419 review). Skip and log rather than `?`-propagate.
        // resolve_account, not plain id_store.identity_get — reagentx P1
        // review on PR #2632: without the identity_store fallback, a
        // migrated/continuing account (resolvable at spawn time) showed as
        // "missing" here, inconsistent with the agent actually being able
        // to spawn with it.
        match crate::identity::resolver::resolve_account(&state.id_store, &state.identity_store, &link.account_id) {
            Ok(Some((acct, _account_store))) => {
                let masked_tail = acct.context.get("masked_tail")
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                accounts.push(json!({
                    "account_id": acct.id, "provider": acct.provider, "name": acct.name,
                    "kind": acct.kind, "status": acct.status, "masked_tail": masked_tail,
                    "updated_at": acct.updated_at,
                }));
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
    let links = state.identity_store.agent_identity_list_for_agent(&def_id)
        .map_err(|e| format!("identity.account.validate: {e}"))?;
    if !links.iter().any(|l| l.account_id == account_id) {
        return Err("FORBIDDEN: account not linked to this agent".to_string());
    }
    // resolve_account, not plain id_store.identity_get — same reagentx P1
    // review as identity_self_accounts_impl above: a migrated/continuing
    // account must validate successfully here too, consistently with the
    // spawn path.
    let (acct, _account_store) = crate::identity::resolver::resolve_account(&state.id_store, &state.identity_store, account_id)
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
    // Emit both keys: `bundle.list` callers read `bundles`; the separate
    // REST route `/api/v1/agent/preset/list` (`PresetList` MCP tool,
    // server/mod.rs) still reads `presets` and is unrelated to the internal
    // WS `preset.*` aliases retired in this pass — do not drop `presets`
    // here without first retiring that REST route too.
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

/// Structurally validate a bundle draft — Armory Bundle Format (ABF)
/// UI-alignment pass. Takes the SAME payload shape `bundle.upsert` accepts
/// (reuses its `normalize_bundle_upsert_input`), not just an id, so the
/// Armory editor's "Validate" button can check an unsaved draft (including a
/// brand-new bundle with no id yet) rather than only whatever was last
/// persisted. Read-only: never touches the Store.
pub(crate) fn bundle_validate_impl(data: serde_json::Value) -> Result<serde_json::Value, String> {
    let memory: Memory = serde_json::from_value(bundle::normalize_bundle_upsert_input(data))
        .map_err(|e| format!("bundle.validate: {e}"))?;
    let report = crate::backend::bundle_validate::validate_bundle(&memory);
    serde_json::to_value(&report).map_err(|e| e.to_string())
}

pub(crate) async fn bundle_self_get_impl(
    state: &AppState,
    agent_id: &str,
) -> Result<serde_json::Value, String> {
    let instance = state.wstore.instance_get_by_slug(agent_id)
        .map_err(|e| format!("bundle.self.get: {e}"))?;
    // `instance_get_by_slug` only ever hits the local `db_agents` table — a
    // live agent that only exists in the global named-agent registry (never
    // created a `db_agents` instance row) falls through with `instance:
    // None` here. Without this fallback that silently read as "no bundle
    // bound" and returned the blank/vanilla preset regardless of what's
    // actually bound, with nothing to distinguish it from a genuinely
    // unbound agent. Mirrors the two-tier lookup
    // `native_memory_handlers::memory_dir_for_agent` already does for the
    // same reason (see that function's own doc comment, issue #1836).
    let memory_id = instance.as_ref()
        .and_then(|i| if i.memory_id.is_empty() { None } else { Some(i.memory_id.clone()) })
        .or_else(|| {
            crate::server::native_memory_handlers::find_active_registry_record_by_slug(agent_id)
                .and_then(|rec| rec.data.memory_id)
        });
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

/// Caller-supplied provenance for a `memory.write` call — mirrors
/// `NativeMemoryWriteProvenance` (the WebSocket RPC's own wire shape) but
/// kept as plain `&str`s here rather than importing that type, since this
/// impl fn is also called directly from tests without going through the
/// RPC layer at all. See
/// docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.1.
pub(crate) struct MemoryWriteProvenance<'a> {
    pub source: &'a str,
    pub detail: &'a str,
}

pub(crate) fn memory_write_impl(
    state: &AppState,
    agent_id: &str,
    filename: &str,
    content: &str,
    provenance: Option<MemoryWriteProvenance<'_>>,
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

    // Version history recorded BEFORE the live-file write below — reagent
    // P1: closes the same fs-watch race described in
    // native_memory_handlers.rs's write_file handler. This is the path the
    // MemoryWrite MCP tool actually calls in production — instrumenting it
    // matters at least as much as the WebSocket RPC path, since it's what
    // an agent's own MemoryWrite tool call hits. Non-fatal on failure — a
    // durability/review layer on top of the write, not the write itself.
    //
    // Keyed by the RESOLVED canonical id, not the raw `agent_id` (slug)
    // parameter — reagent P1: this surface previously stored versions
    // slug-keyed while the WS RPC surface stores them keyed by the
    // resolved `AgentDefinition.id`, so a version written here was
    // invisible to a WS-RPC-based history/diff/revert call for the same
    // logical agent whenever slug != id. See `resolve_agent_uuid`'s own
    // doc for the full resolution-order rationale.
    //
    // Hard-fails on resolution failure — reagent P2 (re-review): this used
    // to silently fall back to the raw slug via `unwrap_or_else`, while
    // memory_history_impl/memory_diff_impl/memory_revert_impl all hard-fail
    // on the identical call. `memory_dir_for_agent` above already succeeded
    // (proving `agent_id` resolves via at least one lookup path), so a
    // failure here is very likely transient (e.g. a registry-file I/O
    // hiccup) rather than "unknown agent" — but silently keying this
    // version by the raw slug on that failure would reintroduce the exact
    // disjoint-keyspace bug (a write invisible to history/diff/revert) this
    // PR exists to fix. A visible, retriable write failure is strictly
    // safer than a silent data-integrity split.
    let version_agent_id = crate::server::native_memory_handlers::resolve_agent_uuid(&state.wstore, agent_id)
        .map_err(|e| format!("memory.write: {e}"))?;
    let (source, detail) = match &provenance {
        Some(p) => (p.source, p.detail),
        None => ("agent_inferred", "{}"),
    };
    if let Err(e) = state.id_store.agent_native_memory_version_insert(&version_agent_id, filename, content, source, detail, "") {
        tracing::warn!(agent_id, filename, error = %e, "memory.write: version insert failed (non-fatal)");
    }

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

    // reagent P1 on PR #2674 (found on memory_revert_impl, same gap exists
    // here): this App-API path backing the `MemoryWrite` MCP tool never
    // updated the `db_agent_native_memory` mirror row, unlike its WS-RPC
    // sibling `agent:memory:write_file` in native_memory_handlers.rs. Per
    // `read_file`'s own fallback logic, a channel with no live copy of the
    // file falls back to the mirror and treats a stale row as permanent —
    // so a write issued through the MCP tool never propagated cross-channel.
    let metadata_type = crate::server::native_memory_handlers::parse_memory_frontmatter_type(content);
    let dest_meta = std::fs::metadata(&dest).ok();
    let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(content.len() as i64);
    let mtime_ms = dest_meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Err(e) = state.id_store.agent_native_memory_upsert(
        &version_agent_id,
        filename,
        content,
        metadata_type.as_deref(),
        &dest.to_string_lossy(),
        size_bytes,
        mtime_ms,
    ) {
        tracing::warn!(agent_id, filename, error = %e, "memory.write: mirror upsert failed (non-fatal)");
    }

    state.broker.publish(crate::backend::wps::WaveEvent {
        event: format!("agent:memory:changed:{agent_id}"),
        scopes: vec![], sender: String::new(), persist: 0, data: None,
    });
    Ok(())
}

pub(crate) fn memory_history_impl(
    state: &AppState,
    agent_id: &str,
    filename: &str,
) -> Result<serde_json::Value, String> {
    crate::server::native_memory_handlers::validate_memory_filename(filename)
        .map_err(|e| format!("memory.history: {e}"))?;
    // See memory_write_impl's own comment — must key by the same resolved
    // canonical id that write used, not the raw slug.
    let version_agent_id = crate::server::native_memory_handlers::resolve_agent_uuid(&state.wstore, agent_id)
        .map_err(|e| format!("memory.history: {e}"))?;
    let versions: Vec<crate::backend::rpc_types::NativeMemoryVersionMeta> = state
        .id_store
        .agent_native_memory_version_list(&version_agent_id, filename)
        .map_err(|e| format!("memory.history: store: {e}"))?
        .into_iter()
        .map(|v| crate::backend::rpc_types::NativeMemoryVersionMeta {
            id: v.id,
            content_hash: v.content_hash,
            parent_version_id: v.parent_version_id,
            source: v.source,
            source_detail: v.source_detail,
            session_id: v.session_id,
            created_at: v.created_at,
        })
        .collect();
    serde_json::to_value(crate::backend::rpc_types::NativeMemoryHistoryResult { versions })
        .map_err(|e| e.to_string())
}

pub(crate) fn memory_diff_impl(
    state: &AppState,
    agent_id: &str,
    from_version_id: &str,
    to_version_id: &str,
) -> Result<serde_json::Value, String> {
    // See memory_write_impl's own comment — must compare against the same
    // resolved canonical id that write used, not the raw slug.
    let version_agent_id = crate::server::native_memory_handlers::resolve_agent_uuid(&state.wstore, agent_id)
        .map_err(|e| format!("memory.diff: {e}"))?;
    let from = state
        .id_store
        .agent_native_memory_version_get(from_version_id)
        .map_err(|e| format!("memory.diff: store: {e}"))?
        .ok_or_else(|| format!("memory.diff: version {from_version_id} not found"))?;
    let to = state
        .id_store
        .agent_native_memory_version_get(to_version_id)
        .map_err(|e| format!("memory.diff: store: {e}"))?
        .ok_or_else(|| format!("memory.diff: version {to_version_id} not found"))?;
    // reagent P1 — see the identical check in native_memory_handlers.rs's
    // WS RPC handler for the full rationale.
    if from.agent_id != version_agent_id || to.agent_id != version_agent_id {
        return Err(format!("memory.diff: one or both versions do not belong to {agent_id}"));
    }
    // reagent P2 — see the identical check in native_memory_handlers.rs's
    // WS RPC handler for the full rationale.
    if from.filename != to.filename {
        return Err(format!(
            "memory.diff: from_version_id and to_version_id are versions of different files ({} vs {})",
            from.filename, to.filename
        ));
    }
    let diff = crate::server::native_memory_handlers::line_diff(&from.content, &to.content);
    serde_json::to_value(crate::backend::rpc_types::NativeMemoryDiffResult { diff }).map_err(|e| e.to_string())
}

pub(crate) fn memory_revert_impl(
    state: &AppState,
    agent_id: &str,
    filename: &str,
    target_version_id: &str,
) -> Result<serde_json::Value, String> {
    crate::server::native_memory_handlers::validate_memory_filename(filename)
        .map_err(|e| format!("memory.revert: {e}"))?;

    // See memory_write_impl's own comment — must key/compare against the
    // same resolved canonical id that write used, not the raw slug.
    let version_agent_id = crate::server::native_memory_handlers::resolve_agent_uuid(&state.wstore, agent_id)
        .map_err(|e| format!("memory.revert: {e}"))?;

    let target = state
        .id_store
        .agent_native_memory_version_get(target_version_id)
        .map_err(|e| format!("memory.revert: store: {e}"))?
        .ok_or_else(|| format!("memory.revert: version {target_version_id} not found"))?;
    if target.agent_id != version_agent_id || target.filename != filename {
        return Err(format!(
            "memory.revert: version {target_version_id} does not belong to {agent_id}/{filename}"
        ));
    }

    let dir = crate::server::native_memory_handlers::memory_dir_for_agent(
        &state.wstore, agent_id,
    ).map_err(|e| format!("memory.revert: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("memory.revert: mkdir: {e}"))?;

    // Version recorded BEFORE the live-file write below — same fs-watch-race
    // rationale as memory_write_impl above (reagent P1). Fatal on failure,
    // same reasoning as the WS RPC revert handler: silently reverting the
    // file but failing to record what it was reverted to would leave the
    // caller with no way to know.
    let detail = json!({ "reverted_to": target_version_id }).to_string();
    let new_version = state
        .id_store
        .agent_native_memory_version_insert(&version_agent_id, filename, &target.content, "revert", &detail, "")
        .map_err(|e| format!("memory.revert: version insert: {e}"))?;

    let dest = dir.join(filename);
    let tmp = dir.join(format!(".{}.{}.tmp", filename, uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&tmp, &target.content) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("memory.revert: write tmp: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("memory.revert: rename: {e}"));
    }

    // reagent P1 on PR #2674: this App-API path backing the `MemoryRevert`
    // MCP tool reverted the live file but never updated the
    // `db_agent_native_memory` mirror row, unlike its WS-RPC sibling
    // `agent:memory:revert` in native_memory_handlers.rs. Per `read_file`'s
    // own fallback logic, a channel with no live copy of the file falls
    // back to the mirror and treats a stale mirror row as permanent, not
    // briefly stale — so a revert issued through the actual `MemoryRevert`
    // tool silently failed to propagate cross-channel, leaving other
    // channels still showing the pre-revert (fabricated) content
    // indefinitely.
    let metadata_type = crate::server::native_memory_handlers::parse_memory_frontmatter_type(&target.content);
    let dest_meta = std::fs::metadata(&dest).ok();
    let size_bytes = dest_meta.as_ref().map(|m| m.len() as i64).unwrap_or(target.content.len() as i64);
    let mtime_ms = dest_meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Err(e) = state.id_store.agent_native_memory_upsert(
        &version_agent_id,
        filename,
        &target.content,
        metadata_type.as_deref(),
        &dest.to_string_lossy(),
        size_bytes,
        mtime_ms,
    ) {
        tracing::warn!(agent_id, filename, error = %e, "memory.revert: mirror upsert failed (non-fatal)");
    }

    state.broker.publish(crate::backend::wps::WaveEvent {
        event: format!("agent:memory:changed:{agent_id}"),
        scopes: vec![], sender: String::new(), persist: 0, data: None,
    });

    serde_json::to_value(crate::backend::rpc_types::NativeMemoryRevertResult {
        version: crate::backend::rpc_types::NativeMemoryVersionMeta {
            id: new_version.id,
            content_hash: new_version.content_hash,
            parent_version_id: new_version.parent_version_id,
            source: new_version.source,
            source_detail: new_version.source_detail,
            session_id: new_version.session_id,
            created_at: new_version.created_at,
        },
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod memory_version_impl_tests {
    use super::*;

    fn agent_def(id: &str, working_directory: &str) -> crate::backend::storage::AgentDefinition {
        crate::backend::storage::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: id.to_string(),
            name: "Test Agent".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: working_directory.to_string(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
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
            memory_id: String::new(),
        }
    }

    /// `test_state()` sets `id_store: wstore.clone()`, so both are the same
    /// in-memory `Store` (`run_object_schema`) — good enough for these
    /// wiring-level tests, since the version-chain logic itself is already
    /// covered by `native_memory_handlers.rs`'s tests against the same
    /// `Store` methods.
    ///
    /// Sets `CLAUDE_CONFIG_DIR` to the given temp dir explicitly — an empty
    /// value would make `memory_dir_for_agent` fall back to the REAL
    /// `~/.agentmux/shared/providers/claude/`, writing test fixtures into
    /// the developer's actual home directory (the same trap
    /// `native_memory_handlers.rs`'s own tests document having hit before).
    fn state_with_agent(agent_id: &str, working_directory: &std::path::Path) -> AppState {
        let state = crate::server::tests::test_state();
        let mut def = agent_def(agent_id, &working_directory.to_string_lossy());
        state.wstore.agent_def_insert(&mut def).unwrap();
        state
            .wstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: agent_id.to_string(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", working_directory.display()),
                updated_at: 0,
            })
            .unwrap();
        state
    }

    #[tokio::test]
    async fn write_then_history_records_a_version_with_default_provenance() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_agent("agent-app-1", tmp.path());

        memory_write_impl(&state, "agent-app-1", "MEMORY.md", "hello", None).unwrap();

        let history = memory_history_impl(&state, "agent-app-1", "MEMORY.md").unwrap();
        let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].get("source").and_then(|v| v.as_str()), Some("agent_inferred"));
    }

    #[tokio::test]
    async fn write_honors_explicit_provenance() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_agent("agent-app-2", tmp.path());

        memory_write_impl(
            &state, "agent-app-2", "MEMORY.md", "content",
            Some(MemoryWriteProvenance { source: "jekt", detail: r#"{"TIER":"sensitive"}"# }),
        ).unwrap();

        let history = memory_history_impl(&state, "agent-app-2", "MEMORY.md").unwrap();
        let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
        assert_eq!(versions[0].get("source").and_then(|v| v.as_str()), Some("jekt"));
    }

    /// Regression for reagent P1 on PR #2674: memory_write_impl (the
    /// App-API path backing the `MemoryWrite` MCP tool) must keep
    /// `db_agent_native_memory` in sync, same as its WS-RPC sibling
    /// `agent:memory:write_file` already does — otherwise `read_file` in a
    /// channel with no live copy of the file falls back to a permanently
    /// stale mirror row.
    #[tokio::test]
    async fn write_updates_the_native_memory_mirror_row() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_agent("agent-app-mirror-1", tmp.path());

        memory_write_impl(&state, "agent-app-mirror-1", "MEMORY.md", "hello mirror", None).unwrap();

        let mirrored = state.id_store.agent_native_memory_read("agent-app-mirror-1", "MEMORY.md").unwrap();
        assert_eq!(mirrored, Some("hello mirror".to_string()));
    }

    #[tokio::test]
    async fn diff_reflects_two_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_agent("agent-app-3", tmp.path());

        memory_write_impl(&state, "agent-app-3", "MEMORY.md", "v1", None).unwrap();
        memory_write_impl(&state, "agent-app-3", "MEMORY.md", "v2", None).unwrap();

        let history = memory_history_impl(&state, "agent-app-3", "MEMORY.md").unwrap();
        let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
        let newest = versions[0].get("id").and_then(|v| v.as_str()).unwrap();
        let oldest = versions[1].get("id").and_then(|v| v.as_str()).unwrap();

        let diff = memory_diff_impl(&state, "agent-app-3", oldest, newest).unwrap();
        let diff_text = diff.get("diff").and_then(|v| v.as_str()).unwrap();
        assert!(diff_text.contains("- v1"), "unexpected diff: {diff_text}");
        assert!(diff_text.contains("+ v2"), "unexpected diff: {diff_text}");
    }

    #[tokio::test]
    async fn diff_rejects_a_version_from_a_different_agent() {
        let tmp_a = tempfile::tempdir().unwrap();
        let tmp_b = tempfile::tempdir().unwrap();
        let state = crate::server::tests::test_state();
        let mut def_a = agent_def("agent-diff-a", &tmp_a.path().to_string_lossy());
        let mut def_b = agent_def("agent-diff-b", &tmp_b.path().to_string_lossy());
        state.wstore.agent_def_insert(&mut def_a).unwrap();
        state.wstore.agent_def_insert(&mut def_b).unwrap();
        for (id, dir) in [("agent-diff-a", tmp_a.path()), ("agent-diff-b", tmp_b.path())] {
            state
                .wstore
                .agent_content_set(&crate::backend::storage::AgentContent {
                    agent_id: id.to_string(),
                    content_type: "env".to_string(),
                    content: format!("CLAUDE_CONFIG_DIR={}\n", dir.display()),
                    updated_at: 0,
                })
                .unwrap();
        }

        memory_write_impl(&state, "agent-diff-a", "MEMORY.md", "v1", None).unwrap();
        memory_write_impl(&state, "agent-diff-a", "MEMORY.md", "v2", None).unwrap();
        let history = memory_history_impl(&state, "agent-diff-a", "MEMORY.md").unwrap();
        let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
        let newest = versions[0].get("id").and_then(|v| v.as_str()).unwrap().to_string();
        let oldest = versions[1].get("id").and_then(|v| v.as_str()).unwrap().to_string();

        let err = memory_diff_impl(&state, "agent-diff-b", &oldest, &newest).unwrap_err();
        assert!(err.contains("do not belong to"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn revert_restores_live_content_and_appends_a_new_version() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_agent("agent-app-4", tmp.path());

        memory_write_impl(&state, "agent-app-4", "MEMORY.md", "good", None).unwrap();
        memory_write_impl(&state, "agent-app-4", "MEMORY.md", "fabricated", None).unwrap();

        let history = memory_history_impl(&state, "agent-app-4", "MEMORY.md").unwrap();
        let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
        let good_id = versions[1].get("id").and_then(|v| v.as_str()).unwrap().to_string();

        let revert = memory_revert_impl(&state, "agent-app-4", "MEMORY.md", &good_id).unwrap();
        assert_eq!(
            revert.get("version").and_then(|v| v.get("source")).and_then(|v| v.as_str()),
            Some("revert")
        );

        let read = memory_read_impl(&state, "agent-app-4", "MEMORY.md").unwrap();
        assert_eq!(read.get("content").and_then(|v| v.as_str()), Some("good"));

        let history_after = memory_history_impl(&state, "agent-app-4", "MEMORY.md").unwrap();
        let versions_after = history_after.get("versions").and_then(|v| v.as_array()).unwrap();
        assert_eq!(versions_after.len(), 3, "revert must not delete or rewrite prior versions");
    }

    /// Regression for reagent P1 on PR #2674: memory_revert_impl (the
    /// App-API path backing the `MemoryRevert` MCP tool) reverted the live
    /// file but never updated `db_agent_native_memory`, unlike its WS-RPC
    /// sibling `agent:memory:revert` — a revert issued through the actual
    /// `MemoryRevert` tool silently failed to propagate cross-channel,
    /// leaving other channels' `read_file` fallback still showing the
    /// pre-revert (fabricated) content indefinitely.
    #[tokio::test]
    async fn revert_updates_the_native_memory_mirror_row() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_agent("agent-app-mirror-2", tmp.path());

        memory_write_impl(&state, "agent-app-mirror-2", "MEMORY.md", "good", None).unwrap();
        memory_write_impl(&state, "agent-app-mirror-2", "MEMORY.md", "fabricated", None).unwrap();
        let history = memory_history_impl(&state, "agent-app-mirror-2", "MEMORY.md").unwrap();
        let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
        let good_id = versions[1].get("id").and_then(|v| v.as_str()).unwrap().to_string();

        memory_revert_impl(&state, "agent-app-mirror-2", "MEMORY.md", &good_id).unwrap();

        let mirrored = state.id_store.agent_native_memory_read("agent-app-mirror-2", "MEMORY.md").unwrap();
        assert_eq!(mirrored, Some("good".to_string()), "mirror must reflect the reverted content, not the fabricated one");
    }

    #[tokio::test]
    async fn revert_rejects_a_version_from_a_different_agent() {
        let tmp_a = tempfile::tempdir().unwrap();
        let tmp_b = tempfile::tempdir().unwrap();
        // Same underlying in-memory Store backs both AppState clones here
        // (test_state() re-opens a fresh Store each call) — build one
        // shared state and register two agents on it instead.
        let state = crate::server::tests::test_state();
        let mut def_a = agent_def("agent-app-5a", &tmp_a.path().to_string_lossy());
        let mut def_b = agent_def("agent-app-5b", &tmp_b.path().to_string_lossy());
        state.wstore.agent_def_insert(&mut def_a).unwrap();
        state.wstore.agent_def_insert(&mut def_b).unwrap();
        for (id, dir) in [("agent-app-5a", tmp_a.path()), ("agent-app-5b", tmp_b.path())] {
            state
                .wstore
                .agent_content_set(&crate::backend::storage::AgentContent {
                    agent_id: id.to_string(),
                    content_type: "env".to_string(),
                    content: format!("CLAUDE_CONFIG_DIR={}\n", dir.display()),
                    updated_at: 0,
                })
                .unwrap();
        }

        memory_write_impl(&state, "agent-app-5a", "MEMORY.md", "agent a's content", None).unwrap();
        let history = memory_history_impl(&state, "agent-app-5a", "MEMORY.md").unwrap();
        let version_id = history.get("versions").and_then(|v| v.as_array()).unwrap()[0]
            .get("id").and_then(|v| v.as_str()).unwrap().to_string();

        let err = memory_revert_impl(&state, "agent-app-5b", "MEMORY.md", &version_id).unwrap_err();
        assert!(err.contains("does not belong to"), "unexpected error: {err}");
    }

    /// Regression for reagent P1 (re-review of PR #2674): this App-API
    /// surface receives the agent SLUG (per `memory_dir_for_agent`'s own
    /// doc), but must key `db_agent_native_memory_versions` by the same
    /// canonical `AgentDefinition.id` the WS RPC surface uses — otherwise
    /// a version written here is invisible to a WS-RPC-based history/diff/
    /// revert call for the same logical agent whenever slug != id. Uses a
    /// deliberately DIFFERENT id and slug (existing test fixtures elsewhere
    /// in this file always set them equal, so they can't catch this class
    /// of bug at all).
    #[tokio::test]
    async fn write_impl_keys_versions_by_the_resolved_id_not_the_raw_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let state = crate::server::tests::test_state();
        let mut def = agent_def("agent-real-uuid-999", &tmp.path().to_string_lossy());
        def.slug = "agent-friendly-slug".to_string();
        state.wstore.agent_def_insert(&mut def).unwrap();
        state
            .wstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: def.id.clone(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", tmp.path().display()),
                updated_at: 0,
            })
            .unwrap();

        // Write via the slug (what the MemoryWrite MCP tool actually sends).
        memory_write_impl(&state, "agent-friendly-slug", "MEMORY.md", "content", None).unwrap();

        // The version must be discoverable under the RESOLVED id — the
        // same id a WS-RPC-based history/diff/revert call would use (it
        // receives AgentDefinition.id directly, per the frontend's own
        // contract) — not under the raw slug string.
        let by_resolved_id = state
            .id_store
            .agent_native_memory_version_list("agent-real-uuid-999", "MEMORY.md")
            .unwrap();
        assert_eq!(by_resolved_id.len(), 1, "version must be keyed by the resolved AgentDefinition.id");

        let by_raw_slug = state
            .id_store
            .agent_native_memory_version_list("agent-friendly-slug", "MEMORY.md")
            .unwrap();
        assert_eq!(by_raw_slug.len(), 0, "version must NOT be keyed by the raw, unresolved slug");

        // And memory_history_impl (called with the slug, same as write)
        // must still find it, proving read-your-own-write consistency
        // through this surface's own resolution.
        let history = memory_history_impl(&state, "agent-friendly-slug", "MEMORY.md").unwrap();
        let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
        assert_eq!(versions.len(), 1);
    }
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
///
/// Reuses `output.idx` when its `covered_size` header already matches the
/// current `output` size — same freshness check
/// `register_blockfile_read_range` (`blockfile.rs`) already does — instead of
/// unconditionally rescanning the entire history on every call. Without this,
/// every pane open for an agent with any global-zone history pays a full
/// O(history size) rescan even when nothing has changed since the index was
/// last built.
pub(super) fn global_zone_line_count(
    gfs: &Arc<crate::backend::storage::filestore::FileStore>,
    zone: &str,
) -> Option<u64> {
    use crate::backend::blockcontroller::shell::OUTPUT_IDX_HEADER_LEN;

    let stat = gfs
        .stat(zone, crate::backend::agent_session::OUTPUT_FILE)
        .ok()??;
    if stat.size == 0 {
        return Some(0);
    }
    let output_size = stat.size as u64;

    if let Ok(Some(idx_stat)) = gfs.stat(zone, "output.idx") {
        if idx_stat.size >= OUTPUT_IDX_HEADER_LEN {
            if let Ok((_, header)) = gfs.read_at(zone, "output.idx", 0, OUTPUT_IDX_HEADER_LEN) {
                if let Ok(bytes) = <[u8; 8]>::try_from(header.as_slice()) {
                    if u64::from_le_bytes(bytes) == output_size {
                        return Some(((idx_stat.size - OUTPUT_IDX_HEADER_LEN) / 8) as u64);
                    }
                }
            }
        }
    }

    crate::backend::blockcontroller::shell::rebuild_output_idx(gfs, zone, output_size)
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
    if let Ok(Some(instance)) = state.wstore.instance_get_by_slug(agent_id) {
        if !instance.definition_id.is_empty() {
            return Ok(instance.definition_id);
        }
    }
    if let Ok(Some(_)) = state.wstore.agent_def_get(agent_id) {
        return Ok(agent_id.to_string());
    }
    // reagentx P1 on PR #2428 (round 4): `instance_get_by_slug` only ever
    // hits the local `db_agents` table — the common case (per
    // `bundle_self_get_impl`'s own doc comment a few lines above: launching
    // an agent does not create a `db_agents` row) is a live agent that only
    // exists in the global named-agent registry. Without this fallback,
    // `identity.self.accounts`/`IdentityAccounts` — the exact tool
    // confirmed live-broken for a real registry-only agent — stayed broken
    // even after `instance_get_by_slug` existed, since a slug-only agent
    // never resolves via either branch above. Mirrors the same registry
    // fallback `bundle_self_get_impl` already has.
    if let Some(rec) = crate::server::native_memory_handlers::find_active_registry_record_by_slug(agent_id) {
        if !rec.data.definition_id.is_empty() {
            return Ok(rec.data.definition_id);
        }
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

    #[test]
    fn global_zone_line_count_reuses_fresh_index_instead_of_rescanning() {
        // Same fresh-index reuse the read_range path already does
        // (blockfile.rs) — a header whose covered_size matches the current
        // `output` size must be trusted as-is, not rebuilt from a full scan.
        // Proven here by seeding a deliberately-wrong-but-size-matching index
        // (2 entries) alongside real `output` content that would scan to 3
        // non-blank lines: a fix that trusts the fresh header returns the
        // cached 2; the old always-rebuild code returns the rescanned 3.
        let global = mem_store();
        let zone = "agent:def-cc-5:current";
        let body: &[u8] = b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n";
        seed_output(&global, zone, body);

        let mut idx = Vec::new();
        idx.extend_from_slice(&(body.len() as u64).to_le_bytes()); // covered_size == output size
        idx.extend_from_slice(&0u64.to_le_bytes()); // fabricated entry 0
        idx.extend_from_slice(&9u64.to_le_bytes()); // fabricated entry 1 (only 2, not 3)
        global
            .make_file(zone, "output.idx", FileMeta::default(), FileOpts::default())
            .unwrap();
        global.write_file(zone, "output.idx", &idx).unwrap();

        assert_eq!(
            global_zone_line_count(&global, zone),
            Some(2),
            "must trust the fresh cached index rather than rescanning `output`",
        );
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

    /// Regression for reagent P2 on PR #2404: an explicit `collapse_tree`
    /// request (`tree_expanded: Some(false)`) has no live mechanism to apply
    /// to an already-mounted pane's tree state — bypass reuse for it
    /// entirely rather than silently ignoring the request.
    #[tokio::test]
    async fn open_editor_bypasses_reuse_when_tree_expanded_requested() {
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

        let mut cmd = editor_open_cmd(None, "/tmp/collapsed.md", Some(caller.block_id.clone()), Some(true));
        cmd.tree_expanded = Some(false);
        let res = open_pane(&state, cmd).await.expect("open_pane collapse_tree request");
        assert_ne!(
            res.block_id, existing_editor.block_id,
            "an explicit tree_expanded request must bypass reuse and create its own pane"
        );
    }

    /// An Editor pane already open in the caller's own tab → reused (file
    /// appended to META_PENDING_OPEN_FILES for the pane to drain) instead of
    /// spawning a second Editor pane.
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
    }

    /// Regression for codex P1 on PR #2404: 2+ reuse calls before the target
    /// pane's frontend ever drains its pending-files meta must all survive,
    /// not overwrite each other down to just the last one.
    #[tokio::test]
    async fn open_editor_reuse_queues_multiple_pending_files() {
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

        let first_editor = open_pane(
            &state,
            editor_open_cmd(Some(tab_id.clone()), "/tmp/first.md", None, None),
        )
        .await
        .expect("open_pane first editor");

        // Three back-to-back reuse calls, none of which drain the queue
        // (no frontend attached in this test) — all three must still be
        // present afterward, not just the last one.
        for path in ["/tmp/a.md", "/tmp/b.md", "/tmp/c.md"] {
            let res = open_pane(
                &state,
                editor_open_cmd(None, path, Some(caller.block_id.clone()), Some(true)),
            )
            .await
            .expect("open_pane reuse");
            assert!(!res.created);
            assert_eq!(res.block_id, first_editor.block_id);
        }

        let block: Block = state.wstore.must_get(&first_editor.block_id).unwrap();
        let pending = block
            .meta
            .get("editor:pending_open_files")
            .and_then(|v| v.as_array())
            .expect("pending_open_files must be an array");
        let paths: Vec<&str> = pending.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            paths,
            vec!["/tmp/a.md", "/tmp/b.md", "/tmp/c.md"],
            "all three stacked reuse requests must survive, in order, not just the last one"
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

// SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md follow-up: `PresetGet`
// self-mode (backed by `bundle_self_get_impl`) only ever checked the local
// `db_agents` table via `instance_get_by_slug`. A live agent that only
// exists in the global named-agent registry (the common case — launching
// an agent does not create a `db_agents` row, see issue #1836, already
// handled the same way by `native_memory_handlers::memory_dir_for_agent`)
// fell through to `instance: None` and silently returned the generic
// blank/vanilla preset, with nothing to distinguish it from an agent that
// genuinely has no bundle bound. Confirmed live against a real registry-only
// agent named "AgentY": `PresetGet` returned `is_blank: true` even though
// the registry's own `memory_id` was set.
#[cfg(test)]
mod bundle_self_get_registry_fallback_tests {
    use super::*;
    use crate::server::tests::test_state;
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    #[tokio::test]
    async fn falls_back_to_the_registrys_own_bound_bundle_when_no_local_instance_row_exists() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let state = test_state();
        let bundle: crate::backend::storage::memory_bundles::Memory =
            serde_json::from_value(serde_json::json!({
                "id": "bundle-agenty-test",
                "name": "AgentY's real bundle",
            }))
            .unwrap();
        state.id_store.bundle_memory_upsert(&bundle).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("AGENTMUX_HOME_OVERRIDE");
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());

        let registry_dir = tmp.path().join("shared").join("agents").join("registry");
        let registry = crate::registry::Registry::open(registry_dir).unwrap();
        registry
            .upsert(&crate::registry::NamedAgentRecord {
                schema_version: 1,
                data: crate::registry::NamedAgentRecordV1 {
                    instance_id: "inst-agenty".to_string(),
                    instance_name: "AgentY".to_string(),
                    definition_id: "def-agenty".to_string(),
                    identity_id: None,
                    memory_id: Some("bundle-agenty-test".to_string()),
                    session_id: None,
                    working_dir: "agenty-0629j".to_string(),
                    source_agents_base: None,
                    created_at_ms: 1,
                    last_launched_at_ms: 1,
                    created_by_version: "test".to_string(),
                    last_launched_by_version: "test".to_string(),
                },
            })
            .unwrap();

        // No `db_agents` row for "agenty" exists in `state.wstore` — this
        // must resolve entirely through the registry fallback.
        let resp = bundle_self_get_impl(&state, "agenty").await;

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_HOME_OVERRIDE", v),
            None => std::env::remove_var("AGENTMUX_HOME_OVERRIDE"),
        }

        let resp = resp.expect("bundle.self.get must succeed via the registry fallback");
        assert_eq!(resp["id"], "bundle-agenty-test");
        assert_eq!(resp["is_blank"], false);
    }
}

#[cfg(test)]
mod identity_self_accounts_tests {
    use super::*;
    use crate::backend::storage::identities::SecretRef;
    use crate::server::tests::test_state;

    fn sample_account(id: &str, provider: &str) -> IdentityAccount {
        IdentityAccount {
            id: id.to_string(),
            name: format!("{provider}-oauth"),
            provider: provider.to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: format!("/tmp/{id}") },
            context: json!({}),
            status: "ok".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    /// Regression for reagent P1 on PR #2419 review: one malformed linked
    /// account must not hide this agent's other, perfectly readable
    /// accounts — the same bug class `identity_list` was fixed for, just
    /// reachable through this separate per-agent lookup too.
    #[tokio::test]
    async fn skips_a_malformed_linked_account_instead_of_failing_the_whole_call() {
        let state = test_state();

        let mut def = AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: uuid::Uuid::new_v4().to_string(),
            slug: String::new(),
            name: "test agent".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            environment: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(),
            auto_continue_enabled: 0,
            memory_id: String::new(),
        };
        state.wstore.agent_def_insert(&mut def).unwrap();

        state.wstore.identity_upsert(&sample_account("acct-good", "claude")).unwrap();
        {
            let conn = state.wstore.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO db_accounts
                    (id, name, provider, kind, display_name, secret_ref, context,
                     status, created_at, updated_at)
                 VALUES ('acct-bad', 'broken', 'github', 'oauth', '',
                         '{\"backend\":\"oauth_config_dir\",\"dir\":\"C:\\bad\\path\"}',
                         '{}', 'unknown', 0, 0)",
                [],
            )
            .unwrap();
        }
        state.wstore.agent_identity_link(&def.id, "acct-good", "claude").unwrap();
        state.wstore.agent_identity_link(&def.id, "acct-bad", "github").unwrap();

        let result = identity_self_accounts_impl(&state, &def.id)
            .await
            .expect("must not fail even with a malformed linked account present");
        let accounts = result["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 1, "the malformed account must be skipped, not error the whole call");
        assert_eq!(accounts[0]["account_id"], json!("acct-good"));
    }

    // reagentx P1 on PR #2428 (round 4): `resolve_agent_definition_id`
    // (the shared resolver behind `identity.self.*`) only tried
    // `instance_get_by_slug` (local `db_agents`) then `agent_def_get`
    // (treating the slug as if it were a definition UUID, which it never
    // is) — no registry fallback, unlike `bundle_self_get_impl`. So the
    // common case (a live agent that only exists in the global
    // named-agent registry, per that function's own doc comment) stayed
    // broken for `IdentityAccounts` even after the slug/name namespace
    // split — the exact tool this PR's own description cited as
    // confirmed-live-broken.
    #[tokio::test]
    async fn resolve_agent_definition_id_falls_back_to_the_registry_for_a_slug_only_agent() {
        use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let state = test_state();
        let mut def = AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: uuid::Uuid::new_v4().to_string(),
            slug: "agenty".to_string(),
            name: "AgentY".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            environment: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(),
            auto_continue_enabled: 0,
            memory_id: String::new(),
        };
        state.wstore.agent_def_insert(&mut def).unwrap();
        state.wstore.identity_upsert(&sample_account("acct-good", "claude")).unwrap();
        state.wstore.agent_identity_link(&def.id, "acct-good", "claude").unwrap();

        // Deliberately NO `instance_create` call — this agent exists only
        // in the global registry, exactly like a real live-launched agent
        // that never got a local `db_agents` instance row.
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("AGENTMUX_HOME_OVERRIDE");
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());

        let registry_dir = tmp.path().join("shared").join("agents").join("registry");
        let registry = crate::registry::Registry::open(registry_dir).unwrap();
        registry
            .upsert(&crate::registry::NamedAgentRecord {
                schema_version: 1,
                data: crate::registry::NamedAgentRecordV1 {
                    instance_id: "inst-agenty".to_string(),
                    instance_name: "AgentY".to_string(),
                    definition_id: def.id.clone(),
                    identity_id: None,
                    memory_id: None,
                    session_id: None,
                    working_dir: "agenty-0629j".to_string(),
                    source_agents_base: None,
                    created_at_ms: 1,
                    last_launched_at_ms: 1,
                    created_by_version: "test".to_string(),
                    last_launched_by_version: "test".to_string(),
                },
            })
            .unwrap();

        let result = identity_self_accounts_impl(&state, "agenty").await;

        match prev {
            Some(v) => std::env::set_var("AGENTMUX_HOME_OVERRIDE", v),
            None => std::env::remove_var("AGENTMUX_HOME_OVERRIDE"),
        }

        let result = result.expect("must resolve via the registry fallback, not error 'unknown agent'");
        let accounts = result["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0]["account_id"], json!("acct-good"));
    }
}
