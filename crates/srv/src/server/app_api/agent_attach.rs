// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent that already has a block, when it is opened or started
// (docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md §3.1, §5.1, §9 P1 PR C): `agent.open` gives
// it a pane by attaching its parked block, or focuses the pane that shows it, instead of making a
// second block; a start with no pane runs it as a parked block. The agent's desired state, whether
// it should be running, is block meta (`agent:desired`), the agent's own state while it has one block.

use super::agent_open::record_agent_open_launch;
use super::*;
use crate::server::AppState;
use agentmux_common::ipc::{Command, Event};

/// Block meta: whether the agent should be running, `running` or `stopped`. Open, start and a
/// message that starts it set `running`; an explicit stop sets `stopped`.
pub(crate) const META_DESIRED: &str = "agent:desired";
pub(crate) const DESIRED_RUNNING: &str = "running";
pub(crate) const DESIRED_STOPPED: &str = "stopped";

/// Marks an open that is a start: the block it makes is parked, not placed in a layout.
pub(super) const START_PARKED: &str = "agent:start_parked";

/// Record the agent's desired state on its block. Best effort: a failed write costs a restart's
/// resume, not the start or stop itself.
pub(crate) fn set_desired(mstore: &Store, block_id: &str, desired: &str) {
    let mut meta = obj::MetaMapType::new();
    meta.insert(META_DESIRED.to_string(), json!(desired));
    if let Err(e) = crate::server::service::update_object_meta(mstore, &format!("block:{block_id}"), &meta) {
        tracing::warn!(block_id, desired, "could not record the agent's desired state: {e}");
    }
}

/// The desired state a block records, if any.
pub(crate) fn desired_of(block: &Block) -> Option<String> {
    let v = obj::meta_get_string(&block.meta, META_DESIRED, "");
    (!v.is_empty()).then_some(v)
}

/// Dispatch to the reducer, mirror to SQLite and publish: what `agent.open` does for a block.
async fn dispatch_all(state: &AppState, cmd: Command) -> Result<Vec<Event>, String> {
    let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
    if let Some(msg) = events.iter().find_map(|e| match e {
        Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(msg);
    }
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore) {
            tracing::warn!("agent attach: mstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &events);
    Ok(events)
}

/// Register the block's controller when it has none (a pane the frontend made, or a parked block
/// after a restart), resuming the agent's session. With `revive`, a controller whose process has
/// ended is resynced too, which resumes it (a start, or an open with the setting on). `tab_id` is
/// empty for a parked block.
fn ensure_controller(
    state: &AppState,
    agent: &crate::backend::storage::agents::AgentDefinition,
    provider: &providers::ProviderConfig,
    block_id: &str,
    tab_id: &str,
    revive: bool,
) -> Result<(), String> {
    let ended = revive
        && blockcontroller::get_block_controller_status(block_id)
            .is_some_and(|s| s.shellprocstatus == blockcontroller::STATUS_DONE);
    if blockcontroller::get_controller(block_id).is_some() && !ended {
        return Ok(());
    }
    let mstore = &state.mstore;
    let controller_type = if agent.agent_type == "container" { "subprocess" } else { provider.controller_type_str() };
    let mut meta_update = obj::MetaMapType::new();
    meta_update.insert("controller".to_string(), json!(controller_type));
    meta_update.insert("agentProvider".to_string(), json!(&agent.provider));
    let _ = crate::server::service::update_object_meta(mstore, &format!("block:{block_id}"), &meta_update);
    // The launch is recorded before the resync: a controller can report its session id during it,
    // and that capture finds the row by block (#3584).
    record_agent_open_launch(mstore, &agent.id, block_id)?;
    let block = mstore.must_get::<Block>(block_id).map_err(|e| format!("agent.open: reload block: {e}"))?;
    let _ = blockcontroller::resync_controller(
        &block,
        tab_id,
        None,
        true,
        true,
        Some(state.broker.clone()),
        Some(state.event_bus.clone()),
        Some(mstore.clone()),
        Some(state.filestore.clone()),
        Some(state.id_store.clone()),
        Some(state.identity_store.clone()),
        mstore.shared_agent_registry(),
        state.boot_id.clone(),
        &state.auth_key,
        Some(state.config_watcher.clone()),
    );
    Ok(())
}

/// Bring the window to the pane that shows `block_id` in `tab_id`: `block.reveal` makes it its pane's
/// visible member and tells its window to restore, un-magnify and focus it; where that finds no open
/// window, its tab is made active and its pane focused here.
async fn focus_shown(state: &AppState, tab_id: &str, block_id: &str) {
    let router = crate::server::notify_handlers::router_for(state);
    if matches!(crate::server::notify_handlers::reveal_block(state, &router, block_id.to_string(), false).await, Ok(r) if r.found) {
        return;
    }
    let (workspace_id, node_id) = {
        let s = state.srv_state.lock().await;
        let ws = s.workspaces.iter().find(|(_, w)| w.tab_ids.iter().any(|t| t == tab_id)).map(|(id, _)| id.clone());
        let node = s
            .tabs
            .get(tab_id)
            .and_then(|t| t.rootnode.as_ref())
            .and_then(|root| crate::backend::layout::find_leaf_containing_block(root, block_id))
            .map(|n| n.id.clone());
        (ws, node)
    };
    if let Some(workspace_id) = workspace_id {
        if let Err(e) = dispatch_all(state, Command::SetActiveTab { workspace_id, tab_id: tab_id.to_string() }).await {
            tracing::warn!(tab_id, "agent.open: could not activate the tab of the open agent: {e}");
        }
    }
    if let Some(node_id) = node_id {
        if let Err(e) = dispatch_all(state, Command::SetFocusedNode { tab_id: tab_id.to_string(), node_id }).await {
            tracing::warn!(tab_id, block_id, "agent.open: could not focus the open agent's pane: {e}");
        }
    }
}

/// A new block made by a start: out of its tab at once, so it is the agent's parked block.
pub(super) async fn park_new(state: &AppState, tab_id: &str, block_id: &str) -> Result<(), String> {
    dispatch_all(state, Command::ParkBlock { tab_id: tab_id.to_string(), block_id: block_id.to_string() })
        .await
        .map(|_| ())
        .map_err(|e| format!("agent.open: ParkBlock: {e}"))
}

/// What `agent.open` (or a start, `headless`) does when the agent already has a block:
///   - in `tab_id`: that pane, as before (a start leaves it as it is);
///   - parked: attach it to `tab_id` and place its pane, no new block (a start leaves it parked);
///   - shown in another tab, with `agents:always_available` on: that pane, focused, no second one.
///
/// `None`: nothing to reuse, the caller makes the block.
pub(super) async fn reuse_existing_pane(
    state: &AppState,
    agent: &crate::backend::storage::agents::AgentDefinition,
    provider: &providers::ProviderConfig,
    cmd: &CommandAgentOpenData,
    tab_id: &str,
    headless: bool,
    by_uid: bool,
) -> Result<Option<AgentOpenResult>, String> {
    let mstore = &state.mstore;
    let revive = headless || crate::sagas::park_agents::enabled(state);
    let result = |block_id: &str, tab: &str, created_provider: &str| {
        let status = blockcontroller::get_block_controller_status(block_id)
            .map(|s| s.shellprocstatus)
            .unwrap_or_else(|| "init".to_string());
        AgentOpenResult {
            block_id: block_id.to_string(),
            tab_id: tab.to_string(),
            agent_id: cmd.agent_id.clone(),
            provider: created_provider.to_string(),
            controller_type: provider.controller_type_str().to_string(),
            status,
            created: false,
        }
    };

    // No tab left (a start after the last workspace was deleted): only the by-agent lookup below applies.
    if !tab_id.is_empty() {
        if let Some(existing) = find_agent_block(mstore, tab_id, &agent.id)? {
            ensure_controller(state, agent, provider, &existing.oid, tab_id, revive)?;
            if revive {
                set_desired(mstore, &existing.oid, DESIRED_RUNNING);
            }
            return Ok(Some(result(&existing.oid, tab_id, &agent.provider)));
        }
    }
    // A pane being filled in (layout import) only ever reuses a pane of this tab.
    if !by_uid {
        return Ok(None);
    }

    let Some(found) = super::agent_block::find_agent_block_by_uid(mstore, &agent.id)? else {
        return Ok(None);
    };
    let block_id = found.block.oid.clone();
    match found.tab_id {
        None => {
            if headless {
                ensure_controller(state, agent, provider, &block_id, "", true)?;
                set_desired(mstore, &block_id, DESIRED_RUNNING);
                return Ok(Some(result(&block_id, "", &agent.provider)));
            }
            dispatch_all(state, Command::AttachBlock { block_id: block_id.clone(), dst_tab_id: tab_id.to_string(), dst_index: u32::MAX })
                .await
                .map_err(|e| format!("agent.open: AttachBlock: {e}"))?;
            let placement = super::pane::resolve_placement(
                cmd.split_direction.as_deref(),
                cmd.split_reference_block_id.as_deref(),
                cmd.focus.unwrap_or(true),
            );
            if let Err(e) = crate::server::service::layout_intents::place_block(state, tab_id, &block_id, placement).await {
                tracing::warn!("agent.open: placing the attached pane failed: {e}");
            }
            ensure_controller(state, agent, provider, &block_id, tab_id, true)?;
            set_desired(mstore, &block_id, DESIRED_RUNNING);
            Ok(Some(result(&block_id, tab_id, &agent.provider)))
        }
        Some(shown_in) if revive => {
            ensure_controller(state, agent, provider, &block_id, &shown_in, true)?;
            set_desired(mstore, &block_id, DESIRED_RUNNING);
            if !headless && cmd.focus.unwrap_or(true) {
                focus_shown(state, &shown_in, &block_id).await;
            }
            Ok(Some(result(&block_id, &shown_in, &agent.provider)))
        }
        Some(_) => Ok(None),
    }
}

/// The tab `agent.open` works in, or the agent's existing pane when it has one (`Err`: nothing more to
/// do). A start of a terminal-tier CLI (aider, crush) is refused: its term view starts it, and a start
/// has none. A start needs no tab to run an agent that already has a block, so with no tab left it only
/// reuses that block; a first block still needs a tab.
pub(super) async fn tab_or_reuse(
    state: &AppState,
    agent: &crate::backend::storage::agents::AgentDefinition,
    provider: &providers::ProviderConfig,
    cmd: &CommandAgentOpenData,
    headless: bool,
    by_uid: bool,
) -> Result<Result<String, AgentOpenResult>, String> {
    if headless && provider.controller_type == providers::ControllerType::Terminal {
        return Err(format!("agent.start: {} runs in a terminal pane and has no headless mode", agent.provider));
    }
    let tab_id = match super::resolve_tab_id(&state.mstore, cmd.tab_id.as_deref()) {
        Ok(tab_id) => tab_id,
        Err(e) if headless => return reuse_existing_pane(state, agent, provider, cmd, "", true, by_uid).await?.map(Err).ok_or(e),
        Err(e) => return Err(e),
    };
    Ok(match reuse_existing_pane(state, agent, provider, cmd, &tab_id, headless, by_uid).await? {
        Some(done) => Err(done),
        None => Ok(tab_id),
    })
}

/// Start the agent with no pane: its block (made and parked if it has none) with its process running
/// and its session resumed. Returns the block. Admission, the spawn gate and the lease apply as for
/// `agent.open`. Needs an active tab to create a first block in.
pub(crate) async fn start_agent(state: &AppState, agent_id: &str) -> Result<String, String> {
    // A start over the cap (or the host's commit limit) stops an idle agent with no view, or is refused.
    crate::server::agent_lifecycle::admit_start(state, agent_id).await?;
    start_admitted(state, agent_id).await
}

/// [`start_agent`] that never stops another agent to make room: over the cap or the commit limit it is
/// refused (`Err("capacity: ...")`). Restart resume starts the agents that were running this way.
pub(crate) async fn start_agent_without_eviction(state: &AppState, agent_id: &str) -> Result<String, String> {
    crate::server::agent_lifecycle::admit_start_without_eviction(state, agent_id).await?;
    start_admitted(state, agent_id).await
}

/// The start itself, once admitted: a failure gives the admission's room back.
async fn start_admitted(state: &AppState, agent_id: &str) -> Result<String, String> {
    let mut marker = obj::MetaMapType::new();
    marker.insert(START_PARKED.to_string(), json!(true));
    let opened = super::agent_open::open_agent_inner(
        state,
        CommandAgentOpenData {
            agent_id: agent_id.to_string(),
            tab_id: None,
            split_direction: None,
            split_reference_block_id: None,
            focus: Some(false),
        },
        None,
        marker,
    )
    .await
    .map_err(|e| {
        crate::server::agent_lifecycle::release_start(state, agent_id);
        e
    })?;
    Ok(opened.block_id)
}
